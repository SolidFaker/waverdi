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

const KEYS: &[&str] = &[
    "General   q quit   o open (system dialog when available)",
    "          O open browser   : goto time   g: ge / gg / G",
    "          s search signal   F1 / ? help",
    "Search    v find value (hex/bin/oct/dec/ascii text)",
    "          n / N next / previous match (wraps around)",
    "View      z / Z / - / = zoom   f fit   c center",
    "          h / l move cursor   ← → / Shift+← → x10",
    "          j / k next / previous row   gg / G first / last",
    "          J / K move signal down / up   Space select",
    "          w / b next / previous edge (1-bit: rising)",
    "          e / ge next / previous falling edge",
    "          0 / $ start / end of time   , / . prev / next change",
    "          Time button in the nWave bar cycles the time base",
    "          Home / End start / end   Tab cycle focus",
    "Signals   a / Enter add (Instance)   V visual   x cut (nWave)",
    "          dd cut selection   p paste below   Esc clear",
    "          r cycle radix / rename group   h full names",
    "          Shift/Alt/Space multi-select   Ctrl+click range",
    "          Shift+↑↓ extend selection   right-click: menu",
    "Groups    new signals go to the cursor group; G0 is the default",
    "          adding to the newest group appends a fresh group",
    "          Enter / ← → collapse / expand   r: rename group",
    "          right-click: New Group / Rename / Expand / Remove",
    "          ↑ ↓ / PgUp PgDn navigate lists",
    "Mouse     menus / toolbar: click to execute",
    "          drag pane borders: resize panes",
    "          drag scrollbars: scroll / pan time",
    "          drag list rows: reorder signals",
    "          Instance: Shift/Alt+click multi-select, double add all",
    "          click ruler: cursor   drag waveform: range",
    "          click inside a selection: zoom to it",
    "          wheel over waveform: zoom   over lists: scroll",
    "          dialogs: ✕ closes, scrollbars are draggable",
];

const OPEN_W: u16 = 76;
const OPEN_H: u16 = 20;

