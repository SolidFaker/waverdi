use super::{App, Dialog};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    OpenTui,
    AddSignals,
    Quit,
    ZoomIn,
    ZoomOut,
    Fit,
    Center,
    Goto,
    Find,
    FindValue,
    FindNextValue,
    FindPrevValue,
    AddSel,
    DelSel,
    DelAll,
    Prev,
    Next,
    Radix,
    LoadFilelist,
    Settings,
    Keys,
    About,
}

impl Action {
    /// Execute the action. Returns `true` when the application should quit.
    pub fn run(self, app: &mut App) -> bool {
        match self {
            Action::Quit => return true,
            Action::Open => app.open_file_dialog(),
            Action::OpenTui => app.open_tui_browser(),
            Action::AddSignals => app.open_add_signals(),
            Action::ZoomIn => app.zoom_in(),
            Action::ZoomOut => app.zoom_out(),
            Action::Fit => app.fit(),
            Action::Center => app.center_cursor(),
            Action::Goto => {
                app.open_dialog(Dialog::Goto);
                app.input.clear();
            }
            Action::Find => {
                app.open_dialog(Dialog::Find);
                app.input.clear();
                app.find_sel = 0;
            }
            Action::FindValue => app.find_value_dialog(),
            Action::FindNextValue => app.search_value(true),
            Action::FindPrevValue => app.search_value(false),
            Action::AddSel => app.tree_enter(),
            Action::DelSel => app.remove_selected(),
            Action::DelAll => app.clear_all(),
            Action::Prev => app.jump_transition(false),
            Action::Next => app.jump_transition(true),
            Action::Radix => app.cycle_radix(),
            Action::LoadFilelist => app.open_filelist_dialog(),
            Action::Settings => {
                app.settings_sel = 0;
                app.open_dialog(Dialog::Settings);
            }
            Action::Keys => app.open_dialog(Dialog::Keys),
            Action::About => app.open_dialog(Dialog::About),
        }
        false
    }
}
