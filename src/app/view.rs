use super::{App, ListRow};
use crate::ui::layout::Layout;
use crate::ui::wave::{WaveCell, WaveKey};
use crate::waveform::{SigKind, Ticks, Value, Waveform};
use std::collections::HashMap;
use std::rc::Rc;

impl App {
    pub(crate) fn clamp_view(&mut self) {
        let Some(wf) = &self.wf else { return };
        let span = self.cols().max(1) as f64 * self.scale;
        let start = wf.start as f64;
        let end = wf.end as f64;
        let max_t0 = (end - span).max(start);
        self.t0 = self.t0.clamp(start, max_t0);
    }

    pub fn zoom_at(&mut self, t: f64, factor: f64) {
        let Some(wf) = &self.wf else {
            return;
        };
        let cols = self.cols().max(1) as f64;
        let total = wf.total_ticks() as f64;
        let min_scale = if total > 0.0 {
            total / cols
        } else {
            f64::MIN_POSITIVE
        };
        if factor > 1.0 && self.scale >= min_scale {
            // The whole range is already visible: do not zoom out further.
            return;
        }
        let x_mid = ((t - self.t0) / self.scale).clamp(0.0, cols);
        let mut scale = self.scale * factor;
        if factor > 1.0 {
            scale = scale.min(min_scale);
        }
        self.scale = scale.clamp(1e-9, 1e15);
        self.t0 = t - x_mid * self.scale;
        self.clamp_view();
    }

    pub fn zoom_in(&mut self) {
        self.zoom_at(self.cursor as f64, 1.0 / 1.3);
    }

    pub fn zoom_out(&mut self) {
        self.zoom_at(self.cursor as f64, 1.3);
    }

    pub fn fit(&mut self) {
        let Some(wf) = &self.wf else { return };
        let cols = self.cols().max(1) as f64;
        let total = wf.total_ticks() as f64;
        self.scale = if total <= 0.0 { 1.0 } else { total / cols };
        self.t0 = wf.start as f64;
        self.clamp_view();
    }

    pub fn center_cursor(&mut self) {
        self.t0 = self.cursor as f64 - self.span() * 0.4;
        self.clamp_view();
    }

    /// Zoom the time axis to the current mouse selection.
    pub fn zoom_to_range(&mut self) {
        let Some((a, b)) = self.range else { return };
        if b <= a {
            return;
        }
        let cols = self.cols().max(1) as f64;
        self.scale = ((b - a) as f64 / cols).max(1e-9);
        self.t0 = a as f64;
        self.clamp_view();
    }

    pub(crate) fn reveal_cursor(&mut self) {
        let span = self.span();
        if (self.cursor as f64) < self.t0 || (self.cursor as f64) > self.t0 + span {
            self.t0 = self.cursor as f64 - span * 0.4;
            self.clamp_view();
        }
    }

    pub fn move_cursor(&mut self, delta: i64) {
        let Some(wf) = &self.wf else { return };
        let target = self.cursor as i128 + delta as i128;
        self.cursor = target.clamp(wf.start as i128, wf.end as i128) as Ticks;
        self.reveal_cursor();
    }

    /// Move the cursor to the next/previous value change.
    ///
    /// When a signal row is selected only that signal is considered,
    /// otherwise every displayed signal participates - like Verdi's search
    /// across the waveform window.
    pub fn jump_transition(&mut self, next: bool) {
        self.jump_edge(next, None);
    }

    /// Move the cursor to the next/previous edge of the current signal.
    ///
    /// `polarity` filters single-bit signals: `Some(1)` jumps between rising
    /// edges, `Some(0)` between falling edges. Multi-bit signals always jump
    /// between changes. Falls back to all displayed signals when no signal
    /// row is selected.
    pub fn jump_edge(&mut self, forward: bool, polarity: Option<u8>) {
        let Some(wf) = &self.wf else { return };
        if self.display.is_empty() {
            return;
        }
        let candidates: Vec<usize> = match self.selected_row() {
            Some(ListRow::Signal { sig, .. }) => vec![sig],
            _ => self.display.clone(),
        };
        let cursor = self.cursor;
        let target = candidates
            .iter()
            .filter_map(|&idx| edge_time(wf, idx, cursor, forward, polarity))
            .reduce(|a, b| if forward { a.min(b) } else { a.max(b) });
        if let Some(t) = target {
            self.cursor = t;
            self.reveal_cursor();
        }
    }

