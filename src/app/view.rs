use super::{App, ListRow};
use crate::waveform::Ticks;

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
        if self.wf.is_none() {
            return;
        }
        let cols = self.cols().max(1) as f64;
        let x_mid = ((t - self.t0) / self.scale).clamp(0.0, cols);
        self.scale = (self.scale * factor).clamp(1e-9, 1e15);
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

    fn reveal_cursor(&mut self) {
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
        let Some(wf) = &self.wf else { return };
        if self.display.is_empty() {
            return;
        }
        let candidates: Vec<usize> = match self.selected_row() {
            Some(ListRow::Signal { sig, .. }) => vec![sig],
            _ => self.display.clone(),
        };
        let target = candidates
            .iter()
            .filter_map(|&idx| {
                let changes = &wf.signals[idx].changes;
                if next {
                    let i = changes.partition_point(|c| c.t <= self.cursor);
                    changes.get(i).map(|c| c.t)
                } else {
                    let i = changes.partition_point(|c| c.t < self.cursor);
                    i.checked_sub(1).map(|j| changes[j].t)
                }
            })
            .reduce(|a, b| if next { a.min(b) } else { a.max(b) });
        if let Some(t) = target {
            self.cursor = t;
            self.reveal_cursor();
        }
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
    fn next_and_prev_transition_with_selection() {
        let mut app = app_with(VCD);
        app.display = vec![0];
        app.sel_row = Some(0);
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
        app.display = vec![0, 1];
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
}
