//! "Add Signals" picker: hierarchy tree, instances of the current level and
//! the signals of that level with a type filter.

use super::{App, Dialog, TreeNode};
use crate::waveform::{SigKind, Signal};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddFilter {
    All,
    Inputs,
    Outputs,
    Inouts,
    Nets,
    Regs,
}

impl AddFilter {
    pub const CYCLE: [AddFilter; 6] = [
        AddFilter::All,
        AddFilter::Inputs,
        AddFilter::Outputs,
        AddFilter::Inouts,
        AddFilter::Nets,
        AddFilter::Regs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AddFilter::All => "all",
            AddFilter::Inputs => "input",
            AddFilter::Outputs => "output",
            AddFilter::Inouts => "inout",
            AddFilter::Nets => "net",
            AddFilter::Regs => "reg",
        }
    }

    pub fn next(self) -> Self {
        let index = Self::CYCLE
            .iter()
            .position(|filter| *filter == self)
            .unwrap_or(0);
        Self::CYCLE[(index + 1) % Self::CYCLE.len()]
    }

    pub fn matches(self, signal: &Signal) -> bool {
        match self {
            AddFilter::All => true,
            AddFilter::Inputs => signal.dir == "input",
            AddFilter::Outputs => signal.dir == "output",
            AddFilter::Inouts => signal.dir == "inout",
            AddFilter::Nets => is_net(&signal.var_type),
            AddFilter::Regs => {
                signal.kind != SigKind::Str
                    && !is_net(&signal.var_type)
                    && !matches!(signal.var_type.as_str(), "array" | "aggregate")
            }
        }
    }
}

