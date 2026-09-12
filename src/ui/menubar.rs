use crate::app::{Action, App};
use crate::ui::layout::Layout;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Clear, Widget as _};

pub const MENUS: [(&str, &[(&str, Action)]); 5] = [
    (
        "File",
        &[
            ("Open Waveform...", Action::Open),
            ("Open in TUI Browser...", Action::OpenTui),
            ("Quit", Action::Quit),
        ],
    ),
    (
        "View",
        &[
            ("Zoom In", Action::ZoomIn),
            ("Zoom Out", Action::ZoomOut),
            ("Fit to Screen", Action::Fit),
            ("Center Cursor", Action::Center),
            ("Go to Time...", Action::Goto),
            ("Settings...", Action::Settings),
        ],
    ),
    (
        "Signal",
        &[
            ("Search Signal...", Action::Find),
            ("Find Value...", Action::FindValue),
            ("Find Next Value", Action::FindNextValue),
            ("Find Previous Value", Action::FindPrevValue),
            ("Change Radix", Action::Radix),
            ("Previous Transition", Action::Prev),
            ("Next Transition", Action::Next),
        ],
    ),
    (
        "Trace",
        &[
            ("Add Selected Signal", Action::AddSel),
            ("Remove Selected Signal", Action::DelSel),
            ("Remove All", Action::DelAll),
        ],
    ),
    (
        "Help",
        &[("Key Bindings", Action::Keys), ("About", Action::About)],
    ),
];

const BRAND: &str = " waverdi ";

pub fn menu_len(idx: usize) -> usize {
    MENUS[idx].1.len()
}

pub fn menu_action(idx: usize, item: usize) -> Action {
    MENUS[idx].1[item].1
}

fn item_offsets(menu: Rect) -> Vec<u16> {
    let mut x = menu.x + BRAND.len() as u16;
    let mut out = Vec::with_capacity(MENUS.len());
    for (name, _) in MENUS {
        out.push(x);
        x += (name.len() + 3) as u16;
    }
    out
}

pub fn menu_item_at(menu: Rect, col: u16) -> Option<usize> {
    for (i, x) in item_offsets(menu).iter().enumerate() {
        let width = (MENUS[i].0.len() + 3) as u16;
        if col >= *x && col < x + width {
            return Some(i);
        }
    }
    None
}

pub fn dropdown_rect(l: &Layout, idx: usize) -> Rect {
    let items = MENUS[idx].1;
    let width = items.iter().map(|(name, _)| name.len()).max().unwrap_or(10) + 4;
    Rect {
        x: item_offsets(l.menu)[idx],
        y: l.menu.bottom(),
        width: width as u16,
        height: items.len() as u16 + 2,
    }
}

pub fn draw_bar(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    buf.set_style(l.menu, Style::new().bg(t.menubar_bg));
    buf.set_string(
        l.menu.x,
        l.menu.y,
        BRAND,
        Style::new()
            .fg(Color::Black)
            .bg(t.accent)
            .add_modifier(Modifier::BOLD),
    );
    for (i, (name, _)) in MENUS.iter().enumerate() {
        let x = item_offsets(l.menu)[i];
        let style = if app.menu.open == Some(i) {
            Style::new().fg(t.text).bg(t.menu_active)
        } else {
            Style::new().fg(t.text)
        };
        buf.set_string(x, l.menu.y, format!(" {name} "), style);
    }
}

pub fn draw_dropdown(buf: &mut Buffer, l: &Layout, app: &App, idx: usize) {
    let t = &app.theme;
    let area = dropdown_rect(l, idx);
    Clear.render(area, buf);
    buf.set_style(area, Style::new().bg(t.popup_bg));
    Block::bordered()
        .border_style(Style::new().fg(t.accent))
        .render(area, buf);
    for (k, (name, _)) in MENUS[idx].1.iter().enumerate() {
        let selected = k == app.menu.sel;
        let style = if selected {
            Style::new().fg(Color::Black).bg(t.accent)
        } else {
            Style::new().fg(t.text)
        };
        text::put(buf, area.x + 1, area.y + 1 + k as u16, name, style);
    }
}
