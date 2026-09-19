//! Format-independent loading runtime: one worker owns the dump source and a
//! small compute pool installs the values it reads.
//!
//! Parsing a large dump can take seconds to minutes; keeping it on the UI
//! thread would freeze the TUI. A [`LoadJob`] bundles the session: the worker
//! thread, the events it reports (progress, capabilities, hierarchy, RTL
//! sources, value changes) and the request channel used to pull values from
//! the open backend. The UI owns a [`LoadJob`] and drains its events on its
//! idle ticks.
//!
//! Every format is opened through a [`crate::dump::source::DumpSource`]: the
//! worker hands the hierarchy to the UI first and keeps the source alive
//! afterwards, loading signal values only when they are actually added to the
//! waveform.
//!
//! The worker is split in two: one thread owns the source - for FSDB that is
//! the FFR reader (FFR cannot be used from two threads - concurrent readers
//! deadlock inside the library), so reads stay serialized - while everything
//! that does not need the backend - installing values, aggregate/array
//! recomputation, radix re-formatting - runs on a small compute pool. Queued
//! signal requests are coalesced into one batch per read pass, so adding a
//! whole hierarchy level keeps the reader busy and produces a single UI
//! update per batch.

use crate::dump::source::Capabilities;
use crate::dump::{LoadProgress, ParseOut, Stage};
use crate::rtl::{RtlDb, SourceSet};
use crate::waveform::Waveform;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;

/// Requests sent from the UI to the loader worker.
pub enum LoadRequest {
    /// Materialize the value changes of `wf.signals[index]` (and of the array
    /// or aggregate signals that become complete once its members are loaded).
    Signal(usize),
    /// Re-format array/aggregate brace texts with the given radix overrides.
    RebuildArrays(std::collections::HashMap<usize, crate::waveform::Radix>),
    Shutdown,
}

pub enum LoadEvent {
    Progress(LoadProgress),
    /// What the opened backend can provide. Sent right after the source opens
    /// and before [`LoadEvent::Waveform`], so the UI can gate features the
    /// moment the dump is there.
    Capabilities(Capabilities),
    Waveform(Box<ParseOut>),
    Rtl(Box<SourceSet>, Box<RtlDb>),
    /// `(signal index, changes)` updates plus warnings from the backend. The
    /// changes travel as an `Arc` so installing them into the app moves the
    /// worker's allocation instead of copying it.
    Changes(Vec<(usize, Arc<Vec<crate::waveform::Change>>)>, Vec<String>),
    Failed(String),
    Done,
}

/// A running (or finished) background load: the session worker, the events it
/// reports and the request channel used to pull lazy values from the open
/// backend. The job stays alive after [`LoadEvent::Done`] to serve value
/// requests.
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
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let path_owned = path.to_string();
        let worker = std::thread::spawn(move || {
            let cancelled = || flag.load(Ordering::Relaxed);
            let mut report = |progress: LoadProgress| -> bool {
                let _ = tx.send(LoadEvent::Progress(progress));
                !cancelled()
            };
            let mut source =
                match crate::dump::source::open_source(Path::new(&path_owned), &mut report) {
                    Ok(source) => source,
                    Err(err) => {
                        let _ = tx.send(LoadEvent::Failed(err));
                        return;
                    }
                };
            // Describe the backend before any of its data arrives: the UI can
            // then gate features without asking the format.
            let _ = tx.send(LoadEvent::Capabilities(source.capabilities()));
            if cancelled() {
                let _ = tx.send(LoadEvent::Failed("load cancelled".to_string()));
                return;
            }
            let mut out = ParseOut {
                wf: source.take_hierarchy(),
                warnings: source.take_warnings(),
            };
            // Array grouping and scope aggregates touch every signal; do it
            // off the UI thread. Values are still lazy at this point.
            out.wf.build_arrays();
            out.wf.build_scope_aggregates();
            for signal in &mut out.wf.signals {
                if signal.var_type == "array" {
                    signal.state = crate::waveform::SigState::Lazy;
                }
            }
            // Values are still lazy at this point, so the copy shares the
            // parsed changes as arcs; installing a signal below then only
            // allocates the freshly decoded list.
            let meta = Arc::new(std::sync::Mutex::new(out.wf.clone()));
            let _ = tx.send(LoadEvent::Waveform(Box::new(out)));

            if discover_sources && path_owned.ends_with(".fsdb") {
                parse_rtl_sources(&path_owned, &tx, &flag);
            }
            let _ = tx.send(LoadEvent::Done);
            // Eager dumps already carry their values: they end here and the
            // UI recomputes aggregates locally instead of round-tripping.
            if !source.is_lazy() {
                return;
            }
            serve_session(meta, &mut *source, &req_rx, &tx);
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
        // Wait for the worker to leave the backend before the process exits:
        // FFR's teardown stalls while a reader thread is still inside it.
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

/// Values on their way into the shared waveform. The FSDB reader hands over
/// raw bytes and the pool decodes them, so the FFR lock is not held during
/// the value math for wide signals.
enum PendingValues {
    /// Freshly decoded or already-shared changes; the `Arc` moves into the
    /// waveform and is cloned (refcount only) on its way to the UI.
    Decoded(Arc<Vec<crate::waveform::Change>>),
    #[cfg(fsdb_sdk)]
    Raw(crate::fsdb::RawChanges),
}

/// Work that needs no backend access and can run on the compute pool.
enum ComputeTask {
    /// Install a batch of freshly read signals into the shared waveform and
    /// complete the aggregate/array math. One task per batch keeps the pool
    /// fed while the reader thread moves on to the next signal.
    Install(Vec<(usize, PendingValues, Vec<String>)>),
    Rebuild(std::collections::HashMap<usize, crate::waveform::Radix>),
}

fn pool_size() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 4))
        .unwrap_or(2)
}

