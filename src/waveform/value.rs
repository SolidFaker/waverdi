#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    /// Signals up to [`SMALL_MAX_BITS`] bits, packed two bits per bit (the
    /// same 0/1/2/3 coding as [`Value::Bits`]), LSB first. Keeps the common
    /// narrow signals (clocks, counters, state) free of per-change heap
    /// allocations.
    Small(u64, u8),
    Bits(Vec<u8>),
    Real(f64),
    Str(String),
}

/// Widest signal stored inline in [`Value::Small`].
pub const SMALL_MAX_BITS: usize = 32;

impl Value {
    /// Pack `bits` inline when the signal is narrow enough, else keep the
    /// byte-per-bit vector.
    pub fn compact(bits: Vec<u8>) -> Value {
        if bits.len() <= SMALL_MAX_BITS {
            let mut packed = 0u64;
            for (i, &b) in bits.iter().enumerate() {
                packed |= ((b & 3) as u64) << (2 * i);
            }
            Value::Small(packed, bits.len() as u8)
        } else {
            Value::Bits(bits)
        }
    }

    /// Number of bits of a logic value (0 for real/string values).
    pub fn bits_len(&self) -> usize {
        match self {
            Value::Small(_, width) => *width as usize,
            Value::Bits(bits) => bits.len(),
            _ => 0,
        }
    }

    /// Bit `i` (LSB first) as a 0/1/2/3 code.
    pub fn bit(&self, i: usize) -> Option<u8> {
        match self {
            Value::Small(packed, width) if i < *width as usize => {
                Some(((packed >> (2 * i)) & 3) as u8)
            }
            Value::Bits(bits) => bits.get(i).copied(),
            _ => None,
        }
    }

    /// True when any bit of a logic value is `x` or `z`.
    pub fn has_unknown(&self) -> bool {
        match self {
            Value::Small(packed, width) => {
                let mask = if *width as usize >= 32 {
                    u64::MAX
                } else {
                    (1u64 << (2 * *width as usize)) - 1
                };
                packed & mask & 0xAAAA_AAAA_AAAA_AAAA != 0
            }
            Value::Bits(bits) => bits.iter().any(|&b| b >= 2),
            _ => false,
        }
    }

    /// Unknown kind of a logic value, with x taking precedence: `Some(2)`
    /// when any bit is x, `Some(3)` when only z bits are unknown and `None`
    /// when every bit is known (also for real/string values). Matches the
    /// grouped-digit rule, where a digit holding x and z renders as `x`.
    pub fn unknown_kind(&self) -> Option<u8> {
        match self {
            Value::Small(packed, width) => {
                let mask = if *width as usize >= SMALL_MAX_BITS {
                    u64::MAX
                } else {
                    (1u64 << (2 * *width as usize)) - 1
                };
                // The two-bit codes sit on even positions: `10` is x and `11`
                // is z, so `pairs & !(pairs >> 1)` marks x and
                // `pairs & (pairs >> 1)` marks z.
                let pairs = packed & mask;
                let even = 0x5555_5555_5555_5555;
                let any_x = !pairs & (pairs >> 1) & even;
                let any_z = pairs & (pairs >> 1) & even;
                if any_x != 0 {
                    Some(2)
                } else if any_z != 0 {
                    Some(3)
                } else {
                    None
                }
            }
            Value::Bits(bits) => {
                let mut any_z = false;
                for &bit in bits {
                    match bit {
                        2 => return Some(2),
                        3 => any_z = true,
                        _ => {}
                    }
                }
                any_z.then_some(3)
            }
            _ => None,
        }
    }

