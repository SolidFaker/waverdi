use crate::dump::ParseOut;
use crate::waveform::{
    Change, ScopeTree, SigKind, SigState, Signal, Ticks, TimeScale, Value, Waveform,
};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

pub fn parse_vcd(path: &Path) -> Result<ParseOut, String> {
    let data = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    parse_bytes(&data)
}

fn str(b: &[u8]) -> &str {
    std::str::from_utf8(b).unwrap_or("")
}

fn parse_u64(b: &[u8]) -> Result<u64, String> {
    str(b)
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("bad integer: {:?}", str(b)))
}

fn parse_u32(b: &[u8]) -> Result<u32, String> {
    str(b)
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("bad integer: {:?}", str(b)))
}

fn split_word(b: &[u8]) -> (&[u8], &[u8]) {
    let b = b.trim_ascii();
    if let Some(i) = b.iter().position(|c| c.is_ascii_whitespace()) {
        (&b[..i], b[i..].trim_ascii())
    } else {
        (b, &[])
    }
}

fn split_ws(b: &[u8]) -> Vec<&[u8]> {
    b.split(|c| c.is_ascii_whitespace())
        .filter(|s| !s.is_empty())
        .collect()
}

fn split_value_id(s: &[u8]) -> (&[u8], &[u8]) {
    let mut k = 0;
    while k < s.len() && !s[k].is_ascii_whitespace() {
        k += 1;
    }
    (&s[..k], s[k..].trim_ascii())
}

/// Split a real value from its id code. Real literals are not length
/// delimited, so they end at the first character that cannot be part of a
/// number (`r1.25$`, `r-2.5 $`, ...).
fn split_real(s: &[u8]) -> (&[u8], &[u8]) {
    let mut k = 0;
    while k < s.len() && (s[k].is_ascii_digit() || matches!(s[k], b'.' | b'+' | b'-' | b'e' | b'E'))
    {
        k += 1;
    }
    (&s[..k], s[k..].trim_ascii())
}

/// Accept both `1ns` and `1 ns` forms of `$timescale`.
fn parse_timescale(toks: &[&[u8]]) -> Option<TimeScale> {
    let first = toks.first()?;
    let split = first.iter().position(|c| c.is_ascii_alphabetic());
    let (num, unit) = match split {
        Some(i) => (&first[..i], &first[i..]),
        None => (*first, *toks.get(1)?),
    };
    TimeScale::from_unit(parse_u32(num).ok()?, unit)
}

fn contains_end(line: &[u8]) -> bool {
    line.windows(4).any(|w| w == b"$end")
}

fn skip_to_end(lines: &[&[u8]], i: usize) -> usize {
    for (k, line) in lines.iter().enumerate().skip(i) {
        if contains_end(line) {
            return k;
        }
    }
    lines.len()
}

fn parse_bin(d: &[u8]) -> Vec<u8> {
    let mut bits = Vec::with_capacity(d.len());
    for &c in d {
        match c {
            b'0' => bits.push(0),
            b'1' => bits.push(1),
            b'x' | b'X' => bits.push(2),
            b'z' | b'Z' => bits.push(3),
            b'_' => {}
            _ => bits.push(2),
        }
    }
    bits.reverse();
    bits
}

fn parse_group(d: &[u8], grp: usize) -> Vec<u8> {
    let mut bits = Vec::with_capacity(d.len() * grp);
    for &c in d {
        let nib: Option<u8> = match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            b'x' | b'X' => {
                bits.extend(std::iter::repeat_n(2, grp));
                continue;
            }
            b'z' | b'Z' => {
                bits.extend(std::iter::repeat_n(3, grp));
                continue;
            }
            _ => None,
        };
        if let Some(v) = nib {
            for k in (0..grp).rev() {
                bits.push((v >> k) & 1);
            }
        }
    }
    bits.reverse();
    bits
}

/// Parse a decimal value. `None` reports a malformed value (a sign or any
/// character other than the decimal digits and `x`/`z`), which the caller
/// warns about; an empty vector means all-`x` digits, padded once the signal
/// width is known.
fn parse_dec(d: &[u8]) -> Option<Vec<u8>> {
    let mut v: u128 = 0;
    let mut ok = true;
    for &c in d {
        match c {
            b'0'..=b'9' => {
                v = v
                    .checked_mul(10)
                    .and_then(|x| x.checked_add((c - b'0') as u128))
                    .unwrap_or_else(|| {
                        ok = false;
                        0
                    });
            }
            b'x' | b'X' | b'z' | b'Z' => {
                ok = false;
                break;
            }
            _ => return None,
        }
    }
    if !ok {
        return Some(Vec::new()); // padded to all-x later
    }
    let mut bits = Vec::new();
    while v > 0 {
        bits.push((v & 1) as u8);
        v >>= 1;
    }
    Some(bits)
}

