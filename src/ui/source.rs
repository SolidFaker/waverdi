use crate::app::App;
use crate::theme::Theme;
use crate::ui::layout::Layout;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Widget as _};

/// Bordered frame of the RTL source pane.
pub fn draw_frame(buf: &mut Buffer, l: &Layout, t: &Theme) {
    Block::bordered()
        .title(" Source ")
        .title_style(Style::new().fg(t.text).add_modifier(Modifier::BOLD))
        .border_style(Style::new().fg(t.panel_border))
        .render(l.source, buf);
}

/// Placeholder body: shows the selected instance until a source loader exists.
pub fn draw(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
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
        Style::new().fg(t.dim).add_modifier(Modifier::BOLD),
    );
    let selected = app.selected_scope_path().or_else(|| {
        app.wf
            .as_ref()
            .map(|wf| wf.tree.nodes[wf.tree.root].name.clone())
    });
    if let Some(path) = selected {
        let module = app.selected_scope_module().unwrap_or_default();
        let label = if module.is_empty() || module == path {
            format!("instance: {path}")
        } else {
            format!("instance: {path}   [module {module}]")
        };
        text::put(buf, inner.x, inner.y + 1, &label, Style::new().fg(t.path));
    }
    text::put(
        buf,
        inner.x,
        inner.y + 3,
        "(source loading is not implemented yet)",
        Style::new().fg(t.dim),
    );
}
