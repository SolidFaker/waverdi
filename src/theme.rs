use ratatui::style::Color;

/// Selectable colour scheme.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeKind {
    Dark,
    Light,
    /// Dark waveform panes with light chrome.
    Mixed,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 3] = [ThemeKind::Dark, ThemeKind::Light, ThemeKind::Mixed];

    pub fn name(self) -> &'static str {
        match self {
            ThemeKind::Dark => "Dark",
            ThemeKind::Light => "Light",
            ThemeKind::Mixed => "Mixed",
        }
    }

    pub fn theme(self) -> Theme {
        match self {
            ThemeKind::Dark => Theme::DARK,
            ThemeKind::Light => Theme::LIGHT,
            ThemeKind::Mixed => Theme::MIXED,
        }
    }
}

/// All colours used by the UI. Copy so it can be handed to draw functions.
#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Color,
    /// Waveform pane background (mixed theme uses a dark waveform).
    pub wave_bg: Color,
    pub wave_alt: Color,
    /// Text / dim / selection colours used inside the waveform pane.
    pub wave_text: Color,
    pub wave_dim: Color,
    pub wave_sel_bg: Color,
    pub wave_multi_bg: Color,
    pub menubar_bg: Color,
    pub toolbar_bg: Color,
    pub accent: Color,
    pub menu_active: Color,
    pub panel_border: Color,
    pub popup_bg: Color,
    pub overlay: Color,
    pub row_alt: Color,
    pub row_sel_bg: Color,
    pub multi_sel_bg: Color,
    pub range_bg: Color,
    pub list_header_bg: Color,
    pub status_bg: Color,
    pub msg_bg: Color,
    pub text: Color,
    pub name: Color,
    pub value: Color,
    pub scope: Color,
    pub path: Color,
    pub high: Color,
    pub low: Color,
    pub xcol: Color,
    pub zcol: Color,
    pub bus: Color,
    pub bus_text: Color,
    pub cursor: Color,
    pub analog: Color,
    pub tick: Color,
    pub dim: Color,
    pub green_dim: Color,
    pub input_bg: Color,
}

impl Theme {
    pub const DARK: Theme = Theme {
        bg: Color::Rgb(12, 12, 16),
        wave_bg: Color::Rgb(12, 12, 16),
        wave_alt: Color::Rgb(16, 16, 22),
        wave_text: Color::White,
        wave_dim: Color::Rgb(90, 90, 100),
        wave_sel_bg: Color::Rgb(40, 46, 70),
        wave_multi_bg: Color::Rgb(74, 58, 26),
        menubar_bg: Color::Rgb(25, 25, 40),
        toolbar_bg: Color::Rgb(20, 20, 28),
        accent: Color::Rgb(255, 200, 60),
        menu_active: Color::Rgb(60, 80, 160),
        panel_border: Color::Rgb(70, 70, 90),
        popup_bg: Color::Rgb(30, 30, 40),
        overlay: Color::Rgb(15, 15, 20),
        row_alt: Color::Rgb(16, 16, 22),
        row_sel_bg: Color::Rgb(40, 46, 70),
        multi_sel_bg: Color::Rgb(74, 58, 26),
        range_bg: Color::Rgb(26, 40, 60),
        list_header_bg: Color::Rgb(24, 24, 34),
        status_bg: Color::Rgb(22, 22, 30),
        msg_bg: Color::Rgb(18, 18, 24),
        text: Color::White,
        name: Color::Rgb(190, 190, 200),
        value: Color::Rgb(220, 220, 230),
        scope: Color::Yellow,
        path: Color::Cyan,
        high: Color::Rgb(80, 220, 130),
        low: Color::Rgb(150, 150, 150),
        xcol: Color::Rgb(255, 90, 90),
        zcol: Color::Rgb(110, 170, 255),
        bus: Color::Rgb(215, 175, 75),
        bus_text: Color::Rgb(230, 230, 240),
        cursor: Color::Rgb(255, 220, 80),
        analog: Color::Rgb(80, 200, 255),
        tick: Color::Rgb(140, 140, 160),
        dim: Color::Rgb(90, 90, 100),
        green_dim: Color::Rgb(120, 180, 140),
        input_bg: Color::Rgb(30, 30, 40),
    };

    pub const LIGHT: Theme = Theme {
        bg: Color::Rgb(248, 248, 250),
        wave_bg: Color::Rgb(248, 248, 250),
        wave_alt: Color::Rgb(238, 240, 246),
        wave_text: Color::Rgb(20, 20, 25),
        wave_dim: Color::Rgb(135, 135, 145),
        wave_sel_bg: Color::Rgb(198, 214, 240),
        wave_multi_bg: Color::Rgb(246, 224, 170),
        menubar_bg: Color::Rgb(222, 226, 235),
        toolbar_bg: Color::Rgb(232, 234, 240),
        accent: Color::Rgb(170, 90, 0),
        menu_active: Color::Rgb(170, 195, 240),
        panel_border: Color::Rgb(140, 145, 155),
        popup_bg: Color::Rgb(250, 250, 252),
        overlay: Color::Rgb(238, 238, 242),
        row_alt: Color::Rgb(238, 240, 246),
        row_sel_bg: Color::Rgb(198, 214, 240),
        multi_sel_bg: Color::Rgb(246, 224, 170),
        range_bg: Color::Rgb(214, 228, 246),
        list_header_bg: Color::Rgb(228, 230, 238),
        status_bg: Color::Rgb(230, 232, 238),
        msg_bg: Color::Rgb(240, 240, 246),
        text: Color::Rgb(20, 20, 25),
        name: Color::Rgb(55, 55, 65),
        value: Color::Rgb(30, 30, 40),
        scope: Color::Rgb(150, 95, 0),
        path: Color::Rgb(0, 110, 150),
        high: Color::Rgb(0, 130, 60),
        low: Color::Rgb(90, 90, 90),
        xcol: Color::Rgb(200, 30, 30),
        zcol: Color::Rgb(30, 90, 200),
        bus: Color::Rgb(150, 110, 0),
        bus_text: Color::Rgb(40, 40, 45),
        cursor: Color::Rgb(200, 130, 0),
        analog: Color::Rgb(0, 120, 190),
        tick: Color::Rgb(115, 115, 125),
        dim: Color::Rgb(135, 135, 145),
        green_dim: Color::Rgb(60, 140, 90),
        input_bg: Color::White,
    };

