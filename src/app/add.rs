//! "Add Signals" picker: hierarchy tree, instances of the current level and
//! the signals of that level with a type filter.

use super::{App, Dialog, TreeNode};
use crate::waveform::{SigKind, Signal};
use std::collections::HashSet;
use std::rc::Rc;

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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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

    pub fn prev(self) -> Self {
        match self {
            AddFocus::Tree => AddFocus::Signals,
            AddFocus::Instances => AddFocus::Tree,
            AddFocus::Signals => AddFocus::Instances,
        }
    }
}

/// One semantic step of the Add Signals picker. The key and mouse handlers
/// both map their events onto this list and run it through
/// [`App::add_action`], so the picker behaviour cannot drift between them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddAction {
    Close,
    /// Move the focus through the panes; positive steps go forward.
    CycleFocus(i8),
    /// Move the focused pane's cursor.
    Move(i64),
    /// Put the tree cursor on a node and expand or collapse it.
    ToggleNode(usize),
    /// Put the tree cursor on a node and make it the current level.
    OpenScope(usize),
    /// Open the instance at that position of the Instances pane.
    OpenInstance(usize),
    /// Toggle one signal (waveform index).
    ToggleSignal(usize),
    /// Toggle the signal under the Signals-pane cursor.
    ToggleCursorSignal,
    Apply {
        close: bool,
    },
    CycleFilter,
    /// Wheel over one pane.
    Scroll {
        pane: AddFocus,
        rows: i64,
    },
    /// Drag one pane's scrollbar to a screen row.
    ScrollTo {
        pane: AddFocus,
        row: u16,
    },
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

/// Items and display names of one picker pane, valid while the picker level,
/// its filter and the waveform are unchanged. Draw and hit testing share it,
/// so names are cloned once per content change instead of per frame and per
/// event. The two grid panes are cached separately because a frame reads both.
pub(crate) struct AddNamesCache {
    key: (usize, AddFilter, u64, u64),
    items: Rc<[usize]>,
    names: Rc<[String]>,
}

impl App {
    /// Items of the "Add Signals" picker: signal (or tree node) indices and
    /// their display names, cached per pane until the picker content changes.
    pub(crate) fn add_pane(&self, pane: AddFocus) -> (Rc<[usize]>, Rc<[String]>) {
        let Some(add) = &self.add_signals else {
            return (Rc::from([]), Rc::from([]));
        };
        let key = (
            add.scope,
            add.filter,
            self.waveform_version,
            self.rtl_version,
        );
        let mut cache = self.add_names.borrow_mut();
        if let Some(cached) = cache.get(&pane) {
            if cached.key == key {
                return (cached.items.clone(), cached.names.clone());
            }
        }
        let items: Vec<usize> = match pane {
            AddFocus::Instances => self.add_instances(),
            AddFocus::Signals => self.add_signal_list(),
            AddFocus::Tree => Vec::new(),
        };
        let names: Vec<String> = items
            .iter()
            .map(|&index| match pane {
                AddFocus::Instances => self
                    .wf
                    .as_ref()
                    .map(|wf| wf.tree.nodes[index].name.clone())
                    .unwrap_or_default(),
                _ => self
                    .wf
                    .as_ref()
                    .and_then(|wf| wf.signals.get(index))
                    .map(|signal| signal.name.clone())
                    .unwrap_or_default(),
            })
            .collect();
        let items: Rc<[usize]> = items.into();
        let names: Rc<[String]> = names.into();
        cache.insert(
            pane,
            AddNamesCache {
                key,
                items: items.clone(),
                names: names.clone(),
            },
        );
        (items, names)
    }

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

