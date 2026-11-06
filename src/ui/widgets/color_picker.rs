use super::{Alignment, Button, Label, Orientation, Slider, TextBox};
use crate::app::InputState;
use crate::settings::theme::Theme;
use crate::ui::text;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Style};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorField {
    #[default]
    SaturationValue,
    Hue,
    Red,
    Green,
    Blue,
    Hex,
    Ok,
    Cancel,
}

impl ColorField {
    const ALL: [Self; 8] = [
        Self::SaturationValue,
        Self::Hue,
        Self::Red,
        Self::Green,
        Self::Blue,
        Self::Hex,
        Self::Ok,
        Self::Cancel,
    ];

    fn next(self, delta: i64) -> Self {
        let index = Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0) as i64;
        Self::ALL[(index + delta).rem_euclid(Self::ALL.len() as i64) as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorPickerAction {
    Accept(Color),
    Cancel,
    InvalidHex,
}

#[derive(Clone)]
pub struct ColorPickerState {
    rgb: (u8, u8, u8),
    hue: f64,
    saturation: f64,
    value: f64,
    focus: ColorField,
    hex: InputState,
    drag: Option<ColorField>,
}

impl ColorPickerState {
    pub fn new(color: Color) -> Option<Self> {
        let rgb = color_to_rgb(color)?;
        let (hue, saturation, value) = rgb_to_hsv(rgb);
        let mut hex = InputState::default();
        hex.set(&hex_text(rgb));
        Some(Self {
            rgb,
            hue,
            saturation,
            value,
            focus: ColorField::SaturationValue,
            hex,
            drag: None,
        })
    }

    pub fn color(&self) -> Color {
        Color::Rgb(self.rgb.0, self.rgb.1, self.rgb.2)
    }

    pub fn selected_color(&self) -> Option<Color> {
        parse_hex(&self.hex.as_string())
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<ColorPickerAction> {
        match key.code {
            KeyCode::Esc => return Some(ColorPickerAction::Cancel),
            KeyCode::Tab => {
                self.focus = self.focus.next(1);
                return None;
            }
            KeyCode::BackTab => {
                self.focus = self.focus.next(-1);
                return None;
            }
            KeyCode::Enter => {
                return if self.focus == ColorField::Cancel {
                    Some(ColorPickerAction::Cancel)
                } else {
                    self.selected_color()
                        .map(ColorPickerAction::Accept)
                        .or(Some(ColorPickerAction::InvalidHex))
                };
            }
            _ => {}
        }
        if self.focus == ColorField::Hex {
            let text = self.hex.as_string();
            let limit = if text.starts_with('#') { 7 } else { 6 };
            let accepted = match key.code {
                KeyCode::Char('#') => text.is_empty(),
                KeyCode::Char(c) => c.is_ascii_hexdigit() && text.chars().count() < limit,
                KeyCode::Backspace
                | KeyCode::Delete
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End => true,
                _ => false,
            };
            if accepted {
                TextBox::handle_key(&mut self.hex, key);
                self.apply_hex();
            }
            return None;
        }
        match self.focus {
            ColorField::SaturationValue => {
                let step = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    0.1
                } else {
                    0.02
                };
                match key.code {
                    KeyCode::Left => self.saturation -= step,
                    KeyCode::Right => self.saturation += step,
                    KeyCode::Up => self.value += step,
                    KeyCode::Down => self.value -= step,
                    KeyCode::Home => self.saturation = 0.0,
                    KeyCode::End => self.saturation = 1.0,
                    _ => return None,
                }
                self.saturation = self.saturation.clamp(0.0, 1.0);
                self.value = self.value.clamp(0.0, 1.0);
                self.apply_hsv();
            }
            ColorField::Hue => {
                let step = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    15
                } else {
                    5
                };
                let slider = Slider::new(
                    self.hue.round() as usize,
                    0,
                    359,
                    Orientation::Vertical,
                    Rect::default(),
                )
                .step(step);
                if let Some(hue) = slider.handle_key(key) {
                    self.hue = hue as f64;
                    self.apply_hsv();
                }
            }
            ColorField::Red | ColorField::Green | ColorField::Blue => {
                let index = match self.focus {
                    ColorField::Red => 0,
                    ColorField::Green => 1,
                    ColorField::Blue => 2,
                    _ => unreachable!(),
                };
                let step = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    16
                } else {
                    1
                };
                let slider = Slider::new(
                    self.rgb_component(index) as usize,
                    0,
                    255,
                    Orientation::Horizontal,
                    Rect::default(),
                )
                .step(step);
                if let Some(value) = slider.handle_key(key) {
                    self.set_channel(index, value as u8);
                }
            }
            ColorField::Ok | ColorField::Cancel => match key.code {
                KeyCode::Left | KeyCode::Up => self.focus = self.focus.next(-1),
                KeyCode::Right | KeyCode::Down => self.focus = self.focus.next(1),
                _ => {}
            },
            ColorField::Hex => unreachable!(),
        }
        None
    }

