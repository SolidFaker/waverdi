use crate::app::App;
use crate::theme::Theme;
use crate::ui::layout::Layout;
use crate::ui::scrollbar;
use crate::ui::text;
use crate::waveform::{self, Change, Signal, Value, Waveform};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Widget as _};
use std::collections::HashMap;
use std::rc::Rc;

const VLINE: &str = "│";
/// High / low level traces (thin lines at the top / bottom of the cell).
const LEVEL_HIGH: &str = "▔";
const LEVEL_LOW: &str = "▁";
/// Rising / falling edges. Used when a cell contains a single transition;
/// dense activity collapses to a vertical bar, so a narrow pulse is never
/// dropped even at very coarse zoom levels.
const EDGE_RISE: &str = "/";
const EDGE_FALL: &str = "\\";
/// Bus traces and their change markers.
const BUS_LINE: &str = "─";
const BUS_CROSS: &str = "╳";
const RULER_TICK: &str = "┴";
const SCROLL_THUMB: &str = "█";
const SCROLL_TRACK: &str = "─";
const HALF: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Logical colour of a sampled cell. Themes are resolved when the cell is
/// painted, so a theme change never invalidates the sampled columns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WaveInk {
    High,
    Low,
    X,
    Z,
    Bus,
    Analog,
}

/// One sampled waveform column: the glyph plus the ink that colours it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct WaveCell {
    pub(crate) glyph: &'static str,
    pub(crate) ink: WaveInk,
}

/// Range sentinel of rows that have no analog scale. NaN is unreachable as a
/// real range (`draw_analog_row` normalizes non-finite ranges), so a bit or
/// bus row can never collide with the analog row of the same signal - not
/// even when that signal's range is exactly `(0.0, 0.0)`.
const NO_RANGE: (f64, f64) = (f64::NAN, f64::NAN);

/// Key of a sampled row: the zoom window, the pane width, the value / radix
/// versions and the analog display range. The cursor and selection are absent
/// on purpose so overlay-only redraws reuse the sampled columns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct WaveKey {
    t0: u64,
    scale: u64,
    cols: u16,
    width: u16,
    waveform: u64,
    radix: u64,
    min: u64,
    max: u64,
}

impl WaveKey {
    /// Key of one drawn row; `range` only matters for analog rows (others
    /// pass [`NO_RANGE`]).
    pub(crate) fn new(
        t0: f64,
        scale: f64,
        cols: usize,
        width: u16,
        waveform: u64,
        radix: u64,
        range: (f64, f64),
    ) -> Self {
        Self {
            t0: t0.to_bits(),
            scale: scale.to_bits(),
            cols: cols as u16,
            width,
            waveform,
            radix,
            min: range.0.to_bits(),
            max: range.1.to_bits(),
        }
    }
}

/// Sampled columns of waveform rows, keyed per row index. `get_or_compute`
/// runs the sampler only on a miss, so cursor moves, range highlights and
/// selection changes only repaint the cached cells.
#[derive(Default)]
pub(crate) struct WaveRowCache {
    rows: HashMap<usize, (WaveKey, Rc<[WaveCell]>)>,
}

impl WaveRowCache {
    pub(crate) fn clear(&mut self) {
        self.rows.clear();
    }

    pub(crate) fn get_or_compute(
        &mut self,
        index: usize,
        key: WaveKey,
        compute: impl FnOnce() -> Vec<WaveCell>,
    ) -> Rc<[WaveCell]> {
        if let Some((cached, cells)) = self.rows.get(&index) {
            if *cached == key {
                return cells.clone();
            }
        }
        let cells: Rc<[WaveCell]> = compute().into();
        self.rows.insert(index, (key, cells.clone()));
        cells
    }
}

/// Frame of the merged nWave window (Signal List + waveforms).
pub fn draw_nwave_frame(buf: &mut Buffer, l: &Layout, t: &Theme, focused: bool) {
    let border_style = if focused {
        Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(t.panel_border)
    };
    let title_style = if focused {
        Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(t.text).add_modifier(Modifier::BOLD)
    };
    Block::bordered()
        .title(" nWave ")
        .title_style(title_style)
        .border_style(border_style)
        .render(l.nwave, buf);
}

pub fn draw(buf: &mut Buffer, l: &Layout, app: &App, wf: &Waveform) {
    let t = &app.theme;
    let canvas = ratatui::layout::Rect {
        x: l.wave.x,
        y: l.wave.y,
        width: l.wave.width,
        height: l.wave.height,
    };
    buf.set_style(canvas, Style::new().bg(t.wave_bg));

    draw_ruler(buf, l, app, wf);

    let rows = app.list_rows();
    let scroll = app.row_scroll.min(rows.len().saturating_sub(l.rows_h));
    for row in 0..l.rows_h {
        let k = scroll + row;
        let Some(list_row) = rows.get(k) else { break };
        let selected = Some(k) == app.sel_row;
        let multi = matches!(list_row, crate::app::ListRow::Signal { sig, .. } if app.selection.contains(sig));
        let row_bg = if selected {
            t.wave_sel_bg
        } else if multi {
            t.wave_multi_bg
        } else if row.is_multiple_of(2) {
            t.wave_alt
        } else {
            t.wave_bg
        };
        let y = l.rows.y + row as u16;
        match list_row {
            crate::app::ListRow::Group { .. } => draw_group_row(buf, l, t, selected, y),
            crate::app::ListRow::Signal { sig, .. } => {
                draw_signal_row(buf, l, app, *sig, wf, row_bg, y);
            }
        }
    }

    if app.display.is_empty() {
        let msg = "Instance: select a signal, Enter / 'a' to add — or press 's' to search";
        let y = l.rows.y + (l.rows_h as u16 / 2);
        text::put(buf, l.rows.x, y, msg, Style::new().fg(t.wave_dim));
    }

    draw_range(buf, l, app);
    draw_cursor(buf, l, app);
    draw_scrollbars(buf, l, app);
}