    /// Byte-per-bit copy of a logic value.
    pub fn to_bits_vec(&self) -> Option<Vec<u8>> {
        match self {
            Value::Small(packed, width) => Some(
                (0..*width as usize)
                    .map(|i| ((packed >> (2 * i)) & 3) as u8)
                    .collect(),
            ),
            Value::Bits(bits) => Some(bits.clone()),
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

/// Format any value; logic values go through `fmt_bits`.
pub fn fmt_value(value: &Value, radix: Radix) -> String {
    match value {
        Value::Small(packed, width) => fmt_packed(*packed, *width as usize, radix),
        Value::Bits(bits) => fmt_bits(bits, radix),
        Value::Real(real) => fmt_real(*real),
        Value::Str(s) => s.clone(),
    }
}

/// Decimal value of an all-known logic value (unknowns give `None`).
pub fn value_number(value: &Value) -> Option<u64> {
    if value.has_unknown() {
        return None;
    }
    match value {
        Value::Small(packed, width) => {
            let mut v = 0u64;
            for i in (0..*width as usize).rev() {
                v = (v << 1) | ((packed >> (2 * i)) & 1);
            }
            Some(v)
        }
        Value::Bits(bits) => {
            if bits.len() > 64 {
                return None;
            }
            let mut v = 0u64;
            for &b in bits.iter().rev() {
                v = (v << 1) | b as u64;
            }
            Some(v)
        }
        _ => None,
    }
}

/// Format a packed logic value without materializing its byte-per-bit
/// vector: `Value::Small` values are formatted thousands of times per frame,
/// so the intermediate allocation shows up in profiles.
fn fmt_packed(packed: u64, width: usize, radix: Radix) -> String {
    let bit = |i: usize| ((packed >> (2 * i)) & 3) as u8;
    match radix {
        Radix::Bin => {
            let mut s = String::with_capacity(width + 1);
            s.push('b');
            for i in (0..width).rev() {
                s.push(match bit(i) {
                    0 => '0',
                    1 => '1',
                    2 => 'x',
                    _ => 'z',
                });
            }
            s
        }
        Radix::Hex => group_packed(packed, width, 4, "0123456789abcdef", 'h'),
        Radix::Oct => group_packed(packed, width, 3, "01234567", 'o'),
        Radix::Dec => {
            let mut s = String::new();
            s.push('d');
            s.push_str(&dec_text((0..width).rev().map(&bit)));
            s
        }
        Radix::Ascii => {
            let mut s = String::new();
            let mut i = width;
            while i > 0 {
                let lo = i.saturating_sub(8);
                let (v, marker) = digit_group((lo..i).rev().map(&bit));
                s.push(match marker {
                    Some(marker) => marker,
                    None if (0x20..=0x7e).contains(&v) => v as u8 as char,
                    None => '.',
                });
                i = lo;
            }
            s
        }
    }
}

/// Decode one digit group from LSB-first 0/1/2/3 codes into its value and,
/// when the whole group is unknown, the marker that replaces the digit.
/// Unknown bits inside a partially known group count as zero, so e.g. a
/// nibble `X1X0` still renders a hex digit; only a group whose bits are all
/// x/z collapses to `x`/`z` (`x` wins when x and z are mixed).
fn digit_group(bits: impl Iterator<Item = u8>) -> (u64, Option<char>) {
    let mut v = 0u64;
    let mut any_x = false;
    let mut all_unknown = true;
    for b in bits {
        // Only a known `1` sets the bit: x and z contribute zero.
        v = (v << 1) | u64::from(b == 1);
        match b {
            2 => any_x = true,
            3 => {}
            _ => all_unknown = false,
        }
    }
    let marker = all_unknown.then_some(if any_x { 'x' } else { 'z' });
    (v, marker)
}

/// Decimal text of an LSB-first value (prefix excluded). Decimal digits are
/// not bit-aligned, so unlike power-of-two radices a partially unknown value
/// cannot be split into per-digit x/z runs: unknown bits are converted as
/// zero and only an entirely unknown value degrades to a single marker.
fn dec_text(bits: impl Iterator<Item = u8>) -> String {
    let mut v: u128 = 0;
    let mut count = 0usize;
    let mut any_x = false;
    let mut all_unknown = true;
    for b in bits {
        // Only a known `1` sets the bit: x and z contribute zero.
        v = (v << 1) | u128::from(b == 1);
        match b {
            2 => any_x = true,
            3 => {}
            _ => all_unknown = false,
        }
        count += 1;
    }
    if count > 0 && all_unknown {
        if any_x { "x" } else { "z" }.to_string()
    } else {
        v.to_string()
    }
}

/// Packed counterpart of `group`: same digit / unknown rules, read straight
/// from the two-bit codes.
fn group_packed(packed: u64, width: usize, grp: usize, digits: &str, prefix: char) -> String {
    let bit = |i: usize| ((packed >> (2 * i)) & 3) as u8;
    let ngrp = width.div_ceil(grp);
    let mut s = String::with_capacity(ngrp + 1);
    s.push(prefix);
    for gi in (0..ngrp).rev() {
        let lo = gi * grp;
        let hi = ((gi + 1) * grp).min(width);
        let (v, marker) = digit_group((lo..hi).rev().map(&bit));
        s.push(match marker {
            Some(marker) => marker,
            None => digits.as_bytes()[v as usize] as char,
        });
    }
    s
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
            s.push_str(&dec_text(bits.iter().rev().copied()));
            s
        }
        Radix::Ascii => {
            let mut s = String::new();
            let mut i = bits.len();
            while i > 0 {
                let lo = i.saturating_sub(8);
                let (v, marker) = digit_group((lo..i).rev().map(|j| bits[j]));
                s.push(match marker {
                    Some(marker) => marker,
                    None if (0x20..=0x7e).contains(&v) => v as u8 as char,
                    None => '.',
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
        Radix::Ascii => "x".repeat(digits(8)),
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
        let (v, marker) = digit_group((lo..hi).rev().map(|j| bits[j]));
        s.push(match marker {
            Some(marker) => marker,
            None => digits.as_bytes()[v as usize] as char,
        });
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
        // Grouped radices read unknown bits as zero; a fully unknown digit
        // group keeps its x/z marker (covered by the tests below).
        assert_eq!(fmt_bits(&[2, 0, 3, 1], Radix::Hex), "h8");
        assert_eq!(fmt_bits(&[2, 0, 3, 1, 0, 0, 0, 0], Radix::Hex), "h08");
        assert_eq!(fmt_bits(&[3, 1, 0, 0], Radix::Hex), "h2");
        let wide = vec![0, 0, 1, 1, 1, 0, 1, 0]; // 0b01011100 = '\\'
        assert_eq!(fmt_bits(&wide, Radix::Ascii), "\\");
        assert_eq!(fmt_bits(&nib, Radix::Ascii), ".");
    }

    #[test]
    fn binary_keeps_per_bit_unknowns() {
        // Only the grouped radices collapse unknown digits; binary shows
        // every x/z bit.
        assert_eq!(
            fmt_value(&Value::compact(vec![2, 3, 1]), Radix::Bin),
            "b1zx"
        );
    }

    #[test]
    fn grouped_radices_keep_fully_unknown_digits() {
        // 16'hF4XZ: the fully unknown nibbles keep their marker (`z` below
        // `x`), the known ones render as digits.
        let bits: Vec<u8> = vec![3, 3, 3, 3, 2, 2, 2, 2, 0, 0, 1, 0, 1, 1, 1, 1];
        assert_eq!(fmt_value(&Value::compact(bits), Radix::Hex), "hf4xz");
        // 8'hX4 with the high nibble all-x: that nibble is `x`, the low one
        // keeps its digit.
        assert_eq!(
            fmt_value(&Value::compact(vec![0, 0, 1, 0, 2, 2, 2, 2]), Radix::Hex),
            "hx4"
        );
        // A mixed nibble (X1X0) keeps a digit: the unknowns read as 0.
        assert_eq!(
            fmt_value(&Value::compact(vec![0, 2, 1, 2]), Radix::Hex),
            "h4"
        );
        // A nibble mixing x and z with no known bit renders `x`; all-z is z.
        assert_eq!(
            fmt_value(&Value::compact(vec![3, 2, 3, 2]), Radix::Hex),
            "hx"
        );
        assert_eq!(fmt_value(&Value::compact(vec![3; 4]), Radix::Hex), "hz");
    }

    #[test]
    fn oct_groups_unknown_bits_per_triplet() {
        // Low triplet all unknown (x wins over z), high triplet 0b010 -> 2.
        assert_eq!(
            fmt_value(&Value::compact(vec![2, 3, 2, 0, 1, 0]), Radix::Oct),
            "o2x"
        );
        // Three triplets: 1, 7 and a fully z one (MSB first).
        assert_eq!(
            fmt_value(&Value::compact(vec![3, 3, 3, 1, 1, 1, 1, 0, 0]), Radix::Oct),
            "o17z"
        );
        // Partial triplet with known bits: unknowns read as 0.
        assert_eq!(fmt_value(&Value::compact(vec![2, 2, 1]), Radix::Oct), "o4");
    }

    #[test]
    fn ascii_keeps_fully_unknown_bytes() {
        // High byte all-x, low byte 0x41 ('A').
        let mut bits = vec![1u8, 0, 0, 0, 0, 0, 1, 0];
        bits.extend([2u8; 8]);
        assert_eq!(fmt_value(&Value::compact(bits), Radix::Ascii), "xA");
        // One unknown bit read as 0: 0x41 loses its LSB -> 0x40 '@'.
        assert_eq!(
            fmt_value(&Value::compact(vec![2, 0, 0, 0, 0, 0, 1, 0]), Radix::Ascii),
            "@"
        );
        // A fully z byte renders `z`; a known non-printable stays `.`.
        assert_eq!(fmt_value(&Value::compact(vec![3; 8]), Radix::Ascii), "z");
        assert_eq!(fmt_value(&Value::compact(vec![0; 8]), Radix::Ascii), ".");
    }

    #[test]
    fn decimal_masks_unknown_bits_to_zero() {
        // Decimal digits are not bit-aligned, so partially unknown values
        // convert with the unknowns as 0; only an entirely unknown value
        // shows a single x/z.
        assert_eq!(
            fmt_value(&Value::compact(vec![1, 0, 0, 0, 0, 0, 0, 2]), Radix::Dec),
            "d1"
        );
        assert_eq!(fmt_value(&Value::compact(vec![2; 8]), Radix::Dec), "dx");
        assert_eq!(fmt_value(&Value::compact(vec![3; 8]), Radix::Dec), "dz");
        assert_eq!(
            fmt_value(&Value::compact(vec![2, 3, 2, 3]), Radix::Dec),
            "dx"
        );
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
    fn unknown_kind_prefers_x_and_matches_the_bit_scan() {
        assert_eq!(Value::compact(vec![0, 1, 0, 1]).unknown_kind(), None);
        assert_eq!(Value::compact(vec![3, 0, 3, 1]).unknown_kind(), Some(3));
        assert_eq!(Value::compact(vec![2, 3, 0, 1]).unknown_kind(), Some(2));
        assert_eq!(Value::compact(vec![2]).unknown_kind(), Some(2));
        assert_eq!(Value::compact(Vec::new()).unknown_kind(), None);
        assert_eq!(Value::Real(1.5).unknown_kind(), None);
        assert_eq!(Value::Str("xz".into()).unknown_kind(), None);
        // The packed fast path must match a byte-per-bit scan at every width.
        for width in 0..=SMALL_MAX_BITS {
            for shift in 0..4u8 {
                let bits: Vec<u8> = (0..width).map(|i| (i as u8 + shift) % 4).collect();
                let expected = if bits.contains(&2) {
                    Some(2)
                } else if bits.contains(&3) {
                    Some(3)
                } else {
                    None
                };
                assert_eq!(
                    Value::compact(bits).unknown_kind(),
                    expected,
                    "width={width} shift={shift}"
                );
            }
        }
        // Wider than the inline limit uses the vector path.
        let mut wide = vec![0u8; SMALL_MAX_BITS + 4];
        wide[SMALL_MAX_BITS + 1] = 3;
        assert_eq!(Value::Bits(wide.clone()).unknown_kind(), Some(3));
        wide[1] = 2;
        assert_eq!(Value::Bits(wide).unknown_kind(), Some(2));
    }

    #[test]
    fn radix_cycles_all_values() {
        let mut r = Radix::Bin;
        for _ in 0..Radix::CYCLE.len() {
            r = r.next();
        }
        assert_eq!(r, Radix::Bin);
    }

    #[test]
    fn packed_formatting_matches_the_bit_slice() {
        // `Value::Small` formatting must stay byte-identical to `fmt_bits`
        // for every width, radix and unknown/z mix (the fast path only skips
        // the intermediate Vec).
        for width in 0..=SMALL_MAX_BITS {
            for shift in 0..4u8 {
                let bits: Vec<u8> = (0..width).map(|i| (i as u8 + shift) % 4).collect();
                let mut packed = 0u64;
                for (i, &b) in bits.iter().enumerate() {
                    packed |= ((b & 3) as u64) << (2 * i);
                }
                for radix in Radix::CYCLE {
                    assert_eq!(
                        fmt_packed(packed, width, radix),
                        fmt_bits(&bits, radix),
                        "width={width} shift={shift} radix={radix:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn compact_values_roundtrip() {
        let bits = vec![0u8, 1, 2, 3, 1, 0, 1];
        let value = Value::compact(bits.clone());
        assert!(matches!(value, Value::Small(..)));
        assert_eq!(value.to_bits_vec(), Some(bits.clone()));
        assert_eq!(fmt_value(&value, Radix::Hex), fmt_bits(&bits, Radix::Hex));
        assert_eq!(fmt_value(&value, Radix::Bin), fmt_bits(&bits, Radix::Bin));
        assert!(value.has_unknown());
        assert_eq!(value_number(&value), None);

        let one = Value::compact(vec![1]);
        assert_eq!(fmt_value(&one, Radix::Bin), "b1");
        assert_eq!(one.bit(0), Some(1));
        assert_eq!(one.bits_len(), 1);

        let known = Value::compact(vec![1, 0, 1]);
        assert_eq!(value_number(&known), Some(5));
        assert!(!known.has_unknown());

        // Wider than the inline limit stays a byte-per-bit vector.
        let wide = vec![1u8; SMALL_MAX_BITS + 1];
        let value = Value::compact(wide.clone());
        assert!(matches!(value, Value::Bits(_)));
        assert_eq!(value.to_bits_vec(), Some(wide));
    }
}
