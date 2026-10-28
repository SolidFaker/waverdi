use crate::settings::theme::Theme;
use crate::ui::text;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

pub struct Slider {
    value: usize,
    min: usize,
    max: usize,
    step: usize,
    orientation: Orientation,
    area: Rect,
}

impl Slider {
    pub fn new(value: usize, min: usize, max: usize, orientation: Orientation, area: Rect) -> Self {
        Self {
            value: value.clamp(min, max),
            min,
            max,
            step: 1,
            orientation,
            area,
        }
    }

    pub fn step(mut self, step: usize) -> Self {
        self.step = step.max(1);
        self
    }

    pub fn draw(self, buf: &mut Buffer, theme: &Theme) {
        let length = self.length();
        if length == 0 {
            return;
        }
        let thumb = self.thumb_position();
        for offset in 0..length {
            let (x, y) = self.point(offset);
            let symbol = if offset == thumb { "█" } else { self.track() };
            text::set_cell(buf, x, y, symbol, theme.accent, theme.popup_bg);
        }
    }

    pub fn value_at(&self, col: u16, row: u16) -> Option<usize> {
        if !crate::ui::layout::pt_in(self.area, col, row) {
            return None;
        }
        let length = self.length();
        if length == 0 {
            return None;
        }
        let offset = match self.orientation {
            Orientation::Horizontal => col.saturating_sub(self.area.x) as usize,
            Orientation::Vertical => row.saturating_sub(self.area.y) as usize,
        }
        .min(length - 1);
        let ratio = offset as f64 / (length - 1).max(1) as f64;
        let span = self.max - self.min;
        let raw = match self.orientation {
            Orientation::Horizontal => self.min + (ratio * span as f64).round() as usize,
            Orientation::Vertical => self.max - (ratio * span as f64).round() as usize,
        };
        Some(self.round(raw))
    }

    pub fn handle_key(&self, key: KeyEvent) -> Option<usize> {
        let next = match key.code {
            KeyCode::Left | KeyCode::Down => self.value.saturating_sub(self.step),
            KeyCode::Right | KeyCode::Up => self.value.saturating_add(self.step),
            KeyCode::Home => self.min,
            KeyCode::End => self.max,
            _ => return None,
        };
        Some(next.clamp(self.min, self.max))
    }

    fn round(&self, raw: usize) -> usize {
        let stepped = (raw / self.step) * self.step;
        stepped.clamp(self.min, self.max)
    }

    fn length(&self) -> usize {
        match self.orientation {
            Orientation::Horizontal => self.area.width as usize,
            Orientation::Vertical => self.area.height as usize,
        }
    }

    fn track(&self) -> &'static str {
        match self.orientation {
            Orientation::Horizontal => "─",
            Orientation::Vertical => "│",
        }
    }

    fn thumb_position(&self) -> usize {
        let span = self.max - self.min;
        if span == 0 {
            return 0;
        }
        let ratio = (self.value - self.min) as f64 / span as f64;
        let ratio = match self.orientation {
            Orientation::Horizontal => ratio,
            Orientation::Vertical => 1.0 - ratio,
        };
        (ratio * self.length().saturating_sub(1) as f64).round() as usize
    }

    fn point(&self, offset: usize) -> (u16, u16) {
        match self.orientation {
            Orientation::Horizontal => (self.area.x + offset as u16, self.area.y),
            Orientation::Vertical => (self.area.x, self.area.y + offset as u16),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn horizontal_and_vertical_sliders_map_points_and_clamp() {
        let horizontal = Slider::new(50, 0, 100, Orientation::Horizontal, Rect::new(0, 0, 11, 1));
        assert_eq!(horizontal.value_at(5, 0), Some(50));
        assert_eq!(horizontal.value_at(10, 0), Some(100));
        let vertical =
            Slider::new(50, 0, 100, Orientation::Vertical, Rect::new(0, 0, 1, 11)).step(5);
        assert_eq!(vertical.value_at(0, 0), Some(100));
        assert_eq!(vertical.value_at(0, 10), Some(0));
        assert_eq!(vertical.value_at(0, 5), Some(50));
        assert_eq!(
            vertical.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
            Some(100)
        );
    }
}
