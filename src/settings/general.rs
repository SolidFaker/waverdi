pub const BUS_BITS_OPTIONS: [u32; 5] = [1_024, 2_048, 4_096, 8_192, 16_384];
pub const BUS_WORK_OPTIONS: [usize; 5] = [1_000_000, 4_000_000, 8_000_000, 16_000_000, 64_000_000];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaveValueOverflow {
    #[default]
    Hide,
    Ellipsize,
}

impl WaveValueOverflow {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hide => "hide",
            Self::Ellipsize => "ellipsis",
        }
    }

    fn cycle(self, delta: i64) -> Self {
        const ALL: [WaveValueOverflow; 2] = [WaveValueOverflow::Hide, WaveValueOverflow::Ellipsize];
        let index = ALL.iter().position(|value| *value == self).unwrap_or(0) as i64;
        ALL[(index + delta).rem_euclid(ALL.len() as i64) as usize]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneralSettings {
    pub bus_split_bits: u32,
    pub bus_split_work: usize,
    pub wave_overflow: WaveValueOverflow,
    pub full_names: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            bus_split_bits: 4_096,
            bus_split_work: 4_000_000,
            wave_overflow: WaveValueOverflow::Hide,
            full_names: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneralSetting {
    BusSplitBits,
    BusSplitWork,
    WaveOverflow,
    FullNames,
    TimeBase,
    TopSplit,
}

impl GeneralSetting {
    pub const ALL: [GeneralSetting; 6] = [
        GeneralSetting::BusSplitBits,
        GeneralSetting::BusSplitWork,
        GeneralSetting::WaveOverflow,
        GeneralSetting::FullNames,
        GeneralSetting::TimeBase,
        GeneralSetting::TopSplit,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::BusSplitBits => "bus split max bits",
            Self::BusSplitWork => "bus split work limit",
            Self::WaveOverflow => "wave label ellipsis",
            Self::FullNames => "full hierarchical names",
            Self::TimeBase => "time base",
            Self::TopSplit => "top pane ratio",
        }
    }

    pub fn value(self, settings: &GeneralSettings) -> String {
        match self {
            Self::BusSplitBits => settings.bus_split_bits.to_string(),
            Self::BusSplitWork => format_work(settings.bus_split_work),
            Self::WaveOverflow => settings.wave_overflow.label().to_string(),
            Self::FullNames => settings.full_names.to_string(),
            Self::TimeBase | Self::TopSplit => String::new(),
        }
    }

    pub fn cycle(self, settings: &mut GeneralSettings, delta: i64) {
        match self {
            Self::BusSplitBits => {
                settings.bus_split_bits =
                    cycle_value(settings.bus_split_bits, &BUS_BITS_OPTIONS, delta)
            }
            Self::BusSplitWork => {
                settings.bus_split_work =
                    cycle_value(settings.bus_split_work, &BUS_WORK_OPTIONS, delta)
            }
            Self::WaveOverflow => {
                settings.wave_overflow = settings.wave_overflow.cycle(delta);
            }
            Self::FullNames => settings.full_names = !settings.full_names,
            Self::TimeBase | Self::TopSplit => {}
        }
    }

    pub fn reset(self, settings: &mut GeneralSettings) {
        let defaults = GeneralSettings::default();
        match self {
            Self::BusSplitBits => settings.bus_split_bits = defaults.bus_split_bits,
            Self::BusSplitWork => settings.bus_split_work = defaults.bus_split_work,
            Self::WaveOverflow => settings.wave_overflow = defaults.wave_overflow,
            Self::FullNames => settings.full_names = defaults.full_names,
            Self::TimeBase | Self::TopSplit => {}
        }
    }
}

fn cycle_value<T: Copy + PartialEq>(current: T, values: &[T], delta: i64) -> T {
    let index = values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0) as i64;
    values[(index + delta).rem_euclid(values.len() as i64) as usize]
}

fn format_work(value: usize) -> String {
    if value.is_multiple_of(1_000_000) {
        format!("{}M", value / 1_000_000)
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_defaults_preserve_existing_limits_and_overflow() {
        let settings = GeneralSettings::default();
        assert_eq!(settings.bus_split_bits, 4_096);
        assert_eq!(settings.bus_split_work, 4_000_000);
        assert_eq!(settings.wave_overflow, WaveValueOverflow::Hide);
        assert!(!settings.full_names);
    }

    #[test]
    fn general_settings_cycle_and_reset() {
        let mut settings = GeneralSettings::default();
        GeneralSetting::BusSplitBits.cycle(&mut settings, 1);
        assert_eq!(settings.bus_split_bits, 8_192);
        GeneralSetting::BusSplitWork.cycle(&mut settings, -1);
        assert_eq!(settings.bus_split_work, 1_000_000);
        GeneralSetting::WaveOverflow.cycle(&mut settings, 1);
        assert_eq!(settings.wave_overflow, WaveValueOverflow::Ellipsize);
        GeneralSetting::FullNames.cycle(&mut settings, 1);
        assert!(settings.full_names);
        for setting in GeneralSetting::ALL {
            setting.reset(&mut settings);
        }
        assert_eq!(settings, GeneralSettings::default());
    }
}
