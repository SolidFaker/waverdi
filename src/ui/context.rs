use crate::app::{App, ContextMenu, CtxItem, CtxTarget};
use crate::theme::*;
use crate::ui::layout::Layout;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Clear, Widget as _};

fn rect(l: &Layout, items: &[(&str, CtxItem)], menu: &ContextMenu) -> Rect {
    let width = items
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(10) as u16
        + 4;
    let height = items.len() as u16 + 2;
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

pub fn item_at(app: &App, col: u16, row: u16) -> Option<usize> {
    let menu = app.ctx_menu.as_ref()?;
    let l = app.layout();
    let items = app.ctx_items();
    let area = rect(&l, items, menu);
    if col <= area.x || col >= area.right().saturating_sub(1) {
        return None;
    }
    if row <= area.y || row >= area.bottom().saturating_sub(1) {
        return None;
    }
    let index = (row - area.y - 1) as usize;
    (index < items.len()).then_some(index)
}

pub fn draw(buf: &mut Buffer, l: &Layout, app: &App) {
    let Some(menu) = &app.ctx_menu else { return };
    let items = app.ctx_items();
    let area = rect(l, items, menu);
    Clear.render(area, buf);
    buf.set_style(area, Style::new().bg(POPUP_BG));

    let title = match &menu.target {
        CtxTarget::Signal(sig) => app
            .wf
            .as_ref()
            .and_then(|wf| wf.signals.get(*sig))
            .map(|sig| sig.full_name())
            .unwrap_or_default(),
        CtxTarget::Group(path) => path.clone(),
    };
    Block::bordered()
        .title(format!(
            " {} ",
            text::trunc(&title, area.width.saturating_sub(2) as usize)
        ))
        .title_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
        .border_style(Style::new().fg(ACCENT))
        .render(area, buf);

    for (k, (name, _)) in items.iter().enumerate() {
        let selected = k == menu.sel;
        let style = if selected {
            Style::new().fg(Color::Black).bg(ACCENT)
        } else {
            Style::new().fg(Color::White)
        };
        text::put(buf, area.x + 1, area.y + 1 + k as u16, name, style);
    }
}