/// Group boundary row: the group name is only shown in the Signal List.
fn draw_group_row(buf: &mut Buffer, l: &Layout, t: &Theme, selected: bool, y: u16) {
    let bg = if selected { t.wave_sel_bg } else { t.wave_bg };
    buf.set_style(
        ratatui::layout::Rect {
            x: l.rows.x,
            y,
            width: l.rows.width,
            height: 1,
        },
        Style::new().bg(bg),
    );
    buf.set_string(
        l.rows.x,
        y,
        "─".repeat(l.rows.width as usize),
        Style::new().fg(t.wave_dim).bg(bg),
    );
}

/// Pane label; highlighted while the waveform pane owns the focus.
fn draw_ruler(buf: &mut Buffer, l: &Layout, app: &App, wf: &Waveform) {
    let t = &app.theme;
    if l.cols == 0 || app.scale <= 0.0 {
        return;
    }
    let ts = wf.ts;
    let step = waveform::nice_step(app.scale, &ts);
    if !step.is_finite() || step <= 0.0 {
        return;
    }
    let t_end = app.t0 + l.cols as f64 * app.scale;
    let mut tick = (app.t0 / step).ceil() * step;
    let mut last_label_end: i64 = i64::MIN;
    let tick_color = if app.focus == crate::app::Focus::Wave {
        t.accent
    } else {
        t.tick
    };
    // Cursor time at the right edge of the ruler (like Verdi's cursor label).
    // Reserve its space so tick labels never overlap it or the scrollbar.
    let cursor_label = format!(
        " {} ",
        waveform::format_time_base(app.cursor as f64, &ts, app.time_base)
    );
    let cursor_w = cursor_label.chars().count() as u16;
    let cursor_x = l.wave.right().saturating_sub(cursor_w + 1);
    let mut guard = 0;
    while tick <= t_end && guard < 10_000 {
        guard += 1;
        let col = ((tick - app.t0) / app.scale).round() as i64;
        if col >= 0 && (col as usize) < l.cols {
            let x = l.wave.x as i64 + col;
            if x >= 0 && (x as usize) < l.wave.right() as usize {
                text::set_cell(
                    buf,
                    x as u16,
                    l.ruler.y + 1,
                    RULER_TICK,
                    tick_color,
                    t.wave_bg,
                );
                let mut label = waveform::format_time_base(tick, &ts, app.time_base);
                let mut lw = label.chars().count() as i64;
                let mut lx = x - lw / 2;
                // A label that would run under the list/wave divider is clipped
                // and marked with `<` instead of being dropped.
                let mut clipped = false;
                if lx < l.wave.x as i64 {
                    let skip = (l.wave.x as i64 - lx).min(lw) as usize;
                    let rest: String = label.chars().skip(skip).collect();
                    label = format!("<{rest}");
                    lw = label.chars().count() as i64;
                    lx = l.wave.x as i64;
                    clipped = true;
                }
                let gap = if clipped { 1 } else { 2 };
                if lw > 1 && lx >= last_label_end + gap && lx + lw <= cursor_x as i64 {
                    text::put(
                        buf,
                        lx as u16,
                        l.ruler.y,
                        &label,
                        Style::new().fg(t.wave_text),
                    );
                    last_label_end = lx + lw;
                }
            }
        }
        tick += step;
    }

    text::put(
        buf,
        cursor_x,
        l.ruler.y,
        &cursor_label,
        Style::new().fg(t.cursor).add_modifier(Modifier::BOLD),
    );
}

fn draw_signal_row(
    buf: &mut Buffer,
    l: &Layout,
    app: &App,
    idx: usize,
    wf: &Waveform,
    row_bg: Color,
    y: u16,
) {
    let sig = &wf.signals[idx];
    if sig.state != crate::waveform::SigState::Ready {
        let label = if sig.state == crate::waveform::SigState::Loading {
            "… loading"
        } else {
            "…"
        };
        text::put(
            buf,
            l.rows.x + 1,
            y,
            label,
            Style::new().fg(app.theme.dim).bg(row_bg),
        );
        return;
    }
    // Highlighted signals paint their waveform row like the Signal List name.
    let row_bg = app.highlight_of(idx).unwrap_or(row_bg);
    if let Some(&(min, max)) = app.analog.get(&idx) {
        draw_analog_row(buf, l, app, idx, sig, row_bg, y, (min, max));
        return;
    }
    match sig.kind {
        waveform::SigKind::Bits if sig.bits <= 1 => draw_bit_row(buf, l, app, idx, sig, row_bg, y),
        waveform::SigKind::Bits | waveform::SigKind::Str => {
            draw_bus_row(buf, l, app, wf, idx, row_bg, y)
        }
        waveform::SigKind::Real => {
            draw_analog_row(buf, l, app, idx, sig, row_bg, y, (sig.min, sig.max))
        }
    }
}