/// Decode a batch of leaves on the reader thread, for sources without a raw
/// value path. FSDB skips this: its bytes go to the pool undecoded.
fn decode_batch(
    session: &mut dyn crate::dump::source::DumpSource,
    leaves: &[usize],
) -> Vec<(PendingValues, Vec<String>)> {
    session
        .read_signals(leaves)
        .into_iter()
        .map(|(changes, warns)| (PendingValues::Decoded(Arc::new(changes)), warns))
        .collect()
}

/// Serve on-demand value requests for an open source until the UI drops the
/// job (the request channel closes) or asks for shutdown.
///
/// Value reads must happen on this thread - FFR cannot be used from several
/// threads (two simultaneous readers deadlock inside the library) - so the
/// reads of one batch stay back to back while the compute pool installs the
/// previous batch, clones the values for the UI and completes the aggregate
/// math in parallel.
fn serve_session(
    meta: Arc<std::sync::Mutex<Waveform>>,
    session: &mut dyn crate::dump::source::DumpSource,
    req_rx: &Receiver<LoadRequest>,
    tx: &Sender<LoadEvent>,
) {
    let (task_tx, task_rx) = mpsc::channel::<ComputeTask>();
    let task_rx = Arc::new(std::sync::Mutex::new(task_rx));
    // Signals the UI asked for. Checked by the pool at install time, so a
    // request that arrives while its read is already in flight is honoured.
    let wanted: Arc<std::sync::Mutex<std::collections::HashSet<usize>>> = Arc::default();
    for _ in 0..pool_size() {
        let rx = Arc::clone(&task_rx);
        let meta = Arc::clone(&meta);
        let tx = tx.clone();
        let wanted = Arc::clone(&wanted);
        std::thread::spawn(move || loop {
            let task = rx.lock().unwrap_or_else(|err| err.into_inner()).recv();
            let Ok(task) = task else {
                break;
            };
            match task {
                ComputeTask::Install(pending) => {
                    // Install first so the aggregates below can see the new
                    // values, then clone once for the UI - and only for the
                    // signals the UI actually asked for. The wanted set is
                    // checked here (not when the read started) so a signal
                    // requested while its read was in flight still arrives.
                    let updates = {
                        let mut wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                        let mut updates = Vec::with_capacity(pending.len());
                        let mut warnings = Vec::new();
                        for (leaf, values, mut warns) in pending {
                            let wanted = wanted
                                .lock()
                                .unwrap_or_else(|err| err.into_inner())
                                .contains(&leaf);
                            // Raw bytes come straight out of FFR; decode them
                            // here on the pool, not on the reader thread.
                            #[cfg(fsdb_sdk)]
                            let changes = match values {
                                PendingValues::Decoded(changes) => changes,
                                PendingValues::Raw(raw) => {
                                    let name = wf
                                        .signals
                                        .get(leaf)
                                        .map(|signal| signal.name.clone())
                                        .unwrap_or_default();
                                    Arc::new(crate::fsdb::decode_raw(raw, &name, &mut warns))
                                }
                            };
                            #[cfg(not(fsdb_sdk))]
                            let PendingValues::Decoded(changes) = values;
                            if let Some(signal) = wf.signals.get_mut(leaf) {
                                signal.changes = changes;
                                signal.state = crate::waveform::SigState::Ready;
                                if wanted {
                                    // The UI and the worker now share this
                                    // allocation; only the refcount travels.
                                    updates.push((leaf, Arc::clone(&signal.changes)));
                                }
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
                // Phase A claims every leaf that still has to be read and
                // forwards the ones already in the shared waveform; no read
                // happens before the whole batch is collected, so FFR sees
                // one pass instead of one per signal.
                let mut claimed: Vec<(usize, usize)> = Vec::new();
                let mut delivered = std::collections::HashSet::new();
                for &index in &batch {
                    {
                        let mut wanted = wanted.lock().unwrap_or_else(|err| err.into_inner());
                        wanted.insert(index);
                    }
                    let leaves = {
                        let wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                        if index >= wf.signals.len() {
                            continue;
                        }
                        wf.value_leaves(index)
                    };
                    for leaf in leaves {
                        // Synthesized brace values are joined from the
                        // members at draw time, so the UI needs every leaf of
                        // a requested aggregate, not just the aggregate.
                        wanted
                            .lock()
                            .unwrap_or_else(|err| err.into_inner())
                            .insert(leaf);
                        // Claim the leaf before reading it: the install runs
                        // on the pool, so the next request must not read it
                        // again while it is in flight.
                        let claimed_leaf = {
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
                        if claimed_leaf {
                            claimed.push((index, leaf));
                            continue;
                        }
                        // Already loaded for an earlier aggregate: the UI may
                        // still need its own copy now that it asks for it.
                        let existing = {
                            let wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                            (wf.signals[leaf].state == crate::waveform::SigState::Ready
                                && wanted
                                    .lock()
                                    .unwrap_or_else(|err| err.into_inner())
                                    .contains(&leaf))
                            .then(|| wf.signals[leaf].changes.clone())
                        };
                        if let Some(changes) = existing {
                            if leaf == index {
                                delivered.insert(index);
                            }
                            raw.push((leaf, PendingValues::Decoded(changes), Vec::new()));
                        }
                    }
                }
                // Phase B reads every claimed leaf back to back in one FFR
                // load pass, in request order. FSDB hands over raw bytes for
                // the pool to decode; other lazy sources decode inline.
                let leaves: Vec<usize> = claimed.iter().map(|&(_, leaf)| leaf).collect();
                #[cfg(fsdb_sdk)]
                let reads = match session.read_signals_raw(&leaves) {
                    Some(reads) => reads
                        .into_iter()
                        .map(|(raw, warns)| (PendingValues::Raw(raw), warns))
                        .collect(),
                    None => decode_batch(session, &leaves),
                };
                #[cfg(not(fsdb_sdk))]
                let reads = decode_batch(session, &leaves);
                for (&(requested, leaf), (values, warns)) in claimed.iter().zip(reads) {
                    if leaf == requested {
                        delivered.insert(requested);
                    }
                    raw.push((leaf, values, warns));
                }
                // A requested signal that is already complete in the shared
                // waveform (e.g. an interface whose members were read earlier)
                // still has to reach the UI. A synthesized signal has no
                // stored changes, but its readiness must still be delivered.
                for &index in &batch {
                    if delivered.contains(&index) {
                        continue;
                    }
                    let existing = {
                        let wf = meta.lock().unwrap_or_else(|err| err.into_inner());
                        wf.signals.get(index).and_then(|signal| {
                            (signal.state == crate::waveform::SigState::Ready
                                && wanted
                                    .lock()
                                    .unwrap_or_else(|err| err.into_inner())
                                    .contains(&index))
                            .then(|| signal.changes.clone())
                        })
                    };
                    if let Some(changes) = existing {
                        raw.push((index, PendingValues::Decoded(changes), Vec::new()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::{Change, ScopeTree, SigKind, SigState, Signal, TimeScale, Value};
    use std::sync::{Arc, Mutex};

    /// Lazy source with canned values, so the install path can be exercised
    /// without opening a dump file.
    struct FakeSource {
        values: Vec<Vec<Change>>,
    }

    impl crate::dump::source::DumpSource for FakeSource {
        fn var_count(&self) -> usize {
            self.values.len()
        }

        fn take_hierarchy(&mut self) -> Waveform {
            unreachable!("the session installs into the waveform it was given")
        }

        fn read_signal(&mut self, index: usize) -> (Vec<Change>, Vec<String>) {
            (
                self.values.get(index).cloned().unwrap_or_default(),
                Vec::new(),
            )
        }
    }

    fn lazy_signal(name: &str) -> Signal {
        Signal {
            name: name.to_string(),
            bits: 1,
            var_type: "wire".to_string(),
            dir: String::new(),
            scope: Vec::new(),
            kind: SigKind::Bits,
            changes: Arc::new(Vec::new()),
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            parent: None,
            members: Vec::new(),
            state: SigState::Lazy,
        }
    }

    /// A freshly decoded signal is installed into the shared waveform and the
    /// very same allocation travels to the UI: the update event and the
    /// worker's signal point at one `Arc`, so no values are copied.
    #[test]
    fn installed_changes_share_one_allocation() {
        let wf = Waveform {
            ts: TimeScale::default(),
            start: 0,
            end: 10,
            signals: vec![lazy_signal("clk")],
            tree: ScopeTree::new(),
            radix: std::collections::HashMap::new(),
            value_times_cache: Vec::new(),
        };
        let meta = Arc::new(Mutex::new(wf));
        let (req_tx, req_rx) = mpsc::channel();
        req_tx.send(LoadRequest::Signal(0)).expect("queue request");
        drop(req_tx);
        let (tx, rx) = mpsc::channel();
        let mut source = FakeSource {
            values: vec![vec![Change {
                t: 0,
                v: Value::Small(1, 1),
            }]],
        };
        serve_session(Arc::clone(&meta), &mut source, &req_rx, &tx);
        drop(tx);
        let mut seen = 0usize;
        for event in rx {
            let LoadEvent::Changes(updates, _) = event else {
                continue;
            };
            for (index, changes) in updates {
                let meta = meta.lock().unwrap();
                let signal = &meta.signals[index];
                assert_eq!(*changes, *signal.changes);
                assert!(
                    Arc::ptr_eq(&changes, &signal.changes),
                    "signal {index} was copied instead of shared"
                );
                seen += 1;
            }
        }
        assert_eq!(seen, 1, "the requested signal reached the UI once");
    }

    /// A batch of requests is read back to back and installed by the pool:
    /// every requested signal becomes Ready and its values reach the UI.
    #[test]
    #[cfg(fsdb_sdk)]
    fn batched_requests_install_their_signals() {
        let Some(home) = std::env::var_os("VERDI_HOME") else {
            return;
        };
        let path =
            std::path::PathBuf::from(home).join("demo/nCompare/nCmp_demo1/demo_RTL_verilog.fsdb");
        if !path.is_file() {
            return;
        }
        let mut source =
            crate::dump::source::open_source(&path, &mut |_| true).expect("lazy parse");
        let out = crate::dump::ParseOut {
            wf: source.take_hierarchy(),
            warnings: source.take_warnings(),
        };
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
        serve_session(Arc::clone(&meta), &mut *source, &req_rx, &tx);
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

    /// Requesting an aggregate must load and deliver every leaf of its
    /// subtree, whatever the request order: the leaves carry the values, the
    /// synthesized nodes only join them.
    #[test]
    fn requested_aggregates_deliver_every_leaf() {
        let mut signals = vec![lazy_signal("a"), lazy_signal("b"), lazy_signal("c")];
        let mut array = lazy_signal("arr");
        array.var_type = "array".to_string();
        array.members = vec![0, 1];
        signals.push(array);
        let mut aggregate = lazy_signal("agg");
        aggregate.var_type = "aggregate".to_string();
        aggregate.members = vec![2, 3];
        signals.push(aggregate);
        signals[0].parent = Some(3);
        signals[1].parent = Some(3);
        let wf = Waveform {
            ts: TimeScale::default(),
            start: 0,
            end: 10,
            signals,
            tree: ScopeTree::new(),
            radix: std::collections::HashMap::new(),
            value_times_cache: Vec::new(),
        };
        let values = |t: u64, v: u8| {
            vec![Change {
                t,
                v: Value::compact(vec![v]),
            }]
        };
        let meta = Arc::new(Mutex::new(wf));
        let (req_tx, req_rx) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        for index in [4usize, 0, 2, 3, 1] {
            req_tx.send(LoadRequest::Signal(index)).expect("queue");
        }
        drop(req_tx);
        let mut source = FakeSource {
            values: vec![values(0, 1), values(1, 0), values(2, 1)],
        };
        serve_session(Arc::clone(&meta), &mut source, &req_rx, &tx);
        drop(tx);
        let mut installed = std::collections::HashSet::new();
        for event in rx {
            if let LoadEvent::Changes(updates, _) = event {
                installed.extend(updates.into_iter().map(|(index, _)| index));
            }
        }
        for index in 0..3 {
            assert!(installed.contains(&index), "leaf {index} was not delivered");
            let wf = meta.lock().unwrap();
            assert_eq!(wf.signals[index].state, SigState::Ready);
            assert!(!wf.signals[index].changes.is_empty());
        }
        for index in 3..5 {
            assert_eq!(meta.lock().unwrap().signals[index].state, SigState::Ready);
        }
    }
}
