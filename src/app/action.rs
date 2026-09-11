use super::{App, Dialog};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    Quit,
    ZoomIn,
    ZoomOut,
    Fit,
    Center,
    Goto,
    Find,
    AddSel,
    DelSel,
    DelAll,
    Prev,
    Next,
    Radix,
    Keys,
    About,
}

impl Action {
    /// Execute the action. Returns `true` when the application should quit.
    pub fn run(self, app: &mut App) -> bool {
        match self {
            Action::Quit => return true,
            Action::Open => app.open_file_dialog(),
            Action::ZoomIn => app.zoom_in(),
            Action::ZoomOut => app.zoom_out(),
            Action::Fit => app.fit(),
            Action::Center => app.center_cursor(),
            Action::Goto => {
                app.dialog = Some(Dialog::Goto);
                app.input.clear();
            }
            Action::Find => {
                app.dialog = Some(Dialog::Find);
                app.input.clear();
                app.find_sel = 0;
            }
            Action::AddSel => app.tree_enter(),
            Action::DelSel => app.remove_selected(),
            Action::DelAll => app.clear_all(),
            Action::Prev => app.jump_transition(false),
            Action::Next => app.jump_transition(true),
            Action::Radix => app.cycle_radix(),
            Action::Keys => app.dialog = Some(Dialog::Keys),
            Action::About => app.dialog = Some(Dialog::About),
        }
        false
    }
}