    /// Mixed: light chrome (menus, lists, dialogs), dark waveform panes.
    pub const MIXED: Theme = Theme {
        // Waveform pane only: its own dark background and colours.
        wave_bg: Theme::DARK.bg,
        wave_alt: Theme::DARK.row_alt,
        wave_text: Theme::DARK.text,
        wave_dim: Theme::DARK.dim,
        wave_sel_bg: Theme::DARK.row_sel_bg,
        wave_multi_bg: Theme::DARK.multi_sel_bg,
        high: Theme::DARK.high,
        low: Theme::DARK.low,
        xcol: Theme::DARK.xcol,
        zcol: Theme::DARK.zcol,
        bus: Theme::DARK.bus,
        bus_text: Theme::DARK.bus_text,
        cursor: Theme::DARK.cursor,
        analog: Theme::DARK.analog,
        tick: Theme::DARK.tick,
        range_bg: Theme::DARK.range_bg,
        // Everything else (list, dialogs, menus, ...) stays light.
        ..Theme::LIGHT
    };
}

/// Waveform colours that the settings dialog can customise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WaveSetting {
    Background,
    High,
    Low,
    Unknown,
    HighZ,
    Bus,
    BusText,
    Cursor,
    Analog,
    Ticks,
}

impl WaveSetting {
    pub const ALL: [WaveSetting; 10] = [
        WaveSetting::Background,
        WaveSetting::High,
        WaveSetting::Low,
        WaveSetting::Unknown,
        WaveSetting::HighZ,
        WaveSetting::Bus,
        WaveSetting::BusText,
        WaveSetting::Cursor,
        WaveSetting::Analog,
        WaveSetting::Ticks,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WaveSetting::Background => "background",
            WaveSetting::High => "high level",
            WaveSetting::Low => "low level",
            WaveSetting::Unknown => "unknown (x)",
            WaveSetting::HighZ => "high-Z (z)",
            WaveSetting::Bus => "bus value",
            WaveSetting::BusText => "bus text",
            WaveSetting::Cursor => "cursor",
            WaveSetting::Analog => "analog",
            WaveSetting::Ticks => "time ticks",
        }
    }

    pub fn get(self, theme: &Theme) -> Color {
        match self {
            WaveSetting::Background => theme.wave_bg,
            WaveSetting::High => theme.high,
            WaveSetting::Low => theme.low,
            WaveSetting::Unknown => theme.xcol,
            WaveSetting::HighZ => theme.zcol,
            WaveSetting::Bus => theme.bus,
            WaveSetting::BusText => theme.bus_text,
            WaveSetting::Cursor => theme.cursor,
            WaveSetting::Analog => theme.analog,
            WaveSetting::Ticks => theme.tick,
        }
    }

    pub fn set(self, theme: &mut Theme, color: Color) {
        match self {
            WaveSetting::Background => theme.wave_bg = color,
            WaveSetting::High => theme.high = color,
            WaveSetting::Low => theme.low = color,
            WaveSetting::Unknown => theme.xcol = color,
            WaveSetting::HighZ => theme.zcol = color,
            WaveSetting::Bus => theme.bus = color,
            WaveSetting::BusText => theme.bus_text = color,
            WaveSetting::Cursor => theme.cursor = color,
            WaveSetting::Analog => theme.analog = color,
            WaveSetting::Ticks => theme.tick = color,
        }
    }
}

/// Colours cycled through by the settings dialog.
pub const PALETTE: [Color; 14] = [
    Color::Rgb(80, 220, 130),
    Color::Rgb(150, 150, 150),
    Color::Rgb(255, 90, 90),
    Color::Rgb(110, 170, 255),
    Color::Rgb(215, 175, 75),
    Color::Rgb(255, 200, 60),
    Color::Rgb(80, 200, 255),
    Color::Rgb(255, 130, 200),
    Color::Rgb(170, 120, 255),
    Color::White,
    Color::Rgb(200, 200, 200),
    Color::Rgb(90, 90, 100),
    Color::Rgb(30, 30, 40),
    Color::Rgb(12, 12, 16),
];

/// Human readable name of a palette colour (for the settings dialog).
pub fn color_name(color: Color) -> String {
    match color {
        Color::White => "white".to_string(),
        Color::Black => "black".to_string(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        other => format!("{other:?}").to_lowercase(),
    }
}
