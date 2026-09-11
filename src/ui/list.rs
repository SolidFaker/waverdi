use crate::app::{App, Focus};
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

    for row in 0..l.rows_h {
        let k = app.row_scroll + row;
        if k >= app.display.len() {
            break;
        }
        let idx = app.display[k];
        let sig = &wf.signals[idx];
        let y = l.list.y + 2 + row as u16;
        let selected = Some(k) == app.sel_row;
        let bg = if selected {
            ROW_SEL_BG
        } else if row % 2 == 0 {
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

        let name_style = if selected && app.focus == Focus::List {
            Style::new().fg(Color::White).bg(bg)
        } else {
            Style::new().fg(Color::Rgb(190, 190, 200)).bg(bg)
        };
        let name_w = (l.list.width as usize).saturating_sub(value_w + 1);
        let name = text::trunc(&sig.full_name(), name_w);
        buf.set_string(l.list.x, y, &name, name_style);

        let radix = app.radix_for(idx);
        let transition = sig.display_change(app.cursor, radix);
        let on_edge = transition.is_some();
        let value = match transition {
            Some(text) if text.chars().count() <= value_w => text,
            _ => sig.display_value(app.cursor, radix),
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

fn value_col_width(l: &Layout) -> usize {
    (l.list.width as usize / 4).clamp(6, 14)
}
