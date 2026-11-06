use super::{App, Dialog, InputState};
use crate::waveform::logic::{evaluate_logic, parse_logic, quote_signal, LogicProgram};
use crate::waveform::{Change, SigKind, SigState, Signal};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicFocus {
    Name,
    Expression,
    Signals,
    Create,
}

pub struct LogicBuilder {
    pub name: InputState,
    pub expression: InputState,
    pub signals: Vec<usize>,
    pub sel: usize,
    pub focus: LogicFocus,
    pub error: Option<String>,
    pub creating: bool,
    program: Option<LogicProgram>,
}

pub(crate) struct LogicResult {
    scope: Vec<String>,
    name: String,
    changes: Arc<Vec<Change>>,
}

impl App {
    pub fn open_logic_operation(&mut self, idx: usize) {
        let signals = self.action_targets(idx);
        let Some(wf) = self.wf.as_ref() else { return };
        if signals.is_empty() {
            return;
        }
        let scope = wf.signals[signals[0]].scope.clone();
        let mut number = 1usize;
        let name = loop {
            let name = format!("logical_expression_{number}");
            if !wf
                .signals
                .iter()
                .any(|signal| signal.scope == scope && signal.name == name)
            {
                break name;
            }
            number += 1;
        };
        let expression = quote_signal(&wf.signals[signals[0]].full_name());
        self.logic_builder = Some(LogicBuilder {
            name: InputState::from_text(&name),
            expression: InputState::from_text(&expression),
            signals,
            sel: 0,
            focus: LogicFocus::Expression,
            error: None,
            creating: false,
            program: None,
        });
        self.open_dialog(Dialog::LogicOperation);
    }

    pub fn logic_builder(&self) -> Option<&LogicBuilder> {
        self.logic_builder.as_ref()
    }

    pub fn logic_focus(&mut self, focus: LogicFocus) {
        if let Some(builder) = self.logic_builder.as_mut() {
            builder.focus = focus;
        }
    }

    pub fn logic_cycle_focus(&mut self, backwards: bool) {
        let Some(builder) = self.logic_builder.as_mut() else {
            return;
        };
        let next = match (builder.focus, backwards) {
            (LogicFocus::Name, false) => LogicFocus::Expression,
            (LogicFocus::Expression, false) => LogicFocus::Signals,
            (LogicFocus::Signals, false) => LogicFocus::Create,
            (LogicFocus::Create, false) => LogicFocus::Name,
            (LogicFocus::Name, true) => LogicFocus::Create,
            (LogicFocus::Expression, true) => LogicFocus::Name,
            (LogicFocus::Signals, true) => LogicFocus::Expression,
            (LogicFocus::Create, true) => LogicFocus::Signals,
        };
        builder.focus = next;
    }

    pub fn logic_move(&mut self, delta: i64) {
        let rows =
            crate::ui::dialog::visible_rows(self.last_area, self, Dialog::LogicOperation).max(1);
        let Some(builder) = self.logic_builder.as_mut() else {
            return;
        };
        if builder.signals.is_empty() {
            return;
        }
        builder.sel =
            (builder.sel as i64 + delta).clamp(0, builder.signals.len() as i64 - 1) as usize;
        if builder.sel < self.dialog_scroll {
            self.dialog_scroll = builder.sel;
        } else if builder.sel >= self.dialog_scroll + rows {
            self.dialog_scroll = builder.sel + 1 - rows;
        }
    }

    pub fn logic_insert_selected(&mut self) {
        let Some(builder) = self.logic_builder.as_mut() else {
            return;
        };
        let Some(&index) = builder.signals.get(builder.sel) else {
            return;
        };
        let Some(signal) = self.wf.as_ref().and_then(|wf| wf.signals.get(index)) else {
            return;
        };
        let text = quote_signal(&signal.full_name());
        builder.focus = LogicFocus::Expression;
        insert_text(&mut builder.expression, &text);
    }

    pub fn logic_insert_operator(&mut self, text: &str) {
        if let Some(builder) = self.logic_builder.as_mut() {
            builder.focus = LogicFocus::Expression;
            insert_text(&mut builder.expression, text);
        }
    }

    pub fn logic_clear(&mut self) {
        if let Some(builder) = self.logic_builder.as_mut() {
            builder.expression.clear();
            builder.focus = LogicFocus::Expression;
            builder.error = None;
        }
    }

