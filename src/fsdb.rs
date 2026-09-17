//! FSDB loader built on the Synopsys FSDB Reader (FFR) SDK.
//!
//! The FFR library only exports C++ symbols, so `csrc/ffr_bridge.cpp` is
//! compiled against the official headers and exposes the plain C functions
//! used here. `build.rs` only enables this module (`cfg(fsdb_sdk)`) when
//! `VERDI_HOME` points at an installation shipping `share/FsdbReader`.

use crate::dump::{
    ParseOut, MAX_CHANGES_PER_SIGNAL, MAX_CHANGES_READ_PER_SIGNAL, MAX_TOTAL_CHANGES,
};
use crate::waveform::{Change, ScopeTree, SigKind, SigState, Signal, TimeScale, Value, Waveform};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;

type ScopeCb = extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char, u32);
type VarCb = extern "C" fn(*mut c_void, *const c_char, i64, u32, u32, u32, u32, u32, u32);
type UpscopeCb = extern "C" fn(*mut c_void);
type GroupBeginCb = extern "C" fn(*mut c_void, *const c_char, u32);

extern "C" {
    fn wav_fsdb_is_fsdb(path: *const c_char) -> i32;
    fn wav_fsdb_open(
        path: *const c_char,
        scope: ScopeCb,
        var: VarCb,
        upscope: UpscopeCb,
        group_begin: GroupBeginCb,
        group_end: UpscopeCb,
        user: *mut c_void,
    ) -> *mut c_void;
    fn wav_fsdb_read_tree(handle: *mut c_void) -> i32;
    fn wav_fsdb_add_signal(handle: *mut c_void, idcode: i64) -> i32;
    fn wav_fsdb_load_signals(handle: *mut c_void) -> i32;
    fn wav_fsdb_reset_signal_list(handle: *mut c_void) -> i32;
    fn wav_fsdb_unload_signals(handle: *mut c_void) -> i32;
    fn wav_fsdb_read_changes(
        vc: *mut c_void,
        times: *mut u64,
        values: *mut u8,
        lengths: *mut u64,
        capacity: u64,
        stride: u64,
        count: *mut u64,
    ) -> i32;
    fn wav_fsdb_time_range(handle: *mut c_void, min: *mut u64, max: *mut u64) -> i32;
    fn wav_fsdb_scale_unit(handle: *mut c_void, buf: *mut c_char, len: u64) -> i32;
    fn wav_fsdb_close(handle: *mut c_void);
    fn wav_fsdb_vc_handle(handle: *mut c_void, idcode: i64) -> *mut c_void;
    fn wav_fsdb_has_vc(vc: *mut c_void) -> i32;
    fn wav_fsdb_min_time(vc: *mut c_void, t: *mut u64) -> i32;
    fn wav_fsdb_goto_time(vc: *mut c_void, t: u64) -> i32;
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
    /// `fsdbVarDirection`: 1 input, 2 output, 3 inout, else implicit.
    direction: u32,
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

/// Non-Unix builds do not need the fd dance; the bridge only ships on Linux.
#[cfg(not(unix))]
struct StderrSilencer;

#[cfg(not(unix))]
impl StderrSilencer {
    fn new() -> Self {
        Self
    }
}

/// FFR keeps process-global state (active object, message hooks), so all
/// access is serialized even when parsing from multiple threads.
static FFR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg_attr(not(test), allow(dead_code))]
pub fn parse_fsdb(path: &Path) -> Result<ParseOut, String> {
    parse_fsdb_with(path, &mut |_| true)
}

pub fn parse_fsdb_with(path: &Path, progress: crate::dump::Progress) -> Result<ParseOut, String> {
    // FFR keeps process-global state: the lock must span the whole eager
    // parse, including the value reads in `assemble`. Two loads (e.g. the
    // user opening a second dump while the first is still parsing) would
    // otherwise race inside FFR and crash.
    let _lock = FFR_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let _silence = StderrSilencer::new();
    let mut input = open_input(path, true)?;
    let mut budget = MAX_TOTAL_CHANGES;
    let result = assemble(
        &mut input,
        &mut |handle, var, warnings, budget| read_changes(handle, var, warnings, budget),
        &mut budget,
        progress,
    );
    unsafe { wav_fsdb_close(input.handle) };
    result
}

/// Lazy variant used by the background loader: only the hierarchy is read
/// eagerly; value changes are fetched per signal through [`FsdbSession`].
pub fn parse_fsdb_lazy(
    path: &Path,
    progress: crate::dump::Progress,
) -> Result<(ParseOut, FsdbSession), String> {
    let mut input = {
        let _lock = FFR_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let _silence = StderrSilencer::new();
        open_input(path, false)?
    };
    let mut budget = MAX_TOTAL_CHANGES;
    let mut out = match assemble(
        &mut input,
        &mut |_, _, _, _| Vec::new(),
        &mut budget,
        progress,
    ) {
        Ok(out) => out,
        Err(err) => {
            unsafe { wav_fsdb_close(input.handle) };
            return Err(err);
        }
    };
    // Signalled below by the loader: no values were materialized.
    for signal in &mut out.wf.signals {
        signal.state = SigState::Lazy;
    }
    let vars = std::mem::take(&mut input.collector.vars);
    Ok((
        out,
        FsdbSession {
            handle: input.handle,
            vars,
            budget,
        },
    ))
}

