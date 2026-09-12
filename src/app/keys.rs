use super::{App, Dialog, Focus, ListRow};
use crate::ui::menubar;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Handle one key press. Returns `true` when the application should quit.
pub fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.kind == KeyEventKind::Release {
        return false;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'q'))
    {
        return true;
    }
    if app.pending_g {
        app.pending_g = false;
        match key.code {
            KeyCode::Char('e') => app.jump_edge(false, Some(0)),
            KeyCode::Char('g') => match app.focus {
                Focus::Tree => {
                    app.tree_sel = 0;
                    app.tree_scroll_to_sel();
                }
                _ => app.select_first_row(),
            },
            _ => {}
        }
        return false;
    }
    if app.pending_d {
        app.pending_d = false;
        if key.code == KeyCode::Char('d') {
            app.delete_selected_signals();
        }
        return false;
    }
    if app.dialog.is_some() {
        return dialog_key(app, key);
    }
    if app.ctx_menu.is_some() {
        return ctx_key(app, key);
    }
    if app.time_menu.is_some() {
        return time_menu_key(app, key);
    }
    if app.menu.open.is_some() {
        return menu_key(app, key);
    }
    match key.code {
        KeyCode::Char('q') => true,
        KeyCode::Esc => {
            app.selection.clear();
            app.sel_anchor = None;
            app.visual = false;
            false
        }
        KeyCode::Char('o') => {
            app.open_file_dialog();
            false
        }
        KeyCode::Char('O') => {
            app.open_tui_browser();
            false
        }
        KeyCode::F(2) => {
            app.settings_sel = 0;
            app.open_dialog(Dialog::Settings);
            false
        }
        KeyCode::F(1) | KeyCode::Char('?') => {
            app.open_dialog(Dialog::Keys);
            false
        }
        _ if app.wf.is_none() => false,
        KeyCode::Char('z') => {
            app.zoom_in();
            false
        }
        KeyCode::Char('Z') => {
            app.zoom_out();
            false
        }
        KeyCode::Char('=') | KeyCode::Char('+') => {
            app.zoom_in();
            false
        }
        KeyCode::Char('-') => {
            app.zoom_out();
            false
        }
        KeyCode::Char('f') => {
            app.fit();
            false
        }
        KeyCode::Char('c') => {
            app.center_cursor();
            false
        }
        KeyCode::Char('g') => {
            app.pending_g = true;
            app.msg("g-  (e: previous falling edge, g: first row)");
            false
        }
        KeyCode::Char(':') => {
            app.open_dialog(Dialog::Goto);
            app.input.clear();
            false
        }
        KeyCode::Char('s') => {
            app.open_dialog(Dialog::Find);
            app.input.clear();
            app.find_sel = 0;
            false
        }
        KeyCode::Char('v') => {
            app.find_value_dialog();
            false
        }
        KeyCode::Char('n') => {
            app.search_value(true);
            false
        }
        KeyCode::Char('N') => {
            app.search_value(false);
            false
        }
        KeyCode::Char('r') => {
            if app.focus == Focus::List {
                if let Some(ListRow::Group { id, .. }) = app.selected_row() {
                    app.open_rename_group(id);
                    return false;
                }
            }
            app.cycle_radix();
            false
        }
        KeyCode::Char(',') => {
            app.jump_transition(false);
            false
        }
        KeyCode::Char('.') => {
            app.jump_transition(true);
            false
        }
        KeyCode::Char('0') => {
            if let Some(wf) = &app.wf {
                app.cursor = wf.start;
            }
            app.reveal_cursor();
            false
        }
        KeyCode::Char('$') => {
            if let Some(wf) = &app.wf {
                app.cursor = wf.end;
            }
            app.reveal_cursor();
            false
        }
        KeyCode::Char('G') => {
            match app.focus {
                Focus::Tree => app.move_tree(1000),
                _ => app.move_sel(1000),
            }
            false
        }
        KeyCode::Char('d') => {
            app.pending_d = true;
            app.msg("d-  (dd: cut the selection into the register)");
            false
        }
        KeyCode::Char('p') => {
            app.paste_register();
            false
        }
        KeyCode::Char('V') => {
            if app.visual {
                app.visual = false;
            } else if app.focus != Focus::Source {
                if let Some(sig) = app.selected_signal() {
                    app.visual = true;
                    app.selection = vec![sig];
                    app.sel_anchor = Some(sig);
                }
            }
            false
        }
        KeyCode::Char('x') if app.focus == Focus::Wave => {
            // Alias for `dd` in the waveform pane.
            app.delete_selected_signals();
            false
        }
        KeyCode::Char('j') => {
            if app.focus == Focus::Source {
                app.move_source_cursor(1, 0);
            } else if app.visual {
                app.extend_selection(1);
            } else {
                match app.focus {
                    Focus::Tree => app.move_tree(1),
                    _ => app.move_sel(1),
                }
            }
            false
        }
        KeyCode::Char('k') => {
            if app.focus == Focus::Source {
                app.move_source_cursor(-1, 0);
            } else if app.visual {
                app.extend_selection(-1);
            } else {
                match app.focus {
                    Focus::Tree => app.move_tree(-1),
                    _ => app.move_sel(-1),
                }
            }
            false
        }
        KeyCode::Char('J') => {
            app.move_selected_signal(1);
            false
        }
        KeyCode::Char('K') => {
            app.move_selected_signal(-1);
            false
        }
        KeyCode::Char(' ') => {
            if let Some(row) = app.sel_row {
                app.toggle_row_selection(row);
            }
            false
        }
        KeyCode::Char('h') if app.focus == Focus::List => {
            app.show_full_names = !app.show_full_names;
            app.msg(if app.show_full_names {
                "signal names: full hierarchy"
            } else {
                "signal names: short"
            });
            false
        }
        KeyCode::Char('h') if app.focus == Focus::Source => {
            app.move_source_cursor(0, -1);
            false
        }
        KeyCode::Char('h') if app.focus == Focus::Wave => {
            let step = (app.scale.round() as i64).max(1);
            app.move_cursor(-step);
            false
        }
        KeyCode::Char('l') if app.focus == Focus::Source => {
            app.move_source_cursor(0, 1);
            false
        }
        KeyCode::Char('l') if app.focus == Focus::Wave => {
            let step = (app.scale.round() as i64).max(1);
            app.move_cursor(step);
            false
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Add the Source selection to the waveform (Verdi shortcut).
            app.add_source_selection();
            false
        }
        KeyCode::Char('w') => {
            app.jump_edge(true, Some(1));
            false
        }
        KeyCode::Char('e') => {
            app.jump_edge(true, Some(0));
            false
        }
        KeyCode::Char('b') => {
            app.jump_edge(false, Some(1));
            false
        }
        KeyCode::Char('a') => {
            match app.focus {
                Focus::Tree => app.tree_enter(),
                Focus::Source => app.add_source_word(),
                _ => {}
            }
            false
        }
        KeyCode::Enter => {
            match app.focus {
                Focus::Tree => app.tree_enter(),
                Focus::Source => app.add_source_word(),
                Focus::List => app.list_enter(),
                Focus::Wave => {}
            }
            false
        }
        KeyCode::Tab => {
            app.focus = app.focus.next();
            false
        }
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
            match app.focus {
                Focus::Tree => app.move_tree(-1),
                Focus::Source => app.extend_source_selection(-1),
                _ => app.extend_selection(-1),
            }
            false
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
            match app.focus {
                Focus::Tree => app.move_tree(1),
                Focus::Source => app.extend_source_selection(1),
                _ => app.extend_selection(1),
            }
            false
        }
        KeyCode::Up => {
            match app.focus {
                Focus::Tree => app.move_tree(-1),
                Focus::Source => app.move_source_cursor(-1, 0),
                _ => app.move_sel(-1),
            }
            false
        }
        KeyCode::Down => {
            match app.focus {
                Focus::Tree => app.move_tree(1),
                Focus::Source => app.move_source_cursor(1, 0),
                _ => app.move_sel(1),
            }
            false
        }
        KeyCode::PageUp => {
            page(app, false);
            false
        }
        KeyCode::PageDown => {
            page(app, true);
            false
        }
        KeyCode::Left => {
            if app.focus == Focus::Source {
                app.move_source_cursor(0, -1);
                return false;
            }
            if let Some(ListRow::Group {
                index, collapsed, ..
            }) = app.selected_row()
            {
                if app.focus == Focus::List && !collapsed {
                    app.set_group_collapsed(index, true);
                    return false;
                }
            }
            let step = app.span()
                / if key.modifiers.contains(KeyModifiers::SHIFT) {
                    10.0
                } else {
                    100.0
                };
            app.move_cursor(-(step as i64).max(1));
            false
        }
        KeyCode::Right => {
            if app.focus == Focus::Source {
                app.move_source_cursor(0, 1);
                return false;
            }
            if let Some(ListRow::Group {
                index, collapsed, ..
            }) = app.selected_row()
            {
                if app.focus == Focus::List && collapsed {
                    app.set_group_collapsed(index, false);
                    return false;
                }
            }
            let step = app.span()
                / if key.modifiers.contains(KeyModifiers::SHIFT) {
                    10.0
                } else {
                    100.0
                };
            app.move_cursor((step as i64).max(1));
            false
        }
        KeyCode::Home => {
            if app.focus == Focus::Source {
                app.move_source_cursor(0, -1_000_000);
                return false;
            }
            if let Some(wf) = &app.wf {
                app.cursor = wf.start;
            }
            app.center_cursor();
            false
        }
        KeyCode::End => {
            if app.focus == Focus::Source {
                app.move_source_cursor(0, 1_000_000);
                return false;
            }
            if let Some(wf) = &app.wf {
                app.cursor = wf.end;
            }
            app.center_cursor();
            false
        }
        _ => false,
    }
}

