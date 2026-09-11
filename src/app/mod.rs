mod action;
mod dialog;
mod input;
mod keys;
mod mouse;
mod nav;
mod view;

pub use action::Action;
pub use dialog::Dialog;
pub use input::{parse_time_spec, InputState};
pub use keys::handle_key;
pub use mouse::handle_mouse;

use crate::ui::layout::{compute_layout, Layout, Splits};
use crate::vcd;
use crate::waveform::{Radix, Ticks, Waveform};
use ratatui::layout::Rect;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Tree,
    List,
    Wave,
}

impl Focus {
    pub fn name(self) -> &'static str {
        match self {
            Focus::Tree => "nTrace",
            Focus::List => "Signal List",
            Focus::Wave => "Waveform",
        }
    }

    pub fn next(self) -> Focus {
        match self {
            Focus::Tree => Focus::List,
            Focus::List => Focus::Wave,
            Focus::Wave => Focus::Tree,
        }
    }
}

#[derive(Clone, Default)]
pub struct MenuState {
    pub open: Option<usize>,
    pub sel: usize,
}

#[derive(Clone, Copy)]
pub(crate) enum DragMode {
    Cursor,
    Range,
    VScroll,
    HScroll,
    TreeScroll,
    SplitTree,
    SplitList,
}

#[derive(Clone, Copy)]
pub(crate) struct Drag {
    pub mode: DragMode,
    pub start_x: u16,
    pub start_pct: f64,
}

/// Flattened view of the hierarchy tree, produced on demand for the nTrace pane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreeNode {
    Scope { id: usize, depth: usize },
    Signal { sig: usize, depth: usize },
}