/// An open FSDB file that can serve value changes on demand. Lives on the
/// loader thread; [`Drop`] closes the FFR handle there.
pub struct FsdbSession {
    handle: *mut c_void,
    vars: Vec<VarMeta>,
    budget: u64,
}

impl FsdbSession {
    pub fn var_count(&self) -> usize {
        self.vars.len()
    }

    /// Read the value changes of one dump variable.
    pub fn read_signal(&mut self, index: usize) -> (Vec<Change>, Vec<String>) {
        let Some(var) = self.vars.get(index) else {
            return (Vec::new(), Vec::new());
        };
        let _lock = FFR_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let _silence = StderrSilencer::new();
        let mut warnings = Vec::new();
        // The FFR signal list accumulates and `ffrLoadSignals` loads every
        // signal in it, so a read only stays cheap if the previous signal is
        // unloaded and the list reset first: otherwise each new signal would
        // reload all previously added ones (quadratic in the signals added).
        unsafe {
            wav_fsdb_unload_signals(self.handle);
            wav_fsdb_reset_signal_list(self.handle);
            wav_fsdb_add_signal(self.handle, var.idcode);
        }
        if unsafe { wav_fsdb_load_signals(self.handle) } != 0 {
            warnings.push(format!("{}: failed to load signal values", var.name));
            return (Vec::new(), warnings);
        }
        let changes = read_changes(self.handle, var, &mut warnings, &mut self.budget);
        (changes, warnings)
    }
}

impl Drop for FsdbSession {
    fn drop(&mut self) {
        let _lock = FFR_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { wav_fsdb_close(self.handle) };
    }
}

/// An opened dump: FFR handle plus the collected hierarchy.
struct Input {
    handle: *mut c_void,
    collector: Collector,
    ts: TimeScale,
    min_time: u64,
    max_time: u64,
    have_range: bool,
    warnings: Vec<String>,
}

/// Open the dump and collect its hierarchy. Callers must hold [`FFR_LOCK`]
/// and a [`StderrSilencer`]; [`read_signal`](FsdbSession::read_signal) and
/// the session teardown take them per call.
fn open_input(path: &Path, load_values: bool) -> Result<Input, String> {
    let display = path.display().to_string();
    let cpath =
        CString::new(display.clone()).map_err(|_| format!("{display}: path contains NUL"))?;

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
            on_group_begin,
            on_upscope,
            &mut collector as *mut Collector as *mut c_void,
        )
    };
    if handle.is_null() {
        return Err(format!("{display}: failed to open FSDB file (FFR)"));
    }

    // The callbacks write through a raw pointer to `collector`, so it must
    // not move until the tree has been read.
    if unsafe { wav_fsdb_read_tree(handle) } != 0 {
        unsafe { wav_fsdb_close(handle) };
        return Err(format!("{display}: failed to read the FSDB hierarchy"));
    }
    if load_values {
        for var in &collector.vars {
            unsafe { wav_fsdb_add_signal(handle, var.idcode) };
        }
        if unsafe { wav_fsdb_load_signals(handle) } != 0 {
            unsafe { wav_fsdb_close(handle) };
            return Err(format!("{display}: failed to load signal values"));
        }
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
    let mut warnings = std::mem::take(&mut collector.warnings);
    let ts = parse_timescale(&scale_unit, collector.time_unit.as_deref(), &mut warnings);

    Ok(Input {
        handle,
        collector,
        ts,
        min_time,
        max_time,
        have_range,
        warnings,
    })
}

/// Build the signal list from the collected variables, calling `read` for
/// each variable's value changes.
fn assemble(
    input: &mut Input,
    read: &mut dyn FnMut(*mut c_void, &VarMeta, &mut Vec<String>, &mut u64) -> Vec<Change>,
    budget: &mut u64,
    progress: crate::dump::Progress,
) -> Result<ParseOut, String> {
    let mut warnings = std::mem::take(&mut input.warnings);
    let mut tree = input.collector.tree.take().unwrap_or_default();
    tree.remove_scopes_named(&["$attribute_root", "$interconnect_root"]);
    let mut signals = Vec::with_capacity(input.collector.vars.len());
    let mut start = u64::MAX;
    let mut end = 0u64;
    let total_vars = input.collector.vars.len();
    for (index, var) in input.collector.vars.iter().enumerate() {
        let changes = read(input.handle, var, &mut warnings, budget);
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
            dir: direction_name(var.direction),
            scope: var.scope.clone(),
            kind,
            changes,
            min,
            max,
            parent: None,
            members: Vec::new(),
            state: SigState::Ready,
        };
        let signal_index = signals.len();
        signals.push(signal);
        tree.nodes[var.tree_node].signals.push(signal_index);
        if index % 64 == 0 || index + 1 == total_vars {
            let done = index + 1;
            if !progress(crate::dump::LoadProgress {
                stage: crate::dump::Stage::Waveform,
                done,
                total: total_vars,
                changes: MAX_TOTAL_CHANGES - *budget,
            }) {
                return Err("load cancelled".to_string());
            }
        }
    }
    if start == u64::MAX {
        start = input.min_time;
    }
    if input.have_range {
        start = start.min(input.min_time);
        end = end.max(input.max_time);
    }

    Ok(ParseOut {
        wf: Waveform {
            ts: input.ts,
            start,
            end,
            signals,
            tree,
        },
        warnings,
    })
}

