mod action;
mod browser;
mod context;
mod dialog;
mod input;
mod keys;
mod mouse;
mod nav;
mod value;
mod view;

pub use action::Action;
pub(crate) use browser::is_waveform;
pub use browser::{EntryKind, FileBrowser};
pub use context::{BusBuilder, ContextMenu, CtxEntry, CtxTarget};
pub use dialog::Dialog;
pub use input::{parse_time_spec, InputState};
pub use keys::handle_key;
pub use mouse::handle_mouse;
pub use nav::{Group, ListRow};

use crate::rtl::{RtlDb, SourceSet, SourceView};
use crate::theme::{Theme, ThemeKind, UiSetting, WaveSetting};
use crate::ui::layout::{compute_layout, Layout, Splits};
use crate::waveform::{Radix, Ticks, TimeBase, Waveform};
use ratatui::layout::Rect;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Tree,
    Source,
    List,
    Wave,
}

impl Focus {
    pub fn name(self) -> &'static str {
        match self {
            Focus::Tree => "Instance",
            Focus::Source => "Source",
            Focus::List => "Signal List",
            Focus::Wave => "Waveform",
        }
    }

    pub fn next(self) -> Focus {
        match self {
            Focus::Tree => Focus::Source,
            Focus::Source => Focus::List,
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DragMode {
    Cursor,
    Range,
    Reorder,
    VScroll,
    HScroll,
    TreeScroll,
    DialogScroll,
    SplitTree,
    SplitList,
    SplitTop,
    SplitValue,
}

#[derive(Clone, Copy)]
pub(crate) struct Drag {
    pub mode: DragMode,
    pub start_x: u16,
    pub start_pct: f64,
    /// Row being dragged for `DragMode::Reorder`.
    pub row: usize,
}

/// Flattened view of the hierarchy tree, produced on demand for the Instance pane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreeNode {
    Scope { id: usize, depth: usize },
}

pub struct App {
    pub path: String,
    pub wf: Option<Waveform>,
    pub display: Vec<usize>,
    /// User-defined Signal List groups partitioning `display` into blocks.
    pub groups: Vec<Group>,
    pub sel_row: Option<usize>,
    /// Show full hierarchical signal names in the Signal List (`h`).
    pub show_full_names: bool,
    /// Signals picked with Shift/Ctrl+click (multi-selection).
    pub selection: Vec<usize>,
    /// Signal the current range selection started at (Ctrl+click anchor).
    pub sel_anchor: Option<usize>,
    /// Pending `g` prefix of the vim-style edge navigation.
    pub(crate) pending_g: bool,
    /// Pending `d` prefix: `dd` deletes the selection into the register.
    pub(crate) pending_d: bool,
    /// Visual mode (`V`): `j`/`k` extend the multi-selection.
    pub visual: bool,
    /// Cut/paste register filled by `dd`.
    pub register: Vec<usize>,
    pub row_scroll: usize,
    pub focus: Focus,
    pub expanded: HashSet<usize>,
    pub tree_sel: usize,
    pub tree_scroll: usize,
    pub t0: f64,
    pub scale: f64,
    pub cursor: Ticks,
    /// Time base of the ruler and status readouts (timescale by default).
    pub time_base: TimeBase,
    /// Open time-base dropdown: highlighted entry index.
    pub time_menu: Option<usize>,
    pub range: Option<(Ticks, Ticks)>,
    pub(crate) dragging: Option<Drag>,
    pub last_area: Rect,
    pub messages: Vec<String>,
    pub menu: MenuState,
    pub dialog: Option<Dialog>,
    /// Scroll offset of the dialog body (help / lists).
    pub dialog_scroll: usize,
    pub input: InputState,
    pub find_sel: usize,
    pub radix: HashMap<usize, Radix>,
    pub last_click: Option<(u16, u16, Instant)>,
    pub splits: Splits,
    /// Per-signal analog rendering range; presence means "show as analog".
    pub analog: HashMap<usize, (f64, f64)>,
    /// Last "Find Value" query, searched with `n` / `N`.
    pub value_query: Option<String>,
    pub ctx_menu: Option<ContextMenu>,
    /// Signal waiting for a split width from `Dialog::SplitBus`.
    pub pending_split: Option<usize>,
    /// Group being renamed by `Dialog::GroupName` (stable id).
    pub renaming_group: Option<u32>,
    /// Ordering state of `Dialog::CreateBus`.
    pub bus_builder: Option<BusBuilder>,
    /// Use the native GUI file dialog instead of the built-in browser.
    pub use_gui: bool,
    pub browser: Option<FileBrowser>,
    /// Active colour scheme.
    pub theme: Theme,
    pub theme_kind: ThemeKind,
    /// Selected row of the settings dialog (0 = theme, rest = waveform colours).
    pub settings_sel: usize,
    /// RTL sources for the Source pane (filelist or discovered from the dump).
    pub sources: Option<SourceSet>,
    /// Parsed RTL modules of `sources` (declaration/driver/load lines).
    pub rtl: Option<RtlDb>,
    /// Highlighted source of the instance selected in the Instance pane.
    pub source_view: Option<SourceView>,
    /// True once a filelist was given explicitly (disables auto-discovery).
    pub sources_explicit: bool,
    pending_fit: bool,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            path: String::new(),
            wf: None,
            display: Vec::new(),
            groups: vec![Group::new(0)],
            sel_row: None,
            show_full_names: false,
            selection: Vec::new(),
            sel_anchor: None,
            pending_g: false,
            pending_d: false,
            visual: false,
            register: Vec::new(),
            row_scroll: 0,
            focus: Focus::Tree,
            expanded: HashSet::new(),
            tree_sel: 0,
            tree_scroll: 0,
            t0: 0.0,
            scale: 1.0,
            cursor: 0,
            time_base: TimeBase::Scale,
            time_menu: None,
            range: None,
            dragging: None,
            last_area: Rect::new(0, 0, 0, 0),
            messages: Vec::new(),
            menu: MenuState::default(),
            dialog: None,
            dialog_scroll: 0,
            input: InputState::default(),
            find_sel: 0,
            radix: HashMap::new(),
            last_click: None,
            splits: Splits::default(),
            analog: HashMap::new(),
            value_query: None,
            ctx_menu: None,
            pending_split: None,
            renaming_group: None,
            bus_builder: None,
            use_gui: crate::picker::detect_gui(),
            browser: None,
            theme: Theme::DARK,
            theme_kind: ThemeKind::Dark,
            settings_sel: 0,
            sources: None,
            rtl: None,
            source_view: None,
            sources_explicit: false,
            pending_fit: false,
        };
        app.msg("waverdi 0.1 — press 'o' to open a waveform dump, F1/? for key bindings");
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

    /// Tick range covered by the column the cursor is drawn in.
    ///
    /// Uses rounding, matching `x_at_tick`, so an edge drawn in the cursor's
    /// column counts even when the exact tick is a fraction of a column away.
    pub fn cursor_column_range(&self) -> (f64, f64) {
        let col = self.x_at_tick(self.cursor) as f64;
        let center = self.t0 + col * self.scale;
        (center - 0.5 * self.scale, center + 0.5 * self.scale)
    }

    /// Load a VCD from disk, reporting failures through the message log.
    pub fn load(&mut self, path: &str) -> bool {
        self.msg(format!("Loading {path} ..."));
        match crate::dump::parse(Path::new(path)) {
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

    /// Load the filelist typed into the `Dialog::Filelist` prompt.
    pub fn apply_load_filelist(&mut self) {
        let path = self.input.as_string();
        self.dialog = None;
        let path = path.trim().to_string();
        if !path.is_empty() {
            self.load_filelist(&path);
        }
    }

    /// Load a VCS-style RTL filelist into the Source pane.
    pub fn load_filelist(&mut self, path: &str) {
        match SourceSet::from_filelist(Path::new(path)) {
            Ok(set) if !set.is_empty() => {
                self.msg(format!(
                    "RTL sources: {} file(s) from {}",
                    set.files.len(),
                    set.origin
                ));
                self.sources = Some(set);
                self.sources_explicit = true;
                self.rebuild_rtl();
            }
            Ok(set) => self.msg(format!("filelist {}: no source files found", set.origin)),
            Err(e) => self.msg(format!("filelist error: {e}")),
        }
    }

    /// Open the operating system's file dialog and load the selection.
    pub fn open_file_dialog(&mut self) {
        if self.use_gui {
            if let Some(path) = crate::picker::pick_vcd() {
                self.load(&path.display().to_string());
            }
            return;
        }
        self.open_tui_browser();
    }

    /// Open the built-in terminal file browser (used over SSH / headless).
    pub fn open_tui_browser(&mut self) {
        let start = if self.path.is_empty() {
            std::env::current_dir().unwrap_or_default()
        } else {
            Path::new(&self.path)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default()
        };
        self.browser = Some(FileBrowser::new(&start));
        self.open_dialog(Dialog::Open);
    }

    /// Install a parsed waveform and reset the view state.
    pub fn apply_parsed(&mut self, path: impl Into<String>, out: crate::dump::ParseOut) {
        let wf = out.wf;
        self.msg(format!("Loaded {}", wf.summary()));
        for warning in out.warnings {
            self.msg(format!("  warn: {warning}"));
        }
        self.path = path.into();
        self.expanded.clear();
        self.expanded.insert(wf.tree.root);
        self.display.clear();
        self.groups = vec![Group::new(0)];
        self.sel_row = None;
        self.show_full_names = false;
        self.selection.clear();
        self.sel_anchor = None;
        self.pending_g = false;
        self.pending_d = false;
        self.visual = false;
        self.register.clear();
        self.row_scroll = 0;
        self.tree_sel = 0;
        self.tree_scroll = 0;
        self.range = None;
        self.radix.clear();
        self.analog.clear();
        self.ctx_menu = None;
        self.pending_split = None;
        self.renaming_group = None;
        self.bus_builder = None;
        self.find_sel = 0;
        self.cursor = wf.start;
        self.wf = Some(wf);
        self.dialog = None;
        self.dialog_scroll = 0;
        self.time_menu = None;
        self.focus = Focus::Tree;
        self.pending_fit = true;

        // FSDB dumps that sit next to the Verdi KDB can recover their source
        // list automatically; an explicit filelist always wins.
        if !self.sources_explicit {
            self.sources = self
                .path
                .ends_with(".fsdb")
                .then(|| SourceSet::discover_from_dump(Path::new(&self.path)))
                .flatten();
            if let Some(set) = &self.sources {
                self.msg(format!(
                    "RTL sources: {} file(s) from {}",
                    set.files.len(),
                    set.origin
                ));
            }
            self.rebuild_rtl();
        }
    }

    /// Parse the current source set into the RTL database.
    fn rebuild_rtl(&mut self) {
        self.rtl = self.sources.as_ref().map(RtlDb::parse_sources);
        if let Some(db) = &self.rtl {
            if !db.modules.is_empty() {
                self.msg(format!(
                    "RTL parsed: {} module(s) in {} file(s)",
                    db.modules.len(),
                    db.files.len()
                ));
            }
        }
        self.source_view = None;
        self.sync_source();
    }

    /// Reload the Source pane when the selected instance changed.
    pub fn sync_source(&mut self) {
        let module = self.selected_scope_module();
        if module == self.source_view.as_ref().map(|view| view.module.clone()) {
            return;
        }
        let loaded = module.and_then(|name| {
            let def = self.rtl.as_ref()?.module(&name)?;
            SourceView::load(def)
        });
        self.source_view = loaded;
    }

    fn source_rows(&self) -> usize {
        self.layout().source.height.saturating_sub(3) as usize
    }

    pub fn move_source_cursor(&mut self, delta_line: i64, delta_col: i64) {
        let rows = self.source_rows();
        if let Some(view) = self.source_view.as_mut() {
            view.move_cursor(delta_line, delta_col, rows);
        }
    }

    pub fn set_source_cursor(&mut self, line: usize, col: usize) {
        let rows = self.source_rows();
        if let Some(view) = self.source_view.as_mut() {
            view.set_cursor(line, col, rows);
        }
    }

    pub fn scroll_source(&mut self, delta: i64) {
        let rows = self.source_rows();
        if let Some(view) = self.source_view.as_mut() {
            view.scroll_by(delta, rows);
        }
    }

    pub fn source_page(&mut self, down: bool) {
        let rows = self.source_rows();
        if let Some(view) = self.source_view.as_mut() {
            view.page(down, rows);
        }
    }

    /// Add the identifier under the Source cursor to the Signal List.
    pub fn add_source_word(&mut self) {
        let Some((word, module)) = self.source_view.as_ref().and_then(|view| {
            view.word_at_cursor()
                .map(|word| (word, view.module.clone()))
        }) else {
            self.msg("source: no signal name under the cursor (a: add)");
            return;
        };
        match self.find_signal_in_scope(&word) {
            Some(index) => {
                self.add_signal(index);
                self.focus = Focus::Source;
                self.msg(format!("added {word} from {module} to the Signal List"));
            }
            None => self.msg(format!("source: {word} was not dumped in this scope")),
        }
    }

    /// Instance scope of the selected tree node (without the design root).
    fn selected_scope_steps(&self) -> Vec<String> {
        self.selected_scope_path()
            .map(|path| path.split('.').skip(1).map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Find the dumped signal matching `name` in the selected instance.
    fn find_signal_in_scope(&self, name: &str) -> Option<usize> {
        let scope = self.selected_scope_steps();
        let wf = self.wf.as_ref()?;
        if let Some(index) = wf
            .signals
            .iter()
            .position(|sig| sig.name == name && sig.scope == scope)
        {
            return Some(index);
        }
        let mut matches = wf.signals.iter().enumerate().filter(|(_, sig)| {
            sig.name == name && !scope.is_empty() && sig.scope.starts_with(&scope)
        });
        let first = matches.next()?;
        matches.next().is_none().then_some(first.0)
    }

    /// Open a dialog, resetting its body scroll.
    pub fn open_dialog(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
        self.dialog_scroll = 0;
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
        let targets = self.selected_signals();
        if targets.is_empty() {
            return;
        }
        let mut last = Radix::Bin;
        for &idx in &targets {
            let next = self.radix_for(idx).next();
            self.radix.insert(idx, next);
            last = next;
        }
        if targets.len() == 1 {
            let name = self.wf.as_ref().unwrap().signals[targets[0]].name.clone();
            self.msg(format!("radix of {name}: {}", last.name()));
        } else {
            self.msg(format!(
                "radix of {} signals: {}",
                targets.len(),
                last.name()
            ));
        }
    }

    /// Label of the current time base for the shortcut bar.
    pub fn time_base_label(&self) -> String {
        match self.time_base {
            TimeBase::Scale => self
                .wf
                .as_ref()
                .map(|wf| wf.ts.label())
                .unwrap_or_else(|| "ts".to_string()),
            other => other.label().to_string(),
        }
    }

    /// Select an explicit ruler time base from the shortcut-bar dropdown.
    pub fn set_time_base(&mut self, base: TimeBase) {
        self.time_base = base;
        self.time_menu = None;
        let label = self.time_base_label();
        self.msg(format!("time base: {label}"));
    }

    /// Switch the colour scheme, dropping any per-colour customisation.
    pub fn set_theme_kind(&mut self, kind: ThemeKind) {
        self.theme_kind = kind;
        self.theme = kind.theme();
        self.msg(format!("theme: {}", kind.name()));
    }

    /// Cycle the colour scheme from the settings dialog.
    pub fn cycle_theme(&mut self, delta: i64) {
        let count = ThemeKind::ALL.len() as i64;
        let index = ThemeKind::ALL
            .iter()
            .position(|kind| *kind == self.theme_kind)
            .unwrap_or(0) as i64;
        let next = (index + delta).rem_euclid(count) as usize;
        self.set_theme_kind(ThemeKind::ALL[next]);
    }

    /// Cycle one waveform colour through the built-in palette.
    pub fn cycle_wave_setting(&mut self, setting: WaveSetting, delta: i64) {
        let color = crate::theme::cycle_color(setting.get(&self.theme), delta);
        setting.set(&mut self.theme, color);
    }

    /// Restore the waveform colour to the value of the active theme.
    pub fn reset_wave_setting(&mut self, setting: WaveSetting) {
        let base = self.theme_kind.theme();
        setting.set(&mut self.theme, setting.get(&base));
    }

    /// Cycle a general UI colour through the built-in palette.
    pub fn cycle_ui_setting(&mut self, setting: UiSetting, delta: i64) {
        let color = crate::theme::cycle_color(setting.get(&self.theme), delta);
        setting.set(&mut self.theme, color);
    }

    /// Restore a general UI colour to the value of the active theme.
    pub fn reset_ui_setting(&mut self, setting: UiSetting) {
        let base = self.theme_kind.theme();
        setting.set(&mut self.theme, setting.get(&base));
    }

    /// Settings dialog rows: theme, UI colours, then waveform colours.
    pub fn settings_len(&self) -> usize {
        1 + UiSetting::ALL.len() + WaveSetting::ALL.len()
    }

    /// UI colour shown on a settings row, if the row is a UI colour.
    pub fn settings_ui(&self, row: usize) -> Option<UiSetting> {
        row.checked_sub(1)
            .filter(|index| *index < UiSetting::ALL.len())
            .and_then(|index| UiSetting::ALL.get(index).copied())
    }

    /// Waveform colour shown on a settings row, if the row is one.
    pub fn settings_setting(&self, row: usize) -> Option<WaveSetting> {
        row.checked_sub(1 + UiSetting::ALL.len())
            .and_then(|index| WaveSetting::ALL.get(index).copied())
    }

    /// Test helper: install a flat display list owned by the first group.
    #[cfg(test)]
    pub fn set_display(&mut self, sigs: Vec<usize>) {
        self.display = sigs;
        if self.groups.is_empty() {
            self.groups.push(Group::new(0));
        }
        for group in self.groups.iter_mut().skip(1) {
            group.count = 0;
        }
        self.groups[0].count = self.display.len();
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
        let out = crate::vcd::parse_bytes(vcd_text.as_bytes()).unwrap();
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
    fn filelist_loading_and_fsdb_source_discovery() {
        let dir = std::env::temp_dir().join(format!("waverdi_app_fl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("simv.daidir/debug_dump")).unwrap();
        let src = dir.join("top.sv");
        std::fs::write(&src, "module top; endmodule\n").unwrap();
        std::fs::write(
            dir.join("simv.daidir/debug_dump/src_files_verilog"),
            format!("{}\n", src.display()),
        )
        .unwrap();
        let list = dir.join("files.f");
        std::fs::write(&list, "top.sv\n").unwrap();

        let mut app = App::new();
        app.load_filelist(&list.display().to_string());
        assert!(app.sources_explicit);
        assert_eq!(app.sources.as_ref().unwrap().files, vec![src.clone()]);

        // Loading an .fsdb without an explicit filelist discovers the KDB list.
        let mut app = App::new();
        let out = crate::vcd::parse_bytes(
            b"$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n",
        )
        .unwrap();
        app.apply_parsed(dir.join("counter.fsdb").display().to_string(), out);
        let set = app.sources.as_ref().expect("discovered sources");
        assert!(set.files.iter().any(|file| file.ends_with("top.sv")));
    }

    #[test]
    fn theme_switching_and_wave_colour_customisation() {
        use crate::theme::{Theme, ThemeKind, WaveSetting};
        let mut app =
            app_with("$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n");
        assert_eq!(app.theme_kind, ThemeKind::Dark);
        app.cycle_theme(1);
        assert_eq!(app.theme_kind, ThemeKind::Light);
        assert_eq!(app.theme.bg, Theme::LIGHT.bg);
        // The UI background and the waveform background are separate settings.
        app.cycle_ui_setting(crate::theme::UiSetting::Background, 1);
        assert_ne!(app.theme.bg, Theme::LIGHT.bg);
        assert_eq!(app.theme.wave_bg, Theme::LIGHT.wave_bg);
        app.reset_ui_setting(crate::theme::UiSetting::Background);
        assert_eq!(app.theme.bg, Theme::LIGHT.bg);
        // A custom waveform colour is applied and can be reset.
        app.cycle_wave_setting(WaveSetting::High, 1);
        assert_ne!(app.theme.high, Theme::LIGHT.high);
        app.reset_wave_setting(WaveSetting::High);
        assert_eq!(app.theme.high, Theme::LIGHT.high);
        // Mixed keeps light chrome with a dark waveform.
        app.cycle_theme(1); // Light -> Mixed
        assert_eq!(app.theme_kind, ThemeKind::Mixed);
        assert_eq!(app.theme.bg, Theme::LIGHT.bg);
        assert_eq!(app.theme.wave_bg, Theme::DARK.bg);
        assert_eq!(app.theme.high, Theme::DARK.high);
        // The cycle wraps back to dark.
        app.cycle_theme(1);
        assert_eq!(app.theme_kind, ThemeKind::Dark);
    }

    #[test]
    fn open_without_gui_uses_tui_browser() {
        let mut app =
            app_with("$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n");
        app.use_gui = false;
        Action::Open.run(&mut app);
        assert_eq!(app.dialog, Some(Dialog::Open));
        assert!(app.browser.is_some());
    }

    #[test]
    fn radix_defaults_and_cycles() {
        let mut app = app_with(
            "$timescale 1ns $end\n$var wire 1 ! clk $end\n$var wire 4 \" data $end\n$enddefinitions $end\n#0\n0!\nb0000 \"\n",
        );
        assert_eq!(app.radix_for(0), Radix::Bin);
        assert_eq!(app.radix_for(1), Radix::Hex);
        app.set_display(vec![1]);
        app.sel_row = Some(1);
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
