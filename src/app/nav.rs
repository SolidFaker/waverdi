use super::{App, Focus, TreeNode};
use crate::waveform::{Ticks, Waveform};
use std::collections::{HashMap, HashSet};

/// A row of the Signal List / waveform pane. Signals are shown inside
/// collapsible groups derived from their design hierarchy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListRow {
    Group {
        path: String,
        name: String,
        depth: usize,
        count: usize,
        collapsed: bool,
    },
    Signal {
        sig: usize,
        depth: usize,
    },
}

impl App {
    /// Flatten the displayed signals into hierarchy groups plus signal rows.
    pub fn list_rows(&self) -> Vec<ListRow> {
        let Some(wf) = &self.wf else {
            return Vec::new();
        };

        let mut counts: HashMap<String, usize> = HashMap::new();
        for &sig in &self.display {
            let scope = &wf.signals[sig].scope;
            for depth in 0..scope.len() {
                *counts.entry(scope[..=depth].join(".")).or_insert(0) += 1;
            }
        }

        let mut rows = Vec::new();
        let mut open: Vec<String> = Vec::new();
        for &sig in &self.display {
            let scope = &wf.signals[sig].scope;
            let mut common = 0;
            while common < open.len()
                && common < scope.len()
                && open[common] == scope[..=common].join(".")
            {
                common += 1;
            }
            open.truncate(common);

            if open.iter().any(|path| self.collapsed.contains(path)) {
                continue;
            }

            for depth in common..scope.len() {
                let path = scope[..=depth].join(".");
                let collapsed = self.collapsed.contains(&path);
                rows.push(ListRow::Group {
                    path: path.clone(),
                    name: scope[depth].clone(),
                    depth,
                    count: counts.get(&path).copied().unwrap_or(0),
                    collapsed,
                });
                open.push(path);
                if collapsed {
                    break;
                }
            }

            if open.len() == scope.len() && !open.iter().any(|p| self.collapsed.contains(p)) {
                rows.push(ListRow::Signal {
                    sig,
                    depth: scope.len(),
                });
            }
        }
        rows
    }

    pub fn rows_len(&self) -> usize {
        self.list_rows().len()
    }

    pub fn selected_row(&self) -> Option<ListRow> {
        let row = self.sel_row?;
        self.list_rows().into_iter().nth(row)
    }

    pub fn selected_signal(&self) -> Option<usize> {
        match self.selected_row()? {
            ListRow::Signal { sig, .. } => Some(sig),
            ListRow::Group { .. } => None,
        }
    }

    pub fn add_signal(&mut self, idx: usize) {
        if self.display.contains(&idx) {
            return;
        }
        self.display.push(idx);
        self.sel_row = self
            .list_rows()
            .iter()
            .position(|row| matches!(row, ListRow::Signal { sig, .. } if *sig == idx))
            .or(Some(self.rows_len().saturating_sub(1)));
        self.scroll_to_sel();
        self.focus = Focus::List;
    }

    pub fn remove_selected(&mut self) {
        match self.selected_row() {
            Some(ListRow::Signal { sig, .. }) => self.remove_signal(sig),
            Some(ListRow::Group { path, .. }) => self.remove_group(&path),
            None => {}
        }
    }

    pub fn remove_signal(&mut self, sig: usize) {
        if let Some(position) = self.display.iter().position(|&s| s == sig) {
            self.display.remove(position);
        }
        self.clamp_sel();
    }

    /// Remove every displayed signal inside the given scope path.
    pub fn remove_group(&mut self, path: &str) {
        let parts: Vec<&str> = path.split('.').collect();
        let scopes: Vec<Vec<String>> = self
            .wf
            .as_ref()
            .map(|wf| wf.signals.iter().map(|s| s.scope.clone()).collect())
            .unwrap_or_default();
        self.display.retain(|&sig| {
            let scope = &scopes[sig];
            !(scope.len() >= parts.len() && scope.iter().zip(&parts).all(|(a, b)| a.as_str() == *b))
        });
        self.clamp_sel();
    }

    pub fn clear_all(&mut self) {
        self.display.clear();
        self.sel_row = None;
        self.row_scroll = 0;
    }