fn page(app: &mut App, down: bool) {
    if app.focus == Focus::Source {
        app.source_page(down);
        return;
    }
    let (tree_h, rows_h) = {
        let l = app.layout();
        (l.tree_height().max(1), l.rows_h.max(1))
    };
    if app.focus == Focus::Tree {
        if down {
            app.tree_scroll += tree_h;
            app.tree_sel = app.tree_sel.saturating_add(tree_h);
            app.tree_scroll_to_sel();
        } else {
            app.tree_scroll = app.tree_scroll.saturating_sub(tree_h);
            app.tree_sel = app.tree_sel.min(app.tree_scroll);
        }
    } else if down {
        app.row_scroll += rows_h;
        if let Some(row) = app.sel_row {
            app.sel_row = Some((row + rows_h).min(app.display.len().saturating_sub(1)));
        }
        app.scroll_to_sel();
    } else {
        app.row_scroll = app.row_scroll.saturating_sub(rows_h);
        app.scroll_to_sel();
    }
}

fn dialog_key(app: &mut App, key: KeyEvent) -> bool {
    if app.dialog == Some(Dialog::Open) {
        browser_key(app, key);
        return false;
    }
    if app.dialog == Some(Dialog::CreateBus) {
        bus_builder_key(app, key);
        return false;
    }
    match key.code {
        KeyCode::Esc => {
            app.dialog = None;
            app.renaming_group = None;
        }
        KeyCode::Up if app.dialog == Some(Dialog::Settings) => {
            let len = app.settings_len();
            app.settings_sel = app.settings_sel.checked_sub(1).unwrap_or(len - 1);
        }
        KeyCode::Down if app.dialog == Some(Dialog::Settings) => {
            app.settings_sel = (app.settings_sel + 1) % app.settings_len();
        }
        KeyCode::Left | KeyCode::Right if app.dialog == Some(Dialog::Settings) => {
            let delta = if key.code == KeyCode::Left { -1 } else { 1 };
            settings_change(app, delta);
        }
        KeyCode::Char('r') if app.dialog == Some(Dialog::Settings) => settings_reset(app),
        KeyCode::Enter if app.dialog == Some(Dialog::Settings) => settings_change(app, 1),
        KeyCode::Enter => match app.dialog {
            Some(Dialog::Goto) => app.apply_goto(),
            Some(Dialog::FindValue) => app.apply_find_value(),
            Some(Dialog::SplitBus) => app.apply_split_bus(),
            Some(Dialog::Filelist) => app.apply_load_filelist(),
            Some(Dialog::GroupName) => app.apply_rename_group(),
            Some(Dialog::Find) => {
                let matches = app.find_matches();
                if !matches.is_empty() {
                    let pick = matches[app.find_sel.min(matches.len() - 1)];
                    app.add_signal(pick);
                }
                app.dialog = None;
            }
            _ => app.dialog = None,
        },
        KeyCode::Char(c) => app.input.insert(c),
        KeyCode::Backspace => app.input.backspace(),
        KeyCode::Delete => app.input.delete(),
        KeyCode::Left => app.input.left(),
        KeyCode::Right => app.input.right(),
        KeyCode::Home => app.input.home(),
        KeyCode::End => app.input.end(),
        KeyCode::Up if app.dialog == Some(Dialog::Keys) => {
            app.dialog_scroll = app.dialog_scroll.saturating_sub(1);
        }
        KeyCode::Down if app.dialog == Some(Dialog::Keys) => {
            app.dialog_scroll = app.dialog_scroll.saturating_add(1);
        }
        KeyCode::PageUp if app.dialog.is_some() => {
            let page = crate::ui::dialog::visible_rows(app.last_area, app, app.dialog.unwrap());
            app.dialog_scroll = app.dialog_scroll.saturating_sub(page);
        }
        KeyCode::PageDown if app.dialog.is_some() => {
            let page = crate::ui::dialog::visible_rows(app.last_area, app, app.dialog.unwrap());
            app.dialog_scroll = app.dialog_scroll.saturating_add(page);
        }
        KeyCode::Up if app.dialog == Some(Dialog::Find) => {
            app.find_sel = app.find_sel.saturating_sub(1);
        }
        KeyCode::Down if app.dialog == Some(Dialog::Find) => {
            let n = app.find_matches().len();
            app.find_sel = (app.find_sel + 1).min(n.saturating_sub(1));
        }
        _ => {}
    }
    false
}