pub struct App {
    pub path: String,
    pub wf: Option<Waveform>,
    pub display: Vec<usize>,
    pub sel_row: Option<usize>,
    pub row_scroll: usize,
    pub focus: Focus,
    pub expanded: HashSet<usize>,
    pub tree_sel: usize,
    pub tree_scroll: usize,
    pub t0: f64,
    pub scale: f64,
    pub cursor: Ticks,
    pub range: Option<(Ticks, Ticks)>,
    pub(crate) dragging: Option<Drag>,
    pub last_area: Rect,
    pub messages: Vec<String>,
    pub menu: MenuState,
    pub dialog: Option<Dialog>,
    pub input: InputState,
    pub find_sel: usize,
    pub radix: HashMap<usize, Radix>,
    pub last_click: Option<(u16, u16, Instant)>,
    pub splits: Splits,
    pending_fit: bool,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            path: String::new(),
            wf: None,
            display: Vec::new(),
            sel_row: None,
            row_scroll: 0,
            focus: Focus::Tree,
            expanded: HashSet::new(),
            tree_sel: 0,
            tree_scroll: 0,
            t0: 0.0,
            scale: 1.0,
            cursor: 0,
            range: None,
            dragging: None,
            last_area: Rect::new(0, 0, 0, 0),
            messages: Vec::new(),
            menu: MenuState::default(),
            dialog: None,
            input: InputState::default(),
            find_sel: 0,
            radix: HashMap::new(),
            last_click: None,
            splits: Splits::default(),
            pending_fit: false,
        };
        app.msg("waverdi 0.1 — press 'o' to open a VCD file, F1/? for key bindings");
        app
    }

    pub fn msg(&mut self, s: impl Into<String>) {
        self.messages.push(s.into());
        let n = self.messages.len();
        if n > 200 {
            self.messages.drain(..n - 200);
        }
    }

    /// Called once per frame before drawing so view math uses current geometry.
    pub fn sync_layout(&mut self, area: Rect) {
        self.last_area = area;
        if self.pending_fit && area.width > 0 && self.wf.is_some() {
            self.pending_fit = false;
            self.fit();
        }
    }

    pub fn layout(&self) -> Layout {
        compute_layout(self.last_area, self.splits)
    }

    pub fn cols(&self) -> usize {
        self.layout().cols
    }

    pub fn rows_h(&self) -> usize {
        self.layout().rows_h
    }

    pub fn span(&self) -> f64 {
        self.cols().max(1) as f64 * self.scale
    }

    pub fn tick_at_x(&self, x: u16) -> Ticks {
        let l = self.layout();
        let xr = x.saturating_sub(l.wave.x) as f64 + 0.5;
        ((self.t0 + xr * self.scale).max(0.0)) as Ticks
    }

    pub fn x_at_tick(&self, t: Ticks) -> i64 {
        (((t as f64) - self.t0) / self.scale).round() as i64
    }

    /// Load a VCD from disk, reporting failures through the message log.
    pub fn load(&mut self, path: &str) -> bool {
        self.msg(format!("Loading {path} ..."));
        match vcd::parse_vcd(Path::new(path)) {
            Ok(out) => {
                self.apply_parsed(path, out);
                set_title(path);
                true
            }
            Err(e) => {
                self.msg(format!("Error: {e}"));
                self.dialog = None;
                false
            }
        }
    }

    /// Open the operating system's file dialog and load the selection.
    pub fn open_file_dialog(&mut self) {
        if let Some(path) = crate::picker::pick_vcd() {
            self.load(&path.display().to_string());
        }
    }

    /// Install a parsed waveform and reset the view state.
    pub fn apply_parsed(&mut self, path: impl Into<String>, out: vcd::ParseOut) {
        let wf = out.wf;
        self.msg(format!("Loaded {}", wf.summary()));
        for warning in out.warnings {
            self.msg(format!("  warn: {warning}"));
        }
        self.path = path.into();
        self.expanded.clear();
        self.expanded.insert(wf.tree.root);
        self.display.clear();
        self.sel_row = None;
        self.row_scroll = 0;
        self.tree_sel = 0;
        self.tree_scroll = 0;
        self.range = None;
        self.radix.clear();
        self.find_sel = 0;
        self.cursor = wf.start;
        self.wf = Some(wf);
        self.dialog = None;
        self.focus = Focus::Tree;
        self.pending_fit = true;
    }

    pub fn radix_for(&self, idx: usize) -> Radix {
        if let Some(r) = self.radix.get(&idx) {
            return *r;
        }
        if self
            .wf
            .as_ref()
            .map(|w| w.signals[idx].bits == 1)
            .unwrap_or(false)
        {
            Radix::Bin
        } else {
            Radix::Hex
        }
    }

    pub fn cycle_radix(&mut self) {
        let Some(idx) = self.selected_signal() else {
            return;
        };
        let next = self.radix_for(idx).next();
        self.radix.insert(idx, next);
        let name = self.wf.as_ref().unwrap().signals[idx].name.clone();
        self.msg(format!("radix of {name}: {}", next.name()));
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

pub fn set_title(path: &str) {
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::SetTitle(format!("waverdi - {path}"))
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    pub(crate) fn app_with(vcd_text: &str) -> App {
        let out = vcd::parse_bytes(vcd_text.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(Rect::new(0, 0, 100, 40));
        app
    }

    #[test]
    fn apply_parsed_resets_view() {
        let app = app_with(
            "$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n#10\n1!\n",
        );
        assert_eq!(app.path, "<test>");
        assert_eq!(app.display.len(), 0);
        assert_eq!(app.cursor, 0);
        assert!(app.expanded.contains(&app.wf.as_ref().unwrap().tree.root));
        // After sync_layout the pending fit ran, so the full range is visible.
        let wf = app.wf.as_ref().unwrap();
        assert!((app.scale - wf.total_ticks() as f64 / app.cols() as f64).abs() < 1e-9);
    }

    #[test]
    fn radix_defaults_and_cycles() {
        let mut app = app_with(
            "$timescale 1ns $end\n$var wire 1 ! clk $end\n$var wire 4 \" data $end\n$enddefinitions $end\n#0\n0!\nb0000 \"\n",
        );
        assert_eq!(app.radix_for(0), Radix::Bin);
        assert_eq!(app.radix_for(1), Radix::Hex);
        app.display.push(1);
        app.sel_row = Some(0);
        let start = app.radix_for(1);
        let mut seen = HashSet::new();
        for _ in 0..Radix::CYCLE.len() {
            seen.insert(app.radix_for(1));
            app.cycle_radix();
        }
        assert_eq!(seen.len(), Radix::CYCLE.len());
        assert_eq!(app.radix_for(1), start);
    }
}
