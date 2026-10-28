use crate::app::InputState;
use crate::settings::theme::Theme;
use crate::ui::text;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Style};

pub struct TextBox<'a> {
    state: &'a InputState,
    area: Rect,
}

impl<'a> TextBox<'a> {
    pub fn new(state: &'a InputState, area: Rect) -> Self {
        Self { state, area }
    }

    pub fn draw(self, buf: &mut Buffer, theme: &Theme) -> Position {
        let style = Style::new().fg(theme.text).bg(theme.input_bg);
        buf.set_style(self.area, style);
        let width = self.area.width as usize;
        let value = self.state.as_string();
        let shown = text::trunc(&value, width);
        text::put(buf, self.area.x, self.area.y, &shown, style);
        let caret = self.state.cursor().min(shown.chars().count()) as u16;
        let x = self.area.x + caret;
        let y = self.area.y;
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_bg(theme.text);
            cell.set_fg(Color::Black);
        }
        Position::new(x, y)
    }

    pub fn handle_key(state: &mut InputState, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c) => state.insert(c),
            KeyCode::Backspace => state.backspace(),
            KeyCode::Delete => state.delete(),
            KeyCode::Left => state.left(),
            KeyCode::Right => state.right(),
            KeyCode::Home => state.home(),
            KeyCode::End => state.end(),
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn text_box_edits_and_positions_cursor() {
        let mut state = InputState::default();
        assert!(TextBox::handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)
        ));
        assert!(TextBox::handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE)
        ));
        assert!(TextBox::handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)
        ));
        assert!(TextBox::handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)
        ));
        assert_eq!(state.as_string(), "acb");
        assert!(!TextBox::handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ));
    }
}