enum BrowserCmd {
    Stay,
    Close,
    Load(String),
}

/// Keys of the "Create Bus" ordering window.
fn bus_builder_key(app: &mut App, key: KeyEvent) {
    let reorder = key.modifiers.contains(KeyModifiers::SHIFT)
        || key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => app.bus_builder_cancel(),
        KeyCode::Enter => app.bus_builder_commit(),
        KeyCode::Up if reorder => app.bus_builder_reorder(-1),
        KeyCode::Down if reorder => app.bus_builder_reorder(1),
        KeyCode::Up => app.bus_builder_move(-1),
        KeyCode::Down => app.bus_builder_move(1),
        KeyCode::Home => app.bus_builder_move(-1000),
        KeyCode::End => app.bus_builder_move(1000),
        KeyCode::Char('h') => app.bus_builder_trim(false, -1),
        KeyCode::Char('l') => app.bus_builder_trim(false, 1),
        KeyCode::Char('H') => app.bus_builder_trim(true, -1),
        KeyCode::Char('L') => app.bus_builder_trim(true, 1),
        KeyCode::Char('x') => app.bus_builder_reset_range(),
        KeyCode::Char('s') => app.bus_builder_sort(true),
        KeyCode::Char('S') => app.bus_builder_sort(false),
        KeyCode::Char('r') => app.bus_builder_reverse(),
        _ => {}
    }
}