fn is_net(var_type: &str) -> bool {
    matches!(
        var_type,
        "wire"
            | "tri"
            | "triand"
            | "trior"
            | "trireg"
            | "tri0"
            | "tri1"
            | "wand"
            | "wor"
            | "supply0"
            | "supply1"
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddFocus {
    Tree,
    Instances,
    Signals,
}

impl AddFocus {
    pub fn next(self) -> Self {
        match self {
            AddFocus::Tree => AddFocus::Instances,
            AddFocus::Instances => AddFocus::Signals,
            AddFocus::Signals => AddFocus::Tree,
        }
    }
}

pub struct AddSignals {
    /// Hierarchy level whose instances and signals are shown.
    pub scope: usize,
    pub tree_sel: usize,
    pub tree_scroll: usize,
    pub expanded: HashSet<usize>,
    pub instance_sel: usize,
    pub instance_scroll: usize,
    pub signal_sel: usize,
    pub signal_scroll: usize,
    /// Chosen signal indices (still to be added with Apply / OK).
    pub selected: HashSet<usize>,
    /// Struct/interface children of the current level and their aggregate
    /// signal: they are listed in the signals pane, not the instance pane.
    pub aggregates: Vec<(usize, usize)>,
    pub filter: AddFilter,
    pub focus: AddFocus,
}

impl App {
    /// Open the "Add Signals" picker at the currently selected hierarchy.
    pub fn open_add_signals(&mut self) {
        let Some(wf) = &self.wf else {
            self.msg("no waveform loaded");
            return;
        };
        let scope = wf.tree.root;
        let mut expanded = HashSet::new();
        expanded.insert(scope);
        // Start at the instance selected in the Instance pane, if any.
        let mut add = AddSignals {
            scope,
            tree_sel: 0,
            tree_scroll: 0,
            expanded,
            instance_sel: 0,
            instance_scroll: 0,
            signal_sel: 0,
            signal_scroll: 0,
            selected: HashSet::new(),
            aggregates: Vec::new(),
            filter: AddFilter::All,
            focus: AddFocus::Tree,
        };
        if let Some(id) = self.tree_visible().get(self.tree_sel).map(|row| {
            let TreeNode::Scope { id, .. } = row;
            *id
        }) {
            add.scope = id;
            add.expanded.insert(id);
        }
        add.aggregates = self.add_aggregate_cache(add.scope);
        self.add_signals = Some(add);
        self.open_dialog(Dialog::AddSignals);
    }

    pub fn close_add_signals(&mut self) {
        self.add_signals = None;
        if self.dialog == Some(Dialog::AddSignals) {
            self.dialog = None;
        }
    }

    /// Flattened scope rows of the picker tree: `(node, depth)`.
    pub fn add_tree_rows(&self) -> Vec<(usize, usize)> {
        let Some(add) = &self.add_signals else {
            return Vec::new();
        };
        let Some(wf) = &self.wf else {
            return Vec::new();
        };
        fn rec(
            wf: &crate::waveform::Waveform,
            id: usize,
            depth: usize,
            expanded: &HashSet<usize>,
            out: &mut Vec<(usize, usize)>,
        ) {
            out.push((id, depth));
            if expanded.contains(&id) {
                for &child in &wf.tree.nodes[id].children {
                    rec(wf, child, depth + 1, expanded, out);
                }
            }
        }
        let mut out = Vec::new();
        rec(wf, wf.tree.root, 0, &add.expanded, &mut out);
        out
    }

    /// Child instances of the picker's current scope. Struct/interface
    /// children are values, not instances, and are listed with the signals.
    pub fn add_instances(&self) -> Vec<usize> {
        let Some(add) = &self.add_signals else {
            return Vec::new();
        };
        let Some(wf) = &self.wf else {
            return Vec::new();
        };
        wf.tree.nodes[add.scope]
            .children
            .iter()
            .copied()
            .filter(|child| !add.aggregates.iter().any(|&(node, _)| node == *child))
            .collect()
    }

    /// Aggregate signal of every struct/interface child of `scope`.
    fn add_aggregate_cache(&self, scope: usize) -> Vec<(usize, usize)> {
        let Some(wf) = &self.wf else {
            return Vec::new();
        };
        let path = wf.tree.path_of(scope);
        let mut out = Vec::new();
        for &child in &wf.tree.nodes[scope].children {
            if !self.add_is_structured_child(child) {
                continue;
            }
            let mut child_path = path.clone();
            child_path.push(wf.tree.nodes[child].name.clone());
            if let Some(aggregate) = wf.scope_aggregate(&child_path) {
                out.push((child, aggregate));
            }
        }
        out
    }

    /// True when the child is a struct/union/record or an interface value.
    fn add_is_structured_child(&self, node: usize) -> bool {
        let Some(wf) = &self.wf else {
            return false;
        };
        let Some(scope) = wf.tree.nodes.get(node) else {
            return false;
        };
        if scope.group || scope.module.contains("/.") {
            return true;
        }
        if scope.module.is_empty() {
            return false;
        }
        self.rtl
            .as_ref()
            .and_then(|db| db.modules.get(&scope.module))
            .map(|def| def.kind == crate::rtl::scan::DefKind::Interface)
            .unwrap_or(false)
    }

    /// Signals of the picker's current scope that pass the filter, followed by
    /// the struct/interface aggregates of that level.
    pub fn add_signal_list(&self) -> Vec<usize> {
        let Some(add) = &self.add_signals else {
            return Vec::new();
        };
        let Some(wf) = &self.wf else {
            return Vec::new();
        };
        let pass = |&index: &usize| {
            wf.signals
                .get(index)
                .map(|signal| add.filter.matches(signal))
                .unwrap_or(false)
        };
        let mut out: Vec<usize> = wf.tree.nodes[add.scope]
            .signals
            .iter()
            .copied()
            .filter(pass)
            .collect();
        out.extend(
            add.aggregates
                .iter()
                .map(|&(_, aggregate)| aggregate)
                .filter(pass),
        );
        out
    }

    /// Switch the picker to another hierarchy level.
    pub fn add_navigate(&mut self, node: usize) {
        let Some(wf) = &self.wf else { return };
        if node >= wf.tree.nodes.len() {
            return;
        }
        let aggregates = self.add_aggregate_cache(node);
        if let Some(add) = &mut self.add_signals {
            add.scope = node;
            add.aggregates = aggregates;
            add.instance_sel = 0;
            add.instance_scroll = 0;
            add.signal_sel = 0;
            add.signal_scroll = 0;
            add.selected.clear();
        }
    }

    /// Move the cursor of the focused picker pane.
    pub fn add_move(&mut self, delta: i64) {
        let Some(add) = &self.add_signals else {
            return;
        };
        let focus = add.focus;
        let len = self.add_pane_len(focus);
        if len == 0 {
            return;
        }
        let next = (self.add_cursor(focus) as i64 + delta).clamp(0, len as i64 - 1) as usize;
        if focus == AddFocus::Tree {
            let node = self.add_tree_rows().get(next).map(|(id, _)| *id);
            if let Some(node) = node {
                self.add_navigate(node);
            }
            if let Some(add) = &mut self.add_signals {
                add.tree_sel = next;
            }
        } else if let Some(add) = &mut self.add_signals {
            match focus {
                AddFocus::Instances => add.instance_sel = next,
                AddFocus::Signals => add.signal_sel = next,
                AddFocus::Tree => {}
            }
        }
        self.add_ensure_visible(focus);
    }

    /// Number of items in a picker pane.
    pub fn add_pane_len(&self, pane: AddFocus) -> usize {
        match pane {
            AddFocus::Tree => self.add_tree_rows().len(),
            AddFocus::Instances => self.add_instances().len(),
            AddFocus::Signals => self.add_signal_list().len(),
        }
    }

    /// Cursor position inside a picker pane.
    fn add_cursor(&self, pane: AddFocus) -> usize {
        let Some(add) = &self.add_signals else {
            return 0;
        };
        match pane {
            AddFocus::Tree => add.tree_sel,
            AddFocus::Instances => add.instance_sel,
            AddFocus::Signals => add.signal_sel,
        }
    }

    /// Number of items visible at once in a picker pane.
    fn add_visible(&self, pane: AddFocus) -> usize {
        crate::ui::add::pane_visible(self.last_area, self, pane)
    }

    /// Window start index of a picker pane, clamped to the current content.
    pub fn add_scroll_of(&self, pane: AddFocus) -> usize {
        let Some(add) = &self.add_signals else {
            return 0;
        };
        let start = match pane {
            AddFocus::Tree => add.tree_scroll,
            AddFocus::Instances => add.instance_scroll,
            AddFocus::Signals => add.signal_scroll,
        };
        let visible = self.add_visible(pane);
        start.min(self.add_pane_len(pane).saturating_sub(visible))
    }

    /// Scroll a pane to an absolute window start (wheel / scrollbar drag).
    pub fn add_set_scroll(&mut self, pane: AddFocus, start: usize) {
        let visible = self.add_visible(pane);
        let len = self.add_pane_len(pane);
        if len == 0 {
            return;
        }
        let start = start.min(len.saturating_sub(visible));
        if let Some(add) = &mut self.add_signals {
            match pane {
                AddFocus::Tree => add.tree_scroll = start,
                AddFocus::Instances => add.instance_scroll = start,
                AddFocus::Signals => add.signal_scroll = start,
            }
        }
    }

    /// Wheel over a pane: scroll whole rows (grid panes move a full row).
    pub fn add_scroll_rows(&mut self, pane: AddFocus, rows: i64) {
        let step = crate::ui::add::pane_cols(self.last_area, self, pane) as i64;
        let start = (self.add_scroll_of(pane) as i64 + rows * step).max(0);
        self.add_set_scroll(pane, start as usize);
    }

    /// Drag a pane's scrollbar to a screen row.
    pub fn add_scroll_to_row(&mut self, pane: AddFocus, row: u16) {
        let l = crate::ui::add::layout(self.last_area);
        let rect = match pane {
            AddFocus::Tree => l.tree,
            AddFocus::Instances => l.instances,
            AddFocus::Signals => l.signals,
        };
        let visible = self.add_visible(pane);
        let len = self.add_pane_len(pane);
        let rel = row.saturating_sub(rect.y) as usize;
        let start = crate::ui::add::scroll_from_track(rect.height as usize, len, visible, rel);
        self.add_set_scroll(pane, start);
    }

    /// Keep the cursor of a pane inside the visible window.
    fn add_ensure_visible(&mut self, pane: AddFocus) {
        let visible = self.add_visible(pane).max(1);
        let len = self.add_pane_len(pane);
        if len == 0 {
            return;
        }
        let cursor = self.add_cursor(pane);
        let scroll = self.add_scroll_of(pane);
        let start = if cursor < scroll {
            cursor
        } else if cursor >= scroll + visible {
            cursor + 1 - visible
        } else {
            scroll
        }
        .min(len.saturating_sub(visible));
        if start != scroll {
            if let Some(add) = &mut self.add_signals {
                match pane {
                    AddFocus::Tree => add.tree_scroll = start,
                    AddFocus::Instances => add.instance_scroll = start,
                    AddFocus::Signals => add.signal_scroll = start,
                }
            }
        }
    }

    /// Expand/collapse the selected tree row.
    pub fn add_toggle_expand(&mut self) {
        let Some((node, _)) = self
            .add_signals
            .as_ref()
            .and_then(|add| self.add_tree_rows().get(add.tree_sel).copied())
        else {
            return;
        };
        if let Some(add) = &mut self.add_signals {
            if !add.expanded.remove(&node) {
                add.expanded.insert(node);
            }
        }
    }

    pub fn add_cycle_filter(&mut self) {
        if let Some(add) = &mut self.add_signals {
            add.filter = add.filter.next();
            add.signal_sel = 0;
            add.signal_scroll = 0;
        }
    }

    /// Toggle the signal under the cursor of the Signals pane.
    pub fn add_toggle_signal(&mut self) {
        let Some(index) = self
            .add_signals
            .as_ref()
            .and_then(|add| self.add_signal_list().get(add.signal_sel).copied())
        else {
            return;
        };
        if let Some(add) = &mut self.add_signals {
            if !add.selected.remove(&index) {
                add.selected.insert(index);
            }
        }
    }

    pub fn add_toggle_signal_at(&mut self, index: usize) {
        if let Some(add) = &mut self.add_signals {
            if !add.selected.remove(&index) {
                add.selected.insert(index);
            }
        }
    }

    /// Add the chosen signals to the waveform. `close` finishes the dialog.
    pub fn apply_add_signals(&mut self, close: bool) {
        let chosen: Vec<usize> = {
            let Some(add) = &self.add_signals else { return };
            self.add_signal_list()
                .into_iter()
                .filter(|index| add.selected.contains(index))
                .collect()
        };
        if chosen.is_empty() {
            self.msg("add signals: nothing selected");
            return;
        }
        let count = chosen.len();
        for index in chosen {
            self.add_signal(index);
        }
        if close {
            self.close_add_signals();
        } else if let Some(add) = &mut self.add_signals {
            add.selected.clear();
        }
        self.msg(format!("added {count} signal(s) to the waveform"));
    }
}
