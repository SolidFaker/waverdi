//! Rendering and hit-testing for the "Add Signals" picker.

use crate::app::{AddFocus, App};
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Clear, Widget as _};

pub const ADD_W: u16 = 100;
pub const ADD_H: u16 = 26;

pub struct AddLayout {
    pub outer: Rect,
    pub tree: Rect,
    pub instances: Rect,
    pub signals: Rect,
    pub filter: Rect,
    pub apply: Rect,
    pub ok: Rect,
    pub cancel: Rect,
}

pub fn layout(screen: Rect) -> AddLayout {
    let width = ADD_W
        .min(screen.width.saturating_sub(4))
        .max(40)
        .min(screen.width);
    let height = ADD_H
        .min(screen.height.saturating_sub(2))
        .max(10)
        .min(screen.height);
    let outer = Rect {
        x: screen.x + screen.width.saturating_sub(width) / 2,
        y: screen.y + screen.height.saturating_sub(height) / 2,
        width,
        height,
    };
    let inner = Rect {
        x: outer.x + 1,
        y: outer.y + 1,
        width: outer.width.saturating_sub(2),
        height: outer.height.saturating_sub(2),
    };
    let bottom = inner.bottom().saturating_sub(1);
    let body_h = inner.height.saturating_sub(2).max(1);
    let left_w = (inner.width * 42 / 100).max(20).min(inner.width);
    let tree = Rect {
        x: inner.x,
        y: inner.y,
        width: left_w,
        height: body_h,
    };
    let right_x = inner.x + left_w + 1;
    let right_w = inner.right().saturating_sub(right_x);
    let top_h = (body_h * 45 / 100).max(2).min(body_h);
    let instances = Rect {
        x: right_x,
        y: inner.y,
        width: right_w,
        height: top_h,
    };
    let signals = Rect {
        x: right_x,
        y: inner.y + top_h + 1,
        width: right_w,
        height: body_h.saturating_sub(top_h + 1),
    };
    let filter = Rect {
        x: inner.x,
        y: bottom,
        width: left_w,
        height: 1,
    };
    let apply_w = 9u16;
    let ok_w = 6u16;
    let cancel_w = 10u16;
    let total = apply_w + ok_w + cancel_w + 2;
    let bx = inner.right().saturating_sub(total).max(inner.x);
    let apply = Rect {
        x: bx,
        y: bottom,
        width: apply_w,
        height: 1,
    };
    let ok = Rect {
        x: (apply.right() + 1).min(inner.right()),
        y: bottom,
        width: ok_w,
        height: 1,
    };
    let cancel = Rect {
        x: (ok.right() + 1).min(inner.right()),
        y: bottom,
        width: cancel_w,
        height: 1,
    };
    AddLayout {
        outer,
        tree,
        instances,
        signals,
        filter,
        apply,
        ok,
        cancel,
    }
}

fn col_width(items: &[String]) -> usize {
    items
        .iter()
        .map(|name| name.chars().count())
        .max()
        .unwrap_or(8)
        .max(8)
        + 2
}

/// Grid geometry inside `rect`: column width, column count and the number of
/// items that fit, reserving the last column for the scrollbar. Computed once
/// per draw and then reused by the row loop and the scrollbar.
#[derive(Clone, Copy)]
struct GridGeometry {
    col_w: usize,
    cols: usize,
    visible: usize,
}

fn grid_geometry(rect: Rect, items: &[String]) -> GridGeometry {
    let col_w = col_width(items);
    let width = rect.width.saturating_sub(1) as usize;
    let cols = (width / col_w).max(1);
    GridGeometry {
        col_w,
        cols,
        visible: cols * rect.height as usize,
    }
}

/// Columns of a picker pane (the tree is a single column).
pub fn pane_cols(screen: Rect, app: &App, pane: AddFocus) -> usize {
    let l = layout(screen);
    match pane {
        AddFocus::Tree => 1,
        AddFocus::Instances => grid_geometry(l.instances, &app.add_pane(pane).1).cols,
        AddFocus::Signals => grid_geometry(l.signals, &app.add_pane(pane).1).cols,
    }
}

/// Items visible at once in a picker pane.
pub fn pane_visible(screen: Rect, app: &App, pane: AddFocus) -> usize {
    let l = layout(screen);
    match pane {
        AddFocus::Tree => l.tree.height as usize,
        AddFocus::Instances => grid_geometry(l.instances, &app.add_pane(pane).1).visible,
        AddFocus::Signals => grid_geometry(l.signals, &app.add_pane(pane).1).visible,
    }
}

/// Window start for a scrollbar dragged to `row` of a `track`-row bar.
pub fn scroll_from_track(track: usize, len: usize, visible: usize, row: usize) -> usize {
    if len <= visible || track == 0 || visible == 0 {
        return 0;
    }
    let max_start = len - visible;
    let thumb = ((track * visible) / len).max(1).min(track);
    let span = track.saturating_sub(thumb);
    if span == 0 {
        return 0;
    }
    row.min(span) * max_start / span
}

