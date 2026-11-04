use super::{DumpSource, ReadResult};
use crate::waveform::{Change, Waveform};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

struct FstCache {
    by_index: HashMap<usize, Arc<Vec<Change>>>,
    by_ref: HashMap<wellen::SignalRef, (Arc<Vec<Change>>, usize)>,
    budget: u64,
    warned_total: bool,
}

/// Lazy FST source. Only requested signals and their aliases are decoded;
/// later requests clone the same allocation instead of re-reading the file.
pub(super) struct FstSource {
    hierarchy: Option<Waveform>,
    warnings: Vec<String>,
    values: crate::fst::FstValues,
    refs: Vec<wellen::SignalRef>,
    names: Vec<String>,
    cache: Mutex<FstCache>,
}

impl FstSource {
    pub(super) fn open(path: &Path) -> Result<Self, String> {
        let crate::fst::FstHierarchy { out, refs, values } = crate::fst::parse_fst_hierarchy(path)?;
        let names = out
            .wf
            .signals
            .iter()
            .map(|signal| signal.name.clone())
            .collect();
        Ok(Self {
            hierarchy: Some(out.wf),
            warnings: out.warnings,
            values,
            refs,
            names,
            cache: Mutex::new(FstCache {
                by_index: HashMap::new(),
                by_ref: HashMap::new(),
                budget: crate::dump::MAX_TOTAL_CHANGES,
                warned_total: false,
            }),
        })
    }
}

impl DumpSource for FstSource {
    fn var_count(&self) -> usize {
        self.refs.len()
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
        let mut cache = self.cache.lock().unwrap_or_else(|err| err.into_inner());
        let unique: Vec<wellen::SignalRef> = {
            let mut seen = std::collections::HashSet::new();
            indexes
                .iter()
                .filter_map(|&index| self.refs.get(index).copied())
                .filter(|signal_ref| !cache.by_ref.contains_key(signal_ref))
                .filter(|signal_ref| seen.insert(*signal_ref))
                .collect()
        };
        let loaded = self.values.load(&unique);
        let mut decoded = HashMap::new();
        for signal_ref in unique {
            let changes = loaded
                .get(&signal_ref)
                .map(|signal| crate::fst::materialize(signal, self.values.time_table()))
                .unwrap_or_default();
            decoded.insert(signal_ref, changes);
        }

        indexes
            .iter()
            .enumerate()
            .map(|(slot, &index)| {
                let mut warnings = Vec::new();
                if let Some(cached) = cache.by_index.get(&index) {
                    return (Arc::clone(cached), warnings);
                }
                let Some(signal_ref) = self.refs.get(index).copied() else {
                    return (Arc::new(Vec::new()), warnings);
                };
                let name = self.names.get(index).map(String::as_str).unwrap_or("");
                if let Some((changes, original_len)) = cache.by_ref.get(&signal_ref) {
                    let changes = Arc::clone(changes);
                    charge_alias(
                        name,
                        *original_len,
                        changes.len(),
                        &mut cache,
                        &mut warnings,
                    );
                    cache.by_index.insert(index, Arc::clone(&changes));
                    return (changes, warnings);
                }
                let raw = decoded.remove(&signal_ref).unwrap_or_default();
                let original_len = raw.len();
                let changes = Arc::new(crate::dump::limit_changes(
                    name,
                    raw,
                    crate::dump::MAX_CHANGES_PER_SIGNAL,
                    &mut cache.budget,
                    &mut warnings,
                ));
                if cache.budget == 0 && !cache.warned_total {
                    cache.warned_total = true;
                    warnings.push(format!(
                        "value changes are limited to {} in total; remaining signals are loaded without values",
                        crate::dump::MAX_TOTAL_CHANGES
                    ));
                }
                cache
                    .by_ref
                    .insert(signal_ref, (Arc::clone(&changes), original_len));
                cache.by_index.insert(index, Arc::clone(&changes));
                if slot > 0 {
                    warnings.clear();
                }
                (changes, warnings)
            })
            .collect()
    }
}

fn charge_alias(
    name: &str,
    original_len: usize,
    kept_len: usize,
    cache: &mut FstCache,
    warnings: &mut Vec<String>,
) {
    if original_len > crate::dump::MAX_CHANGES_PER_SIGNAL {
        warnings.push(format!(
            "{name}: {original_len} value changes decimated to {kept_len} for display"
        ));
    }
    cache.budget = cache.budget.saturating_sub(kept_len as u64);
    if cache.budget == 0 && !cache.warned_total {
        cache.warned_total = true;
        warnings.push(format!(
            "value changes are limited to {} in total; remaining signals are loaded without values",
            crate::dump::MAX_TOTAL_CHANGES
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::SigState;

    #[test]
    fn lazy_fst_matches_eager_and_loads_only_requested_signals() {
        let path = Path::new("waveform/demo.fst");
        let eager = crate::fst::parse_fst(path).expect("eager parse");
        let mut source = super::super::open_source(path, &mut |_| true).expect("lazy open");

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
        assert!(source.take_warnings().is_empty());

        let first = source.read_signals(&[0, 2]);
        assert_eq!(
            first[0].0.as_slice(),
            eager.wf.signals[0].changes.as_slice()
        );
        assert_eq!(
            first[1].0.as_slice(),
            eager.wf.signals[2].changes.as_slice()
        );
        let second = source.read_signals(&[4]);
        assert_eq!(
            second[0].0.as_slice(),
            eager.wf.signals[4].changes.as_slice()
        );
        let cached = source.read_signals(&[0]);
        assert!(Arc::ptr_eq(&first[0].0, &cached[0].0));
    }

    #[test]
    fn aliases_share_one_materialized_arc() {
        use fst_writer::*;

        let path = std::env::temp_dir().join(format!("waverdi_alias_{}.fst", std::process::id()));
        let info = FstInfo {
            start_time: 0,
            timescale_exponent: -9,
            version: "waverdi alias test".to_string(),
            date: "2026-10-27".to_string(),
            file_type: FstFileType::Verilog,
        };
        let mut writer = open_fst(&path, &info).expect("create alias fixture");
        writer.scope("tb", "tb", FstScopeType::Module).unwrap();
        let original = writer
            .var(
                "original",
                FstSignalType::bit_vec(40),
                FstVarType::Wire,
                FstVarDirection::Implicit,
                None,
            )
            .unwrap();
        let _alias = writer
            .var(
                "alias",
                FstSignalType::bit_vec(40),
                FstVarType::Wire,
                FstVarDirection::Implicit,
                Some(original),
            )
            .unwrap();
        writer.up_scope().unwrap();
        let mut writer = writer.finish().unwrap();
        writer.signal_change(original, b"0").unwrap();
        writer.time_change(10).unwrap();
        writer.signal_change(original, b"1").unwrap();
        writer.finish().unwrap();

        let mut source = super::super::open_source(&path, &mut |_| true).expect("lazy open");
        let wf = source.take_hierarchy();
        let original_index = wf
            .signals
            .iter()
            .position(|signal| signal.name == "original")
            .unwrap();
        let alias_index = wf
            .signals
            .iter()
            .position(|signal| signal.name == "alias")
            .unwrap();
        let values = source.read_signals(&[original_index, alias_index]);
        assert!(Arc::ptr_eq(&values[0].0, &values[1].0));
        assert_eq!(values[0].0.len(), 2);
        let _ = std::fs::remove_file(path);
    }
}
