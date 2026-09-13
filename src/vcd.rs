use crate::dump::ParseOut;
use crate::waveform::{
    Change, ScopeTree, SigKind, SigState, Signal, Ticks, TimeScale, Value, Waveform,
};
use std::collections::HashMap;
use std::path::Path;

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

fn parse_dec(d: &[u8]) -> Vec<u8> {
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
            _ => {}
        }
    }
    if !ok {
        return Vec::new(); // padded to all-x later
    }
    let mut bits = Vec::new();
    while v > 0 {
        bits.push((v & 1) as u8);
        v >>= 1;
    }
    bits
}

struct Parser {
    wf: Waveform,
    idmap: HashMap<String, usize>,
    stack: Vec<usize>,
    cur_scope: Vec<String>,
    cur_time: Ticks,
    warnings: Vec<String>,
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
            },
            idmap: HashMap::new(),
            stack: vec![0],
            cur_scope: Vec::new(),
            cur_time: 0,
            warnings: Vec::new(),
        }
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
                    None => self
                        .warnings
                        .push(format!("bad $timescale: {:?}", str(rest))),
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
                    self.warnings
                        .push(format!("bad $var line: {:?}", str(rest)));
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
                    self.warnings
                        .push(format!("duplicate id code '{id}' ignored"));
                } else {
                    let sig = Signal {
                        name,
                        bits: size,
                        var_type,
                        dir: String::new(),
                        scope: self.cur_scope.clone(),
                        kind,
                        changes: Vec::new(),
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
                        self.change(ln);
                    }
                    k += 1;
                }
                Ok(k.min(lines.len()).saturating_add(1))
            }
            _ => Ok((skip_to_end(lines, i)).min(lines.len()).saturating_add(1)),
        }
    }

    fn push_change(&mut self, id: &str, v: Value) {
        let Some(&idx) = self.idmap.get(id) else {
            if !id.is_empty() {
                self.warnings.push(format!("unknown id code '{id}'"));
            }
            return;
        };
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
        sig.changes.push(Change {
            t: self.cur_time,
            v,
        });
    }

    fn change(&mut self, ln: &[u8]) {
        if ln.is_empty() {
            return;
        }
        let c = ln[0];
        match c {
            b'0' | b'1' | b'x' | b'X' | b'z' | b'Z' => {
                let v = match c {
                    b'0' => 0u8,
                    b'1' => 1u8,
                    b'x' | b'X' => 2u8,
                    _ => 3u8,
                };
                let id = str(ln[1..].trim_ascii()).to_string();
                self.push_change(&id, Value::Bits(vec![v]));
            }
            b'b' | b'B' => {
                let (digits, id) = split_value_id(&ln[1..]);
                let id = str(id).to_string();
                self.push_change(&id, Value::Bits(parse_bin(digits)));
            }
            b'o' | b'O' => {
                let (digits, id) = split_value_id(&ln[1..]);
                let id = str(id).to_string();
                self.push_change(&id, Value::Bits(parse_group(digits, 3)));
            }
            b'h' | b'H' | b't' | b'T' => {
                let (digits, id) = split_value_id(&ln[1..]);
                let id = str(id).to_string();
                self.push_change(&id, Value::Bits(parse_group(digits, 4)));
            }
            b'd' | b'D' => {
                let (digits, id) = split_value_id(&ln[1..]);
                let id = str(id).to_string();
                self.push_change(&id, Value::Bits(parse_dec(digits)));
            }
            b'r' | b'R' => {
                let (num, id) = split_real(&ln[1..]);
                let id = str(id).to_string();
                match str(num).parse::<f64>() {
                    Ok(v) => self.push_change(&id, Value::Real(v)),
                    Err(_) => self
                        .warnings
                        .push(format!("bad real value: {:?}", str(num))),
                }
            }
            b's' | b'S' => {
                let rest = &ln[1..];
                let mut q0 = None;
                for (k, &ch) in rest.iter().enumerate() {
                    if ch == b'"' {
                        q0 = Some(k);
                        break;
                    }
                }
                let Some(q0) = q0 else {
                    return;
                };
                let mut q1 = None;
                for (k, &ch) in rest[q0 + 1..].iter().enumerate() {
                    if ch == b'"' {
                        q1 = Some(q0 + 1 + k);
                        break;
                    }
                }
                let Some(q1) = q1 else {
                    return;
                };
                let s = String::from_utf8_lossy(&rest[q0 + 1..q1]).into_owned();
                let id = str(rest[q1 + 1..].trim_ascii()).to_string();
                self.push_change(&id, Value::Str(s));
            }
            b'c' | b'C' => {
                self.warnings
                    .push("char-based value codes (c/C) not supported".to_string());
            }
            _ => {
                self.warnings
                    .push(format!("unparsed value line: {:?}", str(ln)));
            }
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
            b'#' => {
                p.cur_time = parse_u64(&ln[1..]).map_err(|e| format!("bad time marker: {e}"))?;
                i += 1;
            }
            b'$' => {
                let (kw, rest) = split_word(&ln[1..]);
                i = p.directive(kw, rest, &lines, i)?;
            }
            _ => {
                p.change(ln);
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
}
