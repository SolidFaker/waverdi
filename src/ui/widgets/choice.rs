use crate::settings::theme::Theme;
use crate::ui::text;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

pub struct RadioGroup<'a> {
    options: &'a [&'a str],
    selected: usize,
    area: Rect,
}

impl<'a> RadioGroup<'a> {
    pub fn new(options: &'a [&'a str], selected: usize, area: Rect) -> Self {
        Self {
            options,
            selected,
            area,
        }
    }

    pub fn draw(self, buf: &mut Buffer, theme: &Theme) {
        for (index, option) in self.options.iter().enumerate() {
            let selected = index == self.selected;
            let style = if selected {
                Style::new().fg(theme.accent)
            } else {
                Style::new().fg(theme.text)
            };
            let marker = if selected { "(●)" } else { "( )" };
            text::put(
                buf,
                self.area.x,
                self.area.y + index as u16,
                &format!("{marker} {option}"),
                style,
            );
        }
    }

    pub fn option_at(&self, col: u16, row: u16) -> Option<usize> {
        crate::ui::layout::pt_in(self.area, col, row)
            .then_some((row - self.area.y) as usize)
            .filter(|index| *index < self.options.len())
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
        let marker = if self.value { "[on]" } else { "[off]" };
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
    fn selected_radio_style_is_not_checkbox_style() {
        let backend = ratatui::backend::TestBackend::new(12, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let theme = Theme::DARK;
        terminal
            .draw(|frame| {
                RadioGroup::new(&["Dark"], 0, Rect::new(0, 0, 12, 1))
                    .draw(frame.buffer_mut(), &theme)
            })
            .unwrap();
        assert!(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|cell| cell.symbol() == "●"));
    }
}
