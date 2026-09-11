use crate::app::{App, Dialog, EntryKind};
use crate::theme::*;
use crate::ui::layout::Layout;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Widget as _};
use ratatui::Frame;

const KEYS: [&str; 23] = [
    "General   q quit   o open (system dialog when available)",
    "          O open built-in TUI browser   g goto time",
    "          s search signal   F1 / ? help",
    "Search    v find value (hex/bin/oct/dec/ascii text)",
    "          n / N next / previous match (wraps around)",
    "View      z / Z zoom in / out   f fit   c center",
    "          ← → move cursor   Shift+← → x10",
    "          , / . prev / next transition (selected signal)",
    "          Home / End to start / end   Tab cycle focus",
    "Signals   a / Enter add (nTrace)   d remove   x remove all",
    "          r cycle radix (Bin/Oct/Dec/Hex/Ascii)",
    "          right-click: radix / waveform / bus operations",
    "Groups    Enter / ← → collapse / expand a group",
    "          right-click a group: expand / collapse / remove",
    "          ↑ ↓ / PgUp PgDn navigate lists",
    "Mouse     menus / toolbar: click to execute",
    "          drag pane borders: resize panes",
    "          drag scrollbars: scroll / pan time",
    "          drag list rows: reorder signals",
    "          click ruler: cursor   drag waveform: range",
    "          click inside a selection: zoom to it",
    "          wheel over waveform: zoom   over lists: scroll",
    "          double click: expand scope / group / add signal",
];

const OPEN_W: u16 = 76;
const OPEN_H: u16 = 20;

/// Geometry of the built-in TUI file browser dialog.
pub fn open_rect(screen: Rect) -> Rect {
    let width = OPEN_W.min(screen.width.saturating_sub(4)).max(30);
    let height = OPEN_H.min(screen.height.saturating_sub(4)).max(7);
    Rect {
        x: screen.x + screen.width.saturating_sub(width) / 2,
        y: screen.y + screen.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

/// Number of file rows shown by the built-in browser.
pub fn browser_rows(screen: Rect) -> usize {
    open_rect(screen).height.saturating_sub(4) as usize
}

pub fn draw(frame: &mut Frame, l: &Layout, app: &App, dialog: Dialog) {
    let title = match dialog {
        Dialog::Open => "Open Waveform",
        Dialog::Goto => "Go to Time",
        Dialog::Find => "Search Signal",
        Dialog::FindValue => "Find Value",
        Dialog::Keys => "Key Bindings",
        Dialog::About => "About",
    };
    let area = if dialog == Dialog::Open {
        open_rect(l.area)
    } else {
        let wanted_height = match dialog {
            Dialog::Find => 13,
            Dialog::Keys => KEYS.len() as u16 + 4,
            _ => 7,
        };
        let width = 64u16.min(l.area.width.saturating_sub(8)).max(28);
        let height = wanted_height.min(l.area.height.saturating_sub(4));
        Rect {
            x: l.area.x + l.area.width.saturating_sub(width) / 2,
            y: l.area.y + l.area.height.saturating_sub(height) / 2,
            width,
            height,
        }
    };

    let mut cursor = None;
    {
        let buf = frame.buffer_mut();
        buf.set_style(l.area, Style::new().bg(OVERLAY));
        Clear.render(area, buf);
        let mut block = Block::bordered()
            .title(format!(" {title} "))
            .title_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
            .border_style(Style::new().fg(ACCENT));
        if dialog == Dialog::Open {
            block = block.title_bottom(
                Line::from(" ↑↓ move   Enter open   Backspace parent   Esc cancel ")
                    .style(Style::new().fg(DIM)),
            );
        }
        block.render(area, buf);

        let inner_x = area.x + 2;
        let inner_w = area.width.saturating_sub(4) as usize;
        match dialog {
            Dialog::Open => draw_browser(buf, area, app),
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
            Dialog::FindValue => {
                cursor = draw_input(
                    buf,
                    area,
                    inner_x,
                    inner_w,
                    app,
                    (
                        "Value: ",
                        "Enter: find next    n / N: next / prev    Esc: cancel",
                    ),
                );
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
                    "formats: VCD, FST (FSDB via the Verdi FFR library)",
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

fn draw_browser(buf: &mut Buffer, area: Rect, app: &App) {
    let Some(browser) = &app.browser else {
        text::put(
            buf,
            area.x + 2,
            area.y + 2,
            "no browser state",
            Style::new().fg(XCOL),
        );
        return;
    };

    text::put(
        buf,
        area.x + 2,
        area.y + 1,
        &browser.dir.display().to_string(),
        Style::new().fg(Color::Cyan),
    );
    let sep_w = area.width.saturating_sub(2) as usize;
    buf.set_string(
        area.x + 1,
        area.y + 2,
        "─".repeat(sep_w),
        Style::new().fg(PANEL_BORDER),
    );

    if let Some(error) = &browser.error {
        text::put(buf, area.x + 2, area.y + 3, error, Style::new().fg(XCOL));
        return;
    }

    let rows = area.height.saturating_sub(4) as usize;
    for row in 0..rows {
        let k = browser.scroll + row;
        if k >= browser.entries.len() {
            break;
        }
        let entry = &browser.entries[k];
        let selected = k == browser.sel;
        let style = if selected {
            Style::new().fg(Color::Black).bg(ACCENT)
        } else {
            match entry.kind {
                EntryKind::Parent => Style::new().fg(DIM),
                EntryKind::Dir => Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                EntryKind::File if crate::app::is_waveform(&entry.name) => Style::new().fg(HIGH),
                EntryKind::File => Style::new().fg(Color::Rgb(170, 170, 180)),
            }
        };
        let label = match entry.kind {
            EntryKind::Dir | EntryKind::Parent => format!(" {}/", entry.name),
            EntryKind::File => format!("  {}", entry.name),
        };
        text::put(buf, area.x + 2, area.y + 3 + row as u16, &label, style);
    }

    let total = browser.entries.len();
    if total > rows && rows > 0 {
        let x = area.right().saturating_sub(2);
        let thumb = ((rows as f64 / total as f64) * rows as f64).max(1.0) as usize;
        let top = ((browser.scroll as f64 / total as f64) * rows as f64) as usize;
        for row in 0..rows {
            let symbol = if row >= top && row < top + thumb {
                "█"
            } else {
                "│"
            };
            text::set_cell(buf, x, area.y + 3 + row as u16, symbol, DIM, BG);
        }
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
