use crate::app::{Action, App};
use crate::ui::layout::Layout;
use crate::ui::text;
use crate::waveform::{TimeBase, TimeScale};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Clear, Widget as _};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    HierarchyUp,
    HierarchyDown,
    Back,
    Forward,
    Search,
    SearchCase,
    SearchRegex,
    SearchDown,
    SearchUp,
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
            _ => unreachable!(),
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

const NAV_TOOLS: [(&str, Tool); 6] = [
    ("↑", Tool::HierarchyUp),
    ("↓", Tool::HierarchyDown),
    ("←", Tool::Back),
    ("→", Tool::Forward),
    ("Aa", Tool::SearchCase),
    (".*", Tool::SearchRegex),
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

#[derive(Clone, Copy, PartialEq, Eq)]
struct ToolbarKey {
    tb: Rect,
    nav_tb: Rect,
    base: TimeBase,
    ts: Option<TimeScale>,
}

#[derive(Default)]
pub(crate) struct ToolbarCache {
    key: Option<ToolbarKey>,
    buttons: Vec<(Tool, Rect, Rc<str>)>,
    nav_buttons: Vec<(Tool, Rect, Rc<str>)>,
    menu: Vec<(TimeBase, Rc<str>)>,
    button: Rect,
    search: Rect,
}

impl ToolbarCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn get(&mut self, tb: Rect, nav_tb: Rect, app: &App) -> &Self {
        let key = ToolbarKey {
            tb,
            nav_tb,
            base: app.settings.time_base,
            ts: app.wf.as_ref().map(|wf| wf.ts),
        };
        if self.key == Some(key) {
            return self;
        }
        self.key = Some(key);
        self.buttons.clear();
        self.nav_buttons.clear();
        let mut x = tb.x + 1;
        for (_, tool) in TOOLS {
            let label: Rc<str> = Rc::from(tool_label(tool, app));
            let width = (label.chars().count() + 2) as u16;
            self.buttons.push((
                tool,
                Rect {
                    x,
                    y: tb.y,
                    width,
                    height: 1,
                },
                label,
            ));
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

        let y = nav_tb.y;
        let mut x = nav_tb.x + 1;
        for (name, tool) in NAV_TOOLS {
            let label: Rc<str> = Rc::from(name);
            let width = (name.chars().count() + 2) as u16;
            self.nav_buttons.push((
                tool,
                Rect {
                    x,
                    y,
                    width,
                    height: 1,
                },
                label,
            ));
            x += width + 1;
        }
        let search_down_x = nav_tb.right().saturating_sub(9);
        let search_up_x = nav_tb.right().saturating_sub(4);
        self.search = Rect {
            x: x + 1,
            y,
            width: search_down_x.saturating_sub(x + 2),
            height: 1,
        };
        self.nav_buttons.push((
            Tool::SearchDown,
            Rect {
                x: search_down_x,
                y,
                width: 4,
                height: 1,
            },
            Rc::from("▼"),
        ));
        self.nav_buttons.push((
            Tool::SearchUp,
            Rect {
                x: search_up_x,
                y,
                width: 4,
                height: 1,
            },
            Rc::from("▲"),
        ));
        self
    }
}

pub fn tool_at(tb: Rect, nav_tb: Rect, app: &App, col: u16, row: u16) -> Option<Tool> {
    let mut cache = app.toolbar_cache.borrow_mut();
    let cache = cache.get(tb, nav_tb, app);
    cache
        .buttons
        .iter()
        .chain(cache.nav_buttons.iter())
        .find(|(_, rect, _)| crate::ui::layout::pt_in(*rect, col, row))
        .map(|(tool, _, _)| *tool)
        .or_else(|| crate::ui::layout::pt_in(cache.search, col, row).then_some(Tool::Search))
}

pub fn time_button(tb: Rect, app: &App) -> Rect {
    let mut cache = app.toolbar_cache.borrow_mut();
    cache.get(tb, Rect::default(), app).button
}

fn base_label(base: TimeBase, app: &App) -> String {
    match base {
        TimeBase::Scale => match &app.wf {
            Some(wf) => format!("ts ({})", wf.ts.label()),
            None => "ts".to_string(),
        },
        other => other.label().to_string(),
    }
}

pub fn time_menu_rect(tb: Rect, app: &App) -> Rect {
    let button = time_button(tb, app);
    let mut cache = app.toolbar_cache.borrow_mut();
    let width = cache
        .get(tb, Rect::default(), app)
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

pub fn draw_time_menu(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.settings.theme;
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
    for (i, (_, entry)) in cache
        .get(l.toolbar, Rect::default(), app)
        .menu
        .iter()
        .enumerate()
    {
        let style = if i == selected {
            Style::new().fg(Color::Black).bg(t.accent)
        } else {
            Style::new().fg(t.text)
        };
        text::put(buf, area.x + 1, area.y + 1 + i as u16, entry, style);
    }
}

pub fn run_tool(app: &mut App, tool: Tool) -> bool {
    match tool {
        Tool::TimeBase => {
            let current = TimeBase::CYCLE
                .iter()
                .position(|base| *base == app.settings.time_base)
                .unwrap_or(0);
            app.time_menu = Some(current);
            false
        }
        Tool::HierarchyUp => {
            app.hierarchy_up();
            false
        }
        Tool::HierarchyDown => {
            app.hierarchy_down();
            false
        }
        Tool::Back => {
            app.history_back();
            false
        }
        Tool::Forward => {
            app.history_forward();
            false
        }
        Tool::Search => {
            app.focus_source_search();
            false
        }
        Tool::SearchCase => {
            app.toggle_search_case_sensitive();
            false
        }
        Tool::SearchRegex => {
            app.toggle_search_regex();
            false
        }
        Tool::SearchDown => {
            app.search_source(true);
            false
        }
        Tool::SearchUp => {
            app.search_source(false);
            false
        }
        _ => {
            app.toolbar_search = false;
            tool.action().run(app)
        }
    }
}

pub fn draw(buf: &mut Buffer, l: &Layout, app: &App) -> Option<Position> {
    let t = &app.settings.theme;
    buf.set_style(l.toolbar, Style::new().bg(t.toolbar_bg));
    buf.set_style(l.nav_toolbar, Style::new().bg(t.toolbar_bg));
    let enabled = app.wf.is_some();
    let mut cache = app.toolbar_cache.borrow_mut();
    let cache = cache.get(l.toolbar, l.nav_toolbar, app);
    for (tool, rect, label) in &cache.buttons {
        let style = if !enabled && *tool != Tool::Add {
            Style::new().fg(t.dim)
        } else {
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
        };
        text::put(buf, rect.x + 1, rect.y, label, style);
        text::put(buf, rect.right(), rect.y, "│", Style::new().fg(t.dim));
    }
    for (tool, rect, label) in &cache.nav_buttons {
        let active = match tool {
            Tool::SearchCase => app.search_case_sensitive,
            Tool::SearchRegex => app.search_regex,
            _ => false,
        };
        let style = if active {
            Style::new().fg(Color::Black).bg(t.accent)
        } else {
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
        };
        text::put(buf, rect.x + 1, rect.y, label, style);
        text::put(buf, rect.right(), rect.y, "│", Style::new().fg(t.dim));
    }
    if app.toolbar_search && cache.search.width > 0 {
        return Some(crate::ui::widgets::TextBox::new(&app.input, cache.search).draw(buf, t));
    }
    let value = app.input.as_string();
    let (value, fg) = if value.is_empty() {
        ("Search source".to_string(), t.dim)
    } else {
        (value, t.text)
    };
    crate::ui::widgets::Label::new(&value, cache.search.width as usize).draw(
        buf,
        cache.search.x,
        cache.search.y,
        Style::new().fg(fg).bg(t.input_bg),
    );
    None
}
