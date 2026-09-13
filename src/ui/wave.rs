use crate::app::App;
use crate::theme::Theme;
use crate::ui::layout::Layout;
use crate::ui::text;
use crate::waveform::{self, Signal, Value, Waveform};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Widget as _};

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
                draw_signal_row(buf, l, app, *sig, &wf.signals[*sig], row_bg, y);
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
    sig: &Signal,
    row_bg: Color,
    y: u16,
) {
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
        draw_analog_row(buf, l, app, sig, row_bg, y, (min, max));
        return;
    }
    match sig.kind {
        waveform::SigKind::Bits if sig.bits <= 1 => draw_bit_row(buf, l, app, sig, row_bg, y),
        waveform::SigKind::Bits | waveform::SigKind::Str => {
            draw_bus_row(buf, l, app, idx, sig, row_bg, y)
        }
        waveform::SigKind::Real => draw_analog_row(buf, l, app, sig, row_bg, y, (sig.min, sig.max)),
    }
}

/// Draw a single-bit signal as a square wave: high/low rails joined by edges.
fn draw_bit_row(buf: &mut Buffer, l: &Layout, app: &App, sig: &Signal, row_bg: Color, y: u16) {
    let t = &app.theme;
    let (t0, scale) = (app.t0, app.scale);
    let changes = &sig.changes;
    // A change belongs to the column its tick rounds to, so an edge symbol
    // sits exactly under the cursor line at the same time.
    let cut = t0 - 0.5 * scale;
    let mut i = changes.partition_point(|c| (c.t as f64) < cut);
    let mut value: Option<&Value> = if i > 0 { Some(&changes[i - 1].v) } else { None };

    for col in 0..l.cols {
        let col_end = t0 + (col as f64 + 0.5) * scale;
        // Only the last change per column matters for the drawn level; a
        // binary search keeps zoomed-out views independent of the change
        // count (view-dependent sparse sampling).
        let before = i;
        i = changes.partition_point(|c| (c.t as f64) < col_end);
        let transitions = (i - before) as u32;
        if transitions > 0 {
            value = Some(&changes[i - 1].v);
        }
        let (symbol, fg) = match transitions {
            0 => match summarize(value) {
                1 => (LEVEL_HIGH, t.high),
                0 => (LEVEL_LOW, t.low),
                2 => (BUS_LINE, t.xcol),
                _ => (BUS_LINE, t.zcol),
            },
            1 => match value.and_then(|v| v.bit(0)) {
                Some(1) => (EDGE_RISE, t.high),
                Some(0) => (EDGE_FALL, t.low),
                _ => (VLINE, rail_color(t, value)),
            },
            // Several transitions in one cell (a pulse or dense activity):
            // collapse them to a bar like `____|____`, never dropping the
            // event from the view.
            _ => (VLINE, rail_color(t, value)),
        };
        text::set_cell(buf, l.rows.x + col as u16, y, symbol, fg, row_bg);
    }
}

/// Draw a bus (or string) as a horizontal trace, writing the value inside each
/// visible segment the way Verdi's nWave does.
fn draw_bus_row(
    buf: &mut Buffer,
    l: &Layout,
    app: &App,
    idx: usize,
    sig: &Signal,
    row_bg: Color,
    y: u16,
) {
    let t = &app.theme;
    let (t0, scale) = (app.t0, app.scale);
    let changes = &sig.changes;
    let n = changes.len();
    // Change markers use the same rounding as the cursor column.
    let cut = t0 - 0.5 * scale;
    let mut i = changes.partition_point(|c| (c.t as f64) < cut);

    for col in 0..l.cols {
        let col_end = t0 + (col as f64 + 0.5) * scale;
        // Binary search instead of walking every change: zoomed-out views
        // stay fast no matter how many changes the signal has.
        let before = i;
        i = changes.partition_point(|c| (c.t as f64) < col_end);
        let transition = i > before;
        let symbol = if transition { BUS_CROSS } else { BUS_LINE };
        text::set_cell(buf, l.rows.x + col as u16, y, symbol, t.bus, row_bg);
    }

    if n == 0 || l.cols == 0 {
        return;
    }
    let radix = app.radix_for(idx);
    let width = l.cols as i64;
    let t_end = t0 + width as f64 * scale;
    let mut j = changes
        .partition_point(|c| (c.t as f64) <= t0)
        .saturating_sub(1);
    while j < n {
        let cs = changes[j].t as f64;
        if cs >= t_end {
            break;
        }
        let ce = if j + 1 < n {
            changes[j + 1].t as f64
        } else {
            t_end
        };
        let c0 = (((cs - t0) / scale).round() as i64).clamp(0, width);
        let c1 = (((ce - t0) / scale).round() as i64).clamp(0, width);
        let value = segment_text(&changes[j].v, radix);
        let len = value.chars().count() as i64;
        if c1 - c0 >= len + 2 {
            text::put(
                buf,
                l.rows.x + (c0 + 1) as u16,
                y,
                &value,
                Style::new().fg(t.bus_text).bg(row_bg),
            );
            j += 1;
        } else {
            // Segment too narrow for a label: jump to the segment that is
            // active at the first column where a label could fit. This skips
            // long runs of narrow changes without ever skipping a wide
            // segment that follows them.
            let target_col = (c0 + (len + 2).max(1)).clamp(1, width);
            let target_t = t0 + target_col as f64 * scale;
            let active = changes
                .partition_point(|c| (c.t as f64) <= target_t)
                .saturating_sub(1);
            j = active.max(j + 1);
        }
    }
}

