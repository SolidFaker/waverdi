use crate::app::{App, Focus};
use crate::theme::Theme;
use crate::ui::layout::Layout;
use crate::ui::text;
use crate::waveform::{self, Ticks, TimeBase, TimeScale};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::rc::Rc;

pub fn draw_messages(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    buf.set_style(l.msg, Style::new().bg(t.msg_bg));
    let n = app.messages.len();
    for row in 0..3 {
        let k = n as i64 - 3 + row as i64;
        if k < 0 || k >= n as i64 {
            continue;
        }
        let style = if row == 2 {
            Style::new().fg(t.text)
        } else {
            Style::new().fg(t.dim)
        };
        text::put(
            buf,
            l.msg.x,
            l.msg.y + row as u16,
            &app.messages[k as usize],
            style,
        );
    }
}

/// One status line segment: its text and its style.
type Segment = (Rc<str>, Style);

/// Key of the status prefix: the path box and the timescale readout.
#[derive(PartialEq)]
struct PrefixKey {
    path: String,
    status: Rect,
    theme: Theme,
    ts: Option<TimeScale>,
}

/// Key of the cursor readout.
#[derive(PartialEq)]
struct CursorKey {
    cursor: Ticks,
    base: TimeBase,
    ts: TimeScale,
    theme: Theme,
}

/// Key of everything after the cursor: range, zoom, counts and focus.
#[derive(PartialEq)]
struct SuffixKey {
    range: Option<(Ticks, Ticks)>,
    scale: f64,
    base: TimeBase,
    ts: TimeScale,
    display: usize,
    selection: usize,
    visual: bool,
    register: usize,
    focus: Focus,
    theme: Theme,
}

/// Status line cache. The line is split into the prefix (path box, timescale),
/// the cursor readout and the suffix (range, zoom, counts, focus), each keyed
/// by its own inputs. Moving the cursor - the most frequent redraw - only
/// re-formats the readout; every other field rebuilds on change, not per
/// frame.
#[derive(Default)]
pub(crate) struct StatusCache {
    prefix_key: Option<PrefixKey>,
    prefix: Vec<Segment>,
    cursor_key: Option<CursorKey>,
    cursor: Vec<Segment>,
    suffix_key: Option<SuffixKey>,
    suffix: Vec<Segment>,
}

impl StatusCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn prefix(&mut self, app: &App, l: &Layout) -> &[Segment] {
        let ts = app.wf.as_ref().map(|wf| wf.ts);
        let stale = match &self.prefix_key {
            Some(key) => {
                key.path != app.path
                    || key.status != l.status
                    || key.theme != app.theme
                    || key.ts != ts
            }
            None => true,
        };
        if stale {
            self.prefix = build_prefix(&app.path, l.status, ts, &app.theme);
            self.prefix_key = Some(PrefixKey {
                path: app.path.clone(),
                status: l.status,
                theme: app.theme,
                ts,
            });
        }
        &self.prefix
    }

    fn cursor(&mut self, app: &App) -> &[Segment] {
        let Some(wf) = &app.wf else {
            return &[];
        };
        let key = CursorKey {
            cursor: app.cursor,
            base: app.time_base,
            ts: wf.ts,
            theme: app.theme,
        };
        if self.cursor_key.as_ref() != Some(&key) {
            self.cursor = vec![(
                Rc::from(waveform::format_time_base(
                    app.cursor as f64,
                    &wf.ts,
                    app.time_base,
                )),
                Style::new().fg(app.theme.cursor),
            )];
            self.cursor_key = Some(key);
        }
        &self.cursor
    }

    fn suffix(&mut self, app: &App) -> &[Segment] {
        let Some(wf) = &app.wf else {
            return &[];
        };
        let key = SuffixKey {
            range: app.range,
            scale: app.scale,
            base: app.time_base,
            ts: wf.ts,
            display: app.display.len(),
            selection: app.selection.len(),
            visual: app.visual,
            register: app.register.len(),
            focus: app.focus,
            theme: app.theme,
        };
        if self.suffix_key.as_ref() != Some(&key) {
            self.suffix = build_suffix(app, wf.ts);
            self.suffix_key = Some(key);
        }
        &self.suffix
    }
}

pub fn draw_status(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    buf.set_style(l.status, Style::new().bg(t.status_bg));
    let mut x = l.status.x;

    if let Some(progress) = app.load_progress_text() {
        write_segment(buf, l, &mut x, " ", Style::new().fg(t.path));
        write_segment(
            buf,
            l,
            &mut x,
            &progress,
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
        );
        return;
    }

    let mut cache = app.status_cache.borrow_mut();
    // The three parts are borrowed one after another so the shared cache is
    // only mutably borrowed while its own segments are painted.
    for (text, style) in cache.prefix(app, l) {
        write_segment(buf, l, &mut x, text, *style);
    }
    for (text, style) in cache.cursor(app) {
        write_segment(buf, l, &mut x, text, *style);
    }
    for (text, style) in cache.suffix(app) {
        write_segment(buf, l, &mut x, text, *style);
    }
}

