//! Format adapter used by the background loader.
//!
//! Every dump format is opened through [`DumpSource`]: the caller gets the
//! hierarchy first and pulls value changes per signal, so the loader, the
//! compute pool and the UI do not need to know which format is behind it.
//!
//! Laziness lives here too, on a spectrum:
//! - formats with random access (FSDB through the Verdi reader) read a signal
//!   from the file the moment it is requested ([`FsdbSource`]);
//! - FST decodes its value data once, on the first request, and serves later
//!   batches from a per-signal cache ([`FstSource`]);
//! - VCD only parses declarations up front and rescans its value section for
//!   signals that were not collected yet ([`VcdSource`]);
//! - formats that cannot be read lazily keep the eager path ([`EagerSource`]).
//!
//! Either way the loader sees the same interface: hierarchy, then values on
//! demand, with the shared change budget and decimation applied by the
//! parsers (`crate::dump::limit_changes`).

use crate::waveform::{Change, Waveform};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

/// What a dump format can provide; the UI uses this to gate features
/// uniformly instead of guessing per format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Values arrive on request instead of with the hierarchy.
    pub lazy: bool,
    /// Signals carry port directions (FSDB does, VCD/FST do not).
    pub directions: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        // Sources that do not describe themselves are assumed to be lazy
        // (the safe pipeline) and to have no direction information.
        Self {
            lazy: true,
            directions: false,
        }
    }
}

/// One dump format as seen by the loader. Reads are serialized by the loader
/// (FFR cannot be used from two threads), so implementations may assume
/// `read_signal` is called from a single thread; the trait is deliberately
/// not `Send` so a source cannot be moved between reader threads.
pub trait DumpSource {
    /// Whether values still have to be fetched from the source on request.
    /// Eager sources deliver their values with the hierarchy and never need
    /// the request pipeline (there is nothing to gain and a clone to lose).
    fn is_lazy(&self) -> bool {
        true
    }
    /// What this source can provide; the UI gates format-specific features on
    /// it (e.g. it hides the direction filters when `directions` is false)
    /// instead of probing the dump format itself.
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            lazy: self.is_lazy(),
            directions: false,
        }
    }
    /// Number of variables the source can serve.
    fn var_count(&self) -> usize;
    /// Take the parsed hierarchy (signals may be lazy). Called once.
    fn take_hierarchy(&mut self) -> Waveform;
    /// Take parser warnings collected while building the hierarchy. Called once.
    fn take_warnings(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// Read one signal's value changes on demand.
    fn read_signal(&mut self, index: usize) -> (Vec<Change>, Vec<String>);
    /// Read several signals in one pass; the results are in the input order.
    fn read_signals(&mut self, indexes: &[usize]) -> Vec<(Vec<Change>, Vec<String>)> {
        indexes
            .iter()
            .map(|&index| self.read_signal(index))
            .collect()
    }
    /// Read several signals without decoding their values, for formats that
    /// can hand over raw bytes (FSDB). The loader decodes them on its compute
    /// pool instead of on the single reader thread. `None` means the source
    /// has no raw path and [`Self::read_signals`] must be used.
    #[cfg(fsdb_sdk)]
    fn read_signals_raw(
        &mut self,
        _indexes: &[usize],
    ) -> Option<Vec<(crate::fsdb::RawChanges, Vec<String>)>> {
        None
    }
}

/// Lazy FSDB source backed by an open Verdi FFR session.
#[cfg(fsdb_sdk)]
struct FsdbSource {
    hierarchy: Option<Waveform>,
    warnings: Vec<String>,
    session: crate::fsdb::FsdbSession,
}

#[cfg(fsdb_sdk)]
impl DumpSource for FsdbSource {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            lazy: true,
            // Directions are only claimed when the session really recorded
            // some; an FSDB without them must not show empty port filters.
            directions: self.session.has_directions(),
        }
    }

    fn var_count(&self) -> usize {
        self.session.var_count()
    }

    fn take_hierarchy(&mut self) -> Waveform {
        self.hierarchy.take().expect("hierarchy is taken once")
    }

    fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    fn read_signal(&mut self, index: usize) -> (Vec<Change>, Vec<String>) {
        self.session.read_signal(index)
    }

    fn read_signals(&mut self, indexes: &[usize]) -> Vec<(Vec<Change>, Vec<String>)> {
        self.session.read_signals(indexes)
    }

    fn read_signals_raw(
        &mut self,
        indexes: &[usize],
    ) -> Option<Vec<(crate::fsdb::RawChanges, Vec<String>)>> {
        Some(self.session.read_signals_raw(indexes))
    }
}

