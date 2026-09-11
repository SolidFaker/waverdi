use crate::app::App;
use crate::theme::*;
use crate::ui::layout::Layout;
use crate::ui::text;
use crate::waveform;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

pub fn draw_messages(buf: &mut Buffer, l: &Layout, app: &App) {
    buf.set_style(l.msg, Style::new().bg(MSG_BG));
    let n = app.messages.len();
    for row in 0..3 {
        let k = n as i64 - 3 + row as i64;
        if k < 0 || k >= n as i64 {
            continue;
        }
        let style = if row == 2 {
            Style::new().fg(Color::White)
        } else {
            Style::new().fg(DIM)
        };
        text::put(
            buf,
            l.msg.x,
            l.msg.y + row as u16,
            &app.messages[k as usize],
            style,
        );
    }
}

pub fn draw_status(buf: &mut Buffer, l: &Layout, app: &App) {
    buf.set_style(l.status, Style::new().bg(STATUS_BG));
    let y = l.status.y;
    let mut x = l.status.x;
    let sep = Style::new().fg(DIM);
    let mut write = |s: &str, style: Style| {
        let shown = text::trunc(s, (l.status.right().saturating_sub(x)) as usize);
        buf.set_string(x, y, &shown, style);
        x = (x + shown.chars().count() as u16).min(l.status.right());
    };

    if app.path.is_empty() {
        write(" no file", Style::new().fg(DIM));
    } else {
        write(" ", Style::new().fg(Color::Cyan));
        write(&text::trunc(&app.path, 30), Style::new().fg(Color::Cyan));
    }
    write(" | ", sep);
    if let Some(wf) = &app.wf {
        write(
            &format!("timescale {}", wf.ts.label()),
            Style::new().fg(Color::White),
        );
        write(" | ", sep);
        write("cursor ", Style::new().fg(DIM));
        write(
            &waveform::format_time(app.cursor as f64, &wf.ts),
            Style::new().fg(CURSOR),
        );
        if let Some((a, b)) = app.range {
            write(" | ", sep);
            write("ΔT ", Style::new().fg(DIM));
            write(
                &waveform::format_time((b - a) as f64, &wf.ts),
                Style::new().fg(Color::Yellow),
            );
        }
        write(" | ", sep);
        write("zoom ", Style::new().fg(DIM));
        write(
            &format!("{}/char", waveform::format_time(app.scale, &wf.ts)),
            Style::new().fg(Color::White),
        );
        write(" | ", sep);
        write("signals ", Style::new().fg(DIM));
        write(&app.display.len().to_string(), Style::new().fg(HIGH));
        write(" | ", sep);
        write("focus ", Style::new().fg(DIM));
        write(
            app.focus.name(),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        );
    }
}

pub fn draw_empty(buf: &mut Buffer, l: &Layout) {
    let lines = [
        "waverdi — Verdi-style terminal RTL waveform viewer",
        "Press 'o' to open a VCD/FST dump, or run: waverdi <file>",
        "F1 / '?' for key bindings",
    ];
    for (i, line) in lines.iter().enumerate() {
        let style = if i == 0 {
            Style::new().fg(ACCENT)
        } else {
            Style::new().fg(Color::White)
        };
        text::put(buf, l.wave.x + 2, l.wave.y + 4 + i as u16, line, style);
    }
    text::put(
        buf,
        l.list.x + 1,
        l.list.y + 3,
        "no waveform",
        Style::new().fg(DIM),
    );
}
