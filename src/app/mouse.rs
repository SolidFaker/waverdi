use super::{App, Drag, DragMode, Focus, TreeNode};
use crate::ui::layout::{pt_in, tree_inner, Layout, Splits};
use crate::ui::menubar;
use crate::ui::toolbar;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::time::{Duration, Instant};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const WHEEL_STEP: usize = 3;

/// Handle one mouse event. Returns `true` when the application should quit.
pub fn handle_mouse(app: &mut App, m: MouseEvent) -> bool {
    let (col, row) = (m.column, m.row);
    let shift = m.modifiers.contains(KeyModifiers::SHIFT);
    match m.kind {
        MouseEventKind::Down(btn) => mouse_down(app, col, row, btn),
        MouseEventKind::Drag(_) => {
            mouse_drag(app, col, row);
            false
        }
        MouseEventKind::Up(_) => {
            app.dragging = None;
            false
        }
        MouseEventKind::ScrollUp => {
            mouse_wheel(app, col, row, true, shift);
            false
        }
        MouseEventKind::ScrollDown => {
            mouse_wheel(app, col, row, false, shift);
            false
        }
        _ => false,
    }
}

fn mouse_down(app: &mut App, col: u16, row: u16, btn: MouseButton) -> bool {
    if app.dialog.is_some() {
        return false;
    }
    if let Some(menu) = app.menu.open {
        return menu_click(app, col, row, menu);
    }

    let l = app.layout();

    if pt_in(l.menu, col, row) {
        if let Some(i) = menubar::menu_item_at(l.menu, col) {
            app.menu.open = Some(i);
            app.menu.sel = 0;
        }
        return false;
    }

    if pt_in(l.toolbar, col, row) {
        return match toolbar::tool_at(l.toolbar, col) {
            Some(tool) => toolbar::run_tool(app, tool),
            None => false,
        };
    }

    if btn == MouseButton::Left {
        if let Some((mode, start_pct)) =
            grip_at(&l, col, row).map(|mode| (mode, split_pct(app, mode)))
        {
            app.dragging = Some(new_drag(mode, col, start_pct));
            return false;
        }
    }

    if btn == MouseButton::Middle && app.wf.is_some() {
        app.zoom_at(app.tick_at_x(col) as f64, 1.3);
        return false;
    }

    let is_double = app
        .last_click
        .map(|(c, r, at)| c == col && r == row && at.elapsed() < DOUBLE_CLICK)
        .unwrap_or(false);
    app.last_click = Some((col, row, Instant::now()));

    let inner = tree_inner(&l);
    if pt_in(inner, col, row) {
        if col == inner.right().saturating_sub(1) {
            app.dragging = Some(new_drag(DragMode::TreeScroll, col, 0.0));
            return false;
        }
        let k = app.tree_scroll + (row - inner.y) as usize;
        let nodes = app.tree_visible();
        if let Some(node) = nodes.get(k).copied() {
            app.tree_sel = k;
            app.focus = Focus::Tree;
            if is_double {
                match node {
                    TreeNode::Scope { id, .. } => app.toggle_scope(id),
                    TreeNode::Signal { sig, .. } => app.add_signal(sig),
                }
            }
        }
        return false;
    }

    if pt_in(l.list, col, row) {
        if row >= l.list.y + 2 {
            let row_in_list = (row - l.list.y - 2) as usize;
            if row_in_list < app.rows_h() && app.row_scroll + row_in_list < app.display.len() {
                app.sel_row = Some(app.row_scroll + row_in_list);
                app.focus = Focus::List;
            }
        }
        return false;
    }

    if pt_in(l.ruler, col, row) {
        app.cursor = app.tick_at_x(col);
        app.dragging = Some(new_drag(DragMode::Cursor, col, 0.0));
        app.focus = Focus::Wave;
        return false;
    }

    if col == l.vscroll_x && row >= l.rows.y && row < l.rows.bottom() {
        scroll_rows(app, &l, row);
        app.dragging = Some(new_drag(DragMode::VScroll, col, 0.0));
        return false;
    }

    if pt_in(l.rows, col, row) {
        app.cursor = app.tick_at_x(col);
        let row_in_wave = (row - l.rows.y) as usize;
        if row_in_wave < app.rows_h() && app.row_scroll + row_in_wave < app.display.len() {
            app.sel_row = Some(app.row_scroll + row_in_wave);
        }
        app.range = Some((app.cursor, app.cursor));
        app.dragging = Some(new_drag(DragMode::Range, col, 0.0));
        app.focus = Focus::Wave;
        return false;
    }

    if pt_in(l.hscroll, col, row) {
        pan_to_col(app, &l, col);
        app.dragging = Some(new_drag(DragMode::HScroll, col, 0.0));
        return false;
    }

    false
}