/// Lazy FST source. The hierarchy pass reads only the header, so the tree is
/// there immediately; the first value request decodes the whole value section
/// (wellen filters by streaming the body anyway) and later batches are clones
/// from the cache, so expanding more signals never re-reads the file.
struct FstSource {
    hierarchy: Option<Waveform>,
    warnings: Vec<String>,
    values: crate::fst::FstValues,
    /// wellen reference per signal index, aligned with the hierarchy.
    refs: Vec<wellen::SignalRef>,
    /// Signal names in index order, for the limit warnings.
    names: Vec<String>,
    changes: Mutex<Option<Vec<Vec<Change>>>>,
}

impl FstSource {
    fn open(path: &Path) -> Result<Self, String> {
        let crate::fst::FstHierarchy { out, refs, values } = crate::fst::parse_fst_hierarchy(path)?;
        let names = out
            .wf
            .signals
            .iter()
            .map(|signal| signal.name.clone())
            .collect();
        let super::ParseOut { wf, warnings } = out;
        Ok(Self {
            hierarchy: Some(wf),
            warnings,
            values,
            refs,
            names,
            changes: Mutex::new(None),
        })
    }
}

impl DumpSource for FstSource {
    fn var_count(&self) -> usize {
        // The hierarchy is handed to the loader on `take_hierarchy`, so the
        // count must not depend on it still being here.
        self.refs.len()
    }

    fn take_hierarchy(&mut self) -> Waveform {
        self.hierarchy.take().expect("hierarchy is taken once")
    }

    fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    fn read_signal(&mut self, index: usize) -> (Vec<Change>, Vec<String>) {
        self.read_signals(&[index])
            .pop()
            .unwrap_or_else(|| (Vec::new(), Vec::new()))
    }

    fn read_signals(&mut self, indexes: &[usize]) -> Vec<(Vec<Change>, Vec<String>)> {
        let mut decode_warnings = Vec::new();
        let mut cache = self.changes.lock().unwrap_or_else(|err| err.into_inner());
        if cache.is_none() {
            let (changes, warnings) = decode_fst_all(&mut self.values, &self.refs, &self.names);
            *cache = Some(changes);
            decode_warnings = warnings;
        }
        let cache = cache.as_ref().expect("decoded above");
        indexes
            .iter()
            .enumerate()
            .map(|(slot, &index)| {
                let changes = cache.get(index).cloned().unwrap_or_default();
                let warnings = if slot == 0 {
                    std::mem::take(&mut decode_warnings)
                } else {
                    Vec::new()
                };
                (changes, warnings)
            })
            .collect()
    }
}

/// Decode every signal in one wellen pass and bound the results with the
/// same limits the eager parse applies, in hierarchy order.
fn decode_fst_all(
    values: &mut crate::fst::FstValues,
    refs: &[wellen::SignalRef],
    names: &[String],
) -> (Vec<Vec<Change>>, Vec<String>) {
    let loaded = values.load(refs);
    let mut warnings = Vec::new();
    let mut budget = crate::dump::MAX_TOTAL_CHANGES;
    let mut changes = Vec::with_capacity(refs.len());
    for (index, signal_ref) in refs.iter().enumerate() {
        let decoded = loaded
            .get(signal_ref)
            .map(|signal| crate::fst::materialize(signal, values.time_table()))
            .unwrap_or_default();
        let name = names.get(index).map(String::as_str).unwrap_or("");
        changes.push(crate::dump::limit_changes(
            name,
            decoded,
            crate::dump::MAX_CHANGES_PER_SIGNAL,
            &mut budget,
            &mut warnings,
        ));
    }
    (changes, warnings)
}

/// Lazy VCD source. The declaration pass builds the hierarchy; the value
/// section is streamed on request and the changes are cached per signal.
/// VCD has no random access, so a batch that mentions a signal which has not
/// been scanned yet rescans the value section for it. That is bounded by how
/// often the user expands new signals: every signal is scanned at most once.
struct VcdSource {
    path: std::path::PathBuf,
    hierarchy: Option<Waveform>,
    warnings: Vec<String>,
    scan: crate::vcd::VcdScanState,
    /// Signal count kept after the hierarchy moves out on `take_hierarchy`.
    vars: usize,
    budget: u64,
    /// Changes collected so far, keyed by signal index.
    changes: HashMap<usize, Vec<Change>>,
    /// Signals whose entry in `changes` is final (an empty list means the
    /// dump has no values for them), so a rescan never revisits them.
    scanned: HashSet<usize>,
    /// File-level warnings already handed to the loader, so rescans do not
    /// repeat them.
    reported: HashSet<String>,
}

