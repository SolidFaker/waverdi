//! Background waveform loading with progress reporting and cancellation.
//!
//! Parsing a large dump can take seconds to minutes; keeping it on the UI
//! thread would freeze the TUI. The worker sends progress events that the
//! App drains from its idle tick. For FSDB the worker keeps the reader alive
//! after parsing and loads signal values on demand, so only signals that are
//! actually added to the waveform materialize their value changes.
//!
//! The session worker is split in two: one thread owns the FSDB reader (FFR
//! cannot be used from two threads - concurrent readers deadlock inside the
//! library), so reads stay serialized and everything that does not need FFR -
//! installing values, aggregate/array recomputation, radix re-formatting -
//! runs on a small compute pool. Queued signal requests are coalesced into
//! one batch per read pass, so adding a whole hierarchy level keeps the
//! reader busy and produces a single UI update per batch.

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
    worker: Option<std::thread::JoinHandle<()>>,
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
        let worker = std::thread::spawn(move || {
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
            worker: Some(worker),
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
        // Wait for the worker to leave FFR before the process exits: the
        // library's teardown stalls while a reader thread is still inside it.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
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
    /// Install a batch of freshly read signals into the shared waveform and
    /// complete the aggregate/array math. One task per batch keeps the pool
    /// fed while the reader thread moves on to the next signal.
    Install(Vec<(usize, Vec<crate::waveform::Change>, Vec<String>)>),
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
/// Value reads must happen on this thread (FFR cannot be used from several
/// threads - two simultaneous readers deadlock inside the library), so the
/// reads of one batch stay back to back while the compute pool installs the
/// previous batch, clones the values for the UI and completes the aggregate
/// math in parallel.
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
                ComputeTask::Install(raw) => {
                    // Install first so the aggregates below can see the new
                    // values, then clone once for the UI.
                    let updates = {
                        let mut wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                        let mut updates = Vec::with_capacity(raw.len());
                        let mut warnings = Vec::new();
                        for (leaf, changes, mut warns) in raw {
                            if let Some(signal) = wf.signals.get_mut(leaf) {
                                signal.changes = changes;
                                signal.state = crate::waveform::SigState::Ready;
                                updates.push((leaf, signal.changes.clone()));
                            }
                            warnings.append(&mut warns);
                        }
                        (updates, warnings)
                    };
                    if !updates.0.is_empty() || !updates.1.is_empty() {
                        let _ = tx.send(LoadEvent::Changes(updates.0, updates.1));
                    }
                    // The batch is fully installed: complete any aggregate
                    // whose members are now all ready.
                    let arrays = {
                        let mut wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                        wf.recompute_ready_arrays()
                            .into_iter()
                            .map(|index| (index, wf.signals[index].changes.clone()))
                            .collect::<Vec<_>>()
                    };
                    if !arrays.is_empty() {
                        let _ = tx.send(LoadEvent::Changes(arrays, Vec::new()));
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
            LoadRequest::Signal(first) => {
                // Adding a whole level queues many requests at once: drain
                // them into one batch so the reads stay back to back and the
                // UI gets a single update per batch.
                let mut batch = vec![first];
                let mut rebuild = None;
                let mut shutdown = false;
                loop {
                    match req_rx.try_recv() {
                        Ok(LoadRequest::Signal(index)) => batch.push(index),
                        Ok(LoadRequest::RebuildArrays(radix)) => rebuild = Some(radix),
                        Ok(LoadRequest::Shutdown) => {
                            shutdown = true;
                            break;
                        }
                        Err(_) => break,
                    }
                }
                let var_count = session.var_count();
                let mut raw = Vec::new();
                for index in batch {
                    let leaves = {
                        let wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                        if index >= wf.signals.len() {
                            continue;
                        }
                        wf.value_leaves(index)
                    };
                    for leaf in leaves {
                        // Claim the leaf before reading it: the install runs
                        // on the pool, so the next request must not read it
                        // again while it is in flight.
                        let claimed = {
                            let mut wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                            match wf.signals.get_mut(leaf) {
                                Some(signal)
                                    if leaf < var_count
                                        && signal.state == crate::waveform::SigState::Lazy =>
                                {
                                    signal.state = crate::waveform::SigState::Loading;
                                    true
                                }
                                _ => false,
                            }
                        };
                        if !claimed {
                            continue;
                        }
                        let (changes, warns) = session.read_signal(leaf);
                        raw.push((leaf, changes, warns));
                    }
                }
                if !raw.is_empty() {
                    let _ = task_tx.send(ComputeTask::Install(raw));
                }
                if let Some(radix) = rebuild {
                    let _ = task_tx.send(ComputeTask::Rebuild(radix));
                }
                if shutdown {
                    break;
                }
            }
            LoadRequest::RebuildArrays(radix) => {
                let _ = task_tx.send(ComputeTask::Rebuild(radix));
            }
            LoadRequest::Shutdown => break,
        }
    }
    drop(task_tx);
}

#[cfg(all(test, fsdb_sdk))]
mod tests {
    use super::*;
    use crate::waveform::SigState;
    use std::sync::{Arc, Mutex};

    /// A batch of requests is read back to back and installed by the pool:
    /// every requested signal becomes Ready and its values reach the UI.
    #[test]
    fn batched_requests_install_their_signals() {
        let Some(home) = std::env::var_os("VERDI_HOME") else {
            return;
        };
        let path =
            std::path::PathBuf::from(home).join("demo/nCompare/nCmp_demo1/demo_RTL_verilog.fsdb");
        if !path.is_file() {
            return;
        }
        let (out, mut session) =
            crate::fsdb::parse_fsdb_lazy(&path, &mut |_| true).expect("lazy parse");
        let indexes: Vec<usize> = (0..out.wf.signals.len())
            .filter(|&index| {
                out.wf.signals[index].state == SigState::Lazy
                    && out.wf.signals[index].members.is_empty()
            })
            .take(24)
            .collect();
        assert!(!indexes.is_empty(), "demo dump has lazy signals");
        let meta = Arc::new(Mutex::new(out.wf.clone()));
        let (req_tx, req_rx) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        let requested = indexes.clone();
        let producer = std::thread::spawn(move || {
            for index in requested {
                let _ = req_tx.send(LoadRequest::Signal(index));
            }
            drop(req_tx);
        });
        serve_session(Arc::clone(&meta), &mut session, &req_rx, &tx);
        producer.join().unwrap();
        drop(tx);
        let mut installed = std::collections::HashSet::new();
        for event in rx {
            if let LoadEvent::Changes(updates, _) = event {
                installed.extend(updates.into_iter().map(|(index, _)| index));
            }
        }
        for index in &indexes {
            assert!(
                installed.contains(index),
                "signal {index} was not installed"
            );
            assert_eq!(
                meta.lock().unwrap().signals[*index].state,
                SigState::Ready,
                "signal {index} was not marked ready"
            );
        }
    }
}
