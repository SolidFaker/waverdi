use crate::app::{App, Focus, ListRow};
use crate::ui::layout::Layout;
use crate::ui::scrollbar;
use crate::ui::text;
use crate::waveform::{SigKind, Waveform};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::rc::Rc;

pub fn draw(buf: &mut Buffer, l: &Layout, app: &App, wf: &Waveform) {
    let t = &app.theme;
    let focused = app.focus == Focus::List;
    let header = Rect {
        x: l.list.x,
        y: l.list.y,
        width: l.list.width,
        height: 2,
    };
    buf.set_style(header, Style::new().bg(t.list_header_bg));
    let title_style = if focused {
        Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(t.text).add_modifier(Modifier::BOLD)
    };
    buf.set_string(l.list.x, l.list.y, " Signal List", title_style);
    let value_w = l.value_col_width(app.splits.value_pct);
    let grip = l.value_grip_x(app.splits.value_pct);
    text::put(
        buf,
        grip + 1,
        l.list.y,
        "Value",
        Style::new().fg(t.text).add_modifier(Modifier::BOLD),
    );
    buf.set_string(
        l.list.x,
        l.list.y + 1,
        "─".repeat(l.list.width as usize),
        Style::new().fg(if focused { t.accent } else { t.panel_border }),
    );

    let rows = app.list_rows();
    let scroll = app.row_scroll.min(rows.len().saturating_sub(l.rows_h));
    let value_content = app.value_content_width();
    for visible in 0..l.rows_h {
        let k = scroll + visible;
        let Some(list_row) = rows.get(k) else { break };
        let y = l.list.y + 2 + visible as u16;
        let selected = Some(k) == app.sel_row;
        let multi = matches!(list_row, ListRow::Signal { sig, .. } if app.selection.contains(sig));
        let bg = if selected {
            t.row_sel_bg
        } else if multi {
            t.multi_sel_bg
        } else if visible % 2 == 0 {
            t.row_alt
        } else {
            t.bg
        };
        buf.set_style(
            Rect {
                x: l.list.x,
                y,
                width: l.list.width,
                height: 1,
            },
            Style::new().bg(bg),
        );

        text::set_cell(
            buf,
            grip,
            y,
            "│",
            if focused { t.accent } else { t.panel_border },
            bg,
        );

        match list_row {
            ListRow::Group {
                name,
                count,
                collapsed,
                ..
            } => {
                let arrow = if *collapsed { "▸" } else { "▾" };
                let label = format!("{arrow} {name} ({count})");
                let style = if selected {
                    Style::new().fg(Color::Black).bg(t.accent)
                } else {
                    Style::new()
                        .fg(t.accent)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD)
                };
                buf.set_string(
                    l.list.x,
                    y,
                    text::trunc(&label, l.list.width as usize),
                    style,
                );
            }
            ListRow::Signal { sig, depth } => {
                let signal = &wf.signals[*sig];
                // Highlighted signals keep the colour on their name and in the
                // waveform row; the selected row wins while it is focused.
                let highlight = app.highlight_of(*sig);
                let name_bg = if selected && app.focus == Focus::List {
                    bg
                } else {
                    highlight.unwrap_or(bg)
                };
                let name_style = if selected && app.focus == Focus::List {
                    Style::new().fg(t.text).bg(name_bg)
                } else {
                    Style::new().fg(t.name).bg(name_bg)
                };
                let full = signal.full_name();
                let (prefix, leaf) = if app.show_full_names {
                    match full.rfind('.') {
                        Some(dot) => (full[..=dot].to_string(), full[dot + 1..].to_string()),
                        None => (String::new(), full),
                    }
                } else {
                    (String::new(), signal.name.clone())
                };
                let indent = "  ".repeat(*depth);
                // Right-aligned by default; the horizontal scrollbar shifts
                // the names to reveal the hierarchy of long names.
                let width = name_width(l, value_w);
                let content_w = app.list_content_width().max(width);
                let h = app.list_h_scroll.min(content_w - width);
                let full_text = format!("{indent}{prefix}{leaf}");
                let full_len = full_text.chars().count();
                let x0 = l.list.x as i64 + (content_w - full_len) as i64 - h as i64;
                let left = l.list.x as i64;
                let right = grip as i64;
                let skip = (left - x0).max(0) as usize;
                let ellipsis = skip > 0;
                let start = if ellipsis { left + 1 } else { x0.max(left) };
                if start < right {
                    let room = (right - start) as usize;
                    let visible: String = full_text.chars().skip(skip).take(room).collect();
                    if ellipsis {
                        text::set_cell(buf, l.list.x, y, "…", t.dim, name_bg);
                    }
                    // The hierarchy path is dim; the signal name keeps its colour.
                    let split = indent.chars().count() + prefix.chars().count();
                    let prefix_cells = (split as i64 - skip as i64)
                        .clamp(0, visible.chars().count() as i64)
                        as usize;
                    let prefix_part: String = visible.chars().take(prefix_cells).collect();
                    let leaf_part: String = visible.chars().skip(prefix_cells).collect();
                    let start = start as u16;
                    buf.set_string(start, y, &prefix_part, Style::new().fg(t.dim).bg(name_bg));
                    buf.set_string(start + prefix_cells as u16, y, &leaf_part, name_style);
                }

                let radix = app.radix_for(*sig);
                let (from, to) = app.cursor_column_range();
                // Brace values of arrays/aggregates are joined at the cursor
                // column; `Waveform` handles both plain and synthesized rows.
                let transition = wf.display_change_in(*sig, from, to, radix);
                let on_edge = transition.is_some();
                // The fallback text is shared with the value-column scrollbar
                // through the per-frame cache; only the edge text is local.
                let value = match transition {
                    Some(text_value) if text_value.chars().count() <= value_w => {
                        Rc::from(text_value)
                    }
                    _ => app.list_display_value(*sig).0,
                };
                let value_style = if on_edge {
                    Style::new()
                        .fg(t.cursor)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(t.value).bg(bg)
                };
                // Keep the value inside its column: never over the divider.
                let value_max = value_content.saturating_sub(value_w);
                let value_offset = app.value_h_scroll.min(value_max);
                let visible = text::scroll_slice(&value, value_offset, value_w);
                // Logic values colour each unknown digit on its own; the
                // text of real/string values keeps the plain foreground so a
                // literal 'x' or 'z' in it is not mistaken for an unknown.
                let unknown_colors =
                    !on_edge && (wf.is_synthesized(*sig) || signal.kind == SigKind::Bits);
                if unknown_colors {
                    text::put_unknown_digits(
                        buf,
                        grip + 1,
                        y,
                        &visible,
                        value_style,
                        t.xcol,
                        t.zcol,
                    );
                } else {
                    text::put(buf, grip + 1, y, &visible, value_style);
                }
            }
        }
    }

    // The Signal List | Value divider runs to the bottom of the pane.
    let border = if focused { t.accent } else { t.panel_border };
    for y in (l.list.y + 2 + l.rows_h as u16)..l.list.bottom() {
        text::set_cell(buf, grip, y, "│", border, t.bg);
    }
    draw_h_scrollbar(buf, l, app);
}

/// Horizontal scrollbars of the Signal List: names (left) and values (right),
/// aligned with the waveform scrollbar.
fn draw_h_scrollbar(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    if l.list.height == 0 {
        return;
    }
    let value_w = l.value_col_width(app.splits.value_pct);
    let name_w = name_width(l, value_w);
    let grip = l.value_grip_x(app.splits.value_pct);
    let y = l.list.bottom().saturating_sub(1);
    let name_content = app.list_content_width();
    let name_max = name_content.saturating_sub(name_w);
    scrollbar::h_scrollbar(
        buf,
        l.list.x,
        grip,
        y,
        name_content,
        name_w,
        app.list_h_scroll.min(name_max),
        t.dim,
        t.bg,
    );
    let value_content = app.value_content_width();
    let value_max = value_content.saturating_sub(value_w);
    scrollbar::h_scrollbar(
        buf,
        grip + 1,
        l.list.right().saturating_sub(1).max(grip + 2),
        y,
        value_content,
        value_w,
        app.value_h_scroll.min(value_max),
        t.dim,
        t.bg,
    );
}

pub fn name_width(l: &Layout, value_w: usize) -> usize {
    (l.list.width as usize).saturating_sub(value_w + 2)
}