    pub fn mouse(&mut self, area: Rect, event: MouseEvent) -> Option<ColorPickerAction> {
        if event.kind == MouseEventKind::Up(MouseButton::Left) {
            self.drag = None;
            return None;
        }
        let layout = ColorPicker::layout(area)?;
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if crate::ui::layout::pt_in(layout.ok, event.column, event.row) {
                    self.focus = ColorField::Ok;
                    return self
                        .selected_color()
                        .map(ColorPickerAction::Accept)
                        .or(Some(ColorPickerAction::InvalidHex));
                }
                if crate::ui::layout::pt_in(layout.cancel, event.column, event.row) {
                    return Some(ColorPickerAction::Cancel);
                }
                if crate::ui::layout::pt_in(layout.sv, event.column, event.row) {
                    self.focus = ColorField::SaturationValue;
                    self.drag = Some(ColorField::SaturationValue);
                    self.set_sv_at(layout.sv, event.column, event.row);
                } else if crate::ui::layout::pt_in(layout.hue, event.column, event.row) {
                    self.focus = ColorField::Hue;
                    self.drag = Some(ColorField::Hue);
                    self.set_hue_at(layout.hue, event.row);
                } else if let Some((index, value)) = channel_value(&layout, self.rgb, event) {
                    self.focus = [ColorField::Red, ColorField::Green, ColorField::Blue][index];
                    self.drag = Some(self.focus);
                    self.set_channel(index, value);
                } else if crate::ui::layout::pt_in(layout.hex, event.column, event.row) {
                    self.focus = ColorField::Hex;
                    self.hex.end();
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => match self.drag {
                Some(ColorField::SaturationValue) => {
                    self.set_sv_at(layout.sv, event.column, event.row)
                }
                Some(ColorField::Hue) => self.set_hue_at(layout.hue, event.row),
                Some(ColorField::Red | ColorField::Green | ColorField::Blue) => {
                    let index = match self.drag {
                        Some(ColorField::Red) => 0,
                        Some(ColorField::Green) => 1,
                        Some(ColorField::Blue) => 2,
                        _ => unreachable!(),
                    };
                    let area = layout.channels[index];
                    let event = MouseEvent {
                        column: event.column.clamp(area.x, area.right() - 1),
                        row: area.y,
                        ..event
                    };
                    if let Some((_, value)) = channel_value(&layout, self.rgb, event) {
                        self.set_channel(index, value);
                    }
                }
                _ => {}
            },
            _ => {}
        }
        None
    }

    fn rgb_component(&self, index: usize) -> u8 {
        [self.rgb.0, self.rgb.1, self.rgb.2][index]
    }

    fn set_channel(&mut self, index: usize, value: u8) {
        let mut rgb = [self.rgb.0, self.rgb.1, self.rgb.2];
        rgb[index] = value;
        self.set_rgb((rgb[0], rgb[1], rgb[2]));
    }

    fn set_rgb(&mut self, rgb: (u8, u8, u8)) {
        self.rgb = rgb;
        (self.hue, self.saturation, self.value) = rgb_to_hsv(rgb);
        self.hex.set(&hex_text(rgb));
    }

    fn apply_hsv(&mut self) {
        self.rgb = hsv_to_rgb(self.hue, self.saturation, self.value);
        self.hex.set(&hex_text(self.rgb));
    }

    fn apply_hex(&mut self) {
        if let Some(Color::Rgb(r, g, b)) = self.selected_color() {
            self.rgb = (r, g, b);
            (self.hue, self.saturation, self.value) = rgb_to_hsv(self.rgb);
        }
    }

    fn set_sv_at(&mut self, area: Rect, col: u16, row: u16) {
        let x = col.saturating_sub(area.x).min(area.width - 1) as f64;
        let y = row.saturating_sub(area.y).min(area.height - 1) as f64;
        self.saturation = x / (area.width - 1).max(1) as f64;
        self.value = 1.0 - y / (area.height - 1).max(1) as f64;
        self.apply_hsv();
    }

