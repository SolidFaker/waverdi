use crate::dump::ParseOut;
use crate::waveform::{Change, ScopeTree, SigKind, SigState, Signal, TimeScale, Value, Waveform};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use wellen::simple::Waveform as WellenWaveform;
use wellen::{Hierarchy, ItemRef, SignalRef, SignalValueRef, TimescaleUnit};

/// Load an FST dump through the `wellen` library.
pub fn parse_fst(path: &Path) -> Result<ParseOut, String> {
    let mut source =
        wellen::simple::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    let refs: Vec<SignalRef> = {
        let hierarchy = source.hierarchy();
        let mut seen = HashSet::new();
        let mut refs = Vec::new();
        for var_ref in hierarchy.all_vars() {
            let signal_ref = hierarchy[var_ref].signal_ref();
            if seen.insert(signal_ref) {
                refs.push(signal_ref);
            }
        }
        refs
    };
    source.load_signals(&refs);

    let mut warnings = Vec::new();
    let ts = convert_timescale(source.hierarchy().timescale(), &mut warnings);

    let hierarchy = source.hierarchy();
    let mut signals = Vec::new();
    let mut tree = ScopeTree::new();
    let mut cache: HashMap<SignalRef, Vec<Change>> = HashMap::new();
    let mut scope = Vec::new();
    for item in hierarchy.items() {
        visit_item(
            &source,
            hierarchy,
            item,
            tree.root,
            &mut tree,
            &mut signals,
            &mut scope,
            &mut cache,
        );
    }

    let mut start = u64::MAX;
    let mut end = 0u64;
    for signal in &signals {
        if let Some(change) = signal.changes.first() {
            start = start.min(change.t);
        }
        if let Some(change) = signal.changes.last() {
            end = end.max(change.t);
        }
    }
    if start == u64::MAX {
        start = source.time_table().first().copied().unwrap_or(0);
    }
    if let Some(last) = source.time_table().last() {
        end = end.max(*last);
    }

    let wf = Waveform {
        ts,
        start,
        end,
        signals,
        tree,
        radix: HashMap::new(),
        value_times_cache: Vec::new(),
    };
    Ok(ParseOut { wf, warnings })
}

#[allow(clippy::too_many_arguments)]
fn visit_item(
    source: &WellenWaveform,
    hierarchy: &Hierarchy,
    item: ItemRef,
    parent: usize,
    tree: &mut ScopeTree,
    signals: &mut Vec<Signal>,
    scope: &mut Vec<String>,
    cache: &mut HashMap<SignalRef, Vec<Change>>,
) {
    match item {
        ItemRef::Scope(scope_ref) => {
            let name = hierarchy[scope_ref].name(hierarchy).to_string();
            let id = tree.add_scope(parent, name.clone(), String::new());
            scope.push(name);
            for child in hierarchy[scope_ref].items(hierarchy) {
                visit_item(source, hierarchy, child, id, tree, signals, scope, cache);
            }
            scope.pop();
        }
        ItemRef::Var(var_ref) => {
            let var = &hierarchy[var_ref];
            let signal_ref = var.signal_ref();
            let changes = if let Some(cached) = cache.get(&signal_ref) {
                cached.clone()
            } else {
                let changes = materialize(source, signal_ref);
                cache.insert(signal_ref, changes.clone());
                changes
            };
            let kind = if var.is_real(hierarchy) {
                SigKind::Real
            } else if var.is_string(hierarchy) {
                SigKind::Str
            } else {
                SigKind::Bits
            };
            let bits = var.length(hierarchy).unwrap_or(match kind {
                SigKind::Real => 64,
                SigKind::Str => 0,
                SigKind::Bits => 1,
            });
            let (min, max) = if kind == SigKind::Real {
                real_range(&changes)
            } else {
                (f64::INFINITY, f64::NEG_INFINITY)
            };
            let signal = Signal {
                name: var.name(hierarchy).to_string(),
                bits,
                var_type: format!("{:?}", var.var_type()).to_lowercase(),
                dir: String::new(),
                scope: scope.clone(),
                kind,
                changes: Arc::new(changes),
                min,
                max,
                parent: None,
                members: Vec::new(),
                state: SigState::Ready,
            };
            let index = signals.len();
            signals.push(signal);
            tree.nodes[parent].signals.push(index);
        }
    }
}

fn materialize(source: &WellenWaveform, signal_ref: SignalRef) -> Vec<Change> {
    let Some(signal) = source.get_signal(signal_ref) else {
        return Vec::new();
    };
    let time_table = source.time_table();
    let mut changes: Vec<Change> = Vec::new();
    for (time_idx, value) in signal.iter_changes() {
        let Some(value) = convert_value(value) else {
            continue;
        };
        if changes.last().map(|c| c.v == value).unwrap_or(false) {
            continue;
        }
        changes.push(Change {
            t: time_table[time_idx as usize],
            v: value,
        });
    }
    changes
}

fn convert_value(value: SignalValueRef<'_>) -> Option<Value> {
    match value {
        SignalValueRef::Event => None,
        SignalValueRef::BitVec(bits) => Some(Value::compact(
            bits.iter_lsb_to_msb()
                .map(|bit| match bit.as_ascii() {
                    '0' | 'l' => 0u8,
                    '1' | 'h' => 1u8,
                    'z' | 'Z' => 3u8,
                    _ => 2u8,
                })
                .collect(),
        )),
        SignalValueRef::String(s) => Some(Value::Str(s.to_string())),
        SignalValueRef::Real(real) => Some(Value::Real(real)),
    }
}

