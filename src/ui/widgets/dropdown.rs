use crate::settings::theme::Theme;
use crate::ui::text;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

pub const MAX_VISIBLE_OPTIONS: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropdownState {
    pub selected: usize,
    pub open: bool,
    pub scroll: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Popup {
    area: Rect,
    first: usize,
    visible: usize,
    scrollable: bool,
}

pub struct Dropdown<'a> {
    options: &'a [&'a str],
    state: DropdownState,
    area: Rect,
}

impl<'a> Dropdown<'a> {
    pub fn new(options: &'a [&'a str], state: DropdownState, area: Rect) -> Self {
        Self {
            options,
            state,
            area,
        }
    }

    pub fn draw(self, buf: &mut Buffer, theme: &Theme) {
        if self.area.width == 0 || self.area.height == 0 {
            return;
        }
        let current = self.options.get(self.state.selected).copied().unwrap_or("");
        let marker = if self.state.open { "▴" } else { "▾" };
        let style = Style::new().fg(theme.text).bg(theme.input_bg);
        buf.set_style(self.area, style);
        text::put(
            buf,
            self.area.x,
            self.area.y,
            &text::trunc(current, self.area.width.saturating_sub(2) as usize),
            style,
        );
        text::put(buf, self.area.right() - 1, self.area.y, marker, style);
    }

    pub fn draw_options(&self, buf: &mut Buffer, theme: &Theme, bounds: Rect) {
        let Some(popup) = self.popup(bounds) else {
            return;
        };
        ratatui::widgets::Widget::render(ratatui::widgets::Clear, popup.area, buf);
        for offset in 0..popup.visible {
            let index = popup.first + offset;
            let rect = option_rect(popup, index);
            let style = if index == self.state.selected {
                Style::new().fg(Color::Black).bg(theme.accent)
            } else {
                Style::new().fg(theme.text).bg(theme.popup_bg)
            };
            buf.set_style(rect, style);
            let reserved = 2 + usize::from(popup.scrollable) as u16;
            text::put(
                buf,
                rect.x + 1,
                rect.y,
                &text::trunc(
                    self.options[index],
                    rect.width.saturating_sub(reserved) as usize,
                ),
                style,
            );
        }
        if popup.scrollable {
            draw_scrollbar(buf, popup, self.options.len(), theme);
        }
    }

    pub fn handle_key(&self, state: &mut DropdownState, key: KeyEvent, bounds: Rect) -> bool {
        if self.options.is_empty() {
            return false;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => {
                state.open = !state.open;
                if state.open {
                    state.selected = state.selected.min(self.options.len() - 1);
                    state.scroll = state.selected;
                }
            }
            KeyCode::Esc if state.open => state.open = false,
            KeyCode::Up if state.open => {
                state.selected = state
                    .selected
                    .checked_sub(1)
                    .unwrap_or(self.options.len() - 1);
            }
            KeyCode::Down if state.open => {
                state.selected = (state.selected + 1) % self.options.len();
            }
            KeyCode::Home if state.open => state.selected = 0,
            KeyCode::End if state.open => state.selected = self.options.len() - 1,
            _ => return false,
        }
        if state.open {
            if let Some(popup) = self.popup_for(*state, bounds) {
                if state.selected < state.scroll {
                    state.scroll = state.selected;
                } else if state.selected >= state.scroll + popup.visible {
                    state.scroll = state.selected + 1 - popup.visible;
                }
                state.scroll = state
                    .scroll
                    .min(self.options.len().saturating_sub(popup.visible));
            }
        }
        true
    }

    pub fn scroll(&self, state: &mut DropdownState, delta: i64, bounds: Rect) {
        let Some(popup) = self.popup_for(*state, bounds) else {
            return;
        };
        let max_scroll = self.options.len().saturating_sub(popup.visible);
        state.scroll = if delta < 0 {
            state.scroll.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            state.scroll.saturating_add(delta as usize)
        }
        .min(max_scroll);
    }

    pub fn contains(&self, col: u16, row: u16) -> bool {
        crate::ui::layout::pt_in(self.area, col, row)
    }

    pub fn option_at(&self, col: u16, row: u16, bounds: Rect) -> Option<usize> {
        let popup = self.popup(bounds)?;
        if !crate::ui::layout::pt_in(popup.area, col, row)
            || (popup.scrollable && col == popup.area.right().saturating_sub(1))
        {
            return None;
        }
        Some(popup.first + (row - popup.area.y) as usize)
    }

    pub fn scrollbar_at(&self, col: u16, row: u16, bounds: Rect) -> bool {
        self.popup(bounds).is_some_and(|popup| {
            popup.scrollable
                && col == popup.area.right().saturating_sub(1)
                && row >= popup.area.y
                && row < popup.area.bottom()
        })
    }

    pub fn scroll_to(&self, state: &mut DropdownState, row: u16, bounds: Rect) {
        let Some(popup) = self.popup_for(*state, bounds) else {
            return;
        };
        state.scroll = crate::ui::scrollbar::scroll_from_track(
            popup.visible,
            self.options.len(),
            popup.visible,
            row.saturating_sub(popup.area.y) as usize,
        );
    }

    fn popup(&self, bounds: Rect) -> Option<Popup> {
        self.popup_for(self.state, bounds)
    }

    fn popup_for(&self, state: DropdownState, bounds: Rect) -> Option<Popup> {
        if !state.open || self.options.is_empty() || bounds.width == 0 || bounds.height == 0 {
            return None;
        }
        let wanted = self.options.len().min(MAX_VISIBLE_OPTIONS) as u16;
        let below = bounds.bottom().saturating_sub(self.area.bottom());
        let above = self.area.y.saturating_sub(bounds.y);
        let down = below >= wanted || (above < wanted && below >= above);
        let available = if down { below } else { above };
        let visible = wanted.min(available);
        if visible == 0 {
            return None;
        }
        let y = if down {
            self.area.bottom()
        } else {
            self.area.y.saturating_sub(visible)
        };
        let y = y.clamp(bounds.y, bounds.bottom() - visible);
        let content = self
            .options
            .iter()
            .map(|option| option.chars().count())
            .max()
            .unwrap_or(0);
        let width = (content.saturating_add(4))
            .max(8)
            .min(bounds.width as usize) as u16;
        let x = self
            .area
            .x
            .clamp(bounds.x, bounds.right().saturating_sub(width));
        let first = state
            .scroll
            .min(self.options.len().saturating_sub(visible as usize));
        Some(Popup {
            area: Rect::new(x, y, width, visible),
            first,
            visible: visible as usize,
            scrollable: self.options.len() > visible as usize,
        })
    }
}

