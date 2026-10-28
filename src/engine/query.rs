use crate::waveform::{fmt_real, Radix, SigKind, Ticks, Value, Waveform};

pub(crate) fn edge_time(
    wf: &Waveform,
    index: usize,
    cursor: Ticks,
    forward: bool,
    polarity: Option<u8>,
) -> Option<Ticks> {
    let sig = wf.signals.get(index)?;
    if wf.is_synthesized(index) {
        let times = wf.value_times(index);
        return if forward {
            let next = times.partition_point(|&t| t <= cursor);
            times.get(next).copied()
        } else {
            let previous = times.partition_point(|&t| t < cursor);
            previous.checked_sub(1).map(|index| times[index])
        };
    }
    let one_bit = sig.kind == SigKind::Bits && sig.bits <= 1;
    let want = if one_bit { polarity } else { None };
    let matches = |value: &Value| match want {
        Some(level) => value.bit(0) == Some(level),
        None => true,
    };
    if forward {
        let start = sig.changes.partition_point(|change| change.t <= cursor);
        sig.changes[start..]
            .iter()
            .find(|change| matches(&change.v))
            .map(|change| change.t)
    } else {
        let end = sig.changes.partition_point(|change| change.t < cursor);
        sig.changes[..end]
            .iter()
            .rev()
            .find(|change| matches(&change.v))
            .map(|change| change.t)
    }
}

pub(crate) struct ValueQuery {
    text: String,
    normalized: String,
    radix: Radix,
    number: Option<u64>,
    real: Option<f64>,
}

impl ValueQuery {
    pub(crate) fn new(query: &str, radix: Radix) -> Self {
        let text = query.trim();
        let normalized = normalize(text);
        Self {
            text: text.to_string(),
            number: number_from_normalized(&normalized, radix),
            normalized,
            radix,
            real: text.parse::<f64>().ok(),
        }
    }

    pub(crate) fn matches(&self, value: &Value, kind: SigKind) -> bool {
        match (kind, value) {
            (SigKind::Bits, Value::Small(..) | Value::Bits(_)) => {
                if let Some(number) = self.number {
                    if let Some(found) = crate::waveform::value_number(value) {
                        return found == number;
                    }
                }
                same_value_text(
                    &crate::waveform::fmt_value(value, self.radix),
                    &self.normalized,
                )
            }
            (SigKind::Real, Value::Real(real)) => match self.real {
                Some(query) => (query - real).abs() < 1e-9,
                None => fmt_real(*real).eq_ignore_ascii_case(&self.text),
            },
            (SigKind::Str, Value::Str(text)) => text.contains(&self.text),
            _ => false,
        }
    }
}

#[cfg(test)]
pub(crate) fn query_number(query: &str, radix: Radix) -> Option<u64> {
    number_from_normalized(&normalize(query), radix)
}

fn number_from_normalized(text: &str, radix: Radix) -> Option<u64> {
    let (base, digits) = match text.as_bytes().first()? {
        b'h' => (16, &text[1..]),
        b'b' => (2, &text[1..]),
        b'o' => (8, &text[1..]),
        b'd' => (10, &text[1..]),
        _ => (
            match radix {
                Radix::Hex => 16,
                Radix::Oct => 8,
                Radix::Bin => 2,
                _ => 10,
            },
            text,
        ),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, base).ok()
}

fn normalize(text: &str) -> String {
    text.trim().to_ascii_lowercase().replace('_', "")
}

fn same_value_text(formatted: &str, normalized_query: &str) -> bool {
    let formatted = normalize(formatted);
    if formatted == normalized_query {
        return true;
    }
    fn strip(text: &str) -> &str {
        match text.as_bytes().first() {
            Some(b'b' | b'h' | b'o' | b'd') if text.len() > 1 => &text[1..],
            _ => text,
        }
    }
    strip(&formatted) == strip(normalized_query)
}