fn new_drag(mode: DragMode, col: u16, start_pct: f64) -> Drag {
    Drag {
        mode,
        start_x: col,
        start_pct,
    }
}

fn split_pct(app: &App, mode: DragMode) -> f64 {
    match mode {
        DragMode::SplitTree => app.splits.tree_pct as f64,
        DragMode::SplitList => app.splits.list_pct as f64,
        _ => 0.0,
    }
}

/// Return the pane border under the pointer, if any.
fn grip_at(l: &Layout, col: u16, row: u16) -> Option<DragMode> {
    if row < l.tree.y || row >= l.tree.bottom() {
        return None;
    }
    if col == l.tree_grip_x() {
        return Some(DragMode::SplitTree);
    }
    if col == l.list_grip_x() {
        return Some(DragMode::SplitList);
    }
    None
}

fn menu_click(app: &mut App, col: u16, row: u16, menu: usize) -> bool {
    let l = app.layout();
    let dropdown = menubar::dropdown_rect(&l, menu);
    if pt_in(dropdown, col, row) {
        if row > dropdown.y {
            let item = (row - dropdown.y - 1) as usize;
            if item < menubar::menu_len(menu) {
                let action = menubar::menu_action(menu, item);
                app.menu.open = None;
                return action.run(app);
            }
        }
        return false;
    }
    if pt_in(l.menu, col, row) {
        if let Some(i) = menubar::menu_item_at(l.menu, col) {
            if i == menu {
                app.menu.open = None;
            } else {
                app.menu.open = Some(i);
                app.menu.sel = 0;
            }
        }
        return false;
    }
    app.menu.open = None;
    false
}

fn mouse_drag(app: &mut App, col: u16, row: u16) {
    let Some(drag) = app.dragging else { return };
    let l = app.layout();
    match drag.mode {
        DragMode::Cursor => {
            app.cursor = app.tick_at_x(col);
            let span = app.span();
            if (app.cursor as f64) < app.t0 || (app.cursor as f64) > app.t0 + span {
                app.t0 = app.cursor as f64 - span * 0.4;
                app.clamp_view();
            }
        }
        DragMode::Range => {
            let t = app.tick_at_x(col);
            let anchor = app.range.map(|(a, _)| a).unwrap_or(t);
            app.range = Some((anchor.min(t), anchor.max(t)));
            app.cursor = t;
        }
        DragMode::VScroll => scroll_rows(app, &l, row),
        DragMode::TreeScroll => scroll_tree(app, &l, row),
        DragMode::HScroll => pan_to_col(app, &l, col),
        DragMode::SplitTree => {
            let delta = col as f64 - drag.start_x as f64;
            let pct = drag.start_pct + delta * 100.0 / l.area.width.max(1) as f64;
            app.splits.tree_pct =
                pct.clamp(Splits::MIN_PCT as f64, app.splits.max_tree_pct() as f64) as u16;
        }
        DragMode::SplitList => {
            let delta = col as f64 - drag.start_x as f64;
            let pct = drag.start_pct + delta * 100.0 / l.area.width.max(1) as f64;
            app.splits.list_pct =
                pct.clamp(Splits::MIN_PCT as f64, app.splits.max_list_pct() as f64) as u16;
        }
    }
}

fn scroll_rows(app: &mut App, l: &Layout, row: u16) {
    let total = app.display.len();
    let visible = l.rows_h;
    if visible == 0 || total <= visible {
        return;
    }
    let max_scroll = total - visible;
    let rel = row.saturating_sub(l.rows.y) as f64;
    let denom = visible.saturating_sub(1).max(1) as f64;
    app.row_scroll = (rel.min(denom) / denom * max_scroll as f64).round() as usize;
    app.row_scroll = app.row_scroll.min(max_scroll);
}

fn scroll_tree(app: &mut App, l: &Layout, row: u16) {
    let inner = tree_inner(l);
    let total = app.tree_visible().len();
    let visible = inner.height as usize;
    if visible == 0 || total <= visible {
        return;
    }
    let max_scroll = total - visible;
    let rel = row.saturating_sub(inner.y) as f64;
    let denom = visible.saturating_sub(1).max(1) as f64;
    app.tree_scroll = (rel.min(denom) / denom * max_scroll as f64).round() as usize;
    app.tree_scroll = app.tree_scroll.min(max_scroll);
}