    fn set_hue_at(&mut self, area: Rect, row: u16) {
        let y = row.saturating_sub(area.y).min(area.height - 1) as f64;
        self.hue = y / (area.height - 1).max(1) as f64 * 359.0;
        self.apply_hsv();
    }
}

pub struct ColorPickerLayout {
    pub sv: Rect,
    pub hue: Rect,
    pub preview: Rect,
    pub channels: [Rect; 3],
    pub hex: Rect,
    pub ok: Rect,
    pub cancel: Rect,
    help: Rect,
}

pub struct ColorPicker<'a> {
    state: &'a ColorPickerState,
    area: Rect,
}

impl<'a> ColorPicker<'a> {
    pub fn new(state: &'a ColorPickerState, area: Rect) -> Self {
        Self { state, area }
    }

    pub fn layout(area: Rect) -> Option<ColorPickerLayout> {
        if area.width < 58 || area.height < 18 {
            return None;
        }
        let x = area.x + 2;
        let y = area.y + 1;
        let sv_width = (area.width - 38).min(30);
        let sv_height = (area.height - 10).min(12);
        let sv = Rect::new(x, y + 1, sv_width, sv_height);
        let hue = Rect::new(sv.right() + 2, sv.y, 2, sv_height);
        let controls_x = hue.right() + 2;
        let controls_width = area.right().saturating_sub(2 + controls_x);
        let channels = std::array::from_fn(|index| {
            Rect::new(controls_x + 7, y + 6 + index as u16, controls_width - 7, 1)
        });
        Some(ColorPickerLayout {
            sv,
            hue,
            preview: Rect::new(controls_x, y + 1, controls_width, 2),
            channels,
            hex: Rect::new(controls_x + 6, y + 4, controls_width - 6, 1),
            ok: Rect::new(area.right().saturating_sub(22), area.bottom() - 2, 10, 1),
            cancel: Rect::new(area.right().saturating_sub(11), area.bottom() - 2, 10, 1),
            help: Rect::new(x, area.bottom() - 3, area.width - 4, 1),
        })
    }

    pub fn draw(self, buf: &mut Buffer, theme: &Theme) -> Option<Position> {
        let Some(layout) = Self::layout(self.area) else {
            Label::new(
                "Increase the terminal size to edit colours.",
                self.area.width.saturating_sub(4) as usize,
            )
            .draw(
                buf,
                self.area.x + 2,
                self.area.y + 1,
                Style::new().fg(theme.dim),
            );
            return None;
        };
        let x = self.area.x + 2;
        let y = self.area.y + 1;
        let focused = |field| {
            Style::new().fg(if self.state.focus == field {
                theme.accent
            } else {
                theme.text
            })
        };
        text::put(
            buf,
            x,
            y,
            "Saturation / value",
            focused(ColorField::SaturationValue),
        );
        text::put(buf, layout.hue.x - 1, y, "Hue", focused(ColorField::Hue));
        draw_sv(buf, layout.sv, self.state);
        draw_hue(buf, layout.hue, self.state.hue);

        let controls_x = layout.preview.x;
        text::put(buf, controls_x, y, "Current", Style::new().fg(theme.text));
        buf.set_style(layout.preview, Style::new().bg(self.state.color()));
        text::put(buf, controls_x, y + 4, "Hex", focused(ColorField::Hex));
        let mut cursor = None;
        if self.state.focus == ColorField::Hex {
            cursor = TextBox::new(&self.state.hex, layout.hex).draw(buf, theme);
        } else {
            Label::new(&self.state.hex.as_string(), layout.hex.width as usize).draw(
                buf,
                layout.hex.x,
                layout.hex.y,
                Style::new().fg(theme.text).bg(theme.input_bg),
            );
        }
        for (index, (label, field)) in [
            ("R", ColorField::Red),
            ("G", ColorField::Green),
            ("B", ColorField::Blue),
        ]
        .into_iter()
        .enumerate()
        {
            let row = layout.channels[index].y;
            Label::new(&format!("{label} {:3}", self.state.rgb_component(index)), 6)
                .align(Alignment::Right)
                .draw(buf, controls_x, row, focused(field));
            Slider::new(
                self.state.rgb_component(index) as usize,
                0,
                255,
                Orientation::Horizontal,
                layout.channels[index],
            )
            .draw(buf, theme);
        }
        let help = if self.state.selected_color().is_none() {
            "Invalid hex: expected #RRGGBB"
        } else {
            "Tab focus  arrows adjust  Enter apply  Esc cancel"
        };
        Label::new(help, layout.help.width as usize).draw(
            buf,
            layout.help.x,
            layout.help.y,
            Style::new().fg(if self.state.selected_color().is_none() {
                theme.xcol
            } else {
                theme.dim
            }),
        );
        Button::new("OK", layout.ok)
            .enabled(self.state.selected_color().is_some())
            .selected(self.state.focus == ColorField::Ok)
            .draw(buf, theme);
        Button::new("Cancel", layout.cancel)
            .selected(self.state.focus == ColorField::Cancel)
            .draw(buf, theme);
        cursor
    }
}

