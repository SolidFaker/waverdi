use super::{keymap, App, Dialog};
use crate::settings::{
    GeneralSetting, GeneralSettings, KeyChord, SettingId, SettingsCategory, ThemeKind,
    BUS_BITS_OPTIONS, BUS_WORK_OPTIONS,
};
use crate::ui::widgets::{Dropdown, Orientation, Slider};
use crate::waveform::TimeBase;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsPane {
    Categories,
    #[default]
    Items,
    Buttons,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsButton {
    #[default]
    Reset,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingRow {
    Setting(SettingId),
    KeyBinding(usize),
}

pub struct SettingRowView {
    pub label: String,
    pub value: String,
    pub color: Option<Color>,
    pub custom: bool,
}

impl App {
    pub fn open_settings(&mut self) {
        self.settings_sel = 0;
        self.settings_pane = SettingsPane::Items;
        self.settings_button = SettingsButton::Reset;
        self.time_dropdown.selected = TimeBase::CYCLE
            .iter()
            .position(|base| *base == self.settings.time_base)
            .unwrap_or(0);
        self.time_dropdown.open = false;
        self.time_dropdown_anchor = 0;
        self.open_dialog(Dialog::Settings);
    }

    pub fn settings_len(&self) -> usize {
        match self.settings_category {
            SettingsCategory::KeyBindings => keymap::bindings().len(),
            category => self.settings.len(category),
        }
    }

    pub fn settings_row(&self) -> Option<SettingRow> {
        self.settings_row_at(self.settings_sel)
    }

    pub fn settings_row_at(&self, row: usize) -> Option<SettingRow> {
        match self.settings_category {
            SettingsCategory::KeyBindings => {
                (row < keymap::bindings().len()).then_some(SettingRow::KeyBinding(row))
            }
            category => self.settings.row(category, row).map(SettingRow::Setting),
        }
    }

    pub fn settings_row_view(&self, row: SettingRow) -> SettingRowView {
        match row {
            SettingRow::KeyBinding(index) => {
                let binding = &keymap::bindings()[index];
                SettingRowView {
                    label: binding.description.to_string(),
                    value: keymap::effective_keys(index, &self.settings.keys),
                    color: None,
                    custom: self.settings.keys.get(index).is_some(),
                }
            }
            SettingRow::Setting(SettingId::General(setting)) => {
                let (value, custom) = match setting {
                    GeneralSetting::TimeBase => (
                        self.time_base_label(),
                        self.settings.time_base != TimeBase::Scale,
                    ),
                    _ => {
                        let defaults = GeneralSettings::default();
                        (
                            setting.value(&self.settings.general),
                            setting.value(&self.settings.general) != setting.value(&defaults),
                        )
                    }
                };
                SettingRowView {
                    label: setting.label().to_string(),
                    value,
                    color: None,
                    custom,
                }
            }
            SettingRow::Setting(SettingId::Theme) => SettingRowView {
                label: "theme".to_string(),
                value: self.settings.theme_kind.name().to_string(),
                color: None,
                custom: self.settings.theme_kind != ThemeKind::Dark,
            },
            SettingRow::Setting(SettingId::Ui(setting)) => {
                let color = setting.get(&self.settings.theme);
                SettingRowView {
                    label: setting.label().to_string(),
                    value: crate::settings::theme::color_name(color),
                    color: Some(color),
                    custom: color != setting.get(&self.settings.theme_kind.theme()),
                }
            }
            SettingRow::Setting(SettingId::Wave(setting)) => {
                let color = setting.get(&self.settings.theme);
                SettingRowView {
                    label: setting.label().to_string(),
                    value: crate::settings::theme::color_name(color),
                    color: Some(color),
                    custom: color != setting.get(&self.settings.theme_kind.theme()),
                }
            }
        }
    }

    pub fn settings_move(&mut self, delta: i64) {
        if self.settings_pane == SettingsPane::Categories {
            self.set_settings_category(self.settings_category.next(delta));
            return;
        }
        if self.settings_pane == SettingsPane::Buttons {
            self.settings_button = match self.settings_button {
                SettingsButton::Reset => SettingsButton::Close,
                SettingsButton::Close => SettingsButton::Reset,
            };
            return;
        }
        let len = self.settings_len();
        if len == 0 {
            self.settings_sel = 0;
            return;
        }
        self.settings_sel = (self.settings_sel as i64 + delta).rem_euclid(len as i64) as usize;
        self.reveal_settings_row();
    }

    pub fn settings_change(&mut self, delta: i64) {
        if self.settings_pane == SettingsPane::Buttons {
            self.settings_button = match self.settings_button {
                SettingsButton::Reset => SettingsButton::Close,
                SettingsButton::Close => SettingsButton::Reset,
            };
            return;
        }
        if self.settings_pane == SettingsPane::Categories {
            if delta > 0 {
                self.settings_pane = SettingsPane::Items;
                self.reveal_settings_row();
            }
            return;
        }
        if let Some(SettingRow::Setting(setting)) = self.settings_row() {
            self.cycle_setting(setting, delta);
        }
    }

    pub fn settings_activate(&mut self) {
        if self.settings_pane == SettingsPane::Buttons {
            match self.settings_button {
                SettingsButton::Reset => self.settings_reset(),
                SettingsButton::Close => self.dialog = None,
            }
            return;
        }
        if self.settings_pane == SettingsPane::Categories {
            self.settings_pane = SettingsPane::Items;
            self.reveal_settings_row();
            return;
        }
        match self.settings_row() {
            Some(SettingRow::KeyBinding(index)) => {
                self.key_capture = Some(index);
                self.dialog = Some(Dialog::BindKey);
            }
            Some(SettingRow::Setting(SettingId::General(GeneralSetting::TimeBase))) => {
                self.time_dropdown_anchor = self.settings_sel;
                self.time_dropdown.open = !self.time_dropdown.open;
            }
            Some(SettingRow::Setting(setting @ (SettingId::Ui(_) | SettingId::Wave(_)))) => {
                self.open_color_picker(setting)
            }
            Some(SettingRow::Setting(setting)) => self.cycle_setting(setting, 1),
            None => {}
        }
    }

    pub fn settings_toggle_pane(&mut self) {
        self.settings_pane = match self.settings_pane {
            SettingsPane::Categories => SettingsPane::Items,
            SettingsPane::Items => SettingsPane::Buttons,
            SettingsPane::Buttons => SettingsPane::Categories,
        };
        self.time_dropdown.open = false;
        self.reveal_settings_row();
    }

    pub fn settings_reset(&mut self) {
        match self.settings_row() {
            Some(SettingRow::Setting(setting)) => self.reset_setting(setting),
            Some(SettingRow::KeyBinding(index)) if self.settings.keys.reset(index) => {
                self.msg(format!(
                    "shortcut reset: {}",
                    keymap::bindings()[index].description
                ));
            }
            Some(SettingRow::KeyBinding(_)) | None => {}
        }
    }

    pub fn settings_reset_all(&mut self) {
        let count = self.settings.keys.len();
        if self.settings_category == SettingsCategory::KeyBindings && self.settings.keys.reset_all()
        {
            self.msg(format!("restored {count} shortcut(s) to defaults"));
        }
    }

    pub fn set_settings_category(&mut self, category: SettingsCategory) {
        self.settings_category = category;
        self.settings_sel = 0;
        self.dialog_scroll = 0;
        self.time_dropdown.open = false;
        self.reveal_settings_row();
    }

    pub fn select_settings_category(&mut self, index: usize) {
        if let Some(category) = SettingsCategory::ALL.get(index).copied() {
            self.settings_pane = SettingsPane::Categories;
            self.set_settings_category(category);
        }
    }

    pub fn select_settings_row(&mut self, row: usize) {
        if row < self.settings_len() {
            self.settings_pane = SettingsPane::Items;
            self.settings_sel = row;
            self.reveal_settings_row();
        }
    }

    pub fn reveal_settings_row(&mut self) {
        let rows = crate::ui::dialog::visible_rows(self.last_area, self, Dialog::Settings).max(1);
        let max_scroll = self.settings_len().saturating_sub(rows);
        if self.settings_sel < self.dialog_scroll {
            self.dialog_scroll = self.settings_sel;
        } else if self.settings_sel >= self.dialog_scroll + rows {
            self.dialog_scroll = self.settings_sel + 1 - rows;
        }
        self.dialog_scroll = self.dialog_scroll.min(max_scroll);
    }

    pub fn general_slider(&self, setting: GeneralSetting, area: Rect) -> Option<Slider> {
        let (index, count) = match setting {
            GeneralSetting::BusSplitBits => (
                BUS_BITS_OPTIONS
                    .iter()
                    .position(|value| *value == self.settings.general.bus_split_bits)
                    .unwrap_or(0),
                BUS_BITS_OPTIONS.len(),
            ),
            GeneralSetting::BusSplitWork => (
                BUS_WORK_OPTIONS
                    .iter()
                    .position(|value| *value == self.settings.general.bus_split_work)
                    .unwrap_or(0),
                BUS_WORK_OPTIONS.len(),
            ),
            _ => return None,
        };
        Some(Slider::new(
            index,
            0,
            count - 1,
            Orientation::Horizontal,
            area,
        ))
    }

    pub fn set_general_value(&mut self, setting: GeneralSetting, value: usize) {
        match setting {
            GeneralSetting::BusSplitBits => {
                if let Some(value) = BUS_BITS_OPTIONS.get(value) {
                    self.settings.general.bus_split_bits = *value;
                }
            }
            GeneralSetting::BusSplitWork => {
                if let Some(value) = BUS_WORK_OPTIONS.get(value) {
                    self.settings.general.bus_split_work = *value;
                }
            }
            GeneralSetting::WaveOverflow => {
                self.settings.general.wave_overflow = if value == 0 {
                    crate::settings::WaveValueOverflow::Hide
                } else {
                    crate::settings::WaveValueOverflow::Ellipsize
                };
            }
            GeneralSetting::FullNames => {
                self.settings.general.full_names = value != 0;
                self.show_full_names = self.settings.general.full_names;
            }
            GeneralSetting::TimeBase => {
                if let Some(base) = TimeBase::CYCLE.get(value).copied() {
                    self.set_time_base(base);
                }
            }
        }
    }

    pub fn time_dropdown_key(&mut self, key: KeyEvent) -> bool {
        let bounds = crate::ui::dialog::settings_popup_bounds(self.last_area, self);
        let anchor = crate::ui::dialog::time_dropdown_anchor(self.last_area, self);
        let options: Vec<&str> = TimeBase::CYCLE.iter().map(|base| base.label()).collect();
        let dropdown = Dropdown::new(&options, self.time_dropdown, anchor);
        let changed = dropdown.handle_key(&mut self.time_dropdown, key, bounds);
        if changed {
            let base = TimeBase::CYCLE[self.time_dropdown.selected];
            self.set_time_base(base);
        }
        changed
    }

    pub fn open_color_picker(&mut self, setting: SettingId) {
        let color = match setting {
            SettingId::Ui(setting) => setting.get(&self.settings.theme),
            SettingId::Wave(setting) => setting.get(&self.settings.theme),
            _ => return,
        };
        let Some(picker) = crate::ui::widgets::ColorPickerState::new(color) else {
            self.msg("this colour cannot be edited");
            return;
        };
        self.time_dropdown.open = false;
        self.color_picker = Some(picker);
        self.color_setting = Some(setting);
        self.dialog = Some(Dialog::ColorPicker);
    }

    pub fn color_picker_key(&mut self, key: KeyEvent) {
        let action = self
            .color_picker
            .as_mut()
            .and_then(|picker| picker.handle_key(key));
        self.finish_color_picker(action);
    }

    pub fn color_picker_mouse(&mut self, area: Rect, event: crossterm::event::MouseEvent) {
        let action = self
            .color_picker
            .as_mut()
            .and_then(|picker| picker.mouse(area, event));
        self.finish_color_picker(action);
    }

    pub fn close_color_picker(&mut self) {
        self.color_picker = None;
        self.color_setting = None;
        self.dialog = Some(Dialog::Settings);
    }

    fn finish_color_picker(&mut self, action: Option<crate::ui::widgets::ColorPickerAction>) {
        use crate::ui::widgets::ColorPickerAction;

        match action {
            Some(ColorPickerAction::Accept(color)) => {
                if let Some(setting) = self.color_setting {
                    self.settings.set_color(setting, color);
                }
                self.close_color_picker();
            }
            Some(ColorPickerAction::Cancel) => self.close_color_picker(),
            Some(ColorPickerAction::InvalidHex) => self.msg("invalid colour: expected #RRGGBB"),
            None => {}
        }
    }

    pub fn capture_key(&mut self, key: KeyEvent) {
        let Some(index) = self.key_capture else {
            self.dialog = Some(Dialog::Settings);
            return;
        };
        match key.code {
            KeyCode::Esc => self.dialog = Some(Dialog::Settings),
            KeyCode::Backspace | KeyCode::Delete => {
                self.settings.keys.reset(index);
                self.dialog = Some(Dialog::Settings);
                self.msg(format!(
                    "shortcut reset: {}",
                    keymap::bindings()[index].description
                ));
            }
            _ => {
                let chord = KeyChord::from_event(key);
                match keymap::validate_chord(index, chord, &self.settings.keys) {
                    Ok(()) => {
                        self.settings.keys.set(index, chord);
                        self.dialog = Some(Dialog::Settings);
                        self.msg(format!(
                            "shortcut: {} = {}",
                            keymap::bindings()[index].description,
                            chord.label()
                        ));
                    }
                    Err(error) => self.msg(error),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::app_with;
    use crate::settings::{ThemeKind, WaveValueOverflow};
    use crossterm::event::KeyModifiers;

    const VCD: &str = "$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n";

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn categories_panes_and_general_settings_work_together() {
        let mut app = app_with(VCD);
        app.open_settings();
        assert_eq!(app.settings_category, SettingsCategory::General);
        assert_eq!(app.settings_len(), 5);
        app.settings_change(1);
        assert_eq!(app.settings.general.bus_split_bits, 8_192);
        app.settings_move(1);
        app.settings_change(1);
        assert_eq!(app.settings.general.bus_split_work, 8_000_000);
        app.settings_move(1);
        app.settings_change(1);
        assert_eq!(
            app.settings.general.wave_overflow,
            WaveValueOverflow::Ellipsize
        );
        for _ in 0..3 {
            app.settings_move(-1);
            app.settings_reset();
        }
        app.settings_toggle_pane();
        assert_eq!(app.settings_pane, SettingsPane::Buttons);
        app.settings_toggle_pane();
        assert_eq!(app.settings_pane, SettingsPane::Categories);
        app.settings_move(1);
        assert_eq!(app.settings_category, SettingsCategory::Theme);
        app.settings_activate();
        assert_eq!(app.settings_pane, SettingsPane::Items);
        app.settings_change(1);
        assert_eq!(app.settings.theme_kind, ThemeKind::Light);
    }

    #[test]
    fn buttons_support_keyboard_focus_reset_and_close() {
        let mut app = app_with(VCD);
        app.settings.general.bus_split_bits = 8_192;
        app.open_settings();
        app.settings_toggle_pane();
        assert_eq!(app.settings_pane, SettingsPane::Buttons);
        assert_eq!(app.settings_button, SettingsButton::Reset);
        app.settings_activate();
        assert_eq!(app.settings.general.bus_split_bits, 4_096);
        app.settings_change(1);
        assert_eq!(app.settings_button, SettingsButton::Close);
        app.settings_activate();
        assert_eq!(app.dialog, None);
    }

    #[test]
    fn key_capture_applies_rejects_and_restores_bindings() {
        let mut app = app_with(VCD);
        app.set_settings_category(SettingsCategory::KeyBindings);
        let zoom = keymap::bindings()
            .iter()
            .position(|binding| binding.action == super::super::Action::ZoomIn)
            .unwrap();
        app.select_settings_row(zoom);
        app.settings_activate();
        assert_eq!(app.dialog, Some(Dialog::BindKey));
        app.capture_key(key(KeyCode::F(5)));
        assert_eq!(app.dialog, Some(Dialog::Settings));
        assert!(app.settings.keys.get(zoom).is_some());
        assert_eq!(
            keymap::resolve(key(KeyCode::F(5)), true, &app.settings.keys),
            Some(super::super::Action::ZoomIn)
        );
        assert_eq!(
            keymap::resolve(key(KeyCode::Char('z')), true, &app.settings.keys),
            None
        );

        app.select_settings_row(zoom + 1);
        app.settings_activate();
        app.capture_key(key(KeyCode::F(5)));
        assert_eq!(app.dialog, Some(Dialog::BindKey));
        assert!(app.settings.keys.get(zoom + 1).is_none());
        app.capture_key(key(KeyCode::Esc));
        assert_eq!(app.dialog, Some(Dialog::Settings));
        app.select_settings_row(zoom);
        app.settings_activate();
        app.capture_key(key(KeyCode::Backspace));
        assert!(app.settings.keys.is_empty());
        app.settings
            .keys
            .set(zoom, KeyChord::from_event(key(KeyCode::F(5))));
        app.settings_reset_all();
        assert!(app.settings.keys.is_empty());
    }

    #[test]
    fn general_checkbox_and_dropdown_update_settings() {
        let mut app = app_with(VCD);
        app.select_settings_row(3);
        app.settings_activate();
        assert!(app.settings.general.full_names);
        assert!(app.show_full_names);

        app.select_settings_row(4);
        app.settings_activate();
        assert!(app.time_dropdown.open);
        app.time_dropdown_key(key(KeyCode::Down));
        assert_eq!(app.settings.time_base, TimeBase::CYCLE[1]);
        app.time_dropdown_key(key(KeyCode::Enter));
        assert!(!app.time_dropdown.open);
    }

    #[test]
    fn dropdown_closes_when_category_changes() {
        let mut app = app_with(VCD);
        app.select_settings_row(4);
        app.settings_activate();
        assert!(app.time_dropdown.open);
        app.set_settings_category(SettingsCategory::KeyBindings);
        assert!(!app.time_dropdown.open);
    }

    #[test]
    fn selected_settings_row_is_revealed_on_small_screens() {
        let mut app = app_with(VCD);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 60, 12));
        app.set_settings_category(SettingsCategory::KeyBindings);
        app.select_settings_row(keymap::bindings().len() - 1);
        let rows = crate::ui::dialog::visible_rows(app.last_area, &app, Dialog::Settings);
        assert!(app.dialog_scroll > 0);
        assert!(app.settings_sel >= app.dialog_scroll);
        assert!(app.settings_sel < app.dialog_scroll + rows);
    }
}