/// One decoded line of the value section. The lexer is shared by the eager
/// parser and the lazy value scanner, so both read a dump the same way.
enum Decoded<'a> {
    Change {
        id: &'a str,
        value: Value,
    },
    BadDecimal(&'a [u8]),
    BadReal(&'a [u8]),
    CharValue,
    Unparsed,
    /// Empty line or a string value without a closing quote: nothing to do.
    Ignored,
}

fn decode_value_line(ln: &[u8]) -> Decoded<'_> {
    if ln.is_empty() {
        return Decoded::Ignored;
    }
    match ln[0] {
        b'0' | b'1' | b'x' | b'X' | b'z' | b'Z' => {
            let v = match ln[0] {
                b'0' => 0u8,
                b'1' => 1u8,
                b'x' | b'X' => 2u8,
                _ => 3u8,
            };
            Decoded::Change {
                id: str(ln[1..].trim_ascii()),
                value: Value::Bits(vec![v]),
            }
        }
        b'b' | b'B' => {
            let (digits, id) = split_value_id(&ln[1..]);
            Decoded::Change {
                id: str(id),
                value: Value::Bits(parse_bin(digits)),
            }
        }
        b'o' | b'O' => {
            let (digits, id) = split_value_id(&ln[1..]);
            Decoded::Change {
                id: str(id),
                value: Value::Bits(parse_group(digits, 3)),
            }
        }
        b'h' | b'H' | b't' | b'T' => {
            let (digits, id) = split_value_id(&ln[1..]);
            Decoded::Change {
                id: str(id),
                value: Value::Bits(parse_group(digits, 4)),
            }
        }
        b'd' | b'D' => {
            let (digits, id) = split_value_id(&ln[1..]);
            match parse_dec(digits) {
                Some(bits) => Decoded::Change {
                    id: str(id),
                    value: Value::Bits(bits),
                },
                None => Decoded::BadDecimal(digits),
            }
        }
        b'r' | b'R' => {
            let (num, id) = split_real(&ln[1..]);
            match str(num).parse::<f64>() {
                Ok(v) => Decoded::Change {
                    id: str(id),
                    value: Value::Real(v),
                },
                Err(_) => Decoded::BadReal(num),
            }
        }
        b's' | b'S' => {
            let rest = &ln[1..];
            let Some(q0) = rest.iter().position(|&ch| ch == b'"') else {
                return Decoded::Ignored;
            };
            let Some(q1) = rest[q0 + 1..].iter().position(|&ch| ch == b'"') else {
                return Decoded::Ignored;
            };
            let q1 = q0 + 1 + q1;
            Decoded::Change {
                id: str(rest[q1 + 1..].trim_ascii()),
                value: Value::Str(String::from_utf8_lossy(&rest[q0 + 1..q1]).into_owned()),
            }
        }
        b'c' | b'C' => Decoded::CharValue,
        _ => Decoded::Unparsed,
    }
}

struct Parser {
    wf: Waveform,
    idmap: HashMap<String, usize>,
    stack: Vec<usize>,
    cur_scope: Vec<String>,
    cur_time: Ticks,
    warnings: Vec<String>,
    /// Signals that hit the per-signal read cap and only warn once.
    truncated: HashSet<usize>,
}

impl Parser {
    fn new() -> Self {
        Self {
            wf: Waveform {
                ts: TimeScale::DEFAULT,
                start: 0,
                end: 0,
                signals: Vec::new(),
                tree: ScopeTree::new(),
                radix: HashMap::new(),
                value_times_cache: Vec::new(),
            },
            idmap: HashMap::new(),
            stack: vec![0],
            cur_scope: Vec::new(),
            cur_time: 0,
            warnings: Vec::new(),
            truncated: HashSet::new(),
        }
    }

    /// Record a warning with the 1-based source line, so malformed input can
    /// be located in multi-gigabyte dumps.
    fn warn(&mut self, line: usize, message: impl Into<String>) {
        let message = message.into();
        self.warnings
            .push(format!("line {}: {}", line + 1, message));
    }

