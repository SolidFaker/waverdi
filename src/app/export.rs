use super::{App, Dialog};
use crate::waveform::{export_indices, export_waveform, ExportFormat, ExportReport, Waveform};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(crate) struct PendingExport {
    path: PathBuf,
    format: ExportFormat,
    indices: Vec<usize>,
}

pub(crate) struct ExportJob {
    path: PathBuf,
    rx: Receiver<Result<ExportReport, String>>,
}

impl App {
    pub fn open_export(&mut self) {
        if self.wf.is_none() || self.display.is_empty() {
            self.msg("export: add signals to the waveform first");
            return;
        }
        if self.export_active() {
            self.msg("export: an export is already running");
            return;
        }
        let path = if self.path.is_empty() {
            PathBuf::from(format!("waveform.{}", self.export_format.extension()))
        } else {
            PathBuf::from(&self.path).with_extension(self.export_format.extension())
        };
        self.input.set(&path.display().to_string());
        self.open_dialog(Dialog::Export);
    }

    pub fn cycle_export_format(&mut self) {
        self.export_format = self.export_format.next();
        let path = PathBuf::from(self.input.as_string());
        self.input.set(
            &path
                .with_extension(self.export_format.extension())
                .display()
                .to_string(),
        );
    }

    pub fn begin_export(&mut self) {
        if self.export_active() {
            self.msg("export: an export is already running");
            return;
        }
        let text = self.input.as_string();
        let path = PathBuf::from(text.trim());
        if path.as_os_str().is_empty() {
            self.msg("export: enter an output path");
            return;
        }
        if path.is_dir() {
            self.msg(format!("export: {} is a directory", path.display()));
            return;
        }
        if self.same_waveform_path(&path) {
            self.msg("export: refusing to overwrite the open waveform");
            return;
        }
        let Some(wf) = self.wf.as_ref() else {
            return;
        };
        let indices = export_indices(wf, self.export_format, &self.display);
        if indices.is_empty() {
            self.msg("export: no signals selected");
            return;
        }
        self.pending_export = Some(PendingExport {
            path: path.clone(),
            format: self.export_format,
            indices: indices.clone(),
        });
        self.dialog = None;
        self.msg(format!(
            "export: loading {} signal(s) for {}",
            indices.len(),
            path.display()
        ));
        for index in indices {
            self.request_signal(index);
        }
    }

    pub fn export_active(&self) -> bool {
        self.pending_export.is_some() || self.export_job.is_some()
    }

    pub fn export_status(&self) -> String {
        if let Some(job) = self.export_job.as_ref() {
            return format!("writing {}", job.path.display());
        }
        if let Some(pending) = self.pending_export.as_ref() {
            return format!("loading values for {}", pending.path.display());
        }
        self.export_format.label().to_string()
    }

    pub(crate) fn poll_export(&mut self) {
        let result = self.export_job.as_ref().map(|job| job.rx.try_recv());
        match result {
            Some(Ok(Ok(report))) => {
                let path = self
                    .export_job
                    .as_ref()
                    .map(|job| job.path.clone())
                    .unwrap_or_default();
                self.export_job = None;
                self.msg(format!(
                    "export: {} signal(s), {} values written to {}",
                    report.signals,
                    report.values,
                    path.display()
                ));
                self.needs_redraw = true;
            }
            Some(Ok(Err(error))) => {
                self.export_job = None;
                self.msg(format!("export: {error}"));
                self.needs_redraw = true;
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.export_job = None;
                self.msg("export: writer stopped unexpectedly");
                self.needs_redraw = true;
            }
            _ => {}
        }
        let Some(pending) = self.pending_export.as_ref() else {
            return;
        };
        let ready = self.wf.as_ref().is_some_and(|wf| {
            pending
                .indices
                .iter()
                .all(|&index| wf.signals[index].state == crate::waveform::SigState::Ready)
        });
        if !ready {
            let requests = self
                .pending_export
                .as_ref()
                .map(|pending| pending.indices.clone())
                .unwrap_or_default();
            for index in requests {
                self.request_signal(index);
            }
            return;
        }
        let Some(pending) = self.pending_export.take() else {
            return;
        };
        let Some(wf): Option<Waveform> = self.wf.clone() else {
            return;
        };
        let path = pending.path.clone();
        let (tx, rx) = mpsc::channel();
        let worker_path = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(export_waveform(
                &worker_path,
                pending.format,
                &wf,
                &pending.indices,
            ));
        });
        self.export_job = Some(ExportJob { path, rx });
        self.needs_redraw = true;
    }

    fn same_waveform_path(&self, output: &Path) -> bool {
        if self.path.is_empty() {
            return false;
        }
        let input = Path::new(&self.path);
        if output == input {
            return true;
        }
        output.exists()
            && input.exists()
            && std::fs::canonicalize(output).ok() == std::fs::canonicalize(input).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VCD: &str =
        "$timescale 1ns $end\n$var wire 1 ! clk $end\n$enddefinitions $end\n#0\n0!\n#10\n1!\n";

    #[test]
    fn export_waits_for_values_and_writes_in_the_background() {
        let dir = std::env::temp_dir().join(format!("waverdi_export_app_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("input.vcd");
        let output = dir.join("signals.csv");
        std::fs::write(&input, VCD).unwrap();
        let mut app = App::new();
        app.start_load(&input.display().to_string());
        for _ in 0..500 {
            app.poll_load();
            if app.wf.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.set_display(vec![0]);
        app.sync_layout(ratatui::layout::Rect::new(0, 0, 100, 40));
        crate::app::handle_key(
            &mut app,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::F(7),
                crossterm::event::KeyModifiers::NONE,
            ),
        );
        assert_eq!(app.dialog, Some(Dialog::Export));
        for _ in 0..2 {
            crate::app::handle_key(
                &mut app,
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Tab,
                    crossterm::event::KeyModifiers::NONE,
                ),
            );
        }
        assert_eq!(app.export_format, ExportFormat::Csv);
        app.input.set(&output.display().to_string());
        let area = crate::ui::dialog::dialog_rect(app.last_area, &app, Dialog::Export);
        crate::app::handle_mouse(
            &mut app,
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: area.right() - 17,
                row: area.bottom() - 2,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
        );
        assert!(app.pending_export.is_some() || app.export_job.is_some());
        for _ in 0..500 {
            app.poll_load();
            app.poll_export();
            if !app.export_active() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.export_active(), "{:?}", app.messages);
        let text = std::fs::read_to_string(output).unwrap();
        assert!(text.contains("0,1ns,clk,b0"));
        assert!(text.contains("10,1ns,clk,b1"));

        app.input.set(&input.display().to_string());
        app.begin_export();
        assert!(app.pending_export.is_none());
        assert!(app.messages.last().unwrap().contains("refusing"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
