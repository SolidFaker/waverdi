//! FSDB loader built on the Synopsys FSDB Reader (FFR) SDK.
//!
//! The FFR library only exports C++ symbols, so `csrc/ffr_bridge.cpp` is
//! compiled against the official headers and exposes the plain C functions
//! used here. `build.rs` only enables this module (`cfg(fsdb_sdk)`) when
//! `VERDI_HOME` points at an installation shipping `share/FsdbReader`.

use crate::dump::ParseOut;
use crate::waveform::{Change, ScopeTree, SigKind, Signal, TimeScale, Value, Waveform};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;

type ScopeCb = extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char, u32);
type VarCb = extern "C" fn(*mut c_void, *const c_char, i64, u32, u32, u32, u32, u32);
type UpscopeCb = extern "C" fn(*mut c_void);

extern "C" {
    fn wav_fsdb_is_fsdb(path: *const c_char) -> i32;
    fn wav_fsdb_open(
        path: *const c_char,
        scope: ScopeCb,
        var: VarCb,
        upscope: UpscopeCb,
        user: *mut c_void,
    ) -> *mut c_void;
    fn wav_fsdb_read_tree(handle: *mut c_void) -> i32;
    fn wav_fsdb_add_signal(handle: *mut c_void, idcode: i64) -> i32;
    fn wav_fsdb_load_signals(handle: *mut c_void) -> i32;
    fn wav_fsdb_time_range(handle: *mut c_void, min: *mut u64, max: *mut u64) -> i32;
    fn wav_fsdb_scale_unit(handle: *mut c_void, buf: *mut c_char, len: u64) -> i32;
    fn wav_fsdb_close(handle: *mut c_void);
    fn wav_fsdb_vc_handle(handle: *mut c_void, idcode: i64) -> *mut c_void;
    fn wav_fsdb_has_vc(vc: *mut c_void) -> i32;
    fn wav_fsdb_min_time(vc: *mut c_void, t: *mut u64) -> i32;
    fn wav_fsdb_goto_time(vc: *mut c_void, t: u64) -> i32;
    fn wav_fsdb_next_vc(vc: *mut c_void) -> i32;
    fn wav_fsdb_cur_time(vc: *mut c_void, t: *mut u64) -> i32;
    fn wav_fsdb_value(vc: *mut c_void, buf: *mut u8, len: u64, out_len: *mut u64) -> i32;
    fn wav_fsdb_bit_size(vc: *mut c_void) -> u32;
    fn wav_fsdb_bytes_per_bit(vc: *mut c_void) -> u32;
    fn wav_fsdb_free_handle(vc: *mut c_void);
}

struct VarMeta {
    idcode: i64,
    name: String,
    scope: Vec<String>,
    width: u32,
    var_type: u32,
    bytes_per_bit: u32,
    tree_node: usize,
}

#[derive(Default)]
struct Collector {
    tree: Option<ScopeTree>,
    scope_names: Vec<String>,
    scope_nodes: Vec<usize>,
    current_node: usize,
    vars: Vec<VarMeta>,
    time_unit: Option<String>,
    warnings: Vec<String>,
}

/// The FFR library writes a banner and warnings straight to stderr, which
/// would corrupt the TUI. Route fd 2 to /dev/null while talking to the SDK.
#[cfg(unix)]
struct StderrSilencer {
    saved: i32,
}

#[cfg(unix)]
impl StderrSilencer {
    fn new() -> Self {
        unsafe {
            let saved = libc::dup(libc::STDERR_FILENO);
            let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
            if devnull >= 0 {
                libc::dup2(devnull, libc::STDERR_FILENO);
                libc::close(devnull);
            }
            Self { saved }
        }
    }
}

#[cfg(unix)]
impl Drop for StderrSilencer {
    fn drop(&mut self) {
        unsafe {
            if self.saved >= 0 {
                libc::dup2(self.saved, libc::STDERR_FILENO);
                libc::close(self.saved);
            }
        }
    }
}

