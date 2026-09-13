mod action;
mod browser;
mod context;
mod dialog;
mod input;
mod keys;
mod load;
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
pub use load::{LoadEvent, LoadJob};
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
    GroupReorder,
    VScroll,
    HScroll,
    TreeScroll,
    SourceScroll,
    SourceHScroll,
    ListHScroll,
    DialogScroll,
    SplitTree,
    SplitList,
    SplitTop,
    SplitValue,
    SplitHier,
    TreeHScroll,
    ModuleHScroll,
    ValueHScroll,
    SourceSel,
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

/// Purpose of the built-in browser / native file dialog (`Open Waveform`
/// versus `Load Filelist`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BrowserMode {
    Waveform,
    Filelist,
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
    /// Horizontal scroll of the Signal List names (`usize::MAX` = right edge).
    pub(crate) list_h_scroll: usize,
    /// Horizontal scroll of the Source pane (in characters).
    pub(crate) source_h_scroll: usize,
    /// Horizontal scroll of the Value column.
    pub(crate) value_h_scroll: usize,
    /// Horizontal scroll of the Instance pane's Hierarchy column.
    pub(crate) tree_h_scroll: usize,
    /// Horizontal scroll of the Instance pane's Module column.
    pub(crate) module_h_scroll: usize,
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
    /// What the next file-dialog pick should load.
    pub(crate) browser_mode: BrowserMode,
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
    /// Last trace line pushed to the log (avoids repeating it every frame).
    pub(crate) last_source_trace: Option<String>,
    /// True once a filelist was given explicitly (disables auto-discovery).
    pub sources_explicit: bool,
    /// Background waveform load in flight (progress shown in the status line).
    pub load: Option<LoadJob>,
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
            list_h_scroll: usize::MAX,
            source_h_scroll: 0,
            value_h_scroll: 0,
            tree_h_scroll: 0,
            module_h_scroll: 0,
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
            browser_mode: BrowserMode::Waveform,
            theme: Theme::DARK,
            theme_kind: ThemeKind::Dark,
            settings_sel: 0,
            sources: None,
            rtl: None,
            source_view: None,
            last_source_trace: None,
            sources_explicit: false,
            load: None,
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
        self.clamp_tree_scroll();
        self.clamp_row_scroll();
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

    /// Start loading a waveform on a background thread; the status line shows
    /// progress while it runs.
    pub fn start_load(&mut self, path: &str) {
        if let Some(job) = &self.load {
            job.cancel();
        }
        // The old dump is about to be replaced: stop any lazy loads for it so
        // their requests cannot be routed to the new backend.
        if let Some(wf) = &mut self.wf {
            for signal in &mut wf.signals {
                if signal.state != crate::waveform::SigState::Ready {
                    signal.state = crate::waveform::SigState::Ready;
                }
            }
        }
        self.msg(format!("Loading {path} ..."));
        self.load = Some(LoadJob::start(path, !self.sources_explicit));
    }

    /// Cancel an in-flight background load.
    pub fn cancel_load(&mut self) {
        if let Some(job) = &self.load {
            if !job.finished {
                job.cancel();
                self.msg("Cancelling load ...");
            }
        }
    }

    /// Ask the backend for the value changes of a lazily loaded signal (and
    /// of the array signals that complete together with it).
    pub fn request_signal(&mut self, index: usize) {
        let Some(wf) = self.wf.as_mut() else { return };
        let Some(signal) = wf.signals.get(index) else {
            return;
        };
        if signal.state != crate::waveform::SigState::Lazy {
            return;
        }
        // An aggregate whose members are all loaded (or an eager dump) is
        // computed locally; only missing values need the backend.
        if wf.recompute_aggregate(index) {
            return;
        }
        let sent = self
            .load
            .as_ref()
            .map(|job| job.request(index))
            .unwrap_or(false);
        if sent {
            wf.signals[index].state = crate::waveform::SigState::Loading;
        } else {
            self.msg("waveform backend is no longer available for this dump");
        }
    }

    /// Message for the status line while a background load is running.
    pub fn load_progress_text(&self) -> Option<String> {
        let job = self.load.as_ref().filter(|job| !job.finished)?;
        let progress = job.progress;
        let stage = match progress.stage {
            crate::dump::Stage::Waveform => "signals",
            crate::dump::Stage::Rtl => "RTL files",
        };
        let items = if progress.total > 0 {
            format!("{} {}/{}", stage, progress.done, progress.total)
        } else {
            format!("{stage} ...")
        };
        let changes = if progress.changes > 0 {
            format!(", {} value changes", progress.changes)
        } else {
            String::new()
        };
        Some(format!(
            "loading {}: {items}{changes} — Esc cancels",
            job.path
        ))
    }

    /// Drain background loader events; called from the idle tick.
    pub fn poll_load(&mut self) {
        loop {
            let event = match self.load.as_ref().map(LoadJob::try_recv) {
                Some(Ok(event)) => event,
                Some(Err(std::sync::mpsc::TryRecvError::Empty)) => break,
                Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                    self.load = None;
                    break;
                }
                None => break,
            };
            match event {
                LoadEvent::Progress(progress) => {
                    if let Some(job) = &mut self.load {
                        job.progress = progress;
                    }
                }
                LoadEvent::Waveform(out) => {
                    let path = self
                        .load
                        .as_ref()
                        .map(|job| job.path.clone())
                        .unwrap_or_default();
                    self.apply_waveform(path.clone(), out.wf, out.warnings);
                    set_title(&path);
                }
                LoadEvent::Rtl(set, db) => {
                    self.msg(format!(
                        "RTL parsed: {} module(s) in {} file(s)",
                        db.modules.len(),
                        db.files.len()
                    ));
                    self.sources = Some(*set);
                    self.rtl = Some(*db);
                    self.source_view = None;
                    self.last_source_trace = None;
                    self.sync_source();
                }
                LoadEvent::Changes(updates, warnings) => {
                    self.apply_signal_changes(updates);
                    for warning in warnings {
                        self.msg(format!("  warn: {warning}"));
                    }
                }
                LoadEvent::Failed(err) => {
                    self.load = None;
                    if err == "load cancelled" {
                        self.msg("Load cancelled");
                    } else {
                        self.msg(format!("Error: {err}"));
                        self.dialog = None;
                    }
                    break;
                }
                LoadEvent::Done => {
                    if let Some(job) = &mut self.load {
                        job.finished = true;
                    }
                    break;
                }
            }
        }
    }

    /// Install value changes that arrived from a lazy backend.
    fn apply_signal_changes(&mut self, updates: Vec<(usize, Vec<crate::waveform::Change>)>) {
        let Some(wf) = self.wf.as_mut() else { return };
        for (index, changes) in updates {
            if let Some(signal) = wf.signals.get_mut(index) {
                signal.changes = changes;
                signal.state = crate::waveform::SigState::Ready;
            }
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
        self.browser_mode = BrowserMode::Waveform;
        if self.use_gui {
            if let Some(path) = crate::picker::pick_vcd() {
                self.start_load(&path.display().to_string());
            }
            return;
        }
        self.open_tui_browser();
    }

    /// Open the same file dialog as `Open Waveform`, but load a filelist.
    pub fn open_filelist_dialog(&mut self) {
        self.browser_mode = BrowserMode::Filelist;
        if self.use_gui {
            if let Some(path) = crate::picker::pick_filelist() {
                self.load_filelist(&path.display().to_string());
            }
            return;
        }
        self.open_tui_browser();
    }

    /// Finish a file-dialog / browser pick according to its purpose.
    pub(crate) fn browser_load(&mut self, path: &str) {
        match self.browser_mode {
            BrowserMode::Waveform => {
                self.start_load(path);
            }
            BrowserMode::Filelist => {
                self.load_filelist(path);
                self.dialog = None;
            }
        }
        self.browser_mode = BrowserMode::Waveform;
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

    /// Install a parsed waveform and reset the view state. Array grouping is
    /// done here; the background loader uses [`App::apply_waveform`] instead.
    #[cfg(test)]
    pub fn apply_parsed(&mut self, path: impl Into<String>, out: crate::dump::ParseOut) {
        let mut wf = out.wf;
        // Unpacked arrays are dumped element by element; group them under
        // expandable parent signals. Dump scopes (structs, interfaces) get an
        // aggregate signal each.
        wf.build_arrays();
        wf.build_scope_aggregates();
        self.apply_waveform(path.into(), wf, out.warnings);

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

    /// Install an already grouped waveform and reset the view state.
    pub fn apply_waveform(&mut self, path: String, wf: Waveform, warnings: Vec<String>) {
        self.msg(format!("Loaded {}", wf.summary()));
        for warning in warnings {
            self.msg(format!("  warn: {warning}"));
        }
        self.path = path;
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
        self.list_h_scroll = usize::MAX;
        self.source_h_scroll = 0;
        self.value_h_scroll = 0;
        self.tree_h_scroll = 0;
        self.module_h_scroll = 0;
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
        self.last_source_trace = None;
        self.sync_source();
    }

    /// Reload the Source pane when the selected instance changed.
    pub fn sync_source(&mut self) {
        let module = self.selected_module_name();
        if module == self.source_view.as_ref().map(|view| view.module.clone()) {
            return;
        }
        let loaded = module.and_then(|name| {
            let rtl = self.rtl.as_ref()?;
            let def = rtl.module(&name)?;
            SourceView::load(def, rtl)
        });
        if let Some(view) = &loaded {
            // The header that used to sit above the code is logged instead.
            self.msg(format!(
                "source: {} [module {}] {}",
                self.selected_scope_steps().join("."),
                view.module,
                view.file.display()
            ));
        }
        self.source_view = loaded;
    }

    /// Frame title of the Source pane:
    /// `Source - tb.u_proc.u_cluster(/path/cluster.sv)`.
    pub fn source_title(&self) -> Option<String> {
        let view = self.source_view.as_ref()?;
        let scope = self.selected_scope_steps().join(".");
        Some(format!("Source - {scope}({})", view.file.display()))
    }

    /// Trace line that used to sit below the source code: declaration,
    /// drivers and loads of the selected Signal List signal in the module
    /// under the Source cursor.
    pub fn source_trace_line(&self) -> Option<String> {
        let view = self.source_view.as_ref()?;
        let wf = self.wf.as_ref()?;
        let signal = wf.signals.get(self.selected_signal()?)?.name.clone();
        let module = view.module_at_line(view.line);
        let rtl = self.rtl.as_ref()?;
        let trace = rtl.trace(module, &signal)?;
        let lines = |locations: &[crate::rtl::scan::Location]| {
            locations
                .iter()
                .map(|loc| loc.line.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        let always = rtl
            .module(module)
            .and_then(|def| {
                trace.drivers.iter().find_map(|loc| {
                    def.always
                        .iter()
                        .find(|block| block.start <= loc.line && loc.line <= block.end)
                })
            })
            .map(|block| format!("  always {}-{}", block.start, block.end))
            .unwrap_or_default();
        Some(format!(
            "{signal}: decl {}  drivers [{}]  loads [{}]{always}",
            trace.decl.as_ref().map(|loc| loc.line).unwrap_or(0),
            lines(&trace.drivers),
            lines(&trace.loads)
        ))
    }

    /// Append the source trace to the message log when it changes.
    pub fn sync_source_trace(&mut self) {
        let trace = self.source_trace_line();
        if trace != self.last_source_trace {
            if let Some(trace) = &trace {
                self.msg(format!("source trace: {trace}"));
            }
            self.last_source_trace = trace;
        }
    }

    /// Module of the selected instance: recorded by the dump (FSDB) or, for
    /// VCD/FST, inferred by walking the elaborated RTL design hierarchy.
    /// Tool-generated scopes (`unnamed$$_0`, `$attribute_root`) that the dump
    /// labels with their own name fall back to the enclosing module.
    pub fn selected_module_name(&self) -> Option<String> {
        let rtl = self.rtl.as_ref();
        if let Some(name) = self.selected_scope_module() {
            if rtl.map(|db| db.module(&name).is_some()).unwrap_or(false) {
                return Some(name);
            }
        }
        let steps = self.selected_scope_steps();
        rtl?.module_at_scope(&steps).map(|def| def.name.clone())
    }

    fn source_rows(&self) -> usize {
        // Borders plus the bottom horizontal scrollbar row.
        self.layout().source.height.saturating_sub(3) as usize
    }

    pub fn move_source_cursor(&mut self, delta_line: i64, delta_col: i64) {
        let rows = self.source_rows();
        if let Some(view) = self.source_view.as_mut() {
            view.move_cursor(delta_line, delta_col, rows);
        }
        self.ensure_source_col_visible();
    }

    pub fn set_source_cursor(&mut self, line: usize, col: usize) {
        let rows = self.source_rows();
        if let Some(view) = self.source_view.as_mut() {
            view.set_cursor(line, col, rows);
        }
        self.ensure_source_col_visible();
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
        let Some((word, chain, module, indices, port_line)) =
            self.source_view.as_ref().and_then(|view| {
                let (start, end) = view.word_span_at_cursor()?;
                let word = view.word_at_cursor()?;
                Some((
                    word,
                    view.qualifier_chain(view.line, start),
                    view.module_at_line(view.line).to_string(),
                    view.indices_after(view.line, end),
                    view.dotted_port(view.line, start).then_some(view.line),
                ))
            })
        else {
            self.msg("source: no signal name under the cursor (a: add)");
            return;
        };
        match self.resolve_source_reference(&module, &chain, &word, &indices, port_line) {
            Some(index) => {
                self.add_signal(index);
                self.focus = Focus::Source;
                let switched = self.switch_to_module(&module);
                let suffix = if switched { ", switched hierarchy" } else { "" };
                self.msg(format!(
                    "added {word} from {module} to the Signal List{suffix}"
                ));
            }
            None => self.msg(format!("source: {word} was not dumped in this scope")),
        }
    }

    /// Start a source selection at the cursor (mouse down).
    pub fn begin_source_selection(&mut self) {
        if let Some(view) = self.source_view.as_mut() {
            view.begin_selection();
        }
    }

    /// Extend the source selection vertically (Shift+arrows).
    pub fn extend_source_selection(&mut self, delta: i64) {
        let rows = self.source_rows();
        if let Some(view) = self.source_view.as_mut() {
            view.extend_selection_by(delta, 0, rows);
        }
    }

    /// Extend the source selection horizontally (Shift+Left/Right).
    pub fn extend_source_selection_h(&mut self, delta: i64) {
        let rows = self.source_rows();
        if let Some(view) = self.source_view.as_mut() {
            view.extend_selection_by(0, delta, rows);
        }
        self.ensure_source_col_visible();
    }

    /// Extend the source selection to a concrete character (mouse drag).
    pub fn extend_source_selection_to(&mut self, line: usize, col: usize) {
        if let Some(view) = self.source_view.as_mut() {
            view.extend_selection_to(line, col);
        }
    }

    /// Finish a mouse selection: a plain click picks the word under it.
    pub fn finish_source_selection(&mut self) {
        if let Some(view) = self.source_view.as_mut() {
            view.finish_selection();
        }
    }

    /// Select the identifier under the cursor (double click).
    pub fn select_source_word(&mut self) {
        if let Some(view) = self.source_view.as_mut() {
            view.select_word();
        }
    }

    /// `Ctrl+A` / context menu: select all text of the active module.
    pub fn select_all_source(&mut self) {
        let rows = self.source_rows();
        let Some(module) = self.source_view.as_ref().map(|view| view.module.clone()) else {
            return;
        };
        let range = self
            .rtl
            .as_ref()
            .and_then(|db| db.module(&module))
            .map(|def| (def.start, def.end));
        let Some((start, end)) = range else {
            self.msg("source: no parsed module to select");
            return;
        };
        if let Some(view) = self.source_view.as_mut() {
            if view.select_region(start, end, rows) {
                self.focus = Focus::Source;
            }
        }
    }

    /// Keep the Source cursor inside the horizontally scrolled viewport.
    pub(crate) fn ensure_source_col_visible(&mut self) {
        let Some(view) = &self.source_view else {
            return;
        };
        let l = self.layout();
        let code = crate::ui::source::code_rect(&l);
        let gutter = crate::ui::source::gutter_width(view) as usize;
        let vbar = crate::ui::source::scrollbar_col(&l, view).is_some();
        let text_w = (code.width as usize).saturating_sub(gutter + usize::from(vbar));
        let content = view
            .lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        let max = content.saturating_sub(text_w);
        let col = view.col;
        let h = self.source_h_scroll.min(max);
        let h = if col < h {
            col
        } else if text_w > 0 && col >= h + text_w {
            (col + 1).saturating_sub(text_w)
        } else {
            h
        };
        self.source_h_scroll = h.min(max);
    }

    /// Called on idle ticks: auto-scroll a Source selection that is dragged
    /// beyond the visible code area so more text can be selected.
    pub fn tick_source_drag(&mut self) {
        let Some(drag) = self.dragging else {
            return;
        };
        if drag.mode != DragMode::SourceSel {
            return;
        }
        let rect = crate::ui::source::code_rect(&self.layout());
        let rows = rect.height.saturating_sub(1);
        if rows == 0 {
            return;
        }
        if drag.row < rect.y as usize {
            self.scroll_source(-1);
            let line = self
                .source_view
                .as_ref()
                .map(|view| view.scroll)
                .unwrap_or(0);
            self.extend_source_selection_to(line, 0);
        } else if drag.row >= rect.bottom() as usize {
            self.scroll_source(1);
            let line = self
                .source_view
                .as_ref()
                .map(|view| view.scroll + rows as usize - 1)
                .unwrap_or(0);
            self.extend_source_selection_to(line, usize::MAX);
        }
    }

    /// Dump signals covered by the current Source selection, resolved through
    /// the RTL design AST (deduplicated, in selection order). Also returns the
    /// selected names that could not be resolved to a dumped signal, and the
    /// module of the last resolved one (the hierarchy to switch to).
    pub fn source_selection_signals(&self) -> (Vec<usize>, Vec<String>, Option<String>) {
        let Some(view) = &self.source_view else {
            return (Vec::new(), Vec::new(), None);
        };
        let Some((first, last)) = view.sel.map(|(start, end)| (start.0, end.0)) else {
            return (Vec::new(), Vec::new(), None);
        };
        let mut signals = Vec::new();
        let mut missing = Vec::new();
        let mut last_module = None;
        for line in first..=last {
            let Some((from, to)) = view.selection_interval(line) else {
                continue;
            };
            let Some(spans) = view.spans.get(line) else {
                continue;
            };
            let module = view.module_at_line(line).to_string();
            let mut col = 0usize;
            for span in spans {
                let span_start = col;
                let span_end = col + span.text.chars().count();
                col = span_end;
                if span_end <= from || span_start >= to {
                    continue;
                }
                if span.kind != crate::rtl::view::HlKind::Signal {
                    continue;
                }
                let chain = view.qualifier_chain(line, span_start);
                let indices = view.indices_after(line, span_end);
                let port_line = view.dotted_port(line, span_start).then_some(line);
                match self
                    .resolve_source_reference(&module, &chain, &span.text, &indices, port_line)
                {
                    Some(index) => {
                        if !signals.contains(&index) {
                            signals.push(index);
                        }
                        last_module = Some(module.clone());
                    }
                    None => {
                        if !missing.contains(&span.text) {
                            missing.push(span.text.clone());
                        }
                    }
                }
            }
        }
        (signals, missing, last_module)
    }

    /// `Ctrl+W` / context menu: add every selected source signal at once.
    pub fn add_source_selection(&mut self) {
        let (signals, missing, module) = self.source_selection_signals();
        if signals.is_empty() && missing.is_empty() {
            self.msg("source: select signal names first (drag or Shift+arrows)");
            return;
        }
        let mut added = 0usize;
        for index in signals {
            self.add_signal(index);
            added += 1;
        }
        self.focus = Focus::Source;
        if !missing.is_empty() {
            self.msg(format!(
                "source: not dumped in this scope: {}",
                missing.join(", ")
            ));
        }
        if added > 0 {
            self.msg(format!("added {added} signal(s) from the source selection"));
        }
        if let Some(module) = module {
            if self.switch_to_module(&module) {
                self.msg(format!(
                    "source: switched hierarchy to the {module} instance"
                ));
            }
        }
    }

    /// Instance scope of the selected tree node (without the design root).
    fn selected_scope_steps(&self) -> Vec<String> {
        self.selected_scope_path()
            .map(|path| path.split('.').skip(1).map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Resolve a Source reference, first trying `.port` selections as named
    /// port connections of the child instance on that line (`.valid_i` of
    /// `u_cluster` -> `...u_cluster.valid_i`).
    fn resolve_source_reference(
        &self,
        module: &str,
        chain: &[String],
        name: &str,
        indices: &[String],
        port_line: Option<usize>,
    ) -> Option<usize> {
        if chain.is_empty() {
            if let Some(line) = port_line {
                if let Some(instance) = self.port_instance(module, line, name) {
                    if let Some(found) =
                        self.resolve_source_signal(module, &[instance], name, indices)
                    {
                        return Some(found);
                    }
                }
            }
        }
        self.resolve_source_signal(module, chain, name, indices)
    }

    /// Instance of `module` whose named port connection on `line` (0-based
    /// view line) is `port`.
    fn port_instance(&self, module: &str, line: usize, port: &str) -> Option<String> {
        let rtl = self.rtl.as_ref()?;
        let def = rtl.module(module)?;
        def.instances
            .iter()
            .find(|inst| {
                inst.ports
                    .iter()
                    .any(|(name, at)| name == port && *at == line + 1)
            })
            .map(|inst| inst.name.clone())
    }

    /// Resolve a signal reference from the Source pane through the RTL AST.
    ///
    /// The qualifier chain (`u_dut.count`) is walked through the instances of
    /// the module, and the plain name must be declared there. The dump is then
    /// matched by *exact* scope, so equally named signals in different
    /// hierarchies never collide. FSDB stores ranges (`count[7:0]`), so base
    /// names are compared as well.
    ///
    /// When the selected scope is a `generate` block (`...genblk1[1]`), the
    /// module's own signals live on the enclosing instance
    /// (`...u_proc.clk`), and genvars bound by the block (e.g. `i = 1`)
    /// resolve indexes like `cluster_valid[i]` or `data_chain[k]`.
    ///
    /// Code from another module in the same file belongs to another instance:
    /// its scope is found in the elaborated design instead of the active one.
    fn resolve_source_signal(
        &self,
        module: &str,
        chain: &[String],
        name: &str,
        indices: &[String],
    ) -> Option<usize> {
        let Some(rtl) = self.rtl.as_ref() else {
            // No AST at all: trust the dump at the exact scope.
            return chain
                .is_empty()
                .then(|| self.find_signal_exact(&self.selected_scope_steps(), name))
                .flatten();
        };
        let active = self
            .source_view
            .as_ref()
            .map(|view| view.module.as_str())
            .unwrap_or_default();
        let (base, bindings) = if module == active {
            match rtl.scope_info(&self.selected_scope_steps()) {
                Some(found) => (found.instance_scope, found.env),
                None => (self.selected_scope_steps(), Default::default()),
            }
        } else {
            (self.foreign_scope(module)?, Default::default())
        };
        let (path, signal) = rtl.resolve_reference_with(module, chain, name, &bindings)?;
        let base_scope = base.clone();
        let mut scope = base;
        scope.extend(path.iter().cloned());
        // `arr[i][j]` first, then drop dimensions (`arr[i]`, `arr`) so both
        // whole arrays and their elements can be added.
        let mut candidates: Vec<String> = Vec::new();
        let mut current = signal.clone();
        for text in indices {
            let Some(value) = crate::rtl::scan::parse_expr_text(text)
                .and_then(|expr| crate::rtl::scan::eval(&expr, &bindings))
            else {
                break;
            };
            current = format!("{current}[{value}]");
            candidates.push(current.clone());
        }
        candidates.reverse();
        for candidate in candidates {
            if let Some(index) = self.find_signal_exact(&scope, &candidate) {
                return Some(index);
            }
        }
        if let Some(index) = self.find_signal_exact(&scope, &signal) {
            return Some(index);
        }
        // A whole struct variable, interface or instance is added as the
        // aggregate of its dump scope (`{member, ...}`, expandable).
        let mut aggregate_scope = scope.clone();
        if !signal.is_empty() {
            aggregate_scope.push(signal.clone());
        }
        let aggregate = self
            .wf
            .as_ref()
            .and_then(|wf| wf.scope_aggregate(&aggregate_scope));
        if aggregate.is_some() {
            return aggregate;
        }
        // `sig.field` of a packed struct dumped as one vector: the head.
        if path.len() == 1 {
            if let Some(index) = self.find_signal_exact(&base_scope, &path[0]) {
                return Some(index);
            }
        }
        None
    }

    /// Instance scope of `module` to use when adding a signal from code that
    /// belongs to another module of the same file. Prefers instances below the
    /// active scope, then parents, then siblings in the same top.
    fn foreign_scope(&self, module: &str) -> Option<Vec<String>> {
        let rtl = self.rtl.as_ref()?;
        let active = self.selected_scope_steps();
        let mut best: Option<(u32, Vec<String>)> = None;
        for (path, name) in rtl.placements() {
            if name != module {
                continue;
            }
            let score = if active.is_empty() || path.starts_with(&active) {
                (path.len() - active.len()) as u32
            } else if active.starts_with(&path) {
                (active.len() - path.len()) as u32 + 100
            } else if path.first() == active.first() {
                path.len() as u32 + 200
            } else {
                continue;
            };
            if best.as_ref().map(|(best, _)| score < *best).unwrap_or(true) {
                best = Some((score, path));
            }
        }
        best.map(|(_, path)| path)
    }

    /// Select the instance of `module` in the Instance pane when the added
    /// signal came from an inactive module; the Source pane follows.
    fn switch_to_module(&mut self, module: &str) -> bool {
        if self.source_view.as_ref().map(|view| view.module.as_str()) == Some(module) {
            return false;
        }
        let Some(scope) = self.foreign_scope(module) else {
            return false;
        };
        self.switch_scope(&scope)
    }

    /// Move the Instance pane selection to a dump scope, expanding ancestors.
    fn switch_scope(&mut self, scope: &[String]) -> bool {
        let (node, ancestors) = {
            let Some(wf) = self.wf.as_ref() else {
                return false;
            };
            let mut node = wf.tree.root;
            let mut ancestors = Vec::new();
            for step in scope {
                let Some(child) = wf.tree.nodes[node]
                    .children
                    .iter()
                    .copied()
                    .find(|child| wf.tree.nodes[*child].name == *step)
                else {
                    return false;
                };
                ancestors.push(node);
                node = child;
            }
            (node, ancestors)
        };
        self.expanded.extend(ancestors);
        let Some(index) = self
            .tree_visible()
            .iter()
            .position(|entry| matches!(entry, TreeNode::Scope { id, .. } if *id == node))
        else {
            return false;
        };
        self.tree_sel = index;
        self.tree_scroll_to_sel();
        self.sync_source();
        true
    }

    /// Double-click on a waveform: select the instance that owns the signal
    /// and move the Source cursor to the logic that drives it - its drivers in
    /// its own module, the parent's port connection for input ports, or its
    /// declaration as a last resort.
    pub fn jump_to_driver(&mut self, sig: usize) {
        let Some(signal) = self.wf.as_ref().and_then(|wf| wf.signals.get(sig)).cloned() else {
            return;
        };
        // A bit chunk or element is driven by the bus it came from.
        let mut base_sig = sig;
        while let Some(parent) = self
            .wf
            .as_ref()
            .and_then(|wf| wf.signals.get(base_sig))
            .and_then(|signal| signal.parent)
        {
            base_sig = parent;
        }
        let name = self
            .wf
            .as_ref()
            .and_then(|wf| wf.signals.get(base_sig))
            .map(|signal| {
                signal
                    .name
                    .split('[')
                    .next()
                    .unwrap_or(&signal.name)
                    .to_string()
            })
            .unwrap_or_default();
        let scope = signal.scope.clone();

        let mut module = String::new();
        let mut drivers: Option<(Vec<String>, usize)> = None;
        let mut decl: Option<(Vec<String>, usize)> = None;
        if let Some(rtl) = self.rtl.as_ref() {
            if let Some(def) = rtl.module_at_scope(&scope) {
                module = def.name.clone();
                if let Some(trace) = rtl.trace(&module, &name) {
                    if let Some(location) = trace.drivers.first() {
                        drivers = Some((scope.clone(), location.line));
                    }
                    if let Some(location) = trace.decl.as_ref() {
                        decl = Some((scope.clone(), location.line));
                    }
                }
            }
        }
        // An input port is driven by the parent's port connection.
        let mut port_driver: Option<(Vec<String>, usize)> = None;
        if !scope.is_empty() {
            let parent = &scope[..scope.len() - 1];
            let instance = scope.last().cloned().unwrap_or_default();
            if let Some(rtl) = self.rtl.as_ref() {
                if let Some(def) = rtl.module_at_scope(parent) {
                    if let Some(inst) = def.instances.iter().find(|inst| inst.name == instance) {
                        if let Some((_, line)) = inst.ports.iter().find(|(port, _)| {
                            port == &name || crate::rtl::scan::signal_name_matches(port, &name)
                        }) {
                            module = def.name.clone();
                            port_driver = Some((parent.to_vec(), *line));
                        }
                    }
                }
            }
        }

        let Some((scope, line)) = drivers.or(port_driver).or(decl) else {
            self.msg(format!("no driver found for {name}"));
            return;
        };
        if !self.switch_scope(&scope) {
            self.msg(format!("no driver found for {name}"));
            return;
        }
        self.set_source_cursor(line.saturating_sub(1), 0);
        self.focus = Focus::Source;
        self.msg(format!("{name}: driver at {module}:{line}"));
    }

    /// Find a dumped signal in exactly this scope (no descendant or global
    /// fallbacks), comparing the plain and the range-stripped name.
    fn find_signal_exact(&self, scope: &[String], name: &str) -> Option<usize> {
        let wf = self.wf.as_ref()?;
        wf.signals.iter().position(|sig| {
            sig.scope.as_slice() == scope && crate::rtl::scan::signal_name_matches(&sig.name, name)
        })
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
            self.apply_radix(idx, next);
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

    /// Set the radix of a signal; array signals pass it on to their elements.
    pub(crate) fn apply_radix(&mut self, idx: usize, radix: Radix) {
        let mut stack = vec![idx];
        while let Some(node) = stack.pop() {
            self.radix.insert(node, radix);
            let children: Vec<usize> = self
                .wf
                .as_ref()
                .map(|wf| wf.children(node))
                .unwrap_or_default();
            stack.extend(children);
        }
        self.refresh_arrays();
    }

    /// Re-format the brace values of array signals with the current radixes.
    fn refresh_arrays(&mut self) {
        let radix = std::mem::take(&mut self.radix);
        if let Some(wf) = self.wf.as_mut() {
            wf.rebuild_array_texts(&radix);
        }
        self.radix = radix;
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

/// Idle tick from the event loop: time-based interactions keep running while
/// no input events arrive (e.g. auto-scrolling a dragged Source selection).
pub fn tick(app: &mut App) {
    app.tick_source_drag();
    app.poll_load();
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
    fn filelist_dialog_uses_the_browser() {
        let dir =
            std::env::temp_dir().join(format!("waverdi_app_fl_dialog_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("top.sv");
        std::fs::write(&src, "module top; endmodule\n").unwrap();
        let list = dir.join("files.f");
        std::fs::write(&list, "top.sv\n").unwrap();

        let mut app = App::new();
        app.use_gui = false;
        crate::app::Action::LoadFilelist.run(&mut app);
        assert_eq!(app.dialog, Some(Dialog::Open));
        assert!(app.browser.is_some());
        app.browser_load(&list.display().to_string());
        assert_eq!(app.dialog, None);
        assert!(app.sources_explicit);
        assert_eq!(app.sources.as_ref().unwrap().files, vec![src]);
        // The mode was reset: the next pick loads a waveform again.
        assert!(matches!(app.browser_mode, BrowserMode::Waveform));
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

    #[test]
    fn adding_a_scope_aggregate_shows_brace_values() {
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $scope module dut $end\n\
            $var wire 1 ! run $end\n\
            $var wire 4 \" pc [3:0] $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\n0!\nb0101 \"\n";
        let out = crate::vcd::parse_bytes(vcd.as_bytes()).unwrap();
        let mut app = App::new();
        app.apply_parsed("<test>", out);
        app.sync_layout(Rect::new(0, 0, 100, 40));
        let aggregate = app
            .wf
            .as_ref()
            .and_then(|wf| wf.scope_aggregate(&["tb".to_string(), "dut".to_string()]))
            .expect("scope aggregate");
        assert_eq!(
            app.wf.as_ref().unwrap().signals[aggregate].var_type,
            "aggregate"
        );
        app.add_signal(aggregate);
        let signal = &app.wf.as_ref().unwrap().signals[aggregate];
        assert_eq!(signal.state, crate::waveform::SigState::Ready);
        assert!(
            signal.display_value(0, Radix::Hex).starts_with('{'),
            "{}",
            signal.display_value(0, Radix::Hex)
        );
        // The members can still be added on their own.
        app.add_signal(aggregate);
        let children = app.wf.as_ref().unwrap().children(aggregate);
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn background_load_applies_the_waveform() {
        let dir = std::env::temp_dir().join(format!("waverdi_app_load_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let vcd = dir.join("wave.vcd");
        std::fs::write(
            &vcd,
            "$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n#10\n1!\n",
        )
        .unwrap();

        let mut app = App::new();
        app.start_load(&vcd.display().to_string());
        assert!(app.load.is_some());
        for _ in 0..500 {
            app.poll_load();
            if app.wf.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let wf = app.wf.as_ref().expect("waveform loaded");
        assert_eq!(wf.signals.len(), 1);
        assert_eq!(app.path, vcd.display().to_string());
        assert!(
            app.load.as_ref().is_some_and(|job| job.finished),
            "loader marked finished after Done"
        );
    }
}