    fn directive(
        &mut self,
        kw: &[u8],
        rest: &[u8],
        lines: &[&[u8]],
        i: usize,
    ) -> Result<usize, String> {
        match kw {
            b"end" => Ok(i + 1),
            b"timescale" => {
                match parse_timescale(&split_ws(rest)) {
                    Some(ts) => self.wf.ts = ts,
                    None => self.warn(i, format!("bad $timescale: {:?}", str(rest))),
                }
                Ok((skip_to_end(lines, i)).min(lines.len()).saturating_add(1))
            }
            b"scope" => {
                let toks = split_ws(rest);
                let name = str(toks.get(1).copied().unwrap_or(b"")).to_string();
                let parent = *self.stack.last().unwrap_or(&self.wf.tree.root);
                let id = self.wf.tree.add_scope(parent, name.clone(), String::new());
                self.stack.push(id);
                self.cur_scope.push(name);
                Ok((skip_to_end(lines, i)).min(lines.len()).saturating_add(1))
            }
            b"upscope" => {
                self.stack.pop();
                self.cur_scope.pop();
                Ok((skip_to_end(lines, i)).min(lines.len()).saturating_add(1))
            }
            b"var" => {
                let toks = split_ws(rest);
                if toks.len() < 4 {
                    self.warn(i, format!("bad $var line: {:?}", str(rest)));
                    return Ok((skip_to_end(lines, i)).min(lines.len()).saturating_add(1));
                }
                let var_type = str(toks[0]).to_string();
                let size = parse_u32(toks[1]).unwrap_or(1);
                let id = str(toks[2]).to_string();
                let name = str(toks[3]).to_string();
                let kind = if var_type == "real" {
                    SigKind::Real
                } else if var_type == "string" {
                    SigKind::Str
                } else {
                    SigKind::Bits
                };
                let parent = *self.stack.last().unwrap_or(&self.wf.tree.root);
                let idx = self.wf.signals.len();
                if self.idmap.contains_key(&id) {
                    self.warn(i, format!("duplicate id code '{id}' ignored"));
                } else {
                    let sig = Signal {
                        name,
                        bits: size,
                        var_type,
                        dir: String::new(),
                        scope: self.cur_scope.clone(),
                        kind,
                        changes: Arc::new(Vec::new()),
                        min: f64::INFINITY,
                        max: f64::NEG_INFINITY,
                        parent: None,
                        members: Vec::new(),
                        state: SigState::Ready,
                    };
                    self.wf.signals.push(sig);
                    self.idmap.insert(id, idx);
                    self.wf.tree.nodes[parent].signals.push(idx);
                }
                Ok((skip_to_end(lines, i)).min(lines.len()).saturating_add(1))
            }
            b"enddefinitions" => Ok((skip_to_end(lines, i)).min(lines.len()).saturating_add(1)),
            b"dumpvars" | b"dumpoff" | b"dumpon" | b"dumpall" => {
                let mut k = i + 1;
                while k < lines.len() && !contains_end(lines[k]) {
                    let ln = lines[k];
                    if !ln.is_empty() && ln[0] != b'$' {
                        self.change(ln, k);
                    }
                    k += 1;
                }
                Ok(k.min(lines.len()).saturating_add(1))
            }
            _ => Ok((skip_to_end(lines, i)).min(lines.len()).saturating_add(1)),
        }
    }

    fn push_change(&mut self, id: &str, v: Value, line: usize) {
        let Some(&idx) = self.idmap.get(id) else {
            if !id.is_empty() {
                self.warn(line, format!("unknown id code '{id}'"));
            }
            return;
        };
        // Once the read cap is hit, further changes are dropped; the
        // dispatcher decimates the kept prefix afterwards, like the FSDB
        // reader does. A pathological dump must not exhaust memory here.
        if self.truncated.contains(&idx) {
            return;
        }
        let sig = &mut self.wf.signals[idx];
        let v = match v {
            Value::Bits(mut b) => {
                let w = sig.bits as usize;
                if b.len() < w {
                    b.resize(w, 2);
                } else if b.len() > w {
                    b.truncate(w);
                }
                Value::compact(b)
            }
            other => other,
        };
        if let Some(last) = sig.changes.last() {
            if last.v == v {
                return;
            }
        }
        if let Value::Real(r) = v {
            sig.min = sig.min.min(r);
            sig.max = sig.max.max(r);
        }
        // Parsing builds the list in place and nothing shares it yet.
        let changes = Arc::make_mut(&mut sig.changes);
        changes.push(Change {
            t: self.cur_time,
            v,
        });
        if changes.len() >= crate::dump::MAX_CHANGES_READ_PER_SIGNAL {
            let name = sig.name.clone();
            self.truncated.insert(idx);
            self.warn(
                line,
                format!(
                    "{name}: value changes truncated at {} while reading",
                    crate::dump::MAX_CHANGES_READ_PER_SIGNAL
                ),
            );
        }
    }

    fn change(&mut self, ln: &[u8], line: usize) {
        match decode_value_line(ln) {
            Decoded::Change { id, value } => self.push_change(id, value, line),
            Decoded::BadDecimal(digits) => {
                self.warn(line, format!("bad decimal value: {:?}", str(digits)))
            }
            Decoded::BadReal(num) => self.warn(line, format!("bad real value: {:?}", str(num))),
            Decoded::CharValue => self.warn(
                line,
                "char-based value codes (c/C) not supported".to_string(),
            ),
            Decoded::Unparsed => self.warn(line, format!("unparsed value line: {:?}", str(ln))),
            Decoded::Ignored => {}
        }
    }
}

