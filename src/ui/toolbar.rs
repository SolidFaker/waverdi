use crate::app::{Action, App};
use crate::theme::*;
use crate::ui::layout::Layout;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Open,
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
            Tool::Open => Action::Open,
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
    ("Open", Tool::Open),
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
        Tool::TimeBase => format!("Time: {}", app.time_base_label()),
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

pub fn run_tool(app: &mut App, tool: Tool) -> bool {
    if tool == Tool::TimeBase {
        app.cycle_time_base();
        return false;
    }
    tool.action().run(app)
}

pub fn draw(buf: &mut Buffer, l: &Layout, app: &App) {
    buf.set_style(l.toolbar, Style::new().bg(TOOLBAR_BG));
    let enabled = app.wf.is_some();
    for (tool, rect) in toolbar_rects(l.toolbar, app) {
        let label = tool_label(tool, app);
        let style = if !enabled && tool != Tool::Open {
            Style::new().fg(DIM)
        } else {
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
        };
        text::put(buf, rect.x + 1, rect.y, &label, style);
        text::put(buf, rect.right(), rect.y, "│", Style::new().fg(DIM));
    }
}
