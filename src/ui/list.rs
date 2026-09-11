use crate::app::{App, Focus, ListRow};
use crate::theme::*;
use crate::ui::layout::Layout;
use crate::ui::text;
use crate::waveform::Waveform;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

pub fn draw(buf: &mut Buffer, l: &Layout, app: &App, wf: &Waveform) {
    let header = Rect {
        x: l.list.x,
        y: l.list.y,
        width: l.list.width,
        height: 2,
    };
    buf.set_style(header, Style::new().bg(LIST_HEADER_BG));
    buf.set_string(
        l.list.x,
        l.list.y,
        " Signal List",
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
    );
    let value_w = value_col_width(l);
    text::put(
        buf,
        l.list.right().saturating_sub(value_w as u16),
        l.list.y,
        "Value",
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
    );
    buf.set_string(
        l.list.x,
        l.list.y + 1,
        "─".repeat(l.list.width as usize),
        Style::new().fg(PANEL_BORDER),
    );

    let rows = app.list_rows();
    let scroll = app.row_scroll.min(rows.len().saturating_sub(l.rows_h));
    for visible in 0..l.rows_h {
        let k = scroll + visible;
        let Some(list_row) = rows.get(k) else { break };
        let y = l.list.y + 2 + visible as u16;
        let selected = Some(k) == app.sel_row;
        let bg = if selected {
            ROW_SEL_BG
        } else if visible % 2 == 0 {
            ROW_ALT
        } else {
            BG
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

        match list_row {
            ListRow::Group {
                name,
                depth,
                count,
                collapsed,
                ..
            } => {
                let arrow = if *collapsed { "▸" } else { "▾" };
                let label = format!("{}{arrow} {name}/ ({count})", "  ".repeat(*depth));
                let style = if selected {
                    Style::new().fg(Color::Black).bg(ACCENT)
                } else {
                    Style::new().fg(ACCENT).bg(bg).add_modifier(Modifier::BOLD)
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
                let name_style = if selected && app.focus == Focus::List {
                    Style::new().fg(Color::White).bg(bg)
                } else {
                    Style::new().fg(Color::Rgb(190, 190, 200)).bg(bg)
                };
                let indent = "  ".repeat(*depth);
                let name = text::trunc(&format!("{indent}{}", signal.name), name_width(l, value_w));
                buf.set_string(l.list.x, y, &name, name_style);

                let radix = app.radix_for(*sig);
                let (from, to) = app.cursor_column_range();
                let transition = signal.display_change_in(from, to, radix);
                let on_edge = transition.is_some();
                let value = match transition {
                    Some(text_value) if text_value.chars().count() <= value_w => text_value,
                    _ => signal.display_value(app.cursor, radix),
                };
                let value_style = if on_edge {
                    Style::new().fg(CURSOR).bg(bg).add_modifier(Modifier::BOLD)
                } else if value.contains('x') || value.contains('z') {
                    Style::new().fg(XCOL).bg(bg)
                } else {
                    Style::new().fg(Color::Rgb(220, 220, 230)).bg(bg)
                };
                text::put(
                    buf,
                    l.list.right().saturating_sub(value_w as u16),
                    y,
                    &value,
                    value_style,
                );
            }
        }
    }
}

fn name_width(l: &Layout, value_w: usize) -> usize {
    (l.list.width as usize).saturating_sub(value_w + 1)
}

fn value_col_width(l: &Layout) -> usize {
    (l.list.width as usize / 4).clamp(6, 14)
}