pub fn parse_bytes(data: &[u8]) -> Result<ParseOut, String> {
    let mut p = Parser::new();
    let lines: Vec<&[u8]> = data
        .split(|&b| b == b'\n')
        .map(|l| l.trim_ascii())
        .collect();
    let mut i = 0usize;
    while i < lines.len() {
        let ln = lines[i];
        if ln.is_empty() {
            i += 1;
            continue;
        }
        match ln[0] {
            b'#' => match parse_u64(&ln[1..]) {
                Ok(time) => {
                    p.cur_time = time;
                    i += 1;
                }
                Err(e) => {
                    // A viewer is better served by the part that parsed: keep
                    // the changes seen so far and stop reading value lines.
                    p.warn(i, format!("bad time marker: {e}"));
                    break;
                }
            },
            b'$' => {
                let (kw, rest) = split_word(&ln[1..]);
                i = p.directive(kw, rest, &lines, i)?;
            }
            _ => {
                p.change(ln, i);
                i += 1;
            }
        }
    }

    let mut start = Ticks::MAX;
    let mut end = 0u64;
    for s in &p.wf.signals {
        if let Some(c) = s.changes.first() {
            start = start.min(c.t);
        }
        if let Some(c) = s.changes.last() {
            end = end.max(c.t);
        }
    }
    if start == Ticks::MAX {
        start = 0;
    }
    end = end.max(p.cur_time);
    p.wf.start = start;
    p.wf.end = end;
    Ok(ParseOut {
        wf: p.wf,
        warnings: p.warnings,
    })
}

/// State the lazy source needs to stream the value section after the
/// declaration pass has built the hierarchy.
pub(crate) struct VcdScanState {
    /// Id code -> signal index, as collected from `$var`.
    pub idmap: HashMap<String, usize>,
    pub names: Vec<String>,
    pub bits: Vec<u32>,
    /// Where the value section starts (just past `$enddefinitions`), so a
    /// rescan can resume there instead of re-reading the declarations.
    pub value_offset: u64,
    /// 0-based line index of the first value line; warnings use it to keep
    /// the same line numbers as the eager parser.
    pub value_line: usize,
}

/// Result of the hierarchy-only pass.
pub(crate) struct VcdHierarchy {
    pub out: ParseOut,
    pub scan: VcdScanState,
}

/// Read only the declarations (up to `$enddefinitions`); every signal comes
/// back empty and `Lazy`. The value section is left for on-demand scans.
pub(crate) fn read_vcd_hierarchy(path: &Path) -> Result<VcdHierarchy, String> {
    let (decl, value_offset, value_line) = read_declarations(path)?;

    let mut p = Parser::new();
    let lines: Vec<&[u8]> = decl
        .split(|&b| b == b'\n')
        .map(|line| line.trim_ascii())
        .collect();
    let mut i = 0usize;
    while i < lines.len() {
        let ln = lines[i];
        if ln.is_empty() {
            i += 1;
            continue;
        }
        match ln[0] {
            b'$' => {
                let (kw, rest) = split_word(&ln[1..]);
                if kw == b"enddefinitions" {
                    break;
                }
                // Value-carrying directives do not belong before
                // `$enddefinitions`; skip them without materializing values,
                // so the lazy hierarchy really starts out empty.
                if matches!(kw, b"dumpvars" | b"dumpoff" | b"dumpon" | b"dumpall") {
                    i = skip_to_end(&lines, i).min(lines.len()).saturating_add(1);
                    continue;
                }
                i = p.directive(kw, rest, &lines, i)?;
            }
            // Value lines never precede `$enddefinitions`; stop if one does.
            b'#' => break,
            _ => i += 1,
        }
    }

    let mut names = Vec::with_capacity(p.wf.signals.len());
    let mut bits = Vec::with_capacity(p.wf.signals.len());
    for signal in &mut p.wf.signals {
        signal.state = SigState::Lazy;
        names.push(signal.name.clone());
        bits.push(signal.bits);
    }
    let idmap = std::mem::take(&mut p.idmap);
    // The declaration pass cannot know the time range; a bounded tail read
    // finds the last `#` marker so the UI has a usable range immediately.
    p.wf.start = 0;
    p.wf.end = last_time_marker(path).unwrap_or(0);
    Ok(VcdHierarchy {
        out: ParseOut {
            wf: p.wf,
            warnings: p.warnings,
        },
        scan: VcdScanState {
            idmap,
            names,
            bits,
            value_offset,
            value_line,
        },
    })
}

/// Read the file up to and including the `$enddefinitions` line; returns the
/// declaration bytes plus where the value section and its first line start.
fn read_declarations(path: &Path) -> Result<(Vec<u8>, u64, usize), String> {
    let display = path.display();
    let file = File::open(path).map_err(|e| format!("cannot read {display}: {e}"))?;
    let mut reader = BufReader::new(file);
    let mut decl = Vec::new();
    let mut buf = Vec::new();
    let mut offset = 0u64;
    let mut lines = 0usize;
    loop {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("cannot read {display}: {e}"))?;
        if n == 0 {
            break;
        }
        decl.extend_from_slice(&buf);
        offset += n as u64;
        lines += 1;
        if contains_word(&buf, b"$enddefinitions") {
            break;
        }
    }
    Ok((decl, offset, lines))
}

