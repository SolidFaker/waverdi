//! Background waveform loading with progress reporting and cancellation.
//!
//! Parsing a large dump can take seconds to minutes; keeping it on the UI
//! thread would freeze the TUI. The worker sends progress events that the
//! App drains from its idle tick. For FSDB the worker keeps the reader alive
//! after parsing and loads signal values on demand, so only signals that are
//! actually added to the waveform materialize their value changes.

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
    #[cfg_attr(not(fsdb_sdk), allow(dead_code))]
    /// Materialize the value changes of `wf.signals[index]` (and of the array
    /// signals that become complete once its elements are loaded).
    Signal(usize),
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
                        let mut meta = out.wf.clone();
                        let _ = tx.send(LoadEvent::Waveform(Box::new(out)));
                        if discover_sources {
                            parse_rtl_sources(&path_owned, &tx, &flag);
                        }
                        let _ = tx.send(LoadEvent::Done);
                        serve_requests(&mut meta, &mut session, &req_rx, &tx);
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

/// Serve on-demand value requests for an open FSDB session until the App
/// drops the job (the request channel closes) or asks for shutdown.
#[cfg(fsdb_sdk)]
fn serve_requests(
    meta: &mut Waveform,
    session: &mut crate::fsdb::FsdbSession,
    req_rx: &Receiver<LoadRequest>,
    tx: &Sender<LoadEvent>,
) {
    while let Ok(request) = req_rx.recv() {
        match request {
            LoadRequest::Signal(index) => {
                if let Some((updates, warnings)) = load_signal(meta, session, index) {
                    let _ = tx.send(LoadEvent::Changes(updates, warnings));
                }
            }
            LoadRequest::Shutdown => break,
        }
    }
}

/// Load `index`'s element values and recompute the array signals that become
/// complete. Returns the updates to send to the App.
#[cfg(fsdb_sdk)]
fn load_signal(
    meta: &mut Waveform,
    session: &mut crate::fsdb::FsdbSession,
    index: usize,
) -> Option<(Vec<(usize, Vec<crate::waveform::Change>)>, Vec<String>)> {
    use crate::waveform::SigState;
    if index >= meta.signals.len() {
        return None;
    }
    let var_count = session.var_count();
    let mut updates = Vec::new();
    let mut warnings = Vec::new();
    for leaf in meta.value_leaves(index) {
        if leaf >= var_count || meta.signals[leaf].state != SigState::Lazy {
            continue;
        }
        let (changes, warns) = session.read_signal(leaf);
        meta.signals[leaf].changes = changes;
        meta.signals[leaf].state = SigState::Ready;
        warnings.extend(warns);
        updates.push((leaf, meta.signals[leaf].changes.clone()));
    }
    for array in meta.recompute_ready_arrays() {
        updates.push((array, meta.signals[array].changes.clone()));
    }
    if updates.is_empty() && warnings.is_empty() {
        None
    } else {
        Some((updates, warnings))
    }
}