impl VcdSource {
    fn open(path: &Path) -> Result<Self, String> {
        let crate::vcd::VcdHierarchy { out, scan } = crate::vcd::read_vcd_hierarchy(path)?;
        let vars = out.wf.signals.len();
        let super::ParseOut { wf, warnings } = out;
        Ok(Self {
            path: path.to_path_buf(),
            hierarchy: Some(wf),
            warnings,
            scan,
            vars,
            budget: crate::dump::MAX_TOTAL_CHANGES,
            changes: HashMap::new(),
            scanned: HashSet::new(),
            reported: HashSet::new(),
        })
    }
}

impl DumpSource for VcdSource {
    fn var_count(&self) -> usize {
        self.vars
    }

    fn take_hierarchy(&mut self) -> Waveform {
        self.hierarchy.take().expect("hierarchy is taken once")
    }

    fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    fn read_signal(&mut self, index: usize) -> (Vec<Change>, Vec<String>) {
        self.read_signals(&[index])
            .pop()
            .unwrap_or_else(|| (Vec::new(), Vec::new()))
    }

    fn read_signals(&mut self, indexes: &[usize]) -> Vec<(Vec<Change>, Vec<String>)> {
        // Unknown indices have nothing to scan; cache them as empty so a
        // repeated request cannot spin through rescans forever.
        for &index in indexes {
            if index >= self.scan.names.len() && self.scanned.insert(index) {
                self.changes.entry(index).or_default();
            }
        }
        let missing: Vec<usize> = indexes
            .iter()
            .copied()
            .filter(|index| !self.scanned.contains(index))
            .collect();
        let mut globals = Vec::new();
        let mut per_signal: HashMap<usize, Vec<String>> = HashMap::new();
        if !missing.is_empty() {
            match crate::vcd::scan_vcd_values(&self.path, &self.scan, &missing) {
                Ok(out) => {
                    let crate::vcd::VcdScanOut {
                        changes,
                        mut signal_warnings,
                        warnings,
                    } = out;
                    for (index, raw) in changes {
                        let name = self.scan.names.get(index).map(String::as_str).unwrap_or("");
                        let mut warns = signal_warnings.remove(&index).unwrap_or_default();
                        let limited = crate::dump::limit_changes(
                            name,
                            raw,
                            crate::dump::MAX_CHANGES_PER_SIGNAL,
                            &mut self.budget,
                            &mut warns,
                        );
                        if !warns.is_empty() {
                            per_signal.insert(index, warns);
                        }
                        self.changes.insert(index, limited);
                        self.scanned.insert(index);
                    }
                    for warning in warnings {
                        if self.reported.insert(warning.clone()) {
                            globals.push(warning);
                        }
                    }
                }
                Err(err) => {
                    // The scan failed; the signals stay unscanned so a later
                    // request can retry, and the message is reported once.
                    if self.reported.insert(err.clone()) {
                        globals.push(err);
                    }
                }
            }
        }
        let mut first = true;
        indexes
            .iter()
            .map(|&index| {
                let changes = self.changes.get(&index).cloned().unwrap_or_default();
                let mut warns = per_signal.remove(&index).unwrap_or_default();
                if first && !globals.is_empty() {
                    warns.append(&mut globals);
                }
                first = false;
                (changes, warns)
            })
            .collect()
    }
}

/// Eager source: dumps the adapter cannot read lazily are parsed up front,
/// values and all. They do not use the request pipeline (`is_lazy` is false),
/// so the UI gets the complete waveform in one event exactly like before the
/// adapter layer existed - a second copy of every value would only cost time
/// and memory.
struct EagerSource {
    hierarchy: Option<Waveform>,
    warnings: Vec<String>,
}

impl DumpSource for EagerSource {
    fn is_lazy(&self) -> bool {
        false
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            lazy: false,
            directions: false,
        }
    }

    fn var_count(&self) -> usize {
        self.hierarchy
            .as_ref()
            .map(|wf| wf.signals.len())
            .unwrap_or_default()
    }

    fn take_hierarchy(&mut self) -> Waveform {
        self.hierarchy.take().expect("hierarchy is taken once")
    }

    fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    fn read_signal(&mut self, _index: usize) -> (Vec<Change>, Vec<String>) {
        // The loader never requests values from an eager source.
        (Vec::new(), Vec::new())
    }
}

