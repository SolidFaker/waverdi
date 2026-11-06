use crate::settings::theme::Theme;
use crate::ui::text;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RadioStyle {
    #[default]
    Marker,
    Highlight,
}

pub struct RadioGroup<'a> {
    options: &'a [&'a str],
    selected: usize,
    area: Rect,
    style: RadioStyle,
    horizontal: bool,
}

impl<'a> RadioGroup<'a> {
    pub fn new(options: &'a [&'a str], selected: usize, area: Rect) -> Self {
        Self {
            options,
            selected,
            area,
            style: RadioStyle::Marker,
            horizontal: false,
        }
    }

    pub fn horizontal(options: &'a [&'a str], selected: usize, area: Rect) -> Self {
        Self {
            options,
            selected,
            area,
            style: RadioStyle::Highlight,
            horizontal: true,
        }
    }

    pub fn with_style(mut self, style: RadioStyle) -> Self {
        self.style = style;
        self
    }

    fn option_rects(&self) -> Vec<Rect> {
        if !self.horizontal {
            return (0..self.options.len())
                .map(|index| Rect::new(self.area.x, self.area.y + index as u16, self.area.width, 1))
                .collect();
        }
        let mut x = self.area.x;
        let mut rects = Vec::with_capacity(self.options.len());
        for option in self.options {
            let padding = if self.style == RadioStyle::Highlight {
                2
            } else {
                4
            };
            let width = option.chars().count() as u16 + padding;
            let rect = Rect::new(x, self.area.y, width, 1);
            if rect.right() > self.area.right() {
                break;
            }
            rects.push(rect);
            x += width;
        }
        rects
    }

    pub fn draw(self, buf: &mut Buffer, theme: &Theme) {
        for (index, (option, rect)) in self.options.iter().zip(self.option_rects()).enumerate() {
            let selected = index == self.selected;
            match self.style {
                RadioStyle::Marker => {
                    let marker = if selected { "[x]" } else { "[ ]" };
                    let style = Style::new().fg(if selected { theme.accent } else { theme.text });
                    text::put(buf, rect.x, rect.y, &format!("{marker} {option}"), style);
                }
                RadioStyle::Highlight => {
                    let style = if selected {
                        Style::new()
                            .fg(theme.text)
                            .bg(theme.row_sel_bg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::new().fg(theme.text).bg(theme.popup_bg)
                    };
                    buf.set_style(rect, style);
                    text::put(buf, rect.x, rect.y, &format!(" {option} "), style);
                }
            }
        }
    }

    pub fn option_at(&self, col: u16, row: u16) -> Option<usize> {
        self.option_rects()
            .iter()
            .position(|rect| crate::ui::layout::pt_in(*rect, col, row))
    }

    pub fn handle_key(selected: usize, count: usize, key: KeyEvent) -> Option<usize> {
        if count == 0 {
            return None;
        }
        match key.code {
            KeyCode::Left | KeyCode::Up => Some(selected.checked_sub(1).unwrap_or(count - 1)),
            KeyCode::Right | KeyCode::Down => Some((selected + 1) % count),
            _ => None,
        }
    }
}

pub struct Toggle {
    value: bool,
    label: &'static str,
    area: Rect,
}

impl Toggle {
    pub fn new(value: bool, label: &'static str, area: Rect) -> Self {
        Self { value, label, area }
    }

    pub fn draw(self, buf: &mut Buffer, theme: &Theme) {
        let marker = if self.value { "[x]" } else { "[ ]" };
        let style = Style::new().fg(if self.value { theme.accent } else { theme.dim });
        text::put(
            buf,
            self.area.x,
            self.area.y,
            &format!("{marker} {}", self.label),
            style,
        );
    }

    pub fn changed(current: bool, key: KeyEvent) -> Option<bool> {
        matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')).then_some(!current)
    }

    pub fn changed_at(&self, col: u16, row: u16) -> Option<bool> {
        crate::ui::layout::pt_in(self.area, col, row).then_some(!self.value)
    }
}

pub struct Checkbox {
    value: bool,
    label: &'static str,
    area: Rect,
}

impl Checkbox {
    pub fn new(value: bool, label: &'static str, area: Rect) -> Self {
        Self { value, label, area }
    }

    pub fn draw(self, buf: &mut Buffer, theme: &Theme) {
        let marker = if self.value { "[x]" } else { "[ ]" };
        let style = Style::new().fg(if self.value { theme.accent } else { theme.text });
        text::put(
            buf,
            self.area.x,
            self.area.y,
            &format!("{marker} {}", self.label),
            style,
        );
    }

    pub fn changed(current: bool, key: KeyEvent) -> Option<bool> {
        matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')).then_some(!current)
    }

    pub fn changed_at(&self, col: u16, row: u16) -> Option<bool> {
        crate::ui::layout::pt_in(self.area, col, row).then_some(!self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn choice_inputs_have_distinct_state_and_geometry() {
        let options = ["Dark", "Light", "Mixed"];
        let radio = RadioGroup::new(&options, 1, Rect::new(0, 0, 10, 3));
        assert_eq!(radio.option_at(0, 2), Some(2));
        assert_eq!(RadioGroup::handle_key(1, 3, key(KeyCode::Right)), Some(2));
        assert_eq!(RadioGroup::handle_key(0, 3, key(KeyCode::Left)), Some(2));
        assert_eq!(Toggle::changed(false, key(KeyCode::Enter)), Some(true));
        assert_eq!(
            Toggle::new(false, "ellipsis", Rect::new(0, 0, 10, 1)).changed_at(0, 0),
            Some(true)
        );
        assert_eq!(
            Checkbox::changed(true, key(KeyCode::Char(' '))),
            Some(false)
        );
        assert_eq!(
            Checkbox::new(false, "full names", Rect::new(0, 0, 12, 1)).changed_at(0, 0),
            Some(true)
        );
    }

    #[test]
    fn choice_markers_have_fixed_width() {
        let backend = ratatui::backend::TestBackend::new(12, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let theme = Theme::DARK;
        terminal
            .draw(|frame| {
                RadioGroup::new(&["Dark"], 0, Rect::new(0, 0, 12, 1))
                    .with_style(RadioStyle::Marker)
                    .draw(frame.buffer_mut(), &theme)
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.starts_with("[x] Dark"), "{text}");
    }

    #[test]
    fn horizontal_highlight_radios_share_drawing_and_hit_geometry() {
        let backend = ratatui::backend::TestBackend::new(16, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let theme = Theme::DARK;
        let options = ["all", "net", "reg"];
        let radio = RadioGroup::horizontal(&options, 1, Rect::new(0, 0, 16, 1));
        assert_eq!(radio.option_at(0, 0), Some(0));
        assert_eq!(radio.option_at(6, 0), Some(1));
        assert_eq!(radio.option_at(11, 0), Some(2));
        assert_eq!(radio.option_at(15, 0), None);
        terminal
            .draw(|frame| radio.draw(frame.buffer_mut(), &theme))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert_eq!(text, " all  net  reg  ");
        assert_eq!(buffer.cell((0, 0)).unwrap().bg, theme.popup_bg);
        assert_eq!(buffer.cell((5, 0)).unwrap().bg, theme.row_sel_bg);
        assert_eq!(buffer.cell((9, 0)).unwrap().bg, theme.row_sel_bg);
        assert_eq!(buffer.cell((10, 0)).unwrap().bg, theme.popup_bg);
    }
}