fn read_changes(
    handle: *mut c_void,
    var: &VarMeta,
    warnings: &mut Vec<String>,
    budget: &mut u64,
) -> Vec<Change> {
    if var.var_type == 17 || var.var_type == 18 {
        warnings.push(format!("{}: memory signals are not displayed", var.name));
        return Vec::new();
    }
    if *budget == 0 {
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
            let bytes_per_bit = unsafe { wav_fsdb_bytes_per_bit(vc) };
            let bits = unsafe { wav_fsdb_bit_size(vc) } as usize;
            let size = if bytes_per_bit == 0 {
                bits.max(1)
            } else {
                1usize << bytes_per_bit.min(3)
            };
            // Read in chunks: one FFI call per chunk instead of three per
            // change, with the decoding left to Rust.
            const CHUNK: usize = 4096;
            let stride = size.max(8);
            let mut times = vec![0u64; CHUNK];
            let mut lengths = vec![0u64; CHUNK];
            let mut buf = vec![0u8; CHUNK * stride];
            'read: loop {
                let mut count = 0u64;
                let rc = unsafe {
                    wav_fsdb_read_changes(
                        vc,
                        times.as_mut_ptr(),
                        buf.as_mut_ptr(),
                        lengths.as_mut_ptr(),
                        CHUNK as u64,
                        stride as u64,
                        &mut count,
                    )
                };
                if rc != 0 || count == 0 {
                    break;
                }
                for k in 0..count as usize {
                    let start = k * stride;
                    let bytes = &buf[start..start + lengths[k] as usize];
                    if let Some(value) = decode_value(bytes_per_bit, bits, bytes) {
                        if !changes.last().map(|c| c.v == value).unwrap_or(false) {
                            changes.push(Change {
                                t: times[k],
                                v: value,
                            });
                        }
                    }
                    if changes.len() >= MAX_CHANGES_READ_PER_SIGNAL {
                        warnings.push(format!(
                            "{}: value changes truncated at {} while reading",
                            var.name, MAX_CHANGES_READ_PER_SIGNAL
                        ));
                        break 'read;
                    }
                }
                if count < CHUNK as u64 {
                    break;
                }
            }
        }
    }
    unsafe { wav_fsdb_free_handle(vc) };

    let mut changes =
        crate::dump::limit_changes(&var.name, changes, MAX_CHANGES_PER_SIGNAL, budget, warnings);
    changes.shrink_to_fit();
    changes
}

fn decode_value(bytes_per_bit: u32, bits: usize, bytes: &[u8]) -> Option<Value> {
    match bytes_per_bit {
        // one byte per bit, most significant bit first
        0 => Some(Value::compact(
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

/// `fsdbVarDirection` -> RTL port direction, empty for implicit variables.
fn direction_name(direction: u32) -> String {
    match direction {
        1 => "input",
        2 => "output",
        3 => "inout",
        4 => "buffer",
        5 => "linkage",
        _ => "",
    }
    .to_string()
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

/// SV struct/union (or VHDL record) grouping: the fields that follow belong
/// to a scope named after the variable, so `clk_fetch.run` becomes a member
/// `run` of the `clk_fetch` scope, like Verdi shows it.
extern "C" fn on_group_begin(user: *mut c_void, name: *const c_char, _field_count: u32) {
    let collector = unsafe { &mut *(user as *mut Collector) };
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    let tree = collector.tree.get_or_insert_with(ScopeTree::new);
    if name.is_empty() {
        // Keep the scope stack balanced for the matching group end.
        let current = collector.current_node;
        collector.scope_names.push(String::new());
        collector.scope_nodes.push(current);
        return;
    }
    let parent = collector.scope_nodes.last().copied().unwrap_or(tree.root);
    let id = tree.add_scope(parent, name.clone(), String::new());
    tree.nodes[id].group = true;
    collector.scope_names.push(name);
    collector.scope_nodes.push(id);
    collector.current_node = id;
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
    direction: u32,
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
        direction,
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
