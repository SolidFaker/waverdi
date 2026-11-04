use super::{fmt_value, Radix, SigKind, SigState, Ticks, Value, Waveform};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Vcd,
}

impl ExportFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Vcd => "VCD",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Vcd => "vcd",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Csv => Self::Vcd,
            Self::Vcd => Self::Csv,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportReport {
    pub signals: usize,
    pub values: usize,
}

pub fn export_indices(wf: &Waveform, format: ExportFormat, requested: &[usize]) -> Vec<usize> {
    let mut seen = HashSet::new();
    let mut indices = Vec::new();
    for &index in requested {
        if format == ExportFormat::Vcd && wf.is_synthesized(index) {
            for leaf in wf.value_leaves(index) {
                if seen.insert(leaf) {
                    indices.push(leaf);
                }
            }
        } else if seen.insert(index) {
            indices.push(index);
        }
    }
    indices
}

pub fn export_waveform(
    path: &Path,
    format: ExportFormat,
    wf: &Waveform,
    requested: &[usize],
) -> Result<ExportReport, String> {
    let indices = export_indices(wf, format, requested);
    if indices.is_empty() {
        return Err("no signals selected for export".to_string());
    }
    if indices.iter().any(|&index| {
        wf.signals
            .get(index)
            .is_none_or(|signal| signal.state != SigState::Ready)
    }) {
        return Err("export signals are not fully loaded".to_string());
    }
    let temporary = temporary_path(path);
    let result = write_temporary(&temporary, format, wf, &indices);
    match result {
        Ok(report) => {
            let replace = (|| {
                if path.exists() {
                    std::fs::remove_file(path)
                        .map_err(|error| format!("cannot replace {}: {error}", path.display()))?;
                }
                std::fs::rename(&temporary, path)
                    .map_err(|error| format!("cannot replace {}: {error}", path.display()))
            })();
            if let Err(error) = replace {
                let _ = std::fs::remove_file(&temporary);
                Err(error)
            } else {
                Ok(report)
            }
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn write_temporary(
    path: &Path,
    format: ExportFormat,
    wf: &Waveform,
    indices: &[usize],
) -> Result<ExportReport, String> {
    let file =
        File::create(path).map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    let mut out = BufWriter::new(file);
    let mut report = ExportReport {
        signals: indices.len(),
        values: 0,
    };
    match format {
        ExportFormat::Csv => write_csv(&mut out, wf, indices, &mut report)?,
        ExportFormat::Vcd => write_vcd(&mut out, wf, indices, &mut report)?,
    }
    out.flush()
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(report)
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".tmp");
    PathBuf::from(name)
}

fn write_csv<W: Write>(
    out: &mut W,
    wf: &Waveform,
    indices: &[usize],
    report: &mut ExportReport,
) -> Result<(), String> {
    writeln!(out, "time_ticks,timescale,signal,value").map_err(io_error)?;
    for &index in indices {
        let signal = &wf.signals[index];
        let name = signal.full_name();
        if wf.is_synthesized(index) {
            for &time in wf.value_times(index).iter() {
                if time < wf.start || time > wf.end {
                    continue;
                }
                let text = wf
                    .value_at(index, time)
                    .map(|value| export_value(&value))
                    .unwrap_or_else(|| "x".to_string());
                write_csv_row(out, time, wf, &name, &text)?;
                report.values += 1;
            }
        } else {
            for change in signal.changes.iter() {
                if change.t < wf.start || change.t > wf.end {
                    continue;
                }
                write_csv_row(out, change.t, wf, &name, &export_value(&change.v))?;
                report.values += 1;
            }
        }
    }
    Ok(())
}

fn write_csv_row<W: Write>(
    out: &mut W,
    time: Ticks,
    wf: &Waveform,
    name: &str,
    value: &str,
) -> Result<(), String> {
    writeln!(
        out,
        "{},{},{},{}",
        time,
        csv_field(&wf.ts.label()),
        csv_field(name),
        csv_field(value)
    )
    .map_err(io_error)
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn export_value(value: &Value) -> String {
    match value {
        Value::Real(value) => value.to_string(),
        _ => fmt_value(value, Radix::Bin),
    }
}

fn write_vcd<W: Write>(
    out: &mut W,
    wf: &Waveform,
    indices: &[usize],
    report: &mut ExportReport,
) -> Result<(), String> {
    writeln!(out, "$version waverdi {} $end", env!("CARGO_PKG_VERSION")).map_err(io_error)?;
    writeln!(out, "$timescale {} $end", wf.ts).map_err(io_error)?;
    let vars: Vec<(usize, String)> = indices
        .iter()
        .enumerate()
        .map(|(position, &index)| (index, vcd_identifier(position)))
        .collect();
    let mut scopes: BTreeMap<Vec<String>, Vec<(usize, String)>> = BTreeMap::new();
    for &(index, ref id) in &vars {
        scopes
            .entry(wf.signals[index].scope.clone())
            .or_default()
            .push((index, id.clone()));
    }
    write_vcd_scopes(out, wf, &scopes, &mut Vec::new())?;
    writeln!(out, "$enddefinitions $end").map_err(io_error)?;
    writeln!(out, "$dumpvars").map_err(io_error)?;
    for &(index, ref id) in &vars {
        let signal = &wf.signals[index];
        write_vcd_value(out, signal.value_at(wf.start), id, signal.kind, signal.bits)?;
        report.values += 1;
    }
    writeln!(out, "$end").map_err(io_error)?;

    let mut heap = BinaryHeap::new();
    for (position, &(index, _)) in vars.iter().enumerate() {
        let next = wf.signals[index]
            .changes
            .partition_point(|change| change.t <= wf.start);
        if let Some(change) = wf.signals[index].changes.get(next) {
            heap.push(Reverse(VcdEvent {
                time: change.t,
                signal: position,
                change: next,
            }));
        }
    }
    let mut last_time = None;
    while let Some(Reverse(event)) = heap.pop() {
        if event.time > wf.end {
            break;
        }
        if last_time != Some(event.time) {
            writeln!(out, "#{}", event.time).map_err(io_error)?;
            last_time = Some(event.time);
        }
        let (index, ref id) = vars[event.signal];
        let change = &wf.signals[index].changes[event.change];
        write_vcd_value(out, Some(&change.v), id, SigKind::Bits, 0)?;
        report.values += 1;
        let next = event.change + 1;
        if let Some(change) = wf.signals[index].changes.get(next) {
            heap.push(Reverse(VcdEvent {
                time: change.t,
                signal: event.signal,
                change: next,
            }));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct VcdEvent {
    time: Ticks,
    signal: usize,
    change: usize,
}

fn write_vcd_scopes<W: Write>(
    out: &mut W,
    wf: &Waveform,
    scopes: &BTreeMap<Vec<String>, Vec<(usize, String)>>,
    prefix: &mut Vec<String>,
) -> Result<(), String> {
    if let Some(vars) = scopes.get(prefix.as_slice()) {
        for &(index, ref id) in vars {
            let signal = &wf.signals[index];
            let kind = match signal.kind {
                SigKind::Real => "real",
                SigKind::Str => "string",
                SigKind::Bits => match signal.var_type.as_str() {
                    "reg" | "integer" | "time" => signal.var_type.as_str(),
                    _ => "wire",
                },
            };
            let name = if signal.bits > 1 && !signal.name.contains('[') {
                format!("{} [{}:0]", signal.name, signal.bits - 1)
            } else {
                signal.name.clone()
            };
            writeln!(
                out,
                "$var {kind} {} {} {} $end",
                signal.bits.max(1),
                id,
                vcd_name(&name)
            )
            .map_err(io_error)?;
        }
    }
    let children: BTreeSet<&str> = scopes
        .keys()
        .filter(|path| path.len() > prefix.len())
        .filter_map(|path| path.get(prefix.len()).map(String::as_str))
        .collect();
    for child in children {
        writeln!(out, "$scope module {} $end", vcd_name(child)).map_err(io_error)?;
        prefix.push(child.to_string());
        write_vcd_scopes(out, wf, scopes, prefix)?;
        prefix.pop();
        writeln!(out, "$upscope $end").map_err(io_error)?;
    }
    Ok(())
}

fn vcd_name(name: &str) -> String {
    if name.chars().any(char::is_whitespace) {
        format!("\\{name} ")
    } else {
        name.to_string()
    }
}

fn vcd_identifier(mut value: usize) -> String {
    let mut id = String::new();
    loop {
        id.push(char::from(b'!' + (value % 94) as u8));
        value /= 94;
        if value == 0 {
            break;
        }
        value -= 1;
    }
    id
}

fn write_vcd_value<W: Write>(
    out: &mut W,
    value: Option<&Value>,
    id: &str,
    kind: SigKind,
    bits: u32,
) -> Result<(), String> {
    match value {
        Some(Value::Real(value)) => writeln!(out, "r{value} {id}").map_err(io_error),
        Some(Value::Str(value)) => writeln!(out, "s{value} {id}").map_err(io_error),
        Some(value) => {
            let text = fmt_value(value, Radix::Bin);
            let digits = text.strip_prefix('b').unwrap_or(&text);
            if bits == 1 && digits.len() == 1 {
                writeln!(out, "{digits}{id}").map_err(io_error)
            } else {
                writeln!(out, "b{digits} {id}").map_err(io_error)
            }
        }
        None => match kind {
            SigKind::Real => writeln!(out, "r0 {id}").map_err(io_error),
            SigKind::Str => writeln!(out, "s {id}").map_err(io_error),
            SigKind::Bits if bits == 1 => writeln!(out, "x{id}").map_err(io_error),
            SigKind::Bits => {
                writeln!(out, "b{} {id}", "x".repeat(bits.max(1) as usize)).map_err(io_error)
            }
        },
    }
}

fn io_error(error: std::io::Error) -> String {
    format!("export write failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::{Change, ScopeTree, Signal, TimeScale};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn signal(name: &str, bits: u32, changes: &[(u64, u8)]) -> Signal {
        Signal {
            name: name.to_string(),
            bits,
            var_type: "wire".to_string(),
            dir: String::new(),
            scope: vec!["top".to_string()],
            kind: SigKind::Bits,
            changes: Arc::new(
                changes
                    .iter()
                    .map(|&(time, value)| Change {
                        t: time,
                        v: Value::compact(if bits == 1 {
                            vec![value]
                        } else {
                            vec![value, 0]
                        }),
                    })
                    .collect(),
            ),
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            parent: None,
            members: Vec::new(),
            state: SigState::Ready,
        }
    }

    fn waveform() -> Waveform {
        Waveform {
            ts: TimeScale::DEFAULT,
            start: 0,
            end: 20,
            signals: vec![
                signal("clk", 1, &[(0, 0), (10, 1), (20, 0)]),
                signal("data", 2, &[(0, 0), (10, 1)]),
            ],
            tree: ScopeTree::new(),
            radix: HashMap::new(),
            value_times_cache: Vec::new(),
        }
    }

    #[test]
    fn csv_export_is_streamed_with_escaped_names_and_lossless_values() {
        let path = std::env::temp_dir().join(format!("waverdi_export_{}.csv", std::process::id()));
        let report = export_waveform(&path, ExportFormat::Csv, &waveform(), &[0, 1]).unwrap();
        assert_eq!(
            report,
            ExportReport {
                signals: 2,
                values: 5
            }
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("time_ticks,timescale,signal,value\n"));
        assert!(text.contains("10,1ns,top.clk,b1\n"));
        assert!(text.contains("10,1ns,top.data,b01\n"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn vcd_export_can_be_loaded_again() {
        let path = std::env::temp_dir().join(format!("waverdi_export_{}.vcd", std::process::id()));
        let report = export_waveform(&path, ExportFormat::Vcd, &waveform(), &[0, 1]).unwrap();
        assert_eq!(report.signals, 2);
        let out = crate::vcd::parse_bytes(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(out.wf.signals.len(), 2);
        assert_eq!(out.wf.signals[0].changes, waveform().signals[0].changes);
        assert_eq!(out.wf.signals[1].changes, waveform().signals[1].changes);
        let _ = std::fs::remove_file(path);
    }
}
