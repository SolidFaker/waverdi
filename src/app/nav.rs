use super::{App, Focus, TreeNode};
use crate::waveform::{Ticks, Waveform};
use std::collections::HashSet;
use std::rc::Rc;

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
    /// Flatten the groups and their signals into rows. The result is cached
    /// until `panes_version` changes, so draw code can call it freely.
    pub fn list_rows(&self) -> Rc<Vec<ListRow>> {
        let version = self.panes_version;
        if let Some((cached, rows)) = self.list_cache.borrow().as_ref() {
            if *cached == version {
                return rows.clone();
            }
        }
        let rows = Rc::new(self.build_list_rows());
        *self.list_cache.borrow_mut() = Some((version, rows.clone()));
        rows
    }

    fn build_list_rows(&self) -> Vec<ListRow> {
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

    /// Insert a signal at `at` and hand it to `group`. Lazily loaded signals
    /// request their value changes from the backend as soon as they are added.
    pub(crate) fn display_insert(&mut self, group: usize, at: usize, sig: usize) {
        self.touch_panes();
        let at = at.min(self.display.len());
        self.display.insert(at, sig);
        self.groups[group].count += 1;
        self.request_signal(sig);
    }

    /// Append a fresh empty group when a signal lands in the newest group.
    pub(crate) fn grow_groups(&mut self, group: usize) {
        if group + 1 == self.groups.len() {
            self.groups.push(Group::new(self.next_group_id()));
            self.touch_panes();
        }
    }

    /// Move a displayed signal from one slot to another; the signal joins the
    /// group that owns the drop position.
    pub fn move_signal(&mut self, from: usize, to: usize) {
        self.touch_panes();
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

    /// Move the displayed signal at `from` to `to` and hand it to `group`,
    /// which may be empty (the slot is where its signals would start).
    /// Returns the signal's new display position.
    #[cfg(test)]
    pub fn move_signal_to(&mut self, from: usize, group: usize, to: usize) -> Option<usize> {
        self.touch_panes();
        if from >= self.display.len() || group >= self.groups.len() {
            return None;
        }
        let src = self.group_of_pos(from);
        let to = to.min(self.display.len());
        let sig = self.display.remove(from);
        let to = if from < to { to - 1 } else { to }.min(self.display.len());
        self.display.insert(to, sig);
        if src != group {
            self.groups[src].count = self.groups[src].count.saturating_sub(1);
            self.groups[group].count += 1;
        }
        Some(to)
    }

    /// Keep an empty group at the end of the list. Called after a drop or a
    /// keyboard move, never mid-drag, so dragging through the groups does not
    /// create a trail of empty ones.
    pub(crate) fn ensure_trailing_group(&mut self) {
        if self
            .groups
            .last()
            .map(|group| group.count > 0)
            .unwrap_or(false)
        {
            let id = self.next_group_id();
            self.groups.push(Group::new(id));
            self.touch_panes();
        }
    }

    /// Move a group up / down (`J`/`K` on a group row or a header drag).
    /// Groups keep their ids; only their order (and display blocks) changes.
    pub fn move_group(&mut self, index: usize, delta: i64) {
        self.touch_panes();
        let target = index as i64 + delta;
        if delta == 0
            || target < 0
            || target as usize >= self.groups.len()
            || index >= self.groups.len()
        {
            return;
        }
        let target = target as usize;
        let mut blocks: Vec<Vec<usize>> = Vec::with_capacity(self.groups.len());
        let mut start = 0;
        for group in &self.groups {
            let end = (start + group.count).min(self.display.len());
            blocks.push(self.display[start..end].to_vec());
            start = end;
        }
        self.groups.swap(index, target);
        blocks.swap(index, target);
        self.display = blocks.into_iter().flatten().collect();
        self.sel_row = self
            .list_rows()
            .iter()
            .position(|row| matches!(row, ListRow::Group { index, .. } if *index == target));
        self.scroll_to_sel();
        self.ensure_trailing_group();
        self.msg(format!(
            "moved {} to position {}",
            self.groups[target].name,
            target + 1
        ));
    }

    pub fn rows_len(&self) -> usize {
        self.list_rows().len()
    }

    /// Width (in characters) of the widest displayed signal name; used by the
    /// Signal List horizontal scrollbar.
    pub fn list_content_width(&self) -> usize {
        let Some(wf) = &self.wf else {
            return 0;
        };
        self.list_rows()
            .iter()
            .filter_map(|row| match row {
                ListRow::Signal { sig, .. } => {
                    let signal = &wf.signals[*sig];
                    Some(if self.show_full_names {
                        signal.full_name().chars().count()
                    } else {
                        signal.name.chars().count()
                    })
                }
                ListRow::Group { .. } => None,
            })
            .max()
            .unwrap_or(0)
    }

    pub fn selected_row(&self) -> Option<ListRow> {
        let row = self.sel_row?;
        self.list_rows().get(row).cloned()
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
        match self.list_rows().get(row)? {
            ListRow::Signal { sig, .. } => Some(*sig),
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

    /// Width (in characters) of the widest Value column text at the cursor.
    pub fn value_content_width(&self) -> usize {
        if self.wf.is_none() {
            return 0;
        }
        self.display
            .iter()
            .map(|&sig| self.list_display_value(sig).1)
            .max()
            .unwrap_or(0)
    }

    /// `(text, display width)` of a displayed signal's value at the cursor.
    /// Both the Signal List rows and the value-column scrollbar read it, so
    /// each signal is formatted once per `(cursor, radix, waveform, panes)`
    /// key instead of once per reader. The returned `Rc` makes the hit path
    /// free of string copies.
    pub fn list_display_value(&self, sig: usize) -> (Rc<str>, usize) {
        let key = (
            self.cursor,
            self.radix_version,
            self.waveform_version,
            self.panes_version,
        );
        let mut cache = self.list_value_cache.borrow_mut();
        match cache.as_mut() {
            Some(cached) if cached.key == key => {}
            _ => {
                *cache = Some(super::ListValueCache {
                    key,
                    values: Vec::new(),
                });
            }
        }
        let cache = cache.as_mut().unwrap();
        if cache.values.len() <= sig {
            cache.values.resize_with(sig + 1, || None);
        }
        if let Some(entry) = &cache.values[sig] {
            return entry.clone();
        }
        // Synthesized brace values must be joined at the cursor; the signal
        // itself stores no text.
        let text: Rc<str> = match &self.wf {
            Some(wf) => Rc::from(wf.display_value(sig, self.cursor, self.radix_for(sig))),
            None => Rc::from(""),
        };
        let width = text.chars().count();
        let entry = (text, width);
        cache.values[sig] = Some(entry.clone());
        entry
    }

    /// Width (in characters) of the widest Hierarchy label (indent + arrow).
    pub fn tree_content_width(&self) -> usize {
        let Some(wf) = &self.wf else {
            return 0;
        };
        self.tree_visible()
            .iter()
            .map(|node| match node {
                TreeNode::Scope { id, depth } => {
                    let name = &wf.tree.nodes[*id].name;
                    depth * 2 + 2 + name.chars().count()
                }
            })
            .max()
            .unwrap_or(0)
    }

    /// Width (in characters) of the widest Module name.
    pub fn module_content_width(&self) -> usize {
        let Some(wf) = &self.wf else {
            return 0;
        };
        self.tree_visible()
            .iter()
            .map(|node| match node {
                TreeNode::Scope { id, .. } => wf.tree.module_of(*id).chars().count(),
            })
            .max()
            .unwrap_or(0)
    }

    /// Move the selected signal(s) up / down (`J`/`K`). With a multi-selection
    /// every selected signal moves as one block; movement crosses group
    /// boundaries. On a group row, the group itself is moved.
    pub fn move_selected_signal(&mut self, delta: i64) {
        self.touch_panes();
        if let Some(ListRow::Group { index, .. }) = self.selected_row() {
            self.move_group(index, delta);
            return;
        }
        if !self.selection.is_empty() {
            let anchor = self.selected_signal();
            self.move_selection(delta);
            if let Some(sig) = anchor {
                self.select_signal_row(sig);
            }
            self.ensure_trailing_group();
            return;
        }
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
                self.grow_groups(group + 1);
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
                self.grow_groups(group - 1);
            } else {
                return;
            }
        } else {
            return;
        }
        self.ensure_trailing_group();
        self.scroll_to_row_of(sig);
    }

    /// Move every selected signal one step up / down as one block.
    pub fn move_selection(&mut self, delta: i64) {
        self.touch_panes();
        let mut positions: Vec<usize> = self
            .display
            .iter()
            .enumerate()
            .filter(|(_, sig)| self.selection.contains(sig))
            .map(|(position, _)| position)
            .collect();
        if positions.is_empty() {
            return;
        }
        positions.sort_unstable();
        let count = positions.len();
        if delta > 0 {
            // A block at the very end of the display hands itself to the next
            // group (same rule as a single signal).
            if positions.last() == Some(&(self.display.len() - 1))
                && positions == (self.display.len() - count..self.display.len()).collect::<Vec<_>>()
            {
                let group = self.group_of_pos(self.display.len() - count);
                if group + 1 < self.groups.len() {
                    self.groups[group].count = self.groups[group].count.saturating_sub(count);
                    self.groups[group + 1].count += count;
                    return;
                }
            }
            for &position in positions.iter().rev() {
                if position + 1 < self.display.len() {
                    self.move_signal(position, position + 1);
                }
            }
        } else if delta < 0 {
            if positions.first() == Some(&0) && positions == (0..count).collect::<Vec<_>>() {
                let group = self.group_of_pos(0);
                if group > 0 {
                    self.groups[group].count = self.groups[group].count.saturating_sub(count);
                    self.groups[group - 1].count += count;
                    return;
                }
            }
            for &position in positions.iter() {
                if position > 0 {
                    self.move_signal(position, position - 1);
                }
            }
        }
    }

    /// Double-click on a signal row: expand a multi-bit signal into its bits,
    /// or collapse the bit rows created earlier (also those of `Split Bus`).
    pub fn toggle_signal_expand(&mut self, sig: usize) {
        self.touch_panes();
        let children: Vec<usize> = match &self.wf {
            Some(wf) => wf.children(sig),
            None => return,
        };
        let Some(pos) = self.display.iter().position(|&s| s == sig) else {
            return;
        };
        if children.is_empty() {
            let Some((first, last)) = self.append_bit_chunks(sig, 1) else {
                return;
            };
            let group = self.group_of_pos(pos);
            for (offset, child) in (first..last).enumerate() {
                self.display_insert(group, pos + 1 + offset, child);
            }
            self.select_signal_row(sig);
            self.msg(format!("expanded signal into {} bit(s)", last - first));
            return;
        }
        let visible: Vec<usize> = children
            .iter()
            .copied()
            .filter(|child| self.display.contains(child))
            .collect();
        if visible.len() < children.len() {
            // Expand: add the children that are not displayed yet right below
            // the parent (a partially expanded array must not collapse).
            let group = self.group_of_pos(pos);
            let mut added = 0usize;
            for (offset, child) in children
                .iter()
                .filter(|child| !visible.contains(child))
                .enumerate()
            {
                self.display_insert(group, pos + 1 + offset, *child);
                added += 1;
            }
            self.select_signal_row(sig);
            self.msg(format!("expanded signal into {added} row(s)"));
        } else {
            // Collapse the whole subtree below this signal.
            let subtree = self.signal_subtree(sig);
            let mut positions: Vec<usize> = self
                .display
                .iter()
                .enumerate()
                .filter(|(_, signal)| subtree.contains(signal))
                .map(|(position, _)| position)
                .collect();
            positions.sort_unstable_by(|a, b| b.cmp(a));
            for position in positions {
                let group = self.group_of_pos(position);
                self.display.remove(position);
                self.groups[group].count = self.groups[group].count.saturating_sub(1);
            }
            self.selection.retain(|signal| !subtree.contains(signal));
            self.select_signal_row(sig);
            self.msg(format!("collapsed {} row(s)", visible.len()));
        }
    }

    /// All signals below `root` (direct and expanded descendants).
    fn signal_subtree(&self, root: usize) -> Vec<usize> {
        let Some(wf) = &self.wf else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            for child in wf.children(node) {
                out.push(child);
                stack.push(child);
            }
        }
        out
    }

    /// Select the row of `sig` and keep it visible.
    fn select_signal_row(&mut self, sig: usize) {
        self.sel_row = self
            .list_rows()
            .iter()
            .position(|row| matches!(row, ListRow::Signal { sig: s, .. } if *s == sig));
        self.scroll_to_sel();
        self.focus = Focus::List;
    }

    pub fn add_signal(&mut self, idx: usize) {
        // The same signal may be added repeatedly; every add appends a row.
        let group = self.active_group();
        let at = self.group_range(group).end.min(self.display.len());
        self.display_insert(group, at, idx);
        self.grow_groups(group);
        // Select the occurrence that was just inserted.
        let occurrence = self.display[..=at]
            .iter()
            .filter(|&&sig| sig == idx)
            .count();
        let mut seen = 0usize;
        self.sel_row = self.list_rows().iter().position(|row| match row {
            ListRow::Signal { sig, .. } if *sig == idx => {
                seen += 1;
                seen == occurrence
            }
            _ => false,
        });
        self.sel_row = self.sel_row.or(Some(self.rows_len().saturating_sub(1)));
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
            Some(ListRow::Signal { sig, .. }) => {
                // Remove the occurrence on the selected row, not just the
                // first one (a signal may be displayed several times).
                let rank = self
                    .sel_row
                    .map(|row| {
                        let rows = self.list_rows();
                        let end = (row + 1).min(rows.len());
                        rows[..end]
                            .iter()
                            .filter(
                                |row| matches!(row, ListRow::Signal { sig: s, .. } if *s == sig),
                            )
                            .count()
                    })
                    .unwrap_or(1)
                    .max(1);
                self.remove_occurrence(sig, rank);
            }
            Some(ListRow::Group { index, .. }) => self.remove_group(index),
            None => {}
        }
    }

    /// Remove the `rank`-th displayed occurrence of a signal.
    fn remove_occurrence(&mut self, sig: usize, rank: usize) {
        self.touch_panes();
        let mut seen = 0usize;
        let mut position = None;
        for (index, &s) in self.display.iter().enumerate() {
            if s == sig {
                seen += 1;
                if seen == rank {
                    position = Some(index);
                    break;
                }
            }
        }
        if let Some(position) = position {
            let group = self.group_of_pos(position);
            self.display.remove(position);
            self.groups[group].count = self.groups[group].count.saturating_sub(1);
        }
        self.clamp_sel();
    }

    pub fn remove_signal(&mut self, sig: usize) {
        self.touch_panes();
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
        self.touch_panes();
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
        self.touch_panes();
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
        self.touch_panes();
        let id = self.next_group_id();
        let at = (index + 1).min(self.groups.len());
        self.groups.insert(at, Group::new(id));
        self.focus_group(at);
        self.msg(format!("created group G{id}"));
    }

    pub fn clear_all(&mut self) {
        self.touch_panes();
        self.display.clear();
        self.groups = vec![Group::new(0)];
        self.sel_row = None;
        self.selection.clear();
        self.sel_anchor = None;
        self.row_scroll = 0;
    }

    pub fn toggle_group(&mut self, index: usize) {
        self.touch_panes();
        if let Some(group) = self.groups.get_mut(index) {
            group.collapsed = !group.collapsed;
        }
        self.focus_group(index);
    }

    pub fn set_group_collapsed(&mut self, index: usize, collapsed: bool) {
        self.touch_panes();
        if let Some(group) = self.groups.get_mut(index) {
            group.collapsed = collapsed;
        }
        self.focus_group(index);
    }

    pub fn collapse_all(&mut self) {
        self.touch_panes();
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
        self.touch_panes();
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

    /// Flatten the hierarchy according to the expanded scopes. The result is
    /// cached until `panes_version` changes, so draw code can call it freely.
    pub fn tree_visible(&self) -> Rc<Vec<TreeNode>> {
        let version = self.panes_version;
        if let Some((cached, nodes)) = self.tree_cache.borrow().as_ref() {
            if *cached == version {
                return nodes.clone();
            }
        }
        let nodes = Rc::new(self.build_tree_visible());
        *self.tree_cache.borrow_mut() = Some((version, nodes.clone()));
        nodes
    }

    fn build_tree_visible(&self) -> Vec<TreeNode> {
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

    /// Maximum scroll offset of the Signal List rows.
    pub fn row_max_scroll(&self) -> usize {
        self.rows_len().saturating_sub(self.rows_h())
    }

    /// Clamp the row scroll after rows were collapsed / removed so the
    /// drawing and the click mapping stay in sync.
    pub(crate) fn clamp_row_scroll(&mut self) {
        self.row_scroll = self.row_scroll.min(self.row_max_scroll());
    }

    /// Maximum scroll offset of the Instance pane: both the drawing and the
    /// click mapping use it, so they never drift apart.
    pub fn tree_max_scroll(&self) -> usize {
        self.tree_visible()
            .len()
            .saturating_sub(self.layout().tree_height())
    }

    /// Clamp the Instance scroll offset after the tree size changed.
    pub(crate) fn clamp_tree_scroll(&mut self) {
        self.tree_scroll = self.tree_scroll.min(self.tree_max_scroll());
    }

    pub(crate) fn tree_scroll_to_sel(&mut self) {
        self.clamp_tree_scroll();
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
        self.touch_panes();
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
            self.locate_scope_in_source();
        }
    }

    /// Signals matching the Find dialog query, in dump order, capped at 200.
    /// The list is cached per query text and waveform version: typing
    /// recomputes, a redraw of the open dialog does not.
    pub fn find_matches(&self) -> Rc<[usize]> {
        let query = self.input.as_string();
        {
            let cache = self.find_cache.borrow();
            if let Some(cached) = cache.as_ref() {
                if cached.query == query && cached.version == self.waveform_version {
                    return cached.matches.clone();
                }
            }
        }
        let matches: Rc<[usize]> = Rc::from(self.compute_find_matches(&query));
        *self.find_cache.borrow_mut() = Some(super::FindCache {
            query,
            version: self.waveform_version,
            matches: matches.clone(),
        });
        matches
    }

    /// Scan every signal once for the lowercased query. The full name is
    /// built into a reused buffer and only lowercased for non-ASCII names,
    /// so the 90k-signal dump does not allocate two strings per signal.
    fn compute_find_matches(&self, raw: &str) -> Vec<usize> {
        let Some(wf) = &self.wf else {
            return Vec::new();
        };
        let query = raw.to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let ascii = query.is_ascii();
        let mut full = String::new();
        let mut matches = Vec::new();
        for (i, s) in wf.signals.iter().enumerate() {
            if s.var_type == "aggregate" {
                continue;
            }
            full.clear();
            for part in &s.scope {
                full.push_str(part);
                full.push('.');
            }
            full.push_str(&s.name);
            let hit = if ascii && full.is_ascii() {
                contains_ascii_case_insensitive(&full, &query)
            } else {
                full.to_lowercase().contains(&query)
            };
            if hit {
                matches.push(i);
                if matches.len() == 200 {
                    break;
                }
            }
        }
        matches
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

/// Allocation-free case-insensitive `contains` for ASCII text; `query` must
/// already be lowercase. Non-ASCII names keep the exact Unicode semantics via
/// `str::to_lowercase` in the caller.
fn contains_ascii_case_insensitive(haystack: &str, query: &str) -> bool {
    let needle = query.as_bytes();
    let hay = haystack.as_bytes();
    if needle.is_empty() {
        return true;
    }
    hay.windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
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
    fn pane_caches_rebuild_only_after_mutations() {
        use std::rc::Rc;

        let mut app = app_with(VCD);
        let rows = app.list_rows();
        assert!(Rc::ptr_eq(&rows, &app.list_rows()));

        app.add_signal(0);
        let grown = app.list_rows();
        assert!(!Rc::ptr_eq(&rows, &grown));
        // The signal row plus the fresh trailing group row.
        assert_eq!(grown.len(), rows.len() + 2);

        let nodes = app.tree_visible();
        assert!(Rc::ptr_eq(&nodes, &app.tree_visible()));
        app.toggle_scope(1);
        let toggled = app.tree_visible();
        assert!(!Rc::ptr_eq(&nodes, &toggled));
        assert_eq!(toggled.len(), nodes.len() + 1);
    }

    #[test]
    fn the_trailing_empty_group_is_added_on_drop() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.grow_groups(0); // G1 stays empty
        assert_eq!(app.groups.len(), 2);
        // Moving (dragging) alone does not create groups...
        app.move_signal_to(0, 1, 2);
        assert_eq!(app.groups[1].count, 1);
        assert_eq!(app.groups.len(), 2, "no empty group during the drag");
        // ...only the drop does.
        app.ensure_trailing_group();
        assert_eq!(app.groups.len(), 3);
        assert_eq!(app.groups[2].count, 0);
        // The next signal can join the same group without another growth.
        app.move_signal_to(0, 1, 3);
        assert_eq!(app.groups[1].count, 2);
        app.ensure_trailing_group();
        assert_eq!(app.groups.len(), 3);
    }

    #[test]
    fn expanding_a_multi_dim_array_keeps_the_signals_below_it() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 8 ! e0 [7:0] $end\n\
            $var wire 8 \" e1 [7:0] $end\n\
            $var wire 8 # e2 [7:0] $end\n\
            $var wire 8 $ e3 [7:0] $end\n\
            $var wire 1 % clk $end\n\
            $enddefinitions $end\n#0\nb0 !\nb0 \"\nb0 #\nb0 $\n0%\n";
        let mut app = app_with(vcd);
        {
            let signals = &mut app.wf.as_mut().unwrap().signals;
            signals[0].name = "arr[0][0][7:0]".to_string();
            signals[1].name = "arr[0][1][7:0]".to_string();
            signals[2].name = "arr[1][0][7:0]".to_string();
            signals[3].name = "arr[1][1][7:0]".to_string();
        }
        app.wf.as_mut().unwrap().build_arrays();
        let root = app
            .wf
            .as_ref()
            .unwrap()
            .signals
            .iter()
            .position(|signal| signal.name == "arr")
            .unwrap();
        let clk = app
            .wf
            .as_ref()
            .unwrap()
            .signals
            .iter()
            .position(|signal| signal.name == "clk")
            .unwrap();
        app.set_display(vec![root, clk]);
        app.toggle_signal_expand(root);
        assert_eq!(app.display.len(), 4, "{:?}", app.display);
        assert_eq!(app.display.last(), Some(&clk));
    }

    #[test]
    fn expanding_a_partially_shown_array_adds_missing_children() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 8 ! e0 [7:0] $end\n\
            $var wire 8 \" e1 [7:0] $end\n\
            $var wire 8 # tail $end\n\
            $enddefinitions $end\n#0\nb0 !\nb0 \"\n0#\n";
        let mut app = app_with(vcd);
        {
            let signals = &mut app.wf.as_mut().unwrap().signals;
            signals[0].name = "arr[0][7:0]".to_string();
            signals[1].name = "arr[1][7:0]".to_string();
        }
        app.wf.as_mut().unwrap().build_arrays();
        let find = |app: &crate::app::App, name: &str| {
            app.wf
                .as_ref()
                .unwrap()
                .signals
                .iter()
                .position(|signal| signal.name == name)
                .unwrap()
        };
        let root = find(&app, "arr");
        let child0 = find(&app, "arr[0][7:0]");
        let child1 = find(&app, "arr[1][7:0]");
        let tail = find(&app, "tail");
        // One child is already on the list (added by hand): the first
        // double-click must expand, not collapse and drop the rows below.
        app.set_display(vec![root, child0, tail]);
        app.toggle_signal_expand(root);
        assert!(app.display.contains(&child1));
        assert!(app.display.contains(&child0));
        assert!(app.display.contains(&tail));
        assert_eq!(app.display.len(), 4, "{:?}", app.display);
        // Once every child is shown, the next double-click collapses.
        app.toggle_signal_expand(root);
        assert!(!app.display.contains(&child0));
        assert!(!app.display.contains(&child1));
        assert!(app.display.contains(&tail));
    }

    #[test]
    fn add_remove_signals() {
        let mut app = app_with(VCD);
        app.add_signal(0);
        app.add_signal(1);
        app.add_signal(0); // the same signal may be added again
        assert_eq!(app.display, vec![0, 1, 0]);
        assert_eq!(app.selected_signal(), Some(0));
        app.remove_selected();
        assert_eq!(app.display, vec![0, 1]);
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
        assert_eq!(app.find_matches().to_vec(), vec![1]);
    }

    #[test]
    fn find_matches_are_cached_until_the_query_changes() {
        use std::rc::Rc;

        let mut app = app_with(VCD);
        app.input.insert('c');
        let first = app.find_matches();
        let hit = app.find_matches();
        assert!(Rc::ptr_eq(&first, &hit));
        assert_eq!(first.to_vec(), vec![0]);

        // Typing replaces the query text; the cache must not serve the old
        // list (even though the result happens to stay the same here).
        app.input.insert('l');
        let narrowed = app.find_matches();
        assert!(!Rc::ptr_eq(&first, &narrowed));
        assert_eq!(narrowed.to_vec(), vec![0]);

        // A new waveform invalidates by version even with the same query.
        let out = crate::vcd::parse_bytes(
            b"$timescale 1ns $end\n$var wire 1 ! x $end\n$enddefinitions $end\n#0\n1!\n",
        )
        .unwrap();
        app.apply_waveform("<other>".to_string(), out.wf, out.warnings);
        let fresh = app.find_matches();
        assert!(!Rc::ptr_eq(&narrowed, &fresh));
        assert!(fresh.is_empty());
        assert!(Rc::ptr_eq(&fresh, &app.find_matches()));
    }

    #[test]
    fn list_values_are_cached_until_the_key_changes() {
        use std::rc::Rc;

        let mut app = app_with(
            "$timescale 1ns $end\n\
             $var wire 1 ! clk $end\n\
             $var wire 4 \" data $end\n\
             $enddefinitions $end\n#0\n0!\nb0000 \"\n#10\n1!\nb1111 \"\n",
        );
        app.set_display(vec![0, 1]);
        let (text, width) = app.list_display_value(0);
        assert_eq!(&*text, "b0");
        assert_eq!(width, 2);
        assert!(Rc::ptr_eq(&text, &app.list_display_value(0).0));
        // The Value column width comes from the same cached entries.
        assert_eq!(app.value_content_width(), app.list_display_value(1).1);

        // A cursor move invalidates the whole cache.
        app.move_cursor(10);
        let moved = app.list_display_value(0);
        assert!(!Rc::ptr_eq(&text, &moved.0));
        assert_eq!(&*moved.0, "b1");

        // A radix change invalidates it as well.
        app.apply_radix(0, crate::waveform::Radix::Dec);
        let radix = app.list_display_value(0);
        assert!(!Rc::ptr_eq(&moved.0, &radix.0));
        assert_eq!(&*radix.0, "d1");

        // Adding a signal bumps the pane version; the next read rebuilds.
        let before = app.list_display_value(0);
        app.add_signal(1);
        let after = app.list_display_value(0);
        assert!(!Rc::ptr_eq(&before.0, &after.0));
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