/// Paint sampled columns of one row. The cursor and range overlays are drawn
/// afterwards and stay on top of the cached cells.
fn paint_cells(buf: &mut Buffer, l: &Layout, t: &Theme, cells: &[WaveCell], y: u16, row_bg: Color) {
    for (col, cell) in cells.iter().enumerate() {
        let fg = match cell.ink {
            WaveInk::High => t.high,
            WaveInk::Low => t.low,
            WaveInk::X => t.xcol,
            WaveInk::Z => t.zcol,
            WaveInk::Bus => t.bus,
            WaveInk::Analog => t.analog,
        };
        text::set_cell(buf, l.rows.x + col as u16, y, cell.glyph, fg, row_bg);
    }
}

/// Ink of a value summary (`summarize` code): 0 low, 1 high, 2 unknown, 3 z.
fn summary_ink(summary: u8) -> WaveInk {
    match summary {
        1 => WaveInk::High,
        0 => WaveInk::Low,
        2 => WaveInk::X,
        _ => WaveInk::Z,
    }
}

/// Cell of a column that holds no transition: the level/unknown rail.
fn run_cell(summary: u8) -> WaveCell {
    match summary {
        1 => WaveCell {
            glyph: LEVEL_HIGH,
            ink: WaveInk::High,
        },
        0 => WaveCell {
            glyph: LEVEL_LOW,
            ink: WaveInk::Low,
        },
        2 => WaveCell {
            glyph: BUS_LINE,
            ink: WaveInk::X,
        },
        _ => WaveCell {
            glyph: BUS_LINE,
            ink: WaveInk::Z,
        },
    }
}

/// Ink of a bus span from its value's unknown kind (2 = x, 3 = z, `None` =
/// known): the whole span between two transitions is tinted like the value it
/// holds, while known values keep the ordinary bus ink. x wins over z, like
/// the per-digit colouring of the value text.
fn unknown_ink(kind: Option<u8>) -> WaveInk {
    match kind {
        Some(2) => WaveInk::X,
        Some(3) => WaveInk::Z,
        _ => WaveInk::Bus,
    }
}

