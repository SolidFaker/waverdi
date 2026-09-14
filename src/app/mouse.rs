use super::{App, CtxTarget, Dialog, Drag, DragMode, Focus, ListRow, TreeNode};
use crate::ui::layout::{pt_in, tree_inner, Layout, Splits};
use crate::ui::menubar;
use crate::ui::source;
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
        if dialog != Dialog::AddSignals
            && m.kind == MouseEventKind::Down(MouseButton::Left)
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
        } else if dialog == Dialog::AddSignals {
            add_signals_mouse(app, m);
        } else {
            dialog_mouse(app, dialog, m);
        }
        return false;
    }

    if let Some(selected) = app.time_menu {
        let l = app.layout();
        let area = toolbar::time_menu_rect(l.toolbar, app);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if pt_in(area, col, row) {
                    // The box adds a border row above the entries; a click on
                    // the border (or below the last entry) selects nothing.
                    let index = row
                        .checked_sub(area.y + 1)
                        .map(|r| r as usize)
                        .filter(|&index| index < crate::waveform::TimeBase::CYCLE.len());
                    if let Some(index) = index {
                        app.set_time_base(crate::waveform::TimeBase::CYCLE[index]);
                    }
                } else {
                    app.time_menu = None;
                }
                return false;
            }
            MouseEventKind::ScrollUp => {
                app.time_menu = Some(
                    selected
                        .checked_sub(1)
                        .unwrap_or(crate::waveform::TimeBase::CYCLE.len() - 1),
                );
                return false;
            }
            MouseEventKind::ScrollDown => {
                app.time_menu = Some((selected + 1) % crate::waveform::TimeBase::CYCLE.len());
                return false;
            }
            _ => {}
        }
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
            if app
                .dragging
                .map(|drag| drag.mode == DragMode::SourceSel)
                .unwrap_or(false)
            {
                app.finish_source_selection();
            }
            // A drop may leave the newest group populated: only now add the
            // trailing empty group (never while dragging through groups).
            if app
                .dragging
                .map(|drag| matches!(drag.mode, DragMode::Reorder | DragMode::GroupReorder))
                .unwrap_or(false)
            {
                app.ensure_trailing_group();
            }
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
fn add_signals_mouse(app: &mut App, m: MouseEvent) {
    use crate::app::AddAction;

    match m.kind {
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let rows = if m.kind == MouseEventKind::ScrollUp {
                -(WHEEL_STEP as i64)
            } else {
                WHEEL_STEP as i64
            };
            if let Some(pane) = crate::ui::add::pane_at(app.last_area, m.column, m.row) {
                app.add_action(AddAction::Scroll { pane, rows });
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let Some(hit) = crate::ui::add::hit(app.last_area, app, m.column, m.row) else {
                return;
            };
            match hit {
                crate::ui::add::AddHit::Tree(node, arrow) => {
                    if arrow {
                        app.add_action(AddAction::ToggleNode(node));
                    } else {
                        app.add_action(AddAction::OpenScope(node));
                    }
                }
                crate::ui::add::AddHit::Instance(index) => {
                    app.add_action(AddAction::OpenInstance(index));
                }
                crate::ui::add::AddHit::Signal(signal) => {
                    app.add_action(AddAction::ToggleSignal(signal));
                }
                crate::ui::add::AddHit::Scrollbar(pane) => {
                    app.dragging = Some(new_drag(DragMode::AddScroll(pane), m.column, 0.0));
                    app.add_action(AddAction::ScrollTo { pane, row: m.row });
                }
                crate::ui::add::AddHit::Filter => app.add_action(AddAction::CycleFilter),
                crate::ui::add::AddHit::Apply => app.add_action(AddAction::Apply { close: false }),
                crate::ui::add::AddHit::Ok => app.add_action(AddAction::Apply { close: true }),
                crate::ui::add::AddHit::Cancel => app.add_action(AddAction::Close),
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(drag) = app.dragging {
                if let DragMode::AddScroll(pane) = drag.mode {
                    app.add_action(AddAction::ScrollTo { pane, row: m.row });
                }
            }
        }
        _ => {}
    }
}

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
            app.browser_load(&path.display().to_string());
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
        return match toolbar::tool_at(l.toolbar, app, col) {
            Some(tool) => toolbar::run_tool(app, tool),
            None => false,
        };
    }

    if btn == MouseButton::Left {
        if let Some(mode) = grip_at(&l, app.splits, col, row) {
            let (start, pct) = match mode {
                // Vertical splits remember the grabbed row instead of a column.
                DragMode::SplitTop => (row, app.splits.top_pct as f64),
                DragMode::SplitValue => (col, app.splits.value_pct as f64),
                _ => (col, split_pct(app, mode)),
            };
            app.dragging = Some(new_drag(mode, start, pct));
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
        let scrollbar = app.layout().tree_height();
        if col == inner.right().saturating_sub(1)
            && row > inner.y
            && app.tree_visible().len() > scrollbar
        {
            app.dragging = Some(new_drag(DragMode::TreeScroll, col, 0.0));
            return false;
        }
        let (_, sep_x, module_x) = crate::ui::tree::columns(inner, app.splits.hier_pct);
        // Bottom row: one horizontal scrollbar per column.
        if row == inner.bottom().saturating_sub(1) {
            if col < sep_x {
                scroll_tree_h(app, &l, col, false);
                app.dragging = Some(new_drag(DragMode::TreeHScroll, col, 0.0));
                app.focus = Focus::Tree;
            } else if col >= module_x && col < inner.right().saturating_sub(1) {
                scroll_tree_h(app, &l, col, true);
                app.dragging = Some(new_drag(DragMode::ModuleHScroll, col, 0.0));
                app.focus = Focus::Tree;
            }
            return false;
        }
        // The Hierarchy | Module divider resizes the columns.
        if col == sep_x {
            app.dragging = Some(Drag {
                mode: DragMode::SplitHier,
                start_x: col,
                start_pct: app.splits.hier_pct as f64,
                row: 0,
            });
            return false;
        }
        if row == inner.y {
            return false; // column header
        }
        // Keep the drawing and the click mapping in sync.
        app.clamp_tree_scroll();
        let k = app.tree_scroll + (row - inner.y - 1) as usize;
        let nodes = app.tree_visible();
        if let Some(node) = nodes.get(k).copied() {
            app.tree_sel = k;
            app.focus = Focus::Tree;
            if is_double {
                let TreeNode::Scope { id, .. } = node;
                app.toggle_scope(id);
                app.locate_scope_in_source();
            }
        }
        return false;
    }

    if pt_in(source::code_rect(&l), col, row) {
        let rect = source::code_rect(&l);
        // Right edge: draggable scrollbar when the file is longer than the pane.
        if let Some(view) = app.source_view.as_ref() {
            if source::scrollbar_col(&l, view) == Some(col) {
                scroll_source(app, &l, row);
                app.dragging = Some(new_drag(DragMode::SourceScroll, col, 0.0));
                app.focus = Focus::Source;
                return false;
            }
        }
        // Bottom row: horizontal scrollbar of the code text.
        if row == rect.bottom().saturating_sub(1) {
            scroll_source_h(app, &l, col);
            app.dragging = Some(new_drag(DragMode::SourceHScroll, col, 0.0));
            app.focus = Focus::Source;
            return false;
        }
        let line = app
            .source_view
            .as_ref()
            .map(|view| view.scroll + (row - rect.y) as usize);
        if let Some(index) = line {
            let gutter = app
                .source_view
                .as_ref()
                .map(source::gutter_width)
                .unwrap_or(0);
            let text_x = rect.x.saturating_add(gutter);
            let char_col = col.saturating_sub(text_x) as usize + app.source_h_scroll;
            app.focus = Focus::Source;
            match btn {
                MouseButton::Right => {
                    app.set_source_cursor(index, char_col);
                    app.open_context_menu(CtxTarget::Source, col, row);
                }
                MouseButton::Left if shift => {
                    // Shift+click extends the previous selection (or the
                    // cursor position) to the clicked character.
                    app.extend_source_selection_to(index, char_col);
                    app.dragging = Some(Drag {
                        mode: DragMode::SourceSel,
                        start_x: col,
                        start_pct: 0.0,
                        row: row as usize,
                    });
                }
                MouseButton::Left if is_double => {
                    app.set_source_cursor(index, char_col);
                    app.select_source_word();
                }
                MouseButton::Left => {
                    app.set_source_cursor(index, char_col);
                    app.begin_source_selection();
                    app.dragging = Some(Drag {
                        mode: DragMode::SourceSel,
                        start_x: col,
                        start_pct: 0.0,
                        row: row as usize,
                    });
                }
                _ => {}
            }
        }
        return false;
    }

    if pt_in(l.list, col, row) {
        // Bottom row: one horizontal scrollbar for the names and one for the
        // Value column.
        let value_w = l.value_col_width(app.splits.value_pct);
        if row == l.list.bottom().saturating_sub(1) {
            let grip = l.value_grip_x(app.splits.value_pct);
            if col > grip {
                scroll_list_value_h(app, &l, col);
                app.dragging = Some(new_drag(DragMode::ValueHScroll, col, 0.0));
                app.focus = Focus::List;
                return false;
            }
            if app.list_content_width() > crate::ui::list::name_width(&l, value_w) {
                scroll_list_h(app, &l, col);
                app.dragging = Some(new_drag(DragMode::ListHScroll, col, 0.0));
                app.focus = Focus::List;
                return false;
            }
        }
        if row >= l.list.y + 2 {
            // Rows may have been collapsed since the last scroll: re-clamp so
            // clicks map to the row that is actually drawn there.
            app.clamp_row_scroll();
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
                    // Keep a multi-selection when clicking one of its rows so
                    // the whole block can be dragged.
                    let picked = matches!(
                        &list_row,
                        ListRow::Signal { sig, .. }
                            if app.selection.len() > 1 && app.selection.contains(sig)
                    );
                    if picked {
                        app.sel_row = Some(index);
                    } else {
                        app.select_row(index);
                    }
                }
                match (btn, list_row) {
                    (MouseButton::Right, ListRow::Signal { sig, .. }) => {
                        app.open_context_menu(CtxTarget::Signal(sig), col, row)
                    }
                    (MouseButton::Right, ListRow::Group { id, .. }) => {
                        app.open_context_menu(CtxTarget::Group(id), col, row)
                    }
                    (MouseButton::Left, ListRow::Signal { sig, .. })
                        if is_double && !shift && !ctrl =>
                    {
                        app.toggle_signal_expand(sig)
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
                    (MouseButton::Left, ListRow::Group { index, .. })
                        if !shift && !ctrl && !is_double =>
                    {
                        app.dragging = Some(Drag {
                            mode: DragMode::GroupReorder,
                            start_x: col,
                            start_pct: 0.0,
                            row: index,
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
        if btn == MouseButton::Left && ctrl {
            app.focus = Focus::Wave;
            app.dragging = Some(Drag {
                mode: DragMode::Pan,
                start_x: col,
                start_pct: app.t0,
                row: 0,
            });
            return false;
        }
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
        app.clamp_row_scroll();
        let row_in_wave = (row - l.rows.y) as usize;
        let list_row = app
            .list_rows()
            .into_iter()
            .nth(app.row_scroll + row_in_wave)
            .filter(|_| row_in_wave < app.rows_h());
        // Ctrl+drag pans the time window; multi-selection stays a Signal List
        // gesture.
        if btn == MouseButton::Left && ctrl {
            app.focus = Focus::Wave;
            app.dragging = Some(Drag {
                mode: DragMode::Pan,
                start_x: col,
                start_pct: app.t0,
                row: 0,
            });
            return false;
        }
        if list_row.is_some() && btn == MouseButton::Left && shift {
            let index = app.row_scroll + row_in_wave;
            app.focus = Focus::Wave;
            app.toggle_row_selection(index);
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
            if is_double {
                if let ListRow::Signal { sig, .. } = list_row {
                    // Double-click on the waveform jumps to its driver logic.
                    app.jump_to_driver(sig);
                    return false;
                }
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
fn grip_at(l: &Layout, splits: Splits, col: u16, row: u16) -> Option<DragMode> {
    if row == l.split_grip_y() && col >= l.area.x && col < l.area.right() {
        return Some(DragMode::SplitTop);
    }
    if col == l.tree_grip_x() && row >= l.tree.y && row < l.tree.bottom() {
        return Some(DragMode::SplitTree);
    }
    if col == l.list_grip_x() && row >= l.list.y && row < l.list.bottom() {
        return Some(DragMode::SplitList);
    }
    if col == l.value_grip_x(splits.value_pct) && row >= l.list.y + 2 && row < l.list.bottom() {
        return Some(DragMode::SplitValue);
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
        DragMode::Pan => {
            // Drag the time window with the pointer: moving right shows
            // earlier time, moving left shows later time.
            let dx = col as i64 - drag.start_x as i64;
            app.t0 = drag.start_pct - dx as f64 * app.scale;
            app.clamp_view();
        }
        DragMode::Reorder => {
            let rows = app.list_rows();
            if rows.is_empty() || drag.row >= app.display.len() {
                return;
            }
            // The pointer row maps to a row of the full list, not the viewport.
            let hover =
                (app.row_scroll + row.saturating_sub(l.list.y + 2) as usize).min(rows.len() - 1);
            let dragged = app.display[drag.row];
            if app.selection.len() > 1 && app.selection.contains(&dragged) {
                // Move the whole multi-selection one step toward the pointer.
                match rows[hover] {
                    ListRow::Group { index, .. } => {
                        let group = app.group_of_signal(dragged).unwrap_or(0);
                        if group != index {
                            app.move_selection(if index > group { 1 } else { -1 });
                        }
                    }
                    ListRow::Signal { sig, .. } => {
                        let Some(hover_pos) = app.display.iter().position(|&s| s == sig) else {
                            return;
                        };
                        let selected: Vec<usize> = app
                            .display
                            .iter()
                            .enumerate()
                            .filter(|(_, signal)| app.selection.contains(signal))
                            .map(|(position, _)| position)
                            .collect();
                        let (Some(&first), Some(&last)) = (selected.first(), selected.last())
                        else {
                            return;
                        };
                        if hover_pos > drag.row && last < hover_pos {
                            app.move_selection(1);
                        } else if hover_pos < drag.row && first > hover_pos {
                            app.move_selection(-1);
                        }
                    }
                }
                if let Some(position) = app.display.iter().position(|&s| s == dragged) {
                    if let Some(active) = app.dragging.as_mut() {
                        active.row = position;
                    }
                    app.sel_row = app.list_rows().iter().position(
                        |row| matches!(row, ListRow::Signal { sig, .. } if *sig == dragged),
                    );
                }
                return;
            }
            // Move one slot toward the hovered row: no long jumps and no
            // auto-scroll, so dragging across expanded arrays stays stable.
            let to = match rows[hover] {
                ListRow::Group { index, .. } => {
                    let group = app.group_of_signal(dragged).unwrap_or(0);
                    if group == index {
                        return;
                    }
                    let start = app.group_range(index).start;
                    if start >= app.display.len() {
                        if drag.row + 1 == app.display.len() {
                            // The target group is empty and sits after the
                            // last row: hand the signal over without moving.
                            app.groups[group].count = app.groups[group].count.saturating_sub(1);
                            app.groups[index].count += 1;
                            return;
                        }
                        // Step toward the end first, then hand over.
                        drag.row + 1
                    } else if start > drag.row {
                        drag.row + 1
                    } else {
                        drag.row.saturating_sub(1)
                    }
                }
                ListRow::Signal { sig, .. } => {
                    let Some(pos) = app.display.iter().position(|&s| s == sig) else {
                        return;
                    };
                    if pos == drag.row {
                        return;
                    }
                    if pos > drag.row {
                        drag.row + 1
                    } else {
                        drag.row.saturating_sub(1)
                    }
                }
            };
            if to != drag.row && to < app.display.len() {
                app.move_signal(drag.row, to);
                if let Some(active) = app.dragging.as_mut() {
                    active.row = to;
                }
                app.sel_row = app
                    .list_rows()
                    .iter()
                    .position(|row| matches!(row, ListRow::Signal { sig, .. } if *sig == dragged));
            }
        }
        DragMode::GroupReorder => {
            // The pointer may sit above the first row (list title) or below
            // the last one; the rules are:
            //  - up:   past the previous group's name row,
            //  - down: past the next group's last signal row.
            let first_row = (l.list.y + 2) as i64;
            let hover = row as i64 - first_row;
            let mut current = app.dragging.map(|drag| drag.row).unwrap_or(0);
            loop {
                let rows = app.list_rows();
                let spans = group_row_spans(&rows);
                if current >= spans.len() {
                    break;
                }
                let delta = if current > 0 && hover < spans[current - 1].0 as i64 {
                    -1
                } else if current + 1 < spans.len() && hover > spans[current + 1].1 as i64 {
                    1
                } else {
                    break;
                };
                app.move_group(current, delta);
                current = (current as i64 + delta).max(0) as usize;
                if let Some(active) = app.dragging.as_mut() {
                    active.row = current;
                }
            }
        }
        DragMode::VScroll => scroll_rows(app, &l, row),
        DragMode::TreeScroll => scroll_tree(app, &l, row),
        DragMode::SourceScroll => scroll_source(app, &l, row),
        DragMode::SourceHScroll => scroll_source_h(app, &l, col),
        DragMode::ListHScroll => scroll_list_h(app, &l, col),
        DragMode::ValueHScroll => scroll_list_value_h(app, &l, col),
        DragMode::TreeHScroll => scroll_tree_h(app, &l, col, false),
        DragMode::ModuleHScroll => scroll_tree_h(app, &l, col, true),
        DragMode::SplitHier => {
            let delta = col as f64 - drag.start_x as f64;
            let inner_w = crate::ui::layout::tree_inner(&l).width.max(1) as f64;
            let pct = drag.start_pct + delta * 100.0 / inner_w;
            app.splits.hier_pct =
                pct.clamp(Splits::MIN_HIER_PCT as f64, Splits::MAX_HIER_PCT as f64) as u16;
        }
        DragMode::DialogScroll => {}
        DragMode::AddScroll(_) => {}
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
        DragMode::SplitTop => {
            let delta = row as f64 - drag.start_x as f64;
            let pct = drag.start_pct + delta * 100.0 / l.area.height.max(1) as f64;
            app.splits.top_pct =
                pct.clamp(Splits::MIN_TOP_PCT as f64, Splits::MAX_TOP_PCT as f64) as u16;
        }
        DragMode::SplitValue => {
            // Dragging the grip left widens the Value column.
            let list_w = l.list.width.max(1) as f64;
            let delta = col as f64 - drag.start_x as f64;
            let pct = drag.start_pct - delta * 100.0 / list_w;
            app.splits.value_pct =
                pct.clamp(Splits::MIN_VALUE_PCT as f64, Splits::MAX_VALUE_PCT as f64) as u16;
        }
        DragMode::SourceSel => {
            let rect = source::code_rect(&l);
            let target = app.source_view.as_ref().map(|view| {
                let line = view.scroll + row.saturating_sub(rect.y) as usize;
                let gutter = source::gutter_width(view);
                (line, col.saturating_sub(rect.x + gutter) as usize)
            });
            if let Some((line, char_col)) = target {
                app.extend_source_selection_to(line, char_col);
            }
            if let Some(active) = app.dragging.as_mut() {
                active.row = row as usize;
            }
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
    // Rows live between the column header and the bottom scrollbar row.
    let visible = l.tree_height();
    if visible == 0 || total <= visible {
        return;
    }
    let max_scroll = total - visible;
    let rel = row.saturating_sub(inner.y + 1) as f64;
    let denom = visible.saturating_sub(1).max(1) as f64;
    app.tree_scroll = (rel.min(denom) / denom * max_scroll as f64).round() as usize;
    app.tree_scroll = app.tree_scroll.min(max_scroll);
}

/// Horizontal scroll of the Signal List names.
fn scroll_list_h(app: &mut App, l: &Layout, col: u16) {
    let value_w = l.value_col_width(app.splits.value_pct);
    let name_w = crate::ui::list::name_width(l, value_w);
    let content = app.list_content_width();
    let max = content.saturating_sub(name_w);
    if max == 0 {
        return;
    }
    let grip = l.value_grip_x(app.splits.value_pct);
    let end = grip.saturating_sub(1);
    let span = end.saturating_sub(l.list.x).max(1) as f64;
    let rel = col.saturating_sub(l.list.x) as f64 / span;
    app.list_h_scroll = (rel.min(1.0) * max as f64).round() as usize;
}

/// Horizontal scroll of the Value column.
fn scroll_list_value_h(app: &mut App, l: &Layout, col: u16) {
    let value_w = l.value_col_width(app.splits.value_pct);
    let content = app.value_content_width();
    let max = content.saturating_sub(value_w);
    if max == 0 {
        return;
    }
    let x0 = l.value_grip_x(app.splits.value_pct).saturating_add(1);
    let x1 = l.list.right().saturating_sub(1).max(x0 + 1);
    let end = x1.saturating_sub(1);
    let span = end.saturating_sub(x0).max(1) as f64;
    let rel = col.saturating_sub(x0) as f64 / span;
    app.value_h_scroll = (rel.min(1.0) * max as f64).round() as usize;
}

/// Horizontal scroll of the Instance pane columns (`module` selects the
/// Module column, otherwise the Hierarchy column).
fn scroll_tree_h(app: &mut App, l: &Layout, col: u16, module: bool) {
    let inner = crate::ui::layout::tree_inner(l);
    let (hier_w, sep_x, module_x) = crate::ui::tree::columns(inner, app.splits.hier_pct);
    let (x0, x1, content, view) = if module {
        let right = inner.right().saturating_sub(1);
        let view = right.saturating_sub(module_x) as usize;
        (module_x, right, app.module_content_width(), view)
    } else {
        let view = hier_w.saturating_sub(1) as usize;
        (inner.x, sep_x, app.tree_content_width(), view)
    };
    let max = content.saturating_sub(view);
    if max == 0 || x1 <= x0 {
        return;
    }
    let span = x1.saturating_sub(x0).max(1) as f64;
    let rel = col.saturating_sub(x0) as f64 / span;
    let offset = (rel.min(1.0) * max as f64).round() as usize;
    if module {
        app.module_h_scroll = offset;
    } else {
        app.tree_h_scroll = offset;
    }
}

/// Row span `(header, last)` of every group in the flattened list rows.
fn group_row_spans(rows: &[ListRow]) -> Vec<(usize, usize)> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (row, entry) in rows.iter().enumerate() {
        match entry {
            ListRow::Group { index, .. } => {
                while spans.len() <= *index {
                    spans.push((row, row));
                }
                spans[*index] = (row, row);
            }
            ListRow::Signal { .. } => {
                if let Some(last) = spans.last_mut() {
                    last.1 = row;
                }
            }
        }
    }
    spans
}

fn scroll_source(app: &mut App, l: &Layout, row: u16) {
    let code = crate::ui::source::code_rect(l);
    let Some(view) = app.source_view.as_ref() else {
        return;
    };
    let total = view.lines.len();
    // The bottom row is the horizontal scrollbar.
    let visible = code.height.saturating_sub(1) as usize;
    if visible == 0 || total <= visible {
        return;
    }
    let max_scroll = total - visible;
    let rel = row.saturating_sub(code.y) as f64;
    let denom = visible.saturating_sub(1).max(1) as f64;
    let scroll = (rel.min(denom) / denom * max_scroll as f64).round() as usize;
    if let Some(view) = app.source_view.as_mut() {
        view.scroll = scroll.min(max_scroll);
    }
}

/// Horizontal scroll of the Source pane.
fn scroll_source_h(app: &mut App, l: &Layout, col: u16) {
    let code = crate::ui::source::code_rect(l);
    let Some(view) = app.source_view.as_ref() else {
        return;
    };
    let gutter = crate::ui::source::gutter_width(view);
    let vbar = crate::ui::source::scrollbar_col(l, view).is_some();
    let text_x0 = code.x + gutter;
    let text_w = (code.width as usize).saturating_sub(gutter as usize + usize::from(vbar));
    let content = view
        .lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let max = content.saturating_sub(text_w);
    if max == 0 {
        return;
    }
    let x1 = code
        .right()
        .saturating_sub(u16::from(vbar))
        .max(text_x0 + 1);
    let end = x1.saturating_sub(1);
    let span = end.saturating_sub(text_x0).max(1) as f64;
    let rel = col.saturating_sub(text_x0) as f64 / span;
    app.source_h_scroll = (rel.min(1.0) * max as f64).round() as usize;
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

    if pt_in(l.source, col, row) {
        app.scroll_source(if up {
            -(WHEEL_STEP as i64)
        } else {
            WHEEL_STEP as i64
        });
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
    use crate::app::{Focus, Splits};
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
    fn double_click_tree_toggles_the_scope() {
        let mut app = app_with(VCD);
        let l = app.layout();
        let row = l.tree.y + 2; // border, column header, first node
        crate::app::handle_mouse(&mut app, click(2, row));
        assert_eq!(app.focus, Focus::Tree);
        let expanded = app.expanded.contains(&0);
        crate::app::handle_mouse(&mut app, click(2, row));
        assert_eq!(app.expanded.contains(&0), !expanded);
        assert!(app.display.is_empty());
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
    fn ctrl_drag_in_the_waveform_pans_the_window() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        let mid = app.wf.as_ref().unwrap().total_ticks() / 2;
        app.cursor = mid;
        app.zoom_in();
        app.zoom_in();
        let l = app.layout();
        let x = l.rows.x + 30;
        let y = l.rows.y + 1;
        crate::app::handle_mouse(&mut app, click_with(x, y, KeyModifiers::CONTROL));
        assert!(app.dragging.is_some());
        let before = app.t0;
        crate::app::handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: x + 5,
                row: y,
                modifiers: KeyModifiers::CONTROL,
            },
        );
        assert!(
            app.t0 < before,
            "pan did not move the window: {before} -> {}",
            app.t0
        );
        crate::app::handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: x + 5,
                row: y,
                modifiers: KeyModifiers::CONTROL,
            },
        );
        assert!(app.dragging.is_none());
    }

    #[test]
    fn drag_value_grip_resizes_the_value_column() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        let l = app.layout();
        let x = l.value_grip_x(app.splits.value_pct);
        let y = l.list.y + 2;
        crate::app::handle_mouse(&mut app, click(x, y));
        crate::app::handle_mouse(&mut app, drag(x - 4, y));
        app.dragging = None;
        assert!(app.splits.value_pct > Splits::default().value_pct);
    }

    #[test]
    fn source_click_maps_to_the_code_column() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $scope module dut $end\n\
            $var wire 4 ! count [3:0] $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\nb0000 !\n";
        let mut app = app_with(vcd);
        let dir = std::env::temp_dir().join(format!("waverdi_src_click_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("counter.sv");
        std::fs::write(
            &path,
            "module counter(output logic [3:0] count);\n\
             assign count = 4'b0;\n\
             endmodule\n",
        )
        .unwrap();
        app.sources = Some(SourceSet::from_files(vec![path], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[2].module = "counter".to_string();
        app.expanded.insert(1);
        app.tree_sel = 2;
        app.sync_source();

        let l = app.layout();
        let rect = crate::ui::source::code_rect(&l);
        let view = app.source_view.as_ref().unwrap();
        let line = 0usize;
        let col = view.lines[line].rfind("count").unwrap();
        let gutter = crate::ui::source::gutter_width(view);
        let x = rect.x + gutter + col as u16;
        let y = rect.y + line as u16;
        crate::app::handle_mouse(&mut app, click(x, y));
        let view = app.source_view.as_ref().unwrap();
        assert_eq!(view.line, line);
        assert_eq!(view.col, col);
        assert_eq!(view.word_at_cursor().as_deref(), Some("count"));

        // Releasing without dragging selects only the clicked signal word.
        let up = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        };
        crate::app::handle_mouse(&mut app, up);
        assert_eq!(app.source_selection_signals().0, vec![0]);
    }

    #[test]
    fn drag_top_divider_resizes_the_areas() {
        let mut app = app_with(VCD);
        let l = app.layout();
        let before = l.nwave.y;
        let y = l.split_grip_y();
        crate::app::handle_mouse(&mut app, click(50, y));
        crate::app::handle_mouse(&mut app, drag(50, y + 5));
        app.dragging = None;
        assert!(app.splits.top_pct > Splits::default().top_pct);
        assert!(app.layout().nwave.y > before);
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
    fn toolbar_time_base_dropdown_selects_a_base() {
        let mut app = app_with(VCD);
        let l = app.layout();
        let button = crate::ui::toolbar::time_button(l.toolbar, &app);
        crate::app::handle_mouse(&mut app, click(button.x + 1, button.y));
        assert_eq!(app.time_menu, Some(0));
        let area = crate::ui::toolbar::time_menu_rect(l.toolbar, &app);
        // Entry 0 is the timescale, entry 1 is fs.
        crate::app::handle_mouse(&mut app, click(area.x + 1, area.y + 2));
        assert_eq!(app.time_base, crate::waveform::TimeBase::Fs);
        assert!(app.time_menu.is_none());
    }

    #[test]
    fn time_menu_border_clicks_are_ignored() {
        let mut app = app_with(VCD);
        let l = app.layout();
        let button = crate::ui::toolbar::time_button(l.toolbar, &app);
        crate::app::handle_mouse(&mut app, click(button.x + 1, button.y));
        let area = crate::ui::toolbar::time_menu_rect(l.toolbar, &app);
        app.time_menu = Some(1);
        let before = app.time_base;
        // The top border row must not underflow into an entry index.
        crate::app::handle_mouse(&mut app, click(area.x + 1, area.y));
        assert_eq!(app.time_menu, Some(1));
        assert_eq!(app.time_base, before);
        // The bottom border row is below the last entry.
        crate::app::handle_mouse(&mut app, click(area.x + 1, area.bottom() - 1));
        assert_eq!(app.time_base, before);
    }

    #[test]
    fn reorder_while_scrolled_keeps_the_view() {
        let mut vcd = String::from("$timescale 1ns $end\n");
        let mut body = String::new();
        for i in 0..20u8 {
            let id = (b'!' + i) as char;
            vcd.push_str(&format!("$var wire 1 {id} s{i} $end\n"));
            body.push_str(&format!("0{id}\n"));
        }
        vcd.push_str("$enddefinitions $end\n#0\n");
        vcd.push_str(&body);
        let mut app = app_with(&vcd);
        app.set_display((0..20).collect());
        app.row_scroll = app.rows_len().saturating_sub(app.rows_h());
        let bottom = app.row_scroll;
        assert!(bottom >= 2);
        let l = app.layout();
        let y = l.list.y + 2; // first visible row
        crate::app::handle_mouse(&mut app, click(30, y));
        crate::app::handle_mouse(&mut app, drag(30, y + 1));
        app.dragging = None;
        assert_eq!(app.row_scroll, bottom, "the list must not jump to the top");
        assert_eq!(app.display[bottom - 1], bottom);
        assert_eq!(app.display[bottom], bottom - 1);
    }

    #[test]
    fn double_click_signal_expands_and_collapses_bits() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 4 ! bus [3:0] $end\n\
            $enddefinitions $end\n#0\nb0000 !\n";
        let mut app = app_with(vcd);
        app.set_display(vec![0]);
        let row = app.layout().list.y + 3; // the signal row (after the G0 header)
        crate::app::handle_mouse(&mut app, click(30, row));
        crate::app::handle_mouse(&mut app, click(30, row));
        assert_eq!(app.display, vec![0, 1, 2, 3, 4]);
        assert_eq!(app.wf.as_ref().unwrap().signals[1].name, "bus[0]");
        assert_eq!(app.wf.as_ref().unwrap().signals[4].name, "bus[3]");
        assert_eq!(app.wf.as_ref().unwrap().signals[1].parent, Some(0));
        assert_eq!(app.wf.as_ref().unwrap().signals[4].parent, Some(0));
        app.dragging = None;
        app.last_click = None; // long enough pause for a new double click
        crate::app::handle_mouse(&mut app, click(30, row));
        crate::app::handle_mouse(&mut app, click(30, row));
        assert_eq!(app.display, vec![0]);
        // Expanding again reuses the already created bit signals.
        app.dragging = None;
        app.last_click = None;
        crate::app::handle_mouse(&mut app, click(30, row));
        crate::app::handle_mouse(&mut app, click(30, row));
        assert_eq!(app.display, vec![0, 1, 2, 3, 4]);
        assert_eq!(app.wf.as_ref().unwrap().signals.len(), 5);
    }

    #[test]
    fn source_scrollbar_drag_scrolls_the_file() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let mut app = app_with(vcd);
        let dir = std::env::temp_dir().join(format!("waverdi_src_bar_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("long.sv");
        let mut text = String::from("module long(input logic clk);\n");
        for line in 0..80 {
            text.push_str(&format!("    assign w{line} = clk;\n"));
        }
        text.push_str("endmodule\n");
        std::fs::write(&file, text).unwrap();
        app.sources = Some(SourceSet::from_files(vec![file], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[1].module = "long".to_string();
        app.tree_sel = 1;
        app.sync_source();

        let l = app.layout();
        let view = app.source_view.as_ref().unwrap();
        let col = crate::ui::source::scrollbar_col(&l, view).expect("scrollbar");
        let code = crate::ui::source::code_rect(&l);
        crate::app::handle_mouse(&mut app, click(col, code.y));
        assert!(matches!(
            app.dragging.map(|drag| drag.mode),
            Some(crate::app::DragMode::SourceScroll)
        ));
        crate::app::handle_mouse(&mut app, drag(col, code.y + code.height.saturating_sub(1)));
        assert!(app.source_view.as_ref().unwrap().scroll > 0);
        app.dragging = None;
    }

    #[test]
    fn double_click_expands_array_dimensions_step_by_step() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 8 ! e0 [7:0] $end\n\
            $var wire 8 \" e1 [7:0] $end\n\
            $var wire 8 # e2 [7:0] $end\n\
            $var wire 8 $ e3 [7:0] $end\n\
            $enddefinitions $end\n#0\nb0 !\nb0 \"\nb0 #\nb0 $\n";
        let mut app = app_with(vcd);
        {
            let signals = &mut app.wf.as_mut().unwrap().signals;
            signals[0].name = "arr[0][0][7:0]".to_string();
            signals[1].name = "arr[0][1][7:0]".to_string();
            signals[2].name = "arr[1][0][7:0]".to_string();
            signals[3].name = "arr[1][1][7:0]".to_string();
        }
        app.wf.as_mut().unwrap().build_arrays();
        // arr[0] = 4, arr[1] = 5, arr = 6.
        app.set_display(vec![6]);
        let row = app.layout().list.y + 3;
        double_click(&mut app, row);
        assert_eq!(app.display, vec![6, 4, 5]);

        // Expanding arr[0] reveals its two elements.
        let row0 = app.layout().list.y + 4;
        double_click(&mut app, row0);
        assert_eq!(app.display, vec![6, 4, 0, 1, 5]);

        // Collapsing arr[0] removes only its subtree.
        double_click(&mut app, row0);
        assert_eq!(app.display, vec![6, 4, 5]);

        // Collapsing the root removes every dimension below it.
        double_click(&mut app, row);
        assert_eq!(app.display, vec![6]);
    }

    fn double_click(app: &mut crate::app::App, row: u16) {
        crate::app::handle_mouse(app, click(30, row));
        crate::app::handle_mouse(app, click(30, row));
        app.dragging = None;
        app.last_click = None; // long enough pause for the next double click
    }

    #[test]
    fn drag_signal_into_an_empty_group() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.grow_groups(0); // G1 stays empty
        assert_eq!(app.groups.len(), 2);
        assert_eq!(app.groups[1].count, 0);
        let l = app.layout();
        let src_row = l.list.y + 3; // first signal
        let dst_row = l.list.y + 5; // G1 header (G0, s0, s1, G1)
        crate::app::handle_mouse(&mut app, click(30, src_row));
        for _ in 0..6 {
            crate::app::handle_mouse(&mut app, drag(30, dst_row));
        }
        app.dragging = None;
        assert_eq!(app.groups[0].count, 1);
        assert_eq!(
            app.groups[1].count, 1,
            "the empty group must accept the drop"
        );
        assert_eq!(app.group_of_signal(0), Some(1));
        assert_eq!(app.display, vec![1, 0]);

        // Dropping back onto G0 restores the original order.
        app.sel_row = None;
        let dst_row = l.list.y + 2; // G0 header
        crate::app::handle_mouse(&mut app, click(30, l.list.y + 5)); // sig 0 in G1
        for _ in 0..6 {
            crate::app::handle_mouse(&mut app, drag(30, dst_row));
        }
        app.dragging = None;
        assert_eq!(app.groups[0].count, 2);
        assert_eq!(app.groups[1].count, 0);
        assert_eq!(app.group_of_signal(0), Some(0));
        assert_eq!(app.display, vec![0, 1]);
    }

    #[test]
    fn drag_group_header_reorders_groups() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.grow_groups(0);
        app.move_signal_to(1, 1, 2); // signal 1 joins the empty G1
        assert_eq!(app.display, vec![0, 1]);
        let l = app.layout();
        let g1_row = l.list.y + 4; // G0, s0, G1, s1
        crate::app::handle_mouse(&mut app, click(30, g1_row));
        // Above the previous group's name row (the list title strip) moves up.
        crate::app::handle_mouse(&mut app, drag(30, l.list.y + 1));
        app.dragging = None;
        assert_eq!(app.groups[0].id, 1);
        assert_eq!(app.groups[1].id, 0);
        assert_eq!(app.display, vec![1, 0]);
    }

    #[test]
    fn drag_group_down_moves_past_the_next_groups_last_signal() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.grow_groups(0);
        app.move_signal_to(1, 1, 2); // G0 = [0], G1 = [1]
        let l = app.layout();
        let g0_row = l.list.y + 2; // G0, s0, G1, s1
        crate::app::handle_mouse(&mut app, click(30, g0_row));
        // Hovering over G1's own rows is not enough.
        crate::app::handle_mouse(&mut app, drag(30, l.list.y + 4)); // G1 header
        crate::app::handle_mouse(&mut app, drag(30, l.list.y + 5)); // G1 signal
        assert_eq!(
            app.groups[0].id, 0,
            "must not move before passing the last signal"
        );
        // Past G1's last signal row moves the group down.
        crate::app::handle_mouse(&mut app, drag(30, l.list.y + 6));
        app.dragging = None;
        assert_eq!(app.groups[0].id, 1);
        assert_eq!(app.groups[1].id, 0);
        assert_eq!(app.display, vec![1, 0]);
    }

    #[test]
    fn double_click_wave_row_jumps_to_the_driver() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $var wire 8 ! data [7:0] $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\nb00000000 !\n";
        let mut app = app_with(vcd);
        let dir = std::env::temp_dir().join(format!("waverdi_wave_jump_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tb.sv");
        std::fs::write(
            &file,
            "module tb;\n\
             \x20   logic [7:0] data;\n\
             \x20   assign data = 8'h00;\n\
             endmodule\n",
        )
        .unwrap();
        app.sources = Some(SourceSet::from_files(vec![file], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[1].module = "tb".to_string();
        app.tree_sel = 1;
        app.sync_source();
        app.set_display(vec![0]);

        let l = app.layout();
        let y = l.rows.y + 1; // G0 header is the first row, the signal follows
        crate::app::handle_mouse(&mut app, click(l.rows.x + 5, y));
        crate::app::handle_mouse(&mut app, click(l.rows.x + 5, y));
        assert_eq!(app.focus, Focus::Source);
        assert_eq!(app.source_view.as_ref().unwrap().module, "tb");
        // The assign driver is on line 3 (0-based 2).
        assert_eq!(app.source_view.as_ref().unwrap().line, 2);
    }

    #[test]
    fn source_drag_autoscrolls_at_the_pane_edges() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let mut app = app_with(vcd);
        let dir = std::env::temp_dir().join(format!("waverdi_src_as_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("long.sv");
        let mut text = String::from("module long(input logic clk);\n");
        for line in 0..80 {
            text.push_str(&format!("    assign w{line} = clk;\n"));
        }
        text.push_str("endmodule\n");
        std::fs::write(&file, text).unwrap();
        app.sources = Some(SourceSet::from_files(vec![file], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[1].module = "long".to_string();
        app.tree_sel = 1;
        app.sync_source();

        let l = app.layout();
        let rect = crate::ui::source::code_rect(&l);
        let x = rect.x + 6;
        crate::app::handle_mouse(&mut app, click(x, rect.y + 1));
        crate::app::handle_mouse(&mut app, drag(x, rect.bottom() + 2));
        assert!(matches!(
            app.dragging.map(|drag| drag.mode),
            Some(crate::app::DragMode::SourceSel)
        ));
        let before = app.source_view.as_ref().unwrap().scroll;
        for _ in 0..3 {
            app.tick_source_drag();
        }
        let view = app.source_view.as_ref().unwrap();
        assert!(view.scroll > before, "scroll {} -> {}", before, view.scroll);
        let (_, (last, _)) = view.sel.expect("selection");
        assert!(last >= view.scroll + rect.height as usize - 2);
        app.dragging = None;
    }

    #[test]
    fn a_click_does_not_select_from_the_scroll_origin() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let mut app = app_with(vcd);
        let dir =
            std::env::temp_dir().join(format!("waverdi_src_click_sel_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("long.sv");
        let mut text = String::from("module long(input logic clk);\n");
        for line in 0..80 {
            text.push_str(&format!("    assign w{line} = clk;\n"));
        }
        text.push_str("endmodule\n");
        std::fs::write(&file, text).unwrap();
        app.sources = Some(SourceSet::from_files(vec![file], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[1].module = "long".to_string();
        app.tree_sel = 1;
        app.sync_source();
        app.scroll_source(10);

        let l = app.layout();
        let rect = crate::ui::source::code_rect(&l);
        let y = rect.y + 2;
        let (line, col) = {
            let view = app.source_view.as_ref().unwrap();
            let line = view.scroll + (y - rect.y) as usize;
            (line, view.lines[line].find('w').unwrap())
        };
        let x = rect.x
            + crate::ui::source::gutter_width(app.source_view.as_ref().unwrap())
            + col as u16;
        crate::app::handle_mouse(&mut app, click(x, y));

        // An idle tick before the button is released must not grow the
        // selection from the first displayed line.
        let scroll = app.source_view.as_ref().unwrap().scroll;
        app.tick_source_drag();
        let view = app.source_view.as_ref().unwrap();
        assert_eq!(view.scroll, scroll);
        let (start, end) = view.sel.expect("click selection");
        assert_eq!(start, end, "a click must not become a range selection");

        // Releasing without dragging picks the word under the cursor.
        let up = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        };
        crate::app::handle_mouse(&mut app, up);
        let view = app.source_view.as_ref().unwrap();
        let (start, end) = view.sel.expect("word selection");
        assert_eq!(start.0, end.0, "the word selection stays on one line");
        assert_eq!(start.0, line);
        assert!(end.1 > start.1);
    }

    #[test]
    fn list_horizontal_scrollbar_shifts_long_names() {
        let vcd = "$timescale 1ns $end\n\
            $scope module top $end\n\
            $scope module sub $end\n\
            $var wire 1 ! a_very_long_signal_name $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let mut app = app_with(vcd);
        app.set_display(vec![0]);
        app.show_full_names = true;
        let l = app.layout();
        let value_w = l.value_col_width(app.splits.value_pct);
        let name_w = crate::ui::list::name_width(&l, value_w);
        let max = app.list_content_width() - name_w;
        assert!(max > 0, "the test needs an overflowing name");
        let y = l.list.bottom() - 1;

        // Drag the bar to the middle: the names scroll left.
        crate::app::handle_mouse(&mut app, click(l.list.x + 2, y));
        assert!(matches!(
            app.dragging.map(|drag| drag.mode),
            Some(crate::app::DragMode::ListHScroll)
        ));
        let grip = l.value_grip_x(app.splits.value_pct);
        crate::app::handle_mouse(&mut app, drag(l.list.x + (grip - l.list.x) / 2, y));
        app.dragging = None;
        assert!(app.list_h_scroll > 0 && app.list_h_scroll < max);

        // Dragging to the right end shows the tail again.
        crate::app::handle_mouse(&mut app, click(grip - 1, y));
        app.dragging = None;
        assert_eq!(app.list_h_scroll, max);
        crate::app::handle_mouse(&mut app, click(l.list.x, y));
        app.dragging = None;
        assert_eq!(app.list_h_scroll, 0);
    }

    #[test]
    fn tree_clicks_stay_in_sync_after_scrolling() {
        let mut vcd = String::from("$timescale 1ns $end\n$scope module tb $end\n");
        for i in 0..20 {
            vcd.push_str(&format!("$scope module s{i} $end\n$upscope $end\n"));
        }
        vcd.push_str("$upscope $end\n$enddefinitions $end\n#0\n");
        let mut app = app_with(&vcd);
        app.expanded.insert(1); // tb
        let l = app.layout();
        let inner = crate::ui::layout::tree_inner(&l);
        let col = inner.right() - 1;
        // Drag the scrollbar to the bottom.
        crate::app::handle_mouse(&mut app, click(col, inner.y + 1));
        crate::app::handle_mouse(&mut app, drag(col, inner.bottom() - 1));
        app.dragging = None;
        let rows = app.layout().tree_height();
        let total = app.tree_visible().len();
        assert!(total > rows, "the tree must scroll for this test");
        assert_eq!(app.tree_scroll, total - rows);

        // Clicks map to the node drawn on that row.
        let expected = app.tree_scroll;
        crate::app::handle_mouse(&mut app, click(inner.x + 2, inner.y + 1));
        assert_eq!(app.tree_sel, expected);

        // A stale offset (e.g. after collapsing scopes) is clamped first.
        app.tree_scroll = total + 5;
        crate::app::handle_mouse(&mut app, click(inner.x + 2, inner.y + 1));
        assert_eq!(app.tree_sel, total - rows);
    }

    #[test]
    fn drag_moves_the_whole_multi_selection() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 1 ! a $end\n\
            $var wire 1 \" b $end\n\
            $var wire 1 # c $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let mut app = app_with(vcd);
        app.set_display(vec![0, 1, 2]);
        app.selection = vec![0, 1];
        let l = app.layout();
        let first = l.list.y + 3; // G0, a, b, c
        let third = l.list.y + 5;
        crate::app::handle_mouse(&mut app, click(30, first));
        assert_eq!(app.selection, vec![0, 1], "clicking a picked row keeps it");
        crate::app::handle_mouse(&mut app, drag(30, third));
        app.dragging = None;
        assert_eq!(app.display, vec![2, 0, 1]);
        assert_eq!(app.selection, vec![0, 1]);
    }

    #[test]
    fn a_drop_creates_the_trailing_empty_group() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 1 ! a $end\n\
            $var wire 1 \" b $end\n\
            $var wire 1 # c $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let mut app = app_with(vcd);
        app.set_display(vec![0, 1, 2]);
        app.grow_groups(0);
        assert_eq!(app.groups.len(), 2);
        let l = app.layout();
        let src_row = l.list.y + 3;
        let g1_row = l.list.y + 6; // G0, a, b, c, G1
        crate::app::handle_mouse(&mut app, click(30, src_row));
        for _ in 0..8 {
            crate::app::handle_mouse(&mut app, drag(30, g1_row));
        }
        assert_eq!(app.groups[1].count, 1);
        assert_eq!(app.groups.len(), 2, "no group is created mid-drag");
        let up = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 30,
            row: g1_row,
            modifiers: KeyModifiers::NONE,
        };
        crate::app::handle_mouse(&mut app, up);
        assert_eq!(app.groups.len(), 3, "the drop adds the empty group");
        assert_eq!(app.groups[2].count, 0);
    }

    #[test]
    fn value_column_scrollbar_shifts_long_values() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 64 ! big [63:0] $end\n\
            $enddefinitions $end\n#0\nb0 !\n";
        let mut app = app_with(vcd);
        app.set_display(vec![0]);
        let l = app.layout();
        let value_w = l.value_col_width(app.splits.value_pct);
        assert!(
            app.value_content_width() > value_w,
            "needs an overflowing value"
        );
        let grip = l.value_grip_x(app.splits.value_pct);
        let y = l.list.bottom() - 1;
        crate::app::handle_mouse(&mut app, click(grip + 1, y));
        assert!(matches!(
            app.dragging.map(|drag| drag.mode),
            Some(crate::app::DragMode::ValueHScroll)
        ));
        crate::app::handle_mouse(&mut app, drag(l.list.right() - 2, y));
        app.dragging = None;
        assert!(app.value_h_scroll > 0, "the value bar must scroll");
        assert!(app.value_h_scroll >= app.value_content_width() - value_w - 1);
    }

    #[test]
    fn instance_pane_columns_scroll_and_resize() {
        let mut vcd = String::from("$timescale 1ns $end\n");
        for (level, name) in ["a", "b", "c", "d", "e"].iter().enumerate() {
            vcd.push_str(&format!("$scope module {name}{level} $end\n"));
        }
        for _ in 0..5 {
            vcd.push_str("$upscope $end\n");
        }
        vcd.push_str("$enddefinitions $end\n#0\n");
        let mut app = app_with(&vcd);
        let nodes = app.wf.as_ref().unwrap().tree.nodes.len();
        app.expanded.extend(0..nodes);
        for id in 1..nodes {
            app.wf.as_mut().unwrap().tree.nodes[id].module = "counter_pipeline_stage".to_string();
        }
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 40));

        let l = app.layout();
        let inner = crate::ui::layout::tree_inner(&l);
        let (hier_w, sep_x, module_x) = crate::ui::tree::columns(inner, app.splits.hier_pct);
        let name_w = hier_w.saturating_sub(1) as usize;
        let module_w = inner.right().saturating_sub(module_x + 1) as usize;
        assert!(app.tree_content_width() > name_w);
        assert!(app.module_content_width() > module_w);

        // Hierarchy scrollbar.
        let bar_y = inner.bottom() - 1;
        crate::app::handle_mouse(&mut app, click(inner.x + 1, bar_y));
        crate::app::handle_mouse(&mut app, drag(sep_x - 1, bar_y));
        app.dragging = None;
        assert!(app.tree_h_scroll > 0);

        // Module scrollbar.
        crate::app::handle_mouse(&mut app, click(module_x + 1, bar_y));
        crate::app::handle_mouse(&mut app, drag(inner.right() - 2, bar_y));
        app.dragging = None;
        assert!(app.module_h_scroll > 0);

        // The Hierarchy | Module divider resizes the columns.
        let before = app.splits.hier_pct;
        crate::app::handle_mouse(&mut app, click(sep_x, inner.y + 2));
        crate::app::handle_mouse(&mut app, drag(sep_x + 6, inner.y + 2));
        app.dragging = None;
        assert!(app.splits.hier_pct > before);
        let (_, sep_after, _) = crate::ui::tree::columns(inner, app.splits.hier_pct);
        assert!(sep_after > sep_x);
    }

    #[test]
    fn source_horizontal_scrollbar_shifts_long_lines() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $var wire 1 ! clk $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let mut app = app_with(vcd);
        let dir = std::env::temp_dir().join(format!("waverdi_src_hbar_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("wide.sv");
        let mut text = String::from("module wide(input logic clk);\n");
        for line in 0..20 {
            text.push_str(&format!("    assign w{line} = {};\n", "x".repeat(120)));
        }
        text.push_str("endmodule\n");
        std::fs::write(&file, text).unwrap();
        app.sources = Some(SourceSet::from_files(vec![file], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[1].module = "wide".to_string();
        app.tree_sel = 1;
        app.sync_source();

        let l = app.layout();
        let code = crate::ui::source::code_rect(&l);
        let gutter = crate::ui::source::gutter_width(app.source_view.as_ref().unwrap());
        let y = code.bottom() - 1;
        crate::app::handle_mouse(&mut app, click(code.x + gutter + 2, y));
        assert!(matches!(
            app.dragging.map(|drag| drag.mode),
            Some(crate::app::DragMode::SourceHScroll)
        ));
        crate::app::handle_mouse(&mut app, drag(code.right() - 2, y));
        app.dragging = None;
        // Every frame re-syncs the layout; the offset must not snap back.
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 40));
        let view = app.source_view.as_ref().unwrap();
        let vbar = crate::ui::source::scrollbar_col(&l, view).is_some();
        let text_w = (code.width as usize).saturating_sub(gutter as usize + usize::from(vbar));
        let content = view
            .lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        assert_eq!(
            app.source_h_scroll,
            content.saturating_sub(text_w),
            "the bar must reach the right edge"
        );
    }

    #[test]
    fn collapsing_expanded_rows_clamps_the_row_scroll() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 8 ! e0 [7:0] $end\n\
            $var wire 8 \" e1 [7:0] $end\n\
            $var wire 8 # e2 [7:0] $end\n\
            $enddefinitions $end\n#0\nb0 !\nb0 \"\nb0 #\n";
        let mut app = app_with(vcd);
        {
            let signals = &mut app.wf.as_mut().unwrap().signals;
            signals[0].name = "arr[0][7:0]".to_string();
            signals[1].name = "arr[1][7:0]".to_string();
            signals[2].name = "arr[2][7:0]".to_string();
        }
        app.wf.as_mut().unwrap().build_arrays();
        let root = app
            .wf
            .as_ref()
            .unwrap()
            .signals
            .iter()
            .position(|signal| signal.name == "arr")
            .unwrap();
        app.set_display(vec![root]);
        app.toggle_signal_expand(root); // root + 3 element rows
                                        // A stale scroll from a taller list is clamped after collapsing.
        app.row_scroll = 7;
        app.toggle_signal_expand(root); // collapse again
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 40));
        assert_eq!(app.row_scroll, 0);
        // The first signal row maps to the root, not to a phantom row.
        let l = app.layout();
        crate::app::handle_mouse(&mut app, click(30, l.list.y + 3));
        assert_eq!(app.selected_signal(), Some(root));
    }

    #[test]
    fn double_click_expand_a_multi_dim_array_keeps_lower_signals() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 8 ! e0 [7:0] $end\n\
            $var wire 8 \" e1 [7:0] $end\n\
            $var wire 8 # e2 [7:0] $end\n\
            $var wire 8 $ e3 [7:0] $end\n\
            $var wire 1 % clk $end\n\
            $enddefinitions $end\n#0\nb0 !\nb0 \"\nb0 #\nb0 $\n0%\n";
        let mut app = app_with(vcd);
        {
            let signals = &mut app.wf.as_mut().unwrap().signals;
            signals[0].name = "arr[0][0][7:0]".to_string();
            signals[1].name = "arr[0][1][7:0]".to_string();
            signals[2].name = "arr[1][0][7:0]".to_string();
            signals[3].name = "arr[1][1][7:0]".to_string();
        }
        app.wf.as_mut().unwrap().build_arrays();
        let root = app
            .wf
            .as_ref()
            .unwrap()
            .signals
            .iter()
            .position(|signal| signal.name == "arr")
            .unwrap();
        let clk = app
            .wf
            .as_ref()
            .unwrap()
            .signals
            .iter()
            .position(|signal| signal.name == "clk")
            .unwrap();
        app.set_display(vec![root, clk]);
        let l = app.layout();
        let y = l.list.y + 3; // the array root row
        let up = |column: u16, row: u16| MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        for _ in 0..2 {
            crate::app::handle_mouse(&mut app, click(30, y));
            crate::app::handle_mouse(&mut app, up(30, y));
        }
        assert!(app.display.contains(&clk), "{:?}", app.display);
        assert_eq!(app.display.len(), 4, "{:?}", app.display);
    }

    #[test]
    fn dragging_across_an_expanded_array_is_stable() {
        let vcd = "$timescale 1ns $end\n\
            $var wire 8 ! e0 [7:0] $end\n\
            $var wire 8 \" e1 [7:0] $end\n\
            $var wire 8 # tail $end\n\
            $enddefinitions $end\n#0\nb0 !\nb0 \"\n0#\n";
        let mut app = app_with(vcd);
        {
            let signals = &mut app.wf.as_mut().unwrap().signals;
            signals[0].name = "arr[0][7:0]".to_string();
            signals[1].name = "arr[1][7:0]".to_string();
        }
        app.wf.as_mut().unwrap().build_arrays();
        let root = app
            .wf
            .as_ref()
            .unwrap()
            .signals
            .iter()
            .position(|signal| signal.name == "arr")
            .unwrap();
        let tail = app
            .wf
            .as_ref()
            .unwrap()
            .signals
            .iter()
            .position(|signal| signal.name == "tail")
            .unwrap();
        app.set_display(vec![root, tail]);
        app.toggle_signal_expand(root); // rows: root, arr[0], arr[1], tail
        assert_eq!(app.display.len(), 4);

        let l = app.layout();
        let tail_display = app.display.iter().position(|&s| s == tail).unwrap();
        let tail_row = l.list.y + 3 + tail_display as u16;
        let root_row = l.list.y + 3;
        crate::app::handle_mouse(&mut app, click(30, tail_row));
        for _ in 0..5 {
            crate::app::handle_mouse(&mut app, drag(30, root_row));
        }
        app.dragging = None;
        assert_eq!(app.display[0], tail, "{:?}", app.display);
        // Repeating the drag does not move it back.
        let before = app.display.clone();
        app.last_click = None;
        crate::app::handle_mouse(&mut app, click(30, l.list.y + 3));
        for _ in 0..3 {
            crate::app::handle_mouse(&mut app, drag(30, root_row));
        }
        app.dragging = None;
        assert_eq!(app.display, before);
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
        // A plain click on a picked row keeps the block (so it can be
        // dragged); a row outside the selection starts a new single one.
        crate::app::handle_mouse(&mut app, click_with(30, top, KeyModifiers::ALT));
        assert_eq!(app.selection, vec![1]);
        crate::app::handle_mouse(&mut app, click(30, top));
        assert!(app.selection.is_empty());
        assert_eq!(app.sel_row, Some(1));
    }
}