fn browser_key(app: &mut App, key: KeyEvent) {
    let rows = crate::ui::dialog::browser_rows(app.layout().area);
    let cmd = {
        let Some(browser) = app.browser.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => BrowserCmd::Close,
            KeyCode::Up => {
                browser.move_sel(-1, rows);
                BrowserCmd::Stay
            }
            KeyCode::Down => {
                browser.move_sel(1, rows);
                BrowserCmd::Stay
            }
            KeyCode::PageUp => {
                browser.move_sel(-(rows as i64), rows);
                BrowserCmd::Stay
            }
            KeyCode::PageDown => {
                browser.move_sel(rows as i64, rows);
                BrowserCmd::Stay
            }
            KeyCode::Home => {
                browser.select(0, rows);
                BrowserCmd::Stay
            }
            KeyCode::End => {
                browser.select(browser.entries.len().saturating_sub(1), rows);
                BrowserCmd::Stay
            }
            KeyCode::Backspace | KeyCode::Left => {
                browser.go_parent();
                BrowserCmd::Stay
            }
            KeyCode::Enter => match browser.activate() {
                Some(path) => BrowserCmd::Load(path.display().to_string()),
                None => BrowserCmd::Stay,
            },
            _ => BrowserCmd::Stay,
        }
    };
    match cmd {
        BrowserCmd::Stay => {}
        BrowserCmd::Close => app.dialog = None,
        BrowserCmd::Load(path) => {
            app.load(&path);
        }
    }
}

