use super::{App, Dialog};
use crate::ui::layout::pt_in;
use ratatui::layout::Rect;

#[derive(Clone, Copy)]
pub(crate) struct WindowState {
    pub area: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowHit {
    Move,
    Resize {
        left: bool,
        right: bool,
        top: bool,
        bottom: bool,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct WindowDrag {
    dialog: Dialog,
    hit: WindowHit,
    col: u16,
    row: u16,
    area: Rect,
}

impl App {
    pub(crate) fn dialog_window_area(&self, dialog: Dialog, screen: Rect) -> Option<Rect> {
        self.dialog_windows
            .get(&dialog)
            .map(|state| clamp_area(state.area, screen, min_size(dialog)))
    }

    pub(crate) fn begin_window_drag(&mut self, dialog: Dialog, col: u16, row: u16) -> bool {
        let screen = self.last_area;
        let area = crate::ui::dialog::dialog_rect(screen, self, dialog);
        let close = crate::ui::dialog::close_button(screen, self, dialog);
        let Some(hit) = window_hit(dialog, area, close, col, row) else {
            return false;
        };
        self.window_drag = Some(WindowDrag {
            dialog,
            hit,
            col,
            row,
            area,
        });
        true
    }

    pub(crate) fn drag_window_to(&mut self, col: u16, row: u16) {
        let Some(drag) = self.window_drag else {
            return;
        };
        if self.dialog != Some(drag.dialog) {
            return;
        }
        let screen = self.last_area;
        let dx = col as i32 - drag.col as i32;
        let dy = row as i32 - drag.row as i32;
        let min = min_size(drag.dialog);
        let area = match drag.hit {
            WindowHit::Move => move_area(drag.area, screen, min, dx, dy),
            WindowHit::Resize {
                left,
                right,
                top,
                bottom,
            } => resize_area(drag.area, screen, min, dx, dy, left, right, top, bottom),
        };
        self.dialog_windows
            .insert(drag.dialog, WindowState { area });
        self.needs_redraw = true;
    }

    pub(crate) fn end_window_drag(&mut self) {
        self.window_drag = None;
    }
}

fn window_hit(dialog: Dialog, area: Rect, close: Rect, col: u16, row: u16) -> Option<WindowHit> {
    if !pt_in(area, col, row) || area.width < 2 || area.height < 2 {
        return None;
    }
    let left = col == area.x;
    let right = col == area.right() - 1;
    let top = row == area.y;
    let bottom = row == area.bottom() - 1;
    if (left || right) && (top || bottom) {
        return Some(WindowHit::Resize {
            left,
            right,
            top,
            bottom,
        });
    }
    if left || right {
        return Some(WindowHit::Resize {
            left,
            right,
            top: false,
            bottom: false,
        });
    }
    if bottom {
        return Some(WindowHit::Resize {
            left: false,
            right: false,
            top: false,
            bottom: true,
        });
    }
    if top && (dialog == Dialog::AddSignals || !pt_in(close, col, row)) {
        return Some(WindowHit::Move);
    }
    None
}

fn min_size(dialog: Dialog) -> (u16, u16) {
    match dialog {
        Dialog::Open => (30, 7),
        Dialog::AddSignals => (40, 10),
        Dialog::Settings => (64, 12),
        Dialog::ColorPicker => (58, 18),
        Dialog::CreateBus => (40, 8),
        Dialog::Find => (40, 8),
        Dialog::Follow | Dialog::Dataflow | Dialog::Compare | Dialog::Markers | Dialog::Keys => {
            (36, 5)
        }
        _ => (36, 7),
    }
}

fn clamp_area(mut area: Rect, screen: Rect, min: (u16, u16)) -> Rect {
    area.width = area.width.max(min.0).min(screen.width);
    area.height = area.height.max(min.1).min(screen.height);
    let max_x = screen.right().saturating_sub(area.width).max(screen.x);
    let max_y = screen.bottom().saturating_sub(area.height).max(screen.y);
    area.x = area.x.clamp(screen.x, max_x);
    area.y = area.y.clamp(screen.y, max_y);
    area
}

fn move_area(area: Rect, screen: Rect, min: (u16, u16), dx: i32, dy: i32) -> Rect {
    let mut moved = area;
    moved.x = (area.x as i32 + dx).clamp(i32::from(screen.x), i32::from(screen.right())) as u16;
    moved.y = (area.y as i32 + dy).clamp(i32::from(screen.y), i32::from(screen.bottom())) as u16;
    clamp_area(moved, screen, min)
}

#[allow(clippy::too_many_arguments)]
fn resize_area(
    area: Rect,
    screen: Rect,
    min: (u16, u16),
    dx: i32,
    dy: i32,
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
) -> Rect {
    let (mut x0, mut x1) = (area.x as i32, area.right() as i32);
    let (mut y0, mut y1) = (area.y as i32, area.bottom() as i32);
    let min_w = min.0.min(screen.width) as i32;
    let min_h = min.1.min(screen.height) as i32;
    if left {
        x0 = (x0 + dx).clamp(screen.x as i32, x1 - min_w);
    }
    if right {
        x1 = (x1 + dx).clamp(x0 + min_w, screen.right() as i32);
    }
    if top {
        y0 = (y0 + dy).clamp(screen.y as i32, y1 - min_h);
    }
    if bottom {
        y1 = (y1 + dy).clamp(y0 + min_h, screen.bottom() as i32);
    }
    clamp_area(
        Rect::new(x0 as u16, y0 as u16, (x1 - x0) as u16, (y1 - y0) as u16),
        screen,
        min,
    )
}

#[cfg(test)]
mod tests {
    use crate::app::tests::app_with;
    use crate::app::Dialog;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

    const VCD: &str = "$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n";

    fn mouse(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn title_drag_moves_without_resizing() {
        let mut app = app_with(VCD);
        app.open_dialog(Dialog::Keys);
        let before = crate::ui::dialog::dialog_rect(app.last_area, &app, Dialog::Keys);
        crate::app::handle_mouse(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                before.x + 5,
                before.y,
            ),
        );
        crate::app::handle_mouse(
            &mut app,
            mouse(
                MouseEventKind::Drag(MouseButton::Left),
                before.x + 12,
                before.y + 1,
            ),
        );
        crate::app::handle_mouse(
            &mut app,
            mouse(
                MouseEventKind::Up(MouseButton::Left),
                before.x + 12,
                before.y + 1,
            ),
        );
        let after = crate::ui::dialog::dialog_rect(app.last_area, &app, Dialog::Keys);
        assert_eq!((after.x, after.y), (before.x + 7, before.y + 1));
        assert_eq!((after.width, after.height), (before.width, before.height));
        assert!(!app.is_dragging());
    }

    #[test]
    fn borders_and_corners_resize_with_minimum_and_screen_limits() {
        let mut app = app_with(VCD);
        app.open_dialog(Dialog::Settings);
        let before = crate::ui::dialog::dialog_rect(app.last_area, &app, Dialog::Settings);
        let x = before.right() - 1;
        let y = before.bottom() - 1;
        crate::app::handle_mouse(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), x, y),
        );
        crate::app::handle_mouse(
            &mut app,
            mouse(MouseEventKind::Drag(MouseButton::Left), x + 30, y + 10),
        );
        let after = crate::ui::dialog::dialog_rect(app.last_area, &app, Dialog::Settings);
        assert_eq!(after.right(), app.last_area.right());
        assert_eq!(after.bottom(), app.last_area.bottom());
        assert!(after.width > before.width && after.height > before.height);

        crate::app::handle_mouse(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                after.x,
                after.bottom() - 1,
            ),
        );
        crate::app::handle_mouse(
            &mut app,
            mouse(MouseEventKind::Drag(MouseButton::Left), after.x + 500, 0),
        );
        let shrunk = crate::ui::dialog::dialog_rect(app.last_area, &app, Dialog::Settings);
        assert_eq!((shrunk.width, shrunk.height), (64, 12));
        crate::app::handle_mouse(&mut app, mouse(MouseEventKind::Up(MouseButton::Left), 0, 0));
        assert!(!app.is_dragging());
    }