    /// Brace label of a synthesized waveform row at `time`, cached for the
    /// current zoom window, radix and waveform. Consecutive redraws (cursor
    /// moves, selection changes) reuse the joined member texts; `compute` is
    /// only called on a miss.
    pub(crate) fn wave_label(
        &self,
        index: usize,
        time: Ticks,
        compute: impl FnOnce() -> String,
    ) -> (Rc<str>, usize) {
        let key = (
            self.t0.to_bits(),
            self.scale.to_bits(),
            self.cols() as u16,
            self.radix_version,
            self.waveform_version,
        );
        let mut rows = self.wave_labels.borrow_mut();
        let row = rows.entry(index).or_insert_with(|| super::WaveRowLabels {
            key,
            labels: HashMap::new(),
        });
        if row.key != key {
            row.key = key;
            row.labels.clear();
        }
        if let Some(label) = row.labels.get(&time) {
            return label.clone();
        }
        let text: Rc<str> = Rc::from(compute());
        let width = text.chars().count();
        row.labels.insert(time, (text.clone(), width));
        (text, width)
    }

    /// Sampled columns of one waveform row. `compute` runs only when the row
    /// was not sampled for the current zoom window, pane width, value/radix
    /// versions and analog range; overlay-only redraws hit the cache.
    pub(crate) fn wave_row(
        &self,
        index: usize,
        l: &Layout,
        range: (f64, f64),
        compute: impl FnOnce() -> Vec<WaveCell>,
    ) -> Rc<[WaveCell]> {
        let key = WaveKey::new(
            self.t0,
            self.scale,
            l.cols,
            l.rows.width,
            self.waveform_version,
            self.radix_version,
            range,
        );
        self.wave_rows
            .borrow_mut()
            .get_or_compute(index, key, compute)
    }
}

/// Time of the next/previous edge of a signal, optionally filtered to one
/// polarity for single-bit signals, and read from the synthesized times for
/// arrays/aggregates (which store no change list).
fn edge_time(
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
            times.iter().copied().find(|&t| t > cursor)
        } else {
            times.iter().copied().rev().find(|&t| t < cursor)
        };
    }
    let one_bit = sig.kind == SigKind::Bits && sig.bits <= 1;
    let want = if one_bit { polarity } else { None };
    let matches = |value: &Value| match want {
        Some(level) => value.bit(0) == Some(level),
        None => true,
    };
    if forward {
        sig.changes
            .iter()
            .find(|change| change.t > cursor && matches(&change.v))
            .map(|change| change.t)
    } else {
        sig.changes
            .iter()
            .rev()
            .find(|change| change.t < cursor && matches(&change.v))
            .map(|change| change.t)
    }
}

#[cfg(test)]
mod tests {
    use crate::app::tests::app_with;

    const VCD: &str = "$timescale 1ns $end\n\
        $var wire 1 ! clk $end\n\
        $var wire 1 \" rst $end\n\
        $enddefinitions $end\n\
        #0\n0!\n0\"\n\
        #10\n1!\n\
        #20\n0!\n1\"\n\
        #30\n1!\n";

    #[test]
    fn synthesized_wave_labels_are_cached_per_zoom_window() {
        use std::cell::Cell;
        use std::rc::Rc;

        let vcd = "$timescale 1ns $end\n\
            $var wire 8 ! m0 [7:0] $end\n\
            $var wire 8 \" m1 [7:0] $end\n\
            $enddefinitions $end\n#0\nb1 !\nb10 \"\n";
        let mut app = app_with(vcd);
        {
            let signals = &mut app.wf.as_mut().unwrap().signals;
            signals[0].name = "mem[0][7:0]".to_string();
            signals[1].name = "mem[1][7:0]".to_string();
        }
        app.wf.as_mut().unwrap().build_arrays();
        let root = app
            .wf
            .as_ref()
            .unwrap()
            .signals
            .iter()
            .position(|signal| signal.name == "mem")
            .unwrap();

        let calls = Cell::new(0);
        let compute = || {
            calls.set(calls.get() + 1);
            "{1, 2}".to_string()
        };
        let first = app.wave_label(root, 0, compute);
        let hit = app.wave_label(root, 0, compute);
        assert_eq!(calls.get(), 1, "a cached label must not be recomputed");
        assert!(Rc::ptr_eq(&first.0, &hit.0));
        assert_eq!(&*first.0, "{1, 2}");
        assert_eq!(first.1, 6);

        // A zoom change moves the window key and drops the labels.
        app.zoom_in();
        let zoomed = app.wave_label(root, 0, compute);
        assert!(!Rc::ptr_eq(&first.0, &zoomed.0));
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn sampled_rows_are_reused_across_cursor_moves() {
        use crate::ui::wave::{WaveCell, WaveInk};
        use std::rc::Rc;

        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 30));
        let l = app.layout();
        let build = || {
            vec![WaveCell {
                glyph: "▁",
                ink: WaveInk::Low,
            }]
        };
        let first = app.wave_row(0, &l, (0.0, 0.0), build);
        let second = app.wave_row(0, &l, (0.0, 0.0), build);
        assert!(Rc::ptr_eq(&first, &second));