fn draw_analog_row(
    buf: &mut Buffer,
    l: &Layout,
    app: &App,
    sig: &Signal,
    row_bg: Color,
    y: u16,
    range: (f64, f64),
) {
    let t = &app.theme;
    let (min, max) = if range.0.is_finite() && range.1.is_finite() {
        range
    } else {
        (0.0, 1.0)
    };
    let (t0, scale) = (app.t0, app.scale);
    let changes = &sig.changes;
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

    for col in 0..l.cols {
        let col_end = t0 + (col as f64 + 0.5) * scale;
        let before = i;
        i = changes.partition_point(|c| (c.t as f64) < col_end);
        if i > before {
            // Keep spikes visible: if the column contains a value far from
            // the previous level (e.g. a 1ps pulse) draw that extreme. The
            // scan is capped so dense columns stay cheap.
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
        text::set_cell(
            buf,
            l.rows.x + col as u16,
            y,
            HALF[half.min(7)],
            t.analog,
            row_bg,
        );
    }
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

fn rail_color(t: &Theme, value: Option<&Value>) -> Color {
    match summarize(value) {
        1 => t.high,
        0 => t.low,
        2 => t.xcol,
        _ => t.zcol,
    }
}

fn segment_text(value: &Value, radix: waveform::Radix) -> String {
    waveform::fmt_value(value, radix)
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
    let total = app.rows_len();
    let visible = l.rows_h;
    if total <= visible {
        return;
    }
    let thumb = ((visible as f64 / total as f64) * visible as f64).max(1.0) as usize;
    let top = ((app.row_scroll as f64 / total as f64) * visible as f64) as usize;
    for row in 0..visible {
        let (symbol, fg) = if row >= top && row < top + thumb {
            (SCROLL_THUMB, t.accent)
        } else {
            (VLINE, t.wave_dim)
        };
        text::set_cell(
            buf,
            l.vscroll_x,
            l.rows.y + row as u16,
            symbol,
            fg,
            t.wave_bg,
        );
    }
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
    for col in 0..l.cols {
        let on_thumb = col >= thumb_x && col < thumb_x + thumb_w;
        let (symbol, fg) = if on_thumb {
            (SCROLL_THUMB, t.accent)
        } else {
            (SCROLL_TRACK, t.wave_dim)
        };
        text::set_cell(
            buf,
            l.wave.x + col as u16,
            l.hscroll.y,
            symbol,
            fg,
            t.wave_bg,
        );
    }
}

/// Redraw the scrollbars (used after overlays such as the focused frame).
pub(crate) fn draw_scrollbars(buf: &mut Buffer, l: &Layout, app: &App) {
    draw_vscroll(buf, l, app);
    draw_hscroll(buf, l, app);
}

#[cfg(test)]
mod tests {
    use super::hscroll_thumb;

    #[test]
    fn scrollbar_thumb_reflects_visible_fraction() {
        // Whole waveform visible: thumb fills the bar.
        assert_eq!(hscroll_thumb(50, 16.0, 0.0, 0.0, 800.0), (0, 50));
        // A quarter visible: quarter-width thumb.
        assert_eq!(hscroll_thumb(50, 4.0, 0.0, 0.0, 800.0), (0, 13));
        // Scrolled to the end: thumb sits flush right.
        assert_eq!(hscroll_thumb(50, 4.0, 600.0, 0.0, 800.0), (37, 13));
    }
}
