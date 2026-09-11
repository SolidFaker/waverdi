use ratatui::layout::{Constraint, Layout as TuiLayout, Rect};

/// Horizontal pane proportions, adjustable by dragging the pane borders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Splits {
    pub tree_pct: u16,
    pub list_pct: u16,
}

impl Default for Splits {
    fn default() -> Self {
        Self {
            tree_pct: 24,
            list_pct: 31,
        }
    }
}

impl Splits {
    pub const MIN_PCT: u16 = 8;
    pub const MAX_PCT: u16 = 60;
    /// Minimum width left for the waveform pane, in percent.
    pub const MIN_WAVE_PCT: u16 = 10;

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
    pub tree: Rect,
    pub list: Rect,
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

    /// Column of the tree/list border, which can be dragged to resize.
    pub fn tree_grip_x(&self) -> u16 {
        self.tree.right().saturating_sub(1)
    }

    /// Column of the list/wave separator, which can be dragged to resize.
    pub fn list_grip_x(&self) -> u16 {
        self.list.right()
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
        Constraint::Length(TOOLBAR_H),
        Constraint::Fill(1),
        Constraint::Length(MESSAGE_H),
        Constraint::Length(STATUS_H),
    ])
    .split(area);
    let (menu, toolbar, main, msg, status) = (rows[0], rows[1], rows[2], rows[3], rows[4]);

    let cols = TuiLayout::horizontal([
        Constraint::Percentage(splits.tree_pct),
        Constraint::Percentage(splits.list_pct),
        Constraint::Fill(1),
    ])
    .split(main);
    let (tree, list, wave) = (cols[0], cols[1], cols[2]);

    let rows_h = main.height.saturating_sub(RULER_H + 1);
    let rows = Rect {
        x: wave.x,
        y: wave.y + RULER_H,
        width: wave.width.saturating_sub(1),
        height: rows_h,
    };
    let ruler = Rect {
        x: wave.x,
        y: wave.y,
        width: wave.width,
        height: RULER_H,
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
        assert_eq!(l.menu.height, 1);
        assert_eq!(l.toolbar.y, 1);
        assert_eq!(l.rows.y, l.wave.y + 2);
        assert_eq!(l.rows.bottom(), l.hscroll.y);
        assert_eq!(l.rows.width as usize, l.cols);
        assert_eq!(l.vscroll_x, l.wave.right() - 1);
        assert_eq!(l.hscroll.right(), l.vscroll_x);
        assert_eq!(l.msg.y + l.msg.height, l.status.y);
        assert_eq!(l.tree_grip_x() + 1, l.list.x);
        assert_eq!(l.list_grip_x(), l.wave.x);
    }

    #[test]
    fn splits_change_pane_widths() {
        let wide = compute_layout(
            Rect::new(0, 0, 100, 40),
            Splits {
                tree_pct: 40,
                list_pct: 30,
            },
        );
        let narrow = compute_layout(
            Rect::new(0, 0, 100, 40),
            Splits {
                tree_pct: 12,
                list_pct: 30,
            },
        );
        assert!(wide.tree.width > narrow.tree.width);
        assert_eq!(wide.list.width, narrow.list.width);
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
