use crate::app::App;
use crate::theme::Theme;
use crate::ui::layout::Layout;
use crate::ui::text;
use crate::waveform;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

pub fn draw_messages(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    buf.set_style(l.msg, Style::new().bg(t.msg_bg));
    let n = app.messages.len();
    for row in 0..3 {
        let k = n as i64 - 3 + row as i64;
        if k < 0 || k >= n as i64 {
            continue;
        }
        let style = if row == 2 {
            Style::new().fg(t.text)
        } else {
            Style::new().fg(t.dim)
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
    let t = &app.theme;
    buf.set_style(l.status, Style::new().bg(t.status_bg));
    let y = l.status.y;
    let mut x = l.status.x;
    let sep = Style::new().fg(t.dim);
    let mut write = |s: &str, style: Style| {
        let shown = text::trunc(s, (l.status.right().saturating_sub(x)) as usize);
        buf.set_string(x, y, &shown, style);
        x = (x + shown.chars().count() as u16).min(l.status.right());
    };

    if app.path.is_empty() {
        write(" no file", Style::new().fg(t.dim));
    } else {
        // Right-aligned path box: keep the file name (tail) readable when the
        // path is too long, and shrink the box on narrow terminals.
        const PATH_W: usize = 30;
        let avail = l.status.right().saturating_sub(l.status.x + 1) as usize;
        let width = avail.min(PATH_W);
        let shown = text::trunc_left(&app.path, width);
        let pad = width.saturating_sub(shown.chars().count());
        write(" ", Style::new().fg(t.path));
        write(&" ".repeat(pad), Style::new().fg(t.path));
        write(&shown, Style::new().fg(t.path));
    }
    write(" | ", sep);
    if let Some(wf) = &app.wf {
        write(
            &format!("timescale {}", wf.ts.label()),
            Style::new().fg(t.text),
        );
        write(" | ", sep);
        write("cursor ", Style::new().fg(t.dim));
        write(
            &waveform::format_time_base(app.cursor as f64, &wf.ts, app.time_base),
            Style::new().fg(t.cursor),
        );
        if let Some((a, b)) = app.range {
            write(" | ", sep);
            write("ΔT ", Style::new().fg(t.dim));
            write(
                &waveform::format_time_base((b - a) as f64, &wf.ts, app.time_base),
                Style::new().fg(t.scope),
            );
        }
        write(" | ", sep);
        write("zoom ", Style::new().fg(t.dim));
        write(
            &format!(
                "{}/char",
                waveform::format_time_base(app.scale, &wf.ts, app.time_base)
            ),
            Style::new().fg(t.text),
        );
        write(" | ", sep);
        write("signals ", Style::new().fg(t.dim));
        write(&app.display.len().to_string(), Style::new().fg(t.high));
        if !app.selection.is_empty() {
            write(" | ", sep);
            write("sel ", Style::new().fg(t.dim));
            write(
                &app.selection.len().to_string(),
                Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
            );
        }
        if app.visual {
            write(" | ", sep);
            write(
                "VISUAL",
                Style::new()
                    .fg(Color::Black)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD),
            );
        }
        if !app.register.is_empty() {
            write(" | ", sep);
            write("reg ", Style::new().fg(t.dim));
            write(&app.register.len().to_string(), Style::new().fg(t.scope));
        }
        write(" | ", sep);
        write("focus ", Style::new().fg(t.dim));
        write(
            app.focus.name(),
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
        );
    }
}

pub fn draw_empty(buf: &mut Buffer, l: &Layout, t: &Theme) {
    let lines = [
        "waverdi — Verdi-style terminal RTL waveform viewer",
        "Press 'o' to open a VCD/FST dump, or run: waverdi <file>",
        "F1 / '?' for key bindings",
    ];
    for (i, line) in lines.iter().enumerate() {
        let style = if i == 0 {
            Style::new().fg(t.accent)
        } else {
            Style::new().fg(t.text)
        };
        text::put(buf, l.wave.x + 2, l.wave.y + 4 + i as u16, line, style);
    }
    text::put(
        buf,
        l.list.x + 1,
        l.list.y + 3,
        "no waveform",
        Style::new().fg(t.dim),
    );
}
