use super::{App, CtxTarget, Dialog, Drag, DragMode, Focus, ListRow, TreeNode};
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
    // Some terminals (e.g. Windows Terminal) consume Shift+click for text
    // selection, so Alt+click is accepted as an alias for multi-select.
    let shift =
        m.modifiers.contains(KeyModifiers::SHIFT) || m.modifiers.contains(KeyModifiers::ALT);
    let ctrl = m.modifiers.contains(KeyModifiers::CONTROL);

    if let Some(dialog) = app.dialog {
        // While a dialog owns the focus, mouse input goes to it only.
        let l = app.layout();
        if m.kind == MouseEventKind::Down(MouseButton::Left)
            && pt_in(
                crate::ui::dialog::close_button(l.area, app, dialog),
                col,
                row,
            )
        {
            app.dialog = None;
            app.renaming_group = None;
            app.dragging = None;
            return false;
        }
        if matches!(m.kind, MouseEventKind::Up(_)) {
            app.dragging = None;
            return false;
        }
        if dialog == Dialog::Open {
            match m.kind {
                MouseEventKind::ScrollUp => browser_wheel(app, -(WHEEL_STEP as i64)),
                MouseEventKind::ScrollDown => browser_wheel(app, WHEEL_STEP as i64),
                MouseEventKind::Down(MouseButton::Left) => browser_click(app, col, row),
                _ => {}
            }
        } else {
            dialog_mouse(app, dialog, m);
        }
        return false;
    }

    if app.ctx_menu.is_some() {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                use crate::app::CtxEntry;
                use crate::ui::context::CtxHit;
                match crate::ui::context::item_at(app, col, row) {
                    Some(CtxHit::Root(i)) => match app.ctx_root().get(i).copied() {
                        Some(CtxEntry::Submenu(..)) => app.open_ctx_submenu(i),
                        Some(CtxEntry::Item(_, item)) => {
                            app.run_ctx_item(item);
                        }
                        None => app.ctx_menu = None,
                    },
                    Some(CtxHit::Sub(i)) => {
                        if let Some(CtxEntry::Item(_, item)) = app.ctx_level().get(i).copied() {
                            app.run_ctx_item(item);
                        }
                    }
                    None => app.ctx_menu = None,
                }
                return false;
            }
            MouseEventKind::Down(MouseButton::Right) => {
                // Close and fall through so the right-click can open a new menu.
                app.ctx_menu = None;
            }
            _ => return false,
        }
    }

    match m.kind {
        MouseEventKind::Down(btn) => mouse_down(app, col, row, btn, shift, ctrl),
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

fn browser_wheel(app: &mut App, delta: i64) {
    let rows = crate::ui::dialog::browser_rows(app.layout().area);
    if let Some(browser) = app.browser.as_mut() {
        browser.move_sel(delta, rows);
    }
}

/// Mouse events forwarded to a focused dialog: scrollbar drag and the wheel.
fn dialog_mouse(app: &mut App, dialog: Dialog, m: MouseEvent) {
    if let Some(area) = crate::ui::dialog::scroll_area(app.layout().area, app, dialog) {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if pt_in(area.bar, m.column, m.row) => {
                app.dragging = Some(new_drag(DragMode::DialogScroll, m.column, 0.0));
                dialog_scroll_to(app, dialog, m.row);
                return;
            }
            MouseEventKind::Drag(MouseButton::Left)
                if app
                    .dragging
                    .map(|drag| drag.mode == DragMode::DialogScroll)
                    .unwrap_or(false) =>
            {
                dialog_scroll_to(app, dialog, m.row);
                return;
            }
            _ => {}
        }
    }
    let delta: i64 = match m.kind {
        MouseEventKind::ScrollUp => -1,
        MouseEventKind::ScrollDown => 1,
        _ => return,
    };
    match dialog {
        Dialog::Keys => {
            let step = (WHEEL_STEP as i64) * delta;
            app.dialog_scroll = if step < 0 {
                app.dialog_scroll
                    .saturating_sub(step.unsigned_abs() as usize)
            } else {
                app.dialog_scroll.saturating_add(step as usize)
            };
        }
        Dialog::Find => {
            let count = app.find_matches().len();
            if delta < 0 {
                app.find_sel = app.find_sel.saturating_sub(1);
            } else {
                app.find_sel = (app.find_sel + 1).min(count.saturating_sub(1));
            }
        }
        Dialog::CreateBus => app.bus_builder_move(delta),
        _ => {}
    }
}

