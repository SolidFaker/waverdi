use crate::app::{keymap, App, Dialog, EntryKind};
use crate::settings::theme::Theme;
use crate::settings::KeymapSettings;
use crate::ui::layout::Layout;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Widget as _};
use ratatui::Frame;
use std::sync::{Arc, LazyLock, Mutex};

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
    let wanted_width = if dialog == Dialog::Settings { 92 } else { 64 };
    let wanted_height = match dialog {
        Dialog::Find => 14,
        Dialog::Keys => keymap::help_len() as u16 + 4,
        Dialog::CreateBus => app
            .bus_builder()
            .map(|builder| builder.items.len() as u16 + 4)
            .unwrap_or(8),
        Dialog::Settings => 22,
        _ => 7,
    };
    let width = wanted_width
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
        Dialog::Settings => 4,
        _ => 2,
    };
    (area.height as usize).saturating_sub(reserved).max(1)
}
/// Right-edge scrollbar for a scrolled list.
fn draw_scrollbar(
    buf: &mut Buffer,
    x: u16,
    top: u16,
    rows: usize,
    scroll: usize,
    total: usize,
    t: &Theme,
) {
    let Some((start, len)) = crate::ui::scrollbar::track_geometry(rows, total, rows, scroll, true)
    else {
        return;
    };
    crate::ui::scrollbar::Bar {
        orientation: crate::ui::scrollbar::Orientation::Vertical,
        x,
        y: top,
        span: rows,
        start,
        len,
        thumb: "█",
        track: "│",
        thumb_fg: t.dim,
        track_fg: t.dim,
        bg: t.bg,
    }
    .draw(buf);
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

struct KeyHelpCache {
    revision: u64,
    widths: Vec<Option<Arc<[String]>>>,
}

static KEYS_WRAPPED: LazyLock<Mutex<KeyHelpCache>> = LazyLock::new(|| {
    Mutex::new(KeyHelpCache {
        revision: u64::MAX,
        widths: vec![None; 65],
    })
});

fn keys_lines(inner_w: usize, settings: &KeymapSettings) -> Arc<[String]> {
    let mut cache = KEYS_WRAPPED.lock().unwrap_or_else(|err| err.into_inner());
    if cache.revision != settings.revision {
        let lines = keymap::help_lines(settings);
        cache.widths = (0..=64)
            .map(|width| {
                let wrapped: Vec<String> = lines
                    .iter()
                    .flat_map(|line| text::wrap(line, width))
                    .collect();
                Some(Arc::from(wrapped))
            })
            .collect();
        cache.revision = settings.revision;
    }
    let index = inner_w.min(cache.widths.len().saturating_sub(1));
    cache.widths[index]
        .clone()
        .unwrap_or_else(|| Arc::from(Vec::new()))
}

fn keys_line_count(inner_w: usize, settings: &KeymapSettings) -> usize {
    keys_lines(inner_w, settings).len()
}

/// Scrollable body of a dialog: `None` when everything fits.
pub fn scroll_area(screen: Rect, app: &App, dialog: Dialog) -> Option<ScrollArea> {
    let area = dialog_rect(screen, app, dialog);
    let (top, rows, total) = match dialog {
        Dialog::Keys => (
            area.y + 1,
            area.height.saturating_sub(2) as usize,
            keys_line_count(area.width.saturating_sub(4) as usize, &app.settings.keys),
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
        Dialog::Settings => (
            area.y + 1,
            area.height.saturating_sub(4) as usize,
            app.settings_len(),
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
    let t = &app.settings.theme;
    let title = match dialog {
        Dialog::Open => "Open Waveform",
        Dialog::Goto => "Go to Time",
        Dialog::Find => "Search Signal",
        Dialog::FindValue => "Find Value",
        Dialog::SplitBus => "Split Bus",
        Dialog::CreateBus => "Create Bus",
        Dialog::GroupName => "Rename Group",
        Dialog::Settings => "Settings",
        Dialog::BindKey => "Edit Shortcut",
        Dialog::TopSplit => "Top Pane Ratio",
        Dialog::Keys => "Key Bindings",
        Dialog::About => "About",
        Dialog::AddSignals => "Add Signals",
    };
    let area = dialog_rect(l.area, app, dialog);

    let mut cursor = None;
    {
        let buf = frame.buffer_mut();
        dim_outside(buf, l.area, area);
        Clear.render(area, buf);
        buf.set_style(area, Style::new().bg(t.popup_bg));
        let mut block = Block::bordered()
            .title(format!(" {title} "))
            .title_style(Style::new().fg(t.accent).add_modifier(Modifier::BOLD))
            .border_style(Style::new().fg(t.accent));
        if dialog == Dialog::Open {
            block = block.title_bottom(
                Line::from(" ↑↓ move   Enter open   Backspace parent   Esc cancel ")
                    .style(Style::new().fg(t.dim)),
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
            Style::new().fg(t.xcol).add_modifier(Modifier::BOLD),
        );

        let inner_x = area.x + 2;
        let inner_w = area.width.saturating_sub(4) as usize;
        match dialog {
            Dialog::AddSignals => {}
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
            Dialog::Settings => draw_settings(buf, area, app),
            Dialog::BindKey => draw_bind_key(buf, area, app),
            Dialog::TopSplit => draw_top_split(buf, area, app),
            Dialog::Keys => {
                let inner_w = area.width.saturating_sub(4) as usize;
                let lines = keys_lines(inner_w, &app.settings.keys);
                let rows = area.height.saturating_sub(2) as usize;
                let max_scroll = lines.len().saturating_sub(rows);
                let scroll = app.dialog_scroll.min(max_scroll);
                for (i, line) in lines.iter().skip(scroll).take(rows).enumerate() {
                    text::put(
                        buf,
                        inner_x,
                        area.y + 1 + i as u16,
                        line,
                        Style::new().fg(t.text),
                    );
                }
                draw_scrollbar(
                    buf,
                    area.right().saturating_sub(2),
                    area.y + 1,
                    rows,
                    scroll,
                    lines.len(),
                    t,
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
                            Style::new().fg(t.text),
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

struct SettingsLayout {
    categories: Rect,
    items: Rect,
    rows: usize,
}

fn settings_layout(area: Rect) -> SettingsLayout {
    use crate::ui::widgets::{Column, Constraint, Row};

    let outer = Column::new(&[Constraint::Fill, Constraint::Fixed(2)])
        .gap(1)
        .split(Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        ));
    let body = outer[0];
    let columns = Row::new(&[Constraint::Ratio(1, 4), Constraint::Fill])
        .gap(1)
        .split(body);
    SettingsLayout {
        categories: columns[0],
        items: columns[1],
        rows: body.height as usize,
    }
}

fn settings_buttons(
    area: Rect,
) -> (
    crate::ui::widgets::Button<'static>,
    crate::ui::widgets::Button<'static>,
) {
    (
        crate::ui::widgets::Button::new(
            "Reset",
            Rect::new(
                area.right().saturating_sub(22),
                area.bottom().saturating_sub(2),
                10,
                1,
            ),
        ),
        crate::ui::widgets::Button::new(
            "Close",
            Rect::new(
                area.right().saturating_sub(11),
                area.bottom().saturating_sub(2),
                10,
                1,
            ),
        ),
    )
}

fn draw_settings(buf: &mut Buffer, area: Rect, app: &App) {
    use crate::app::{SettingRow, SettingsPane};
    use crate::settings::{GeneralSetting, SettingId, SettingsCategory, ThemeKind};
    use crate::ui::widgets::{Alignment, Checkbox, Dropdown, Label, RadioGroup, Toggle};

    let t = &app.settings.theme;
    let layout = settings_layout(area);
    for (index, category) in SettingsCategory::ALL.into_iter().enumerate() {
        let selected = category == app.settings_category;
        let focused = app.settings_pane == SettingsPane::Categories;
        let style = if selected && focused {
            Style::new().fg(Color::Black).bg(t.accent)
        } else if selected {
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(t.text)
        };
        Label::new(category.label(), layout.categories.width as usize).draw(
            buf,
            layout.categories.x,
            layout.categories.y + index as u16,
            style,
        );
    }
    for y in layout.categories.y..layout.categories.bottom() {
        text::set_cell(
            buf,
            layout.items.x.saturating_sub(1),
            y,
            "│",
            t.dim,
            t.popup_bg,
        );
    }

    let total = app.settings_len();
    let scroll = app.dialog_scroll.min(total.saturating_sub(layout.rows));
    for (offset, row) in (scroll..total).take(layout.rows).enumerate() {
        let Some(setting) = app.settings_row_at(row) else {
            continue;
        };
        let y = layout.items.y + offset as u16;
        let row_area = Rect::new(layout.items.x, y, layout.items.width, 1);
        let selected = row == app.settings_sel && app.settings_pane == SettingsPane::Items;
        match setting {
            SettingRow::Setting(SettingId::Theme) => {
                let options: Vec<&str> = ThemeKind::ALL.iter().map(|kind| kind.name()).collect();
                let selected = ThemeKind::ALL
                    .iter()
                    .position(|kind| *kind == app.settings.theme_kind)
                    .unwrap_or(0);
                RadioGroup::new(&options, selected, row_area).draw(buf, t);
            }
            SettingRow::Setting(SettingId::General(GeneralSetting::BusSplitBits))
            | SettingRow::Setting(SettingId::General(GeneralSetting::BusSplitWork)) => {
                let general = match setting {
                    SettingRow::Setting(SettingId::General(setting)) => setting,
                    _ => unreachable!(),
                };
                Label::new(general.label(), 24).draw(buf, row_area.x, y, Style::new().fg(t.text));
                if let Some(slider) = app.general_slider(
                    general,
                    Rect::new(
                        row_area.x + 26,
                        y,
                        row_area.width.saturating_sub(40).max(6),
                        1,
                    ),
                ) {
                    slider.draw(buf, t);
                }
                Label::new(&general.value(&app.settings.general), 8)
                    .align(Alignment::Right)
                    .draw(
                        buf,
                        row_area.right().saturating_sub(8),
                        y,
                        Style::new().fg(t.accent),
                    );
            }
            SettingRow::Setting(SettingId::General(GeneralSetting::WaveOverflow)) => {
                Toggle::new(
                    app.settings.general.wave_overflow
                        == crate::settings::WaveValueOverflow::Ellipsize,
                    "wave label ellipsis",
                    row_area,
                )
                .draw(buf, t);
            }
            SettingRow::Setting(SettingId::General(GeneralSetting::FullNames)) => {
                Checkbox::new(
                    app.settings.general.full_names,
                    "full hierarchical names",
                    row_area,
                )
                .draw(buf, t);
            }
            SettingRow::Setting(SettingId::General(GeneralSetting::TimeBase)) => {
                let options: Vec<&str> = crate::waveform::TimeBase::CYCLE
                    .iter()
                    .map(|base| base.label())
                    .collect();
                Dropdown::new(&options, app.time_dropdown, row_area).draw(buf, t);
            }
            SettingRow::Setting(SettingId::General(GeneralSetting::TopSplit)) => {
                let view = app.settings_row_view(setting);
                Label::new(&format!("{} (Enter to adjust)", view.label), 28).draw(
                    buf,
                    row_area.x,
                    y,
                    Style::new().fg(t.text),
                );
                Label::new(&view.value, 5).align(Alignment::Right).draw(
                    buf,
                    row_area.right().saturating_sub(5),
                    y,
                    Style::new().fg(t.accent),
                );
            }
            _ => {
                let view = app.settings_row_view(setting);
                let selected_style = if selected {
                    Style::new().fg(Color::Black).bg(t.accent)
                } else {
                    Style::new().fg(t.text)
                };
                let marker = if view.custom { "*" } else { " " };
                Label::new(&format!("{marker} {}", view.label), 28).draw(
                    buf,
                    row_area.x,
                    y,
                    selected_style,
                );
                Label::new(&view.value, row_area.width.saturating_sub(30) as usize)
                    .align(Alignment::Right)
                    .draw(buf, row_area.x + 30, y, selected_style);
                if let Some(color) = view.color {
                    text::put(
                        buf,
                        row_area.right().saturating_sub(3),
                        y,
                        "██",
                        Style::new().fg(color),
                    );
                }
            }
        }
    }
    draw_scrollbar(
        buf,
        area.right().saturating_sub(2),
        layout.items.y,
        layout.rows,
        scroll,
        total,
        t,
    );
    let help = if app.settings_category == SettingsCategory::KeyBindings {
        "Tab pane  ↑↓ select  Enter bind  r default  R all defaults"
    } else {
        "Tab pane  ↑↓ select  ←/→ change  Enter edit  r reset"
    };
    text::put(
        buf,
        area.x + 2,
        area.bottom().saturating_sub(2),
        &text::trunc(help, area.width.saturating_sub(28) as usize),
        Style::new().fg(t.dim),
    );
    let (reset, close) = settings_buttons(area);
    reset
        .enabled(app.settings_row().is_some())
        .selected(app.settings_pane == crate::app::SettingsPane::Items)
        .draw(buf, t);
    close.draw(buf, t);
}

fn draw_bind_key(buf: &mut Buffer, area: Rect, app: &App) {
    let t = &app.settings.theme;
    let Some(index) = app.key_capture else { return };
    let binding = &crate::app::keymap::bindings()[index];
    let x = area.x + 2;
    text::put(
        buf,
        x,
        area.y + 1,
        &text::trunc(binding.description, area.width.saturating_sub(4) as usize),
        Style::new().fg(t.text).add_modifier(Modifier::BOLD),
    );
    text::put(
        buf,
        x,
        area.y + 2,
        &format!(
            "current: {}",
            crate::app::keymap::effective_keys(index, &app.settings.keys)
        ),
        Style::new().fg(t.dim),
    );
    text::put(
        buf,
        x,
        area.y + 4,
        "press a new shortcut",
        Style::new().fg(t.accent),
    );
    text::put(
        buf,
        x,
        area.bottom().saturating_sub(2),
        "Esc cancel   Backspace/Delete default",
        Style::new().fg(t.dim),
    );
}

fn draw_top_split(buf: &mut Buffer, area: Rect, app: &App) {
    let t = &app.settings.theme;
    crate::ui::widgets::Label::new(
        &format!("top pane ratio: {}%", app.settings.splits.top_pct),
        area.width.saturating_sub(4) as usize,
    )
    .align(crate::ui::widgets::Alignment::Center)
    .draw(buf, area.x + 2, area.y + 1, Style::new().fg(t.text));
    top_split_slider(app).draw(buf, t);
}

pub fn top_split_slider(app: &App) -> crate::ui::widgets::Slider {
    let area = dialog_rect(app.last_area, app, Dialog::TopSplit);
    crate::ui::widgets::Slider::new(
        app.settings.splits.top_pct as usize,
        crate::settings::Splits::MIN_TOP_PCT as usize,
        crate::settings::Splits::MAX_TOP_PCT as usize,
        crate::ui::widgets::Orientation::Vertical,
        Rect::new(
            area.x + area.width / 2,
            area.y + 2,
            1,
            area.height.saturating_sub(3).max(1),
        ),
    )
    .step(5)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsHit {
    Category(usize),
    Item(usize),
    Theme(usize),
    General {
        row: usize,
        setting: crate::settings::GeneralSetting,
        value: usize,
    },
    Dropdown(usize),
    DismissDropdown,
    Reset,
    Close,
}

pub fn settings_hit(screen: Rect, app: &App, col: u16, row: u16) -> Option<SettingsHit> {
    use crate::settings::{GeneralSetting, SettingId, ThemeKind};
    use crate::ui::widgets::{Checkbox, Dropdown, RadioGroup, Toggle};

    let area = dialog_rect(screen, app, Dialog::Settings);
    let (reset, close) = settings_buttons(area);
    if reset.contains(col, row) {
        return Some(SettingsHit::Reset);
    }
    if close.contains(col, row) {
        return Some(SettingsHit::Close);
    }
    let layout = settings_layout(area);
    let total = app.settings_len();
    let scroll = app.dialog_scroll.min(total.saturating_sub(layout.rows));
    let selected_row = scroll + (row.saturating_sub(layout.items.y) as usize);
    if app.time_dropdown.open {
        let options: Vec<&str> = crate::waveform::TimeBase::CYCLE
            .iter()
            .map(|base| base.label())
            .collect();
        let y = layout.items.y + app.settings_sel.saturating_sub(scroll) as u16;
        let item = Rect::new(layout.items.x, y, layout.items.width, 1);
        let dropdown = Dropdown::new(&options, app.time_dropdown, item);
        if let Some(index) = dropdown.option_at(col, row) {
            return Some(SettingsHit::Dropdown(index));
        }
        return Some(SettingsHit::DismissDropdown);
    }
    if crate::ui::layout::pt_in(layout.categories, col, row) {
        let index = (row - layout.categories.y) as usize;
        return (index < crate::settings::SettingsCategory::ALL.len())
            .then_some(SettingsHit::Category(index));
    }
    if !crate::ui::layout::pt_in(layout.items, col, row) || selected_row >= total {
        return None;
    }
    match app.settings_row_at(selected_row) {
        Some(crate::app::SettingRow::Setting(SettingId::Theme)) => {
            let options: Vec<&str> = ThemeKind::ALL.iter().map(|kind| kind.name()).collect();
            let selected = ThemeKind::ALL
                .iter()
                .position(|kind| *kind == app.settings.theme_kind)
                .unwrap_or(0);
            RadioGroup::new(&options, selected, layout.items)
                .option_at(col, row)
                .map(SettingsHit::Theme)
        }
        Some(crate::app::SettingRow::Setting(SettingId::General(setting))) => {
            let y = layout.items.y + selected_row.saturating_sub(scroll) as u16;
            let item = Rect::new(layout.items.x, y, layout.items.width, 1);
            match setting {
                GeneralSetting::BusSplitBits | GeneralSetting::BusSplitWork => app
                    .general_slider(
                        setting,
                        Rect::new(item.x + 26, y, item.width.saturating_sub(40).max(6), 1),
                    )
                    .and_then(|slider| slider.value_at(col, row))
                    .map(|value| SettingsHit::General {
                        row: selected_row,
                        setting,
                        value,
                    }),
                GeneralSetting::WaveOverflow => Toggle::new(
                    app.settings.general.wave_overflow
                        == crate::settings::WaveValueOverflow::Ellipsize,
                    "wave label ellipsis",
                    item,
                )
                .changed_at(col, row)
                .map(|value| SettingsHit::General {
                    row: selected_row,
                    setting,
                    value: value as usize,
                }),
                GeneralSetting::FullNames => Checkbox::new(
                    app.settings.general.full_names,
                    "full hierarchical names",
                    item,
                )
                .changed_at(col, row)
                .map(|value| SettingsHit::General {
                    row: selected_row,
                    setting,
                    value: value as usize,
                }),
                GeneralSetting::TimeBase => {
                    let options: Vec<&str> = crate::waveform::TimeBase::CYCLE
                        .iter()
                        .map(|base| base.label())
                        .collect();
                    Dropdown::new(&options, app.time_dropdown, item)
                        .contains(col, row)
                        .then_some(SettingsHit::General {
                            row: selected_row,
                            setting,
                            value: usize::MAX,
                        })
                }
                GeneralSetting::TopSplit => Some(SettingsHit::General {
                    row: selected_row,
                    setting,
                    value: usize::MAX,
                }),
            }
        }
        _ => Some(SettingsHit::Item(selected_row)),
    }
}

/// Dim everything outside the dialog in place, preserving each pane's own
/// colours: the waveform background stays dark, the Signal List stays light.
/// This keeps the panes independent of any single "overlay" theme colour.
fn dim_outside(buf: &mut Buffer, screen: Rect, dialog: Rect) {
    for y in screen.y..screen.bottom() {
        for x in screen.x..screen.right() {
            if x >= dialog.x && x < dialog.right() && y >= dialog.y && y < dialog.bottom() {
                continue;
            }
            let Some(cell) = buf.cell_mut((x, y)) else {
                continue;
            };
            if let Some(bg) = dim_color(cell.bg) {
                cell.set_bg(bg);
            }
            if let Some(fg) = dim_color(cell.fg) {
                cell.set_fg(fg);
            }
        }
    }
}

/// Scale a colour down to 55% (RGB and plain white), leaving the terminal
/// default and named colours untouched. `11/20` is the exact integer form of
/// the old `* 0.55` truncation, so the colours (and the test below) are
/// unchanged while the per-cell float math disappears.
fn dim_color(color: Color) -> Option<Color> {
    let scale = |value: u8| (value as u32 * 11 / 20) as u8;
    match color {
        Color::Rgb(r, g, b) => Some(Color::Rgb(scale(r), scale(g), scale(b))),
        Color::White => Some(Color::Rgb(150, 150, 150)),
        _ => None,
    }
}

fn draw_browser(buf: &mut Buffer, area: Rect, app: &App) {
    let t = &app.settings.theme;
    let Some(browser) = &app.browser else {
        text::put(
            buf,
            area.x + 2,
            area.y + 2,
            "no browser state",
            Style::new().fg(t.xcol),
        );
        return;
    };

    text::put(
        buf,
        area.x + 2,
        area.y + 1,
        &browser.dir.display().to_string(),
        Style::new().fg(t.path),
    );
    let sep_w = area.width.saturating_sub(2) as usize;
    buf.set_string(
        area.x + 1,
        area.y + 2,
        "─".repeat(sep_w),
        Style::new().fg(t.panel_border),
    );

    if let Some(error) = &browser.error {
        text::put(buf, area.x + 2, area.y + 3, error, Style::new().fg(t.xcol));
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
            Style::new().fg(Color::Black).bg(t.accent)
        } else {
            match entry.kind {
                EntryKind::Parent => Style::new().fg(t.dim),
                EntryKind::Dir => Style::new().fg(t.scope).add_modifier(Modifier::BOLD),
                EntryKind::File if crate::app::is_waveform(&entry.name) => Style::new().fg(t.high),
                EntryKind::File => Style::new().fg(t.name),
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
            text::set_cell(buf, x, area.y + 3 + row as u16, symbol, t.dim, t.bg);
        }
    }
}

fn draw_bus_builder(buf: &mut Buffer, area: Rect, app: &App) {
    let t = &app.settings.theme;
    let Some(builder) = app.bus_builder() else {
        text::put(
            buf,
            area.x + 2,
            area.y + 2,
            "no bus builder state",
            Style::new().fg(t.xcol),
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
            Style::new().fg(Color::Black).bg(t.accent)
        } else {
            Style::new().fg(t.text)
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
        t,
    );
    let hint_top = " ↑↓ select   ⇧↑↓ reorder   h/l LSB   H/L MSB   x full range";
    let hint_bottom = format!(" s/S sort   r rev   Enter create   Esc cancel   {total} bits ");
    text::put(
        buf,
        area.x + 2,
        area.bottom().saturating_sub(3),
        &text::trunc(hint_top, inner),
        Style::new().fg(t.dim),
    );
    text::put(
        buf,
        area.x + 2,
        area.bottom().saturating_sub(2),
        &text::trunc(&hint_bottom, inner),
        Style::new().fg(t.dim),
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
    use crate::ui::widgets::{Constraint, Row, TextBox};

    let t = &app.settings.theme;
    let (label, hint) = labels;
    let cells = Row::new(&[
        Constraint::Fixed(label.chars().count() as u16),
        Constraint::Fill,
    ])
    .split(Rect::new(x, area.y + 2, width as u16, 1));
    text::put(buf, cells[0].x, cells[0].y, label, Style::new().fg(t.text));
    let caret = TextBox::new(&app.input, cells[1]).draw(buf, t);
    text::put(
        buf,
        x,
        area.y + 4,
        &text::trunc(hint, width),
        Style::new().fg(t.dim),
    );
    Some(caret)
}

fn draw_find(buf: &mut Buffer, area: Rect, x: u16, width: usize, app: &App) -> Option<Position> {
    use crate::ui::widgets::{Constraint, Row, TextBox};

    let t = &app.settings.theme;
    let cells = Row::new(&[Constraint::Fixed(7), Constraint::Fill]).split(Rect::new(
        x,
        area.y + 1,
        width as u16,
        1,
    ));
    text::put(
        buf,
        cells[0].x,
        cells[0].y,
        "Query: ",
        Style::new().fg(t.text),
    );
    let caret = TextBox::new(&app.input, cells[1]).draw(buf, t);
    text::put(
        buf,
        x,
        area.y + 2,
        "Enter: add signal    ↑/↓: navigate    Esc: cancel",
        Style::new().fg(t.dim),
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
                Style::new().fg(Color::Black).bg(t.accent)
            } else {
                Style::new().fg(t.text)
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
        t,
    );
    Some(caret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dim_mix_is_identical_to_the_old_float_blend() {
        // The integer mix replaced per-cell float math; every reachable
        // channel value must truncate exactly like the old expression.
        for value in 0..=u8::MAX {
            let old = (value as f32 * 0.55) as u8;
            let new = (value as u32 * 11 / 20) as u8;
            assert_eq!(old, new, "value={value}");
            assert_eq!(
                dim_color(Color::Rgb(value, value, value)),
                Some(Color::Rgb(old, old, old))
            );
        }
        assert_eq!(dim_color(Color::White), Some(Color::Rgb(150, 150, 150)));
        assert_eq!(dim_color(Color::Reset), None);
    }

    #[test]
    fn keys_help_is_wrapped_once_per_width_and_revision() {
        let mut settings = KeymapSettings::default();
        let first = keys_lines(40, &settings);
        let again = keys_lines(40, &settings);
        assert!(Arc::ptr_eq(&first, &again), "wrap cache missed");
        let expected: usize = keymap::help_lines(&settings)
            .iter()
            .map(|line| text::wrap(line, 40).len())
            .sum();
        assert_eq!(first.len(), expected);
        assert_eq!(keys_line_count(40, &settings), expected);

        settings.set(
            0,
            crate::settings::KeyChord {
                code: crossterm::event::KeyCode::F(5),
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
        );
        let changed = keys_lines(40, &settings);
        assert!(!Arc::ptr_eq(&first, &changed));
        assert!(changed.iter().any(|line| line.contains("F5")));
    }
}
