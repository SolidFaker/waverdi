use crate::app::{App, Dialog};
use crate::theme::*;
use crate::ui::layout::Layout;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Widget as _};
use ratatui::Frame;

const KEYS: [&str; 16] = [
    "General   q quit   o open (system dialog)   g goto time",
    "          s search   F1 / ? help",
    "View      z / Z zoom in / out   f fit   c center",
    "          ← → move cursor   Shift+← → x10",
    "          , / . prev / next transition (selected signal)",
    "          Home / End to start / end   Tab cycle focus",
    "Signals   a / Enter add (nTrace)   d remove   x remove all",
    "          r cycle radix (Bin/Oct/Dec/Hex/Ascii)",
    "          ↑ ↓ / PgUp PgDn navigate lists",
    "Mouse     menus / toolbar: click to execute",
    "          drag pane borders: resize panes",
    "          drag scrollbars: scroll / pan time",
    "          click ruler: cursor   drag waveform: range",
    "          wheel: zoom   shift+wheel: pan   middle: zoom out",
    "          wheel over nTrace / Signal List: scroll",
    "          double click: expand scope / add signal",
];

pub fn draw(frame: &mut Frame, l: &Layout, app: &App, dialog: Dialog) {
    let (title, wanted_height) = match dialog {
        Dialog::Goto => ("Go to Time", 7),
        Dialog::Find => ("Search Signal", 13),
        Dialog::Keys => ("Key Bindings", KEYS.len() as u16 + 4),
        Dialog::About => ("About", 7),
    };
    let width = 64u16.min(l.area.width.saturating_sub(8)).max(28);
    let height = wanted_height.min(l.area.height.saturating_sub(4));
    let area = Rect {
        x: l.area.x + l.area.width.saturating_sub(width) / 2,
        y: l.area.y + l.area.height.saturating_sub(height) / 2,
        width,
        height,
    };

    let mut cursor = None;
    {
        let buf = frame.buffer_mut();
        buf.set_style(l.area, Style::new().bg(OVERLAY));
        Block::bordered()
            .title(format!(" {title} "))
            .title_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
            .border_style(Style::new().fg(ACCENT))
            .render(area, buf);

        let inner_x = area.x + 2;
        let inner_w = area.width.saturating_sub(4) as usize;
        match dialog {
            Dialog::Goto => {
                cursor = draw_input(
                    buf,
                    area,
                    inner_x,
                    inner_w,
                    app,
                    ("Time: ", "Enter: jump (e.g. 1500, 1.5us)    Esc: cancel"),
                );
            }
            Dialog::Find => {
                cursor = draw_find(buf, area, inner_x, inner_w, app);
            }
            Dialog::Keys => {
                for (i, line) in KEYS.iter().enumerate() {
                    text::put(
                        buf,
                        inner_x,
                        area.y + 1 + i as u16,
                        line,
                        Style::new().fg(Color::White),
                    );
                }
            }
            Dialog::About => {
                let lines = [
                    "waverdi 0.1 — Verdi-style terminal RTL waveform viewer",
                    "format: VCD (FSDB support planned)",
                    "ratatui + crossterm on Rust",
                ];
                for (i, line) in lines.iter().enumerate() {
                    text::put(
                        buf,
                        inner_x,
                        area.y + 2 + i as u16,
                        line,
                        Style::new().fg(Color::White),
                    );
                }
            }
        }
    }
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
}

fn draw_input(
    buf: &mut Buffer,
    area: Rect,
    x: u16,
    width: usize,
    app: &App,
    labels: (&str, &str),
) -> Option<Position> {
    let (label, hint) = labels;
    text::put(buf, x, area.y + 2, label, Style::new().fg(Color::White));
    let input_x = x + label.chars().count() as u16;
    let value = app.input.as_string();
    let shown = text::trunc(&value, width);
    buf.set_string(
        input_x,
        area.y + 2,
        &shown,
        Style::new().fg(Color::White).bg(INPUT_BG),
    );
    let caret = app.input.cursor().min(shown.chars().count()) as u16;
    if let Some(cell) = buf.cell_mut((input_x + caret, area.y + 2)) {
        cell.set_bg(Color::White);
        cell.set_fg(Color::Black);
    }
    text::put(buf, x, area.y + 4, hint, Style::new().fg(DIM));
    Some(Position::new(input_x + caret, area.y + 2))
}

fn draw_find(buf: &mut Buffer, area: Rect, x: u16, width: usize, app: &App) -> Option<Position> {
    let cursor = draw_input(
        buf,
        area,
        x,
        width,
        app,
        (
            "Query: ",
            "Enter: add signal    ↑/↓: navigate    Esc: cancel",
        ),
    );
    let matches = app.find_matches();
    let selected = app.find_sel.min(matches.len().saturating_sub(1));
    let rows = (area.height as usize).saturating_sub(6);
    if let Some(wf) = &app.wf {
        for (row, &sig) in matches.iter().enumerate().take(rows) {
            let name = wf.signals[sig].full_name();
            let style = if row == selected {
                Style::new().fg(Color::Black).bg(ACCENT)
            } else {
                Style::new().fg(Color::White)
            };
            text::put(buf, x, area.y + 3 + row as u16, &name, style);
        }
    }
    cursor
}
