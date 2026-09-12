use ratatui::layout::{Constraint, Layout as TuiLayout, Rect};

/// Horizontal pane proportions, adjustable by dragging the pane borders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Splits {
    pub tree_pct: u16,
    pub list_pct: u16,
    /// Height percentage of the instance/source row above nWave.
    pub top_pct: u16,
}

impl Default for Splits {
    fn default() -> Self {
        Self {
            tree_pct: 24,
            list_pct: 31,
            top_pct: 60,
        }
    }
}

impl Splits {
    pub const MIN_PCT: u16 = 8;
    pub const MAX_PCT: u16 = 60;
    /// Minimum width left for the waveform pane, in percent.
    pub const MIN_WAVE_PCT: u16 = 10;
    /// Bounds of the horizontal split between the top row and nWave.
    pub const MIN_TOP_PCT: u16 = 20;
    pub const MAX_TOP_PCT: u16 = 80;

    pub fn max_tree_pct(&self) -> u16 {
        (100 - self.list_pct - Self::MIN_WAVE_PCT).min(Self::MAX_PCT)
    }

    pub fn max_list_pct(&self) -> u16 {
        (100 - self.tree_pct - Self::MIN_WAVE_PCT).min(Self::MAX_PCT)
    }
}

/// Screen geometry shared by rendering and mouse hit testing.
pub struct Layout {
    pub area: Rect,
    pub menu: Rect,
    pub toolbar: Rect,
    /// Instance (hierarchy) pane, top-left.
    pub tree: Rect,
    /// RTL source pane, top-right.
    pub source: Rect,
    /// nWave frame: the signal list and waveform panes live inside it.
    pub nwave: Rect,
    /// Signal List pane inside nWave (no frame of its own).
    pub list: Rect,
    /// Waveform pane inside nWave (no frame of its own).
    pub wave: Rect,
    pub ruler: Rect,
    pub rows: Rect,
    pub hscroll: Rect,
    pub msg: Rect,
    pub status: Rect,
    /// Waveform columns (excluding the vertical scrollbar).
    pub cols: usize,
    /// Waveform rows visible between the ruler and the horizontal scrollbar.
    pub rows_h: usize,
    pub vscroll_x: u16,
}

impl Layout {
    pub fn tree_height(&self) -> usize {
        self.tree.height.saturating_sub(2) as usize
    }

    /// Column of the instance/source border, which can be dragged to resize.
    pub fn tree_grip_x(&self) -> u16 {
        self.tree.right().saturating_sub(1)
    }

    /// Column between the Signal List and the waveforms, draggable.
    pub fn list_grip_x(&self) -> u16 {
        self.wave.x.saturating_sub(1)
    }

    /// Row between the instance/source row and nWave, draggable.
    pub fn split_grip_y(&self) -> u16 {
        self.nwave.y
    }
}

const MENUBAR_H: u16 = 1;
const TOOLBAR_H: u16 = 1;
const MESSAGE_H: u16 = 3;
const STATUS_H: u16 = 1;
const RULER_H: u16 = 2;

pub fn compute_layout(area: Rect, splits: Splits) -> Layout {
    let rows = TuiLayout::vertical([
        Constraint::Length(MENUBAR_H),
        Constraint::Fill(1),
        Constraint::Length(MESSAGE_H),
        Constraint::Length(STATUS_H),
    ])
    .split(area);
    let (menu, main, msg, status) = (rows[0], rows[1], rows[2], rows[3]);

    // Top row: Instance | Source. Bottom: nWave (list + waveforms). The panes
    // share the border row between the two rows.
    let top_rows = ((main.height as u32 * splits.top_pct as u32 / 100) as u16)
        .clamp(4, main.height.saturating_sub(5).max(4))
        .min(main.height);
    let top = Rect {
        x: main.x,
        y: main.y,
        width: main.width,
        height: top_rows.saturating_sub(1),
    };
    let bottom = Rect {
        x: main.x,
        y: main.y.saturating_add(top.height),
        width: main.width,
        height: main.height.saturating_sub(top.height),
    };

    let top_cols =
        TuiLayout::horizontal([Constraint::Percentage(splits.tree_pct), Constraint::Fill(1)])
            .split(top);
    let tree = top_cols[0];
    let source = Rect {
        x: top_cols[1].x.saturating_sub(1),
        y: top.y,
        width: top_cols[1].width + 1,
        height: top.height,
    };

    let nwave = bottom;
    let inner = Rect {
        x: nwave.x + 1,
        y: nwave.y + 1,
        width: nwave.width.saturating_sub(2),
        height: nwave.height.saturating_sub(2),
    };
    let toolbar = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: TOOLBAR_H.min(inner.height),
    };
    let panes = Rect {
        x: inner.x,
        y: inner.y.saturating_add(TOOLBAR_H),
        width: inner.width,
        height: inner.height.saturating_sub(TOOLBAR_H),
    };

    let list_w = (panes.width as u32 * splits.list_pct as u32 / 100) as u16;
    let list = Rect {
        x: panes.x,
        y: panes.y,
        width: list_w.min(panes.width),
        height: panes.height,
    };
    let wave = Rect {
        x: panes.x.saturating_add(list_w).saturating_add(1),
        y: panes.y,
        width: panes.width.saturating_sub(list_w).saturating_sub(1),
        height: panes.height,
    };

    let ruler = Rect {
        x: wave.x,
        y: wave.y,
        width: wave.width,
        height: RULER_H.min(wave.height),
    };
    let rows_h = wave.height.saturating_sub(RULER_H + 1);
    let rows = Rect {
        x: wave.x,
        y: wave.y.saturating_add(RULER_H),
        width: wave.width.saturating_sub(1),
        height: rows_h,
    };
    let hscroll = Rect {
        x: wave.x,
        y: wave.bottom().saturating_sub(1),
        width: wave.width.saturating_sub(1),
        height: 1,
    };
    let vscroll_x = wave.right().saturating_sub(1);

    Layout {
        area,
        menu,
        toolbar,
        tree,
        source,
        nwave,
        list,
        wave,
        ruler,
        rows,
        hscroll,
        msg,
        status,
        cols: rows.width as usize,
        rows_h: rows_h as usize,
        vscroll_x,
    }
}

