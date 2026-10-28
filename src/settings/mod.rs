mod general;
mod keymap;
mod splits;
pub mod theme;

pub use general::{
    GeneralSetting, GeneralSettings, WaveValueOverflow, BUS_BITS_OPTIONS, BUS_WORK_OPTIONS,
};
pub use keymap::{KeyChord, KeymapSettings};
pub use splits::Splits;
pub use theme::{Theme, ThemeKind, UiSetting, WaveSetting};

use crate::waveform::TimeBase;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsCategory {
    #[default]
    General,
    Theme,
    UiColours,
    WaveformColours,
    KeyBindings,
}

impl SettingsCategory {
    pub const ALL: [SettingsCategory; 5] = [
        SettingsCategory::General,
        SettingsCategory::Theme,
        SettingsCategory::UiColours,
        SettingsCategory::WaveformColours,
        SettingsCategory::KeyBindings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Theme => "Theme",
            Self::UiColours => "UI Colours",
            Self::WaveformColours => "Waveform Colours",
            Self::KeyBindings => "Key Bindings",
        }
    }

    pub fn next(self, delta: i64) -> Self {
        let index = Self::ALL.iter().position(|item| *item == self).unwrap_or(0) as i64;
        Self::ALL[(index + delta).rem_euclid(Self::ALL.len() as i64) as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingId {
    General(GeneralSetting),
    Theme,
    Ui(UiSetting),
    Wave(WaveSetting),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub general: GeneralSettings,
    pub keys: KeymapSettings,
    pub theme: Theme,
    pub theme_kind: ThemeKind,
    pub splits: Splits,
    pub time_base: TimeBase,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            general: GeneralSettings::default(),
            keys: KeymapSettings::default(),
            theme: Theme::DARK,
            theme_kind: ThemeKind::Dark,
            splits: Splits::default(),
            time_base: TimeBase::Scale,
        }
    }
}

impl Settings {
    pub fn len(&self, category: SettingsCategory) -> usize {
        match category {
            SettingsCategory::General => GeneralSetting::ALL.len(),
            SettingsCategory::Theme => 1,
            SettingsCategory::UiColours => UiSetting::ALL.len(),
            SettingsCategory::WaveformColours => WaveSetting::ALL.len(),
            SettingsCategory::KeyBindings => 0,
        }
    }

    pub fn row(&self, category: SettingsCategory, row: usize) -> Option<SettingId> {
        match category {
            SettingsCategory::General => GeneralSetting::ALL
                .get(row)
                .copied()
                .map(SettingId::General),
            SettingsCategory::Theme => (row == 0).then_some(SettingId::Theme),
            SettingsCategory::UiColours => UiSetting::ALL.get(row).copied().map(SettingId::Ui),
            SettingsCategory::WaveformColours => {
                WaveSetting::ALL.get(row).copied().map(SettingId::Wave)
            }
            SettingsCategory::KeyBindings => None,
        }
    }

    pub fn cycle(&mut self, setting: SettingId, delta: i64) {
        match setting {
            SettingId::General(GeneralSetting::TimeBase) => {
                let count = TimeBase::CYCLE.len() as i64;
                let index = TimeBase::CYCLE
                    .iter()
                    .position(|base| *base == self.time_base)
                    .unwrap_or(0) as i64;
                self.time_base = TimeBase::CYCLE[(index + delta).rem_euclid(count) as usize];
            }
            SettingId::General(GeneralSetting::TopSplit) => {
                self.splits.top_pct = (self.splits.top_pct as i64 + delta * 5)
                    .clamp(Splits::MIN_TOP_PCT as i64, Splits::MAX_TOP_PCT as i64)
                    as u16;
            }
            SettingId::General(setting) => setting.cycle(&mut self.general, delta),
            SettingId::Theme => {
                let count = ThemeKind::ALL.len() as i64;
                let index = ThemeKind::ALL
                    .iter()
                    .position(|kind| *kind == self.theme_kind)
                    .unwrap_or(0) as i64;
                let next = (index + delta).rem_euclid(count) as usize;
                self.set_theme_kind(ThemeKind::ALL[next]);
            }
            SettingId::Ui(setting) => {
                let color = theme::cycle_color(setting.get(&self.theme), delta);
                setting.set(&mut self.theme, color);
            }
            SettingId::Wave(setting) => {
                let color = theme::cycle_color(setting.get(&self.theme), delta);
                setting.set(&mut self.theme, color);
            }
        }
    }

    pub fn reset(&mut self, setting: SettingId) {
        let base = self.theme_kind.theme();
        match setting {
            SettingId::General(GeneralSetting::TimeBase) => self.time_base = TimeBase::Scale,
            SettingId::General(GeneralSetting::TopSplit) => {
                self.splits.top_pct = Splits::default().top_pct
            }
            SettingId::General(setting) => setting.reset(&mut self.general),
            SettingId::Theme => {}
            SettingId::Ui(setting) => setting.set(&mut self.theme, setting.get(&base)),
            SettingId::Wave(setting) => setting.set(&mut self.theme, setting.get(&base)),
        }
    }

    pub fn set_theme_kind(&mut self, kind: ThemeKind) {
        self.theme_kind = kind;
        self.theme = kind.theme();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_split_by_category() {
        let settings = Settings::default();
        assert_eq!(settings.len(SettingsCategory::General), 6);
        assert_eq!(settings.len(SettingsCategory::Theme), 1);
        assert_eq!(settings.len(SettingsCategory::UiColours), 1);
        assert_eq!(settings.len(SettingsCategory::WaveformColours), 10);
        assert_eq!(
            settings.row(SettingsCategory::Theme, 0),
            Some(SettingId::Theme)
        );
        assert_eq!(settings.row(SettingsCategory::Theme, 1), None);
        assert_eq!(settings.row(SettingsCategory::KeyBindings, 0), None);
        assert_eq!(
            SettingsCategory::General.next(-1),
            SettingsCategory::KeyBindings
        );
    }

    #[test]
    fn cycling_and_resetting_colours_uses_the_active_theme() {
        let mut settings = Settings::default();
        settings.cycle(SettingId::Theme, 1);
        assert_eq!(settings.theme_kind, ThemeKind::Light);
        assert_eq!(settings.theme, Theme::LIGHT);

        settings.cycle(SettingId::Ui(UiSetting::Background), 1);
        settings.cycle(SettingId::Wave(WaveSetting::High), 1);
        assert_ne!(settings.theme.bg, Theme::LIGHT.bg);
        assert_ne!(settings.theme.high, Theme::LIGHT.high);

        settings.reset(SettingId::Ui(UiSetting::Background));
        assert_eq!(settings.theme.bg, Theme::LIGHT.bg);
        assert_ne!(settings.theme.high, Theme::LIGHT.high);
        settings.reset(SettingId::Wave(WaveSetting::High));
        assert_eq!(settings.theme.high, Theme::LIGHT.high);

        settings.cycle(SettingId::Wave(WaveSetting::High), 1);
        settings.cycle(SettingId::Theme, 1);
        assert_eq!(settings.theme_kind, ThemeKind::Mixed);
        assert_eq!(settings.theme, Theme::MIXED);
        settings.cycle(SettingId::Theme, 1);
        assert_eq!(settings.theme_kind, ThemeKind::Dark);
        settings.cycle(SettingId::Theme, -1);
        assert_eq!(settings.theme_kind, ThemeKind::Mixed);
    }
}