/// FFR keeps process-global state (active object, message hooks), so all
/// access is serialized even when parsing from multiple threads.
static FFR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn parse_fsdb(path: &Path) -> Result<ParseOut, String> {
    let display = path.display().to_string();
    let cpath =
        CString::new(display.clone()).map_err(|_| format!("{display}: path contains NUL"))?;

    let _lock = FFR_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let _silence = StderrSilencer::new();
    if unsafe { wav_fsdb_is_fsdb(cpath.as_ptr()) } == 0 {
        return Err(format!("{display}: not an FSDB file"));
    }

    let mut collector = Collector::default();
    let handle = unsafe {
        wav_fsdb_open(
            cpath.as_ptr(),
            on_scope,
            on_var,
            on_upscope,
            &mut collector as *mut Collector as *mut c_void,
        )
    };
    if handle.is_null() {
        return Err(format!("{display}: failed to open FSDB file (FFR)"));
    }

    let result = parse_open(handle, &display, &mut collector);
    unsafe { wav_fsdb_close(handle) };
    result
}

fn parse_open(
    handle: *mut c_void,
    display: &str,
    collector: &mut Collector,
) -> Result<ParseOut, String> {
    if unsafe { wav_fsdb_read_tree(handle) } != 0 {
        return Err(format!("{display}: failed to read the FSDB hierarchy"));
    }

    let mut warnings = std::mem::take(&mut collector.warnings);
    for var in &collector.vars {
        unsafe { wav_fsdb_add_signal(handle, var.idcode) };
    }
    if unsafe { wav_fsdb_load_signals(handle) } != 0 {
        return Err(format!("{display}: failed to load signal values"));
    }

    let mut min_time = 0u64;
    let mut max_time = 0u64;
    let have_range = unsafe { wav_fsdb_time_range(handle, &mut min_time, &mut max_time) } == 0;

    let mut unit_buf = [0 as c_char; 64];
    let scale_unit = if unsafe {
        wav_fsdb_scale_unit(handle, unit_buf.as_mut_ptr(), unit_buf.len() as u64)
    } == 0
    {
        unsafe { CStr::from_ptr(unit_buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    } else {
        String::new()
    };
    let ts = parse_timescale(&scale_unit, collector.time_unit.as_deref(), &mut warnings);

    let mut tree = collector.tree.take().unwrap_or_default();
    let mut signals = Vec::with_capacity(collector.vars.len());
    let mut start = u64::MAX;
    let mut end = 0u64;
    for var in &collector.vars {
        let changes = read_changes(handle, var, &mut warnings);
        for change in &changes {
            start = start.min(change.t);
            end = end.max(change.t);
        }
        let kind = if var.bytes_per_bit >= 2 {
            SigKind::Real
        } else {
            SigKind::Bits
        };
        let (min, max) = if kind == SigKind::Real {
            real_range(&changes)
        } else {
            (f64::INFINITY, f64::NEG_INFINITY)
        };
        let signal = Signal {
            name: var.name.clone(),
            bits: var.width,
            var_type: var_type_name(var.var_type),
            scope: var.scope.clone(),
            kind,
            changes,
            min,
            max,
            parent: None,
        };
        let index = signals.len();
        signals.push(signal);
        tree.nodes[var.tree_node].signals.push(index);
    }
    if start == u64::MAX {
        start = min_time;
    }
    if have_range {
        start = start.min(min_time);
        end = end.max(max_time);
    }

    Ok(ParseOut {
        wf: Waveform {
            ts,
            start,
            end,
            signals,
            tree,
        },
        warnings,
    })
}

fn read_changes(handle: *mut c_void, var: &VarMeta, warnings: &mut Vec<String>) -> Vec<Change> {
    if var.var_type == 17 || var.var_type == 18 {
        warnings.push(format!("{}: memory signals are not displayed", var.name));
        return Vec::new();
    }
    let vc = unsafe { wav_fsdb_vc_handle(handle, var.idcode) };
    if vc.is_null() {
        return Vec::new();
    }
    let mut changes: Vec<Change> = Vec::new();
    if unsafe { wav_fsdb_has_vc(vc) } != 0 {
        let mut min_time = 0u64;
        if unsafe { wav_fsdb_min_time(vc, &mut min_time) } == 0
            && unsafe { wav_fsdb_goto_time(vc, min_time) } == 0
        {
            loop {
                let mut time = 0u64;
                if unsafe { wav_fsdb_cur_time(vc, &mut time) } != 0 {
                    break;
                }
                let bytes_per_bit = unsafe { wav_fsdb_bytes_per_bit(vc) };
                let bits = unsafe { wav_fsdb_bit_size(vc) } as usize;
                let size = if bytes_per_bit == 0 {
                    bits.max(1)
                } else {
                    1usize << bytes_per_bit.min(3)
                };
                let mut buf = vec![0u8; size.max(8)];
                let mut out_len = 0u64;
                if unsafe { wav_fsdb_value(vc, buf.as_mut_ptr(), buf.len() as u64, &mut out_len) }
                    == 0
                {
                    if let Some(value) = decode_value(bytes_per_bit, bits, &buf[..out_len as usize])
                    {
                        if !changes.last().map(|c| c.v == value).unwrap_or(false) {
                            changes.push(Change { t: time, v: value });
                        }
                    }
                }
                if unsafe { wav_fsdb_next_vc(vc) } != 0 {
                    break;
                }
            }
        }
    }
    unsafe { wav_fsdb_free_handle(vc) };
    changes
}

fn decode_value(bytes_per_bit: u32, bits: usize, bytes: &[u8]) -> Option<Value> {
    match bytes_per_bit {
        // one byte per bit, most significant bit first
        0 => Some(Value::Bits(
            bytes
                .iter()
                .take(bits.max(1))
                .rev()
                .map(|&b| match b {
                    0 => 0u8,
                    1 => 1u8,
                    3 => 3u8,
                    _ => 2u8,
                })
                .collect(),
        )),
        2 if bytes.len() >= 4 => {
            Some(Value::Real(
                f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64,
            ))
        }
        3 if bytes.len() >= 8 => Some(Value::Real(f64::from_ne_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))),
        _ => None,
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

fn parse_timescale(
    scale_unit: &str,
    time_unit: Option<&str>,
    warnings: &mut Vec<String>,
) -> TimeScale {
    for candidate in [scale_unit, time_unit.unwrap_or("")] {
        let text = candidate.trim();
        if text.is_empty() {
            continue;
        }
        let digits: String = text.chars().take_while(|c| c.is_ascii_digit()).collect();
        let unit: String = text
            .chars()
            .skip_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .trim()
            .to_string();
        if unit.is_empty() {
            continue;
        }
        let num = digits.parse::<u32>().unwrap_or(1).max(1);
        if let Some(ts) = TimeScale::from_unit(num, unit.as_bytes()) {
            return ts;
        }
    }
    warnings.push("unknown FSDB timescale, assuming 1ns".to_string());
    TimeScale::DEFAULT
}

fn var_type_name(var_type: u32) -> String {
    match var_type {
        0 => "event",
        1 => "integer",
        2 => "parameter",
        3 => "real",
        4 | 20 => "reg",
        5 => "supply0",
        6 => "supply1",
        7 => "time",
        8 => "tri",
        9 => "triand",
        10 => "trior",
        11 => "trireg",
        12 => "tri0",
        13 => "tri1",
        14 => "wand",
        15 => "wire",
        16 => "wor",
        17 => "memory",
        18 => "memory",
        19 => "port",
        32 => "signal",
        33 => "variable",
        _ => "var",
    }
    .to_string()
}

extern "C" fn on_scope(
    user: *mut c_void,
    name: *const c_char,
    module: *const c_char,
    time_unit: *const c_char,
    _scope_type: u32,
) {
    let collector = unsafe { &mut *(user as *mut Collector) };
    if collector.time_unit.is_none() {
        let unit = unsafe { CStr::from_ptr(time_unit) }
            .to_string_lossy()
            .into_owned();
        if !unit.trim().is_empty() {
            collector.time_unit = Some(unit);
        }
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    // The defining module of the instance, e.g. `dut` -> `counter`; empty for
    // old dumps that do not record it. This is the anchor for the RTL view.
    let module = unsafe { CStr::from_ptr(module) }
        .to_string_lossy()
        .into_owned();
    let tree = collector.tree.get_or_insert_with(ScopeTree::new);
    let parent = collector.scope_nodes.last().copied().unwrap_or(tree.root);
    let id = tree.add_scope(parent, name.clone(), module);
    collector.scope_names.push(name);
    collector.scope_nodes.push(id);
    collector.current_node = id;
}

extern "C" fn on_upscope(user: *mut c_void) {
    let collector = unsafe { &mut *(user as *mut Collector) };
    collector.scope_names.pop();
    collector.scope_nodes.pop();
    collector.current_node = collector.scope_nodes.last().copied().unwrap_or(0);
}

extern "C" fn on_var(
    user: *mut c_void,
    name: *const c_char,
    idcode: i64,
    lbit: u32,
    rbit: u32,
    _dtidcode: u32,
    var_type: u32,
    bytes_per_bit: u32,
) {
    let collector = unsafe { &mut *(user as *mut Collector) };
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    let width = lbit.max(rbit) - lbit.min(rbit) + 1;
    collector.vars.push(VarMeta {
        idcode,
        name,
        scope: collector.scope_names.clone(),
        width,
        var_type,
        bytes_per_bit,
        tree_node: collector.current_node,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn verdi_demo() -> Option<PathBuf> {
        let home = std::env::var_os("VERDI_HOME")?;
        let path = PathBuf::from(home).join("demo/dumper/modelsim_link_third_party/sample.fsdb");
        path.is_file().then_some(path)
    }

    #[test]
    fn var_type_names() {
        assert_eq!(var_type_name(15), "wire");
        assert_eq!(var_type_name(4), "reg");
        assert_eq!(var_type_name(3), "real");
    }

    #[test]
    fn timescale_parsing() {
        let mut warnings = Vec::new();
        assert_eq!(parse_timescale("1 ns", None, &mut warnings).label(), "1ns");
        assert_eq!(
            parse_timescale("", Some("ps"), &mut warnings).label(),
            "1ps"
        );
        assert_eq!(
            parse_timescale("10ns", Some("ps"), &mut warnings).label(),
            "10ns"
        );
    }

    #[test]
    fn parses_verdi_demo_fsdb() {
        let Some(path) = verdi_demo() else {
            return;
        };
        let out = parse_fsdb(&path).expect("demo fsdb should parse");
        assert!(!out.wf.signals.is_empty(), "expected signals");
        assert!(out.wf.total_ticks() > 0, "expected a time range");
        assert!(out.wf.signals.iter().any(|s| !s.changes.is_empty()));
    }

    /// Cross-check the FFR based reader against Verdi's own `fsdb2vcd`
    /// converter: both must produce exactly the same signal values.
    #[test]
    fn fsdb_matches_fsdb2vcd_conversion() {
        use std::collections::HashMap;
        use std::process::{Command, Stdio};

        let Some(home) = std::env::var_os("VERDI_HOME") else {
            return;
        };
        let home = PathBuf::from(home);
        let fsdb = home.join("demo/nCompare/nCmp_demo1/demo_RTL_verilog.fsdb");
        let fsdb2vcd = home.join("platform/LINUXAMD64/bin/fsdb2vcd");
        if !fsdb.is_file() || !fsdb2vcd.is_file() {
            return;
        }

        let vcd_path =
            std::env::temp_dir().join(format!("waverdi_fsdb_check_{}.vcd", std::process::id()));
        let status = Command::new(&fsdb2vcd)
            .arg(&fsdb)
            .arg("-o")
            .arg(&vcd_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let Ok(status) = status else { return };
        if !status.success() {
            let _ = std::fs::remove_file(&vcd_path);
            return;
        }

        let fsdb_out = parse_fsdb(&fsdb).expect("fsdb parse");
        let vcd_out = crate::vcd::parse_vcd(&vcd_path).expect("vcd parse");
        let _ = std::fs::remove_file(&vcd_path);

        assert_eq!(fsdb_out.wf.ts.label(), vcd_out.wf.ts.label());
        let key = |s: &Signal| {
            let base = s.name.split('[').next().unwrap_or(&s.name);
            format!("{}.{}", s.scope.join("."), base)
        };
        let fsdb_signals: HashMap<String, &Signal> =
            fsdb_out.wf.signals.iter().map(|s| (key(s), s)).collect();
        let mut checked = 0;
        for vcd_signal in &vcd_out.wf.signals {
            let name = key(vcd_signal);
            let Some(fsdb_signal) = fsdb_signals.get(&name) else {
                panic!("signal {name} missing from FSDB reader output");
            };
            assert_eq!(
                fsdb_signal.changes.len(),
                vcd_signal.changes.len(),
                "change count differs for {name}"
            );
            for (a, b) in fsdb_signal.changes.iter().zip(&vcd_signal.changes) {
                assert_eq!(a.t, b.t, "time differs for {name}");
                assert_eq!(a.v, b.v, "value differs for {name} at {}", a.t);
            }
            checked += 1;
        }
        assert!(checked > 0, "no signals compared");
    }
}
