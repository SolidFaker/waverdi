use crate::app::App;
use crate::theme::*;
use crate::ui::layout::Layout;
use crate::ui::text;
use crate::waveform::{self, Signal, Value, Waveform};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

const VLINE: &str = "│";
/// High / low level traces (thin lines at the top / bottom of the cell).
const LEVEL_HIGH: &str = "▔";
const LEVEL_LOW: &str = "▁";
/// Rising / falling edges. Used when a cell contains a single transition;
/// dense activity collapses back to a vertical bar.
const EDGE_RISE: &str = "/";
const EDGE_FALL: &str = "\\";
/// Bus traces and their change markers.
const BUS_LINE: &str = "─";
const BUS_CROSS: &str = "╳";
const BUS_TEXT: Color = Color::Rgb(230, 230, 240);
const RULER_TICK: &str = "┴";
const SCROLL_THUMB: &str = "█";
const SCROLL_TRACK: &str = "─";
const HALF: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

pub fn draw(buf: &mut Buffer, l: &Layout, app: &App, wf: &Waveform) {
    let canvas = ratatui::layout::Rect {
        x: l.wave.x,
        y: l.wave.y,
        width: l.wave.width,
        height: l.wave.height.saturating_sub(1),
    };
    buf.set_style(canvas, Style::new().bg(BG));
    buf.set_style(l.hscroll, Style::new().bg(TOOLBAR_BG));

    draw_ruler(buf, l, app, wf);

    let rows = app.list_rows();
    let scroll = app.row_scroll.min(rows.len().saturating_sub(l.rows_h));
    for row in 0..l.rows_h {
        let k = scroll + row;
        let Some(list_row) = rows.get(k) else { break };
        let selected = Some(k) == app.sel_row;
        let y = l.rows.y + row as u16;
        match list_row {
            crate::app::ListRow::Group {
                name,
                depth,
                collapsed,
                ..
            } => draw_group_row(buf, l, *depth, name, *collapsed, selected, y),
            crate::app::ListRow::Signal { sig, .. } => {
                draw_signal_row(buf, l, app, *sig, &wf.signals[*sig], selected, y);
            }
        }
    }

    if app.display.is_empty() {
        let msg = "nTrace: select a signal, Enter / 'a' to add — or press 's' to search";
        let y = l.rows.y + (l.rows_h as u16 / 2);
        text::put(buf, l.rows.x, y, msg, Style::new().fg(DIM));
    }

    draw_range(buf, l, app);
    draw_cursor(buf, l, app);
    draw_vscroll(buf, l, app);
    draw_hscroll(buf, l, app);
}

fn draw_group_row(
    buf: &mut Buffer,
    l: &Layout,
    depth: usize,
    name: &str,
    collapsed: bool,
    selected: bool,
    y: u16,
) {
    let bg = if selected { ROW_SEL_BG } else { LIST_HEADER_BG };
    buf.set_style(
        ratatui::layout::Rect {
            x: l.rows.x,
            y,
            width: l.rows.width,
            height: 1,
        },
        Style::new().bg(bg),
    );
    let arrow = if collapsed { "▸" } else { "▾" };
    let label = format!("{}{arrow} {name}/", "  ".repeat(depth));
    let style = if selected {
        Style::new().fg(Color::Black).bg(bg)
    } else {
        Style::new().fg(ACCENT).bg(bg).add_modifier(Modifier::BOLD)
    };
    text::put(buf, l.rows.x + 1, y, &label, style);
}

fn draw_ruler(buf: &mut Buffer, l: &Layout, app: &App, wf: &Waveform) {
    if l.cols == 0 || app.scale <= 0.0 {
        return;
    }
    let ts = wf.ts;
    let step = waveform::nice_step(app.scale, &ts);
    if !step.is_finite() || step <= 0.0 {
        return;
    }
    let t_end = app.t0 + l.cols as f64 * app.scale;
    let mut t = (app.t0 / step).ceil() * step;
    let mut last_label_end: i64 = i64::MIN;
    let mut guard = 0;
    while t <= t_end && guard < 10_000 {
        guard += 1;
        let col = ((t - app.t0) / app.scale).round() as i64;
        if col >= 0 && (col as usize) < l.cols {
            let x = l.wave.x as i64 + col;
            if x >= 0 && (x as usize) < l.wave.right() as usize {
                text::set_cell(buf, x as u16, l.ruler.y + 1, RULER_TICK, TICK, BG);
                let label = waveform::format_time(t, &ts);
                let lw = label.chars().count() as i64;
                let lx = x - lw / 2;
                if lx >= 0
                    && lx >= last_label_end + 2
                    && (lx + lw) as usize <= l.wave.right() as usize
                {
                    text::put(
                        buf,
                        lx as u16,
                        l.ruler.y,
                        &label,
                        Style::new().fg(Color::White),
                    );
                    last_label_end = lx + lw;
                }
            }
        }
        t += step;
    }
}