    pub fn toggle_group(&mut self, path: &str) {
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.to_string());
        }
        self.focus_group(path);
    }

    pub fn set_group_collapsed(&mut self, path: &str, collapsed: bool) {
        if collapsed {
            self.collapsed.insert(path.to_string());
        } else {
            self.collapsed.remove(path);
        }
        self.focus_group(path);
    }

    pub fn collapse_all(&mut self) {
        let Some(wf) = &self.wf else { return };
        let mut paths = HashSet::new();
        for &sig in &self.display {
            let scope = &wf.signals[sig].scope;
            for depth in 0..scope.len() {
                paths.insert(scope[..=depth].join("."));
            }
        }
        self.collapsed = paths;
        self.sel_row = self
            .list_rows()
            .iter()
            .position(|row| matches!(row, ListRow::Group { .. }));
        self.row_scroll = 0;
    }

    pub fn expand_all(&mut self) {
        self.collapsed.clear();
        self.clamp_sel();
    }

    fn focus_group(&mut self, path: &str) {
        let rows = self.list_rows();
        if let Some(position) = rows
            .iter()
            .position(|row| matches!(row, ListRow::Group { path: p, .. } if p == path))
        {
            self.sel_row = Some(position);
        } else {
            self.clamp_sel();
        }
        self.scroll_to_sel();
    }

    fn clamp_sel(&mut self) {
        let len = self.rows_len();
        self.sel_row = if len == 0 {
            None
        } else {
            Some(self.sel_row.unwrap_or(0).min(len - 1))
        };
        let h = self.rows_h().max(1);
        self.row_scroll = self.row_scroll.min(len.saturating_sub(h));
        self.scroll_to_sel();
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
        let len = self.rows_len();
        if len == 0 {
            return;
        }
        let current = self.sel_row.unwrap_or(0) as i64;
        let next = (current + delta).clamp(0, len as i64 - 1) as usize;
        self.sel_row = Some(next);
        self.scroll_to_sel();
    }

    pub fn list_enter(&mut self) {
        if let Some(ListRow::Group { path, .. }) = self.selected_row() {
            self.toggle_group(&path);
        }
    }

    /// Flatten the hierarchy according to the expanded scopes.
    pub fn tree_visible(&self) -> Vec<TreeNode> {
        fn rec(
            wf: &Waveform,
            id: usize,
            depth: usize,
            expanded: &HashSet<usize>,
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
        assert_eq!(app.selected_signal(), Some(1));
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

    #[test]
    fn list_rows_merges_signals_into_one_group() {
        let vcd = "$timescale 1ns $end\n\
            $scope module top $end\n\
            $var wire 1 ! a $end\n\
            $var wire 1 \" b $end\n\
            $scope module sub $end\n\
            $var wire 1 # c $end\n\
            $var wire 1 % d $end\n\
            $upscope $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n0\"\n0#\n0%\n";
        let mut app = app_with(vcd);
        app.display = vec![0, 1, 2, 3];
        let rows = app.list_rows();
        let groups: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                super::ListRow::Group { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(groups, vec!["top", "sub"]);
        assert_eq!(rows.len(), 6); // 2 groups + 4 signals
    }

    #[test]
    fn list_rows_group_by_scope() {
        let mut app = app_with(VCD);
        app.display = vec![0, 1];
        let rows = app.list_rows();
        assert_eq!(rows.len(), 4); // top, clk, top.sub, data
        assert!(
            matches!(&rows[0], super::ListRow::Group { name, depth: 0, count: 2, collapsed: false, .. } if name == "top")
        );
        assert!(matches!(
            &rows[1],
            super::ListRow::Signal { sig: 0, depth: 1 }
        ));
        assert!(matches!(&rows[2], super::ListRow::Group { name, depth: 1, .. } if name == "sub"));
        assert!(matches!(
            &rows[3],
            super::ListRow::Signal { sig: 1, depth: 2 }
        ));
    }

    #[test]
    fn collapsing_groups_hides_signals() {
        let mut app = app_with(VCD);
        app.display = vec![0, 1];
        app.toggle_group("top");
        let rows = app.list_rows();
        assert_eq!(rows.len(), 1);
        assert!(matches!(
            &rows[0],
            super::ListRow::Group {
                collapsed: true,
                ..
            }
        ));
        app.expand_all();
        assert_eq!(app.list_rows().len(), 4);
        app.collapse_all();
        assert_eq!(app.list_rows().len(), 1); // only "top"
    }

    #[test]
    fn remove_group_drops_its_signals() {
        let mut app = app_with(VCD);
        app.display = vec![0, 1];
        app.remove_group("top.sub");
        assert_eq!(app.display, vec![0]);
        app.remove_group("top");
        assert!(app.display.is_empty());
    }
}
