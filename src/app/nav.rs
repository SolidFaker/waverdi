use super::{App, Focus, TreeNode};
use crate::waveform::Ticks;
use crate::waveform::Waveform;

impl App {
    pub fn selected_signal(&self) -> Option<usize> {
        let row = self.sel_row?;
        self.display.get(row).copied()
    }

    pub fn add_signal(&mut self, idx: usize) {
        if self.display.contains(&idx) {
            return;
        }
        self.display.push(idx);
        self.sel_row = Some(self.display.len() - 1);
        self.scroll_to_sel();
        self.focus = Focus::List;
    }

    pub fn remove_selected(&mut self) {
        let Some(row) = self.sel_row else { return };
        self.display.remove(row);
        self.sel_row = if self.display.is_empty() {
            None
        } else {
            Some(row.min(self.display.len() - 1))
        };
        self.scroll_to_sel();
    }

    pub fn clear_all(&mut self) {
        self.display.clear();
        self.sel_row = None;
        self.row_scroll = 0;
    }

    pub(crate) fn scroll_to_sel(&mut self) {
        let Some(row) = self.sel_row else { return };
        let h = self.rows_h().max(1);
        if row < self.row_scroll {
            self.row_scroll = row;
        } else if row >= self.row_scroll + h {
            self.row_scroll = row + 1 - h;
        }
    }

    pub(crate) fn move_sel(&mut self, delta: i64) {
        if self.display.is_empty() {
            return;
        }
        let current = self.sel_row.unwrap_or(0) as i64;
        let next = (current + delta).clamp(0, self.display.len() as i64 - 1) as usize;
        self.sel_row = Some(next);
        self.scroll_to_sel();
    }

    /// Flatten the hierarchy according to the expanded scopes.
    pub fn tree_visible(&self) -> Vec<TreeNode> {
        fn rec(
            wf: &Waveform,
            id: usize,
            depth: usize,
            expanded: &std::collections::HashSet<usize>,
            out: &mut Vec<TreeNode>,
        ) {
            out.push(TreeNode::Scope { id, depth });
            if !expanded.contains(&id) {
                return;
            }
            for child in &wf.tree.nodes[id].children {
                rec(wf, *child, depth + 1, expanded, out);
            }
            for sig in &wf.tree.nodes[id].signals {
                out.push(TreeNode::Signal {
                    sig: *sig,
                    depth: depth + 1,
                });
            }
        }

        let mut out = Vec::new();
        if let Some(wf) = &self.wf {
            rec(wf, wf.tree.root, 0, &self.expanded, &mut out);
        }
        out
    }

    pub(crate) fn tree_scroll_to_sel(&mut self) {
        let nodes = self.tree_visible();
        let h = self.layout().tree_height().max(1);
        if self.tree_sel >= nodes.len() {
            self.tree_sel = nodes.len().saturating_sub(1);
        }
        if self.tree_sel < self.tree_scroll {
            self.tree_scroll = self.tree_sel;
        } else if self.tree_sel >= self.tree_scroll + h {
            self.tree_scroll = self.tree_sel + 1 - h;
        }
    }

    pub(crate) fn move_tree(&mut self, delta: i64) {
        let nodes = self.tree_visible();
        if nodes.is_empty() {
            return;
        }
        let current = self.tree_sel as i64;
        self.tree_sel = (current + delta).clamp(0, nodes.len() as i64 - 1) as usize;
        self.tree_scroll_to_sel();
    }

    pub fn toggle_scope(&mut self, id: usize) {
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
        let nodes = self.tree_visible();
        self.tree_sel = nodes
            .iter()
            .position(|n| matches!(n, TreeNode::Scope { id: i, .. } if *i == id))
            .unwrap_or(0);
        self.tree_scroll_to_sel();
    }

    /// Enter on the tree: toggle scopes, add signals to the waveform.
    pub fn tree_enter(&mut self) {
        let nodes = self.tree_visible();
        match nodes.get(self.tree_sel) {
            Some(TreeNode::Scope { id, .. }) => self.toggle_scope(*id),
            Some(TreeNode::Signal { sig, .. }) => self.add_signal(*sig),
            None => {}
        }
    }

    pub fn find_matches(&self) -> Vec<usize> {
        let Some(wf) = &self.wf else {
            return Vec::new();
        };
        let query = self.input.as_string().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        wf.signals
            .iter()
            .enumerate()
            .filter(|(_, s)| s.full_name().to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .take(200)
            .collect()
    }

    pub(crate) fn apply_goto(&mut self) {
        let spec = self.input.as_string();
        let seconds = parse_time(&spec);
        let Some(wf) = &self.wf else { return };
        match seconds {
            Ok(seconds) => {
                let ticks = (seconds / wf.ts.secs_per_tick()).round() as i128;
                self.cursor = ticks.clamp(wf.start as i128, wf.end as i128) as Ticks;
                self.t0 = self.cursor as f64 - self.span() * 0.4;
                self.clamp_view();
                self.dialog = None;
            }
            Err(()) => self.msg("bad time format (e.g. 1500, 1.5us)"),
        }
    }
}

fn parse_time(spec: &str) -> Result<f64, ()> {
    super::parse_time_spec(spec)
}

#[cfg(test)]
mod tests {
    use crate::app::tests::app_with;

    const VCD: &str = "$timescale 1ns $end\n\
        $scope module top $end\n\
        $var wire 1 ! clk $end\n\
        $scope module sub $end\n\
        $var wire 4 \" data $end\n\
        $upscope $end\n\
        $upscope $end\n\
        $enddefinitions $end\n\
        #0\n0!\nb0000 \"\n#10\n1!\n#20\nb1111 \"\n";

    #[test]
    fn tree_navigation_and_signal_add() {
        let mut app = app_with(VCD);
        assert_eq!(app.tree_visible().len(), 2); // design, top
        app.toggle_scope(1); // expand top
        assert_eq!(app.tree_visible().len(), 4); // design, top, sub, clk
        app.tree_sel = 3; // clk
        app.tree_enter();
        assert_eq!(app.display, vec![0]);
        app.toggle_scope(2); // expand sub
        assert_eq!(app.tree_visible().len(), 5); // design, top, sub, data, clk
        app.tree_sel = 3; // data
        app.tree_enter();
        assert_eq!(app.display, vec![0, 1]);
        app.toggle_scope(0);
        assert_eq!(app.tree_visible().len(), 1);
    }

    #[test]
    fn add_remove_signals() {
        let mut app = app_with(VCD);
        app.add_signal(0);
        app.add_signal(1);
        app.add_signal(0); // duplicate ignored
        assert_eq!(app.display, vec![0, 1]);
        assert_eq!(app.sel_row, Some(1));
        app.remove_selected();
        assert_eq!(app.display, vec![0]);
        app.clear_all();
        assert!(app.display.is_empty());
        assert_eq!(app.sel_row, None);
    }

    #[test]
    fn search_finds_hierarchical_names() {
        let mut app = app_with(VCD);
        app.input.insert('s');
        app.input.insert('u');
        app.input.insert('b');
        assert_eq!(app.find_matches(), vec![1]);
    }
}
