use crate::app::App;
use crate::theme::*;
use crate::ui::layout::Layout;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Widget as _};

/// Bordered frame of the RTL source pane.
pub fn draw_frame(buf: &mut Buffer, l: &Layout) {
    Block::bordered()
        .title(" Source ")
        .title_style(Style::new().fg(Color::White).add_modifier(Modifier::BOLD))
        .border_style(Style::new().fg(PANEL_BORDER))
        .render(l.source, buf);
}

/// Placeholder body: shows the selected instance until a source loader exists.
pub fn draw(buf: &mut Buffer, l: &Layout, app: &App) {
    let inner = ratatui::layout::Rect {
        x: l.source.x + 2,
        y: l.source.y + 1,
        width: l.source.width.saturating_sub(4),
        height: l.source.height.saturating_sub(2),
    };
    text::put(
        buf,
        inner.x,
        inner.y,
        "RTL source view",
        Style::new().fg(DIM).add_modifier(Modifier::BOLD),
    );
    let selected = app.selected_scope_path().or_else(|| {
        app.wf
            .as_ref()
            .map(|wf| wf.tree.nodes[wf.tree.root].name.clone())
    });
    if let Some(path) = selected {
        text::put(
            buf,
            inner.x,
            inner.y + 1,
            &format!("instance: {path}"),
            Style::new().fg(Color::Cyan),
        );
    }
    text::put(
        buf,
        inner.x,
        inner.y + 3,
        "(source loading is not implemented yet)",
        Style::new().fg(DIM),
    );
}