fn option_rect(popup: Popup, index: usize) -> Rect {
    Rect::new(
        popup.area.x,
        popup.area.y + (index - popup.first) as u16,
        popup.area.width,
        1,
    )
}

fn draw_scrollbar(buf: &mut Buffer, popup: Popup, total: usize, theme: &Theme) {
    let Some((start, len)) = crate::ui::scrollbar::track_geometry(
        popup.visible,
        total,
        popup.visible,
        popup.first,
        true,
    ) else {
        return;
    };
    crate::ui::scrollbar::Bar {
        orientation: crate::ui::scrollbar::Orientation::Vertical,
        x: popup.area.right() - 1,
        y: popup.area.y,
        span: popup.visible,
        start,
        len,
        thumb: "█",
        track: "│",
        thumb_fg: theme.accent,
        track_fg: theme.dim,
        bg: theme.popup_bg,
    }
    .draw(buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn opened<'a>(options: &'a [&'a str], area: Rect) -> (Dropdown<'a>, DropdownState) {
        let state = DropdownState {
            open: true,
            ..DropdownState::default()
        };
        (Dropdown::new(options, state, area), state)
    }

    #[test]
    fn dropdown_opens_selects_and_closes() {
        let options = ["ns", "us", "ms"];
        let bounds = Rect::new(0, 0, 12, 8);
        let area = Rect::new(0, 0, 8, 1);
        let mut state = DropdownState::default();
        let dropdown = Dropdown::new(&options, state, area);
        assert!(dropdown.handle_key(&mut state, key(KeyCode::Enter), bounds));
        assert!(state.open);
        assert!(dropdown.handle_key(&mut state, key(KeyCode::Down), bounds));
        assert_eq!(state.selected, 1);
        assert_eq!(
            Dropdown::new(&options, state, area).option_at(0, 2, bounds),
            Some(1)
        );
        assert!(dropdown.handle_key(&mut state, key(KeyCode::Esc), bounds));
        assert!(!state.open);
    }

    #[test]
    fn dropdown_prefers_down_and_only_flips_when_below_is_insufficient() {
        let options = ["0", "1", "2", "3"];
        let bounds = Rect::new(0, 0, 30, 14);
        let (dropdown, _) = opened(&options, Rect::new(4, 4, 12, 1));
        let popup = dropdown.popup(bounds).unwrap();
        assert_eq!(popup.area.y, 5);
        assert_eq!(popup.area.height, 4);

        let (dropdown, _) = opened(&options, Rect::new(4, 11, 12, 1));
        let popup = dropdown.popup(bounds).unwrap();
        assert_eq!(popup.area.y, 7);
        assert_eq!(popup.area.height, 4);
    }

    #[test]
    fn limited_space_uses_the_larger_side_and_scrolls() {
        let options = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
        let bounds = Rect::new(0, 0, 20, 12);
        let (dropdown, _) = opened(&options, Rect::new(2, 6, 10, 1));
        let popup = dropdown.popup(bounds).unwrap();
        assert_eq!(popup.area.y, 0);
        assert_eq!(popup.area.height, 6);
        assert!(popup.scrollable);

        let (dropdown, _) = opened(&options, Rect::new(2, 4, 10, 1));
        let popup = dropdown.popup(bounds).unwrap();
        assert_eq!(popup.area.y, 5);
        assert_eq!(popup.area.height, 7);
        assert!(popup.scrollable);
    }

    #[test]
    fn popup_width_is_adaptive_and_hit_testing_uses_the_same_rectangle() {
        let options = ["ns", "long-unit"];
        let bounds = Rect::new(0, 0, 50, 16);
        let (dropdown, _) = opened(&options, Rect::new(30, 2, 40, 1));
        let popup = dropdown.popup(bounds).unwrap();
        assert_eq!(popup.area.width, 13);
        assert_eq!(popup.area.x, 30);
        assert_eq!(
            dropdown.option_at(popup.area.right() - 1, 3, bounds),
            Some(0)
        );
        assert_eq!(dropdown.option_at(popup.area.right(), 3, bounds), None);

        let narrow = Rect::new(0, 0, 10, 16);
        let popup = dropdown.popup(narrow).unwrap();
        assert_eq!(popup.area, Rect::new(0, 3, 10, 2));
        assert_eq!(dropdown.option_at(9, 4, narrow), Some(1));
        assert_eq!(dropdown.option_at(10, 4, narrow), None);
    }

    #[test]
    fn long_lists_have_a_row_cap_and_keyboard_and_wheel_scrolling() {
        let options = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"];
        let bounds = Rect::new(0, 0, 20, 20);
        let (dropdown, mut state) = opened(&options, Rect::new(2, 1, 12, 1));
        assert_eq!(dropdown.popup(bounds).unwrap().visible, MAX_VISIBLE_OPTIONS);
        for _ in 0..11 {
            dropdown.handle_key(&mut state, key(KeyCode::Down), bounds);
        }
        assert_eq!(state.selected, 11);
        assert_eq!(state.scroll, 4);
        let popup = dropdown.popup_for(state, bounds).unwrap();
        assert_eq!(popup.first, 4);
        let current = Dropdown::new(&options, state, dropdown.area);
        assert_eq!(
            current.option_at(popup.area.x + 1, popup.area.bottom() - 1, bounds),
            Some(11)
        );
        current.scroll(&mut state, -2, bounds);
        assert_eq!(state.scroll, 2);
        assert_eq!(state.selected, 11);
    }

    #[test]
    fn rendering_and_hit_testing_share_visible_rows() {
        let options = [
            "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
        ];
        let bounds = Rect::new(0, 0, 24, 16);
        let (dropdown, state) = opened(&options, Rect::new(2, 1, 12, 1));
        let popup = dropdown.popup(bounds).unwrap();
        let backend = ratatui::backend::TestBackend::new(24, 16);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| dropdown.draw_options(frame.buffer_mut(), &Theme::DARK, bounds))
            .unwrap();
        for offset in 0..popup.visible {
            let y = popup.area.y + offset as u16;
            assert_eq!(
                dropdown.option_at(popup.area.x + 1, y, bounds),
                Some(offset)
            );
        }
        let bar: String = (popup.area.y..popup.area.bottom())
            .map(|y| {
                terminal
                    .backend()
                    .buffer()
                    .cell((popup.area.right() - 1, y))
                    .unwrap()
                    .symbol()
            })
            .collect();
        assert!(bar.contains('█'));
        assert!(bar.contains('│'));
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn popup_clears_underlying_text_and_scrollbar_does_not_select() {
        let options = [
            "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
        ];
        let bounds = Rect::new(0, 0, 24, 16);
        let (dropdown, mut state) = opened(&options, Rect::new(2, 1, 12, 1));
        let popup = dropdown.popup(bounds).unwrap();
        let mut buffer = Buffer::empty(bounds);
        for cell in &mut buffer.content {
            cell.set_symbol("x");
        }
        dropdown.draw_options(&mut buffer, &Theme::DARK, bounds);
        assert_eq!(
            buffer
                .cell((popup.area.right() - 2, popup.area.y))
                .unwrap()
                .symbol(),
            " "
        );
        let bar_x = popup.area.right() - 1;
        assert!(dropdown.scrollbar_at(bar_x, popup.area.y, bounds));
        assert_eq!(dropdown.option_at(bar_x, popup.area.y, bounds), None);
        dropdown.scroll_to(&mut state, popup.area.bottom() - 1, bounds);
        assert_eq!(state.scroll, options.len() - popup.visible);
        assert_eq!(state.selected, 0);
    }
}