fn channel_value(
    layout: &ColorPickerLayout,
    rgb: (u8, u8, u8),
    event: MouseEvent,
) -> Option<(usize, u8)> {
    layout
        .channels
        .iter()
        .enumerate()
        .find_map(|(index, area)| {
            Slider::new(
                [rgb.0, rgb.1, rgb.2][index] as usize,
                0,
                255,
                Orientation::Horizontal,
                *area,
            )
            .value_at(event.column, event.row)
            .map(|value| (index, value as u8))
        })
}

fn draw_sv(buf: &mut Buffer, area: Rect, state: &ColorPickerState) {
    for row in 0..area.height {
        for col in 0..area.width {
            let saturation = col as f64 / (area.width - 1).max(1) as f64;
            let value = 1.0 - row as f64 / (area.height - 1).max(1) as f64;
            let (r, g, b) = hsv_to_rgb(state.hue, saturation, value);
            text::set_cell(
                buf,
                area.x + col,
                area.y + row,
                " ",
                Color::Rgb(r, g, b),
                Color::Rgb(r, g, b),
            );
        }
    }
    let col = (state.saturation * (area.width - 1) as f64).round() as u16;
    let row = ((1.0 - state.value) * (area.height - 1) as f64).round() as u16;
    text::set_cell(
        buf,
        area.x + col,
        area.y + row,
        "+",
        contrasting(state.color()),
        state.color(),
    );
}

fn draw_hue(buf: &mut Buffer, area: Rect, hue: f64) {
    for row in 0..area.height {
        let degrees = row as f64 / (area.height - 1).max(1) as f64 * 359.0;
        let (r, g, b) = hsv_to_rgb(degrees, 1.0, 1.0);
        for col in 0..area.width {
            text::set_cell(
                buf,
                area.x + col,
                area.y + row,
                " ",
                Color::Rgb(r, g, b),
                Color::Rgb(r, g, b),
            );
        }
    }
    let row = (hue / 359.0 * (area.height - 1) as f64).round() as u16;
    let (r, g, b) = hsv_to_rgb(hue, 1.0, 1.0);
    let color = Color::Rgb(r, g, b);
    text::put(
        buf,
        area.x,
        area.y + row.min(area.height - 1),
        "<>",
        Style::new().fg(contrasting(color)).bg(color),
    );
}

fn contrasting(color: Color) -> Color {
    let Some((r, g, b)) = color_to_rgb(color) else {
        return Color::White;
    };
    if r as u32 * 299 + g as u32 * 587 + b as u32 * 114 >= 128_000 {
        Color::Black
    } else {
        Color::White
    }
}

fn parse_hex(text: &str) -> Option<Color> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some(Color::Rgb(
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ))
}

fn hex_text(rgb: (u8, u8, u8)) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2)
}

fn rgb_to_hsv(rgb: (u8, u8, u8)) -> (f64, f64, f64) {
    let r = rgb.0 as f64 / 255.0;
    let g = rgb.1 as f64 / 255.0;
    let b = rgb.2 as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let mut hue = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    if hue >= 360.0 {
        hue -= 360.0;
    }
    let saturation = if max == 0.0 { 0.0 } else { delta / max };
    (hue, saturation, max)
}

