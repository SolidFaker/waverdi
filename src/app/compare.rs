use super::{App, BrowserMode, Dialog, Focus, LoadJob};
use crate::waveform::{
    compare_signals, fmt_value, format_time, Comparison, Signal, Ticks, Waveform,
};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

pub struct CompareRow {
    pub left: usize,
    pub right: Option<usize>,
    pub name: String,
    pub supported: bool,
    pub comparison: Option<Comparison>,
    queued: bool,
}

pub struct CompareSession {
    pub path: String,
    pub job: LoadJob,
    pub wf: Option<Waveform>,
    pub rows: Vec<CompareRow>,
    pub error: Option<String>,
    generation: u64,
    tasks: Sender<CompareTask>,
    results: Receiver<(usize, u64, Comparison)>,
}

struct CompareTask {
    row: usize,
    generation: u64,
    left: Signal,
    right: Signal,
    start: Ticks,
    end: Ticks,
}

impl CompareSession {
    fn new(path: &str) -> Self {
        let (tasks, task_rx) = mpsc::channel::<CompareTask>();
        let (result_tx, results) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(task) = task_rx.recv() {
                let comparison = compare_signals(&task.left, &task.right, task.start, task.end);
                if result_tx
                    .send((task.row, task.generation, comparison))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            path: path.to_string(),
            job: LoadJob::start(path, false),
            wf: None,
            rows: Vec::new(),
            error: None,
            generation: 0,
            tasks,
            results,
        }
    }
}

impl App {
    pub fn open_compare(&mut self) {
        if self.compare.is_some() {
            self.open_dialog(Dialog::Compare);
            return;
        }
        if self.wf.is_none() || self.display.is_empty() {
            self.msg("compare: add signals to the waveform first");
            return;
        }
        self.browser_mode = BrowserMode::Compare;
        if self.use_gui {
            if let Some(path) = crate::picker::pick_vcd() {
                self.start_compare(&path.display().to_string());
            }
            return;
        }
        self.open_tui_browser();
        self.browser_mode = BrowserMode::Compare;
    }

    pub fn start_compare(&mut self, path: &str) {
        if self.wf.is_none() || self.display.is_empty() {
            self.msg("compare: add signals to the waveform first");
            return;
        }
        self.compare = Some(CompareSession::new(path));
        self.compare_sel = 0;
        self.open_dialog(Dialog::Compare);
        self.msg(format!("compare: loading reference {path}"));
    }

    pub fn close_compare(&mut self) {
        self.compare = None;
        self.compare_sel = 0;
        if self.dialog == Some(Dialog::Compare) {
            self.dialog = None;
        }
        self.msg("compare: reference closed");
    }

    pub fn compare_active(&self) -> bool {
        self.compare.as_ref().is_some_and(|session| {
            !session.job.finished
                || session
                    .rows
                    .iter()
                    .any(|row| row.right.is_some() && row.supported && row.comparison.is_none())
        })
    }

    pub fn compare_move(&mut self, delta: i64) {
        let Some(session) = self.compare.as_ref() else {
            return;
        };
        if session.rows.is_empty() {
            return;
        }
        self.compare_sel =
            (self.compare_sel as i64 + delta).clamp(0, session.rows.len() as i64 - 1) as usize;
        let rows = crate::ui::dialog::visible_rows(self.last_area, self, Dialog::Compare).max(1);
        if self.compare_sel < self.dialog_scroll {
            self.dialog_scroll = self.compare_sel;
        } else if self.compare_sel >= self.dialog_scroll + rows {
            self.dialog_scroll = self.compare_sel + 1 - rows;
        }
    }

    pub fn compare_refresh(&mut self) {
        self.rebuild_compare_rows();
        self.compare_sel = 0;
        self.dialog_scroll = 0;
        self.queue_comparisons();
    }

    pub fn compare_activate(&mut self, forward: bool) {
        let Some((left, name, total, difference)) = self.compare.as_ref().and_then(|session| {
            let row = session.rows.get(self.compare_sel)?;
            let comparison = row.comparison.as_ref()?;
            let differences = &comparison.differences;
            let difference = if differences.is_empty() {
                None
            } else {
                Some(if forward {
                    differences
                        .iter()
                        .find(|difference| difference.time > self.cursor)
                        .unwrap_or(&differences[0])
                } else {
                    differences
                        .iter()
                        .rev()
                        .find(|difference| difference.time < self.cursor)
                        .unwrap_or_else(|| differences.last().unwrap())
                })
            };
            Some((
                row.left,
                row.name.clone(),
                comparison.total,
                difference.cloned(),
            ))
        }) else {
            self.msg("compare: values are still loading or the signal is missing");
            return;
        };
        let Some(difference) = difference else {
            self.msg(format!("compare: {name} is identical"));
            return;
        };
        if total > 1 {
            self.msg(format!("compare: {total} difference(s) in {name}"));
        }
        self.cursor = difference.time;
        self.sel_row = self
            .list_rows()
            .iter()
            .position(|row| matches!(row, super::ListRow::Signal { sig, .. } if *sig == left));
        self.focus = Focus::Wave;
        self.dialog = None;
        self.reveal_cursor();
        let radix = self.radix_for(left);
        let value = |value: &Option<crate::waveform::Value>| {
            value
                .as_ref()
                .map(|value| fmt_value(value, radix))
                .unwrap_or_else(|| "x".to_string())
        };
        let ts = self.wf.as_ref().unwrap().ts;
        self.msg(format!(
            "compare {name}: {} != {} at {}",
            value(&difference.left),
            value(&difference.right),
            format_time(difference.time as f64, &ts)
        ));
    }

