//! Background waveform loading with progress reporting and cancellation.
//!
//! Parsing a large dump can take seconds to minutes; keeping it on the UI
//! thread would freeze the TUI. The worker sends progress events that the
//! App drains from its idle tick. For FSDB the worker keeps the reader alive
//! after parsing and loads signal values on demand, so only signals that are
//! actually added to the waveform materialize their value changes.
//!
//! The session worker is split in two: one thread owns the FSDB reader (FFR
//! is process-global, so reads stay serialized) and forwards everything that
//! does not need FFR - aggregate/array recomputation, radix re-formatting -
//! to a small compute pool. That keeps the UI thread free and lets heavy
//! value math run in parallel with the next value read.

use crate::dump::{self, LoadProgress, ParseOut, Stage};
use crate::rtl::{RtlDb, SourceSet};
#[cfg(fsdb_sdk)]
use crate::waveform::Waveform;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;

/// Requests sent from the App to the loader worker.
pub enum LoadRequest {
    /// Materialize the value changes of `wf.signals[index]` (and of the array
    /// or aggregate signals that become complete once its members are loaded).
    #[cfg_attr(not(fsdb_sdk), allow(dead_code))]
    Signal(usize),
    /// Re-format array/aggregate brace texts with the given radix overrides.
    #[cfg_attr(not(fsdb_sdk), allow(dead_code))]
    RebuildArrays(std::collections::HashMap<usize, crate::waveform::Radix>),
    Shutdown,
}

pub enum LoadEvent {
    Progress(LoadProgress),
    Waveform(Box<ParseOut>),
    Rtl(Box<SourceSet>, Box<RtlDb>),
    #[cfg_attr(not(fsdb_sdk), allow(dead_code))]
    /// `(signal index, changes)` updates plus warnings from the backend.
    Changes(Vec<(usize, Vec<crate::waveform::Change>)>, Vec<String>),
    Failed(String),
    Done,
}

/// A running (or finished) background load. For lazy FSDB dumps the job stays
/// alive after [`LoadEvent::Done`] to serve value requests.
pub struct LoadJob {
    pub path: String,
    pub rx: Receiver<LoadEvent>,
    requests: Option<Sender<LoadRequest>>,
    cancel: Arc<AtomicBool>,
    pub progress: LoadProgress,
    pub finished: bool,
}

impl LoadJob {
    /// Start parsing `path` on a worker thread. When `discover_sources` is
    /// set the worker also parses the RTL sources of a Verdi KDB next to the
    /// dump and reports them through [`LoadEvent::Rtl`].
    pub fn start(path: &str, discover_sources: bool) -> LoadJob {
        let (tx, rx) = mpsc::channel();
        let (req_tx, req_rx) = mpsc::channel();
        // Only the lazy FSDB path consumes requests.
        #[cfg(not(fsdb_sdk))]
        let _ = &req_rx;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let path_owned = path.to_string();
        std::thread::spawn(move || {
            let cancelled = || flag.load(Ordering::Relaxed);

            #[cfg(fsdb_sdk)]
            if dump::detect(Path::new(&path_owned)) == dump::Format::Fsdb {
                let mut report = |progress: LoadProgress| -> bool {
                    let _ = tx.send(LoadEvent::Progress(progress));
                    !cancelled()
                };
                match crate::fsdb::parse_fsdb_lazy(Path::new(&path_owned), &mut report) {
                    Ok((mut out, mut session)) => {
                        if cancelled() {
                            let _ = tx.send(LoadEvent::Failed("load cancelled".to_string()));
                            return;
                        }
                        // Array grouping and scope aggregates touch every
                        // signal; do it off the UI thread. Values are still
                        // lazy at this point.
                        out.wf.build_arrays();
                        out.wf.build_scope_aggregates();
                        for signal in &mut out.wf.signals {
                            if signal.var_type == "array" {
                                signal.state = crate::waveform::SigState::Lazy;
                            }
                        }
                        let meta = Arc::new(std::sync::Mutex::new(out.wf.clone()));
                        let _ = tx.send(LoadEvent::Waveform(Box::new(out)));
                        if discover_sources {
                            parse_rtl_sources(&path_owned, &tx, &flag);
                        }
                        let _ = tx.send(LoadEvent::Done);
                        serve_session(meta, &mut session, &req_rx, &tx);
                        return;
                    }
                    Err(err) => {
                        let _ = tx.send(LoadEvent::Failed(err));
                        return;
                    }
                }
            }

            let mut report = |progress: LoadProgress| -> bool {
                let _ = tx.send(LoadEvent::Progress(progress));
                !cancelled()
            };
            let mut out = match dump::parse_with_progress(Path::new(&path_owned), &mut report) {
                Ok(out) => out,
                Err(err) => {
                    let _ = tx.send(LoadEvent::Failed(err));
                    return;
                }
            };
            if cancelled() {
                let _ = tx.send(LoadEvent::Failed("load cancelled".to_string()));
                return;
            }
            // Array grouping and scope aggregates touch every signal; do it
            // off the UI thread.
            out.wf.build_arrays();
            out.wf.build_scope_aggregates();
            let _ = tx.send(LoadEvent::Waveform(Box::new(out)));

            if discover_sources && path_owned.ends_with(".fsdb") {
                parse_rtl_sources(&path_owned, &tx, &flag);
            }
            let _ = tx.send(LoadEvent::Done);
        });
        LoadJob {
            path: path.to_string(),
            rx,
            requests: Some(req_tx),
            cancel,
            progress: LoadProgress {
                stage: Stage::Waveform,
                done: 0,
                total: 0,
                changes: 0,
            },
            finished: false,
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Ask the worker for a signal's value changes. Returns false when no
    /// lazy backend is alive.
    pub fn request(&self, index: usize) -> bool {
        self.requests
            .as_ref()
            .map(|tx| tx.send(LoadRequest::Signal(index)).is_ok())
            .unwrap_or(false)
    }

    /// Ask the worker to re-format array/aggregate brace texts off the UI
    /// thread. Returns false when no lazy backend is alive.
    pub fn request_rebuild(
        &self,
        radix: std::collections::HashMap<usize, crate::waveform::Radix>,
    ) -> bool {
        self.requests
            .as_ref()
            .map(|tx| tx.send(LoadRequest::RebuildArrays(radix)).is_ok())
            .unwrap_or(false)
    }

    /// Non-blocking poll of the worker.
    pub fn try_recv(&self) -> Result<LoadEvent, TryRecvError> {
        self.rx.try_recv()
    }
}

impl Drop for LoadJob {
    fn drop(&mut self) {
        self.cancel();
        if let Some(tx) = &self.requests {
            let _ = tx.send(LoadRequest::Shutdown);
        }
    }
}

/// Discover and parse the RTL sources recorded in the Verdi KDB next to the
/// dump, sending them back as [`LoadEvent::Rtl`].
fn parse_rtl_sources(path: &str, tx: &Sender<LoadEvent>, flag: &AtomicBool) {
    let Some(set) = SourceSet::discover_from_dump(Path::new(path)) else {
        return;
    };
    let total = set.files.len();
    let _ = tx.send(LoadEvent::Progress(LoadProgress {
        stage: Stage::Rtl,
        done: 0,
        total,
        changes: 0,
    }));
    let db = RtlDb::parse_sources_with_progress(&set, &mut |done, total| {
        let _ = tx.send(LoadEvent::Progress(LoadProgress {
            stage: Stage::Rtl,
            done,
            total,
            changes: 0,
        }));
        !flag.load(Ordering::Relaxed)
    });
    if !flag.load(Ordering::Relaxed) {
        let _ = tx.send(LoadEvent::Rtl(Box::new(set), Box::new(db)));
    }
}

/// Work that needs no FFR access and can run on the compute pool.
#[cfg(fsdb_sdk)]
enum ComputeTask {
    Recompute,
    Rebuild(std::collections::HashMap<usize, crate::waveform::Radix>),
}

#[cfg(fsdb_sdk)]
fn pool_size() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 4))
        .unwrap_or(2)
}