fn ctx_key(app: &mut App, key: KeyEvent) -> bool {
    use crate::app::CtxEntry;
    let Some(sel) = app.ctx_menu.as_ref().map(|menu| menu.sel) else {
        return false;
    };
    let in_submenu = app
        .ctx_menu
        .as_ref()
        .map(|menu| menu.submenu.is_some())
        .unwrap_or(false);
    let count = app.ctx_level().len();
    match key.code {
        KeyCode::Esc | KeyCode::Left => {
            if in_submenu {
                app.close_ctx_submenu();
            } else {
                app.ctx_menu = None;
            }
        }
        KeyCode::Right => {
            if !in_submenu && matches!(app.ctx_level().get(sel), Some(CtxEntry::Submenu(..))) {
                app.open_ctx_submenu(sel);
            }
        }
        KeyCode::Up => {
            if let Some(menu) = app.ctx_menu.as_mut() {
                menu.sel = sel.checked_sub(1).unwrap_or(count - 1);
            }
        }
        KeyCode::Down => {
            if let Some(menu) = app.ctx_menu.as_mut() {
                menu.sel = (sel + 1) % count;
            }
        }
        KeyCode::Enter => match app.ctx_level().get(sel).copied() {
            Some(CtxEntry::Submenu(..)) => app.open_ctx_submenu(sel),
            Some(CtxEntry::Item(_, item)) => app.run_ctx_item(item),
            None => {}
        },
        _ => {}
    }
    false
}

/// Change the selected settings row (theme, UI colour or waveform colour).
fn settings_change(app: &mut App, delta: i64) {
    if app.settings_sel == 0 {
        app.cycle_theme(delta);
    } else if let Some(setting) = app.settings_ui(app.settings_sel) {
        app.cycle_ui_setting(setting, delta);
    } else if let Some(setting) = app.settings_setting(app.settings_sel) {
        app.cycle_wave_setting(setting, delta);
    }
}

/// Reset the selected settings row to the active theme's value.
fn settings_reset(app: &mut App) {
    if let Some(setting) = app.settings_ui(app.settings_sel) {
        app.reset_ui_setting(setting);
    } else if let Some(setting) = app.settings_setting(app.settings_sel) {
        app.reset_wave_setting(setting);
    }
}

/// Keys of the time-base dropdown in the nWave shortcut bar.
fn time_menu_key(app: &mut App, key: KeyEvent) -> bool {
    use crate::waveform::TimeBase;
    let Some(sel) = app.time_menu else {
        return false;
    };
    let count = TimeBase::CYCLE.len();
    match key.code {
        KeyCode::Esc => app.time_menu = None,
        KeyCode::Up => app.time_menu = Some(sel.checked_sub(1).unwrap_or(count - 1)),
        KeyCode::Down => app.time_menu = Some((sel + 1) % count),
        KeyCode::Enter => app.set_time_base(TimeBase::CYCLE[sel]),
        _ => app.time_menu = None,
    }
    false
}

fn menu_key(app: &mut App, key: KeyEvent) -> bool {
    let Some(menu) = app.menu.open else {
        return false;
    };
    let count = menubar::menu_len(menu);
    match key.code {
        KeyCode::Esc => app.menu.open = None,
        KeyCode::Up => app.menu.sel = app.menu.sel.checked_sub(1).unwrap_or(count - 1),
        KeyCode::Down => app.menu.sel = (app.menu.sel + 1) % count,
        KeyCode::Enter => {
            let action = menubar::menu_action(menu, app.menu.sel);
            app.menu.open = None;
            return action.run(app);
        }
        KeyCode::Char('q') => {
            app.menu.open = None;
            return true;
        }
        _ => {}
    }
    false
}