    pub fn logic_active(&self) -> bool {
        self.logic_job.is_some()
            || self
                .logic_builder
                .as_ref()
                .is_some_and(|builder| builder.creating)
    }

    pub fn logic_cancel(&mut self) {
        self.logic_builder = None;
        self.logic_job = None;
        self.dialog = None;
    }

    pub fn logic_operation_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Enter {
            self.begin_logic_operation();
            return;
        }
        let Some(builder) = self.logic_builder.as_mut() else {
            return;
        };
        if builder.creating {
            if key.code == KeyCode::Esc {
                self.logic_cancel();
            }
            return;
        }
        match key.code {
            KeyCode::Esc => self.logic_cancel(),
            KeyCode::Tab => self.logic_cycle_focus(false),
            KeyCode::BackTab => self.logic_cycle_focus(true),
            KeyCode::Enter if builder.focus == LogicFocus::Name => {
                builder.focus = LogicFocus::Expression;
            }
            KeyCode::Enter if builder.focus == LogicFocus::Expression => {
                self.begin_logic_operation();
            }
            KeyCode::Enter if builder.focus == LogicFocus::Signals => self.logic_insert_selected(),
            KeyCode::Enter => self.begin_logic_operation(),
            KeyCode::Up if builder.focus == LogicFocus::Signals => self.logic_move(-1),
            KeyCode::Down if builder.focus == LogicFocus::Signals => self.logic_move(1),
            KeyCode::Home if builder.focus == LogicFocus::Signals => {
                builder.sel = 0;
                self.logic_move(0);
            }
            KeyCode::End if builder.focus == LogicFocus::Signals => {
                let last = builder.signals.len().saturating_sub(1);
                builder.sel = last;
                self.logic_move(0);
            }
            _ => match builder.focus {
                LogicFocus::Name => {
                    crate::ui::widgets::TextBox::handle_key(&mut builder.name, key);
                }
                LogicFocus::Expression => {
                    crate::ui::widgets::TextBox::handle_key(&mut builder.expression, key);
                }
                LogicFocus::Signals | LogicFocus::Create => {}
            },
        }
    }

    pub fn begin_logic_operation(&mut self) {
        let Some(builder) = self.logic_builder.as_mut() else {
            return;
        };
        if builder.creating {
            return;
        }
        let name = builder.name.as_string().trim().to_string();
        if !valid_signal_name(&name) {
            builder.error = Some("name must be a simple Verilog identifier".to_string());
            return;
        }
        let Some(wf) = self.wf.as_ref() else { return };
        let scope = wf.signals[builder.signals[0]].scope.clone();
        if wf
            .signals
            .iter()
            .any(|signal| signal.scope == scope && signal.name == name)
        {
            builder.error = Some(format!("signal already exists: {name}"));
            return;
        }
        let expression = builder.expression.as_string();
        let program = match parse_logic(&expression, wf) {
            Ok(program) => program,
            Err(error) => {
                builder.error = Some(error);
                return;
            }
        };
        for &index in &program.signals {
            self.request_signal(index);
        }
        if let Some(builder) = self.logic_builder.as_mut() {
            builder.program = Some(program);
            builder.creating = true;
            builder.error = Some("loading values and evaluating...".to_string());
        }
        self.poll_logic_operation();
    }

    pub(crate) fn poll_logic_operation(&mut self) {
        let result = self.logic_job.as_ref().map(|job| job.try_recv());
        match result {
            Some(Ok(Ok(result))) => {
                self.logic_job = None;
                self.finish_logic_operation(result);
                return;
            }
            Some(Ok(Err(error))) => {
                self.logic_job = None;
                if let Some(builder) = self.logic_builder.as_mut() {
                    builder.creating = false;
                    builder.error = Some(error);
                }
                self.needs_redraw = true;
                return;
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.logic_job = None;
                if let Some(builder) = self.logic_builder.as_mut() {
                    builder.creating = false;
                    builder.error = Some("logic evaluator stopped unexpectedly".to_string());
                }
                self.needs_redraw = true;
                return;
            }
            _ => {}
        }
        let Some(builder) = self.logic_builder.as_ref() else {
            return;
        };
        if !builder.creating || self.logic_job.is_some() {
            return;
        }
        let Some(program) = builder.program.clone() else {
            return;
        };
        let ready = program.signals.iter().all(|&index| {
            self.wf.as_ref().is_some_and(|wf| {
                wf.signals
                    .get(index)
                    .is_some_and(|signal| signal.state == SigState::Ready)
            })
        });
        if !ready {
            return;
        }
        let Some(wf) = self.wf.clone() else { return };
        let result = LogicResult {
            scope: wf.signals[builder.signals[0]].scope.clone(),
            name: builder.name.as_string().trim().to_string(),
            changes: Arc::new(Vec::new()),
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = evaluate_logic(&program, &wf, wf.start, wf.end)
                .map(|changes| LogicResult { changes, ..result });
            let _ = tx.send(result);
        });
        self.logic_job = Some(rx);
        self.needs_redraw = true;
    }

    fn finish_logic_operation(&mut self, result: LogicResult) {
        let Some(wf) = self.wf.as_mut() else { return };
        let index = wf.signals.len();
        wf.signals.push(Signal {
            name: result.name.clone(),
            bits: result
                .changes
                .iter()
                .map(|change| change.v.bits_len())
                .max()
                .unwrap_or(1) as u32,
            var_type: "logic".to_string(),
            dir: String::new(),
            scope: result.scope,
            kind: SigKind::Bits,
            changes: result.changes,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            parent: None,
            members: Vec::new(),
            state: SigState::Ready,
        });
        self.touch_waveform();
        self.add_signal(index);
        self.logic_builder = None;
        self.logic_job = None;
        self.dialog = None;
        self.msg(format!("added logical expression {}", result.name));
        self.needs_redraw = true;
    }
}

