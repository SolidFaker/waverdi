use super::{App, Dialog, Focus, ListRow};
use crate::ui::menubar;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Handle one key press. Returns `true` when the application should quit.
pub fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'q'))
    {
        return true;
    }
    if app.dialog.is_some() {
        return dialog_key(app, key);
    }
    if app.ctx_menu.is_some() {
        return ctx_key(app, key);
    }
    if app.menu.open.is_some() {
        return menu_key(app, key);
    }
    match key.code {
        KeyCode::Char('q') => true,
        KeyCode::Char('o') => {
            app.open_file_dialog();
            false
        }
        KeyCode::Char('O') => {
            app.open_tui_browser();
            false
        }
        KeyCode::F(1) | KeyCode::Char('?') => {
            app.dialog = Some(Dialog::Keys);
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
        KeyCode::Char('f') => {
            app.fit();
            false
        }
        KeyCode::Char('c') => {
            app.center_cursor();
            false
        }
        KeyCode::Char('g') => {
            app.dialog = Some(Dialog::Goto);
            app.input.clear();
            false
        }
        KeyCode::Char('s') => {
            app.dialog = Some(Dialog::Find);
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
        KeyCode::Char('d') => {
            app.remove_selected();
            false
        }
        KeyCode::Char('x') => {
            app.clear_all();
            false
        }
        KeyCode::Char('a') => {
            app.tree_enter();
            false
        }
        KeyCode::Enter => {
            match app.focus {
                Focus::Tree => app.tree_enter(),
                Focus::List => app.list_enter(),
                Focus::Wave => {}
            }
            false
        }
        KeyCode::Tab => {
            app.focus = app.focus.next();
            false
        }
        KeyCode::Up => {
            match app.focus {
                Focus::Tree => app.move_tree(-1),
                _ => app.move_sel(-1),
            }
            false
        }
        KeyCode::Down => {
            match app.focus {
                Focus::Tree => app.move_tree(1),
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
            if let Some(ListRow::Group {
                path, collapsed, ..
            }) = app.selected_row()
            {
                if app.focus == Focus::List && !collapsed {
                    app.set_group_collapsed(&path, true);
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
            if let Some(ListRow::Group {
                path, collapsed, ..
            }) = app.selected_row()
            {
                if app.focus == Focus::List && collapsed {
                    app.set_group_collapsed(&path, false);
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
            if let Some(wf) = &app.wf {
                app.cursor = wf.start;
            }
            app.center_cursor();
            false
        }
        KeyCode::End => {
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
    match key.code {
        KeyCode::Esc => app.dialog = None,
        KeyCode::Enter => match app.dialog {
            Some(Dialog::Goto) => app.apply_goto(),
            Some(Dialog::FindValue) => app.apply_find_value(),
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
    let Some(sel) = app.ctx_menu.as_ref().map(|menu| menu.sel) else {
        return false;
    };
    let count = app.ctx_items().len();
    match key.code {
        KeyCode::Esc | KeyCode::Left => app.ctx_menu = None,
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
        KeyCode::Enter => {
            let item = app.ctx_items()[sel].1;
            app.run_ctx_item(item);
        }
        _ => {}
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
