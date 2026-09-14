//! Scrollbar drawing shared by the panes.
//!
//! One routine per orientation renders the track and thumb; the geometry
//! helpers cover the two ways the callers map content to a bar: the scroll
//! range (`track_geometry`) and the visible fraction of the whole content
//! (`proportional_geometry`).

use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

/// One scrollbar: where it sits, how long it is and which part is the thumb.
pub struct Bar<'a> {
    pub orientation: Orientation,
    /// Top-left cell of the bar.
    pub x: u16,
    pub y: u16,
    /// Bar length in cells.
    pub span: usize,
    /// First cell of the thumb, counted from the bar origin.
    pub start: usize,
    /// Thumb length in cells.
    pub len: usize,
    pub thumb: &'a str,
    pub track: &'a str,
    pub thumb_fg: Color,
    pub track_fg: Color,
    pub bg: Color,
}

impl Bar<'_> {
    pub fn draw(&self, buf: &mut Buffer) {
        if self.span == 0 || self.len == 0 {
            return;
        }
        for cell in 0..self.span {
            let on_thumb = cell >= self.start && cell < self.start + self.len;
            let (symbol, fg) = if on_thumb {
                (self.thumb, self.thumb_fg)
            } else {
                (self.track, self.track_fg)
            };
            let (x, y) = match self.orientation {
                Orientation::Vertical => (self.x, self.y + cell as u16),
                Orientation::Horizontal => (self.x + cell as u16, self.y),
            };
            text::set_cell(buf, x, y, symbol, fg, self.bg);
        }
    }
}

/// Thumb geometry for a bar with `content` items, `view` of them visible and
/// `offset` scrolled. `round` selects the position rounding used by the
/// dialog bars. `None` when everything fits.
pub fn track_geometry(
    span: usize,
    content: usize,
    view: usize,
    offset: usize,
    round: bool,
) -> Option<(usize, usize)> {
    if span == 0 || view == 0 || content <= view {
        return None;
    }
    let max = content - view;
    let len = ((view as f64 / content as f64) * span as f64).max(1.0) as usize;
    let len = len.min(span);
    let room = span - len;
    let start = (offset.min(max) as f64 / max as f64) * room as f64;
    let start = if round {
        start.round() as usize
    } else {
        start as usize
    };
    Some((start.min(room), len))
}

/// Thumb geometry when the thumb tracks the visible fraction of the whole
/// content (`view / content` of the bar) and the position is the scrolled
/// fraction of that content rather than of the scroll range.
pub fn proportional_geometry(
    span: usize,
    content: usize,
    view: usize,
    offset: usize,
) -> Option<(usize, usize)> {
    if span == 0 || view == 0 || content <= view {
        return None;
    }
    let len = ((view as f64 / content as f64) * span as f64).max(1.0) as usize;
    let start = ((offset.min(content) as f64 / content as f64) * span as f64) as usize;
    Some((start, len))
}

/// Draw a horizontal scrollbar track from `x0` to `x1` (exclusive) on `y`.
#[allow(clippy::too_many_arguments)]
pub fn h_scrollbar(
    buf: &mut Buffer,
    x0: u16,
    x1: u16,
    y: u16,
    content: usize,
    view: usize,
    offset: usize,
    fg: Color,
    bg: Color,
) {
    if x1 <= x0 || y >= buf.area().bottom() {
        return;
    }
    let Some((start, len)) = track_geometry((x1 - x0) as usize, content, view, offset, false)
    else {
        return;
    };
    Bar {
        orientation: Orientation::Horizontal,
        x: x0,
        y,
        span: (x1 - x0) as usize,
        start,
        len,
        thumb: "█",
        track: "─",
        thumb_fg: fg,
        track_fg: fg,
        bg,
    }
    .draw(buf);
}
