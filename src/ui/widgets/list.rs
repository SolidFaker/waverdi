use crate::settings::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListView {
    pub area: Rect,
    pub bar: Rect,
    pub total: usize,
    pub scroll: usize,
}

impl ListView {
    pub fn new(area: Rect, total: usize, requested: usize, selected: usize) -> Self {
        let rows = area.height as usize;
        let max_scroll = total.saturating_sub(rows);
        let mut scroll = requested.min(max_scroll);
        if selected < scroll {
            scroll = selected;
        } else if rows > 0 && selected >= scroll + rows {
            scroll = selected + 1 - rows;
        }
        scroll = scroll.min(max_scroll);
        Self {
            area: Rect {
                width: area.width.saturating_sub(1),
                ..area
            },
            bar: Rect {
                x: area.right().saturating_sub(1),
                width: 1,
                ..area
            },
            total,
            scroll,
        }
    }

    pub fn dialog(
        outer: Rect,
        top: u16,
        bottom: u16,
        total: usize,
        scroll: usize,
        selected: usize,
    ) -> Self {
        Self::new(
            Rect {
                x: outer.x.saturating_add(2),
                y: outer.y.saturating_add(top),
                width: outer.width.saturating_sub(4),
                height: outer.height.saturating_sub(top.saturating_add(bottom)),
            },
            total,
            scroll,
            selected,
        )
    }

    pub fn rows(&self) -> usize {
        self.area.height as usize
    }

    pub fn row_at(&self, col: u16, row: u16) -> Option<usize> {
        if !crate::ui::layout::pt_in(self.area, col, row) {
            return None;
        }
        let index = self.scroll + (row - self.area.y) as usize;
        (index < self.total).then_some(index)
    }

    pub fn scroll_to(&self, row: u16) -> Option<usize> {
        (self.rows() > 0 && self.total > self.rows()).then(|| {
            crate::ui::scrollbar::scroll_from_track(
                self.rows(),
                self.total,
                self.rows(),
                row.saturating_sub(self.bar.y) as usize,
            )
        })
    }

    pub fn draw_scrollbar(&self, buf: &mut Buffer, theme: &Theme) {
        let Some((start, len)) = crate::ui::scrollbar::track_geometry(
            self.rows(),
            self.total,
            self.rows(),
            self.scroll,
            true,
        ) else {
            return;
        };
        crate::ui::scrollbar::Bar {
            orientation: crate::ui::scrollbar::Orientation::Vertical,
            x: self.bar.x,
            y: self.bar.y,
            span: self.rows(),
            start,
            len,
            thumb: "█",
            track: "│",
            thumb_fg: theme.accent,
            track_fg: theme.dim,
            bg: theme.popup_bg,
        }
        .draw(buf);
    }

    pub fn row_style(theme: &Theme, selected: bool) -> Style {
        if selected {
            Style::new()
                .fg(Color::Black)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.text).bg(theme.popup_bg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_drawing_hits_and_scrolling_share_geometry() {
        let list = ListView::new(Rect::new(2, 3, 12, 5), 20, 0, 12);
        assert_eq!(list.scroll, 8);
        assert_eq!(list.rows(), 5);
        assert_eq!(list.row_at(2, 3), Some(8));
        assert_eq!(list.row_at(2, 7), Some(12));
        assert_eq!(list.row_at(2, 8), None);
        assert_eq!(list.row_at(13, 3), None);
        assert_eq!(list.row_at(14, 3), None);
        assert_eq!(list.scroll_to(7), Some(15));
        assert_eq!(list.scroll_to(3), Some(0));
        assert!(list.row_at(2, 7).is_some());

        let fits = ListView::new(Rect::new(0, 0, 10, 4), 3, 0, 0);
        assert_eq!(fits.scroll_to(0), None);
        let zero = ListView::new(Rect::new(0, 0, 10, 0), 30, 0, 10);
        assert_eq!(zero.row_at(0, 0), None);
        assert_eq!(zero.scroll_to(0), None);
    }
}