fn contains_word(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Last `#<time>` line in a bounded tail window. VCD has no time table, so
/// this is the cheap way to give the UI the full range up front; a value
/// scan only finds the same markers again.
fn last_time_marker(path: &Path) -> Option<Ticks> {
    const WINDOW: u64 = 256 * 1024;
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(WINDOW);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;

    let mut body = tail.as_slice();
    // A bounded window may start inside a line; drop that partial line.
    if start > 0 {
        if let Some(pos) = body.iter().position(|&b| b == b'\n') {
            body = &body[pos + 1..];
        }
    }
    for line in body.split(|&b| b == b'\n').rev() {
        let line = line.trim_ascii();
        if line.first() == Some(&b'#') {
            if let Ok(time) = parse_u64(&line[1..]) {
                return Some(time);
            }
        }
    }
    None
}

/// Changes collected for the requested signals plus the warnings the scan
/// produced.
pub(crate) struct VcdScanOut {
    /// `(signal index, changes)`, one entry per requested signal.
    pub changes: Vec<(usize, Vec<Change>)>,
    /// Warnings tied to a signal (its read cap was hit).
    pub signal_warnings: HashMap<usize, Vec<String>>,
    /// File-level warnings (bad values, unknown ids, malformed time markers).
    pub warnings: Vec<String>,
}

/// Collect changes for `targets` by streaming the value section from its
/// start. Every call re-reads the section - VCD has no index, so a rescan is
/// the price of asking for signals that were not collected before.
pub(crate) fn scan_vcd_values(
    path: &Path,
    state: &VcdScanState,
    targets: &[usize],
) -> Result<VcdScanOut, String> {
    scan_vcd_values_with_cap(
        path,
        state,
        targets,
        crate::dump::MAX_CHANGES_READ_PER_SIGNAL,
    )
}

fn scan_vcd_values_with_cap(
    path: &Path,
    state: &VcdScanState,
    targets: &[usize],
    cap: usize,
) -> Result<VcdScanOut, String> {
    let display = path.display();
    let file = File::open(path).map_err(|e| format!("cannot read {display}: {e}"))?;
    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(state.value_offset))
        .map_err(|e| format!("cannot read {display}: {e}"))?;
    let mut lines = LineReader::new(reader, state.value_line);
    let mut scanner = VcdScanner::new(state, targets, cap);
    scanner.run(&mut lines)?;
    Ok(scanner.finish())
}

/// Streams the value section, collecting only the requested signals. The
/// per-signal read cap mirrors the eager parser: once hit, later changes of
/// that signal are dropped and one line-numbered warning is emitted.
struct VcdScanner<'a> {
    state: &'a VcdScanState,
    targets: Vec<bool>,
    changes: HashMap<usize, Vec<Change>>,
    signal_warnings: HashMap<usize, Vec<String>>,
    warnings: Vec<String>,
    truncated: HashSet<usize>,
    cur_time: Ticks,
    cap: usize,
}

impl<'a> VcdScanner<'a> {
    fn new(state: &'a VcdScanState, targets: &[usize], cap: usize) -> Self {
        let mut wanted = vec![false; state.names.len()];
        for &index in targets {
            if let Some(slot) = wanted.get_mut(index) {
                *slot = true;
            }
        }
        Self {
            state,
            targets: wanted,
            changes: HashMap::new(),
            signal_warnings: HashMap::new(),
            warnings: Vec::new(),
            truncated: HashSet::new(),
            cur_time: 0,
            cap,
        }
    }

    /// Record a warning with the 1-based source line, like the eager parser.
    fn warn(&mut self, line: usize, message: impl Into<String>) {
        let message = message.into();
        self.warnings
            .push(format!("line {}: {}", line + 1, message));
    }

    fn run<R: BufRead>(&mut self, lines: &mut LineReader<R>) -> Result<(), String> {
        let mut buf = Vec::new();
        let mut inner = Vec::new();
        while let Some(line) = lines.next(&mut buf)? {
            if buf.is_empty() {
                continue;
            }
            match buf[0] {
                b'#' => match parse_u64(&buf[1..]) {
                    Ok(time) => self.cur_time = time,
                    Err(e) => {
                        // A viewer is better served by the part that parsed:
                        // keep the changes seen so far and stop, exactly like
                        // the eager parser.
                        self.warn(line, format!("bad time marker: {e}"));
                        break;
                    }
                },
                b'$' => {
                    let (kw, _) = split_word(&buf[1..]);
                    self.directive(kw, &buf, lines, &mut inner)?;
                }
                _ => self.change(&buf, line),
            }
        }
        Ok(())
    }