fn hsv_to_rgb(hue: f64, saturation: f64, value: f64) -> (u8, u8, u8) {
    let chroma = value * saturation;
    let segment = hue.rem_euclid(360.0) / 60.0;
    let x = chroma * (1.0 - (segment % 2.0 - 1.0).abs());
    let (r, g, b) = match segment as usize {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = value - chroma;
    (
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}

pub fn color_to_rgb(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Black => Some((0, 0, 0)),
        Color::Red => Some((128, 0, 0)),
        Color::Green => Some((0, 128, 0)),
        Color::Yellow => Some((128, 128, 0)),
        Color::Blue => Some((0, 0, 128)),
        Color::Magenta => Some((128, 0, 128)),
        Color::Cyan => Some((0, 128, 128)),
        Color::Gray => Some((192, 192, 192)),
        Color::DarkGray => Some((128, 128, 128)),
        Color::LightRed => Some((255, 0, 0)),
        Color::LightGreen => Some((0, 255, 0)),
        Color::LightYellow => Some((255, 255, 0)),
        Color::LightBlue => Some((0, 0, 255)),
        Color::LightMagenta => Some((255, 0, 255)),
        Color::LightCyan => Some((0, 255, 255)),
        Color::White => Some((255, 255, 255)),
        Color::Indexed(index) => Some(indexed_rgb(index)),
        Color::Reset => None,
    }
}

fn indexed_rgb(index: u8) -> (u8, u8, u8) {
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let index = index - 16;
            let levels = [0, 95, 135, 175, 215, 255];
            (
                levels[(index / 36) as usize],
                levels[((index / 6) % 6) as usize],
                levels[(index % 6) as usize],
            )
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn colour_conversion_and_hex_editing_are_consistent() {
        let mut state = ColorPickerState::new(Color::White).unwrap();
        assert_eq!(state.selected_color(), Some(Color::Rgb(255, 255, 255)));
        assert_eq!(color_to_rgb(Color::Indexed(196)), Some((255, 0, 0)));
        state.focus = ColorField::Hex;
        state.hex.set("#0080ff");
        state.apply_hex();
        assert_eq!(state.selected_color(), Some(Color::Rgb(0, 128, 255)));
        state.handle_key(key(KeyCode::End));
        state.handle_key(key(KeyCode::Backspace));
        assert_eq!(state.selected_color(), None);
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            Some(ColorPickerAction::InvalidHex)
        );
    }

    #[test]
    fn keyboard_and_mouse_edit_every_colour_control() {
        let area = Rect::new(0, 0, 68, 22);
        let layout = ColorPicker::layout(area).unwrap();
        let mut state = ColorPickerState::new(Color::Rgb(20, 40, 60)).unwrap();
        state.handle_key(key(KeyCode::Right));
        assert_ne!(state.color(), Color::Rgb(20, 40, 60));
        let hue = state.hue;
        state.handle_key(key(KeyCode::Tab));
        state.handle_key(key(KeyCode::Right));
        assert!((state.hue - hue - 5.0).abs() < 0.000_001);
        let event = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        state.mouse(
            area,
            event(
                MouseEventKind::Down(MouseButton::Left),
                layout.channels[1].right() - 1,
                layout.channels[1].y,
            ),
        );
        assert_eq!(state.rgb.1, 255);
        state.mouse(
            area,
            event(
                MouseEventKind::Drag(MouseButton::Left),
                layout.channels[1].x,
                layout.channels[1].y,
            ),
        );
        assert_eq!(state.rgb.1, 0);
        state.mouse(area, event(MouseEventKind::Up(MouseButton::Left), 0, 0));
        assert!(!state.dragging());
        state.mouse(
            area,
            event(
                MouseEventKind::Down(MouseButton::Left),
                layout.sv.right() - 1,
                layout.sv.bottom() - 1,
            ),
        );
        assert_eq!(state.color(), Color::Rgb(0, 0, 0));
        state.mouse(
            area,
            event(
                MouseEventKind::Drag(MouseButton::Left),
                layout.sv.right() - 1,
                layout.sv.y,
            ),
        );
        assert_ne!(state.color(), Color::Rgb(0, 0, 0));
        state.mouse(area, event(MouseEventKind::Up(MouseButton::Left), 0, 0));
        assert!(!state.dragging());
        assert_eq!(
            state.mouse(
                area,
                event(
                    MouseEventKind::Down(MouseButton::Left),
                    layout.cancel.x,
                    layout.cancel.y,
                ),
            ),
            Some(ColorPickerAction::Cancel)
        );
    }

    #[test]
    fn picker_draws_separate_preview_palette_and_hex_input() {
        let area = Rect::new(0, 0, 68, 22);
        let layout = ColorPicker::layout(area).unwrap();
        let state = ColorPickerState::new(Color::Rgb(12, 128, 240)).unwrap();
        let backend = ratatui::backend::TestBackend::new(area.width, area.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                ColorPicker::new(&state, area).draw(frame.buffer_mut(), &Theme::DARK);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer
                .cell((layout.preview.x, layout.preview.y))
                .unwrap()
                .bg,
            state.color()
        );
        let hex: String = (layout.hex.x..layout.hex.x + 7)
            .map(|x| buffer.cell((x, layout.hex.y)).unwrap().symbol())
            .collect();
        assert_eq!(hex, "#0c80f0");
    }
}
