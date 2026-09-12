use super::{App, Focus, TreeNode};
use crate::waveform::{Ticks, Waveform};
use std::collections::HashSet;

/// A row of the Signal List / waveform pane: a user group header or a signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListRow {
    Group {
        /// Index into `App::groups`.
        index: usize,
        /// Stable group id used for naming and context-menu targets.
        id: u32,
        name: String,
        count: usize,
        collapsed: bool,
    },
    Signal {
        sig: usize,
        depth: usize,
    },
}

/// A user-defined Signal List group. Groups partition `App::display` into
/// contiguous blocks of `count` signals each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: u32,
    pub name: String,
    pub count: usize,
    pub collapsed: bool,
}

impl Group {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            name: format!("G{id}"),
            count: 0,
            collapsed: false,
        }
    }
}

impl App {
    /// Flatten the groups and their signals into rows.
    pub fn list_rows(&self) -> Vec<ListRow> {
        let mut rows = Vec::new();
        let mut start = 0usize;
        for (index, group) in self.groups.iter().enumerate() {
            rows.push(ListRow::Group {
                index,
                id: group.id,
                name: group.name.clone(),
                count: group.count,
                collapsed: group.collapsed,
            });
            if !group.collapsed {
                let end = (start + group.count).min(self.display.len());
                for &sig in &self.display[start..end] {
                    rows.push(ListRow::Signal { sig, depth: 0 });
                }
            }
            start += group.count;
        }
        rows
    }

    /// Index of the group owning the display slot `pos`.
    pub fn group_of_pos(&self, pos: usize) -> usize {
        let mut start = 0;
        for (i, group) in self.groups.iter().enumerate() {
            start += group.count;
            if pos < start {
                return i;
            }
        }
        self.groups.len().saturating_sub(1)
    }

    /// Index of the group owning a displayed signal.
    pub fn group_of_signal(&self, sig: usize) -> Option<usize> {
        let pos = self.display.iter().position(|&s| s == sig)?;
        Some(self.group_of_pos(pos))
    }

    pub fn group_index(&self, id: u32) -> Option<usize> {
        self.groups.iter().position(|group| group.id == id)
    }

    /// Display slots owned by a group.
    pub fn group_range(&self, index: usize) -> std::ops::Range<usize> {
        let start: usize = self.groups[..index].iter().map(|g| g.count).sum();
        start..start + self.groups[index].count
    }

    /// Next free group number: highest existing id + 1.
    pub fn next_group_id(&self) -> u32 {
        self.groups.iter().map(|g| g.id).max().unwrap_or(0) + 1
    }

    /// Group that receives new signals: the one under the cursor.
    pub fn active_group(&self) -> usize {
        let Some(row) = self.sel_row else { return 0 };
        match self.list_rows().get(row) {
            Some(ListRow::Group { index, .. }) => *index,
            Some(ListRow::Signal { sig, .. }) => self.group_of_signal(*sig).unwrap_or(0),
            None => 0,
        }
    }

    /// Insert a signal at `at` and hand it to `group`.
    pub(crate) fn display_insert(&mut self, group: usize, at: usize, sig: usize) {
        let at = at.min(self.display.len());
        self.display.insert(at, sig);
        self.groups[group].count += 1;
    }

    /// Append a fresh empty group when a signal lands in the newest group.
    pub(crate) fn grow_groups(&mut self, group: usize) {
        if group + 1 == self.groups.len() {
            self.groups.push(Group::new(self.next_group_id()));
        }
    }

