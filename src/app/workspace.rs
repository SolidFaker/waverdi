use super::{App, Group, Marker};
use crate::app::keymap;
use crate::settings::{GeneralSettings, KeyChord, Splits, ThemeKind, UiSetting, WaveSetting};
use crate::waveform::{Ticks, TimeBase};
use crossterm::event::{KeyCode, KeyModifiers};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Workspace {
    version: u32,
    path: Option<String>,
    filelists: Vec<String>,
    recent_files: Vec<String>,
    settings: WorkspaceSettings,
    view: Option<WorkspaceView>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct WorkspaceSettings {
    general: GeneralSettings,
    splits: Splits,
    time_base: TimeBase,
    theme_kind: ThemeKind,
    colors: BTreeMap<String, String>,
    keys: BTreeMap<usize, String>,
    search_case_sensitive: bool,
    search_regex: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WorkspaceView {
    path: String,
    display: Vec<WorkspaceSignal>,
    groups: Vec<Group>,
    cursor: Ticks,
    t0: f64,
    scale: f64,
    markers: Vec<Marker>,
    sel_row: Option<usize>,
    tree_sel: usize,
    row_scroll: usize,
}

impl Default for WorkspaceView {
    fn default() -> Self {
        Self {
            path: String::new(),
            display: Vec::new(),
            groups: Vec::new(),
            cursor: 0,
            t0: 0.0,
            scale: 1.0,
            markers: Vec::new(),
            sel_row: None,
            tree_sel: 0,
            row_scroll: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorkspaceSignal {
    scope: Vec<String>,
    name: String,
}

impl App {
    pub fn load_workspace(&mut self, resume: bool) {
        self.load_workspace_from(&workspace_path(), resume);
    }

    fn load_workspace_from(&mut self, path: &Path, resume: bool) {
        let Some(workspace) = read_workspace(path) else {
            return;
        };
        self.apply_workspace_settings(workspace.settings);
        self.recent_files = workspace.recent_files;
        self.pending_workspace = workspace.view;
        if !workspace.filelists.is_empty() {
            let files = workspace.filelists.into_iter().map(PathBuf::from).collect();
            self.sources = Some(crate::rtl::SourceSet::from_files(files, "workspace"));
            self.sources_explicit = true;
            self.rebuild_rtl();
        }
        if resume {
            if let Some(path) = workspace
                .path
                .or_else(|| self.recent_files.first().cloned())
            {
                self.start_load(&path);
            }
        }
    }

    pub fn save_workspace(&mut self) {
        if let Err(error) = write_default_workspace(self) {
            self.msg(format!("workspace: {error}"));
        }
    }

    pub(crate) fn remember_file(&mut self, path: &str) {
        if path.is_empty() {
            return;
        }
        self.recent_files.retain(|entry| entry != path);
        self.recent_files.insert(0, path.to_string());
        self.recent_files.truncate(10);
    }

    pub(crate) fn restore_workspace_view(&mut self) {
        let Some(view) = self.pending_workspace.take() else {
            return;
        };
        if view.path != self.path {
            return;
        }
        let Some(wf) = self.wf.as_ref() else {
            return;
        };
        let mut display = Vec::with_capacity(view.display.len());
        for wanted in view.display {
            if let Some(index) = wf
                .signals
                .iter()
                .position(|signal| signal.scope == wanted.scope && signal.name == wanted.name)
            {
                display.push(index);
            }
        }
        self.display = display;
        self.groups = if view.groups.is_empty()
            || view.groups.iter().map(|group| group.count).sum::<usize>() != self.display.len()
        {
            let mut group = Group::new(0);
            group.count = self.display.len();
            vec![group]
        } else {
            view.groups
        };
        self.cursor = view.cursor.clamp(wf.start, wf.end);
        self.t0 = view.t0;
        self.scale = if view.scale.is_finite() && view.scale > 0.0 {
            view.scale
        } else {
            1.0
        };
        self.markers = view
            .markers
            .into_iter()
            .filter(|marker| marker.time >= wf.start && marker.time <= wf.end)
            .collect();
        self.next_marker_id = self
            .markers
            .iter()
            .map(|marker| marker.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.marker_sel = self.marker_sel.min(self.markers.len().saturating_sub(1));
        self.sel_row = view.sel_row;
        self.tree_sel = view.tree_sel;
        self.row_scroll = view.row_scroll;
        self.touch_panes();
        self.clamp_row_scroll();
        self.clamp_tree_scroll();
        self.clamp_view();
        for index in self.display.clone() {
            self.request_signal(index);
        }
    }

    fn apply_workspace_settings(&mut self, settings: WorkspaceSettings) {
        self.settings.general = settings.general;
        self.show_full_names = self.settings.general.full_names;
        self.settings.splits = settings.splits;
        self.settings.time_base = settings.time_base;
        self.time_dropdown.selected = TimeBase::CYCLE
            .iter()
            .position(|base| *base == settings.time_base)
            .unwrap_or(0);
        self.settings.set_theme_kind(settings.theme_kind);
        for (label, text) in settings.colors {
            let Some(color) = parse_color(&text) else {
                continue;
            };
            if let Some(setting) = UiSetting::ALL
                .iter()
                .find(|setting| setting.label() == label)
            {
                self.settings
                    .set_color(crate::settings::SettingId::Ui(*setting), color);
            } else if let Some(setting) = WaveSetting::ALL
                .iter()
                .find(|setting| setting.label() == label)
            {
                self.settings
                    .set_color(crate::settings::SettingId::Wave(*setting), color);
            }
        }
        for (index, label) in settings.keys {
            if index < keymap::bindings().len() {
                if let Some(chord) = parse_chord(&label) {
                    self.settings.keys.set(index, chord);
                }
            }
        }
        self.search_case_sensitive = settings.search_case_sensitive;
        self.search_regex = settings.search_regex;
    }
}

fn read_workspace(path: &Path) -> Option<Workspace> {
    let text = std::fs::read_to_string(path).ok()?;
    match toml::from_str(&text) {
        Ok(workspace) => Some(workspace),
        Err(error) => {
            eprintln!("workspace: cannot parse {}: {error}", path.display());
            None
        }
    }
}

fn write_default_workspace(app: &App) -> Result<(), String> {
    write_workspace(&workspace_path(), app)
}

fn write_workspace(path: &Path, app: &App) -> Result<(), String> {
    let workspace = Workspace {
        version: 1,
        path: (!app.path.is_empty()).then(|| app.path.clone()),
        filelists: if app.sources_explicit {
            app.sources
                .as_ref()
                .map(|sources| {
                    sources
                        .files
                        .iter()
                        .map(|file| file.display().to_string())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        },
        recent_files: app.recent_files.clone(),
        settings: WorkspaceSettings {
            general: app.settings.general.clone(),
            splits: app.settings.splits,
            time_base: app.settings.time_base,
            theme_kind: app.settings.theme_kind,
            colors: workspace_colors(app),
            keys: app
                .settings
                .keys
                .entries()
                .map(|(index, chord)| (index, chord.label()))
                .collect(),
            search_case_sensitive: app.search_case_sensitive,
            search_regex: app.search_regex,
        },
        view: app.wf.as_ref().map(|wf| WorkspaceView {
            path: app.path.clone(),
            display: app
                .display
                .iter()
                .map(|&index| {
                    let signal = &wf.signals[index];
                    WorkspaceSignal {
                        scope: signal.scope.clone(),
                        name: signal.name.clone(),
                    }
                })
                .collect(),
            groups: app.groups.clone(),
            cursor: app.cursor,
            t0: app.t0,
            scale: app.scale,
            markers: app.markers.clone(),
            sel_row: app.sel_row,
            tree_sel: app.tree_sel,
            row_scroll: app.row_scroll,
        }),
    };
    let text = toml::to_string_pretty(&workspace)
        .map_err(|error| format!("cannot serialize {}: {error}", path.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("workspace has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, text)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))
}

fn workspace_colors(app: &App) -> BTreeMap<String, String> {
    let mut colors = BTreeMap::new();
    for setting in UiSetting::ALL {
        colors.insert(
            setting.label().to_string(),
            crate::settings::theme::color_name(setting.get(&app.settings.theme)),
        );
    }
    for setting in WaveSetting::ALL {
        colors.insert(
            setting.label().to_string(),
            crate::settings::theme::color_name(setting.get(&app.settings.theme)),
        );
    }
    colors
}

fn workspace_path() -> PathBuf {
    if let Some(path) = std::env::var_os("WAVERDI_WORKSPACE") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("APPDATA") {
        return PathBuf::from(path).join("waverdi/workspace.toml");
    }
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("waverdi/workspace.toml");
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".config/waverdi/workspace.toml")
}

fn parse_color(text: &str) -> Option<ratatui::style::Color> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some(ratatui::style::Color::Rgb(
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ))
}

fn parse_chord(label: &str) -> Option<KeyChord> {
    let mut modifiers = KeyModifiers::empty();
    let mut rest = label;
    while let Some((prefix, modifier)) = [
        ("Ctrl+", KeyModifiers::CONTROL),
        ("Shift+", KeyModifiers::SHIFT),
        ("Alt+", KeyModifiers::ALT),
    ]
    .into_iter()
    .find(|(prefix, _)| rest.starts_with(prefix))
    {
        modifiers |= modifier;
        rest = &rest[prefix.len()..];
    }
    let code = match rest {
        "Space" => KeyCode::Char(' '),
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "BackTab" => KeyCode::BackTab,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Insert" => KeyCode::Insert,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "PgUp" => KeyCode::PageUp,
        "PgDn" => KeyCode::PageDown,
        _ => {
            if let Some(number) = rest.strip_prefix('F') {
                KeyCode::F(number.parse().ok()?)
            } else if rest.chars().count() == 1 {
                KeyCode::Char(rest.chars().next()?)
            } else {
                return None;
            }
        }
    };
    Some(KeyChord { code, modifiers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::app_with;

    const VCD: &str =
        "$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n#10\n1!\n";

    #[test]
    fn workspace_roundtrip_restores_settings_view_and_markers() {
        let dir = std::env::temp_dir().join(format!("waverdi_workspace_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("workspace.toml");
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        app.settings.general.full_names = true;
        app.search_regex = true;
        app.search_case_sensitive = true;
        app.settings.keys.set(
            0,
            KeyChord {
                code: KeyCode::F(6),
                modifiers: KeyModifiers::CONTROL,
            },
        );
        app.cursor = 10;
        app.add_marker();
        write_workspace(&path, &app).unwrap();
        let workspace: Workspace =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(workspace.path.as_deref(), Some("<test>"));
        assert_eq!(
            workspace.settings.keys.get(&0).map(String::as_str),
            Some("Ctrl+F6")
        );

        let mut restored = app_with(VCD);
        restored.apply_workspace_settings(workspace.settings);
        restored.pending_workspace = workspace.view;
        restored.restore_workspace_view();
        assert!(restored.settings.general.full_names);
        assert!(restored.search_regex);
        assert!(restored.search_case_sensitive);
        assert_eq!(restored.display, vec![0]);
        assert_eq!(restored.markers.len(), 1);
        assert_eq!(restored.markers[0].time, 10);
        assert_eq!(restored.settings.keys.get(0).unwrap().label(), "Ctrl+F6");
        assert!(!path.with_extension("tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resume_loads_the_values_of_restored_signals() {
        let dir =
            std::env::temp_dir().join(format!("waverdi_workspace_resume_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let vcd = dir.join("wave.vcd");
        let workspace = dir.join("workspace.toml");
        std::fs::write(&vcd, VCD).unwrap();

        let mut saved = App::new();
        saved.start_load(&vcd.display().to_string());
        for _ in 0..500 {
            saved.poll_load();
            if saved.wf.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(saved.wf.is_some(), "initial waveform did not load");
        saved.set_display(vec![0]);
        saved.cursor = 10;
        saved.add_marker();
        write_workspace(&workspace, &saved).unwrap();
        drop(saved);

        let mut restored = App::new();
        restored.load_workspace_from(&workspace, true);
        assert!(restored.load.is_some(), "resume did not start a load");
        let mut loaded = false;
        for _ in 0..500 {
            restored.poll_load();
            loaded = restored.wf.as_ref().is_some_and(|wf| {
                wf.signals[0].state == crate::waveform::SigState::Ready
                    && !wf.signals[0].changes.is_empty()
            });
            if loaded {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            loaded,
            "restored signal did not load: {:?}",
            restored.messages
        );
        assert_eq!(restored.path, vcd.display().to_string());
        assert_eq!(restored.display, vec![0]);
        assert_eq!(restored.cursor, 10);
        assert_eq!(restored.markers.len(), 1);
        let wf = restored.wf.as_ref().unwrap();
        assert_eq!(wf.signals[0].changes.len(), 2);
        assert_eq!(wf.signals[0].changes[1].t, 10);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn chord_parser_handles_modifiers_and_special_keys() {
        let chord = parse_chord("Ctrl+Shift+F5").unwrap();
        assert_eq!(chord.code, KeyCode::F(5));
        assert!(chord.modifiers.contains(KeyModifiers::CONTROL));
        assert!(chord.modifiers.contains(KeyModifiers::SHIFT));
        assert_eq!(parse_chord("Alt+Space").unwrap().code, KeyCode::Char(' '));
        assert_eq!(parse_chord("PgDn").unwrap().code, KeyCode::PageDown);
    }
}