/// Serve on-demand value requests for an open FSDB session until the App
/// drops the job (the request channel closes) or asks for shutdown.
///
/// Value reads happen on this thread (FFR is process-global); everything else
/// is handed to the compute pool so reads are not delayed by value math.
#[cfg(fsdb_sdk)]
fn serve_session(
    meta: Arc<std::sync::Mutex<Waveform>>,
    session: &mut crate::fsdb::FsdbSession,
    req_rx: &Receiver<LoadRequest>,
    tx: &Sender<LoadEvent>,
) {
    let (task_tx, task_rx) = mpsc::channel::<ComputeTask>();
    let task_rx = Arc::new(std::sync::Mutex::new(task_rx));
    for _ in 0..pool_size() {
        let rx = Arc::clone(&task_rx);
        let meta = Arc::clone(&meta);
        let tx = tx.clone();
        std::thread::spawn(move || loop {
            let task = rx.lock().unwrap_or_else(|err| err.into_inner()).recv();
            let Ok(task) = task else {
                break;
            };
            match task {
                ComputeTask::Recompute => {
                    let updates = {
                        let mut wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                        wf.recompute_ready_arrays()
                            .into_iter()
                            .map(|index| (index, wf.signals[index].changes.clone()))
                            .collect::<Vec<_>>()
                    };
                    if !updates.is_empty() {
                        let _ = tx.send(LoadEvent::Changes(updates, Vec::new()));
                    }
                }
                ComputeTask::Rebuild(radix) => {
                    let updates = {
                        let mut wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                        wf.rebuild_array_texts(&radix)
                            .into_iter()
                            .map(|index| (index, wf.signals[index].changes.clone()))
                            .collect::<Vec<_>>()
                    };
                    if !updates.is_empty() {
                        let _ = tx.send(LoadEvent::Changes(updates, Vec::new()));
                    }
                }
            }
        });
    }
    drop(task_rx);

    while let Ok(request) = req_rx.recv() {
        match request {
            LoadRequest::Signal(index) => {
                let leaves = {
                    let wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                    if index >= wf.signals.len() {
                        continue;
                    }
                    wf.value_leaves(index)
                };
                let var_count = session.var_count();
                let mut updates = Vec::new();
                let mut warnings = Vec::new();
                for leaf in leaves {
                    let needs_load = {
                        let wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                        leaf < var_count
                            && wf.signals[leaf].state == crate::waveform::SigState::Lazy
                    };
                    if !needs_load {
                        continue;
                    }
                    let (changes, warns) = session.read_signal(leaf);
                    let installed = {
                        let mut wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                        wf.signals[leaf].changes = changes;
                        wf.signals[leaf].state = crate::waveform::SigState::Ready;
                        wf.signals[leaf].changes.clone()
                    };
                    updates.push((leaf, installed));
                    warnings.extend(warns);
                }
                if !updates.is_empty() || !warnings.is_empty() {
                    let _ = tx.send(LoadEvent::Changes(updates, warnings));
                }
                let _ = task_tx.send(ComputeTask::Recompute);
            }
            LoadRequest::RebuildArrays(radix) => {
                let _ = task_tx.send(ComputeTask::Rebuild(radix));
            }
            LoadRequest::Shutdown => break,
        }
    }
    drop(task_tx);
}
