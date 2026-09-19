use super::{App, Dialog};
use crate::waveform::{fmt_real, format_time, Radix, SigKind, Ticks, Value};

impl App {
    /// Open the "Find Value" dialog, pre-filled with the last query.
    pub fn find_value_dialog(&mut self) {
        if let Some(query) = self.value_query.clone() {
            self.input.set(&query);
        } else {
            self.input.clear();
        }
        self.open_dialog(Dialog::FindValue);
    }

    pub fn apply_find_value(&mut self) {
        let query = self.input.as_string();
        self.dialog = None;
        if query.trim().is_empty() {
            return;
        }
        self.value_query = Some(query);
        self.search_value(true);
    }

    /// Jump to the next/previous value change matching the stored query.
    pub fn search_value(&mut self, forward: bool) {
        let Some(query) = self.value_query.clone() else {
            self.msg("no value query yet (press 'v' to enter one)");
            return;
        };
        let Some(index) = self.selected_signal() else {
            self.msg("select a signal before searching for a value");
            return;
        };
        let Some((state, name)) = self
            .wf
            .as_ref()
            .and_then(|wf| wf.signals.get(index))
            .map(|signal| (signal.state, signal.full_name()))
        else {
            return;
        };
        if state != crate::waveform::SigState::Ready {
            self.request_signal(index);
            self.msg(format!(
                "{name}: values are being loaded, try again in a moment"
            ));
            return;
        }
        let Some(wf) = self.wf.as_ref() else { return };
        let Some(signal) = wf.signals.get(index) else {
            return;
        };
        let radix = self.radix_for(index);
        let cursor = self.cursor;
        let count = wf.value_len(index);
        if count == 0 {
            self.msg(format!("{}: no value changes", signal.full_name()));
            return;
        }
        // Synthesized brace values are joined per time, so they are matched
        // against the computed text instead of a stored change list.
        let found: Option<Ticks> = if wf.is_synthesized(index) {
            let times = wf.value_times(index);
            let start = if forward {
                times.partition_point(|&t| t <= cursor)
            } else {
                times.partition_point(|&t| t < cursor)
            };
            let at = |k: usize| wf.value_at(index, times[k]);
            if forward {
                (start..count)
                    .chain(0..start)
                    .find(|&k| {
                        at(k).is_some_and(|value| value_matches(&value, signal.kind, radix, &query))
                    })
                    .map(|k| times[k])
            } else {
                (0..start)
                    .rev()
                    .chain((start..count).rev())
                    .find(|&k| {
                        at(k).is_some_and(|value| value_matches(&value, signal.kind, radix, &query))
                    })
                    .map(|k| times[k])
            }
        } else {
            let start = if forward {
                signal.changes.partition_point(|c| c.t <= cursor)
            } else {
                signal.changes.partition_point(|c| c.t < cursor)
            };
            let matches =
                |k: usize| value_matches(&signal.changes[k].v, signal.kind, radix, &query);
            if forward {
                (start..count)
                    .chain(0..start)
                    .find(|&k| matches(k))
                    .map(|k| signal.changes[k].t)
            } else {
                (0..start)
                    .rev()
                    .chain((start..count).rev())
                    .find(|&k| matches(k))
                    .map(|k| signal.changes[k].t)
            }
        };
        match found {
            Some(t) => {
                let name = signal.full_name();
                self.cursor = t;
                self.reveal_cursor();
                let ts = self.wf.as_ref().unwrap().ts;
                self.msg(format!(
                    "{} = {} at {}",
                    name,
                    query.trim(),
                    format_time(t as f64, &ts)
                ));
            }
            None => {
                let name = signal.full_name();
                self.msg(format!("value '{}' not found in {name}", query.trim()));
            }
        }
    }
}

