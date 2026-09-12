use crate::app::App;
use crate::rtl::view::HlKind;
use crate::ui::layout::Layout;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Widget as _};

/// Inner area of the Source frame (inside the border).
pub fn inner_rect(l: &Layout) -> Rect {
    Rect {
        x: l.source.x + 1,
        y: l.source.y + 1,
        width: l.source.width.saturating_sub(2),
        height: l.source.height.saturating_sub(2),
    }
}

/// Code area: the inner rect minus the instance header row.
pub fn code_rect(l: &Layout) -> Rect {
    let inner = inner_rect(l);
    Rect {
        x: inner.x,
        y: inner.y.saturating_add(1),
        width: inner.width,
        height: inner.height.saturating_sub(1),
    }
}

/// Bordered frame of the RTL source pane; the title names the module.
pub fn draw_frame(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    let title = match app.source_view.as_ref().map(|view| view.module.clone()) {
        Some(module) => format!(" Source — {module} "),
        None => " Source ".to_string(),
    };
    Block::bordered()
        .title(title)
        .title_style(Style::new().fg(t.text).add_modifier(Modifier::BOLD))
        .border_style(Style::new().fg(t.panel_border))
        .render(l.source, buf);
}

/// Instance header + highlighted RTL source of its module.
pub fn draw(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    let inner = inner_rect(l);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = inner.width as usize;

    // Header: selected instance, its module and the source file.
    let selected = app.selected_scope_path();
    let module = app.selected_scope_module().unwrap_or_default();
    let file = app.source_view.as_ref().and_then(|view| {
        view.file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    });
    let header = match (&selected, module.is_empty()) {
        (Some(path), false) => match &file {
            Some(file) => format!("instance: {path}   [module {module}]   {file}"),
            None => format!("instance: {path}   [module {module}]"),
        },
        (Some(path), true) => format!("instance: {path}"),
        (None, _) => "RTL source view".to_string(),
    };
    text::put(
        buf,
        inner.x,
        inner.y,
        &text::trunc(&header, width),
        Style::new().fg(t.path),
    );

    let code = code_rect(l);
    let Some(view) = &app.source_view else {
        // No parsed module for the selection: list the sources we know.
        let mut row = code.y;
        match &app.sources {
            None => text::put(
                buf,
                code.x,
                row,
                "no RTL sources: pass -f <filelist> or use File > Load Filelist...",
                Style::new().fg(t.dim),
            ),
            Some(set) => {
                text::put(
                    buf,
                    code.x,
                    row,
                    &text::trunc(
                        &format!("{} source file(s)  —  {}", set.files.len(), set.origin),
                        width,
                    ),
                    Style::new().fg(t.dim),
                );
                row += 1;
                for file in &set.files {
                    if row >= code.bottom() {
                        break;
                    }
                    let name = file
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    text::put(
                        buf,
                        code.x,
                        row,
                        &text::trunc(&name, width),
                        Style::new().fg(t.text),
                    );
                    row += 1;
                }
                text::put(
                    buf,
                    code.x,
                    code.bottom().saturating_sub(1),
                    "select an instance to show its module",
                    Style::new().fg(t.dim),
                );
            }
        }
        return;
    };

    // Code with line numbers, scrolling, selection and a keyboard cursor.
    let digits = view.lines.len().max(1).to_string().len();
    let focused = app.focus == crate::app::Focus::Source;
    for row in 0..code.height as usize {
        let index = view.scroll + row;
        let Some(spans) = view.spans.get(index) else {
            break;
        };
        let y = code.y + row as u16;
        let selected = view.is_line_selected(index);
        let bg = if selected {
            t.row_sel_bg
        } else if focused && index == view.line {
            t.list_header_bg
        } else if index.is_multiple_of(2) {
            t.row_alt
        } else {
            t.bg
        };
        let word_range = view
            .word
            .filter(|(line, _, _)| *line == index)
            .map(|(_, start, end)| (start, end));
        let number = format!("{:>digits$} ", index + 1);
        text::put(buf, code.x, y, &number, Style::new().fg(t.dim).bg(bg));

        let mut x = code.x + digits as u16 + 1;
        let mut col = 0usize;
        'line: for span in spans {
            let fg = match span.kind {
                HlKind::Plain => t.text,
                HlKind::Keyword => t.src_keyword,
                HlKind::Number => t.src_number,
                HlKind::String => t.src_string,
                HlKind::Comment => t.src_comment,
                HlKind::Directive => t.src_directive,
                HlKind::Signal => t.src_signal,
            };
            for ch in span.text.chars() {
                if x >= code.right() {
                    break 'line;
                }
                let (fg, bg) = if focused && index == view.line && col == view.col {
                    (t.bg, t.cursor)
                } else if word_range
                    .map(|(start, end)| col >= start && col < end)
                    .unwrap_or(false)
                {
                    (t.bg, t.accent)
                } else {
                    (fg, bg)
                };
                text::set_cell(buf, x, y, &ch.to_string(), fg, bg);
                x += 1;
                col += 1;
            }
        }
        if focused && index == view.line && view.col >= col && x < code.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_bg(t.cursor);
            }
        }
    }

    // Trace of the selected signal on the last row (declaration/driver/load).
    if let Some(sig) = app.selected_signal() {
        let name = app
            .wf
            .as_ref()
            .map(|wf| wf.signals[sig].name.clone())
            .unwrap_or_default();
        if let Some(trace) = app
            .rtl
            .as_ref()
            .and_then(|db| db.trace(&view.module, &name))
        {
            let lines = |locations: &[crate::rtl::scan::Location]| {
                locations
                    .iter()
                    .map(|loc| loc.line.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let always = app
                .rtl
                .as_ref()
                .and_then(|db| db.module(&view.module))
                .and_then(|def| {
                    trace.drivers.iter().find_map(|loc| {
                        def.always
                            .iter()
                            .find(|block| block.start <= loc.line && loc.line <= block.end)
                    })
                })
                .map(|block| format!("  always {}-{}", block.start, block.end))
                .unwrap_or_default();
            let label = format!(
                "{name}: decl {}  drivers [{}]  loads [{}]{always}",
                trace.decl.as_ref().map(|loc| loc.line).unwrap_or(0),
                lines(&trace.drivers),
                lines(&trace.loads)
            );
            text::put(
                buf,
                code.x,
                code.bottom().saturating_sub(1),
                &text::trunc(&label, width),
                Style::new().fg(t.cursor),
            );
        }
    }
}
