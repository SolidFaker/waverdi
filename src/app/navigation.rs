use super::{App, Focus};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceLocation {
    scope: Vec<String>,
    module: String,
    file: PathBuf,
    line: usize,
    col: usize,
}

impl App {
    fn current_source_location(&self) -> Option<SourceLocation> {
        let view = self.source_view.as_ref()?;
        Some(SourceLocation {
            scope: self.selected_scope_steps(),
            module: view.module.clone(),
            file: view.file.clone(),
            line: view.line,
            col: view.col,
        })
    }

    pub(crate) fn remember_source_location(&mut self) {
        if self.history_restore {
            return;
        }
        let Some(location) = self.current_source_location() else {
            return;
        };
        if self
            .source_history_index
            .and_then(|index| self.source_history.get(index))
            == Some(&location)
        {
            return;
        }
        if let Some(index) = self.source_history_index {
            self.source_history.truncate(index + 1);
        } else {
            self.source_history.clear();
        }
        self.source_history.push(location);
        if self.source_history.len() > 100 {
            self.source_history.remove(0);
        }
        self.source_history_index = Some(self.source_history.len() - 1);
    }

    pub fn history_back(&mut self) {
        let Some(current) = self.current_source_location() else {
            return;
        };
        let Some(index) = self.source_history_index.filter(|index| {
            self.source_history
                .get(*index)
                .is_some_and(|location| *location != current || *index > 0)
        }) else {
            return;
        };
        let target = if self.source_history[index] == current {
            index - 1
        } else {
            self.source_history.truncate(index + 1);
            self.source_history.push(current);
            index
        };
        self.restore_source_location(target);
    }

    pub fn history_forward(&mut self) {
        let Some(current) = self.current_source_location() else {
            return;
        };
        let Some(index) = self.source_history_index.filter(|index| {
            self.source_history.get(*index) == Some(&current)
                && *index + 1 < self.source_history.len()
        }) else {
            return;
        };
        self.restore_source_location(index + 1);
    }

    fn restore_source_location(&mut self, index: usize) {
        let Some(target) = self.source_history.get(index).cloned() else {
            return;
        };
        self.history_restore = true;
        let switched = self.switch_scope(&target.scope);
        self.history_restore = false;
        if !switched {
            return;
        }
        self.set_source_cursor(target.line, target.col);
        self.focus = Focus::Source;
        self.source_history_index = Some(index);
    }

    pub fn hierarchy_up(&mut self) {
        let nodes = self.tree_visible();
        let Some(current) = nodes.get(self.tree_sel) else {
            return;
        };
        let depth = match current {
            super::TreeNode::Scope { depth, .. } => *depth,
        };
        let Some(target) = (0..self.tree_sel).rev().find(|index| {
            matches!(nodes.get(*index), Some(super::TreeNode::Scope { depth: d, .. }) if *d + 1 == depth)
        }) else {
            return;
        };
        self.remember_source_location();
        self.tree_sel = target;
        self.focus = Focus::Tree;
        self.tree_scroll_to_sel();
        self.sync_source();
    }

    pub fn hierarchy_down(&mut self) {
        let nodes = self.tree_visible();
        let Some(super::TreeNode::Scope { id, depth }) = nodes.get(self.tree_sel).copied() else {
            return;
        };
        let Some(wf) = self.wf.as_ref() else {
            return;
        };
        if wf.tree.nodes[id].children.is_empty() {
            return;
        }
        self.remember_source_location();
        self.expanded.insert(id);
        self.touch_panes();
        let nodes = self.tree_visible();
        let Some(target) = (self.tree_sel + 1..nodes.len()).find(|index| {
            matches!(nodes.get(*index), Some(super::TreeNode::Scope { depth: d, .. }) if *d == depth + 1)
        }) else {
            return;
        };
        self.tree_sel = target;
        self.focus = Focus::Tree;
        self.tree_scroll_to_sel();
        self.sync_source();
    }

    pub fn focus_source_search(&mut self) {
        self.toolbar_search = true;
        self.input.end();
        self.focus = Focus::Source;
    }

    pub fn toggle_search_case_sensitive(&mut self) {
        self.search_case_sensitive = !self.search_case_sensitive;
        self.msg(format!(
            "source search: {}",
            if self.search_case_sensitive {
                "case sensitive"
            } else {
                "ignore case"
            }
        ));
    }

    pub fn toggle_search_regex(&mut self) {
        self.search_regex = !self.search_regex;
        self.msg(format!(
            "source search: regular expressions {}",
            if self.search_regex { "on" } else { "off" }
        ));
    }

    pub fn toolbar_search_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Tab => self.toolbar_search = false,
            KeyCode::Enter => self.search_source(!key.modifiers.contains(KeyModifiers::SHIFT)),
            _ => {
                crate::ui::widgets::TextBox::handle_key(&mut self.input, key);
            }
        }
        false
    }

    pub fn search_source(&mut self, forward: bool) {
        let query = self.input.as_string();
        if query.is_empty() {
            self.msg("source search: enter a query");
            return;
        }
        let Some(result) = self.source_view.as_ref().map(|view| {
            find_in_source(
                view,
                &query,
                forward,
                self.search_case_sensitive,
                self.search_regex,
            )
        }) else {
            self.msg(format!("source search: no match for {query}"));
            return;
        };
        let found = match result {
            Ok(found) => found,
            Err(error) => {
                self.msg(error);
                return;
            }
        };
        let Some((line, start, end)) = found else {
            self.msg(format!("source search: no match for {query}"));
            return;
        };
        self.remember_source_location();
        self.set_source_cursor(line, start);
        if let Some(view) = self.source_view.as_mut() {
            view.anchor = Some((line, start));
            view.sel = Some(((line, start), (line, end)));
        }
        self.focus = Focus::Source;
    }
}