/// Open `path` as a [`DumpSource`]: FSDB is read lazily through the Verdi FFR
/// session, FST reads its header first and decodes values on the first read,
/// VCD parses declarations only and scans values on demand. Anything else
/// (unknown extensions, compressed variants) keeps the eager parse.
pub(crate) fn open_source(
    path: &Path,
    progress: crate::dump::Progress,
) -> Result<Box<dyn DumpSource>, String> {
    #[cfg(fsdb_sdk)]
    if super::detect(path) == super::Format::Fsdb {
        let (out, session) = crate::fsdb::parse_fsdb_lazy(path, progress)?;
        let super::ParseOut { wf, warnings } = out;
        return Ok(Box::new(FsdbSource {
            hierarchy: Some(wf),
            warnings,
            session,
        }));
    }
    match super::detect(path) {
        super::Format::Fst => Ok(Box::new(FstSource::open(path)?)),
        super::Format::Vcd => Ok(Box::new(VcdSource::open(path)?)),
        // Unknown extensions - and FSDB without the Verdi SDK - report the
        // same error they reported before the lazy sources existed.
        _ => {
            let out = super::parse_with_progress(path, progress)?;
            Ok(Box::new(EagerSource {
                hierarchy: Some(out.wf),
                warnings: out.warnings,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dump::ParseOut;
    use crate::waveform::SigState;
    use std::path::{Path, PathBuf};

    /// A VCD with two scopes, a bus, a real and a string signal, plus a
    /// `$dumpvars` block that the lazy scan must treat like the eager parse.
    const LAZY_VCD: &str = r#"$timescale 1ns $end
$scope module top $end
$var wire 1 ! clk $end
$var reg 4 " data [3:0] $end
$scope module u_dut $end
$var real 64 # temp $end
$var string 8 $ tag $end
$upscope $end
$upscope $end
$enddefinitions $end
$dumpvars
0!
b0000 "
r1.5 #
s"init" $
$end
#10
1!
b1010 "
r2.5 #
s"run" $
#20
0!
b0101 "
"#;

    /// Write a dump for one test and return its path. The name carries the
    /// process id so parallel test binaries cannot collide.
    fn temp_dump(name: &str, data: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("waverdi_{name}_{}.vcd", std::process::id()));
        std::fs::write(&path, data).expect("write temp dump");
        path
    }

    /// The eager adapter hands the parsed dump over whole: it is not lazy, so
    /// the values stay in the hierarchy and the loader skips the request
    /// pipeline for it.
    #[test]
    fn eager_source_keeps_its_values_in_the_hierarchy() {
        let path = Path::new("waveform/counter.vcd");
        let ParseOut { wf, warnings } =
            super::super::parse_with_progress(path, &mut |_| true).expect("counter.vcd parses");
        let signals = wf.signals.len();
        let mut source = EagerSource {
            hierarchy: Some(wf),
            warnings,
        };

        assert!(!source.is_lazy());
        assert_eq!(
            source.capabilities(),
            Capabilities {
                lazy: false,
                directions: false,
            }
        );
        assert_eq!(source.var_count(), signals);
        let hierarchy = source.take_hierarchy();
        assert_eq!(hierarchy.signals.len(), signals);
        assert!(hierarchy
            .signals
            .iter()
            .any(|signal| !signal.changes.is_empty()));
    }

    /// The lazy VCD source must agree with the eager parser on the hierarchy,
    /// and its per-batch values - including a batch that forces a rescan for
    /// a signal that was not scanned before - must be identical.
    #[test]
    fn lazy_vcd_matches_eager_and_rescans_for_new_signals() {
        let path = temp_dump("lazy_vcd", LAZY_VCD.as_bytes());
        let eager = crate::vcd::parse_vcd(&path).expect("eager parse");
        let mut source = super::open_source(&path, &mut |_| true).expect("lazy open");

        assert!(source.is_lazy());
        assert_eq!(
            source.capabilities(),
            Capabilities {
                lazy: true,
                directions: false,
            }
        );
        assert_eq!(source.var_count(), eager.wf.signals.len());

        let wf = source.take_hierarchy();
        assert!(wf.signals.iter().all(|s| s.state == SigState::Lazy));
        assert!(wf.signals.iter().all(|s| s.changes.is_empty()));
        // The lazy source keeps serving after the hierarchy has moved out.
        assert_eq!(source.var_count(), wf.signals.len());
        assert_eq!(wf.start, eager.wf.start);
        assert_eq!(wf.end, eager.wf.end);
        assert_eq!(wf.ts.label(), eager.wf.ts.label());
        for (lazy, eager) in wf.signals.iter().zip(&eager.wf.signals) {
            assert_eq!(lazy.name, eager.name);
            assert_eq!(lazy.bits, eager.bits);
            assert_eq!(lazy.kind, eager.kind);
            assert_eq!(lazy.scope, eager.scope);
            assert_eq!(lazy.var_type, eager.var_type);
        }
        assert_eq!(wf.tree.nodes.len(), eager.wf.tree.nodes.len());
        for (lazy, eager) in wf.tree.nodes.iter().zip(&eager.wf.tree.nodes) {
            assert_eq!(lazy.name, eager.name);
            assert_eq!(lazy.children, eager.children);
            assert_eq!(lazy.signals, eager.signals);
        }

        // First batch: both signals match the eager parse and carry no
        // warnings for a well-formed dump.
        let first = source.read_signals(&[0, 1]);
        assert_eq!(first.len(), 2);
        assert_eq!(
            first[0].0.as_slice(),
            eager.wf.signals[0].changes.as_slice()
        );
        assert_eq!(
            first[1].0.as_slice(),
            eager.wf.signals[1].changes.as_slice()
        );
        assert!(first.iter().all(|(_, warns)| warns.is_empty()), "{first:?}");

        // Second batch asks for signals that were not scanned yet: the source
        // rescans the value section and returns the eager changes for them.
        let second = source.read_signals(&[2, 3]);
        assert_eq!(second.len(), 2);
        assert_eq!(
            second[0].0.as_slice(),
            eager.wf.signals[2].changes.as_slice()
        );
        assert_eq!(
            second[1].0.as_slice(),
            eager.wf.signals[3].changes.as_slice()
        );

        // A signal from the first batch is served from the cache unchanged.
        let again = source.read_signals(&[0]);
        assert_eq!(
            again[0].0.as_slice(),
            eager.wf.signals[0].changes.as_slice()
        );

        let _ = std::fs::remove_file(&path);
    }

    /// The lazy FST source reads the hierarchy without values, then serves
    /// value batches that match the eager parse exactly.
    #[test]
    fn lazy_fst_matches_eager_and_serves_values_on_demand() {
        let path = Path::new("waveform/demo.fst");
        let eager = crate::fst::parse_fst(path).expect("eager parse");
        let mut source = super::open_source(path, &mut |_| true).expect("lazy open");

        assert!(source.is_lazy());
        assert_eq!(
            source.capabilities(),
            Capabilities {
                lazy: true,
                directions: false,
            }
        );
        assert_eq!(source.var_count(), eager.wf.signals.len());

        let wf = source.take_hierarchy();
        assert!(wf.signals.iter().all(|s| s.state == SigState::Lazy));
        assert!(wf.signals.iter().all(|s| s.changes.is_empty()));
        // The lazy source keeps serving after the hierarchy has moved out.
        assert_eq!(source.var_count(), wf.signals.len());
        assert_eq!(wf.start, eager.wf.start);
        assert_eq!(wf.end, eager.wf.end);
        assert_eq!(wf.ts.label(), eager.wf.ts.label());
        for (lazy, eager) in wf.signals.iter().zip(&eager.wf.signals) {
            assert_eq!(lazy.name, eager.name);
            assert_eq!(lazy.bits, eager.bits);
            assert_eq!(lazy.kind, eager.kind);
            assert_eq!(lazy.scope, eager.scope);
            assert_eq!(lazy.var_type, eager.var_type);
        }
        assert_eq!(wf.tree.nodes.len(), eager.wf.tree.nodes.len());
        for (lazy, eager) in wf.tree.nodes.iter().zip(&eager.wf.tree.nodes) {
            assert_eq!(lazy.name, eager.name);
            assert_eq!(lazy.children, eager.children);
            assert_eq!(lazy.signals, eager.signals);
        }
        assert!(source.take_warnings().is_empty());

        // The first read decodes the file once; later batches are clones.
        let first = source.read_signals(&[0, 2]);
        assert_eq!(first[0].0, *eager.wf.signals[0].changes);
        assert_eq!(first[1].0, *eager.wf.signals[2].changes);
        let second = source.read_signals(&[4]);
        assert_eq!(second[0].0, *eager.wf.signals[4].changes);
    }
}
