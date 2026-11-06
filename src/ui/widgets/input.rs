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

    pub fn draw(self, buf: &mut Buffer, theme: &Theme) -> Option<Position> {
        let area = self.area.intersection(*buf.area());
        if area.width == 0 || area.height == 0 {
            return None;
        }
        let style = Style::new().fg(theme.text).bg(theme.input_bg);
        buf.set_style(area, style);
        let width = area.width as usize;
        let value = self.state.as_string();
        let cursor = self.state.cursor();
        let len = value.chars().count();
        let start = cursor
            .saturating_add(1)
            .saturating_sub(width)
            .min(len.saturating_sub(width));
        let shown = text::scroll_slice(&value, start, width);
        text::put(buf, area.x, area.y, &shown, style);
        let caret = cursor.saturating_sub(start).min(width.saturating_sub(1)) as u16;
        let x = area.x + caret;
        let y = area.y;
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_bg(theme.text);
            cell.set_fg(Color::Black);
        }
        Some(Position::new(x, y))
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

    #[test]
    fn long_input_scrolls_to_the_cursor() {
        let mut state = InputState::default();
        state.set("abcdefghij");
        let backend = ratatui::backend::TestBackend::new(4, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let theme = Theme::DARK;
        let mut caret = None;
        terminal
            .draw(|frame| {
                caret =
                    TextBox::new(&state, Rect::new(0, 0, 4, 1)).draw(frame.buffer_mut(), &theme);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = (0..4)
            .map(|x| buffer.cell((x, 0)).unwrap().symbol())
            .collect();
        assert_eq!(text, "…ghi");
        assert_eq!(caret, Some(Position::new(3, 0)));
    }

    #[test]
    fn invisible_text_box_has_no_cursor() {
        let mut state = InputState::default();
        state.set("text");
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        assert_eq!(
            TextBox::new(&state, Rect::new(0, 0, 4, 0)).draw(&mut buffer, &Theme::DARK),
            None
        );
        assert_eq!(
            TextBox::new(&state, Rect::new(5, 0, 4, 1)).draw(&mut buffer, &Theme::DARK),
            None
        );
    }
}
