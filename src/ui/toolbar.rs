use crate::app::{Action, App};
use crate::ui::layout::Layout;
use crate::ui::text;
use crate::waveform::TimeBase;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Clear, Widget as _};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Add,
    ZoomIn,
    ZoomOut,
    Fit,
    Center,
    Goto,
    Find,
    Prev,
    Next,
    TimeBase,
}

impl Tool {
    fn action(self) -> Action {
        match self {
            Tool::Add => Action::AddSignals,
            Tool::ZoomIn => Action::ZoomIn,
            Tool::ZoomOut => Action::ZoomOut,
            Tool::Fit => Action::Fit,
            Tool::Center => Action::Center,
            Tool::Goto => Action::Goto,
            Tool::Find => Action::Find,
            Tool::Prev => Action::Prev,
            Tool::Next => Action::Next,
            Tool::TimeBase => Action::Fit,
        }
    }
}

const TOOLS: [(&str, Tool); 10] = [
    ("Add", Tool::Add),
    ("Zoom In", Tool::ZoomIn),
    ("Zoom Out", Tool::ZoomOut),
    ("Fit", Tool::Fit),
    ("Center", Tool::Center),
    ("Goto", Tool::Goto),
    ("Find", Tool::Find),
    ("Prev Tr", Tool::Prev),
    ("Next Tr", Tool::Next),
    ("Time", Tool::TimeBase),
];

fn tool_label(tool: Tool, app: &App) -> String {
    match tool {
        Tool::TimeBase => format!("Time: {} ▾", app.time_base_label()),
        _ => TOOLS
            .iter()
            .find(|(_, t)| *t == tool)
            .map(|(name, _)| (*name).to_string())
            .unwrap_or_default(),
    }
}

fn toolbar_rects(tb: Rect, app: &App) -> Vec<(Tool, Rect)> {
    let mut x = tb.x + 1;
    let mut out = Vec::with_capacity(TOOLS.len());
    for (_, tool) in TOOLS {
        let width = (tool_label(tool, app).chars().count() + 2) as u16;
        out.push((
            tool,
            Rect {
                x,
                y: tb.y,
                width,
                height: 1,
            },
        ));
        x += width + 1;
    }
    out
}

pub fn tool_at(tb: Rect, app: &App, col: u16) -> Option<Tool> {
    toolbar_rects(tb, app)
        .into_iter()
        .find(|(_, r)| col >= r.x && col < r.right())
        .map(|(tool, _)| tool)
}

/// Rectangle of the Time button in the shortcut bar.
pub fn time_button(tb: Rect, app: &App) -> Rect {
    toolbar_rects(tb, app)
        .into_iter()
        .find(|(tool, _)| *tool == Tool::TimeBase)
        .map(|(_, rect)| rect)
        .unwrap_or(Rect {
            x: tb.x,
            y: tb.y,
            width: 0,
            height: 1,
        })
}

/// Label of one time-base dropdown entry.
fn base_label(base: TimeBase, app: &App) -> String {
    match base {
        TimeBase::Scale => match &app.wf {
            Some(wf) => format!("ts ({})", wf.ts.label()),
            None => "ts".to_string(),
        },
        other => other.label().to_string(),
    }
}

/// Dropdown rectangle of the time-base selector.
pub fn time_menu_rect(tb: Rect, app: &App) -> Rect {
    let button = time_button(tb, app);
    let width = TimeBase::CYCLE
        .iter()
        .map(|base| base_label(*base, app).chars().count())
        .max()
        .unwrap_or(6) as u16
        + 4;
    Rect {
        x: button.x,
        y: button.y + 1,
        width,
        height: TimeBase::CYCLE.len() as u16 + 2,
    }
}

/// Draw the time-base dropdown (on top of the panes).
pub fn draw_time_menu(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    let Some(selected) = app.time_menu else {
        return;
    };
    let area = time_menu_rect(l.toolbar, app);
    Clear.render(area, buf);
    buf.set_style(area, Style::new().bg(t.popup_bg));
    Block::bordered()
        .title(" Time base ")
        .title_style(Style::new().fg(t.accent).add_modifier(Modifier::BOLD))
        .border_style(Style::new().fg(t.accent))
        .render(area, buf);
    for (i, base) in TimeBase::CYCLE.iter().enumerate() {
        let entry = base_label(*base, app);
        let style = if i == selected {
            Style::new().fg(Color::Black).bg(t.accent)
        } else {
            Style::new().fg(t.text)
        };
        text::put(buf, area.x + 1, area.y + 1 + i as u16, &entry, style);
    }
}

pub fn run_tool(app: &mut App, tool: Tool) -> bool {
    if tool == Tool::TimeBase {
        let current = TimeBase::CYCLE
            .iter()
            .position(|base| *base == app.time_base)
            .unwrap_or(0);
        app.time_menu = Some(current);
        return false;
    }
    tool.action().run(app)
}

pub fn draw(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    buf.set_style(l.toolbar, Style::new().bg(t.toolbar_bg));
    let enabled = app.wf.is_some();
    for (tool, rect) in toolbar_rects(l.toolbar, app) {
        let label = tool_label(tool, app);
        let style = if !enabled && tool != Tool::Add {
            Style::new().fg(t.dim)
        } else {
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
        };
        text::put(buf, rect.x + 1, rect.y, &label, style);
        text::put(buf, rect.right(), rect.y, "│", Style::new().fg(t.dim));
    }
}
