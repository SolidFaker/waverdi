use super::{Alignment, Label};
use crate::settings::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

pub struct Button<'a> {
    label: &'a str,
    pub area: Rect,
    enabled: bool,
    selected: bool,
}

impl<'a> Button<'a> {
    pub fn new(label: &'a str, area: Rect) -> Self {
        Self {
            label,
            area,
            enabled: true,
            selected: false,
        }
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn draw(self, buf: &mut Buffer, theme: &Theme) {
        let style = if !self.enabled {
            Style::new().fg(theme.dim).bg(theme.popup_bg)
        } else if self.selected {
            Style::new()
                .fg(Color::Black)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.accent).bg(theme.popup_bg)
        };
        let text = format!("[ {} ]", self.label);
        Label::new(&text, self.area.width as usize)
            .align(Alignment::Center)
            .draw(buf, self.area.x, self.area.y, style);
    }

    pub fn contains(&self, x: u16, y: u16) -> bool {
        self.enabled && crate::ui::layout::pt_in(self.area, x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_hit_and_disabled_state() {
        let area = Rect::new(2, 1, 8, 1);
        let button = Button::new("OK", area).selected(true);
        assert!(button.contains(2, 1));
        assert!(!button.contains(1, 1));
        assert!(!Button::new("OK", area).enabled(false).contains(2, 1));
    }
}