/// Write one segment, clipped to the remaining status width.
fn write_segment(buf: &mut Buffer, l: &Layout, x: &mut u16, s: &str, style: Style) {
    let shown = text::trunc(s, l.status.right().saturating_sub(*x) as usize);
    buf.set_string(*x, l.status.y, &shown, style);
    *x = (*x + shown.chars().count() as u16).min(l.status.right());
}

fn build_prefix(path: &str, status: Rect, ts: Option<TimeScale>, t: &Theme) -> Vec<Segment> {
    let sep = Style::new().fg(t.dim);
    let mut out = Vec::new();
    if path.is_empty() {
        out.push((Rc::from(" no file"), Style::new().fg(t.dim)));
    } else {
        // Right-aligned path box: keep the file name (tail) readable when the
        // path is too long, and shrink the box on narrow terminals.
        const PATH_W: usize = 30;
        let avail = status.right().saturating_sub(status.x + 1) as usize;
        let width = avail.min(PATH_W);
        let shown = text::trunc_left(path, width);
        let pad = width.saturating_sub(shown.chars().count());
        out.push((Rc::from(" "), Style::new().fg(t.path)));
        out.push((Rc::from(" ".repeat(pad)), Style::new().fg(t.path)));
        out.push((Rc::from(shown), Style::new().fg(t.path)));
    }
    out.push((Rc::from(" | "), sep));
    if let Some(ts) = ts {
        out.push((
            Rc::from(format!("timescale {}", ts.label())),
            Style::new().fg(t.text),
        ));
        out.push((Rc::from(" | "), sep));
        out.push((Rc::from("cursor "), Style::new().fg(t.dim)));
    }
    out
}

fn build_suffix(app: &App, ts: TimeScale) -> Vec<Segment> {
    let t = &app.theme;
    let sep = Style::new().fg(t.dim);
    let mut out = Vec::new();
    if let Some((a, b)) = app.range {
        out.push((Rc::from(" | "), sep));
        out.push((Rc::from("ΔT "), Style::new().fg(t.dim)));
        out.push((
            Rc::from(waveform::format_time_base(
                (b - a) as f64,
                &ts,
                app.time_base,
            )),
            Style::new().fg(t.scope),
        ));
    }
    out.push((Rc::from(" | "), sep));
    out.push((Rc::from("zoom "), Style::new().fg(t.dim)));
    out.push((
        Rc::from(format!(
            "{}/char",
            waveform::format_time_base(app.scale, &ts, app.time_base)
        )),
        Style::new().fg(t.text),
    ));
    out.push((Rc::from(" | "), sep));
    out.push((Rc::from("signals "), Style::new().fg(t.dim)));
    out.push((
        Rc::from(app.display.len().to_string()),
        Style::new().fg(t.high),
    ));
    if !app.selection.is_empty() {
        out.push((Rc::from(" | "), sep));
        out.push((Rc::from("sel "), Style::new().fg(t.dim)));
        out.push((
            Rc::from(app.selection.len().to_string()),
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
    }
    if app.visual {
        out.push((Rc::from(" | "), sep));
        out.push((
            Rc::from("VISUAL"),
            Style::new()
                .fg(Color::Black)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if !app.register.is_empty() {
        out.push((Rc::from(" | "), sep));
        out.push((Rc::from("reg "), Style::new().fg(t.dim)));
        out.push((
            Rc::from(app.register.len().to_string()),
            Style::new().fg(t.scope),
        ));
    }
    out.push((Rc::from(" | "), sep));
    out.push((Rc::from("focus "), Style::new().fg(t.dim)));
    out.push((
        Rc::from(app.focus.name()),
        Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
    ));
    out
}

pub fn draw_empty(buf: &mut Buffer, l: &Layout, t: &Theme) {
    let lines = [
        "waverdi — Verdi-style terminal RTL waveform viewer",
        "Press 'o' to open a VCD/FST dump, or run: waverdi <file>",
        "F1 / '?' for key bindings",
    ];
    for (i, line) in lines.iter().enumerate() {
        let style = if i == 0 {
            Style::new().fg(t.accent)
        } else {
            Style::new().fg(t.text)
        };
        text::put(buf, l.wave.x + 2, l.wave.y + 4 + i as u16, line, style);
    }
    text::put(
        buf,
        l.list.x + 1,
        l.list.y + 3,
        "no waveform",
        Style::new().fg(t.dim),
    );
}
