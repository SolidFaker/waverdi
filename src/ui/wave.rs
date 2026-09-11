use crate::app::App;
use crate::theme::*;
use crate::ui::layout::Layout;
use crate::ui::text;
use crate::waveform::{self, Signal, Value, Waveform};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Style};

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

    for row in 0..l.rows_h {
        let k = app.row_scroll + row;
        if k >= app.display.len() {
            break;
        }
        let idx = app.display[k];
        let sig = &wf.signals[idx];
        let selected = Some(k) == app.sel_row;
        draw_signal_row(buf, l, app, idx, sig, selected, l.rows.y + row as u16);
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
    match sig.kind {
        waveform::SigKind::Bits if sig.bits <= 1 => draw_bit_row(buf, l, app, sig, row_bg, y),
        waveform::SigKind::Bits | waveform::SigKind::Str => {
            draw_bus_row(buf, l, app, idx, sig, row_bg, y)
        }
        waveform::SigKind::Real => draw_analog_row(buf, l, app, sig, row_bg, y),
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

fn draw_analog_row(buf: &mut Buffer, l: &Layout, app: &App, sig: &Signal, row_bg: Color, y: u16) {
    let (t0, scale) = (app.t0, app.scale);
    let changes = &sig.changes;
    let n = changes.len();
    let mut i = changes.partition_point(|c| (c.t as f64) < t0);
    let mut value = if i > 0 {
        changes[i - 1].v.as_real().unwrap_or(sig.min)
    } else {
        sig.min
    };
    if !value.is_finite() {
        value = 0.0;
    }

    for col in 0..l.cols {
        let col_end = t0 + (col + 1) as f64 * scale;
        while i < n && (changes[i].t as f64) < col_end {
            value = changes[i].v.as_real().unwrap_or(value);
            i += 1;
        }
        let level = if sig.max == sig.min {
            0.5
        } else {
            ((value - sig.min) / (sig.max - sig.min)).clamp(0.0, 1.0)
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
    let total = app.display.len();
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

fn draw_hscroll(buf: &mut Buffer, l: &Layout, app: &App) {
    let Some(wf) = &app.wf else { return };
    if l.cols == 0 {
        return;
    }
    let start = wf.start as f64;
    let end = wf.end as f64;
    let total = (end - start).max(app.scale * l.cols as f64).max(1e-9);
    let frac = (l.cols as f64 / total).clamp(0.0, 1.0);
    let pos = ((app.t0 - start) / total).clamp(0.0, 1.0);
    let thumb_w = (frac * l.cols as f64).max(1.0) as usize;
    let thumb_x = ((pos * l.cols as f64) as usize).min(l.cols.saturating_sub(1));
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