fn value_matches(value: &Value, kind: SigKind, radix: Radix, query: &str) -> bool {
    match (kind, value) {
        (SigKind::Bits, Value::Small(..) | Value::Bits(_)) => {
            // Numeric fast path: avoids formatting every value while
            // searching through millions of changes.
            if let Some(number) = query_number(query, radix) {
                if let Some(found) = crate::waveform::value_number(value) {
                    return found == number;
                }
            }
            same_value_text(&crate::waveform::fmt_value(value, radix), query)
        }
        (SigKind::Real, Value::Real(real)) => query
            .trim()
            .parse::<f64>()
            .map(|q| (q - real).abs() < 1e-9)
            .unwrap_or_else(|_| fmt_real(*real).eq_ignore_ascii_case(query.trim())),
        (SigKind::Str, Value::Str(text)) => text.contains(query.trim()),
        _ => false,
    }
}

/// Numeric value of a search query, honouring an optional `h`/`b`/`o`/`d`
/// prefix and otherwise the row's radix.
fn query_number(query: &str, radix: Radix) -> Option<u64> {
    let text = query.trim().replace('_', "");
    let (base, digits) = match text.as_bytes().first()? {
        b'h' | b'H' => (16, &text[1..]),
        b'b' | b'B' => (2, &text[1..]),
        b'o' | b'O' => (8, &text[1..]),
        b'd' | b'D' => (10, &text[1..]),
        _ => (
            match radix {
                Radix::Hex => 16,
                Radix::Oct => 8,
                Radix::Bin => 2,
                _ => 10,
            },
            text.as_str(),
        ),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, base).ok()
}

/// Compare formatted values, allowing the radix prefix to be omitted.
fn same_value_text(formatted: &str, query: &str) -> bool {
    let normalize = |s: &str| s.trim().to_ascii_lowercase().replace('_', "");
    let a = normalize(formatted);
    let b = normalize(query);
    if a == b {
        return true;
    }
    let strip = |s: &str| match s.as_bytes().first() {
        Some(b'b' | b'h' | b'o' | b'd') if s.len() > 1 => s[1..].to_string(),
        _ => s.to_string(),
    };
    strip(&a) == strip(&b)
}

#[cfg(test)]
mod tests {
    use super::query_number;
    use crate::app::tests::app_with;
    use crate::waveform::Radix;

    const VCD: &str = "$timescale 1ns $end\n\
        $var reg 8 ! data $end\n\
        $enddefinitions $end\n\
        #0\nb00000000 !\n\
        #10\nb10101010 !\n\
        #20\nb11111111 !\n\
        #30\nb10101010 !\n";

    #[test]
    fn search_jumps_to_matching_value() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        app.sel_row = Some(1);
        app.radix.insert(0, Radix::Hex);
        app.value_query = Some("haa".to_string());
        app.search_value(true);
        assert_eq!(app.cursor, 10);
        app.search_value(true);
        assert_eq!(app.cursor, 30);
        app.search_value(true); // wraps
        assert_eq!(app.cursor, 10);
        app.search_value(false);
        assert_eq!(app.cursor, 30);
    }

    #[test]
    fn search_without_prefix_matches() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        app.sel_row = Some(1);
        app.radix.insert(0, Radix::Hex);
        app.value_query = Some("ff".to_string());
        app.search_value(true);
        assert_eq!(app.cursor, 20);
    }

    #[test]
    fn query_numbers_honour_prefix_and_radix() {
        assert_eq!(query_number("haa", Radix::Bin), Some(0xaa));
        assert_eq!(query_number("b1010", Radix::Hex), Some(10));
        assert_eq!(query_number("d255", Radix::Hex), Some(255));
        assert_eq!(query_number("255", Radix::Hex), Some(0x255));
        assert_eq!(query_number("255", Radix::Dec), Some(255));
        // Unknown digits fall back to the formatted comparison.
        assert_eq!(query_number("hxx", Radix::Hex), None);
        assert_eq!(query_number("", Radix::Hex), None);
    }

    #[test]
    fn search_matches_numeric_values() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        app.sel_row = Some(1);
        app.radix.insert(0, Radix::Dec);
        // VCD values: 0x00, 0xaa, 0xff, 0xaa at t = 0, 10, 20, 30.
        app.value_query = Some("d170".to_string());
        app.search_value(true);
        assert_eq!(app.cursor, 10);
        app.value_query = Some("d255".to_string());
        app.search_value(true);
        assert_eq!(app.cursor, 20);
    }
}
