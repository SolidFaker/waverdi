use super::{App, Dialog};

impl App {
    pub fn open_dataflow(&mut self) {
        let Some((scope, signal)) = self.follow_origin() else {
            self.msg("dataflow: no signal name under the cursor");
            return;
        };
        let edges = self
            .rtl
            .as_ref()
            .map(|rtl| rtl.dataflow(&scope, &signal))
            .unwrap_or_default();
        if edges.is_empty() {
            self.msg(format!("dataflow: no local connections found for {signal}"));
            return;
        }
        self.dataflow_edges = edges;
        self.dataflow_sel = 0;
        self.dataflow_origin = Some((scope, signal.clone()));
        self.dataflow_name = signal;
        self.open_dialog(Dialog::Dataflow);
    }

    pub fn dataflow_move(&mut self, delta: i64) {
        if self.dataflow_edges.is_empty() {
            return;
        }
        self.dataflow_sel = (self.dataflow_sel as i64 + delta)
            .clamp(0, self.dataflow_edges.len() as i64 - 1) as usize;
        let rows = crate::ui::dialog::visible_rows(self.last_area, self, Dialog::Dataflow).max(1);
        if self.dataflow_sel < self.dialog_scroll {
            self.dialog_scroll = self.dataflow_sel;
        } else if self.dataflow_sel >= self.dialog_scroll + rows {
            self.dialog_scroll = self.dataflow_sel + 1 - rows;
        }
    }

    pub fn dataflow_activate(&mut self) {
        let Some(edge) = self.dataflow_edges.get(self.dataflow_sel).cloned() else {
            return;
        };
        let Some((scope, signal)) = self.dataflow_origin.as_ref() else {
            return;
        };
        let target = if edge.from.scope == *scope && edge.from.signal == *signal {
            edge.to
        } else {
            edge.from
        };
        self.jump_follow_target(target);
    }
}
