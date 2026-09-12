use crate::app::{App, CtxEntry};
use crate::theme::Theme;
use crate::ui::layout::Layout;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Clear, Widget as _};

/// What the pointer hit inside an open context menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CtxHit {
    Root(usize),
    Sub(usize),
}

fn entries_width(entries: &[CtxEntry]) -> u16 {
    entries
        .iter()
        .map(|entry| {
            let extra = if matches!(entry, CtxEntry::Submenu(..)) {
                3
            } else {
                0
            };
            entry.label().chars().count() + extra
        })
        .max()
        .unwrap_or(10) as u16
        + 4
}

pub fn root_rect(l: &Layout, app: &App) -> Rect {
    let menu = app.ctx_menu.as_ref().expect("context menu open");
    let entries = app.ctx_root();
    let width = entries_width(entries);
    let height = entries.len() as u16 + 2;
    let x = menu
        .x
        .min(l.area.right().saturating_sub(width))
        .max(l.area.x);
    let y = menu
        .y
        .min(l.area.bottom().saturating_sub(height))
        .max(l.area.y);
    Rect {
        x,
        y,
        width,
        height,
    }
}

pub fn sub_rect(l: &Layout, app: &App, root: Rect, index: usize) -> Option<Rect> {
    let entry = app.ctx_root().get(index)?;
    let entries = match entry {
        CtxEntry::Submenu(_, entries) => *entries,
        CtxEntry::Item(..) => return None,
    };
    let width = entries_width(entries);
    let height = entries.len() as u16 + 2;
    let y = (root.y + 1 + index as u16)
        .min(l.area.bottom().saturating_sub(height))
        .max(l.area.y);
    let right = root.right().saturating_sub(1);
    let x = if right + width <= l.area.right() {
        right
    } else {
        root.x.saturating_sub(width.saturating_sub(1))
    };
    Some(Rect {
        x,
        y,
        width,
        height,
    })
}

fn hit(area: Rect, col: u16, row: u16, count: usize) -> Option<usize> {
    if col <= area.x || col >= area.right().saturating_sub(1) {
        return None;
    }
    if row <= area.y || row >= area.bottom().saturating_sub(1) {
        return None;
    }
    let index = (row - area.y - 1) as usize;
    (index < count).then_some(index)
}

pub fn item_at(app: &App, col: u16, row: u16) -> Option<CtxHit> {
    let menu = app.ctx_menu.as_ref()?;
    let l = app.layout();
    let root = root_rect(&l, app);
    if let Some(index) = menu.submenu {
        if let Some(area) = sub_rect(&l, app, root, index) {
            let entries = app.ctx_level();
            if let Some(item) = hit(area, col, row, entries.len()) {
                return Some(CtxHit::Sub(item));
            }
        }
    }
    hit(root, col, row, app.ctx_root().len()).map(CtxHit::Root)
}

fn draw_popup(buf: &mut Buffer, area: Rect, entries: &[CtxEntry], sel: usize, t: &Theme) {
    Clear.render(area, buf);
    buf.set_style(area, Style::new().bg(t.popup_bg));
    Block::bordered()
        .border_style(Style::new().fg(t.accent))
        .render(area, buf);
    let inner = area.width.saturating_sub(2);
    for (k, entry) in entries.iter().enumerate() {
        let selected = k == sel;
        let style = if selected {
            Style::new().fg(Color::Black).bg(t.accent)
        } else {
            Style::new().fg(t.text)
        };
        let y = area.y + 1 + k as u16;
        if selected {
            // Highlight the whole row, not only the label.
            buf.set_style(
                Rect {
                    x: area.x + 1,
                    y,
                    width: inner,
                    height: 1,
                },
                Style::new().bg(t.accent),
            );
        }
        text::put(buf, area.x + 1, y, entry.label(), style);
        if matches!(entry, CtxEntry::Submenu(..)) {
            let arrow_x = area.x + 1 + inner.saturating_sub(3);
            text::put(buf, arrow_x, y, " ▸", style);
        }
    }
}

pub fn draw(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    let Some(menu) = &app.ctx_menu else { return };
    let root = root_rect(l, app);
    draw_popup(
        buf,
        root,
        app.ctx_root(),
        menu.submenu.unwrap_or(menu.sel),
        t,
    );

    if let Some(index) = menu.submenu {
        if let Some(area) = sub_rect(l, app, root, index) {
            draw_popup(buf, area, app.ctx_level(), menu.sel, t);
        }
    }
}