    /// Move a displayed signal from one slot to another; the signal joins the
    /// group that owns the drop position.
    pub fn move_signal(&mut self, from: usize, to: usize) {
        if from == to || from >= self.display.len() || to >= self.display.len() {
            return;
        }
        let src = self.group_of_pos(from);
        let dst = self.group_of_pos(to);
        let sig = self.display.remove(from);
        self.display.insert(to, sig);
        if src != dst {
            self.groups[src].count = self.groups[src].count.saturating_sub(1);
            self.groups[dst].count += 1;
        }
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

    /// Signals an action should apply to: the multi-selection when there is
    /// one, otherwise the signal on the selected row.
    pub fn selected_signals(&self) -> Vec<usize> {
        if !self.selection.is_empty() {
            return self.selection.clone();
        }
        self.selected_signal().into_iter().collect()
    }

    /// Signals a context-menu action applies to: the whole multi-selection
    /// when the clicked signal is part of it, otherwise just that signal.
    pub fn action_targets(&self, sig: usize) -> Vec<usize> {
        if self.selection.contains(&sig) {
            self.selection.clone()
        } else {
            vec![sig]
        }
    }

    /// Signal index shown on a row, if the row is a signal.
    pub fn row_signal(&self, row: usize) -> Option<usize> {
        match self.list_rows().into_iter().nth(row)? {
            ListRow::Signal { sig, .. } => Some(sig),
            ListRow::Group { .. } => None,
        }
    }

    /// Plain click / navigation: select one row and drop any multi-selection.
    pub fn select_row(&mut self, row: usize) {
        self.sel_row = Some(row);
        self.selection.clear();
        self.sel_anchor = self.row_signal(row);
    }

    /// Shift+click: add or remove one signal from the multi-selection.
    pub fn toggle_row_selection(&mut self, row: usize) {
        self.sel_row = Some(row);
        let Some(sig) = self.row_signal(row) else {
            return;
        };
        if let Some(position) = self.selection.iter().position(|&s| s == sig) {
            self.selection.remove(position);
        } else {
            self.selection.push(sig);
            if self.sel_anchor.is_none() {
                self.sel_anchor = Some(sig);
            }
        }
    }

    /// Ctrl+click: select every signal row between the anchor and `row`.
    pub fn select_range_to(&mut self, row: usize) {
        let Some(anchor) = self.sel_anchor.or_else(|| self.selected_signal()) else {
            self.sel_row = Some(row);
            return;
        };
        let rows = self.list_rows();
        let anchor_row = rows
            .iter()
            .position(|r| matches!(r, ListRow::Signal { sig, .. } if *sig == anchor));
        let Some(anchor_row) = anchor_row else {
            self.sel_row = Some(row);
            return;
        };
        let (lo, hi) = (anchor_row.min(row), anchor_row.max(row));
        self.selection = rows[lo..=hi]
            .iter()
            .filter_map(|r| match r {
                ListRow::Signal { sig, .. } => Some(*sig),
                ListRow::Group { .. } => None,
            })
            .collect();
        self.sel_anchor = Some(anchor);
        self.sel_row = Some(row);
    }

    /// Shift+Up/Down: extend the multi-selection by one row from its anchor.
    pub fn extend_selection(&mut self, delta: i64) {
        if self.sel_anchor.is_none() {
            self.sel_anchor = self.selected_signal();
        }
        self.move_sel(delta);
        if let Some(row) = self.sel_row {
            self.select_range_to(row);
        }
    }

    /// Move the selected signal up / down (`J`/`K`). Movement crosses group
    /// boundaries: the signal joins the group that owns its new position.
    pub fn move_selected_signal(&mut self, delta: i64) {
        let Some(sig) = self.selected_signal() else {
            self.msg("move signal: select a signal row first");
            return;
        };
        let Some(pos) = self.display.iter().position(|&s| s == sig) else {
            return;
        };
        let group = self.group_of_pos(pos);
        if delta > 0 {
            if pos + 1 < self.display.len() {
                self.move_signal(pos, pos + 1);
            } else if group + 1 < self.groups.len() {
                // No slot below: hand the signal over to the next group.
                self.groups[group].count = self.groups[group].count.saturating_sub(1);
                self.groups[group + 1].count += 1;
            } else {
                return;
            }
        } else if delta < 0 {
            if pos > 0 {
                self.move_signal(pos, pos - 1);
            } else if group > 0 {
                // No slot above: hand the signal over to the previous group.
                self.groups[group].count = self.groups[group].count.saturating_sub(1);
                self.groups[group - 1].count += 1;
            } else {
                return;
            }
        } else {
            return;
        }
        self.scroll_to_row_of(sig);
    }

    pub fn add_signal(&mut self, idx: usize) {
        if self.display.contains(&idx) {
            return;
        }
        let group = self.active_group();
        let at = self.group_range(group).end.min(self.display.len());
        self.display_insert(group, at, idx);
        self.grow_groups(group);
        self.sel_row = self
            .list_rows()
            .iter()
            .position(|row| matches!(row, ListRow::Signal { sig, .. } if *sig == idx))
            .or(Some(self.rows_len().saturating_sub(1)));
        self.scroll_to_sel();
        self.focus = Focus::List;
    }

    /// `dd`: cut the selected signals into the register.
    pub fn delete_selected_signals(&mut self) {
        let targets = self.selected_signals();
        if targets.is_empty() {
            self.msg("dd: select a signal row first");
            return;
        }
        self.register = targets.clone();
        for sig in targets {
            self.remove_signal(sig);
        }
        self.selection.clear();
        self.sel_anchor = None;
        self.visual = false;
        self.msg(format!(
            "cut {} signal(s) into the register (p pastes them below)",
            self.register.len()
        ));
    }

    /// `p`: paste the register below the current signal / into the current group.
    pub fn paste_register(&mut self) {
        if self.register.is_empty() {
            self.msg("register is empty");
            return;
        }
        let (group, at) = match self.selected_row() {
            Some(ListRow::Signal { sig, .. }) => {
                let pos = self
                    .display
                    .iter()
                    .position(|&s| s == sig)
                    .unwrap_or(self.display.len());
                (self.group_of_signal(sig).unwrap_or(0), pos + 1)
            }
            Some(ListRow::Group { index, .. }) => (index, self.group_range(index).end),
            None => (self.active_group(), self.display.len()),
        };
        let pasted = self.register.clone();
        let start = at.min(self.display.len());
        for (offset, &sig) in pasted.iter().enumerate() {
            self.display_insert(group, start + offset, sig);
        }
        self.grow_groups(group);
        self.scroll_to_row_of(pasted[0]);
        self.msg(format!("pasted {} signal(s)", pasted.len()));
    }

    pub fn remove_selected(&mut self) {
        if !self.selection.is_empty() {
            let count = self.selection.len();
            for sig in std::mem::take(&mut self.selection) {
                self.remove_signal(sig);
            }
            self.sel_anchor = None;
            self.msg(format!("removed {count} signals"));
            return;
        }
        match self.selected_row() {
            Some(ListRow::Signal { sig, .. }) => self.remove_signal(sig),
            Some(ListRow::Group { index, .. }) => self.remove_group(index),
            None => {}
        }
    }

    pub fn remove_signal(&mut self, sig: usize) {
        if let Some(position) = self.display.iter().position(|&s| s == sig) {
            let group = self.group_of_pos(position);
            self.display.remove(position);
            self.groups[group].count = self.groups[group].count.saturating_sub(1);
        }
        self.clamp_sel();
    }

    /// Remove a group together with the signals it owns. Removing the last
    /// group leaves a fresh empty `G0` behind.
    pub fn remove_group(&mut self, index: usize) {
        if index >= self.groups.len() {
            return;
        }
        let range = self.group_range(index);
        self.display.drain(range);
        self.groups.remove(index);
        if self.groups.is_empty() {
            self.groups.push(Group::new(0));
        }
        self.clamp_sel();
    }

    /// Rename a group; its id (and therefore its number) does not change.
    pub fn rename_group(&mut self, id: u32, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            self.msg("group name must not be empty");
            return;
        }
        if let Some(index) = self.group_index(id) {
            self.groups[index].name = name.clone();
            self.msg(format!("group G{id} renamed to {name}"));
        }
    }