fn real_range(changes: &[Change]) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for change in changes {
        if let Some(real) = change.v.as_real() {
            min = min.min(real);
            max = max.max(real);
        }
    }
    if !min.is_finite() || !max.is_finite() {
        (0.0, 1.0)
    } else {
        (min, max)
    }
}

fn convert_timescale(
    timescale: Option<wellen::Timescale>,
    warnings: &mut Vec<String>,
) -> TimeScale {
    let Some(ts) = timescale else {
        return TimeScale::DEFAULT;
    };
    let unit = match ts.unit {
        TimescaleUnit::Seconds => "s",
        TimescaleUnit::MilliSeconds => "ms",
        TimescaleUnit::MicroSeconds => "us",
        TimescaleUnit::NanoSeconds => "ns",
        TimescaleUnit::PicoSeconds => "ps",
        TimescaleUnit::FemtoSeconds => "fs",
        TimescaleUnit::AttoSeconds | TimescaleUnit::ZeptoSeconds => {
            warnings.push("timescale smaller than 1fs rounded to fs".to_string());
            "fs"
        }
        TimescaleUnit::Unknown => {
            warnings.push("unknown timescale, assuming 1ns".to_string());
            "ns"
        }
    };
    let num = if matches!(
        ts.unit,
        TimescaleUnit::AttoSeconds | TimescaleUnit::ZeptoSeconds
    ) {
        (ts.factor / 1000).max(1)
    } else {
        ts.factor.max(1)
    };
    TimeScale { num, unit }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_demo_fst() {
        let path = Path::new("waveform/demo.fst");
        let out = parse_fst(path).expect("waveform/demo.fst should parse");
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.wf.ts.label(), "1ns");
        let clk = out
            .wf
            .signals
            .iter()
            .find(|s| s.name == "clk")
            .expect("clk signal");
        assert_eq!(clk.bits, 1);
        assert!(clk.changes.len() >= 3);
        let data = out
            .wf
            .signals
            .iter()
            .find(|s| s.name == "data")
            .expect("data signal");
        assert_eq!(data.bits, 8);
        let temp = out
            .wf
            .signals
            .iter()
            .find(|s| s.name == "temp")
            .expect("temp signal");
        assert_eq!(temp.kind, SigKind::Real);
        assert!(out.wf.total_ticks() > 0);
    }

    #[test]
    #[ignore = "regenerates the demo.fst fixture (needs the fst-writer dev dependency)"]
    fn generate_demo_fst() {
        use fst_writer::*;
        let info = FstInfo {
            start_time: 0,
            timescale_exponent: -9,
            version: "waverdi demo".to_string(),
            date: "2026-09-12".to_string(),
            file_type: FstFileType::Verilog,
        };
        let _ = std::fs::create_dir_all("waveform");
        let _ = std::fs::remove_file("waveform/demo.fst");
        let mut writer = open_fst("waveform/demo.fst", &info).unwrap();
        writer.scope("tb", "tb", FstScopeType::Module).unwrap();
        let clk = writer
            .var(
                "clk",
                FstSignalType::bit_vec(1),
                FstVarType::Wire,
                FstVarDirection::Implicit,
                None,
            )
            .unwrap();
        let rst_n = writer
            .var(
                "rst_n",
                FstSignalType::bit_vec(1),
                FstVarType::Wire,
                FstVarDirection::Implicit,
                None,
            )
            .unwrap();
        writer
            .scope("u_dut", "u_dut", FstScopeType::Module)
            .unwrap();
        let state = writer
            .var(
                "state",
                FstSignalType::bit_vec(3),
                FstVarType::Reg,
                FstVarDirection::Implicit,
                None,
            )
            .unwrap();
        let data = writer
            .var(
                "data",
                FstSignalType::bit_vec(8),
                FstVarType::Reg,
                FstVarDirection::Implicit,
                None,
            )
            .unwrap();
        let temp = writer
            .var(
                "temp",
                FstSignalType::real(),
                FstVarType::Real,
                FstVarDirection::Implicit,
                None,
            )
            .unwrap();
        writer.up_scope().unwrap();
        writer.up_scope().unwrap();

        let mut writer = writer.finish().unwrap();
        writer.signal_change(clk, b"0").unwrap();
        writer.signal_change(rst_n, b"0").unwrap();
        writer.signal_change(state, b"000").unwrap();
        writer.signal_change(data, b"00000000").unwrap();
        writer.signal_change(temp, &25.0f64.to_le_bytes()).unwrap();

        let steps: [(u64, u8, &[u8], &[u8]); 6] = [
            (10, b'1', b"001", b"00000001"),
            (20, b'0', b"001", b"00000001"),
            (30, b'1', b"010", b"10101010"),
            (40, b'0', b"010", b"10101010"),
            (50, b'1', b"011", b"11110000"),
            (60, b'0', b"011", b"11110000"),
        ];
        for (step, clk_value, state_value, data_value) in steps {
            writer.time_change(step).unwrap();
            writer.signal_change(clk, &[clk_value]).unwrap();
            if step == 10 {
                writer.signal_change(rst_n, b"1").unwrap();
            }
            writer.signal_change(state, state_value).unwrap();
            writer.signal_change(data, data_value).unwrap();
            let real = 25.0 + (step as f64) * 0.5;
            writer.signal_change(temp, &real.to_le_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }
}