fn draw_signal_row(
    buf: &mut Buffer,
    l: &Layout,
    app: &App,
    idx: usize,
    sig: &Signal,
    selected: bool,
    y: u16,
) {
    let row_bg = if selected {
        ROW_SEL_BG
    } else if (y - l.rows.y).is_multiple_of(2) {
        ROW_ALT
    } else {
        BG
    };
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
    let (t0, scale) = (app.t0, app.scale);
    let changes = &sig.changes;
    let n = changes.len();
    let mut i = changes.partition_point(|c| (c.t as f64) < t0);
    let mut value: Option<&[u8]> = if i > 0 {
        changes[i - 1].v.as_bits()
    } else {
        None
    };

    for col in 0..l.cols {
        let col_end = t0 + (col + 1) as f64 * scale;
        let mut transitions = 0u32;
        while i < n && (changes[i].t as f64) < col_end {
            value = changes[i].v.as_bits();
            transitions += 1;
            i += 1;
        }
        let (symbol, fg) = match transitions {
            0 => match summarize(value) {
                1 => (LEVEL_HIGH, HIGH),
                0 => (LEVEL_LOW, LOW),
                2 => (BUS_LINE, XCOL),
                _ => (BUS_LINE, ZCOL),
            },
            1 => match value {
                Some(bits) if bits.first() == Some(&1) => (EDGE_RISE, HIGH),
                Some(bits) if bits.first() == Some(&0) => (EDGE_FALL, LOW),
                _ => (VLINE, rail_color(value)),
            },
            _ => (VLINE, rail_color(value)),
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
    let (t0, scale) = (app.t0, app.scale);
    let changes = &sig.changes;
    let n = changes.len();
    let mut i = changes.partition_point(|c| (c.t as f64) < t0);

    for col in 0..l.cols {
        let col_end = t0 + (col + 1) as f64 * scale;
        let mut transition = false;
        while i < n && (changes[i].t as f64) < col_end {
            i += 1;
            transition = true;
        }
        let symbol = if transition { BUS_CROSS } else { BUS_LINE };
        text::set_cell(buf, l.rows.x + col as u16, y, symbol, BUS, row_bg);
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
                Style::new().fg(BUS_TEXT).bg(row_bg),
            );
        }
        j += 1;
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
    let (min, max) = if range.0.is_finite() && range.1.is_finite() {
        range
    } else {
        (0.0, 1.0)
    };
    let (t0, scale) = (app.t0, app.scale);
    let changes = &sig.changes;
    let n = changes.len();
    let mut i = changes.partition_point(|c| (c.t as f64) < t0);
    let mut value = if i > 0 {
        numeric_value(&changes[i - 1].v).unwrap_or(min)
    } else {
        min
    };
    if !value.is_finite() {
        value = min;
    }

    for col in 0..l.cols {
        let col_end = t0 + (col + 1) as f64 * scale;
        while i < n && (changes[i].t as f64) < col_end {
            value = numeric_value(&changes[i].v).unwrap_or(value);
            i += 1;
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
            ANALOG,
            row_bg,
        );
    }
}

/// Numeric view of a change value, used when rendering logic signals as analog.
fn numeric_value(value: &Value) -> Option<f64> {
    if let Some(real) = value.as_real() {
        return Some(real);
    }
    match value {
        Value::Bits(bits) => {
            if bits.len() > 64 || bits.iter().any(|&b| b >= 2) {
                return None;
            }
            let mut v: u64 = 0;
            for &b in bits.iter().rev() {
                v = (v << 1) | b as u64;
            }
            Some(v as f64)
        }
        _ => None,
    }
}

fn summarize(value: Option<&[u8]>) -> u8 {
    let Some(bits) = value else { return 2 };
    if bits.contains(&2) {
        2
    } else if bits.contains(&3) {
        3
    } else if bits.iter().all(|&b| b == 0) {
        0
    } else {
        1
    }
}

fn rail_color(value: Option<&[u8]>) -> Color {
    match summarize(value) {
        1 => HIGH,
        0 => LOW,
        2 => XCOL,
        _ => ZCOL,
    }
}

fn segment_text(value: &Value, radix: waveform::Radix) -> String {
    match value {
        Value::Bits(bits) => waveform::fmt_bits(bits, radix),
        Value::Real(real) => waveform::fmt_real(*real),
        Value::Str(s) => s.clone(),
    }
}

fn draw_range(buf: &mut Buffer, l: &Layout, app: &App) {
    let Some((a, b)) = app.range else { return };
    if l.cols == 0 || l.rows_h == 0 {
        return;
    }
    let c0 = (((a as f64 - app.t0) / app.scale).floor() as i64).clamp(0, l.cols as i64 - 1);
    let c1 = (((b as f64 - app.t0) / app.scale).ceil() as i64).clamp(0, l.cols as i64);
    for y in l.rows.y..l.rows.bottom() {
        for col in c0..c1 {
            if let Some(cell) = buf.cell_mut((l.rows.x + col as u16, y)) {
                cell.set_bg(RANGE_BG);
            }
        }
    }
}

fn draw_cursor(buf: &mut Buffer, l: &Layout, app: &App) {
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
            cell.set_fg(CURSOR);
            cell.set_bg(bg);
        }
    }
}

fn draw_vscroll(buf: &mut Buffer, l: &Layout, app: &App) {
    let total = app.rows_len();
    let visible = l.rows_h;
    if total <= visible {
        return;
    }
    let thumb = ((visible as f64 / total as f64) * visible as f64).max(1.0) as usize;
    let top = ((app.row_scroll as f64 / total as f64) * visible as f64) as usize;
    for row in 0..visible {
        let symbol = if row >= top && row < top + thumb {
            SCROLL_THUMB
        } else {
            VLINE
        };
        text::set_cell(buf, l.vscroll_x, l.rows.y + row as u16, symbol, DIM, BG);
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
    let Some(wf) = &app.wf else { return };
    if l.cols == 0 {
        return;
    }
    let (thumb_x, thumb_w) =
        hscroll_thumb(l.cols, app.scale, app.t0, wf.start as f64, wf.end as f64);
    for col in 0..l.cols {
        let on_thumb = col >= thumb_x && col < thumb_x + thumb_w;
        let symbol = if on_thumb { SCROLL_THUMB } else { SCROLL_TRACK };
        let fg = if on_thumb { ACCENT } else { DIM };
        text::set_cell(
            buf,
            l.rows.x + col as u16,
            l.hscroll.y,
            symbol,
            fg,
            TOOLBAR_BG,
        );
    }
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
