use super::{App, Dialog, Focus};
use crate::rtl::scan::FollowTarget;

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

    pub fn open_definition(&mut self) {
        let Some((scope, signal)) = self.follow_origin() else {
            self.msg("definition: no signal name under the cursor");
            return;
        };
        let Some((module, trace)) = self
            .rtl
            .as_ref()
            .and_then(|rtl| rtl.trace_at_scope(&scope, &signal))
        else {
            return;
        };
        let Some(decl) = trace.decl else {
            self.msg(format!("definition: no declaration found for {signal}"));
            return;
        };
        self.jump_follow_target(FollowTarget {
            scope,
            module,
            signal,
            file: decl.file,
            line: decl.line,
        });
    }

    pub fn open_reference(&mut self, drivers: bool) {
        let Some((scope, signal)) = self.follow_origin() else {
            self.msg("reference: no signal name under the cursor");
            return;
        };
        let Some((module, trace)) = self
            .rtl
            .as_ref()
            .and_then(|rtl| rtl.trace_at_scope(&scope, &signal))
        else {
            return;
        };
        let locations = if drivers { trace.drivers } else { trace.loads };
        if locations.is_empty() {
            self.msg(format!(
                "reference: no {} found for {signal}",
                if drivers { "drivers" } else { "loads" }
            ));
            return;
        }
        self.follow_targets = locations
            .into_iter()
            .map(|location| FollowTarget {
                scope: scope.clone(),
                module: module.clone(),
                signal: signal.clone(),
                file: location.file,
                line: location.line,
            })
            .collect();
        self.follow_sel = 0;
        self.follow_name = format!("{signal} ({})", if drivers { "drivers" } else { "loads" });
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
        self.jump_follow_target(target);
    }

    pub(crate) fn jump_follow_target(&mut self, target: FollowTarget) {
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

    pub(crate) fn follow_origin(&self) -> Option<(Vec<String>, String)> {
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
                None => {
                    let steps = self.selected_scope_steps();
                    let owner = rtl
                        .signal_owner(&steps)
                        .map(|(scope, _)| scope)
                        .unwrap_or(steps);
                    (owner, Default::default())
                }
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
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    fn hierarchical_app() -> (App, std::path::PathBuf) {
        let vcd = "$timescale 1ns $end\n\
            $scope module top $end\n\
            $var wire 1 ! rst $end\n\
            $scope module u1 $end\n\
            $var wire 1 \" rst_in $end\n\
            $scope module u $end\n\
            $var wire 1 # reset $end\n\
            $var wire 1 $ alias $end\n\
            $upscope $end\n$upscope $end\n\
            $scope module u2 $end\n\
            $var wire 1 % rst_in $end\n\
            $scope module u $end\n\
            $var wire 1 & reset $end\n\
            $var wire 1 ' alias $end\n\
            $upscope $end\n$upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let mut app = app_with(vcd);
        let serial = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "waverdi_follow_app_{}_{}",
            std::process::id(),
            serial
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let top = dir.join("top.sv");
        let child = dir.join("child.sv");
        let sub = dir.join("sub.sv");
        std::fs::write(
            &top,
            "module top(input rst);\nchild u1(.rst_in(rst));\nchild u2(.rst_in(rst));\nendmodule\n",
        )
        .unwrap();
        std::fs::write(
            &child,
            "module child(input rst_in);\nsub u(.reset(rst_in));\nendmodule\n",
        )
        .unwrap();
        std::fs::write(
            &sub,
            "module sub(input reset);\nwire alias;\nassign alias = reset;\nendmodule\n",
        )
        .unwrap();
        app.sources = Some(SourceSet::from_files(vec![top, child, sub], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.tree_sel = 1;
        app.sync_source();
        app.set_source_cursor(1, 17);
        (app, dir)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn follow_signal_menu_opens_targets_and_locates_module() {
        let (mut app, dir) = hierarchical_app();
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

    #[test]
    fn definition_calling_and_reference_locate_the_expected_lines() {
        let vcd = "$timescale 1ns $end\n$scope module leaf $end\n\
            $var wire 2 ! state [1:0] $end\n$var wire 1 \" next $end\n\
            $upscope $end\n$enddefinitions $end\n#0\nb0 !\n";
        let mut app = app_with(vcd);
        let dir = std::env::temp_dir().join(format!("waverdi_trace_app_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("leaf.sv");
        std::fs::write(
            &file,
            "module leaf(input logic clk, input logic rst);\n\
             logic [1:0] state;\n\
             logic next;\n\
             always_ff @(posedge clk) begin\n\
             if (rst) state <= 2'b0;\n\
             else state <= state + 1'b1;\n\
             end\n\
             assign next = state[0];\n\
             endmodule\n",
        )
        .unwrap();
        app.sources = Some(SourceSet::from_files(vec![file], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.tree_sel = 1;
        app.sync_source();
        let cursor_to_use = |app: &mut App| {
            app.set_source_cursor(7, 14);
            assert_eq!(
                app.source_view
                    .as_ref()
                    .unwrap()
                    .word_at_cursor()
                    .as_deref(),
                Some("state")
            );
        };
        cursor_to_use(&mut app);
        app.open_definition();
        assert_eq!(app.source_view.as_ref().unwrap().line, 1);
        assert_eq!(app.dialog, None);

        cursor_to_use(&mut app);
        app.open_reference(true);
        assert_eq!(app.dialog, Some(Dialog::Follow));
        assert_eq!(
            app.follow_targets
                .iter()
                .map(|target| target.line)
                .collect::<Vec<_>>(),
            vec![5, 6]
        );
        app.follow_activate();
        assert_eq!(app.source_view.as_ref().unwrap().line, 4);

        cursor_to_use(&mut app);
        app.open_reference(false);
        assert_eq!(app.follow_targets.len(), 1);
        assert_eq!(app.follow_targets[0].line, 8);
        app.follow_activate();
        assert_eq!(app.source_view.as_ref().unwrap().line, 7);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dataflow_keyboard_moves_between_nested_instances() {
        let (mut app, dir) = hierarchical_app();
        app.open_context_menu(CtxTarget::Source, 5, 5);
        app.run_ctx_item(CtxItem::Dataflow);
        assert_eq!(app.dialog, Some(Dialog::Dataflow));
        assert_eq!(app.dataflow_edges.len(), 2);
        crate::app::handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.dataflow_sel, 1);
        crate::app::handle_key(&mut app, key(KeyCode::PageUp));
        assert_eq!(app.dataflow_sel, 0);
        crate::app::handle_key(&mut app, key(KeyCode::PageDown));
        assert_eq!(app.dataflow_sel, 1);
        crate::app::handle_key(&mut app, key(KeyCode::Up));
        crate::app::handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.dialog, None);
        assert_eq!(app.selected_scope_steps(), vec!["top", "u1"]);
        assert_eq!(app.source_view.as_ref().unwrap().module, "child");

        app.set_source_cursor(1, 14);
        app.open_dataflow();
        assert_eq!(app.dataflow_edges.len(), 2);
        crate::app::handle_key(&mut app, key(KeyCode::Down));
        crate::app::handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.selected_scope_steps(), vec!["top", "u1", "u"]);
        assert_eq!(app.source_view.as_ref().unwrap().module, "sub");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dataflow_mouse_scroll_and_click_activate_the_clicked_edge() {
        let (mut app, dir) = hierarchical_app();
        app.open_dataflow();
        crate::app::handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 30,
                row: 10,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(app.dataflow_sel, 1);
        let area = crate::ui::dialog::dialog_rect(app.last_area, &app, Dialog::Dataflow);
        crate::app::handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x + 2,
                row: area.y + 3,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(app.dialog, None);
        assert_eq!(app.selected_scope_steps(), vec!["top", "u2"]);
        assert_eq!(app.source_view.as_ref().unwrap().module, "child");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn struct_member_menu_actions_follow_and_locate() {
        let vcd = "$timescale 1ns $end\n\
            $scope module top $end\n\
            $scope module clk_pc $end\n\
            $var wire 1 ! r $end\n\
            $upscope $end\n\
            $scope module u $end\n\
            $var wire 1 \" reset $end\n\
            $upscope $end\n\
            $scope module mem[0] $end\n\
            $var wire 1 % r $end\n\
            $upscope $end\n$upscope $end\n\
            $enddefinitions $end\n#0\n0!\n";
        let mut app = app_with(vcd);
        let dir = std::env::temp_dir().join(format!("waverdi_struct_app_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("top.sv");
        std::fs::write(
            &file,
            "module top(input clk);\n\
             typedef struct packed { logic r; logic nxt; } pc_struct;\n\
             pc_struct clk_pc;\n\
             logic clk_fetch;\n\
             child u(.reset(clk_pc.r));\n\
             always_ff @(posedge clk) clk_pc.r <= clk_pc.nxt;\n\
             assign clk_fetch = clk_pc.r;\n\
             assign clk_pc.nxt = clk_pc.r + 1'b1;\n\
             pc_struct mem[2];\n\
             logic mem_value;\n\
             assign mem_value = mem[0].r;\n\
             endmodule\n\
             module child(input reset);\n\
             endmodule\n",
        )
        .unwrap();
        app.sources = Some(SourceSet::from_files(vec![file], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.tree_sel = 1;
        app.sync_source();
        app.set_source_cursor(6, 26);
        assert_eq!(
            app.follow_origin(),
            Some((
                vec!["top".to_string(), "clk_pc".to_string()],
                "r".to_string()
            ))
        );

        app.switch_scope(&["top".to_string(), "clk_pc".to_string()]);
        app.set_source_cursor(6, 26);
        app.add_source_word();
        assert_eq!(app.display, vec![0], "{:?}", app.messages);
        app.clear_all();
        app.select_source_word();
        app.add_source_selection();
        assert_eq!(app.display, vec![0], "{:?}", app.messages);

        app.open_follow_signal();
        assert_eq!(app.dialog, Some(Dialog::Follow));
        assert!(app
            .follow_targets
            .iter()
            .any(|target| { target.scope == ["top", "u"] && target.signal == "reset" }));
        app.dialog = None;
        app.open_dataflow();
        assert_eq!(app.dialog, Some(Dialog::Dataflow));
        app.dataflow_sel = app
            .dataflow_edges
            .iter()
            .position(|edge| edge.to.scope == ["top", "u"] && edge.to.signal == "reset")
            .unwrap();
        app.dataflow_activate();
        assert_eq!(app.selected_scope_steps(), vec!["top", "u"]);
        assert_eq!(app.source_view.as_ref().unwrap().module, "child");

        app.switch_scope(&["top".to_string()]);
        app.set_source_cursor(6, 26);
        app.open_definition();
        assert_eq!(app.selected_scope_steps(), vec!["top", "clk_pc"]);
        assert_eq!(app.source_view.as_ref().unwrap().module, "top");
        assert_eq!(app.source_view.as_ref().unwrap().line, 2);

        app.set_source_cursor(6, 26);
        app.open_reference(true);
        assert_eq!(app.follow_targets.len(), 1);
        assert_eq!(app.follow_targets[0].line, 6);
        app.set_display(vec![0]);
        app.sel_row = Some(1);
        let trace = app.source_trace_line().unwrap();
        assert!(trace.contains("decl 3"), "{trace}");
        assert!(trace.contains("drivers [6]"), "{trace}");
        app.jump_to_driver(0);
        assert_eq!(app.selected_scope_steps(), vec!["top", "clk_pc"]);
        assert_eq!(app.source_view.as_ref().unwrap().line, 5);

        app.switch_scope(&["top".to_string(), "mem[0]".to_string()]);
        let (line, col) = {
            let view = app.source_view.as_ref().unwrap();
            let line = view
                .lines
                .iter()
                .position(|line| line.contains("assign mem_value"))
                .unwrap();
            (line, view.lines[line].rfind('r').unwrap())
        };
        app.set_source_cursor(line, col);
        app.add_source_word();
        assert_eq!(app.display.last(), Some(&2), "{:?}", app.messages);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn packed_struct_member_adds_the_dumped_vector() {
        let vcd = "$timescale 1ns $end\n\
            $scope module top $end\n\
            $var wire 32 ! clk_pc [31:0] $end\n\
            $upscope $end\n$enddefinitions $end\n#0\nb0 !\n";
        let mut app = app_with(vcd);
        let dir = std::env::temp_dir().join(format!("waverdi_packed_app_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("top.sv");
        std::fs::write(
            &file,
            "module top;\n\
             typedef struct packed { logic [31:0] r; } pc_struct;\n\
             pc_struct clk_pc;\n\
             logic x;\n\
             assign x = clk_pc.r;\n\
             endmodule\n",
        )
        .unwrap();
        app.sources = Some(SourceSet::from_files(vec![file], "test"));
        app.rtl = Some(RtlDb::parse_sources(app.sources.as_ref().unwrap()));
        app.tree_sel = 1;
        app.sync_source();
        app.set_source_cursor(4, 18);
        app.add_source_word();
        assert_eq!(app.display, vec![0], "{:?}", app.messages);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(fsdb_sdk)]
    fn real_struct_member_adds_to_waveform_from_source() {
        let (Ok(list), Ok(dump)) = (
            std::env::var("WAVERDI_TEST_FILELIST"),
            std::env::var("WAVERDI_TEST_FSDB"),
        ) else {
            return;
        };
        let sources = SourceSet::from_filelist(std::path::Path::new(&list)).unwrap();
        let mut app = App::new();
        app.rtl = Some(RtlDb::parse_sources(&sources));
        app.sources = Some(sources);
        app.sources_explicit = true;
        app.start_load(&dump);
        for _ in 0..2000 {
            app.poll_load();
            if app.wf.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.wf.is_some(), "{:?}", app.messages);
        let scope = vec![
            "dp_tst".to_string(),
            "APP_INST".to_string(),
            "CPU_INST".to_string(),
            "clk_pc".to_string(),
        ];
        assert!(app.switch_scope(&scope));
        let (line, col) = {
            let view = app.source_view.as_ref().expect("CPU source view");
            let (line, text) = view
                .lines
                .iter()
                .enumerate()
                .find(|(_, line)| line.contains("assign clk_fetch.pc = clk_pc.r"))
                .unwrap();
            (line, text.rfind('r').unwrap())
        };
        app.set_source_cursor(line, col);
        app.add_source_word();
        let index = *app.display.last().expect("add struct field");
        let signal = &app.wf.as_ref().unwrap().signals[index];
        assert_eq!(signal.scope, scope);
        assert!(crate::rtl::scan::signal_name_matches(&signal.name, "r"));

        app.clear_all();
        app.select_source_word();
        let (indices, missing, _) = app.source_selection_signals();
        assert!(!indices.is_empty(), "missing={missing:?}");
        app.add_source_selection();
        assert!(!app.display.is_empty(), "{:?}", app.messages);
    }
}