    /// Create an empty group after `index`, numbered from the highest id + 1.
    pub fn insert_group_after(&mut self, index: usize) {
        let id = self.next_group_id();
        let at = (index + 1).min(self.groups.len());
        self.groups.insert(at, Group::new(id));
        self.focus_group(at);
        self.msg(format!("created group G{id}"));
    }

    pub fn clear_all(&mut self) {
        self.display.clear();
        self.groups = vec![Group::new(0)];
        self.sel_row = None;
        self.selection.clear();
        self.sel_anchor = None;
        self.row_scroll = 0;
    }

    pub fn toggle_group(&mut self, index: usize) {
        if let Some(group) = self.groups.get_mut(index) {
            group.collapsed = !group.collapsed;
        }
        self.focus_group(index);
    }

    pub fn set_group_collapsed(&mut self, index: usize, collapsed: bool) {
        if let Some(group) = self.groups.get_mut(index) {
            group.collapsed = collapsed;
        }
        self.focus_group(index);
    }

    pub fn collapse_all(&mut self) {
        for group in &mut self.groups {
            group.collapsed = true;
        }
        self.sel_row = self
            .list_rows()
            .iter()
            .position(|row| matches!(row, ListRow::Group { .. }));
        self.row_scroll = 0;
    }

    pub fn expand_all(&mut self) {
        for group in &mut self.groups {
            group.collapsed = false;
        }
        self.clamp_sel();
    }

    fn focus_group(&mut self, index: usize) {
        let rows = self.list_rows();
        if let Some(position) = rows
            .iter()
            .position(|row| matches!(row, ListRow::Group { index: i, .. } if *i == index))
        {
            self.sel_row = Some(position);
        } else {
            self.clamp_sel();
        }
        self.scroll_to_sel();
    }

