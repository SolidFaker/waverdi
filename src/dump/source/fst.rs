use super::{DumpSource, ReadResult};
use crate::waveform::{Change, Waveform};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Lazy FST source. The first value request decodes the value section once;
/// later batches clone per-signal arcs from the cache.
pub(super) struct FstSource {
    hierarchy: Option<Waveform>,
    warnings: Vec<String>,
    values: crate::fst::FstValues,
    refs: Vec<wellen::SignalRef>,
    names: Vec<String>,
    changes: Mutex<Option<Vec<Arc<Vec<Change>>>>>,
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
            changes: Mutex::new(None),
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

/// Decode each distinct wellen reference once, then share the bounded result
/// with every alias in hierarchy order.
fn decode_fst_all(
    values: &mut crate::fst::FstValues,
    refs: &[wellen::SignalRef],
    names: &[String],
) -> (Vec<Arc<Vec<Change>>>, Vec<String>) {
    let unique: Vec<wellen::SignalRef> = {
        let mut seen = std::collections::HashSet::new();
        refs.iter().copied().filter(|r| seen.insert(*r)).collect()
    };
    let loaded = values.load(&unique);
    let mut decoded = HashMap::new();
    for signal_ref in unique {
        let changes = loaded
            .get(&signal_ref)
            .map(|signal| crate::fst::materialize(signal, values.time_table()))
            .unwrap_or_default();
        decoded.insert(signal_ref, changes);
    }

    let mut warnings = Vec::new();
    let mut budget = crate::dump::MAX_TOTAL_CHANGES;
    let mut shared: HashMap<wellen::SignalRef, Arc<Vec<Change>>> = HashMap::new();
    let mut original_lengths = HashMap::new();
    let mut changes = Vec::with_capacity(refs.len());
    for (index, signal_ref) in refs.iter().enumerate() {
        let name = names.get(index).map(String::as_str).unwrap_or("");
        if let Some(cached) = shared.get(signal_ref) {
            let original_len = original_lengths.get(signal_ref).copied().unwrap_or(0);
            charge_alias(name, original_len, cached.len(), &mut budget, &mut warnings);
            changes.push(Arc::clone(cached));
            continue;
        }
        let raw = decoded.remove(signal_ref).unwrap_or_default();
        original_lengths.insert(*signal_ref, raw.len());
        let limited = Arc::new(crate::dump::limit_changes(
            name,
            raw,
            crate::dump::MAX_CHANGES_PER_SIGNAL,
            &mut budget,
            &mut warnings,
        ));
        shared.insert(*signal_ref, Arc::clone(&limited));
        changes.push(limited);
    }
    (changes, warnings)
}

/// Apply the same accounting and warnings as `limit_changes` when an alias
/// reuses the already-decimated allocation of its canonical signal.
fn charge_alias(
    name: &str,
    original_len: usize,
    kept_len: usize,
    budget: &mut u64,
    warnings: &mut Vec<String>,
) {
    if original_len > crate::dump::MAX_CHANGES_PER_SIGNAL {
        warnings.push(format!(
            "{name}: {original_len} value changes decimated to {kept_len} for display"
        ));
    }
    *budget = budget.saturating_sub(kept_len as u64);
    if *budget == 0 {
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
    fn lazy_fst_matches_eager_and_serves_values_on_demand() {
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
        let alias = writer
            .var(
                "alias",
                FstSignalType::bit_vec(40),
                FstVarType::Wire,
                FstVarDirection::Implicit,
                Some(original),
            )
            .unwrap();
        assert_eq!(original, alias);
        writer.up_scope().unwrap();
        let mut writer = writer.finish().unwrap();
        writer
            .signal_change(original, "1010".repeat(10).as_bytes())
            .unwrap();
        writer.time_change(1).unwrap();
        writer
            .signal_change(original, "0101".repeat(10).as_bytes())
            .unwrap();
        writer.finish().unwrap();

        let eager = crate::fst::parse_fst(&path).expect("eager alias parse");
        assert_eq!(eager.wf.signals.len(), 2);
        assert!(Arc::ptr_eq(
            &eager.wf.signals[0].changes,
            &eager.wf.signals[1].changes
        ));

        let mut source = FstSource::open(&path).expect("lazy alias open");
        assert_eq!(source.refs.len(), 2);
        assert_eq!(source.refs[0], source.refs[1]);
        let results = source.read_signals(&[1, 0]);
        assert!(!results[0].0.is_empty());
        assert!(Arc::ptr_eq(&results[0].0, &results[1].0));
        let cached = source.read_signals(&[0]);
        assert!(Arc::ptr_eq(&cached[0].0, &results[1].0));
        let _ = std::fs::remove_file(&path);
    }
}
