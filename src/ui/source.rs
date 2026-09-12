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

/// Code area: the whole inner rect (no header row).
pub fn code_rect(l: &Layout) -> Rect {
    inner_rect(l)
}

/// Column of the source scrollbar, when the file is longer than the pane.
pub fn scrollbar_col(l: &Layout, view: &crate::rtl::SourceView) -> Option<u16> {
    let code = code_rect(l);
    (code.height > 0 && view.lines.len() > code.height as usize)
        .then(|| code.right().saturating_sub(1))
}

/// Width of the line-number gutter (digits plus one space).
pub fn gutter_width(view: &crate::rtl::SourceView) -> u16 {
    view.lines.len().max(1).to_string().len() as u16 + 1
}

/// Bordered frame of the RTL source pane; the title names the instance and
/// file, e.g. `Source - tb.u_proc.u_cluster(/path/cluster.sv)`.
pub fn draw_frame(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    let title = match app.source_title() {
        Some(title) => format!(" {title} "),
        None => " Source ".to_string(),
    };
    Block::bordered()
        .title(title)
        .title_style(Style::new().fg(t.text).add_modifier(Modifier::BOLD))
        .border_style(Style::new().fg(t.panel_border))
        .render(l.source, buf);
}

/// Highlighted RTL source of the selected instance's module.
pub fn draw(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    let inner = inner_rect(l);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = inner.width as usize;

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
    let scrollbar = scrollbar_col(l, view);
    // Keep the scrollbar column free of code.
    let text_right = scrollbar
        .map(|_| code.right().saturating_sub(1))
        .unwrap_or(code.right());
    for row in 0..code.height as usize {
        let index = view.scroll + row;
        let Some(spans) = view.spans.get(index) else {
            break;
        };
        let y = code.y + row as u16;
        let interval = view.selection_interval(index);
        let bg = if focused && index == view.line {
            t.list_header_bg
        } else if index.is_multiple_of(2) {
            t.row_alt
        } else {
            t.bg
        };
        let number = format!("{:>digits$} ", index + 1);
        text::put(buf, code.x, y, &number, Style::new().fg(t.dim).bg(bg));

        let mut x = code.x + digits as u16 + 1;
        let mut col = 0usize;
        let active = view.module_at_line(index) == view.module;
        'line: for span in spans {
            // Code of other modules in the same file is shown dimmed.
            let fg = if !active {
                t.dim
            } else {
                match span.kind {
                    HlKind::Plain => t.text,
                    HlKind::Keyword => t.src_keyword,
                    HlKind::Number => t.src_number,
                    HlKind::String => t.src_string,
                    HlKind::Comment => t.src_comment,
                    HlKind::Directive => t.src_directive,
                    HlKind::Signal => t.src_signal,
                }
            };
            for ch in span.text.chars() {
                if x >= text_right {
                    break 'line;
                }
                let in_sel = interval
                    .map(|(from, to)| col >= from && col < to)
                    .unwrap_or(false);
                let (fg, bg) = if focused && index == view.line && col == view.col {
                    (t.bg, t.cursor)
                } else if in_sel && span.kind == HlKind::Signal {
                    // Selected signal names stand out.
                    (t.bg, t.src_signal)
                } else if in_sel {
                    (fg, t.row_sel_bg)
                } else {
                    (fg, bg)
                };
                text::set_cell(buf, x, y, &ch.to_string(), fg, bg);
                x += 1;
                col += 1;
            }
        }
        if focused && index == view.line && view.col >= col && x < text_right {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_bg(t.cursor);
            }
        }
    }

    if let Some(col) = scrollbar {
        draw_scrollbar(buf, l, view, col, t);
    }
}

/// Vertical scrollbar on the right edge of the code area.
fn draw_scrollbar(
    buf: &mut Buffer,
    l: &Layout,
    view: &crate::rtl::SourceView,
    col: u16,
    t: &crate::theme::Theme,
) {
    let code = code_rect(l);
    let height = code.height as usize;
    if height == 0 {
        return;
    }
    let total = view.lines.len();
    let thumb = ((height as f64 / total as f64) * height as f64).max(1.0) as usize;
    let top = ((view.scroll as f64 / total as f64) * height as f64) as usize;
    for row in 0..height {
        let symbol = if row >= top && row < top + thumb {
            "█"
        } else {
            "│"
        };
        text::set_cell(buf, col, code.y + row as u16, symbol, t.dim, t.bg);
    }
}
