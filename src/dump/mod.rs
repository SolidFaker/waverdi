//! Dump formats and the adapter layer between them and the UI.
//!
//! Every format is reached through [`source::DumpSource`]: the loader gets a
//! parsed hierarchy plus a `read_signal` entry point, so the UI, the compute
//! pool and the change budget are format independent. This module also owns
//! the shared limits every reader applies ([`MAX_CHANGES_PER_SIGNAL`],
//! [`MAX_TOTAL_CHANGES`], [`decimate`]), and the register of formats
//! ([`Format`], [`detect`]) that [`source::open_source`] dispatches on.
//!
//! Adding a format:
//! 1. extend [`Format`] and [`detect`] with its extension;
//! 2. write the parser (or converter shim) that produces a [`ParseOut`];
//! 3. add a [`source::DumpSource`] implementation that serves values per
//!    signal - lazily when the format allows random reads, otherwise by
//!    parsing up front and handing the values out on request (see
//!    `EagerSource`); apply the shared limits via [`limit_changes`];
//! 4. dispatch to it from [`source::open_source`];
//! 5. test through `cargo test` and a `dump::source` unit test.

use crate::waveform::{Change, Waveform};
use std::path::Path;

pub mod source;

/// Cap for synthesized (array/aggregate) brace texts. Each of their values is
/// a formatted string, so they cannot hold as many changes as a plain signal.
pub(crate) const MAX_AGGREGATE_CHANGES: usize = 200_000;

/// A parsed waveform plus non-fatal parser warnings.
pub struct ParseOut {
    pub wf: Waveform,
    pub warnings: Vec<String>,
}

/// Coarse progress of a background load, suitable for a status line.
#[derive(Clone, Copy, Debug, Default)]
pub struct LoadProgress {
    pub stage: Stage,
    /// Items finished in the current stage (signals or source files).
    pub done: usize,
    /// Total items of the current stage; zero while unknown.
    pub total: usize,
    /// Value changes kept so far (FSDB only).
    pub changes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    Waveform,
    Rtl,
}

/// Progress callback: return `false` to cancel the load.
pub type Progress<'a> = &'a mut dyn FnMut(LoadProgress) -> bool;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Vcd,
    Fst,
    Fsdb,
    Unknown,
}

/// Value changes kept per signal; longer recordings are decimated evenly.
pub(crate) const MAX_CHANGES_PER_SIGNAL: usize = 2_000_000;
/// Hard stop while reading one signal (the slice is decimated afterwards).
pub(crate) const MAX_CHANGES_READ_PER_SIGNAL: usize = 16_000_000;
/// Total kept value changes across all signals; once reached, the remaining
/// signals load without values so memory stays bounded.
pub(crate) const MAX_TOTAL_CHANGES: u64 = 32_000_000;

/// Cap one signal's change list, then charge the kept changes to the global
/// `budget`. Shared by every format so the limits and their warnings read the
/// same whether the changes arrived eagerly or through the lazy FSDB reader.
pub(crate) fn limit_changes(
    name: &str,
    mut changes: Vec<Change>,
    cap: usize,
    budget: &mut u64,
    warnings: &mut Vec<String>,
) -> Vec<Change> {
    if changes.len() > cap {
        let before = changes.len();
        changes = decimate(changes, cap);
        warnings.push(format!(
            "{name}: {before} value changes decimated to {} for display",
            changes.len()
        ));
    }
    *budget = budget.saturating_sub(changes.len() as u64);
    if *budget == 0 {
        warnings.push(format!(
            "value changes are limited to {MAX_TOTAL_CHANGES} in total; remaining signals are loaded without values"
        ));
    }
    changes
}

/// Keep at most `cap` changes, preserving the first and last and any
/// isolated pulse (a change whose neighbours have the same value), so narrow
/// glitches survive even in heavily decimated signals.
pub(crate) fn decimate(changes: Vec<Change>, cap: usize) -> Vec<Change> {
    let n = changes.len();
    let step = n.div_ceil(cap);
    if step <= 1 {
        return changes;
    }
    let mut keep = vec![false; n];
    let mut index = 0usize;
    while index < n {
        keep[index] = true;
        index += step;
    }
    keep[n - 1] = true;
    for index in 1..n.saturating_sub(1) {
        if changes[index].v != changes[index - 1].v
            && changes[index].v != changes[index + 1].v
            && changes[index - 1].v == changes[index + 1].v
        {
            keep[index - 1] = true;
            keep[index] = true;
            keep[index + 1] = true;
        }
    }
    let mut kept = Vec::with_capacity(cap + cap / 4 + 1);
    for (index, change) in changes.into_iter().enumerate() {
        if keep[index] {
            kept.push(change);
        }
    }
    kept
}

/// Bound every signal of an eagerly parsed dump to the same change limits
/// the lazy FSDB reader enforces, so huge VCD/FST files stay within the
/// same memory budget once parsing is done.
pub(crate) fn enforce_limits(out: &mut ParseOut) {
    let mut budget = MAX_TOTAL_CHANGES;
    for signal in &mut out.wf.signals {
        let name = signal.name.clone();
        let changes = std::mem::take(&mut signal.changes);
        signal.changes = limit_changes(
            &name,
            changes,
            MAX_CHANGES_PER_SIGNAL,
            &mut budget,
            &mut out.warnings,
        );
    }
}