    fn clamp_sel(&mut self) {
        self.selection.retain(|sig| self.display.contains(sig));
        if !self
            .sel_anchor
            .map(|sig| self.display.contains(&sig))
            .unwrap_or(false)
        {
            self.sel_anchor = None;
        }
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

    /// Jump to the first row of the Signal List (vim `gg`).
    pub(crate) fn select_first_row(&mut self) {
        if self.rows_len() == 0 {
            return;
        }
        self.sel_row = Some(0);
        self.scroll_to_sel();
    }

    pub fn list_enter(&mut self) {
        if let Some(ListRow::Group { index, .. }) = self.selected_row() {
            self.toggle_group(index);
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

    /// Hierarchical path from the root to `id`, e.g. `tb.u_dut`.
    pub fn tree_path(&self, id: usize) -> Option<String> {
        fn rec(wf: &Waveform, node: usize, target: usize, path: &mut Vec<String>) -> bool {
            path.push(wf.tree.nodes[node].name.clone());
            if node == target {
                return true;
            }
            for &child in &wf.tree.nodes[node].children {
                if rec(wf, child, target, path) {
                    return true;
                }
            }
            path.pop();
            false
        }
        let wf = self.wf.as_ref()?;
        let mut path = Vec::new();
        rec(wf, wf.tree.root, id, &mut path).then(|| path.join("."))
    }

    /// Path of the instance selected in the Instance pane, if any.
    pub fn selected_scope_path(&self) -> Option<String> {
        match self.tree_visible().get(self.tree_sel) {
            Some(TreeNode::Scope { id, .. }) => self.tree_path(*id),
            _ => None,
        }
    }

    /// Defining module of the instance selected in the Instance pane (FSDB).
    pub fn selected_scope_module(&self) -> Option<String> {
        let id = match self.tree_visible().get(self.tree_sel) {
            Some(TreeNode::Scope { id, .. }) => *id,
            _ => return None,
        };
        let module = self.wf.as_ref()?.tree.module_of(id).to_string();
        (!module.is_empty()).then_some(module)
    }

    /// Enter on the tree: collapse/expand the selected scope.
    pub fn tree_enter(&mut self) {
        let nodes = self.tree_visible();
        if let Some(TreeNode::Scope { id, .. }) = nodes.get(self.tree_sel) {
            self.toggle_scope(*id);
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
    use super::{Group, TreeNode};
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
    fn tree_navigation_shows_scopes_only() {
        let mut app = app_with(VCD);
        assert_eq!(app.tree_visible().len(), 2); // design, top
        app.toggle_scope(1); // expand top
        assert_eq!(app.tree_visible().len(), 3); // design, top, sub
        assert!(app
            .tree_visible()
            .iter()
            .all(|node| matches!(node, TreeNode::Scope { .. })));
        app.tree_sel = 2; // sub
        app.tree_enter();
        assert_eq!(app.tree_visible().len(), 3); // expanding sub adds nothing
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
    fn user_groups_partition_the_display() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        let rows = app.list_rows();
        assert_eq!(rows.len(), 3); // G0 + two signals
        assert!(matches!(
            &rows[0],
            super::ListRow::Group { name, count: 2, collapsed: false, .. } if name == "G0"
        ));
        assert!(matches!(
            &rows[1],
            super::ListRow::Signal { sig: 0, depth: 0 }
        ));
        assert!(matches!(
            &rows[2],
            super::ListRow::Signal { sig: 1, depth: 0 }
        ));
    }

    #[test]
    fn adding_signals_fills_the_active_group_and_appends_one() {
        let mut app = app_with(VCD);
        app.add_signal(0);
        assert_eq!(app.groups.len(), 2);
        assert_eq!(app.groups[0].count, 1);
        assert_eq!(app.groups[1].count, 0);
        assert_eq!(app.groups[1].id, 1);
        app.add_signal(1);
        // The cursor sits on the G0 signal, so G0 grows and G1 stays newest.
        assert_eq!(app.groups[0].count, 2);
        assert_eq!(app.groups.len(), 2);
    }

    #[test]
    fn new_group_numbers_use_the_highest_existing_id() {
        let mut app = app_with(VCD);
        app.add_signal(0); // G0 + auto-appended G1
        for _ in 0..3 {
            let id = app.next_group_id();
            app.groups.push(Group::new(id)); // G2, G3, G4
        }
        assert_eq!(
            app.groups.iter().map(|g| g.id).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
        );
        app.remove_group(2); // remove G2
        assert_eq!(app.next_group_id(), 5);
        let id = app.next_group_id();
        app.groups.push(Group::new(id)); // G5
        app.remove_group(app.group_index(id).unwrap());
        assert_eq!(app.next_group_id(), 5); // G5 is reused, not G6
    }

    #[test]
    fn collapsing_groups_hides_signals() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.toggle_group(0);
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
        assert_eq!(app.list_rows().len(), 3);
        app.collapse_all();
        assert_eq!(app.list_rows().len(), 1);
    }

    #[test]
    fn remove_group_drops_its_signals() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.groups = vec![Group::new(0), Group::new(1)];
        app.groups[0].count = 1;
        app.groups[1].count = 1;
        app.remove_group(1);
        assert_eq!(app.display, vec![0]);
        assert_eq!(app.groups.len(), 1);
        app.remove_group(0); // the last group is replaced by a fresh G0
        assert_eq!(app.groups.len(), 1);
        assert_eq!(app.groups[0].name, "G0");
        assert_eq!(app.groups[0].count, 0);
        assert!(app.display.is_empty());
    }

    #[test]
    fn moving_a_signal_across_groups_moves_ownership() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.groups = vec![Group::new(0), Group::new(1)];
        app.groups[0].count = 1;
        app.groups[1].count = 1;
        app.move_signal(0, 1);
        assert_eq!(app.display, vec![1, 0]);
        assert_eq!(app.groups[0].count, 0);
        assert_eq!(app.groups[1].count, 2);
    }

    #[test]
    fn shift_and_ctrl_selection_helpers() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        // rows: G0 (group), clk, data
        app.select_row(1);
        assert_eq!(app.sel_anchor, Some(0));
        assert!(app.selection.is_empty());
        app.toggle_row_selection(2);
        assert_eq!(app.selection, vec![1]);
        app.toggle_row_selection(1);
        assert_eq!(app.selection, vec![1, 0]);
        app.toggle_row_selection(2); // toggles off
        assert_eq!(app.selection, vec![0]);
        assert_eq!(app.selected_signals(), vec![0]);
        app.select_row(2); // plain selection clears the multi-selection
        assert!(app.selection.is_empty());
        app.toggle_row_selection(2);
        app.select_range_to(1);
        assert_eq!(app.selection, vec![0, 1]);
        assert_eq!(app.sel_row, Some(1));
    }

    #[test]
    fn remove_multi_selection_removes_all() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.selection = vec![0, 1];
        app.sel_row = Some(1);
        app.remove_selected();
        assert!(app.display.is_empty());
        assert!(app.selection.is_empty());
        assert_eq!(app.sel_anchor, None);
    }