fn find_in_source(
    view: &crate::rtl::SourceView,
    query: &str,
    forward: bool,
    case_sensitive: bool,
    regex_enabled: bool,
) -> Result<Option<(usize, usize, usize)>, String> {
    let pattern = if regex_enabled {
        query.to_string()
    } else {
        regex::escape(query)
    };
    let regex = regex::RegexBuilder::new(&pattern)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|error| format!("source search: invalid regex: {error}"))?;
    if view.lines.is_empty() {
        return Ok(None);
    }
    let count = view.lines.len();
    for step in 0..count {
        let (line, start) = if forward {
            (
                (view.line + step) % count,
                (step == 0).then_some(view.col + 1),
            )
        } else {
            (
                (view.line + count - step) % count,
                (step == 0).then_some(view.col),
            )
        };
        let text = &view.lines[line];
        let boundary = start.unwrap_or_else(|| if forward { 0 } else { text.chars().count() });
        let byte = text
            .char_indices()
            .nth(boundary)
            .map(|(index, _)| index)
            .unwrap_or(text.len());
        let found = if forward {
            regex
                .find_at(text, byte)
                .filter(|found| found.start() >= byte)
        } else {
            regex.find_iter(&text[..byte]).last()
        };
        if let Some(found) = found {
            let start = text[..found.start()].chars().count();
            let end = text[..found.end()].chars().count();
            return Ok(Some((line, start, end)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_wraps_in_both_directions() {
        let mut view = crate::rtl::SourceView {
            module: "search".into(),
            file: PathBuf::new(),
            lines: vec!["alpha".into(), "beta alpha".into(), "gamma".into()],
            max_width: 11,
            line_modules: vec!["search".into(); 3],
            spans: vec![Vec::new(); 3],
            inactive: vec![false; 3],
            scroll: 0,
            line: 0,
            col: 0,
            anchor: None,
            sel: None,
        };
        view.line = 1;
        view.col = 5;
        assert_eq!(
            find_in_source(&view, "alpha", true, false, false),
            Ok(Some((0, 0, 5)))
        );
        assert_eq!(
            find_in_source(&view, "alpha", false, false, false),
            Ok(Some((0, 0, 5)))
        );
        view.line = 0;
        view.col = 0;
        assert_eq!(
            find_in_source(&view, "alpha", true, false, false),
            Ok(Some((1, 5, 10)))
        );
        assert_eq!(
            find_in_source(&view, "alpha", false, false, false),
            Ok(Some((1, 5, 10)))
        );
        assert_eq!(
            find_in_source(&view, "missing", true, false, false),
            Ok(None)
        );
    }

    #[test]
    fn search_supports_case_and_regex_modes() {
        let mut view = crate::rtl::SourceView {
            module: "search".into(),
            file: PathBuf::new(),
            lines: vec!["Alpha".into(), "alpha".into(), "foo123".into()],
            max_width: 6,
            line_modules: vec!["search".into(); 3],
            spans: vec![Vec::new(); 3],
            inactive: vec![false; 3],
            scroll: 0,
            line: 1,
            col: 0,
            anchor: None,
            sel: None,
        };
        assert_eq!(
            find_in_source(&view, "^ALPHA$", true, false, true),
            Ok(Some((0, 0, 5)))
        );
        assert_eq!(find_in_source(&view, "^ALPHA$", true, true, true), Ok(None));
        view.line = 0;
        assert_eq!(
            find_in_source(&view, "foo[0-9]+", true, false, false),
            Ok(None)
        );
        assert_eq!(
            find_in_source(&view, "foo[0-9]+", true, false, true),
            Ok(Some((2, 0, 6)))
        );
        assert!(find_in_source(&view, "[", true, false, true).is_err());
    }

    #[test]
    fn hierarchy_and_source_history_navigate_in_both_directions() {
        use crate::app::tests::app_with;
        use crate::rtl::{RtlDb, SourceSet};

        let vcd = "$timescale 1ns $end\n\
            $scope module top $end\n\
            $scope module u1 $end\n\
            $var wire 1 ! rst_in $end\n\
            $upscope $end\n\
            $var wire 1 \" rst $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let mut app = app_with(vcd);
        let dir = std::env::temp_dir().join(format!("waverdi_history_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let top = dir.join("top.sv");
        let child = dir.join("child.sv");
        std::fs::write(
            &top,
            "module top(input rst);\nchild u1(.rst_in(rst));\nendmodule\n",
        )
        .unwrap();
        std::fs::write(&child, "module child(input rst_in);\nendmodule\n").unwrap();
        app.sources = Some(SourceSet::from_files(vec![top, child], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.tree_sel = 1;
        app.sync_source();

        app.input.set("rst");
        app.search_source(true);
        assert_eq!(app.source_view.as_ref().unwrap().line, 0);
        assert_eq!(app.source_view.as_ref().unwrap().col, 17);
        assert_eq!(
            app.source_view.as_ref().unwrap().sel,
            Some(((0, 17), (0, 20)))
        );
        app.hierarchy_down();
        assert_eq!(app.source_view.as_ref().unwrap().module, "child");
        app.history_back();
        assert_eq!(app.source_view.as_ref().unwrap().module, "top");
        assert_eq!(app.source_view.as_ref().unwrap().col, 17);
        app.history_forward();
        assert_eq!(app.source_view.as_ref().unwrap().module, "child");
        app.hierarchy_up();
        assert_eq!(app.source_view.as_ref().unwrap().module, "top");
        let _ = std::fs::remove_dir_all(dir);
    }
}