    /// Number of items in a picker pane. The grid panes read the cached item
    /// list so the draw and hit-test paths do not rebuild indices per frame.
    pub fn add_pane_len(&self, pane: AddFocus) -> usize {
        match pane {
            AddFocus::Tree => self.add_tree_rows().len(),
            AddFocus::Instances | AddFocus::Signals => self.add_pane(pane).0.len(),
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

    /// Cycle the type filter through the ones the backend can fill. Dumps
    /// without recorded port directions (VCD/FST) skip input/output/inout, so
    /// their picker only shows all/net/reg. The walk is bounded by the cycle
    /// length, so it always terminates.
    pub fn add_cycle_filter(&mut self) {
        let directions = self.backend_caps.directions;
        if let Some(add) = &mut self.add_signals {
            for _ in 0..AddFilter::CYCLE.len() {
                add.filter = add.filter.next();
                if directions
                    || !matches!(
                        add.filter,
                        AddFilter::Inputs | AddFilter::Outputs | AddFilter::Inouts
                    )
                {
                    break;
                }
            }
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
        self.add_toggle_signal_at(index);
    }

    /// Toggle one signal and leave the Signals-pane cursor on it.
    pub fn add_toggle_signal_at(&mut self, index: usize) {
        let position = self
            .add_signal_list()
            .iter()
            .position(|&signal| signal == index);
        if let Some(add) = &mut self.add_signals {
            add.focus = AddFocus::Signals;
            if let Some(position) = position {
                add.signal_sel = position;
            }
            if !add.selected.remove(&index) {
                add.selected.insert(index);
            }
        }
    }

    /// Move the picker focus one pane forward (`step >= 0`) or backward.
    fn add_cycle_focus(&mut self, step: i8) {
        if let Some(add) = &mut self.add_signals {
            add.focus = if step >= 0 {
                add.focus.next()
            } else {
                add.focus.prev()
            };
        }
    }

    /// Put the tree cursor on `node` and expand or collapse it.
    pub fn add_toggle_node(&mut self, node: usize) {
        let position = self.add_tree_rows().iter().position(|(id, _)| *id == node);
        if let Some(add) = &mut self.add_signals {
            add.focus = AddFocus::Tree;
            if let Some(position) = position {
                add.tree_sel = position;
            }
            if !add.expanded.remove(&node) {
                add.expanded.insert(node);
            }
        }
    }

    /// Put the tree cursor on `node` and make it the current level.
    pub fn add_open_scope(&mut self, node: usize) {
        let position = self.add_tree_rows().iter().position(|(id, _)| *id == node);
        if let Some(add) = &mut self.add_signals {
            add.focus = AddFocus::Tree;
            if let Some(position) = position {
                add.tree_sel = position;
            }
        }
        self.add_navigate(node);
    }

    /// Open the instance at `index` of the Instances pane.
    pub fn add_open_instance(&mut self, index: usize) {
        let node = self.add_instances().get(index).copied();
        if let Some(add) = &mut self.add_signals {
            add.focus = AddFocus::Instances;
            add.instance_sel = index;
        }
        if let Some(node) = node {
            self.add_navigate(node);
            // Navigating resets the pane cursors; keep the picked row.
            if let Some(add) = &mut self.add_signals {
                add.instance_sel = index;
            }
        }
    }

    /// Run one picker step. The key and mouse handlers map their events onto
    /// [`AddAction`] and go through here, so the two surfaces cannot drift.
    pub fn add_action(&mut self, action: AddAction) {
        match action {
            AddAction::Close => self.close_add_signals(),
            AddAction::CycleFocus(step) => self.add_cycle_focus(step),
            AddAction::Move(delta) => self.add_move(delta),
            AddAction::ToggleNode(node) => self.add_toggle_node(node),
            AddAction::OpenScope(node) => self.add_open_scope(node),
            AddAction::OpenInstance(index) => self.add_open_instance(index),
            AddAction::ToggleSignal(signal) => self.add_toggle_signal_at(signal),
            AddAction::ToggleCursorSignal => self.add_toggle_signal(),
            AddAction::Apply { close } => self.apply_add_signals(close),
            AddAction::CycleFilter => self.add_cycle_filter(),
            AddAction::Scroll { pane, rows } => self.add_scroll_rows(pane, rows),
            AddAction::ScrollTo { pane, row } => self.add_scroll_to_row(pane, row),
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
        self.add_signals(&chosen);
        if close {
            self.close_add_signals();
        } else if let Some(add) = &mut self.add_signals {
            add.selected.clear();
        }
        self.msg(format!("added {count} signal(s) to the waveform"));
    }
}

#[cfg(test)]
mod tests {
    use super::AddFilter;
    use crate::app::tests::app_with;

    /// Dumps without port directions must never cycle onto input/output/inout
    /// (they would always show an empty list), while a backend that records
    /// directions brings them back.
    #[test]
    fn filter_cycle_respects_backend_capabilities() {
        let mut app =
            app_with("$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n");
        app.open_add_signals();
        assert!(!app.backend_caps.directions, "VCD carries no directions");

        let filter = |app: &crate::app::App| app.add_signals.as_ref().unwrap().filter;
        let (mut saw_net, mut saw_reg) = (false, false);
        for _ in 0..AddFilter::CYCLE.len() {
            app.add_cycle_filter();
            assert!(
                !matches!(
                    filter(&app),
                    AddFilter::Inputs | AddFilter::Outputs | AddFilter::Inouts
                ),
                "unsupported filter reached: {:?}",
                filter(&app)
            );
            saw_net |= filter(&app) == AddFilter::Nets;
            saw_reg |= filter(&app) == AddFilter::Regs;
        }
        assert!(saw_net && saw_reg, "net and reg stay reachable");

        app.backend_caps.directions = true;
        let mut saw_input = false;
        for _ in 0..AddFilter::CYCLE.len() {
            app.add_cycle_filter();
            saw_input |= filter(&app) == AddFilter::Inputs;
        }
        assert!(saw_input, "input is reachable with directions recorded");
    }

    /// The picker panes cache their names per pane and drop them when the
    /// filter changes.
    #[test]
    fn picker_pane_names_are_cached_per_pane() {
        use crate::app::AddFocus;
        use std::rc::Rc;

        let vcd = "$timescale 1ns $end\n\
            $scope module top $end\n\
            $var wire 1 ! clk $end\n\
            $var wire 1 \" rst $end\n\
            $scope module sub $end\n\
            $var wire 1 # a $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\n0!\n0\"\n0#\n";
        let mut app = app_with(vcd);
        app.open_add_signals();
        let top = {
            let wf = app.wf.as_ref().unwrap();
            wf.tree.nodes[wf.tree.root].children[0]
        };
        app.add_navigate(top);

        let (_, first) = app.add_pane(AddFocus::Signals);
        let (_, again) = app.add_pane(AddFocus::Signals);
        assert!(
            Rc::ptr_eq(&first, &again),
            "signals pane names must be cached"
        );

        // Alternating panes must not evict each other within one frame.
        let (_, instances) = app.add_pane(AddFocus::Instances);
        let (_, signals) = app.add_pane(AddFocus::Signals);
        assert!(
            Rc::ptr_eq(&first, &signals),
            "instances lookup evicted signals"
        );
        assert_eq!(instances.len(), 1, "only the sub instance at this level");

        // A filter change rebuilds the pane.
        app.add_cycle_filter();
        let (_, filtered) = app.add_pane(AddFocus::Signals);
        assert!(!Rc::ptr_eq(&first, &filtered), "filter change must rebuild");
        assert_eq!(filtered.len(), 2, "both wires pass the net filter");
    }

    const PICKER_VCD: &str = "$timescale 1ns $end\n\
        $var wire 1 ! a $end\n\
        $var wire 1 \" b $end\n\
        $var wire 1 # c $end\n\
        $enddefinitions $end\n#0\n0!\n0\"\n0#\n";

    /// Put the Signal List cursor on the header of the group with `id`.
    fn select_group(app: &mut crate::app::App, id: u32) {
        let row = app
            .list_rows()
            .iter()
            .position(|row| matches!(row, crate::app::ListRow::Group { id: i, .. } if *i == id))
            .expect("group row");
        app.select_row(row);
    }

    /// One Apply action is one batch: all chosen signals land in the group
    /// under the cursor, and only that group's own growth appends a trailing
    /// group. Resolving the target per signal would spread the batch over the
    /// empty groups the earlier inserts appended.
    #[test]
    fn applying_many_signals_onto_a_new_group_creates_one_group() {
        let mut app = app_with(PICKER_VCD);
        app.add_signal(0); // G0 = [a], trailing G1 is the new group
        app.toggle_group(1); // collapse it and put the cursor on its header
        app.open_add_signals();
        app.add_toggle_signal_at(1);
        app.add_toggle_signal_at(2);
        app.apply_add_signals(true);

        assert_eq!(app.display, vec![0, 1, 2]);
        assert_eq!(app.groups.len(), 3, "one group appended, not one per add");
        assert_eq!(app.groups[1].count, 2, "the new group holds all of them");
        assert_eq!(app.groups[2].name, "G2", "a single add would name it G2");
        assert_eq!(app.groups[2].count, 0);
    }

    /// A signal already displayed in an older group must not drag the rest of
    /// the batch back into that group.
    #[test]
    fn applying_a_displayed_signal_again_keeps_the_batch_together() {
        let mut app = app_with(PICKER_VCD);
        app.add_signal(0); // G0 = [a]
        select_group(&mut app, 1); // the new empty group
        app.open_add_signals();
        app.add_toggle_signal_at(0); // already displayed in G0
        app.add_toggle_signal_at(1);
        app.apply_add_signals(true);

        assert_eq!(app.display, vec![0, 0, 1]);
        assert_eq!(app.groups.len(), 3);
        assert_eq!(app.groups[0].count, 1, "the older occurrence stays put");
        assert_eq!(
            app.groups[1].count, 2,
            "the whole batch joins the cursor group"
        );
        assert_eq!(app.groups[2].name, "G2");
    }

    /// Adding into an existing (non-newest) group must not append a group.
    #[test]
    fn applying_many_signals_into_an_existing_group_creates_no_group() {
        let mut app = app_with(PICKER_VCD);
        app.add_signal(0); // the cursor is left on G0's signal
        app.open_add_signals();
        app.add_toggle_signal_at(1);
        app.add_toggle_signal_at(2);
        app.apply_add_signals(true);

        assert_eq!(app.display, vec![0, 1, 2]);
        assert_eq!(app.groups.len(), 2, "G0 grew, the trailing G1 was kept");
        assert_eq!(app.groups[0].count, 3);
        assert_eq!(app.groups[1].count, 0);
    }

    /// A single picker add onto the new group still appends exactly one group.
    #[test]
    fn applying_a_single_signal_onto_a_new_group_creates_one_group() {
        let mut app = app_with(PICKER_VCD);
        app.add_signal(0); // G0 = [a]
        select_group(&mut app, 1);
        app.open_add_signals();
        app.add_toggle_signal_at(1);
        app.apply_add_signals(true);

        assert_eq!(app.display, vec![0, 1]);
        assert_eq!(app.groups.len(), 3);
        assert_eq!(app.groups[1].count, 1);
        assert_eq!(app.groups[2].name, "G2");
        assert_eq!(app.groups[2].count, 0);
    }
}