/// Map a row on a dialog's scrollbar to a scroll offset.
fn dialog_scroll_to(app: &mut App, dialog: Dialog, row: u16) {
    let Some(area) = crate::ui::dialog::scroll_area(app.layout().area, app, dialog) else {
        return;
    };
    let max = area.total.saturating_sub(area.rows);
    let denom = area.rows.saturating_sub(1).max(1) as f64;
    let rel = (row.saturating_sub(area.bar.y) as f64 / denom).clamp(0.0, 1.0);
    let scroll = (rel * max as f64).round() as usize;
    app.dialog_scroll = scroll;
    // List dialogs keep their selection inside the visible window.
    let bottom = (scroll + area.rows.saturating_sub(1)).min(area.total.saturating_sub(1));
    match dialog {
        Dialog::Find => app.find_sel = bottom,
        Dialog::CreateBus => app.bus_builder_select(bottom),
        _ => {}
    }
}

fn browser_click(app: &mut App, col: u16, row: u16) {
    let l = app.layout();
    let rect = crate::ui::dialog::open_rect(l.area);
    let list_top = rect.y + 3;
    let list_bottom = rect.bottom().saturating_sub(1);
    if col < rect.x || col >= rect.right() || row < list_top || row >= list_bottom {
        return;
    }
    let rows = (list_bottom - list_top) as usize;
    let index = app.browser.as_ref().map(|b| b.scroll).unwrap_or(0) + (row - list_top) as usize;

    let is_double = app
        .last_click
        .map(|(c, r, at)| c == col && r == row && at.elapsed() < DOUBLE_CLICK)
        .unwrap_or(false);
    app.last_click = Some((col, row, Instant::now()));

    if let Some(browser) = app.browser.as_mut() {
        browser.select(index, rows);
    }
    if is_double {
        if let Some(path) = app.browser.as_mut().and_then(|browser| browser.activate()) {
            app.load(&path.display().to_string());
        }
    }
}

