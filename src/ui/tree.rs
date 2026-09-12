use crate::app::{App, Focus, TreeNode};
use crate::theme::*;
use crate::ui::layout::{tree_inner, Layout};
use crate::ui::text;
use crate::waveform::Waveform;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Widget as _};

pub fn draw_frame(buf: &mut Buffer, l: &Layout, focused: bool) {
    let border_style = if focused {
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(PANEL_BORDER)
    };
    let title_style = if focused {
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
    };
    Block::bordered()
        .title(" Instance ")
        .title_style(title_style)
        .border_style(border_style)
        .render(l.tree, buf);
}

pub fn draw(buf: &mut Buffer, l: &Layout, app: &App, wf: &Waveform) {
    let focused = app.focus == Focus::Tree;
    draw_frame(buf, l, focused);

    let inner = tree_inner(l);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let nodes = app.tree_visible();
    let height = inner.height as usize;
    let scroll = app.tree_scroll.min(nodes.len().saturating_sub(height));

    for row in 0..height {
        let k = scroll + row;
        let Some(node) = nodes.get(k).copied() else {
            break;
        };
        let selected = k == app.tree_sel;
        let (label, style) = match node {
            TreeNode::Scope { id, depth } => {
                let name = &wf.tree.nodes[id].name;
                let arrow = if app.expanded.contains(&id) {
                    "▾"
                } else {
                    "▸"
                };
                let style = if selected {
                    Style::new().fg(Color::Black).bg(ACCENT)
                } else {
                    Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                };
                (format!("{}{arrow} {name}", "  ".repeat(depth)), style)
            }
            TreeNode::Signal { sig, depth } => {
                let name = &wf.signals[sig].name;
                let multi = app.tree_multi.contains(&sig);
                let style = if selected {
                    Style::new().fg(Color::Black).bg(ACCENT)
                } else if multi {
                    Style::new().fg(Color::White).bg(MULTI_SEL_BG)
                } else if app.display.contains(&sig) {
                    Style::new().fg(GREEN_DIM)
                } else {
                    Style::new().fg(Color::White)
                };
                (format!("{}  {name}", "  ".repeat(depth)), style)
            }
        };
        text::put(buf, inner.x, inner.y + row as u16, &label, style);
    }

    draw_scrollbar(buf, &inner, nodes.len(), scroll, height);
}

fn draw_scrollbar(buf: &mut Buffer, inner: &Rect, total: usize, scroll: usize, height: usize) {
    if total <= height {
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
            inner.y + row as u16,
            symbol,
            DIM,
            BG,
        );
    }
}
