use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    #[default]
    Left,
    Center,
    Right,
}

pub struct Label<'a> {
    text: &'a str,
    width: usize,
    alignment: Alignment,
}

impl<'a> Label<'a> {
    pub fn new(text: &'a str, width: usize) -> Self {
        Self {
            text,
            width,
            alignment: Alignment::Left,
        }
    }

    pub fn align(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    pub fn draw(self, buf: &mut Buffer, x: u16, y: u16, style: Style) -> Rect {
        let area = Rect::new(x, y, self.width as u16, 1);
        if self.width == 0 {
            return area;
        }
        buf.set_style(area, style);
        let shown = text::trunc(self.text, self.width);
        let used = shown.chars().count();
        let offset = match self.alignment {
            Alignment::Left => 0,
            Alignment::Center => self.width.saturating_sub(used) / 2,
            Alignment::Right => self.width.saturating_sub(used),
        };
        text::put(buf, x + offset as u16, y, &shown, style);
        area
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn fixed_width_labels_align_and_truncate() {
        let backend = ratatui::backend::TestBackend::new(12, 2);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                Label::new("abc", 6).draw(frame.buffer_mut(), 0, 0, Style::new().fg(Color::White));
                Label::new("xy", 6).align(Alignment::Right).draw(
                    frame.buffer_mut(),
                    0,
                    1,
                    Style::new().fg(Color::White),
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y| {
            (0..6)
                .map(|x| buffer.cell((x, y)).unwrap().symbol())
                .collect::<String>()
        };
        assert_eq!(row(0), "abc   ");
        assert_eq!(row(1), "    xy");
    }
}