fn pan_to_col(app: &mut App, l: &Layout, col: u16) {
    let (start, end) = match &app.wf {
        Some(wf) => (wf.start as f64, wf.end as f64),
        None => return,
    };
    if l.cols == 0 {
        return;
    }
    let span = l.cols as f64 * app.scale;
    let total = (end - start).max(0.0);
    if span >= total {
        app.t0 = start;
        return;
    }
    let rel = col.saturating_sub(l.rows.x) as f64;
    let denom = (l.cols.saturating_sub(1)).max(1) as f64;
    app.t0 = start + (rel.min(denom) / denom) * (total - span);
}

fn mouse_wheel(app: &mut App, col: u16, row: u16, up: bool, shift: bool) {
    let l = app.layout();

    if pt_in(l.tree, col, row) {
        let h = l.tree_height().max(1);
        app.tree_scroll = if up {
            app.tree_scroll.saturating_sub(WHEEL_STEP)
        } else {
            app.tree_scroll + WHEEL_STEP
        };
        let total = app.tree_visible().len();
        app.tree_scroll = app.tree_scroll.min(total.saturating_sub(h));
        app.tree_sel = app.tree_sel.min(total.saturating_sub(1));
        return;
    }

    if pt_in(l.rows, col, row) || pt_in(l.list, col, row) {
        let h = l.rows_h.max(1);
        app.row_scroll = if up {
            app.row_scroll.saturating_sub(WHEEL_STEP)
        } else {
            app.row_scroll + WHEEL_STEP
        };
        app.row_scroll = app.row_scroll.min(app.display.len().saturating_sub(h));
        return;
    }

    if pt_in(l.wave, col, row) {
        if shift {
            let step = (l.cols as f64 / 4.0) * app.scale;
            app.t0 += if up { -step } else { step };
            app.clamp_view();
        } else {
            let t = app.tick_at_x(col);
            app.zoom_at(t as f64, if up { 1.0 / 1.3 } else { 1.3 });
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::tests::app_with;
    use crate::app::{Focus, Splits, TreeNode};
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

    const VCD: &str = "$timescale 1ns $end\n\
        $var wire 1 ! clk $end\n\
        $enddefinitions $end\n\
        #0\n0!\n#10\n1!\n";

    fn click(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn drag(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn double_click_tree_adds_signal() {
        let mut app = app_with(VCD);
        assert!(matches!(
            app.tree_visible()[1],
            TreeNode::Signal { sig: 0, .. }
        ));
        // design is row 0 of the tree, clk is row 1 (screen y = 3 + 1).
        crate::app::handle_mouse(&mut app, click(2, 4));
        assert_eq!(app.focus, Focus::Tree);
        crate::app::handle_mouse(&mut app, click(2, 4));
        assert_eq!(app.display, vec![0]);
    }

    #[test]
    fn click_ruler_sets_cursor() {
        let mut app = app_with(VCD);
        let x = app.layout().wave.x + 10;
        crate::app::handle_mouse(&mut app, click(x, 2));
        assert_eq!(app.focus, Focus::Wave);
        assert!(app.cursor > 0);
        assert!(app.dragging.is_some());
    }

    #[test]
    fn drag_tree_border_resizes_panes() {
        let mut app = app_with(VCD);
        let l = app.layout();
        let before = l.tree.width;
        crate::app::handle_mouse(&mut app, click(l.tree_grip_x(), 5));
        crate::app::handle_mouse(&mut app, drag(l.tree_grip_x() + 10, 5));
        app.dragging = None;
        assert!(app.splits.tree_pct > Splits::default().tree_pct);
        assert!(app.layout().tree.width > before);
        assert!(app.layout().wave.width > 0);
    }

    #[test]
    fn drag_vertical_scrollbar_scrolls_rows() {
        let mut app = app_with(VCD);
        app.display = vec![0; 40];
        app.row_scroll = 0;
        let l = app.layout();
        crate::app::handle_mouse(&mut app, click(l.vscroll_x, l.rows.y + l.rows_h as u16 - 1));
        assert!(app.row_scroll > 0);
    }

    #[test]
    fn click_menu_item_runs_action() {
        let mut app = app_with(VCD);
        let l = app.layout();
        let help_x = crate::ui::menubar::dropdown_rect(&l, 4).x;
        crate::app::handle_mouse(&mut app, click(help_x + 2, l.menu.y));
        assert_eq!(app.menu.open, Some(4));
        let dropdown = crate::ui::menubar::dropdown_rect(&l, 4);
        crate::app::handle_mouse(&mut app, click(dropdown.x + 2, dropdown.y + 2));
        assert_eq!(app.menu.open, None);
        assert_eq!(app.dialog, Some(crate::app::Dialog::About));
    }
}