    #[test]
    fn moving_a_signal_into_an_empty_group_moves_ownership() {
        let mut app = app_with(VCD);
        app.add_signal(0); // G0 = [clk], trailing G1 is empty
        assert_eq!(app.groups.len(), 2);
        app.sel_row = Some(1); // clk row
        app.move_selected_signal(1); // J: down into G1
        assert_eq!(app.display, vec![0]);
        assert_eq!(app.groups[0].count, 0);
        assert_eq!(app.groups[1].count, 1);
        app.move_selected_signal(-1); // K: back into G0
        assert_eq!(app.groups[0].count, 1);
        assert_eq!(app.groups[1].count, 0);
    }

    #[test]
    fn new_group_from_the_menu_uses_the_next_number() {
        let mut app = app_with(VCD);
        app.insert_group_after(0);
        assert_eq!(app.groups.len(), 2);
        assert_eq!(app.groups[1].name, "G1");
        app.insert_group_after(1);
        assert_eq!(app.groups[2].name, "G2");
        app.remove_group(1); // G1 goes away, G2 stays the highest
        app.insert_group_after(0);
        assert_eq!(app.groups[1].name, "G3");
    }

    #[test]
    fn cut_and_paste_register_round_trips() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.groups[0].count = 2;
        app.selection = vec![0, 1];
        app.delete_selected_signals();
        assert!(app.display.is_empty());
        assert_eq!(app.register, vec![0, 1]);
        app.paste_register();
        assert_eq!(app.display, vec![0, 1]);
        assert_eq!(app.register, vec![0, 1]); // pasting keeps the register
    }

    #[test]
    fn action_targets_use_selection_when_clicked_signal_is_picked() {
        let mut app = app_with(VCD);
        app.selection = vec![0, 1];
        assert_eq!(app.action_targets(1), vec![0, 1]);
        assert_eq!(app.action_targets(7), vec![7]);
    }
}