#[cfg(test)]
mod tests {
    use crate::app::tests::app_with;
    use crate::app::{handle_key, Dialog, Focus};
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    const VCD: &str = "$timescale 1ns $end\n\
        $var wire 1 ! clk $end\n\
        $var wire 4 \" data $end\n\
        $enddefinitions $end\n\
        #0\n0!\nb0000 \"\n\
        #10\n1!\n\
        #20\n0!\n";

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn vim_keys_move_rows_and_cursor() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.focus = Focus::Wave;
        app.sel_row = Some(1);
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.sel_row, Some(2));
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.sel_row, Some(1));
        let before = app.cursor;
        handle_key(&mut app, key(KeyCode::Char('l')));
        assert!(app.cursor > before, "l should move the cursor right");
        handle_key(&mut app, key(KeyCode::Char('h')));
        assert_eq!(app.cursor, before);
        handle_key(&mut app, key(KeyCode::Char(':')));
        assert_eq!(app.dialog, Some(Dialog::Goto));
    }

    #[test]
    fn vim_edges_and_g_prefix() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        app.sel_row = Some(1);
        app.focus = Focus::Wave;
        app.cursor = 0;
        handle_key(&mut app, key(KeyCode::Char('w'))); // clk rising
        assert_eq!(app.cursor, 10);
        handle_key(&mut app, key(KeyCode::Char('e'))); // falling
        assert_eq!(app.cursor, 20);
        handle_key(&mut app, key(KeyCode::Char('b'))); // previous rising
        assert_eq!(app.cursor, 10);
        handle_key(&mut app, key(KeyCode::Char('g')));
        assert!(app.pending_g);
        handle_key(&mut app, key(KeyCode::Char('e'))); // previous falling
        assert!(!app.pending_g);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn vim_first_last_rows_and_time_ends() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.focus = Focus::Wave;
        app.sel_row = Some(2);
        handle_key(&mut app, key(KeyCode::Char('g')));
        handle_key(&mut app, key(KeyCode::Char('g'))); // gg: first row
        assert_eq!(app.sel_row, Some(0));
        handle_key(&mut app, key(KeyCode::Char('G'))); // G: last row
        assert_eq!(app.sel_row, Some(2));
        app.cursor = 10;
        handle_key(&mut app, key(KeyCode::Char('0'))); // 0: start of time
        assert_eq!(app.cursor, 0);
        handle_key(&mut app, key(KeyCode::Char('$'))); // $: end of time
        assert_eq!(app.cursor, app.wf.as_ref().unwrap().end);
    }

    #[test]
    fn key_release_events_are_ignored() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.focus = Focus::Wave;
        app.sel_row = Some(1);
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        handle_key(&mut app, release);
        assert_eq!(app.sel_row, Some(1));
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.sel_row, Some(2));
    }

    #[test]
    fn signal_reorder_and_selection_keys() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.focus = Focus::Wave;
        app.sel_row = Some(1); // clk
        handle_key(&mut app, key(KeyCode::Char('K'))); // K moves up: already first
        assert_eq!(app.display, vec![0, 1]);
        handle_key(&mut app, key(KeyCode::Char('J'))); // J moves down
        assert_eq!(app.display, vec![1, 0]);
        assert_eq!(app.selected_signal(), Some(0));
        handle_key(&mut app, key(KeyCode::Char('K'))); // back up
        assert_eq!(app.display, vec![0, 1]);
        assert_eq!(app.selected_signal(), Some(0));
        handle_key(&mut app, key(KeyCode::Char(' ')));
        assert_eq!(app.selection, vec![0]);
        handle_key(&mut app, key(KeyCode::Char('j')));
        handle_key(&mut app, key(KeyCode::Char(' ')));
        assert_eq!(app.selection, vec![0, 1]);
        handle_key(&mut app, key(KeyCode::Esc));
        assert!(app.selection.is_empty());
    }

    #[test]
    fn shift_arrows_extend_selection() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.focus = Focus::List;
        app.sel_row = Some(1);
        handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
        assert_eq!(app.sel_row, Some(2));
        assert_eq!(app.selection, vec![0, 1]);
    }

    #[test]
    fn minus_and_equals_zoom() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        // The initial fit already shows the whole range: zooming out is a no-op.
        let full = app.scale;
        handle_key(&mut app, key(KeyCode::Char('-')));
        assert!((app.scale - full).abs() < full * 1e-12);
        // Zooming in works, zooming out returns to the full range.
        handle_key(&mut app, key(KeyCode::Char('=')));
        assert!(app.scale < full);
        handle_key(&mut app, key(KeyCode::Char('-')));
        assert!((app.scale - full).abs() < full * 1e-9);
    }

    #[test]
    fn visual_mode_cut_and_paste() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.focus = Focus::Wave;
        app.sel_row = Some(1); // clk
        handle_key(&mut app, key(KeyCode::Char('V')));
        assert!(app.visual);
        handle_key(&mut app, key(KeyCode::Char('j'))); // select clk + data
        assert_eq!(app.selection, vec![0, 1]);
        handle_key(&mut app, key(KeyCode::Char('d')));
        assert!(app.pending_d);
        handle_key(&mut app, key(KeyCode::Char('d')));
        assert!(app.display.is_empty());
        assert_eq!(app.register, vec![0, 1]);
        assert!(!app.visual);
        handle_key(&mut app, key(KeyCode::Char('p')));
        assert_eq!(app.display, vec![0, 1]);
    }

    #[test]
    fn keys_dialog_scrolls_with_arrows_and_pages() {
        let mut app = app_with(VCD);
        let page = crate::ui::dialog::visible_rows(app.last_area, &app, Dialog::Keys);
        assert!(page > 0);
        app.open_dialog(Dialog::Keys);
        handle_key(&mut app, key(KeyCode::PageDown));
        assert_eq!(app.dialog_scroll, page);
        handle_key(&mut app, key(KeyCode::PageUp));
        assert_eq!(app.dialog_scroll, 0);
        handle_key(&mut app, key(KeyCode::Down));
        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.dialog_scroll, 2);
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.dialog_scroll, 1);
    }

    #[test]
    fn x_cuts_in_the_waveform_pane() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.focus = Focus::Wave;
        app.sel_row = Some(1); // clk
        handle_key(&mut app, key(KeyCode::Char('x')));
        assert_eq!(app.display, vec![1]);
        assert_eq!(app.register, vec![0]);
        // Outside the waveform pane `x` is no longer bound.
        app.focus = Focus::List;
        app.sel_row = Some(1);
        handle_key(&mut app, key(KeyCode::Char('x')));
        assert_eq!(app.display, vec![1]);
    }

    #[test]
    fn settings_dialog_changes_theme_and_wave_colours() {
        use crate::theme::{Theme, ThemeKind};
        let mut app = app_with(VCD);
        handle_key(&mut app, key(KeyCode::F(2)));
        assert_eq!(app.dialog, Some(Dialog::Settings));
        handle_key(&mut app, key(KeyCode::Right)); // theme row: Dark -> Light
        assert_eq!(app.theme_kind, ThemeKind::Light);
        handle_key(&mut app, key(KeyCode::Down)); // UI background row
        handle_key(&mut app, key(KeyCode::Right));
        assert_ne!(app.theme.bg, Theme::LIGHT.bg);
        handle_key(&mut app, key(KeyCode::Char('r')));
        assert_eq!(app.theme.bg, Theme::LIGHT.bg);
        handle_key(&mut app, key(KeyCode::Down)); // waveform background row
        handle_key(&mut app, key(KeyCode::Right));
        assert_ne!(app.theme.wave_bg, Theme::LIGHT.wave_bg);
        assert_eq!(app.theme.bg, Theme::LIGHT.bg); // UI colour untouched
        handle_key(&mut app, key(KeyCode::Char('r')));
        assert_eq!(app.theme.wave_bg, Theme::LIGHT.wave_bg);
        handle_key(&mut app, key(KeyCode::Esc));
        assert_eq!(app.dialog, None);
    }

    #[test]
    fn source_pane_keys_move_and_add() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $scope module dut $end\n\
            $var wire 4 ! count [3:0] $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\nb0000 !\n";
        let mut app = app_with(vcd);
        let dir = std::env::temp_dir().join(format!("waverdi_src_keys_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("counter.sv");
        std::fs::write(
            &file,
            "module counter(output logic [3:0] count);\n\
             assign count = 4'b0;\n\
             endmodule\n",
        )
        .unwrap();
        app.sources = Some(SourceSet::from_files(vec![file], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        // The `dut` scope (node 2) is defined by `counter` in this dump.
        app.wf.as_mut().unwrap().tree.nodes[2].module = "counter".to_string();
        app.expanded.insert(1);
        app.tree_sel = 2;
        app.sync_source();
        assert!(app.source_view.is_some());

        app.focus = Focus::Source;
        let start = app.source_view.as_ref().unwrap().line;
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert!(app.source_view.as_ref().unwrap().line > start);
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.source_view.as_ref().unwrap().line, start);
        handle_key(&mut app, key(KeyCode::Char('l')));
        assert_eq!(app.source_view.as_ref().unwrap().col, 1);

        // Put the cursor on `count` in the assign line and add it.
        let (line, col) = {
            let view = app.source_view.as_ref().unwrap();
            let line = view
                .lines
                .iter()
                .position(|line| line.contains("assign count"))
                .unwrap();
            let col = view.lines[line].find("count").unwrap();
            (line, col)
        };
        app.set_source_cursor(line, col);
        handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.display, vec![0]);
        assert_eq!(app.focus, Focus::Source);
    }

    #[test]
    fn source_selection_adds_deduplicated_signals() {
        use crate::rtl::{RtlDb, SourceSet};
        let vcd = "$timescale 1ns $end\n\
            $scope module tb $end\n\
            $scope module dut $end\n\
            $var wire 1 ! clk $end\n\
            $var wire 4 \" count [3:0] $end\n\
            $var wire 1 # en $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\n0!\nb0000 \"\n0#\n";
        let mut app = app_with(vcd);
        let dir = std::env::temp_dir().join(format!("waverdi_src_sel_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("counter.sv");
        std::fs::write(
            &file,
            "module counter(input logic clk, input logic en, output logic [3:0] count);\n\
             always_ff @(posedge clk) begin\n\
             if (en) count <= count + 1'b1;\n\
             end\n\
             endmodule\n",
        )
        .unwrap();
        app.sources = Some(SourceSet::from_files(vec![file], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.wf.as_mut().unwrap().tree.nodes[2].module = "counter".to_string();
        app.expanded.insert(1);
        app.tree_sel = 2;
        app.sync_source();

        // Select the always block lines (clk / en / count with duplicates).
        app.set_source_cursor(1, 0);
        app.begin_source_selection();
        app.extend_source_selection_to(3);
        app.focus = Focus::Source;
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.display, vec![0, 2, 1]);
        assert_eq!(app.focus, Focus::Source);

        // The right-click menu offers the same action for a word selection.
        app.clear_all();
        let col = app.source_view.as_ref().unwrap().lines[2]
            .find("en")
            .unwrap();
        app.set_source_cursor(2, col); // `en` in the if
        app.select_source_word();
        app.open_context_menu(crate::app::CtxTarget::Source, 5, 5);
        handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.display, vec![2]);
    }

    #[test]
    fn r_renames_a_group_and_h_toggles_names() {
        let mut app = app_with(VCD);
        app.add_signal(0);
        app.select_row(0); // G0 header
        app.focus = Focus::List;
        handle_key(&mut app, key(KeyCode::Char('r')));
        assert_eq!(app.dialog, Some(Dialog::GroupName));
        assert_eq!(app.input.as_string(), "G0");
        handle_key(&mut app, key(KeyCode::Backspace));
        handle_key(&mut app, key(KeyCode::Backspace));
        for c in "inputs".chars() {
            handle_key(&mut app, key(KeyCode::Char(c)));
        }
        handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.dialog, None);
        assert_eq!(app.groups[0].name, "inputs");
        assert_eq!(app.groups[0].id, 0); // renaming keeps the number
        assert_eq!(app.groups[1].id, 1);
        handle_key(&mut app, key(KeyCode::Char('h')));
        assert!(app.show_full_names);
        handle_key(&mut app, key(KeyCode::Char('h')));
        assert!(!app.show_full_names);
    }
}
