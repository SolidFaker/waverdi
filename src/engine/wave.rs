use crate::waveform::{self, Change, Ticks, Value};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WaveGlyph {
    VLine,
    LevelHigh,
    LevelLow,
    EdgeRise,
    EdgeFall,
    BusLine,
    BusCross,
    Half1,
    Half2,
    Half3,
    Half4,
    Half5,
    Half6,
    Half7,
    Half8,
}

impl WaveGlyph {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::VLine => "│",
            Self::LevelHigh => "▔",
            Self::LevelLow => "▁",
            Self::EdgeRise => "/",
            Self::EdgeFall => "\\",
            Self::BusLine => "─",
            Self::BusCross => "╳",
            Self::Half1 => "▁",
            Self::Half2 => "▂",
            Self::Half3 => "▃",
            Self::Half4 => "▄",
            Self::Half5 => "▅",
            Self::Half6 => "▆",
            Self::Half7 => "▇",
            Self::Half8 => "█",
        }
    }
}

pub(crate) const VLINE: WaveGlyph = WaveGlyph::VLine;
pub(crate) const LEVEL_HIGH: WaveGlyph = WaveGlyph::LevelHigh;
pub(crate) const LEVEL_LOW: WaveGlyph = WaveGlyph::LevelLow;
pub(crate) const EDGE_RISE: WaveGlyph = WaveGlyph::EdgeRise;
pub(crate) const EDGE_FALL: WaveGlyph = WaveGlyph::EdgeFall;
pub(crate) const BUS_LINE: WaveGlyph = WaveGlyph::BusLine;
pub(crate) const BUS_CROSS: WaveGlyph = WaveGlyph::BusCross;
const HALF: [WaveGlyph; 8] = [
    WaveGlyph::Half1,
    WaveGlyph::Half2,
    WaveGlyph::Half3,
    WaveGlyph::Half4,
    WaveGlyph::Half5,
    WaveGlyph::Half6,
    WaveGlyph::Half7,
    WaveGlyph::Half8,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WaveInk {
    High,
    Low,
    X,
    Z,
    Bus,
    Analog,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct WaveCell {
    pub(crate) glyph: WaveGlyph,
    pub(crate) ink: WaveInk,
}

pub(crate) const NO_RANGE: (f64, f64) = (f64::NAN, f64::NAN);

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
    #[allow(clippy::too_many_arguments)]
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

pub(crate) fn summary_ink(summary: u8) -> WaveInk {
    match summary {
        1 => WaveInk::High,
        0 => WaveInk::Low,
        2 => WaveInk::X,
        _ => WaveInk::Z,
    }
}

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

pub(crate) fn unknown_ink(kind: Option<u8>) -> WaveInk {
    match kind {
        Some(2) => WaveInk::X,
        Some(3) => WaveInk::Z,
        _ => WaveInk::Bus,
    }
}

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

pub(crate) fn sample_bit_row(
    changes: &[Change],
    t0: f64,
    scale: f64,
    cols: usize,
) -> Vec<WaveCell> {
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
            _ => WaveCell {
                glyph: VLINE,
                ink: summary_ink(summary),
            },
        });
    }
    cells
}

pub(crate) fn sample_bus_row(
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
            ink = ink_at(Some(i - 1));
        }
        let glyph = if i > before { BUS_CROSS } else { BUS_LINE };
        cells.push(WaveCell { glyph, ink });
    }
    cells
}

pub(crate) fn sample_analog_row(
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

pub(crate) enum BusTimes<'a> {
    Changes(&'a [Change]),
    Merged(&'a [Ticks]),
}

impl BusTimes<'_> {
    pub(crate) fn len(&self) -> usize {
        match self {
            BusTimes::Changes(changes) => changes.len(),
            BusTimes::Merged(times) => times.len(),
        }
    }

    pub(crate) fn time_at(&self, index: usize) -> Ticks {
        match self {
            BusTimes::Changes(changes) => changes[index].t,
            BusTimes::Merged(times) => times[index],
        }
    }
}

pub(crate) fn time_partition_point(times: &BusTimes<'_>, pred: impl Fn(Ticks) -> bool) -> usize {
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

fn numeric_value(value: &Value) -> Option<f64> {
    if let Some(real) = value.as_real() {
        return Some(real);
    }
    waveform::value_number(value).map(|v| v as f64)
}

pub(crate) fn summarize(value: Option<&Value>) -> u8 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_analog_row(
        changes: &[Change],
        t0: f64,
        scale: f64,
        cols: usize,
        min: f64,
        max: f64,
    ) -> Vec<WaveCell> {
        let cut = t0 - 0.5 * scale;
        let mut position = changes.partition_point(|change| (change.t as f64) < cut);
        let mut value = position
            .checked_sub(1)
            .and_then(|index| numeric_value(&changes[index].v))
            .filter(|value| value.is_finite())
            .unwrap_or(min);
        let mut cells = Vec::with_capacity(cols);
        for col in 0..cols {
            let end = t0 + (col as f64 + 0.5) * scale;
            let start = position;
            let end = changes.partition_point(|change| (change.t as f64) < end);
            position = end;
            if end > start {
                let stride = ((end - start) / 64).max(1);
                let mut extreme = value;
                let mut best = 0.0f64;
                let mut index = start;
                while index < end {
                    let candidate = numeric_value(&changes[index].v).unwrap_or(value);
                    let distance = (candidate - value).abs();
                    if distance > best {
                        best = distance;
                        extreme = candidate;
                    }
                    index += stride;
                }
                let last = numeric_value(&changes[end - 1].v).unwrap_or(value);
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
            cells.push(WaveCell {
                glyph: HALF[((level * 7.0).round() as usize).min(7)],
                ink: WaveInk::Analog,
            });
        }
        cells
    }

    #[test]
    fn analog_sampling_matches_the_reference_for_dense_and_non_finite_values() {
        let mut changes = vec![
            Change {
                t: 0,
                v: Value::Real(f64::NEG_INFINITY),
            },
            Change {
                t: 1,
                v: Value::Real(1.0),
            },
            Change {
                t: 2,
                v: Value::Real(f64::NAN),
            },
            Change {
                t: 3,
                v: Value::Bits(vec![1; 64]),
            },
            Change {
                t: 4,
                v: Value::Bits(vec![1; 1024]),
            },
            Change {
                t: 5,
                v: Value::Real(-0.5),
            },
            Change {
                t: 5,
                v: Value::Real(0.75),
            },
        ];
        for index in 0..10_000 {
            changes.push(Change {
                t: 6 + index / 17,
                v: Value::Real(if index % 2 == 0 { -1.0 } else { 1.0 }),
            });
        }
        for &(t0, scale, cols) in &[
            (0.0, 0.5, 40),
            (0.0, 20.0, 20),
            (-3.0, 2.5, 13),
            (4.0, 0.125, 80),
        ] {
            assert_eq!(
                sample_analog_row(&changes, t0, scale, cols, -1.0, 1.0),
                reference_analog_row(&changes, t0, scale, cols, -1.0, 1.0),
                "analog row differs at t0={t0} scale={scale} cols={cols}"
            );
        }
    }

    #[test]
    fn wave_cells_are_compact() {
        assert!(std::mem::size_of::<WaveCell>() <= 4);
    }
}