/// Geometry of the built-in TUI file browser dialog.
pub fn open_rect(screen: Rect) -> Rect {
    let width = OPEN_W
        .min(screen.width.saturating_sub(4))
        .max(30)
        .min(screen.width);
    let height = OPEN_H
        .min(screen.height.saturating_sub(4))
        .max(7)
        .min(screen.height);
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

/// Outer rectangle of a dialog box.
pub fn dialog_rect(screen: Rect, app: &App, dialog: Dialog) -> Rect {
    if dialog == Dialog::Open {
        return open_rect(screen);
    }
    let wanted_height = match dialog {
        Dialog::Find => 14,
        Dialog::Keys => KEYS.len() as u16 + 4,
        Dialog::CreateBus => app
            .bus_builder()
            .map(|builder| builder.items.len() as u16 + 4)
            .unwrap_or(8),
        _ => 7,
    };
    let width = 64u16
        .min(screen.width.saturating_sub(8))
        .max(28)
        .min(screen.width);
    let height = wanted_height
        .min(screen.height.saturating_sub(4))
        .min(screen.height);
    Rect {
        x: screen.x + screen.width.saturating_sub(width) / 2,
        y: screen.y + screen.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

/// Body rows a dialog shows, used for paging and scrolling.
pub fn visible_rows(screen: Rect, app: &App, dialog: Dialog) -> usize {
    let area = dialog_rect(screen, app, dialog);
    let reserved = match dialog {
        Dialog::Find => 5,
        _ => 2,
    };
    (area.height as usize).saturating_sub(reserved).max(1)
}
/// Right-edge scrollbar for a scrolled list.
fn draw_scrollbar(buf: &mut Buffer, x: u16, top: u16, rows: usize, scroll: usize, total: usize) {
    if rows == 0 || total <= rows {
        return;
    }
    let thumb = ((rows as f64 / total as f64) * rows as f64).max(1.0) as usize;
    let track = (total - rows).max(1);
    let offset = ((scroll.min(track) as f64 / track as f64) * (rows.saturating_sub(thumb)) as f64)
        .round() as usize;
    for row in 0..rows {
        let symbol = if row >= offset && row < offset + thumb {
            "█"
        } else {
            "│"
        };
        text::set_cell(buf, x, top + row as u16, symbol, DIM, BG);
    }
}

/// Scrollbar geometry of a scrollable dialog, used for drawing and clicking.
pub struct ScrollArea {
    pub bar: Rect,
    pub rows: usize,
    pub total: usize,
}

/// Close button (top-right `✕`) of a dialog box.
pub fn close_button(screen: Rect, app: &App, dialog: Dialog) -> Rect {
    let area = dialog_rect(screen, app, dialog);
    Rect {
        x: area.right().saturating_sub(4),
        y: area.y,
        width: 3,
        height: 1,
    }
}

fn keys_line_count(inner_w: usize) -> usize {
    KEYS.iter()
        .map(|line| text::wrap(line, inner_w).len())
        .sum()
}

/// Scrollable body of a dialog: `None` when everything fits.
pub fn scroll_area(screen: Rect, app: &App, dialog: Dialog) -> Option<ScrollArea> {
    let area = dialog_rect(screen, app, dialog);
    let (top, rows, total) = match dialog {
        Dialog::Keys => (
            area.y + 1,
            area.height.saturating_sub(2) as usize,
            keys_line_count(area.width.saturating_sub(4) as usize),
        ),
        Dialog::Find => (
            area.y + 4,
            (area.height as usize).saturating_sub(5),
            app.find_matches().len(),
        ),
        Dialog::CreateBus => (
            area.y + 1,
            area.height.saturating_sub(4) as usize,
            app.bus_builder().map(|b| b.items.len()).unwrap_or(0),
        ),
        _ => return None,
    };
    if rows == 0 || total <= rows {
        return None;
    }
    Some(ScrollArea {
        bar: Rect {
            x: area.right().saturating_sub(2),
            y: top,
            width: 1,
            height: rows as u16,
        },
        rows,
        total,
    })
}

pub fn draw(frame: &mut Frame, l: &Layout, app: &App, dialog: Dialog) {
    let title = match dialog {
        Dialog::Open => "Open Waveform",
        Dialog::Goto => "Go to Time",
        Dialog::Find => "Search Signal",
        Dialog::FindValue => "Find Value",
        Dialog::SplitBus => "Split Bus",
        Dialog::CreateBus => "Create Bus",
        Dialog::GroupName => "Rename Group",
        Dialog::Keys => "Key Bindings",
        Dialog::About => "About",
    };
    let area = dialog_rect(l.area, app, dialog);

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
        // Close button at the top-right of the frame.
        let close = close_button(l.area, app, dialog);
        text::put(
            buf,
            close.x,
            close.y,
            " ✕ ",
            Style::new().fg(XCOL).add_modifier(Modifier::BOLD),
        );

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
            Dialog::SplitBus => {
                cursor = draw_input(
                    buf,
                    area,
                    inner_x,
                    inner_w,
                    app,
                    (
                        "Width: ",
                        "Enter: split into chunks of this width    Esc: cancel",
                    ),
                );
            }
            Dialog::CreateBus => draw_bus_builder(buf, area, app),
            Dialog::GroupName => {
                cursor = draw_input(
                    buf,
                    area,
                    inner_x,
                    inner_w,
                    app,
                    ("Name: ", "Enter: rename group    Esc: cancel"),
                );
            }
            Dialog::Keys => {
                let inner_w = area.width.saturating_sub(4) as usize;
                let mut lines: Vec<String> = Vec::new();
                for line in KEYS {
                    lines.extend(text::wrap(line, inner_w));
                }
                let rows = area.height.saturating_sub(2) as usize;
                let max_scroll = lines.len().saturating_sub(rows);
                let scroll = app.dialog_scroll.min(max_scroll);
                for (i, line) in lines.iter().skip(scroll).take(rows).enumerate() {
                    text::put(
                        buf,
                        inner_x,
                        area.y + 1 + i as u16,
                        line,
                        Style::new().fg(Color::White),
                    );
                }
                draw_scrollbar(
                    buf,
                    area.right().saturating_sub(2),
                    area.y + 1,
                    rows,
                    scroll,
                    lines.len(),
                );
            }
            Dialog::About => {
                let lines = [
                    "waverdi 0.1 — Verdi-style terminal RTL waveform viewer",
                    "formats: VCD, FST (FSDB via the Verdi FFR library)",
                    "ratatui + crossterm on Rust",
                ];
                let inner_w = area.width.saturating_sub(4) as usize;
                let mut row = 0u16;
                for line in lines {
                    for part in text::wrap(line, inner_w) {
                        text::put(
                            buf,
                            inner_x,
                            area.y + 2 + row,
                            &part,
                            Style::new().fg(Color::White),
                        );
                        row += 1;
                    }
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
    let label_width = area.width.saturating_sub(4) as usize;
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
        text::put(
            buf,
            area.x + 2,
            area.y + 3 + row as u16,
            &text::trunc(&label, label_width),
            style,
        );
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

fn draw_bus_builder(buf: &mut Buffer, area: Rect, app: &App) {
    let Some(builder) = app.bus_builder() else {
        text::put(
            buf,
            area.x + 2,
            area.y + 2,
            "no bus builder state",
            Style::new().fg(XCOL),
        );
        return;
    };
    let inner = area.width.saturating_sub(2) as usize;
    let count = builder.items.len();
    let total: u32 = builder.items.iter().map(|item| item.width()).sum();
    let rows = area.height.saturating_sub(4) as usize;
    let max_scroll = count.saturating_sub(rows);
    let mut scroll = app.dialog_scroll.min(max_scroll);
    if builder.sel >= scroll + rows {
        scroll = builder.sel + 1 - rows;
    } else if builder.sel < scroll {
        scroll = builder.sel;
    }
    for (k, item) in builder.items.iter().enumerate().skip(scroll).take(rows) {
        let Some(signal) = app.wf.as_ref().and_then(|wf| wf.signals.get(item.sig)) else {
            continue;
        };
        let range = if signal.bits <= 1 {
            String::new()
        } else {
            format!(" [{}:{}]", item.hi, item.lo)
        };
        let marker = if k == 0 {
            " (MSB)"
        } else if k + 1 == count {
            " (LSB)"
        } else {
            ""
        };
        let label = format!(
            "{:>2}. {}{range} ({}b){marker}",
            k + 1,
            signal.full_name(),
            item.width()
        );
        let style = if k == builder.sel {
            Style::new().fg(Color::Black).bg(ACCENT)
        } else {
            Style::new().fg(Color::White)
        };
        text::put(
            buf,
            area.x + 2,
            area.y + 1 + (k - scroll) as u16,
            &text::trunc(&label, inner),
            style,
        );
    }
    draw_scrollbar(
        buf,
        area.right().saturating_sub(2),
        area.y + 1,
        rows,
        scroll,
        count,
    );
    let hint_top = " ↑↓ select   ⇧↑↓ reorder   h/l LSB   H/L MSB   x full range";
    let hint_bottom = format!(" s/S sort   r rev   Enter create   Esc cancel   {total} bits ");
    text::put(
        buf,
        area.x + 2,
        area.bottom().saturating_sub(3),
        &text::trunc(hint_top, inner),
        Style::new().fg(DIM),
    );
    text::put(
        buf,
        area.x + 2,
        area.bottom().saturating_sub(2),
        &text::trunc(&hint_bottom, inner),
        Style::new().fg(DIM),
    );
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
    let avail = area.right().saturating_sub(input_x) as usize;
    let value = app.input.as_string();
    let shown = text::trunc(&value, width.min(avail));
    text::put(
        buf,
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
    text::put(
        buf,
        x,
        area.y + 4,
        &text::trunc(hint, width),
        Style::new().fg(DIM),
    );
    Some(Position::new(input_x + caret, area.y + 2))
}

fn draw_find(buf: &mut Buffer, area: Rect, x: u16, width: usize, app: &App) -> Option<Position> {
    // Query row, hint row, then the scrolling match list.
    text::put(buf, x, area.y + 1, "Query: ", Style::new().fg(Color::White));
    let input_x = x + 7;
    let avail = area.right().saturating_sub(input_x) as usize;
    let shown = text::trunc(&app.input.as_string(), width.min(avail));
    text::put(
        buf,
        input_x,
        area.y + 1,
        &shown,
        Style::new().fg(Color::White).bg(INPUT_BG),
    );
    let caret = app.input.cursor().min(shown.chars().count()) as u16;
    if let Some(cell) = buf.cell_mut((input_x + caret, area.y + 1)) {
        cell.set_bg(Color::White);
        cell.set_fg(Color::Black);
    }
    text::put(
        buf,
        x,
        area.y + 2,
        "Enter: add signal    ↑/↓: navigate    Esc: cancel",
        Style::new().fg(DIM),
    );

    let matches = app.find_matches();
    let selected = app.find_sel.min(matches.len().saturating_sub(1));
    let rows = (area.height as usize).saturating_sub(5);
    let max_scroll = matches.len().saturating_sub(rows);
    let mut scroll = app.dialog_scroll.min(max_scroll);
    if selected >= scroll + rows {
        scroll = selected + 1 - rows;
    } else if selected < scroll {
        scroll = selected;
    }
    if let Some(wf) = &app.wf {
        for (k, &sig) in matches.iter().enumerate().skip(scroll).take(rows) {
            let name = wf.signals[sig].full_name();
            let style = if k == selected {
                Style::new().fg(Color::Black).bg(ACCENT)
            } else {
                Style::new().fg(Color::White)
            };
            text::put(
                buf,
                x,
                area.y + 4 + (k - scroll) as u16,
                &text::trunc(&name, width.saturating_sub(1)),
                style,
            );
        }
    }
    draw_scrollbar(
        buf,
        area.right().saturating_sub(2),
        area.y + 4,
        rows,
        scroll,
        matches.len(),
    );
    Some(Position::new(input_x + caret, area.y + 1))
}
