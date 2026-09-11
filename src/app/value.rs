use super::{App, Dialog};
use crate::waveform::{fmt_bits, fmt_real, format_time, Radix, SigKind, Value};

impl App {
    /// Open the "Find Value" dialog, pre-filled with the last query.
    pub fn find_value_dialog(&mut self) {
        if let Some(query) = self.value_query.clone() {
            self.input.set(&query);
        } else {
            self.input.clear();
        }
        self.dialog = Some(Dialog::FindValue);
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
        let Some(signal) = self.wf.as_ref().map(|wf| &wf.signals[index]) else {
            return;
        };
        let radix = self.radix_for(index);
        let count = signal.changes.len();
        if count == 0 {
            self.msg(format!("{}: no value changes", signal.full_name()));
            return;
        }
        let start = if forward {
            signal.changes.partition_point(|c| c.t <= self.cursor)
        } else {
            signal.changes.partition_point(|c| c.t < self.cursor)
        };
        let found = if forward {
            (start..count)
                .chain(0..start)
                .find(|&k| value_matches(&signal.changes[k].v, signal.kind, radix, &query))
        } else {
            (0..start)
                .rev()
                .chain((start..count).rev())
                .find(|&k| value_matches(&signal.changes[k].v, signal.kind, radix, &query))
        };
        match found {
            Some(k) => {
                let t = signal.changes[k].t;
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
        (SigKind::Bits, Value::Bits(bits)) => same_value_text(&fmt_bits(bits, radix), query),
        (SigKind::Real, Value::Real(real)) => query
            .trim()
            .parse::<f64>()
            .map(|q| (q - real).abs() < 1e-9)
            .unwrap_or_else(|_| fmt_real(*real).eq_ignore_ascii_case(query.trim())),
        (SigKind::Str, Value::Str(text)) => text.contains(query.trim()),
        _ => false,
    }
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
        app.display = vec![0];
        app.sel_row = Some(0);
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
        app.display = vec![0];
        app.sel_row = Some(0);
        app.radix.insert(0, Radix::Hex);
        app.value_query = Some("ff".to_string());
        app.search_value(true);
        assert_eq!(app.cursor, 20);
    }
}