        // The cursor is not part of the key: moving it must not resample.
        app.move_cursor(1);
        let moved = app.wave_row(0, &l, (0.0, 0.0), build);
        assert!(Rc::ptr_eq(&first, &moved), "cursor-only redraw resampled");

        app.zoom_in();
        let zoomed = app.wave_row(0, &l, (0.0, 0.0), build);
        assert!(!Rc::ptr_eq(&first, &zoomed), "zoom must resample");

        app.touch_radix();
        let radixed = app.wave_row(0, &l, (0.0, 0.0), build);
        assert!(!Rc::ptr_eq(&zoomed, &radixed), "radix change must resample");
    }

    #[test]
    fn next_and_prev_transition_with_selection() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        app.sel_row = Some(1);
        app.jump_transition(true);
        assert_eq!(app.cursor, 10);
        app.jump_transition(true);
        assert_eq!(app.cursor, 20);
        app.jump_transition(false);
        assert_eq!(app.cursor, 10);
        app.jump_transition(false);
        assert_eq!(app.cursor, 0);
        app.jump_transition(false);
        assert_eq!(app.cursor, 0); // no transition before start
    }

    #[test]
    fn next_transition_uses_all_displayed_signals_without_selection() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.sel_row = None;
        app.jump_transition(true); // nearest change across clk/rst after t=0
        assert_eq!(app.cursor, 10);
        app.jump_transition(true);
        assert_eq!(app.cursor, 20);
    }

    #[test]
    fn fit_shows_whole_range() {
        let mut app = app_with(VCD);
        app.scale = 1.0;
        app.fit();
        assert_eq!(app.t0, 0.0);
        assert!((app.scale - 30.0 / app.cols() as f64).abs() < 1e-9);
    }

    #[test]
    fn zoom_out_stops_at_the_full_range() {
        let mut app = app_with(VCD);
        app.fit();
        let fitted = app.scale;
        app.zoom_out();
        app.zoom_out();
        assert!((app.scale - fitted).abs() < 1e-12);
        // From a closer view, zooming out clamps to exactly the full range.
        app.scale = fitted / 1.3 * 1.01;
        app.zoom_out();
        assert!((app.scale - fitted).abs() < fitted * 1e-9);
        // Zooming in still works.
        app.zoom_in();
        assert!(app.scale < fitted);
    }

    #[test]
    fn edge_jumps_respect_single_bit_polarity() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        app.sel_row = Some(1);
        app.cursor = 0;
        app.jump_edge(true, Some(1)); // next rising
        assert_eq!(app.cursor, 10);
        app.jump_edge(true, Some(0)); // next falling
        assert_eq!(app.cursor, 20);
        app.jump_edge(false, Some(1)); // previous rising
        assert_eq!(app.cursor, 10);
        app.jump_edge(false, Some(0)); // previous falling
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn edge_jumps_ignore_polarity_on_buses() {
        let vcd = "$timescale 1ns $end\n\
            $var reg 4 ! data $end\n\
            $enddefinitions $end\n\
            #0\nb0000 !\n#10\nb0011 !\n#20\nb0101 !\n";
        let mut app = app_with(vcd);
        app.set_display(vec![0]);
        app.sel_row = Some(1);
        app.cursor = 0;
        app.jump_edge(true, Some(1)); // falls back to the next change
        assert_eq!(app.cursor, 10);
        app.jump_edge(false, Some(0));
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn edge_jump_without_selection_uses_all_signals() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.sel_row = None;
        app.cursor = 0;
        app.jump_edge(true, None);
        assert_eq!(app.cursor, 10);
        app.sel_row = Some(2); // rst has no falling edge after t=10
        app.jump_edge(true, Some(0));
        assert_eq!(app.cursor, 10);
    }
}
