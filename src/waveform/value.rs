#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    Bits(Vec<u8>),
    Real(f64),
    Str(String),
}

impl Value {
    pub fn as_bits(&self) -> Option<&[u8]> {
        match self {
            Value::Bits(bits) => Some(bits),
            _ => None,
        }
    }

    pub fn as_real(&self) -> Option<f64> {
        match self {
            Value::Real(real) => Some(*real),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Radix {
    Bin,
    Oct,
    Dec,
    Hex,
    Ascii,
}

impl Radix {
    pub const CYCLE: [Radix; 5] = [Radix::Bin, Radix::Oct, Radix::Dec, Radix::Hex, Radix::Ascii];

    pub fn name(self) -> &'static str {
        match self {
            Radix::Bin => "Bin",
            Radix::Oct => "Oct",
            Radix::Dec => "Dec",
            Radix::Hex => "Hex",
            Radix::Ascii => "Ascii",
        }
    }

    /// Next radix in the Verdi-style cycle.
    pub fn next(self) -> Radix {
        let i = Self::CYCLE
            .iter()
            .position(|&r| r == self)
            .unwrap_or(Self::CYCLE.len() - 1);
        Self::CYCLE[(i + 1) % Self::CYCLE.len()]
    }
}

pub fn fmt_real(v: f64) -> String {
    let s = format!("{v:.4}");
    let t = s.trim_end_matches('0').trim_end_matches('.');
    if t.is_empty() {
        "0".to_string()
    } else {
        t.to_string()
    }
}

pub fn fmt_bits(bits: &[u8], radix: Radix) -> String {
    match radix {
        Radix::Bin => {
            let mut s = String::with_capacity(bits.len() + 1);
            s.push('b');
            for &b in bits.iter().rev() {
                s.push(match b {
                    0 => '0',
                    1 => '1',
                    2 => 'x',
                    _ => 'z',
                });
            }
            s
        }
        Radix::Hex => group(bits, 4, "0123456789abcdef", 'h'),
        Radix::Oct => group(bits, 3, "01234567", 'o'),
        Radix::Dec => {
            let mut s = String::new();
            s.push('d');
            if bits.iter().any(|&b| b >= 2) {
                s.push('x');
            } else {
                let mut v: u128 = 0;
                for &b in bits.iter().rev() {
                    v = (v << 1) | (b as u128);
                }
                s.push_str(&v.to_string());
            }
            s
        }
        Radix::Ascii => {
            let mut s = String::new();
            let mut i = bits.len();
            while i > 0 {
                let lo = i.saturating_sub(8);
                let mut v: u8 = 0;
                for j in (lo..i).rev() {
                    v = (v << 1) | (bits[j] & 1);
                }
                s.push(if (0x20..=0x7e).contains(&v) {
                    v as char
                } else {
                    '.'
                });
                i = lo;
            }
            s
        }
    }
}

/// Format an all-unknown value of `width` bits without materializing it.
pub fn fmt_unknown(width: usize, radix: Radix) -> String {
    let digits = |group: usize| width.div_ceil(group);
    match radix {
        Radix::Bin => format!("b{}", "x".repeat(width)),
        Radix::Hex => format!("h{}", "x".repeat(digits(4))),
        Radix::Oct => format!("o{}", "x".repeat(digits(3))),
        Radix::Dec if width == 0 => "d0".to_string(),
        Radix::Dec => "dx".to_string(),
        Radix::Ascii => ".".repeat(digits(8)),
    }
}

fn group(bits: &[u8], grp: usize, digits: &str, prefix: char) -> String {
    let n = bits.len();
    let ngrp = n.div_ceil(grp);
    let mut s = String::with_capacity(ngrp + 1);
    s.push(prefix);
    for gi in (0..ngrp).rev() {
        let lo = gi * grp;
        let hi = ((gi + 1) * grp).min(n);
        let mut v: u64 = 0;
        let mut x = false;
        let mut z = false;
        for j in (lo..hi).rev() {
            v = (v << 1) | (bits[j] as u64 & 1);
            if bits[j] == 2 {
                x = true;
            } else if bits[j] == 3 {
                z = true;
            }
        }
        if x {
            s.push('x');
        } else if z {
            s.push('z');
        } else {
            s.push(digits.as_bytes()[v as usize] as char);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_bits_smoke() {
        let nib = vec![0, 1, 0, 1]; // LSB first = 0b1010
        assert_eq!(fmt_bits(&nib, Radix::Hex), "ha");
        assert_eq!(fmt_bits(&nib, Radix::Bin), "b1010");
        assert_eq!(fmt_bits(&nib, Radix::Oct), "o12");
        assert_eq!(fmt_bits(&nib, Radix::Dec), "d10");
        // Unknown bits win over z inside a group; otherwise z is preserved.
        assert_eq!(fmt_bits(&[2, 0, 3, 1], Radix::Hex), "hx");
        assert_eq!(fmt_bits(&[2, 0, 3, 1, 0, 0, 0, 0], Radix::Hex), "h0x");
        assert_eq!(fmt_bits(&[3, 1, 0, 0], Radix::Hex), "hz");
        let wide = vec![0, 0, 1, 1, 1, 0, 1, 0]; // 0b01011100 = '\\'
        assert_eq!(fmt_bits(&wide, Radix::Ascii), "\\");
        assert_eq!(fmt_bits(&nib, Radix::Ascii), ".");
    }

    #[test]
    fn unknown_matches_zeroed_bits() {
        for width in [0usize, 1, 2, 3, 4, 7, 8, 16, 33] {
            let unknown = vec![2u8; width];
            for radix in Radix::CYCLE {
                assert_eq!(
                    fmt_unknown(width, radix),
                    fmt_bits(&unknown, radix),
                    "width={width} radix={radix:?}"
                );
            }
        }
    }

    #[test]
    fn radix_cycles_all_values() {
        let mut r = Radix::Bin;
        for _ in 0..Radix::CYCLE.len() {
            r = r.next();
        }
        assert_eq!(r, Radix::Bin);
    }
}
