use crate::app::{App, Focus, TreeNode};
use crate::theme::Theme;
use crate::ui::layout::{tree_inner, Layout};
use crate::ui::text;
use crate::waveform::Waveform;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Widget as _};

pub fn draw_frame(buf: &mut Buffer, l: &Layout, t: &Theme, focused: bool) {
    let border_style = if focused {
        Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(t.panel_border)
    };
    let title_style = if focused {
        Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(t.text).add_modifier(Modifier::BOLD)
    };
    Block::bordered()
        .title(" Instance ")
        .title_style(title_style)
        .border_style(border_style)
        .render(l.tree, buf);
}

/// Columns of the instance table: (Hierarchy width, separator x, module x).
pub fn columns(inner: Rect) -> (u16, u16, u16) {
    let hier_w = (inner.width as u32 * 55 / 100) as u16;
    let sep_x = inner.x + hier_w;
    (hier_w, sep_x, sep_x + 1)
}

pub fn draw(buf: &mut Buffer, l: &Layout, app: &App, wf: &Waveform) {
    let t = &app.theme;
    let focused = app.focus == Focus::Tree;
    draw_frame(buf, l, t, focused);

    let inner = tree_inner(l);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let (hier_w, sep_x, module_x) = columns(inner);

    // Header: Hierarchy | Module.
    let header = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    buf.set_style(header, Style::new().bg(t.list_header_bg));
    text::put(
        buf,
        inner.x + 1,
        inner.y,
        "Hierarchy",
        Style::new().fg(t.dim).add_modifier(Modifier::BOLD),
    );
    if module_x < inner.right() {
        text::put(
            buf,
            module_x + 1,
            inner.y,
            "Module",
            Style::new().fg(t.dim).add_modifier(Modifier::BOLD),
        );
    }
    for row in inner.y..inner.bottom() {
        if let Some(cell) = buf.cell_mut((sep_x, row)) {
            cell.set_symbol("│");
            cell.set_fg(t.panel_border);
            cell.set_bg(t.bg);
        }
    }

    let nodes = app.tree_visible();
    let height = inner.height.saturating_sub(1) as usize;
    let scroll = app.tree_scroll.min(nodes.len().saturating_sub(height));

    for row in 0..height {
        let k = scroll + row;
        let Some(node) = nodes.get(k).copied() else {
            break;
        };
        let selected = k == app.tree_sel;
        let y = inner.y + 1 + row as u16;
        let TreeNode::Scope { id, depth } = node;
        let scope = &wf.tree.nodes[id];
        let arrow = if app.expanded.contains(&id) {
            "▾"
        } else {
            "▸"
        };
        let label = format!("{}{arrow} {}", "  ".repeat(depth), scope.name);
        let style = if selected {
            Style::new().fg(Color::Black).bg(t.accent)
        } else {
            Style::new().fg(t.text)
        };
        let name_w = hier_w.saturating_sub(1) as usize;
        text::put(buf, inner.x, y, &text::trunc(&label, name_w), style);
        if module_x < inner.right() {
            let module_style = if selected {
                Style::new().fg(Color::Black).bg(t.accent)
            } else {
                Style::new().fg(t.scope)
            };
            let module_w = inner.right().saturating_sub(module_x) as usize;
            text::put(
                buf,
                module_x,
                y,
                &text::trunc(&scope.module, module_w),
                module_style,
            );
        }
    }

    draw_scrollbar(buf, &inner, nodes.len(), scroll, height, t);
}

fn draw_scrollbar(
    buf: &mut Buffer,
    inner: &Rect,
    total: usize,
    scroll: usize,
    height: usize,
    t: &Theme,
) {
    if total <= height || height == 0 {
        return;
    }
    let thumb = ((height as f64 / total as f64) * height as f64).max(1.0) as usize;
    let top = ((scroll as f64 / total as f64) * height as f64) as usize;
    for row in 0..height {
        let symbol = if row >= top && row < top + thumb {
            "█"
        } else {
            "│"
        };
        text::set_cell(
            buf,
            inner.right().saturating_sub(1),
            inner.y + 1 + row as u16,
            symbol,
            t.dim,
            t.bg,
        );
    }
}
