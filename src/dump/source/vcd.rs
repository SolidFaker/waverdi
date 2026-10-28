use super::{DumpSource, ReadResult};
use crate::waveform::{Change, Waveform};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

/// Lazy VCD source. Each signal is scanned at most once and its final changes
/// stay behind an arc in the per-signal cache.
pub(super) struct VcdSource {
    path: std::path::PathBuf,
    hierarchy: Option<Waveform>,
    warnings: Vec<String>,
    scan: crate::vcd::VcdScanState,
    vars: usize,
    budget: u64,
    changes: HashMap<usize, Arc<Vec<Change>>>,
    scanned: HashSet<usize>,
    reported: HashSet<String>,
}

impl VcdSource {
    pub(super) fn open(path: &Path) -> Result<Self, String> {
        let crate::vcd::VcdHierarchy { out, scan } = crate::vcd::read_vcd_hierarchy(path)?;
        let vars = out.wf.signals.len();
        Ok(Self {
            path: path.to_path_buf(),
            hierarchy: Some(out.wf),
            warnings: out.warnings,
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

    fn read_signal(&mut self, index: usize) -> ReadResult {
        self.read_signals(&[index])
            .pop()
            .unwrap_or_else(|| (Arc::new(Vec::new()), Vec::new()))
    }

    fn read_signals(&mut self, indexes: &[usize]) -> Vec<ReadResult> {
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
                        self.changes.insert(index, Arc::new(limited));
                        self.scanned.insert(index);
                    }
                    for warning in warnings {
                        if self.reported.insert(warning.clone()) {
                            globals.push(warning);
                        }
                    }
                }
                Err(err) => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::SigState;
    use std::path::PathBuf;

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

    fn temp_dump(name: &str, data: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("waverdi_{name}_{}.vcd", std::process::id()));
        std::fs::write(&path, data).expect("write temp dump");
        path
    }

    #[test]
    fn lazy_vcd_matches_eager_and_rescans_for_new_signals() {
        let path = temp_dump("lazy_vcd", LAZY_VCD.as_bytes());
        let eager = crate::vcd::parse_vcd(&path).expect("eager parse");
        let mut source = super::super::open_source(&path, &mut |_| true).expect("lazy open");

        assert!(source.is_lazy());
        assert_eq!(source.var_count(), eager.wf.signals.len());
        let wf = source.take_hierarchy();
        assert!(wf.signals.iter().all(|s| s.state == SigState::Lazy));
        assert!(wf.signals.iter().all(|s| s.changes.is_empty()));
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

        let again = source.read_signals(&[0]);
        assert_eq!(
            again[0].0.as_slice(),
            eager.wf.signals[0].changes.as_slice()
        );
        assert!(Arc::ptr_eq(&first[0].0, &again[0].0));

        let _ = std::fs::remove_file(&path);
    }
}