    pub(crate) fn poll_compare(&mut self) {
        if self.compare.is_none() {
            return;
        }
        loop {
            let event = match self.compare.as_ref().map(|session| session.job.try_recv()) {
                Some(Ok(event)) => event,
                Some(Err(TryRecvError::Empty)) => break,
                Some(Err(TryRecvError::Disconnected)) => {
                    if let Some(session) = self.compare.as_mut() {
                        if !session.job.finished && session.error.is_none() {
                            session.error = Some("reference loader disconnected".to_string());
                            session.job.finished = true;
                        }
                    }
                    break;
                }
                None => break,
            };
            self.needs_redraw = true;
            match event {
                super::LoadEvent::Progress(progress) => {
                    if let Some(session) = self.compare.as_mut() {
                        session.job.progress = progress;
                    }
                }
                super::LoadEvent::Capabilities(_) => {}
                super::LoadEvent::Waveform(out) => {
                    let warnings = out.warnings.clone();
                    if let Some(session) = self.compare.as_mut() {
                        session.wf = Some(out.wf);
                    }
                    self.rebuild_compare_rows();
                    for warning in warnings {
                        self.msg(format!("compare: {warning}"));
                    }
                }
                super::LoadEvent::Rtl(_, _) => {}
                super::LoadEvent::Changes(updates, warnings) => {
                    if let Some(session) = self.compare.as_mut() {
                        if let Some(wf) = session.wf.as_mut() {
                            for (index, changes) in updates {
                                if let Some(signal) = wf.signals.get_mut(index) {
                                    signal.changes = changes;
                                    signal.state = crate::waveform::SigState::Ready;
                                }
                                wf.cache_value_times(index);
                            }
                        }
                    }
                    for warning in warnings {
                        self.msg(format!("compare: {warning}"));
                    }
                }
                super::LoadEvent::Failed(error) => {
                    if let Some(session) = self.compare.as_mut() {
                        session.error = Some(error.clone());
                        session.job.finished = true;
                    }
                    self.msg(format!("compare: {error}"));
                }
                super::LoadEvent::Done => {
                    if let Some(session) = self.compare.as_mut() {
                        session.job.finished = true;
                    }
                }
            }
        }
        self.queue_comparisons();
        loop {
            let result = self
                .compare
                .as_ref()
                .map(|session| session.results.try_recv());
            match result {
                Some(Ok((row, generation, comparison))) => {
                    if let Some(session) = self.compare.as_mut() {
                        if generation == session.generation {
                            if let Some(target) = session.rows.get_mut(row) {
                                target.comparison = Some(comparison);
                                self.needs_redraw = true;
                            }
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn rebuild_compare_rows(&mut self) {
        let Some((primary, reference)) = self.wf.as_ref().zip(
            self.compare
                .as_ref()
                .and_then(|session| session.wf.as_ref()),
        ) else {
            return;
        };
        let by_name: HashMap<(&[String], &str), usize> = reference
            .signals
            .iter()
            .enumerate()
            .map(|(index, signal)| ((signal.scope.as_slice(), signal.name.as_str()), index))
            .collect();
        let mut seen = HashSet::new();
        let rows = self
            .display
            .iter()
            .copied()
            .filter(|index| seen.insert(*index))
            .filter_map(|left| {
                let signal = primary.signals.get(left)?;
                let right = by_name
                    .get(&(signal.scope.as_slice(), signal.name.as_str()))
                    .copied();
                let supported = primary.ts == reference.ts
                    && !primary.is_synthesized(left)
                    && right.is_none_or(|right| !reference.is_synthesized(right));
                Some(CompareRow {
                    left,
                    right,
                    name: signal.full_name(),
                    supported,
                    comparison: None,
                    queued: false,
                })
            })
            .collect();
        if let Some(session) = self.compare.as_mut() {
            session.generation = session.generation.wrapping_add(1);
            session.rows = rows;
        }
    }

    fn queue_comparisons(&mut self) {
        let Some(count) = self.compare.as_ref().map(|session| session.rows.len()) else {
            return;
        };
        for row_index in 0..count {
            let task = (|| {
                let primary = self.wf.as_ref()?;
                let session = self.compare.as_ref()?;
                let reference = session.wf.as_ref()?;
                let row = session.rows.get(row_index)?;
                if row.queued || row.comparison.is_some() || !row.supported {
                    return None;
                }
                let right = row.right?;
                let left_signal = primary.signals.get(row.left)?;
                let right_signal = reference.signals.get(right)?;
                if left_signal.state != crate::waveform::SigState::Ready
                    || right_signal.state != crate::waveform::SigState::Ready
                {
                    return None;
                }
                Some(CompareTask {
                    row: row_index,
                    generation: session.generation,
                    left: left_signal.clone(),
                    right: right_signal.clone(),
                    start: primary.start.min(reference.start),
                    end: primary.end.max(reference.end),
                })
            })();
            if let Some(task) = task {
                let sent = self
                    .compare
                    .as_ref()
                    .is_some_and(|session| session.tasks.send(task).is_ok());
                if sent {
                    if let Some(session) = self.compare.as_mut() {
                        session.rows[row_index].queued = true;
                    }
                }
            }
        }
        let requests: Vec<(usize, Option<usize>)> = self
            .compare
            .as_ref()
            .map(|session| {
                session
                    .rows
                    .iter()
                    .filter(|row| row.supported && !row.queued)
                    .map(|row| (row.left, row.right))
                    .collect()
            })
            .unwrap_or_default();
        for (left, right) in requests {
            self.request_signal(left);
            if let Some(right) = right {
                if let Some(session) = self.compare.as_mut() {
                    let should_request = session.wf.as_ref().is_some_and(|wf| {
                        wf.signals
                            .get(right)
                            .is_some_and(|signal| signal.state == crate::waveform::SigState::Lazy)
                    });
                    if should_request {
                        session.job.request(right);
                        if let Some(wf) = session.wf.as_mut() {
                            wf.signals[right].state = crate::waveform::SigState::Loading;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::tests::app_with;

    const LEFT: &str = "$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n#10\n1!\n#20\n0!\n";
    const RIGHT: &str = "$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n#10\n0!\n#20\n0!\n";

    #[test]
    fn reference_values_are_compared_and_activated() {
        let dir = std::env::temp_dir().join(format!("waverdi_compare_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let reference = dir.join("right.vcd");
        std::fs::write(&reference, RIGHT).unwrap();
        let mut app = app_with(LEFT);
        app.set_display(vec![0]);
        app.start_compare(&reference.display().to_string());
        for _ in 0..500 {
            app.poll_load();
            app.poll_compare();
            if app.compare.as_ref().is_some_and(|session| {
                session
                    .rows
                    .first()
                    .is_some_and(|row| row.comparison.is_some())
            }) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let session = app.compare.as_ref().unwrap();
        let comparison = session.rows[0].comparison.as_ref().unwrap();
        assert_eq!(comparison.total, 1);
        assert_eq!(comparison.differences[0].time, 10);
        crate::app::handle_key(
            &mut app,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
        );
        assert_eq!(app.cursor, 10);
        assert_eq!(app.dialog, None);
        app.close_compare();
        assert!(app.compare.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(fsdb_sdk)]
    fn fsdb_compare_and_export_share_the_lazy_loading_pipeline() {
        let path = std::env::var("WAVERDI_TEST_FSDB").ok().or_else(|| {
            std::env::var_os("VERDI_HOME").map(|home| {
                std::path::PathBuf::from(home)
                    .join("demo/nCompare/nCmp_demo1/demo_RTL_verilog.fsdb")
                    .display()
                    .to_string()
            })
        });
        let Some(path) = path.filter(|path| std::path::Path::new(path).is_file()) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("waverdi_compare_fsdb_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = crate::app::App::new();
        app.start_load(&path);
        for _ in 0..1000 {
            app.poll_load();
            if app.wf.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let index = app
            .wf
            .as_ref()
            .and_then(|wf| {
                wf.signals.iter().enumerate().position(|(index, signal)| {
                    !wf.is_synthesized(index) && signal.members.is_empty()
                })
            })
            .unwrap_or(0);
        app.set_display(vec![index]);
        app.start_compare(&path);
        for _ in 0..1000 {
            app.poll_load();
            app.poll_compare();
            if app.compare.as_ref().is_some_and(|session| {
                session
                    .rows
                    .first()
                    .is_some_and(|row| row.comparison.is_some())
            }) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let comparison = app.compare.as_ref().unwrap().rows[0]
            .comparison
            .as_ref()
            .unwrap();
        assert_eq!(comparison.total, 0);

        let output = dir.join("signals.csv");
        app.input.set(&output.display().to_string());
        app.begin_export();
        for _ in 0..1000 {
            app.poll_load();
            app.poll_compare();
            app.poll_export();
            if !app.export_active() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.export_active(), "{:?}", app.messages);
        let text = std::fs::read_to_string(output).unwrap();
        assert!(text.lines().count() > 1);
        app.close_compare();
        let _ = std::fs::remove_dir_all(dir);
    }
}
