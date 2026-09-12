use crate::app::App;
use crate::theme::Theme;
use crate::ui::layout::Layout;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Widget as _};

/// Bordered frame of the RTL source pane.
pub fn draw_frame(buf: &mut Buffer, l: &Layout, t: &Theme) {
    Block::bordered()
        .title(" Source ")
        .title_style(Style::new().fg(t.text).add_modifier(Modifier::BOLD))
        .border_style(Style::new().fg(t.panel_border))
        .render(l.source, buf);
}

/// Placeholder body: shows the selected instance until a source loader exists.
pub fn draw(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    let inner = ratatui::layout::Rect {
        x: l.source.x + 2,
        y: l.source.y + 1,
        width: l.source.width.saturating_sub(4),
        height: l.source.height.saturating_sub(2),
    };
    text::put(
        buf,
        inner.x,
        inner.y,
        "RTL source view",
        Style::new().fg(t.dim).add_modifier(Modifier::BOLD),
    );
    let width = inner.width as usize;
    let mut row = inner.y + 1;
    let mut module_shown = false;
    let selected = app.selected_scope_path().or_else(|| {
        app.wf
            .as_ref()
            .map(|wf| wf.tree.nodes[wf.tree.root].name.clone())
    });
    if let Some(path) = selected {
        let module = app.selected_scope_module().unwrap_or_default();
        let label = if module.is_empty() || module == path {
            format!("instance: {path}")
        } else {
            format!("instance: {path}   [module {module}]")
        };
        text::put(buf, inner.x, row, &label, Style::new().fg(t.path));
        row += 1;

        // Module summary + declarations + the selected signal's trace.
        if !module.is_empty() {
            if let Some(def) = app.rtl.as_ref().and_then(|db| db.module(&module)) {
                module_shown = true;
                let file = def
                    .file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                text::put(
                    buf,
                    inner.x,
                    row,
                    &text::trunc(
                        &format!(
                            "module {module}  —  {file} (lines {}-{})",
                            def.start, def.end
                        ),
                        width,
                    ),
                    Style::new().fg(t.scope),
                );
                row += 1;
                text::put(
                    buf,
                    inner.x,
                    row,
                    &format!(
                        "{} signal(s)   {} always   {} assign   {} instance(s)",
                        def.signals.len(),
                        def.always.len(),
                        def.assigns.len(),
                        def.instances.len()
                    ),
                    Style::new().fg(t.dim),
                );
                row += 1;
                if let Some(sig) = app.selected_signal() {
                    let name = app
                        .wf
                        .as_ref()
                        .map(|wf| wf.signals[sig].name.clone())
                        .unwrap_or_default();
                    if def.signal(&name).is_some() {
                        if let Some(trace) =
                            app.rtl.as_ref().and_then(|db| db.trace(&module, &name))
                        {
                            let decl = trace.decl.as_ref().map(|loc| loc.line).unwrap_or(0);
                            let lines = |locs: &[crate::rtl::scan::Location]| {
                                locs.iter()
                                    .map(|loc| loc.line.to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            };
                            let always = trace
                                .drivers
                                .iter()
                                .find_map(|loc| {
                                    def.always
                                        .iter()
                                        .find(|b| b.start <= loc.line && loc.line <= b.end)
                                })
                                .map(|b| format!("   always {}-{}", b.start, b.end))
                                .unwrap_or_default();
                            text::put(
                                buf,
                                inner.x,
                                row,
                                &text::trunc(
                                    &format!(
                                        "{name}: decl {decl}   drivers [{}]   loads [{}]{always}",
                                        lines(&trace.drivers),
                                        lines(&trace.loads)
                                    ),
                                    width,
                                ),
                                Style::new().fg(t.cursor),
                            );
                            row += 1;
                        }
                    }
                }
                for decl in &def.signals {
                    if row >= inner.bottom() {
                        break;
                    }
                    let range = decl
                        .range
                        .as_ref()
                        .map(|r| format!(" [{r}]"))
                        .unwrap_or_default();
                    let dir = decl
                        .direction
                        .as_ref()
                        .map(|d| format!("{d} "))
                        .unwrap_or_default();
                    text::put(
                        buf,
                        inner.x,
                        row,
                        &text::trunc(
                            &format!("  {}{range}  {dir}{}", decl.name, decl.kind),
                            width,
                        ),
                        Style::new().fg(t.text),
                    );
                    row += 1;
                }
                for inst in &def.instances {
                    if row >= inner.bottom() {
                        break;
                    }
                    text::put(
                        buf,
                        inner.x,
                        row,
                        &text::trunc(
                            &format!("  {} {}  (line {})", inst.module, inst.name, inst.line),
                            width,
                        ),
                        Style::new().fg(t.name),
                    );
                    row += 1;
                }
            }
        }
    }

    row += 1;
    if module_shown {
        return;
    }
    match &app.sources {
        None => {
            text::put(
                buf,
                inner.x,
                row,
                "no RTL sources: pass -f <filelist> or use File > Load Filelist...",
                Style::new().fg(t.dim),
            );
        }
        Some(set) => {
            text::put(
                buf,
                inner.x,
                row,
                &text::trunc(
                    &format!("{} source file(s)  —  {}", set.files.len(), set.origin),
                    width,
                ),
                Style::new().fg(t.dim),
            );
            row += 1;
            if !set.tops.is_empty() {
                text::put(
                    buf,
                    inner.x,
                    row,
                    &text::trunc(&format!("top: {}", set.tops.join(", ")), width),
                    Style::new().fg(t.scope),
                );
                row += 1;
            }
            for file in &set.files {
                if row >= inner.bottom() {
                    break;
                }
                let name = file
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                text::put(
                    buf,
                    inner.x,
                    row,
                    &text::trunc(&name, width),
                    Style::new().fg(t.text),
                );
                row += 1;
            }
        }
    }
}