/// Index of the first change that does not precede `col_end` (i.e. its time is
/// `>= col_end`), searched from `from`.
///
/// A plain walk would visit every change inside the visible window, which is
/// O(changes in view) per row and explodes at coarse zoom. Galloping from the
/// current index costs O(1) for a sparse column and O(log changes in the
/// column) for a dense one, exactly like the old per-column binary search but
/// without rescanning the whole list. The result is identical to the walk
/// (`partition_point` semantics on the time-ordered list).
fn first_not_before(
    len: usize,
    from: usize,
    col_end: f64,
    time_at: impl Fn(usize) -> f64,
) -> usize {
    if from >= len || time_at(from) >= col_end {
        return from;
    }
    let mut lo = from;
    let mut span = 1usize;
    let mut hi = (lo + span).min(len);
    while hi < len && time_at(hi) < col_end {
        lo = hi;
        span = span.saturating_mul(2);
        hi = lo.saturating_add(span).min(len);
    }
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if time_at(mid) < col_end {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

/// Sample a single-bit row column by column. The change index only ever
/// advances with the columns and the value summary is computed once per run
/// of columns holding the same value, so dense change lists are never
/// re-scanned or re-summarized per column.
fn sample_bit_row(changes: &[Change], t0: f64, scale: f64, cols: usize) -> Vec<WaveCell> {
    // A change belongs to the column its tick rounds to, so an edge symbol
    // sits exactly under the cursor line at the same time.
    let cut = t0 - 0.5 * scale;
    let mut i = changes.partition_point(|c| (c.t as f64) < cut);
    let mut value: Option<&Value> = if i > 0 { Some(&changes[i - 1].v) } else { None };
    let mut summary = summarize(value);
    let mut cells = Vec::with_capacity(cols);

    for col in 0..cols {
        let col_end = t0 + (col as f64 + 0.5) * scale;
        let before = i;
        i = first_not_before(changes.len(), i, col_end, |k| changes[k].t as f64);
        let transitions = i - before;
        if transitions > 0 {
            value = Some(&changes[i - 1].v);
            summary = summarize(value);
        }
        cells.push(match transitions {
            0 => run_cell(summary),
            1 => match value.and_then(|v| v.bit(0)) {
                Some(1) => WaveCell {
                    glyph: EDGE_RISE,
                    ink: WaveInk::High,
                },
                Some(0) => WaveCell {
                    glyph: EDGE_FALL,
                    ink: WaveInk::Low,
                },
                _ => WaveCell {
                    glyph: VLINE,
                    ink: summary_ink(summary),
                },
            },
            // Several transitions in one cell (a pulse or dense activity):
            // collapse them to a bar like `____|____`, never dropping the
            // event from the view.
            _ => WaveCell {
                glyph: VLINE,
                ink: summary_ink(summary),
            },
        });
    }
    cells
}

/// Sample a bus row: change markers use the same rounding as the cursor
/// column, and the time list is searched from the last drawn column instead
/// of binary-searching it per column. `ink_at` classifies the value held
/// during a span (`None` before the first change), so unknown spans carry the
/// x/z ink while known ones keep the bus ink.
fn sample_bus_row(
    times: &BusTimes<'_>,
    t0: f64,
    scale: f64,
    cols: usize,
    ink_at: impl Fn(Option<usize>) -> WaveInk,
) -> Vec<WaveCell> {
    let cut = t0 - 0.5 * scale;
    let mut i = time_partition_point(times, |time| (time as f64) < cut);
    let mut ink = ink_at(i.checked_sub(1));
    let mut cells = Vec::with_capacity(cols);
    for col in 0..cols {
        let col_end = t0 + (col as f64 + 0.5) * scale;
        let before = i;
        i = first_not_before(times.len(), i, col_end, |k| times.time_at(k) as f64);
        if i > before {
            // The column's marker and rail belong to the new value.
            ink = ink_at(Some(i - 1));
        }
        let glyph = if i > before { BUS_CROSS } else { BUS_LINE };
        cells.push(WaveCell { glyph, ink });
    }
    cells
}

/// Sample an analog row. The change index advances with the columns; a column
/// containing changes keeps spikes visible by drawing the extreme value seen
/// in it, with the scan capped so dense columns stay cheap.
fn sample_analog_row(
    changes: &[Change],
    t0: f64,
    scale: f64,
    cols: usize,
    min: f64,
    max: f64,
) -> Vec<WaveCell> {
    let cut = t0 - 0.5 * scale;
    let mut i = changes.partition_point(|c| (c.t as f64) < cut);
    let mut value = if i > 0 {
        numeric_value(&changes[i - 1].v).unwrap_or(min)
    } else {
        min
    };
    if !value.is_finite() {
        value = min;
    }
    let mut cells = Vec::with_capacity(cols);

    for col in 0..cols {
        let col_end = t0 + (col as f64 + 0.5) * scale;
        let before = i;
        i = first_not_before(changes.len(), i, col_end, |k| changes[k].t as f64);
        if i > before {
            let count = i - before;
            let stride = (count / 64).max(1);
            let mut extreme = value;
            let mut best = 0.0f64;
            let mut index = before;
            while index < i {
                let candidate = numeric_value(&changes[index].v).unwrap_or(value);
                let distance = (candidate - value).abs();
                if distance > best {
                    best = distance;
                    extreme = candidate;
                }
                index += stride;
            }
            let last = numeric_value(&changes[i - 1].v).unwrap_or(value);
            if (last - value).abs() >= best {
                extreme = last;
            }
            value = extreme;
        }
        let level = if max == min {
            0.5
        } else {
            ((value - min) / (max - min)).clamp(0.0, 1.0)
        };
        let half = (level * 7.0).round() as usize;
        cells.push(WaveCell {
            glyph: HALF[half.min(7)],
            ink: WaveInk::Analog,
        });
    }
    cells
}

/// Draw a single-bit signal as a square wave: high/low rails joined by edges.
fn draw_bit_row(
    buf: &mut Buffer,
    l: &Layout,
    app: &App,
    idx: usize,
    sig: &Signal,
    row_bg: Color,
    y: u16,
) {
    let cells = app.wave_row(idx, l, NO_RANGE, || {
        sample_bit_row(&sig.changes, app.t0, app.scale, l.cols)
    });
    paint_cells(buf, l, &app.theme, &cells, y, row_bg);
}

/// Draw a bus (or string) as a horizontal trace, writing the value inside each
/// visible segment the way Verdi's nWave does. Synthesized arrays and
/// aggregates join their members at each drawn time instead of reading a
/// stored change list.
fn draw_bus_row(
    buf: &mut Buffer,
    l: &Layout,
    app: &App,
    wf: &Waveform,
    idx: usize,
    row_bg: Color,
    y: u16,
) {
    let radix = app.radix_for(idx);
    // Logic rows (including the brace values of synthesized arrays and
    // aggregates) colour unknown digits; plain string rows do not, so a
    // literal 'x'/'z' in their text keeps the bus colour.
    let unknown_colors = wf.is_synthesized(idx) || wf.signals[idx].kind == waveform::SigKind::Bits;
    // A grouped radix hides the bits behind digits, so an unknown value is
    // only visible as the tint of its span; a binary row prints every bit
    // itself and keeps the plain bus colour.
    let classify = unknown_colors && radix != waveform::Radix::Bin;
    if wf.is_synthesized(idx) {
        // The cached Arc is walked in place; the joined labels are cached per
        // zoom window so a cursor-only redraw does not re-join the members.
        let times = wf.value_times(idx);
        let ink_at = |j: Option<usize>| match (classify, j) {
            (true, Some(j)) => unknown_ink(wf.unknown_kind_at(idx, times[j])),
            // Before the first member change the brace text reads `x`.
            (true, None) => WaveInk::X,
            _ => WaveInk::Bus,
        };
        draw_bus_trace(
            buf,
            l,
            app,
            idx,
            row_bg,
            y,
            unknown_colors,
            BusTimes::Merged(&times),
            ink_at,
            |j| {
                let time = times[j];
                app.wave_label(idx, time, || match wf.value_at(idx, time) {
                    Some(value) => waveform::fmt_value(&value, radix),
                    None => "x".to_string(),
                })
            },
        );
        return;
    }
    let changes = &wf.signals[idx].changes;
    let ink_at = |j: Option<usize>| match (classify, j) {
        (true, Some(j)) => unknown_ink(changes[j].v.unknown_kind()),
        (true, None) => WaveInk::X,
        _ => WaveInk::Bus,
    };
    draw_bus_trace(
        buf,
        l,
        app,
        idx,
        row_bg,
        y,
        unknown_colors,
        BusTimes::Changes(changes),
        ink_at,
        |j| {
            let text = waveform::fmt_value(&changes[j].v, radix);
            let width = text.chars().count();
            (Rc::from(text), width)
        },
    );
}

/// Time source of a bus row: the zero-copy change list of a plain signal or
/// the merged times of a synthesized one.
enum BusTimes<'a> {
    Changes(&'a [waveform::Change]),
    Merged(&'a [waveform::Ticks]),
}

impl BusTimes<'_> {
    fn len(&self) -> usize {
        match self {
            BusTimes::Changes(changes) => changes.len(),
            BusTimes::Merged(times) => times.len(),
        }
    }

    fn time_at(&self, index: usize) -> waveform::Ticks {
        match self {
            BusTimes::Changes(changes) => changes[index].t,
            BusTimes::Merged(times) => times[index],
        }
    }
}

/// Shared drawing of a bus row over a virtual, time-ordered change list. The
/// text of a change and its display width are provided lazily so plain
/// signals keep their zero-copy change list while synthesized signals reuse
/// the labels cached for the current zoom window; `ink_at` likewise classifies
/// a span's held value without materializing the values first.
#[allow(clippy::too_many_arguments)]
fn draw_bus_trace(
    buf: &mut Buffer,
    l: &Layout,
    app: &App,
    idx: usize,
    row_bg: Color,
    y: u16,
    unknown_colors: bool,
    times: BusTimes<'_>,
    ink_at: impl Fn(Option<usize>) -> WaveInk,
    text_at: impl Fn(usize) -> (Rc<str>, usize),
) {
    let t = &app.theme;
    let (t0, scale) = (app.t0, app.scale);
    let n = times.len();
    let cells = app.wave_row(idx, l, NO_RANGE, || {
        sample_bus_row(&times, t0, scale, l.cols, ink_at)
    });
    paint_cells(buf, l, t, &cells, y, row_bg);

    if n == 0 || l.cols == 0 {
        return;
    }
    let width = l.cols as i64;
    let t_end = t0 + width as f64 * scale;
    let mut j = time_partition_point(&times, |time| (time as f64) <= t0).saturating_sub(1);
    while j < n {
        let cs = times.time_at(j) as f64;
        if cs >= t_end {
            break;
        }
        let ce = if j + 1 < n {
            times.time_at(j + 1) as f64
        } else {
            t_end
        };
        let c0 = (((cs - t0) / scale).round() as i64).clamp(0, width);
        let c1 = (((ce - t0) / scale).round() as i64).clamp(0, width);
        let (value, len) = text_at(j);
        let len = len as i64;
        if c1 - c0 >= len + 2 {
            let x = l.rows.x + (c0 + 1) as u16;
            let style = Style::new().fg(t.bus_text).bg(row_bg);
            if unknown_colors {
                text::put_unknown_digits(buf, x, y, &value, style, t.xcol, t.zcol);
            } else {
                text::put(buf, x, y, &value, style);
            }
            j += 1;
        } else {
            // Segment too narrow for a label: jump to the segment that is
            // active at the first column where a label could fit. This skips
            // long runs of narrow changes without ever skipping a wide
            // segment that follows them.
            let target_col = (c0 + (len + 2).max(1)).clamp(1, width);
            let target_t = t0 + target_col as f64 * scale;
            let active =
                time_partition_point(&times, |time| (time as f64) <= target_t).saturating_sub(1);
            j = active.max(j + 1);
        }
    }
}

/// Partition point of a bus row's time list: index of the first entry whose
/// time makes `pred` false, all earlier entries satisfying it
/// (`slice::partition_point` needs a real slice).
fn time_partition_point(times: &BusTimes<'_>, pred: impl Fn(waveform::Ticks) -> bool) -> usize {
    let (mut lo, mut hi) = (0, times.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if pred(times.time_at(mid)) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

#[allow(clippy::too_many_arguments)]
fn draw_analog_row(
    buf: &mut Buffer,
    l: &Layout,
    app: &App,
    idx: usize,
    sig: &Signal,
    row_bg: Color,
    y: u16,
    range: (f64, f64),
) {
    let (min, max) = if range.0.is_finite() && range.1.is_finite() {
        range
    } else {
        (0.0, 1.0)
    };
    let cells = app.wave_row(idx, l, (min, max), || {
        sample_analog_row(&sig.changes, app.t0, app.scale, l.cols, min, max)
    });
    paint_cells(buf, l, &app.theme, &cells, y, row_bg);
}

/// Numeric view of a change value, used when rendering logic signals as analog.
fn numeric_value(value: &Value) -> Option<f64> {
    if let Some(real) = value.as_real() {
        return Some(real);
    }
    waveform::value_number(value).map(|v| v as f64)
}

fn summarize(value: Option<&Value>) -> u8 {
    let Some(value) = value else { return 2 };
    let mut zero = true;
    for i in 0..value.bits_len() {
        match value.bit(i) {
            Some(0) => {}
            Some(1) => zero = false,
            Some(2) => return 2,
            _ => return 3,
        }
    }
    if zero {
        0
    } else {
        1
    }
}

fn draw_range(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    let Some((a, b)) = app.range else { return };
    if l.cols == 0 || l.rows_h == 0 {
        return;
    }
    let c0 = (((a as f64 - app.t0) / app.scale).floor() as i64).clamp(0, l.cols as i64 - 1);
    let c1 = (((b as f64 - app.t0) / app.scale).ceil() as i64).clamp(0, l.cols as i64);
    for y in l.rows.y..l.rows.bottom() {
        for col in c0..c1 {
            if let Some(cell) = buf.cell_mut((l.rows.x + col as u16, y)) {
                cell.set_bg(t.range_bg);
            }
        }
    }
}

fn draw_cursor(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    if l.cols == 0 {
        return;
    }
    let cx = app.x_at_tick(app.cursor);
    if cx < 0 || cx as usize >= l.cols {
        return;
    }
    let x = l.rows.x + cx as u16;
    for y in l.ruler.y..l.rows.bottom() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            let bg = cell.bg;
            cell.set_symbol(VLINE);
            cell.set_fg(t.cursor);
            cell.set_bg(bg);
        }
    }
}

fn draw_vscroll(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    let Some((start, len)) =
        scrollbar::proportional_geometry(l.rows_h, app.rows_len(), l.rows_h, app.row_scroll)
    else {
        return;
    };
    scrollbar::Bar {
        orientation: scrollbar::Orientation::Vertical,
        x: l.vscroll_x,
        y: l.rows.y,
        span: l.rows_h,
        start,
        len,
        thumb: SCROLL_THUMB,
        track: VLINE,
        thumb_fg: t.accent,
        track_fg: t.wave_dim,
        bg: t.wave_bg,
    }
    .draw(buf);
}

/// Thumb position and width (in columns) of the time scrollbar.
pub(crate) fn hscroll_thumb(
    cols: usize,
    scale: f64,
    t0: f64,
    start: f64,
    end: f64,
) -> (usize, usize) {
    if cols == 0 {
        return (0, 0);
    }
    let span = cols as f64 * scale;
    let total = (end - start).max(span).max(1e-9);
    let frac = (span / total).clamp(0.0, 1.0);
    let thumb_w = (frac * cols as f64).round().max(1.0) as usize;
    let max_t0 = (total - span).max(0.0);
    let pos = if max_t0 > 0.0 {
        ((t0 - start) / max_t0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let room = cols.saturating_sub(thumb_w);
    let thumb_x = (pos * room as f64).round() as usize;
    (thumb_x.min(room), thumb_w.min(cols))
}

fn draw_hscroll(buf: &mut Buffer, l: &Layout, app: &App) {
    let t = &app.theme;
    let Some(wf) = &app.wf else { return };
    if l.cols == 0 {
        return;
    }
    let (thumb_x, thumb_w) =
        hscroll_thumb(l.cols, app.scale, app.t0, wf.start as f64, wf.end as f64);
    if thumb_w >= l.cols {
        // The whole range is visible: leave the bar plain.
        return;
    }
    scrollbar::Bar {
        orientation: scrollbar::Orientation::Horizontal,
        x: l.wave.x,
        y: l.hscroll.y,
        span: l.cols,
        start: thumb_x,
        len: thumb_w,
        thumb: SCROLL_THUMB,
        track: SCROLL_TRACK,
        thumb_fg: t.accent,
        track_fg: t.wave_dim,
        bg: t.wave_bg,
    }
    .draw(buf);
}

/// Redraw the scrollbars; overlays such as dropdowns are painted afterwards,
/// so they stay on top.
pub(crate) fn draw_scrollbars(buf: &mut Buffer, l: &Layout, app: &App) {
    draw_vscroll(buf, l, app);
    draw_hscroll(buf, l, app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn scrollbar_thumb_reflects_visible_fraction() {
        // Whole waveform visible: thumb fills the bar.
        assert_eq!(hscroll_thumb(50, 16.0, 0.0, 0.0, 800.0), (0, 50));
        // A quarter visible: quarter-width thumb.
        assert_eq!(hscroll_thumb(50, 4.0, 0.0, 0.0, 800.0), (0, 13));
        // Scrolled to the end: thumb sits flush right.
        assert_eq!(hscroll_thumb(50, 4.0, 600.0, 0.0, 800.0), (37, 13));
    }

    /// The old per-column sampler, kept as the reference: it binary-searches
    /// the change list and summarizes the held value for every column.
    fn reference_bit_row(changes: &[Change], t0: f64, scale: f64, cols: usize) -> Vec<WaveCell> {
        let cut = t0 - 0.5 * scale;
        let mut i = changes.partition_point(|c| (c.t as f64) < cut);
        let mut value: Option<&Value> = if i > 0 { Some(&changes[i - 1].v) } else { None };
        let mut out = Vec::with_capacity(cols);
        for col in 0..cols {
            let col_end = t0 + (col as f64 + 0.5) * scale;
            let before = i;
            i = changes.partition_point(|c| (c.t as f64) < col_end);
            let transitions = (i - before) as u32;
            if transitions > 0 {
                value = Some(&changes[i - 1].v);
            }
            let (glyph, ink) = match transitions {
                0 => match summarize(value) {
                    1 => (LEVEL_HIGH, WaveInk::High),
                    0 => (LEVEL_LOW, WaveInk::Low),
                    2 => (BUS_LINE, WaveInk::X),
                    _ => (BUS_LINE, WaveInk::Z),
                },
                1 => match value.and_then(|v| v.bit(0)) {
                    Some(1) => (EDGE_RISE, WaveInk::High),
                    Some(0) => (EDGE_FALL, WaveInk::Low),
                    _ => (VLINE, summary_ink(summarize(value))),
                },
                _ => (VLINE, summary_ink(summarize(value))),
            };
            out.push(WaveCell { glyph, ink });
        }
        out
    }

    /// Reference bus sampler: one binary search per column.
    fn reference_bus_row(times: &BusTimes<'_>, t0: f64, scale: f64, cols: usize) -> Vec<WaveCell> {
        let cut = t0 - 0.5 * scale;
        let mut i = time_partition_point(times, |time| (time as f64) < cut);
        let mut out = Vec::with_capacity(cols);
        for col in 0..cols {
            let col_end = t0 + (col as f64 + 0.5) * scale;
            let before = i;
            i = time_partition_point(times, |time| (time as f64) < col_end);
            let glyph = if i > before { BUS_CROSS } else { BUS_LINE };
            out.push(WaveCell {
                glyph,
                ink: WaveInk::Bus,
            });
        }
        out
    }

    /// A 64-bit value with x and z bits; `lead` shifts the known-bit pattern.
    fn wide(lead: u8) -> Value {
        let mut bits = vec![0u8; 64];
        for (i, bit) in bits.iter_mut().enumerate() {
            *bit = match i % 16 {
                0 => lead % 4,
                8 => 2,
                9 => 3,
                _ => ((i + lead as usize) % 2) as u8,
            };
        }
        Value::Bits(bits)
    }

    /// Transitions, redundant changes, clustered changes and wide values.
    fn wide_changes() -> Vec<Change> {
        vec![
            Change { t: 0, v: wide(0) },
            Change {
                t: 1,
                v: Value::Bits(vec![1; 64]),
            },
            Change { t: 2, v: wide(1) },
            // Redundant: same value again, the summary must stay valid.
            Change { t: 3, v: wide(1) },
            Change { t: 5, v: wide(2) },
            Change { t: 6, v: wide(3) },
            // LSB falls while higher bits stay set: the edge glyph must use
            // bit 0, not the summary of the wide value.
            Change { t: 9, v: wide(2) },
            // A burst of changes that several coarse columns must collapse.
            Change {
                t: 10,
                v: Value::Bits(vec![0; 64]),
            },
            Change {
                t: 11,
                v: Value::Bits(vec![1; 64]),
            },
            Change {
                t: 12,
                v: Value::Bits(vec![0; 64]),
            },
            Change {
                t: 13,
                v: Value::Bits(vec![1; 64]),
            },
            Change {
                t: 20,
                v: Value::Bits(vec![2; 64]),
            },
        ]
    }

    #[test]
    fn incremental_sampling_matches_the_old_per_column_algorithm() {
        let changes = wide_changes();
        for &(t0, scale, cols) in &[
            (0.0, 0.5, 40),
            (0.0, 2.3, 33),
            (-7.0, 1.7, 51),
            (3.0, 0.1, 64),
            (12.0, 5.0, 10),
        ] {
            assert_eq!(
                sample_bit_row(&changes, t0, scale, cols),
                reference_bit_row(&changes, t0, scale, cols),
                "bit row differs at t0={t0} scale={scale} cols={cols}"
            );
        }
    }

    #[test]
    fn incremental_bus_sampling_matches_the_old_per_column_algorithm() {
        let changes = wide_changes();
        let times: Vec<waveform::Ticks> = changes.iter().map(|c| c.t).collect();
        // The reference draws no unknown tint; the equivalence tests compare
        // the transition walk, so both sides keep the plain bus ink.
        let bus = |_: Option<usize>| WaveInk::Bus;
        for &(t0, scale, cols) in &[(0.0, 0.5, 40), (6.0, 1.9, 27), (-3.0, 2.5, 13)] {
            assert_eq!(
                sample_bus_row(&BusTimes::Changes(&changes), t0, scale, cols, bus),
                reference_bus_row(&BusTimes::Changes(&changes), t0, scale, cols),
                "change list differs at t0={t0} scale={scale}"
            );
            assert_eq!(
                sample_bus_row(&BusTimes::Merged(&times), t0, scale, cols, bus),
                reference_bus_row(&BusTimes::Merged(&times), t0, scale, cols),
                "merged times differ at t0={t0} scale={scale}"
            );
        }
    }

    /// The span between two transitions carries the ink of the value held in
    /// it (x over z over known); before the first change the row is x.
    #[test]
    fn bus_sampling_tints_unknown_spans_per_held_value() {
        let changes = vec![
            Change {
                t: 0,
                v: Value::compact(vec![2, 2, 2, 2]),
            },
            Change {
                t: 10,
                v: Value::compact(vec![3, 3, 3, 3]),
            },
            // x and z mixed: x wins, like the digit rule.
            Change {
                t: 20,
                v: Value::compact(vec![3, 2, 3, 2]),
            },
            Change {
                t: 30,
                v: Value::compact(vec![1, 0, 1, 0]),
            },
        ];
        let ink = |j: Option<usize>| match j {
            Some(j) => unknown_ink(changes[j].v.unknown_kind()),
            None => WaveInk::X,
        };
        assert_eq!(
            sample_bus_row(&BusTimes::Changes(&changes), 0.0, 10.0, 4, ink),
            vec![
                WaveCell {
                    glyph: BUS_CROSS,
                    ink: WaveInk::X,
                },
                WaveCell {
                    glyph: BUS_CROSS,
                    ink: WaveInk::Z,
                },
                WaveCell {
                    glyph: BUS_CROSS,
                    ink: WaveInk::X,
                },
                WaveCell {
                    glyph: BUS_CROSS,
                    ink: WaveInk::Bus,
                },
            ]
        );
        // A window before the first change holds no value: unknown x.
        assert_eq!(
            sample_bus_row(&BusTimes::Changes(&changes), -20.0, 10.0, 1, ink),
            vec![WaveCell {
                glyph: BUS_LINE,
                ink: WaveInk::X,
            }]
        );
    }

    /// Deterministic LCG for the dense-row equivalence test.
    fn lcg(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *state
    }

    /// The galloping search must return exactly what the old per-column
    /// `partition_point` did, also when a column spans thousands of changes,
    /// holds duplicate ticks or the view runs past the last change.
    #[test]
    fn galloping_sampling_matches_the_per_column_reference_on_dense_rows() {
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        let mut changes: Vec<Change> = Vec::with_capacity(60_000);
        let mut t: u64 = 0;
        for k in 0..60_000u64 {
            if k % 11 == 0 {
                // Burst: several changes at the same tick.
            } else if k % 5 == 0 {
                t += 1;
            } else {
                t += lcg(&mut state) % 7 + 1;
            }
            changes.push(Change {
                t,
                v: wide((k % 4) as u8),
            });
        }
        let times: Vec<waveform::Ticks> = changes.iter().map(|c| c.t).collect();
        let bus = |_: Option<usize>| WaveInk::Bus;
        for &(t0, scale, cols) in &[
            (0.0, 500.0, 120),          // dense: hundreds of changes per column
            (0.0, 1.0, 60),             // sparse
            (100.0, 0.25, 80),          // zoomed in
            (500.0, 5.5, 47),           // mixed
            (-20.0, 3.0, 33),           // starts before the first change
            (t as f64 + 10.0, 0.5, 40), // past the last change
        ] {
            assert_eq!(
                sample_bit_row(&changes, t0, scale, cols),
                reference_bit_row(&changes, t0, scale, cols),
                "bit row differs at t0={t0} scale={scale} cols={cols}"
            );
            assert_eq!(
                sample_bus_row(&BusTimes::Changes(&changes), t0, scale, cols, bus),
                reference_bus_row(&BusTimes::Changes(&changes), t0, scale, cols),
                "bus row differs at t0={t0} scale={scale} cols={cols}"
            );
            assert_eq!(
                sample_bus_row(&BusTimes::Merged(&times), t0, scale, cols, bus),
                reference_bus_row(&BusTimes::Merged(&times), t0, scale, cols),
                "merged row differs at t0={t0} scale={scale} cols={cols}"
            );
        }
    }

    #[test]
    fn wave_row_cache_reuses_runs_until_the_key_changes() {
        let mut cache = WaveRowCache::default();
        let key = WaveKey::new(10.0, 2.0, 8, 9, 1, 2, (0.0, 1.0));
        let calls = Cell::new(0);
        let build = || {
            calls.set(calls.get() + 1);
            vec![
                WaveCell {
                    glyph: LEVEL_LOW,
                    ink: WaveInk::Low,
                },
                WaveCell {
                    glyph: LEVEL_HIGH,
                    ink: WaveInk::High,
                },
            ]
        };
        let first = cache.get_or_compute(3, key, build);
        let second = cache.get_or_compute(3, key, build);
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(calls.get(), 1, "a key hit must not resample");

        // Cursor moves never reach the cache (no cursor in the key): the row
        // is still a hit.
        let after_cursor_move = cache.get_or_compute(3, key, build);
        assert!(Rc::ptr_eq(&first, &after_cursor_move));
        assert_eq!(calls.get(), 1);

        for (label, moved) in [
            ("t0", WaveKey::new(11.0, 2.0, 8, 9, 1, 2, (0.0, 1.0))),
            ("scale", WaveKey::new(10.0, 3.0, 8, 9, 1, 2, (0.0, 1.0))),
            ("cols", WaveKey::new(10.0, 2.0, 7, 9, 1, 2, (0.0, 1.0))),
            ("width", WaveKey::new(10.0, 2.0, 8, 8, 1, 2, (0.0, 1.0))),
            ("waveform", WaveKey::new(10.0, 2.0, 8, 9, 5, 2, (0.0, 1.0))),
            ("radix", WaveKey::new(10.0, 2.0, 8, 9, 1, 7, (0.0, 1.0))),
            ("range", WaveKey::new(10.0, 2.0, 8, 9, 1, 2, (0.0, 2.0))),
        ] {
            let before = calls.get();
            let cells = cache.get_or_compute(3, moved, build);
            assert_eq!(calls.get(), before + 1, "{label} change must resample");
            assert!(!Rc::ptr_eq(&first, &cells), "{label} change must rebuild");
        }

        // Other row indices do not share cached runs.
        let other = cache.get_or_compute(4, key, build);
        assert!(!Rc::ptr_eq(&first, &other));

        // A digital sentinel and an all-zero analog range must not collide:
        // when a signal is switched to analog, its (0, 0) range is finite.
        assert_ne!(
            WaveKey::new(10.0, 2.0, 8, 9, 1, 2, NO_RANGE),
            WaveKey::new(10.0, 2.0, 8, 9, 1, 2, (0.0, 0.0))
        );
    }
}