fn mouse_down(
    app: &mut App,
    col: u16,
    row: u16,
    btn: MouseButton,
    shift: bool,
    ctrl: bool,
) -> bool {
    if let Some(dialog) = app.dialog {
        if dialog == Dialog::Open {
            browser_click(app, col, row);
        }
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
            app.ctx_menu = None;
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
            match node {
                TreeNode::Scope { id, .. } => {
                    if is_double {
                        app.toggle_scope(id);
                    }
                }
                TreeNode::Signal { sig, .. } => {
                    if shift {
                        app.toggle_tree_signal(sig);
                    } else if ctrl {
                        app.tree_select_range(sig);
                    } else if !is_double && !app.tree_multi.contains(&sig) {
                        app.tree_multi.clear();
                        app.tree_anchor = Some(sig);
                    }
                    if is_double {
                        if app.tree_multi.contains(&sig) {
                            app.add_tree_selection();
                        } else {
                            app.add_signal(sig);
                        }
                    }
                }
            }
        }
        return false;
    }

    if pt_in(l.list, col, row) {
        if row >= l.list.y + 2 {
            let row_in_list = (row - l.list.y - 2) as usize;
            let list_row = app
                .list_rows()
                .into_iter()
                .nth(app.row_scroll + row_in_list)
                .filter(|_| row_in_list < app.rows_h());
            if let Some(list_row) = list_row {
                let index = app.row_scroll + row_in_list;
                app.focus = Focus::List;
                if btn == MouseButton::Right {
                    let picked = matches!(&list_row, ListRow::Signal { sig, .. } if app.selection.contains(sig));
                    if picked {
                        app.sel_row = Some(index);
                    } else {
                        app.select_row(index);
                    }
                } else if shift {
                    app.toggle_row_selection(index);
                } else if ctrl {
                    app.select_range_to(index);
                } else {
                    app.select_row(index);
                }
                match (btn, list_row) {
                    (MouseButton::Right, ListRow::Signal { sig, .. }) => {
                        app.open_context_menu(CtxTarget::Signal(sig), col, row)
                    }
                    (MouseButton::Right, ListRow::Group { id, .. }) => {
                        app.open_context_menu(CtxTarget::Group(id), col, row)
                    }
                    (MouseButton::Left, ListRow::Signal { sig, .. }) if !shift && !ctrl => {
                        let from = app.display.iter().position(|&s| s == sig).unwrap_or(0);
                        app.dragging = Some(Drag {
                            mode: DragMode::Reorder,
                            start_x: col,
                            start_pct: 0.0,
                            row: from,
                        });
                    }
                    (MouseButton::Left, ListRow::Group { index, .. }) if is_double => {
                        app.toggle_group(index)
                    }
                    _ => {}
                }
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
        let row_in_wave = (row - l.rows.y) as usize;
        let list_row = app
            .list_rows()
            .into_iter()
            .nth(app.row_scroll + row_in_wave)
            .filter(|_| row_in_wave < app.rows_h());
        if list_row.is_some() && btn == MouseButton::Left && (shift || ctrl) {
            let index = app.row_scroll + row_in_wave;
            app.focus = Focus::Wave;
            if shift {
                app.toggle_row_selection(index);
            } else {
                app.select_range_to(index);
            }
            return false;
        }
        app.cursor = app.tick_at_x(col);
        if let Some(list_row) = list_row {
            let index = app.row_scroll + row_in_wave;
            if btn == MouseButton::Right {
                let picked =
                    matches!(&list_row, ListRow::Signal { sig, .. } if app.selection.contains(sig));
                if picked {
                    app.sel_row = Some(index);
                } else {
                    app.select_row(index);
                }
                app.focus = Focus::Wave;
                match list_row {
                    ListRow::Signal { sig, .. } => {
                        app.open_context_menu(CtxTarget::Signal(sig), col, row)
                    }
                    ListRow::Group { id, .. } => {
                        app.open_context_menu(CtxTarget::Group(id), col, row)
                    }
                }
                return false;
            }
            if matches!(list_row, ListRow::Group { .. }) {
                app.sel_row = Some(index);
                if is_double {
                    if let ListRow::Group { index, .. } = list_row {
                        app.toggle_group(index);
                    }
                }
                return false;
            }
            app.sel_row = Some(index);
        }
        if let Some((a, b)) = app.range {
            if b > a && app.cursor >= a && app.cursor <= b {
                app.zoom_to_range();
                app.range = None;
                app.focus = Focus::Wave;
                return false;
            }
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
        row: 0,
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
    if col == l.tree_grip_x() && row >= l.tree.y && row < l.tree.bottom() {
        return Some(DragMode::SplitTree);
    }
    if col == l.list_grip_x() && row >= l.list.y && row < l.list.bottom() {
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
        DragMode::Reorder => {
            let rows = app.list_rows();
            if rows.is_empty() {
                return;
            }
            let hover = (row.saturating_sub(l.list.y + 2) as usize).min(rows.len() - 1);
            // Drop before the nearest signal row (groups are not drop targets).
            let target = rows[hover..]
                .iter()
                .find_map(|r| match r {
                    ListRow::Signal { sig, .. } => Some(*sig),
                    _ => None,
                })
                .or_else(|| {
                    rows[..hover].iter().rev().find_map(|r| match r {
                        ListRow::Signal { sig, .. } => Some(*sig),
                        _ => None,
                    })
                });
            let Some(target_sig) = target else { return };
            let Some(target) = app.display.iter().position(|&s| s == target_sig) else {
                return;
            };
            let dragged = app.display[drag.row];
            if target != drag.row {
                app.move_signal(drag.row, target);
                if let Some(active) = app.dragging.as_mut() {
                    active.row = target;
                }
                app.scroll_to_row_of(dragged);
            }
        }
        DragMode::VScroll => scroll_rows(app, &l, row),
        DragMode::TreeScroll => scroll_tree(app, &l, row),
        DragMode::DialogScroll => {}
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
    let total = app.rows_len();
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
    if span >= total || total <= 0.0 {
        app.t0 = start;
        return;
    }
    let max_t0 = total - span;
    let thumb_w = ((span / total) * l.cols as f64).clamp(1.0, l.cols as f64);
    let denom = (l.cols as f64 - thumb_w).max(1.0);
    let rel = col.saturating_sub(l.rows.x) as f64;
    let pos = ((rel - thumb_w / 2.0) / denom).clamp(0.0, 1.0);
    app.t0 = start + pos * max_t0;
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

    if pt_in(l.list, col, row) {
        let h = l.rows_h.max(1);
        app.row_scroll = if up {
            app.row_scroll.saturating_sub(WHEEL_STEP)
        } else {
            app.row_scroll + WHEEL_STEP
        };
        app.row_scroll = app.row_scroll.min(app.rows_len().saturating_sub(h));
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
        $var wire 1 \" rst $end\n\
        $enddefinitions $end\n\
        #0\n0!\n0\"\n#10\n1!\n";

    fn click(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn click_with(col: u16, row: u16, modifiers: KeyModifiers) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers,
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
        crate::app::handle_mouse(&mut app, click(2, 3));
        assert_eq!(app.focus, Focus::Tree);
        crate::app::handle_mouse(&mut app, click(2, 3));
        assert_eq!(app.display, vec![0]);
    }

    #[test]
    fn click_ruler_sets_cursor() {
        let mut app = app_with(VCD);
        let l = app.layout();
        let x = l.rows.x + 10;
        crate::app::handle_mouse(&mut app, click(x, l.ruler.y));
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
    fn drag_list_row_reorders_signals() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.sel_row = Some(0);
        let l = app.layout();
        let top = l.list.y + 3;
        crate::app::handle_mouse(&mut app, click(30, top));
        crate::app::handle_mouse(&mut app, drag(30, top + 1));
        app.dragging = None;
        assert_eq!(app.display, vec![1, 0]);
        assert_eq!(app.sel_row, Some(2));
    }

    #[test]
    fn click_inside_selection_zooms_to_range() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        app.range = Some((2, 8));
        app.t0 = 0.0;
        app.scale = 1.0;
        let l = app.layout();
        let x = l.rows.x + 5; // tick 5.5, inside [2, 8]
        crate::app::handle_mouse(&mut app, click(x, l.rows.y + 1));
        assert!(app.range.is_none());
        let expected = 6.0 / l.cols as f64;
        assert!((app.scale - expected).abs() < 1e-9);
        assert!((app.t0 - 2.0).abs() < 1e-9);
    }

    #[test]
    fn context_menu_survives_button_release() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        let l = app.layout();
        let y = l.list.y + 2;
        let down = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 30,
            row: y,
            modifiers: KeyModifiers::NONE,
        };
        crate::app::handle_mouse(&mut app, down);
        assert!(app.ctx_menu.is_some());
        let up = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Right),
            column: 30,
            row: y,
            modifiers: KeyModifiers::NONE,
        };
        crate::app::handle_mouse(&mut app, up);
        assert!(app.ctx_menu.is_some());
    }

    #[test]
    fn wheel_over_waveform_zooms() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        let before = app.scale;
        let l = app.layout();
        let up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: l.rows.x + 5,
            row: l.rows.y,
            modifiers: KeyModifiers::NONE,
        };
        crate::app::handle_mouse(&mut app, up);
        assert!(app.scale < before);
        let down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: l.rows.x + 5,
            row: l.rows.y,
            modifiers: KeyModifiers::NONE,
        };
        crate::app::handle_mouse(&mut app, down);
        assert!((app.scale - before).abs() <= before * 1e-9);
    }

    #[test]
    fn drag_vertical_scrollbar_scrolls_rows() {
        let mut app = app_with(VCD);
        app.set_display(vec![0; 40]);
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

    #[test]
    fn shift_click_builds_multi_selection() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        let top = app.layout().list.y + 3;
        crate::app::handle_mouse(&mut app, click(30, top));
        assert!(app.selection.is_empty());
        assert_eq!(app.sel_anchor, Some(0));
        crate::app::handle_mouse(&mut app, click_with(30, top + 1, KeyModifiers::SHIFT));
        assert_eq!(app.selection, vec![1]);
        assert_eq!(app.sel_row, Some(2));
        crate::app::handle_mouse(&mut app, click_with(30, top, KeyModifiers::SHIFT));
        assert_eq!(app.selection, vec![1, 0]);
        // Toggling an already picked signal removes it again.
        crate::app::handle_mouse(&mut app, click_with(30, top, KeyModifiers::SHIFT));
        assert_eq!(app.selection, vec![1]);
    }

    #[test]
    fn dialog_mouse_events_do_not_reach_the_panes() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        let l = app.layout();
        let scale = app.scale;
        app.open_dialog(crate::app::Dialog::Keys);
        // A click on the waveform is ignored while the dialog owns the focus.
        crate::app::handle_mouse(&mut app, click(l.rows.x + 5, l.rows.y + 1));
        assert_eq!(app.cursor, 0);
        assert_eq!(app.scale, scale);
        assert_eq!(app.dialog, Some(crate::app::Dialog::Keys));
        // The wheel scrolls the dialog body instead.
        let scroll = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: l.rows.x + 5,
            row: l.rows.y + 1,
            modifiers: KeyModifiers::NONE,
        };
        crate::app::handle_mouse(&mut app, scroll);
        assert!(app.dialog_scroll > 0);
    }

    #[test]
    fn tree_multi_select_and_double_click_adds_all() {
        let mut app = app_with(VCD);
        let l = app.layout();
        // tree rows: design (0), clk (1), rst (2)
        let clk_y = l.tree.y + 2;
        let rst_y = l.tree.y + 3;
        crate::app::handle_mouse(&mut app, click_with(3, clk_y, KeyModifiers::SHIFT));
        crate::app::handle_mouse(&mut app, click_with(3, rst_y, KeyModifiers::SHIFT));
        assert_eq!(app.tree_multi, vec![0, 1]);
        crate::app::handle_mouse(&mut app, click(3, clk_y));
        crate::app::handle_mouse(&mut app, click(3, clk_y));
        assert_eq!(app.display, vec![0, 1]);
        assert!(app.tree_multi.is_empty());
    }

    #[test]
    fn dialog_close_button_closes_the_dialog() {
        let mut app = app_with(VCD);
        app.open_dialog(crate::app::Dialog::Keys);
        let l = app.layout();
        let close = crate::ui::dialog::close_button(l.area, &app, crate::app::Dialog::Keys);
        crate::app::handle_mouse(&mut app, click(close.x + 1, close.y));
        assert_eq!(app.dialog, None);
    }

    #[test]
    fn dialog_scrollbar_click_scrolls_the_body() {
        let mut app = app_with(VCD);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 60, 14));
        app.open_dialog(crate::app::Dialog::Keys);
        let l = app.layout();
        let area = crate::ui::dialog::scroll_area(l.area, &app, crate::app::Dialog::Keys).unwrap();
        crate::app::handle_mouse(
            &mut app,
            click(area.bar.x, area.bar.y + area.bar.height - 1),
        );
        assert!(app.dialog_scroll > 0);
        assert!(app.dragging.is_some());
    }

    #[test]
    fn double_click_group_toggles_collapse() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        let row = app.layout().list.y + 2; // G0 header
        crate::app::handle_mouse(&mut app, click(30, row));
        crate::app::handle_mouse(&mut app, click(30, row));
        assert!(app.groups[0].collapsed);
        app.last_click = None; // long enough pause for a new double click
        crate::app::handle_mouse(&mut app, click(30, row));
        crate::app::handle_mouse(&mut app, click(30, row));
        assert!(!app.groups[0].collapsed);
    }

    #[test]
    fn alt_click_toggles_multi_selection() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        let top = app.layout().list.y + 3;
        crate::app::handle_mouse(&mut app, click(30, top));
        crate::app::handle_mouse(&mut app, click_with(30, top + 1, KeyModifiers::ALT));
        assert_eq!(app.selection, vec![1]);
    }

    #[test]
    fn ctrl_click_selects_range_from_anchor() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        let top = app.layout().list.y + 3;
        crate::app::handle_mouse(&mut app, click(30, top));
        crate::app::handle_mouse(&mut app, click_with(30, top + 1, KeyModifiers::CONTROL));
        assert_eq!(app.selection, vec![0, 1]);
        assert_eq!(app.sel_row, Some(2));
        // A plain click starts a new single selection.
        crate::app::handle_mouse(&mut app, click(30, top + 1));
        assert!(app.selection.is_empty());
        assert_eq!(app.sel_row, Some(2));
    }
}