/// Picker pane under the pointer, if any.
pub fn pane_at(screen: Rect, col: u16, row: u16) -> Option<AddFocus> {
    let l = layout(screen);
    let inside =
        |rect: Rect| col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom();
    if inside(l.tree) {
        Some(AddFocus::Tree)
    } else if inside(l.instances) {
        Some(AddFocus::Instances)
    } else if inside(l.signals) {
        Some(AddFocus::Signals)
    } else {
        None
    }
}

pub fn draw(frame: &mut ratatui::Frame, screen: Rect, app: &App) {
    let t = &app.theme;
    let Some(add) = app.add_signals.as_ref() else {
        return;
    };
    let l = layout(screen);
    let inner = Rect {
        x: l.outer.x + 1,
        y: l.outer.y + 1,
        width: l.outer.width.saturating_sub(2),
        height: l.outer.height.saturating_sub(2),
    };
    let buf = frame.buffer_mut();
    Clear.render(l.outer, buf);
    buf.set_style(l.outer, Style::new().bg(t.popup_bg));
    Block::bordered()
        .title(" Add Signals ")
        .title_style(Style::new().fg(t.accent).add_modifier(Modifier::BOLD))
        .border_style(Style::new().fg(t.accent))
        .render(l.outer, buf);

    // --- hierarchy tree -------------------------------------------------
    let tree_rows = app.add_tree_rows();
    let tree_visible = l.tree.height as usize;
    let tree_start = app.add_scroll_of(AddFocus::Tree);
    let tree_width = l.tree.width.saturating_sub(1) as usize;
    for (row, (node, depth)) in tree_rows
        .iter()
        .skip(tree_start)
        .take(tree_visible)
        .enumerate()
    {
        let y = l.tree.y + row as u16;
        let selected = add.focus == AddFocus::Tree && tree_start + row == add.tree_sel;
        let current = *node == add.scope;
        let bg = if selected {
            t.list_header_bg
        } else {
            t.popup_bg
        };
        let has_children = !app
            .wf
            .as_ref()
            .map(|wf| wf.tree.nodes[*node].children.is_empty())
            .unwrap_or(true);
        let arrow = if has_children {
            if add.expanded.contains(node) {
                "▾"
            } else {
                "▸"
            }
        } else {
            " "
        };
        let name = app
            .wf
            .as_ref()
            .map(|wf| wf.tree.nodes[*node].name.clone())
            .unwrap_or_default();
        let label = format!("{}{} {}", "  ".repeat(*depth), arrow, name);
        let style = if current {
            Style::new()
                .fg(t.accent)
                .bg(bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(t.text).bg(bg)
        };
        text::put(buf, l.tree.x, y, &text::trunc(&label, tree_width), style);
    }
    draw_scrollbar(buf, l.tree, tree_start, tree_rows.len(), tree_visible, t);

    // --- instances of the current level ---------------------------------
    let (instance_items, instances) = app.add_pane(AddFocus::Instances);
    let instance_start = app.add_scroll_of(AddFocus::Instances);
    let geometry = grid_geometry(l.instances, &instances);
    draw_grid(
        buf,
        l.instances,
        &instances,
        geometry,
        instance_start,
        |index| add.focus == AddFocus::Instances && index == add.instance_sel,
        |_| false,
        t.popup_bg,
        t,
    );
    draw_scrollbar(
        buf,
        l.instances,
        instance_start,
        instance_items.len(),
        geometry.visible,
        t,
    );

    // --- signals of the current level -----------------------------------
    let (signal_items, names) = app.add_pane(AddFocus::Signals);
    let signal_start = app.add_scroll_of(AddFocus::Signals);
    let geometry = grid_geometry(l.signals, &names);
    draw_grid(
        buf,
        l.signals,
        &names,
        geometry,
        signal_start,
        |index| add.focus == AddFocus::Signals && index == add.signal_sel,
        |index| {
            signal_items
                .get(index)
                .map(|signal| add.selected.contains(signal))
                .unwrap_or(false)
        },
        t.popup_bg,
        t,
    );
    draw_scrollbar(
        buf,
        l.signals,
        signal_start,
        signal_items.len(),
        geometry.visible,
        t,
    );

    // --- separators -----------------------------------------------------
    let sep = Style::new().fg(t.dim);
    let sep_x = l.instances.x - 1;
    for y in l.tree.y..l.signals.bottom() {
        text::put(buf, sep_x, y, "│", sep);
    }
    for x in l.instances.x..inner.right() {
        text::put(buf, x, l.instances.bottom(), "─", sep);
    }
    text::put(buf, sep_x, l.instances.bottom(), "├", sep);
    for x in inner.x..inner.right() {
        text::put(buf, x, l.signals.bottom(), "─", sep);
    }
    text::put(buf, sep_x, l.signals.bottom(), "┴", sep);

    // --- filter and buttons ---------------------------------------------
    text::put(
        buf,
        l.filter.x,
        l.filter.y,
        &format!(" Filter: {} ▾", add.filter.label()),
        Style::new().fg(t.accent),
    );
    button(buf, l.apply, " Apply ", app);
    button(buf, l.ok, " OK ", app);
    button(buf, l.cancel, " Cancel ", app);
}

#[allow(clippy::too_many_arguments)]
fn draw_grid(
    buf: &mut Buffer,
    rect: Rect,
    items: &[String],
    geometry: GridGeometry,
    start: usize,
    cursor: impl Fn(usize) -> bool,
    selected: impl Fn(usize) -> bool,
    bg: Color,
    t: &crate::theme::Theme,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let GridGeometry {
        col_w,
        cols,
        visible,
    } = geometry;
    for (offset, name) in items.iter().skip(start).take(visible).enumerate() {
        let row = offset / cols;
        let col = offset % cols;
        let x = rect.x + (col * col_w) as u16;
        let y = rect.y + row as u16;
        let index = start + offset;
        let style = if cursor(index) {
            Style::new()
                .fg(Color::Black)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD)
        } else if selected(index) {
            Style::new()
                .fg(t.accent)
                .bg(bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(t.text).bg(bg)
        };
        text::put(buf, x, y, &text::trunc(name, col_w), style);
    }
}

/// Vertical scrollbar in the rightmost column of a pane.
fn draw_scrollbar(
    buf: &mut Buffer,
    rect: Rect,
    start: usize,
    len: usize,
    visible: usize,
    t: &crate::theme::Theme,
) {
    if rect.width == 0 {
        return;
    }
    let track = rect.height as usize;
    let Some((pos, thumb)) =
        crate::ui::scrollbar::track_geometry(track, len, visible, start, false)
    else {
        return;
    };
    crate::ui::scrollbar::Bar {
        orientation: crate::ui::scrollbar::Orientation::Vertical,
        x: rect.right().saturating_sub(1),
        y: rect.y,
        span: track,
        start: pos,
        len: thumb,
        thumb: "█",
        track: "│",
        thumb_fg: t.accent,
        track_fg: t.dim,
        bg: t.popup_bg,
    }
    .draw(buf);
}

fn button(buf: &mut Buffer, rect: Rect, label: &str, app: &App) {
    let t = &app.theme;
    text::put(
        buf,
        rect.x,
        rect.y,
        label,
        Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
    );
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddHit {
    Tree(usize, bool),
    Instance(usize),
    /// Signal index in the waveform (not the position in the pane).
    Signal(usize),
    Scrollbar(AddFocus),
    Filter,
    Apply,
    Ok,
    Cancel,
}

pub fn hit(screen: Rect, app: &App, col: u16, row: u16) -> Option<AddHit> {
    app.add_signals.as_ref()?;
    let l = layout(screen);
    let inside =
        |rect: Rect| col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom();
    // Scrollbars are the last column of every pane.
    for pane in [AddFocus::Tree, AddFocus::Instances, AddFocus::Signals] {
        let rect = match pane {
            AddFocus::Tree => l.tree,
            AddFocus::Instances => l.instances,
            AddFocus::Signals => l.signals,
        };
        if col == rect.right().saturating_sub(1) && row >= rect.y && row < rect.bottom() {
            let visible = pane_visible(screen, app, pane);
            if app.add_pane_len(pane) > visible && visible > 0 {
                return Some(AddHit::Scrollbar(pane));
            }
        }
    }
    if inside(l.filter) {
        return Some(AddHit::Filter);
    }
    if inside(l.apply) {
        return Some(AddHit::Apply);
    }
    if inside(l.ok) {
        return Some(AddHit::Ok);
    }
    if inside(l.cancel) {
        return Some(AddHit::Cancel);
    }
    if inside(l.tree) {
        let tree_rows = app.add_tree_rows();
        let start = app.add_scroll_of(AddFocus::Tree);
        let index = start + (row - l.tree.y) as usize;
        let (node, depth) = *tree_rows.get(index)?;
        let arrow_zone = l.tree.x + (depth * 2 + 2) as u16;
        return Some(AddHit::Tree(node, col < arrow_zone));
    }
    if inside(l.instances) {
        let (instances, names) = app.add_pane(AddFocus::Instances);
        let geometry = grid_geometry(l.instances, &names);
        let start = app.add_scroll_of(AddFocus::Instances);
        let index = start
            + (row - l.instances.y) as usize * geometry.cols
            + (col.saturating_sub(l.instances.x) as usize / geometry.col_w);
        return instances.get(index).map(|_| AddHit::Instance(index));
    }
    if inside(l.signals) {
        let (list, names) = app.add_pane(AddFocus::Signals);
        let geometry = grid_geometry(l.signals, &names);
        let start = app.add_scroll_of(AddFocus::Signals);
        let index = start
            + (row - l.signals.y) as usize * geometry.cols
            + (col.saturating_sub(l.signals.x) as usize / geometry.col_w);
        return list.get(index).copied().map(AddHit::Signal);
    }
    None
}
