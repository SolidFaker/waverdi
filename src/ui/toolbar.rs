use crate::app::{Action, App};
use crate::ui::layout::Layout;
use crate::ui::text;
use crate::waveform::{TimeBase, TimeScale};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Clear, Widget as _};
use std::rc::Rc;

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

/// Key of the shortcut bar: its rectangle plus the two inputs of the labels.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ToolbarKey {
    tb: Rect,
    base: TimeBase,
    ts: Option<TimeScale>,
}

/// Buttons of the shortcut bar with their rectangles and labels, plus the
/// time-base dropdown entries. Rebuilt only when the key changes; drawing and
/// hit testing share one build instead of formatting every label per call.
#[derive(Default)]
pub(crate) struct ToolbarCache {
    key: Option<ToolbarKey>,
    buttons: Vec<(Tool, Rect, Rc<str>)>,
    menu: Vec<(TimeBase, Rc<str>)>,
    button: Rect,
}

impl ToolbarCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn get(&mut self, tb: Rect, app: &App) -> &Self {
        let key = ToolbarKey {
            tb,
            base: app.time_base,
            ts: app.wf.as_ref().map(|wf| wf.ts),
        };
        if self.key == Some(key) {
            return self;
        }
        self.key = Some(key);
        self.buttons.clear();
        let mut x = tb.x + 1;
        for (_, tool) in TOOLS {
            let label: Rc<str> = Rc::from(tool_label(tool, app));
            let width = (label.chars().count() + 2) as u16;
            let rect = Rect {
                x,
                y: tb.y,
                width,
                height: 1,
            };
            self.buttons.push((tool, rect, label));
            x += width + 1;
        }
        self.button = self
            .buttons
            .iter()
            .find(|(tool, _, _)| *tool == Tool::TimeBase)
            .map(|(_, rect, _)| *rect)
            .unwrap_or(Rect {
                x: tb.x,
                y: tb.y,
                width: 0,
                height: 1,
            });
        self.menu = TimeBase::CYCLE
            .iter()
            .map(|&base| (base, Rc::from(base_label(base, app))))
            .collect();
        self
    }
}

pub fn tool_at(tb: Rect, app: &App, col: u16) -> Option<Tool> {
    let mut cache = app.toolbar_cache.borrow_mut();
    cache
        .get(tb, app)
        .buttons
        .iter()
        .find(|(_, r, _)| col >= r.x && col < r.right())
        .map(|(tool, _, _)| *tool)
}

/// Rectangle of the Time button in the shortcut bar.
pub fn time_button(tb: Rect, app: &App) -> Rect {
    let mut cache = app.toolbar_cache.borrow_mut();
    cache.get(tb, app).button
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
    let mut cache = app.toolbar_cache.borrow_mut();
    let width = cache
        .get(tb, app)
        .menu
        .iter()
        .map(|(_, label)| label.chars().count())
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
    let mut cache = app.toolbar_cache.borrow_mut();
    for (i, (_, entry)) in cache.get(l.toolbar, app).menu.iter().enumerate() {
        let style = if i == selected {
            Style::new().fg(Color::Black).bg(t.accent)
        } else {
            Style::new().fg(t.text)
        };
        text::put(buf, area.x + 1, area.y + 1 + i as u16, entry, style);
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
    let mut cache = app.toolbar_cache.borrow_mut();
    for (tool, rect, label) in &cache.get(l.toolbar, app).buttons {
        let style = if !enabled && *tool != Tool::Add {
            Style::new().fg(t.dim)
        } else {
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
        };
        text::put(buf, rect.x + 1, rect.y, label, style);
        text::put(buf, rect.right(), rect.y, "│", Style::new().fg(t.dim));
    }
}
