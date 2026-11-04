use super::{App, Dialog, Focus};
use crate::waveform::Ticks;

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct Marker {
    pub id: u32,
    pub name: String,
    pub time: Ticks,
}

impl App {
    pub fn add_marker(&mut self) {
        if self.wf.is_none() {
            return;
        }
        if let Some(index) = self
            .markers
            .iter()
            .position(|marker| marker.time == self.cursor)
        {
            self.marker_sel = index;
            self.msg(format!("marker {} selected", self.markers[index].name));
            return;
        }
        let id = self.next_marker_id;
        self.next_marker_id = self.next_marker_id.saturating_add(1);
        let marker = Marker {
            id,
            name: format!("M{id}"),
            time: self.cursor,
        };
        self.markers.push(marker);
        self.markers.sort_by_key(|marker| marker.time);
        self.marker_sel = self
            .markers
            .iter()
            .position(|marker| marker.time == self.cursor)
            .unwrap_or(0);
        self.msg(format!("added marker M{id}"));
    }

    pub fn open_markers(&mut self) {
        if self.wf.is_none() {
            return;
        }
        self.dialog_scroll = 0;
        self.open_dialog(Dialog::Markers);
    }

    pub fn marker_move(&mut self, delta: i64) {
        if self.markers.is_empty() {
            return;
        }
        self.marker_sel =
            (self.marker_sel as i64 + delta).clamp(0, self.markers.len() as i64 - 1) as usize;
        let rows = crate::ui::dialog::visible_rows(self.last_area, self, Dialog::Markers).max(1);
        if self.marker_sel < self.dialog_scroll {
            self.dialog_scroll = self.marker_sel;
        } else if self.marker_sel >= self.dialog_scroll + rows {
            self.dialog_scroll = self.marker_sel + 1 - rows;
        }
    }

    pub fn marker_activate(&mut self) {
        let Some(marker) = self.markers.get(self.marker_sel) else {
            return;
        };
        self.cursor = marker.time;
        self.center_cursor();
        self.focus = Focus::Wave;
        self.dialog = None;
    }

    pub fn delete_selected_marker(&mut self) {
        if self.marker_sel < self.markers.len() {
            self.markers.remove(self.marker_sel);
            self.marker_sel = self.marker_sel.min(self.markers.len().saturating_sub(1));
        }
    }

    pub fn clear_markers(&mut self) {
        self.markers.clear();
        self.marker_sel = 0;
    }

    pub fn marker_delta(&self) -> Option<Ticks> {
        let first = self.markers.first()?.time;
        let second = self.markers.get(1)?.time;
        Some(second.saturating_sub(first))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::app_with;

    const VCD: &str = "$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n#10\n1!\n#20\n0!\n";

    #[test]
    fn markers_jump_delete_and_measure_ab() {
        let mut app = app_with(VCD);
        app.cursor = 20;
        app.add_marker();
        app.cursor = 10;
        app.add_marker();
        assert_eq!(app.markers.len(), 2);
        assert_eq!(app.marker_delta(), Some(10));
        app.open_markers();
        assert_eq!(app.dialog, Some(Dialog::Markers));
        app.marker_move(1);
        app.marker_activate();
        assert_eq!(app.cursor, 20);
        assert_eq!(app.dialog, None);
        app.delete_selected_marker();
        assert_eq!(app.markers.len(), 1);
        assert_eq!(app.markers[0].time, 10);
        app.clear_markers();
        assert!(app.markers.is_empty());
        assert_eq!(app.marker_delta(), None);
    }
}
