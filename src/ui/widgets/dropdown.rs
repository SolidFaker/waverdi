use crate::settings::theme::Theme;
use crate::ui::text;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropdownState {
    pub selected: usize,
    pub open: bool,
}

pub struct Dropdown<'a> {
    options: &'a [&'a str],
    state: DropdownState,
    area: Rect,
}

impl<'a> Dropdown<'a> {
    pub fn new(options: &'a [&'a str], state: DropdownState, area: Rect) -> Self {
        Self {
            options,
            state,
            area,
        }
    }

    pub fn draw(self, buf: &mut Buffer, theme: &Theme) {
        let current = self.options.get(self.state.selected).copied().unwrap_or("");
        let marker = if self.state.open { "▴" } else { "▾" };
        text::put(
            buf,
            self.area.x,
            self.area.y,
            &text::trunc(&format!("{current} {marker}"), self.area.width as usize),
            Style::new().fg(theme.text).bg(theme.input_bg),
        );
        if !self.state.open {
            return;
        }
        for (index, option) in self.options.iter().enumerate() {
            let style = if index == self.state.selected {
                Style::new().fg(Color::Black).bg(theme.accent)
            } else {
                Style::new().fg(theme.text).bg(theme.popup_bg)
            };
            text::put(
                buf,
                self.area.x,
                self.area.y + 1 + index as u16,
                &text::trunc(option, self.area.width as usize),
                style,
            );
        }
    }

    pub fn handle_key(state: &mut DropdownState, count: usize, key: KeyEvent) -> bool {
        if count == 0 {
            return false;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => state.open = !state.open,
            KeyCode::Esc => state.open = false,
            KeyCode::Up if state.open => {
                state.selected = state.selected.checked_sub(1).unwrap_or(count - 1);
            }
            KeyCode::Down if state.open => state.selected = (state.selected + 1) % count,
            _ => return false,
        }
        true
    }

    pub fn contains(&self, col: u16, row: u16) -> bool {
        crate::ui::layout::pt_in(self.area, col, row)
    }

    pub fn option_at(&self, col: u16, row: u16) -> Option<usize> {
        if !self.state.open || col < self.area.x || col >= self.area.right() {
            return None;
        }
        let index = row.checked_sub(self.area.y + 1)? as usize;
        (index < self.options.len()).then_some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn dropdown_opens_selects_and_closes() {
        let options = ["ns", "us", "ms"];
        let mut state = DropdownState::default();
        assert!(Dropdown::handle_key(
            &mut state,
            3,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ));
        assert!(state.open);
        assert!(Dropdown::handle_key(
            &mut state,
            3,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
        ));
        assert_eq!(state.selected, 1);
        let dropdown = Dropdown::new(&options, state, Rect::new(0, 0, 8, 1));
        assert_eq!(dropdown.option_at(0, 2), Some(1));
        assert!(Dropdown::handle_key(
            &mut state,
            3,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        ));
        assert!(!state.open);
    }
}