fn valid_signal_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || matches!(first, '_' | '$'))
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '$'))
}

fn insert_text(input: &mut InputState, text: &str) {
    for c in text.chars() {
        input.insert(c);
    }
}

trait FromText {
    fn from_text(text: &str) -> InputState;
}

impl FromText for InputState {
    fn from_text(text: &str) -> InputState {
        let mut input = InputState::default();
        input.set(text);
        input
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::app_with;
    use crate::app::CtxTarget;

    const VCD: &str = "$timescale 1ns $end\n\
        $scope module top $end\n\
        $var wire 1 ! a $end\n\
        $var wire 1 \" b $end\n\
        $upscope $end\n$enddefinitions $end\n\
        #0\n0!\n0\"\n#10\n1!\n#20\n0\"\n";

    #[test]
    fn multi_selection_builds_a_logical_signal_in_the_background() {
        let mut app = app_with(VCD);
        app.set_display(vec![0, 1]);
        app.selection = vec![0, 1];
        app.open_context_menu(CtxTarget::Signal(0), 5, 5);
        app.run_ctx_item(crate::app::context::CtxItem::LogicalOperation);
        assert_eq!(app.dialog, Some(Dialog::LogicOperation));
        assert_eq!(app.logic_builder().unwrap().signals, vec![0, 1]);
        app.logic_builder
            .as_mut()
            .unwrap()
            .expression
            .set("{top.a, top.b}");
        app.begin_logic_operation();
        for _ in 0..100 {
            app.poll_logic_operation();
            if app.logic_builder.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(app.logic_builder.is_none());
        let index = *app.display.last().unwrap();
        let signal = &app.wf.as_ref().unwrap().signals[index];
        assert_eq!(signal.name, "logical_expression_1");
        assert_eq!(
            signal.value_at(0),
            Some(&crate::waveform::Value::compact(vec![0, 0]))
        );
        assert_eq!(
            signal.value_at(10),
            Some(&crate::waveform::Value::compact(vec![0, 1]))
        );
        assert_eq!(
            signal.value_at(20),
            Some(&crate::waveform::Value::compact(vec![0, 1]))
        );
        assert_eq!(app.dialog, None);
    }

    #[test]
    fn logic_dialog_supports_keyboard_operator_insert_and_cancel() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        app.open_logic_operation(0);
        app.logic_clear();
        app.logic_insert_operator("!");
        assert!(app
            .logic_builder()
            .unwrap()
            .expression
            .as_string()
            .ends_with('!'));
        app.logic_operation_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.logic_builder().unwrap().focus, LogicFocus::Signals);
        app.logic_operation_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.logic_builder().unwrap().expression.as_string(),
            "!\"top.a\""
        );
        let area = crate::ui::dialog::dialog_rect(app.last_area, &app, Dialog::LogicOperation);
        assert_eq!(
            crate::ui::dialog::logic_hit(app.last_area, &app, area.x + 2, area.y + 5),
            Some(crate::ui::dialog::LogicHit::Operator("~"))
        );
        crate::app::handle_mouse(
            &mut app,
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: area.x + 2,
                row: area.y + 5,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert!(app
            .logic_builder()
            .unwrap()
            .expression
            .as_string()
            .ends_with('~'));
        app.logic_operation_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.logic_builder.is_none());
        assert_eq!(app.dialog, None);
    }

    #[test]
    fn logic_operation_waits_for_lazy_values() {
        let dir = std::env::temp_dir().join(format!("waverdi_logic_lazy_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wave.vcd");
        std::fs::write(&path, VCD).unwrap();
        let mut app = App::new();
        app.start_load(&path.display().to_string());
        for _ in 0..500 {
            app.poll_load();
            if app.wf.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        app.set_display(vec![0, 1]);
        app.selection = vec![0, 1];
        app.open_logic_operation(0);
        app.logic_builder
            .as_mut()
            .unwrap()
            .expression
            .set("{top.a, top.b}");
        app.begin_logic_operation();
        for _ in 0..500 {
            app.poll_load();
            app.poll_logic_operation();
            if app.logic_builder.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(app.logic_builder.is_none(), "{:?}", app.messages);
        let signal = app.wf.as_ref().unwrap().signals.last().unwrap();
        assert_eq!(
            signal.value_at(10),
            Some(&crate::waveform::Value::compact(vec![0, 1]))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(fsdb_sdk)]
    fn real_fsdb_signals_work_in_logical_expressions() {
        let Ok(path) = std::env::var("WAVERDI_TEST_FSDB") else {
            return;
        };
        let mut app = App::new();
        app.start_load(&path);
        for _ in 0..2000 {
            app.poll_load();
            if app.wf.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let names = [
            "dp_tst.APP_INST.CPU_INST.clk_pc.r",
            "dp_tst.APP_INST.CPU_INST.clk_fetch.run",
        ];
        let indexes = names.map(|name| {
            let (scope_text, leaf) = name.rsplit_once('.').unwrap();
            let scope: Vec<String> = scope_text.split('.').map(str::to_string).collect();
            app.wf
                .as_ref()
                .unwrap()
                .signals
                .iter()
                .position(|signal| {
                    signal.scope == scope
                        && crate::rtl::scan::signal_name_matches(&signal.name, leaf)
                })
                .unwrap()
        });
        app.set_display(indexes.to_vec());
        app.selection = indexes.to_vec();
        app.open_logic_operation(indexes[0]);
        app.logic_builder.as_mut().unwrap().expression.set(&format!(
            "{{{}, {}}}",
            quote_signal(names[0]),
            quote_signal(names[1])
        ));
        app.begin_logic_operation();
        for _ in 0..2000 {
            app.poll_load();
            app.poll_logic_operation();
            if app.logic_builder.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.logic_builder.is_none(), "{:?}", app.messages);
        let created = app.wf.as_ref().unwrap().signals.last().unwrap();
        let expected_width = indexes
            .iter()
            .map(|&index| app.wf.as_ref().unwrap().signals[index].bits)
            .sum::<u32>();
        assert_eq!(created.bits, expected_width);
        assert!(!created.changes.is_empty());
    }

    #[test]
    fn add_signals_roundtrip_preserves_the_logical_builder() {
        let mut app = app_with(VCD);
        app.set_display(vec![0]);
        app.open_logic_operation(0);
        app.open_logic_add_signals();
        assert_eq!(app.dialog, Some(Dialog::AddSignals));
        assert_eq!(app.add_purpose, crate::app::AddPurpose::LogicOperation);
        let top = app.wf.as_ref().unwrap().tree.nodes[0].children[0];
        app.add_open_scope(top);
        app.add_toggle_signal_at(1);
        app.add_action(crate::app::AddAction::Apply { close: true });
        assert_eq!(app.dialog, Some(Dialog::LogicOperation));
        assert_eq!(app.logic_builder().unwrap().signals, vec![0, 1]);
        assert_eq!(app.display, vec![0], "picker must not add to waveform");

        app.open_logic_add_signals();
        app.add_toggle_signal_at(1);
        app.add_action(crate::app::AddAction::Close);
        assert_eq!(app.dialog, Some(Dialog::LogicOperation));
        assert_eq!(app.logic_builder().unwrap().signals, vec![0, 1]);
    }
}