    fn directive<R: BufRead>(
        &mut self,
        kw: &[u8],
        ln: &[u8],
        lines: &mut LineReader<R>,
        buf: &mut Vec<u8>,
    ) -> Result<(), String> {
        match kw {
            b"end" => {}
            b"dumpvars" | b"dumpoff" | b"dumpon" | b"dumpall" => {
                while let Some(line) = lines.next(buf)? {
                    if contains_end(buf) {
                        break;
                    }
                    if !buf.is_empty() && buf[0] != b'$' {
                        self.change(buf, line);
                    }
                }
            }
            _ => {
                // Directives may span lines; skip to the closing `$end`.
                if !contains_end(ln) {
                    while lines.next(buf)?.is_some() {
                        if contains_end(buf) {
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn change(&mut self, ln: &[u8], line: usize) {
        match decode_value_line(ln) {
            Decoded::Change { id, value } => self.push_change(id, value, line),
            Decoded::BadDecimal(digits) => {
                self.warn(line, format!("bad decimal value: {:?}", str(digits)))
            }
            Decoded::BadReal(num) => self.warn(line, format!("bad real value: {:?}", str(num))),
            Decoded::CharValue => self.warn(
                line,
                "char-based value codes (c/C) not supported".to_string(),
            ),
            Decoded::Unparsed => self.warn(line, format!("unparsed value line: {:?}", str(ln))),
            Decoded::Ignored => {}
        }
    }

    fn push_change(&mut self, id: &str, v: Value, line: usize) {
        let Some(&idx) = self.state.idmap.get(id) else {
            if !id.is_empty() {
                self.warn(line, format!("unknown id code '{id}'"));
            }
            return;
        };
        if !self.targets.get(idx).copied().unwrap_or(false) || self.truncated.contains(&idx) {
            return;
        }
        let width = *self.state.bits.get(idx).unwrap_or(&1) as usize;
        let v = match v {
            Value::Bits(mut b) => {
                if b.len() < width {
                    b.resize(width, 2);
                } else if b.len() > width {
                    b.truncate(width);
                }
                Value::compact(b)
            }
            other => other,
        };
        let changes = self.changes.entry(idx).or_default();
        if changes.last().map(|c| c.v == v).unwrap_or(false) {
            return;
        }
        changes.push(Change {
            t: self.cur_time,
            v,
        });
        if changes.len() >= self.cap {
            let name = self.state.names.get(idx).cloned().unwrap_or_default();
            self.truncated.insert(idx);
            self.signal_warnings.entry(idx).or_default().push(format!(
                "line {}: {name}: value changes truncated at {} while reading",
                line + 1,
                self.cap
            ));
        }
    }

    fn finish(self) -> VcdScanOut {
        let changes = self
            .targets
            .iter()
            .enumerate()
            .filter(|(_, &wanted)| wanted)
            .map(|(index, _)| (index, self.changes.get(&index).cloned().unwrap_or_default()))
            .collect();
        VcdScanOut {
            changes,
            signal_warnings: self.signal_warnings,
            warnings: self.warnings,
        }
    }
}

/// Line reader tracking the 0-based source line, so warnings carry the same
/// line numbers whether the eager parser or a lazy rescan produced them.
struct LineReader<R: BufRead> {
    reader: R,
    line: usize,
}

impl<R: BufRead> LineReader<R> {
    fn new(reader: R, line: usize) -> Self {
        Self { reader, line }
    }

    fn next(&mut self, buf: &mut Vec<u8>) -> Result<Option<usize>, String> {
        buf.clear();
        let n = self
            .reader
            .read_until(b'\n', buf)
            .map_err(|e| format!("cannot read value section: {e}"))?;
        if n == 0 {
            return Ok(None);
        }
        let line = self.line;
        self.line += 1;
        trim_line(buf);
        Ok(Some(line))
    }
}

/// Trim a line like `parse_bytes` does (`trim_ascii`), in place.
fn trim_line(buf: &mut Vec<u8>) {
    let start = buf
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(buf.len());
    let end = buf
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(start);
    buf.truncate(end);
    if start > 0 {
        buf.drain(..start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::Radix;

    fn wf(s: &str) -> (Waveform, Vec<String>) {
        let out = parse_bytes(s.as_bytes()).unwrap();
        (out.wf, out.warnings)
    }

    #[test]
    fn basic() {
        let vcd = r#"
$timescale 1ns $end
$scope module top $end
$var wire 1 ! clk $end
$var reg 4 " data [3:0] $end
$upscope $end
$enddefinitions $end
#0
1!
b1010 "
#100
0!
b0101 "
#200
1!
$end
"#;
        let (w, warns) = wf(vcd);
        assert!(warns.is_empty(), "{warns:?}");
        assert_eq!(w.ts.label(), "1ns");
        assert_eq!(w.signals.len(), 2);
        assert_eq!(w.start, 0);
        assert_eq!(w.end, 200);
        let clk = &w.signals[0];
        assert_eq!(clk.name, "clk");
        assert_eq!(clk.bits, 1);
        assert_eq!(clk.scope, vec!["top".to_string()]);
        let ts: Vec<u64> = clk.changes.iter().map(|c| c.t).collect();
        assert_eq!(ts, vec![0, 100, 200]);
        let data = &w.signals[1];
        let ts: Vec<u64> = data.changes.iter().map(|c| c.t).collect();
        assert_eq!(ts, vec![0, 100]);
        let v = data.display_value(0, Radix::Hex);
        assert_eq!(v, "ha");
        let v = data.display_value(100, Radix::Hex);
        assert_eq!(v, "h5");
        assert_eq!(w.tree.nodes[0].children.len(), 1);
        assert_eq!(w.tree.nodes[1].signals.len(), 2);
    }

    #[test]
    fn dumpvars_and_dedup() {
        let vcd = r#"
$timescale 1ns $end
$var wire 1 ! clk $end
$enddefinitions $end
$dumpvars
0!
0!
0!
$end
#10
1!
$end
"#;
        let (w, warns) = wf(vcd);
        assert!(warns.is_empty(), "{warns:?}");
        let clk = &w.signals[0];
        let ts: Vec<u64> = clk.changes.iter().map(|c| c.t).collect();
        assert_eq!(ts, vec![0, 10]);
        assert_eq!(clk.value_at(0).unwrap().to_bits_vec(), Some(vec![0u8]));
        assert_eq!(clk.value_at(5).unwrap().to_bits_vec(), Some(vec![0u8]));
        assert_eq!(clk.value_at(10).unwrap().to_bits_vec(), Some(vec![1u8]));
    }

    #[test]
    fn xz_and_multi_base() {
        let vcd = r#"
$timescale 1ns $end
$var wire 1 ! a $end
$var reg 4 " b $end
$var reg 8 # c $end
$var reg 8 $ d $end
$enddefinitions $end
#0
x!
b10x1 "
hAz #
d255 $
#5
z!
$end
"#;
        let (w, _) = wf(vcd);
        assert_eq!(
            w.signals[0].value_at(0).unwrap().to_bits_vec(),
            Some(vec![2u8])
        );
        let b = w.signals[1].value_at(0).unwrap().to_bits_vec().unwrap();
        assert_eq!(b, vec![1, 2, 0, 1]);
        let c = w.signals[2].value_at(0).unwrap().to_bits_vec().unwrap();
        assert_eq!(c[0], 3); // lsb nibble z
        assert_eq!(c[4], 0); // msb nibble 0xA
        assert_eq!(c[5], 1);
        let d = w.signals[3].value_at(0).unwrap().to_bits_vec().unwrap();
        assert_eq!(d.len(), 8);
        let mut v: u64 = 0;
        for (i, &bit) in d.iter().enumerate() {
            v |= (bit as u64) << i;
        }
        assert_eq!(v, 255);
        assert_eq!(
            w.signals[0].value_at(5).unwrap().to_bits_vec(),
            Some(vec![3u8])
        );
    }

    #[test]
    fn real_and_string() {
        let vcd = r#"
$timescale 1ns $end
$var real 64 $ r0 $end
$var string 8 % s0 $end
$enddefinitions $end
#0
r1.25$
s"hello"%
#5
r-2.5$
$end
"#;
        let (w, _) = wf(vcd);
        assert_eq!(w.signals[0].value_at(0).unwrap().as_real(), Some(1.25));
        assert_eq!(w.signals[0].value_at(5).unwrap().as_real(), Some(-2.5));
        assert_eq!(w.signals[0].min, -2.5);
        assert_eq!(w.signals[0].max, 1.25);
        assert_eq!(w.signals[1].display_value(0, Radix::Ascii), "hello");
        assert_eq!(w.signals[1].display_value(5, Radix::Ascii), "hello");
    }

    #[test]
    fn negative_decimals_are_warned_and_skipped() {
        let vcd = r#"
$timescale 1ns $end
$var reg 8 " d $end
$enddefinitions $end
#0
d3 "
#5
d-5 "
$end
"#;
        let (w, warns) = wf(vcd);
        assert!(
            warns.iter().any(|w| w.contains("bad decimal value")),
            "{warns:?}"
        );
        // The malformed value is dropped instead of silently becoming 5.
        let ts: Vec<u64> = w.signals[0].changes.iter().map(|c| c.t).collect();
        assert_eq!(ts, vec![0]);
    }

    #[test]
    fn padding_and_truncation() {
        let vcd = r#"
$timescale 1ns $end
$var reg 8 " d $end
$var reg 2 # e $end
$enddefinitions $end
#0
b1 "
b101 #
$end
"#;
        let (w, _) = wf(vcd);
        let d = w.signals[0].value_at(0).unwrap().to_bits_vec().unwrap();
        assert_eq!(d, vec![1, 2, 2, 2, 2, 2, 2, 2]);
        let e = w.signals[1].value_at(0).unwrap().to_bits_vec().unwrap();
        assert_eq!(e, vec![1, 0]);
    }

    #[test]
    fn nested_scopes() {
        let vcd = r#"
$timescale 1ns $end
$scope module top $end
$var wire 1 ! a $end
$scope module sub $end
$var wire 1 " b $end
$upscope $end
$var wire 1 # c $end
$upscope $end
$enddefinitions $end
$end
"#;
        let (w, _) = wf(vcd);
        assert_eq!(
            w.signals[1].scope,
            vec!["top".to_string(), "sub".to_string()]
        );
        assert_eq!(w.signals[1].full_name(), "top.sub.b");
        let design = &w.tree.nodes[0];
        assert_eq!(design.children.len(), 1);
        let top = &w.tree.nodes[1];
        assert_eq!(top.name, "top");
        assert_eq!(top.children.len(), 1);
        assert_eq!(top.signals, vec![0, 2]);
        let sub = &w.tree.nodes[2];
        assert_eq!(sub.name, "sub");
        assert_eq!(sub.signals, vec![1]);
    }

    #[test]
    fn timescale_variants() {
        let (w, _) = wf("$timescale 100 ps $end $enddefinitions $end");
        assert_eq!(w.ts.label(), "100ps");
        let (w, _) = wf("$timescale 10 ns $end $enddefinitions $end");
        assert_eq!(w.ts.label(), "10ns");
    }

    #[test]
    fn multi_line_comment() {
        let vcd = "$comment\nline one\nline two\n$end\n$timescale 1ns $end\n$enddefinitions $end\n";
        let (w, warns) = wf(vcd);
        assert!(warns.is_empty(), "{warns:?}");
        assert_eq!(w.ts.label(), "1ns");
    }

    #[test]
    fn unknown_id_warns() {
        let vcd = "$timescale 1ns $end\n$var wire 1 ! a $end\n$enddefinitions $end\n#0\n1?\n";
        let (_, warns) = wf(vcd);
        assert!(!warns.is_empty());
    }

    #[test]
    fn empty_input_is_ok_without_signals_or_warnings() {
        let out = parse_bytes(b"").expect("empty input parses");
        assert!(out.wf.signals.is_empty());
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.wf.start, 0);
        assert_eq!(out.wf.end, 0);
    }

    #[test]
    fn bad_time_marker_keeps_the_parsed_prefix() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 1 ! a $end\n\
            $enddefinitions $end\n\
            #0\n1!\n#10\n0!\n#abc\n#20\n1!\n";
        let (w, warns) = wf(vcd);
        let line = vcd.lines().position(|l| l == "#abc").unwrap() + 1;
        assert!(
            warns
                .iter()
                .any(|w| w.contains("bad time marker") && w.contains(&format!("line {line}:"))),
            "{warns:?}"
        );
        // Everything before the malformed marker is kept; the lines after it
        // are not read at all.
        let ts: Vec<u64> = w.signals[0].changes.iter().map(|c| c.t).collect();
        assert_eq!(ts, vec![0, 10]);
        // The time range follows the kept prefix, not the dropped marker.
        assert_eq!(w.start, 0);
        assert_eq!(w.end, 10);
    }

    #[test]
    fn octal_values_parse_to_bits() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 3 ! v $end\n\
            $var wire 5 \" w $end\n\
            $enddefinitions $end\n\
            #0\no7 !\no17 \"\n";
        let (w, warns) = wf(vcd);
        assert!(warns.is_empty(), "{warns:?}");
        let value = |bits: &[u8]| {
            bits.iter()
                .enumerate()
                .fold(0u64, |acc, (k, &b)| acc | ((b as u64) << k))
        };
        let v = w.signals[0].value_at(0).unwrap().to_bits_vec().unwrap();
        assert_eq!(value(&v), 7);
        let wide = w.signals[1].value_at(0).unwrap().to_bits_vec().unwrap();
        assert_eq!(value(&wide), 0b1111);
    }

    #[test]
    fn duplicate_id_code_warns_with_a_line_and_drops_the_signal() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 1 ! a $end\n\
            $var wire 1 ! b $end\n\
            $enddefinitions $end\n";
        let (w, warns) = wf(vcd);
        assert_eq!(w.signals.len(), 1);
        assert_eq!(w.signals[0].name, "a");
        let line = vcd.lines().position(|l| l.contains("b $end")).unwrap() + 1;
        assert!(
            warns
                .iter()
                .any(|w| w.contains("duplicate id code") && w.contains(&format!("line {line}:"))),
            "{warns:?}"
        );
    }

    #[test]
    fn short_var_line_warns_with_a_line() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 1 $end\n\
            $enddefinitions $end\n";
        let (w, warns) = wf(vcd);
        assert!(w.signals.is_empty());
        let line = vcd.lines().position(|l| l.contains("$var")).unwrap() + 1;
        assert!(
            warns
                .iter()
                .any(|w| w.contains("bad $var line") && w.contains(&format!("line {line}:"))),
            "{warns:?}"
        );
    }

    #[test]
    fn lazy_value_scan_truncates_with_the_source_line() {
        let vcd = "$timescale 1ns $end\n\
            $var reg 8 \" d $end\n\
            $enddefinitions $end\n\
            #0\nd1 \"\n#1\nd2 \"\n#2\nd3 \"\n";
        let path = std::env::temp_dir().join(format!(
            "waverdi_vcd_scan_{}_{}.vcd",
            std::process::id(),
            "truncate"
        ));
        std::fs::write(&path, vcd).unwrap();

        let hierarchy = read_vcd_hierarchy(&path).expect("declarations parse");
        assert!(hierarchy.out.wf.signals[0].changes.is_empty());
        // A tiny cap exercises the same code path as the 16M default.
        let scan = scan_vcd_values_with_cap(&path, &hierarchy.scan, &[0], 2).expect("scan");
        assert_eq!(scan.changes.len(), 1);
        assert_eq!(scan.changes[0].1.len(), 2);
        let warnings = scan.signal_warnings.get(&0).expect("truncation warning");
        let line = vcd.lines().position(|l| l == "d2 \"").unwrap() + 1;
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("truncated") && w.contains(&format!("line {line}:"))),
            "{warnings:?}"
        );
        let _ = std::fs::remove_file(&path);
    }
}
