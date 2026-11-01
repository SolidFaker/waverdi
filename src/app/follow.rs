use super::{App, Dialog, Focus};

impl App {
    pub fn open_follow_signal(&mut self) {
        let Some((scope, signal)) = self.follow_origin() else {
            self.msg("follow signal: no signal name under the cursor");
            return;
        };
        let targets = self
            .rtl
            .as_ref()
            .map(|rtl| rtl.follow(&scope, &signal))
            .unwrap_or_default();
        if targets.is_empty() {
            self.msg(format!(
                "follow signal: no connected module found for {signal}"
            ));
            return;
        }
        self.follow_targets = targets;
        self.follow_sel = 0;
        self.follow_name = signal;
        self.open_dialog(Dialog::Follow);
    }

    pub fn follow_move(&mut self, delta: i64) {
        if self.follow_targets.is_empty() {
            return;
        }
        self.follow_sel = (self.follow_sel as i64 + delta)
            .clamp(0, self.follow_targets.len() as i64 - 1) as usize;
        let rows = crate::ui::dialog::visible_rows(self.last_area, self, Dialog::Follow).max(1);
        if self.follow_sel < self.dialog_scroll {
            self.dialog_scroll = self.follow_sel;
        } else if self.follow_sel >= self.dialog_scroll + rows {
            self.dialog_scroll = self.follow_sel + 1 - rows;
        }
    }

    pub fn follow_activate(&mut self) {
        let Some(target) = self.follow_targets.get(self.follow_sel).cloned() else {
            return;
        };
        if !self.switch_scope(&target.scope) {
            self.msg(format!(
                "follow signal: {} is not present in the dump hierarchy",
                target.scope.join(".")
            ));
            return;
        }
        self.set_source_cursor(target.line.saturating_sub(1), 0);
        self.focus = Focus::Source;
        self.dialog = None;
        self.msg(format!(
            "follow {} -> {} ({}:{})",
            target.signal,
            target.scope.join("."),
            target.file.display(),
            target.line
        ));
    }

    fn follow_origin(&self) -> Option<(Vec<String>, String)> {
        let view = self.source_view.as_ref()?;
        let rtl = self.rtl.as_ref()?;
        let (start, _) = view.word_span_at_cursor()?;
        let word = view.word_at_cursor()?;
        let chain = view.qualifier_chain(view.line, start);
        let module = view.module_at_line(view.line).to_string();
        let active = view.module.as_str();
        let (base, bindings) = if module == active {
            match rtl.scope_info(&self.selected_scope_steps()) {
                Some(found) => (found.instance_scope, found.env),
                None => (self.selected_scope_steps(), Default::default()),
            }
        } else {
            (self.foreign_scope(&module)?, Default::default())
        };
        let (path, signal) = rtl.resolve_reference_with(&module, &chain, &word, &bindings)?;
        let mut scope = base;
        scope.extend(path);
        Some((scope, signal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::context::CtxItem;
    use crate::app::tests::app_with;
    use crate::app::CtxTarget;
    use crate::rtl::{RtlDb, SourceSet};

    #[test]
    fn follow_signal_menu_opens_targets_and_locates_module() {
        let vcd = "$timescale 1ns $end\n\
            $scope module top $end\n\
            $scope module u1 $end\n\
            $var wire 1 ! rst_in $end\n\
            $upscope $end\n\
            $var wire 1 \" rst $end\n\
            $upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let mut app = app_with(vcd);
        let dir = std::env::temp_dir().join(format!("waverdi_follow_app_{}", std::process::id()));
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
        app.set_source_cursor(1, 17);
        assert_eq!(
            app.source_view
                .as_ref()
                .unwrap()
                .word_at_cursor()
                .as_deref(),
            Some("rst")
        );

        app.open_context_menu(CtxTarget::Source, 5, 5);
        app.run_ctx_item(CtxItem::FollowSignal);
        assert_eq!(app.dialog, Some(Dialog::Follow));
        assert!(!app.follow_targets.is_empty());
        app.follow_activate();
        assert_eq!(app.dialog, None);
        assert_eq!(app.source_view.as_ref().unwrap().module, "child");
        assert_eq!(app.source_view.as_ref().unwrap().line, 0);
        assert_eq!(app.selected_scope_steps(), vec!["top", "u1"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
