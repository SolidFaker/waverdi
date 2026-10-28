use super::{App, Dialog};
use crate::engine::query::ValueQuery;
use crate::waveform::{format_time, Ticks};

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
        let Some(query_text) = self.value_query.clone() else {
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
        let query = ValueQuery::new(&query_text, radix);
        let cursor = self.cursor;
        let count = wf.value_len(index);
        if count == 0 {
            self.msg(format!("{}: no value changes", signal.full_name()));
            return;
        }
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
                    .find(|&k| at(k).is_some_and(|value| query.matches(&value, signal.kind)))
                    .map(|k| times[k])
            } else {
                (0..start)
                    .rev()
                    .chain((start..count).rev())
                    .find(|&k| at(k).is_some_and(|value| query.matches(&value, signal.kind)))
                    .map(|k| times[k])
            }
        } else {
            let start = if forward {
                signal.changes.partition_point(|c| c.t <= cursor)
            } else {
                signal.changes.partition_point(|c| c.t < cursor)
            };
            let matches = |k: usize| query.matches(&signal.changes[k].v, signal.kind);
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
                    query_text.trim(),
                    format_time(t as f64, &ts)
                ));
            }
            None => {
                let name = signal.full_name();
                self.msg(format!("value '{}' not found in {name}", query_text.trim()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::tests::app_with;
    use crate::engine::query::query_number;
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