    #[test]
    fn browser_and_add_picker_use_the_moved_geometry() {
        let mut app = app_with(VCD);
        app.open_dialog(Dialog::Open);
        let before = crate::ui::dialog::dialog_rect(app.last_area, &app, Dialog::Open);
        crate::app::handle_mouse(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                before.x + 4,
                before.y,
            ),
        );
        crate::app::handle_mouse(
            &mut app,
            mouse(
                MouseEventKind::Drag(MouseButton::Left),
                before.x + 9,
                before.y + 2,
            ),
        );
        let after = crate::ui::dialog::dialog_rect(app.last_area, &app, Dialog::Open);
        assert_eq!(after.x, before.x + 5);
        app.dialog = None;

        app.open_add_signals();
        let before = crate::ui::add::layout_for(app.last_area, &app).outer;
        crate::app::handle_mouse(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                before.x + 4,
                before.y,
            ),
        );
        crate::app::handle_mouse(
            &mut app,
            mouse(
                MouseEventKind::Drag(MouseButton::Left),
                before.x + 5,
                before.y + 3,
            ),
        );
        let after = crate::ui::add::layout_for(app.last_area, &app).outer;
        assert_eq!((after.x, after.y), (before.x + 1, before.y + 3));
        app.sync_layout(Rect::new(0, 0, 60, 20));
        let clamped = crate::ui::add::layout_for(app.last_area, &app).outer;
        assert!(clamped.right() <= 60 && clamped.bottom() <= 20);
    }
}