/// Detect the dump format from the file extension.
pub fn detect(path: &Path) -> Format {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("vcd") => Format::Vcd,
        Some("fst") => Format::Fst,
        Some("fsdb") => Format::Fsdb,
        _ => Format::Unknown,
    }
}

/// Parse a waveform dump, dispatching on the file format.
pub fn parse(path: &Path) -> Result<ParseOut, String> {
    parse_with_progress(path, &mut |_| true)
}

/// Parse with progress reporting and cancellation.
pub fn parse_with_progress(path: &Path, progress: Progress) -> Result<ParseOut, String> {
    match detect(path) {
        Format::Vcd => {
            let mut out = crate::vcd::parse_vcd(path)?;
            enforce_limits(&mut out);
            if !progress(LoadProgress {
                stage: Stage::Waveform,
                done: 1,
                total: 1,
                changes: 0,
            }) {
                return Err("load cancelled".to_string());
            }
            Ok(out)
        }
        Format::Fst => {
            let mut out = crate::fst::parse_fst(path)?;
            enforce_limits(&mut out);
            if !progress(LoadProgress {
                stage: Stage::Waveform,
                done: 1,
                total: 1,
                changes: 0,
            }) {
                return Err("load cancelled".to_string());
            }
            Ok(out)
        }
        Format::Fsdb => parse_fsdb(path, progress),
        Format::Unknown => Err(format!(
            "{}: unsupported dump type (supported: .vcd, .fst; .fsdb requires Verdi FFR)",
            path.display()
        )),
    }
}

/// FSDB is read through the Verdi FSDB Reader (FFR) when the SDK was
/// available at build time (`VERDI_HOME` set).
#[cfg(fsdb_sdk)]
fn parse_fsdb(path: &Path, progress: Progress) -> Result<ParseOut, String> {
    crate::fsdb::parse_fsdb_with(path, progress)
}

#[cfg(not(fsdb_sdk))]
fn parse_fsdb(path: &Path, _progress: Progress) -> Result<ParseOut, String> {
    Err(format!(
        "{}: FSDB is a proprietary Synopsys format and can only be read with the \
         Verdi FSDB Reader (FFR) library (nffr.dll / libnffr.so from VERDI_HOME). \
         Build with VERDI_HOME set (source ~/synopsys/env.sh) to enable FSDB support, \
         or convert the dump to VCD or FST (e.g. `fsdb2vcd`) and open that instead.",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::Value;

    fn changes_of(count: usize) -> Vec<Change> {
        (0..count)
            .map(|i| Change {
                t: i as u64,
                v: Value::compact(vec![i as u8]),
            })
            .collect()
    }

    #[test]
    fn limit_changes_decimates_and_stops_at_the_budget() {
        let mut budget = 100u64;
        let mut warnings = Vec::new();
        let kept = limit_changes("s", changes_of(10), 3, &mut budget, &mut warnings);
        assert!(kept.len() <= 4, "{} changes kept", kept.len());
        assert!(
            warnings.iter().any(|w| w.contains("decimated")),
            "{warnings:?}"
        );

        let mut budget = 0u64;
        let mut warnings = Vec::new();
        let _ = limit_changes("s", changes_of(10), 3, &mut budget, &mut warnings);
        assert!(
            warnings.iter().any(|w| w.contains("limited to")),
            "{warnings:?}"
        );
    }

    #[test]
    fn decimates_long_change_lists() {
        let changes: Vec<Change> = (0..1000)
            .map(|i| Change {
                t: i,
                v: Value::Real(i as f64),
            })
            .collect();
        let kept = decimate(changes, 10);
        assert_eq!(kept.len(), 11);
        assert_eq!(kept.first().unwrap().t, 0);
        assert_eq!(kept.last().unwrap().t, 999);
    }

    #[test]
    fn decimation_keeps_isolated_pulses() {
        let mut changes: Vec<Change> = (0..10_000)
            .map(|i| Change {
                t: i,
                v: Value::Real(0.0),
            })
            .collect();
        changes[5_000] = Change {
            t: 5_000,
            v: Value::Real(1.0),
        };
        let kept = decimate(changes, 100);
        assert!(
            kept.iter()
                .any(|change| change.t == 5_000 && change.v.as_real() == Some(1.0)),
            "pulse was decimated away"
        );
    }

    #[test]
    fn vcd_signals_are_still_parsed_normally() {
        let mut out = crate::vcd::parse_bytes(
            b"$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n",
        )
        .expect("vcd parses");
        enforce_limits(&mut out);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.wf.signals.len(), 1);
        assert_eq!(out.wf.signals[0].changes.len(), 1);
    }

    #[test]
    fn detects_formats_by_extension() {
        assert_eq!(detect(Path::new("a.vcd")), Format::Vcd);
        assert_eq!(detect(Path::new("a.FST")), Format::Fst);
        assert_eq!(detect(Path::new("a.fsdb")), Format::Fsdb);
        assert_eq!(detect(Path::new("a.txt")), Format::Unknown);
    }

    #[test]
    #[cfg(not(fsdb_sdk))]
    fn fsdb_reports_verdi_hint() {
        let err = match parse(Path::new("missing.fsdb")) {
            Ok(_) => panic!("fsdb must not parse without the Verdi FFR library"),
            Err(err) => err,
        };
        assert!(err.contains("Verdi"), "{err}");
        assert!(err.contains("FFR"), "{err}");
    }

    #[test]
    #[cfg(fsdb_sdk)]
    fn fsdb_without_file_errors() {
        assert!(parse(Path::new("missing.fsdb")).is_err());
    }
}