pub fn tree_inner(l: &Layout) -> Rect {
    Rect {
        x: l.tree.x + 1,
        y: l.tree.y + 1,
        width: l.tree.width.saturating_sub(2),
        height: l.tree.height.saturating_sub(2),
    }
}

pub fn pt_in(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x && y >= rect.y && x < rect.right() && y < rect.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_consistent() {
        let l = compute_layout(Rect::new(0, 0, 120, 40), Splits::default());
        assert_eq!(l.menu.y, 0);
        assert_eq!(l.menu.height, 1);
        assert_eq!(l.toolbar.y, l.nwave.y + 1);
        assert_eq!(l.toolbar.x, l.nwave.x + 1);
        // Instance and source share their border column.
        assert_eq!(l.tree_grip_x(), l.source.x);
        assert_eq!(l.source.right(), l.area.right());
        assert!(l.nwave.y > l.tree.y);
        assert!(l.nwave.bottom() <= l.msg.y);
        // The top row ends directly above nWave; the grip is the frame line.
        assert_eq!(l.tree.bottom(), l.nwave.y);
        assert_eq!(l.split_grip_y(), l.nwave.y);
        assert_eq!(l.rows.y, l.wave.y + 2);
        assert_eq!(l.rows.bottom(), l.hscroll.y);
        assert_eq!(l.rows.width as usize, l.cols);
        assert_eq!(l.vscroll_x, l.wave.right() - 1);
        assert_eq!(l.hscroll.right(), l.vscroll_x);
        assert_eq!(l.msg.y + l.msg.height, l.status.y);
        // The Signal List and the waveforms are separated by one column.
        assert_eq!(l.list_grip_x() + 1, l.wave.x);
    }

    #[test]
    fn splits_change_pane_widths() {
        let wide = compute_layout(
            Rect::new(0, 0, 100, 40),
            Splits {
                tree_pct: 40,
                list_pct: 30,
                ..Splits::default()
            },
        );
        let narrow = compute_layout(
            Rect::new(0, 0, 100, 40),
            Splits {
                tree_pct: 12,
                list_pct: 30,
                ..Splits::default()
            },
        );
        assert!(wide.tree.width > narrow.tree.width);
        assert_eq!(wide.list.width, narrow.list.width);
    }

    #[test]
    fn top_split_changes_the_row_heights() {
        let tall = compute_layout(
            Rect::new(0, 0, 100, 40),
            Splits {
                top_pct: 70,
                ..Splits::default()
            },
        );
        let short = compute_layout(
            Rect::new(0, 0, 100, 40),
            Splits {
                top_pct: 30,
                ..Splits::default()
            },
        );
        assert!(tall.tree.height > short.tree.height);
        assert!(tall.nwave.height < short.nwave.height);
        assert_eq!(tall.tree.bottom(), tall.nwave.y);
    }

    #[test]
    fn layout_survives_tiny_terminal() {
        let l = compute_layout(Rect::new(0, 0, 0, 0), Splits::default());
        assert_eq!((l.cols, l.rows_h), (0, 0));
        for (w, h) in [(1, 1), (5, 3), (20, 5), (10, 40)] {
            let l = compute_layout(Rect::new(0, 0, w, h), Splits::default());
            assert!(l.rows_h <= h as usize);
            assert!(l.cols <= w as usize);
        }
    }

    #[test]
    fn hit_testing() {
        let r = Rect::new(3, 4, 5, 2);
        assert!(pt_in(r, 3, 4));
        assert!(pt_in(r, 7, 5));
        assert!(!pt_in(r, 8, 5));
        assert!(!pt_in(r, 3, 3));
    }
}
