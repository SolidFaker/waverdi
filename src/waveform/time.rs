use std::fmt;

pub const UNIT_SECS: [(&str, f64); 6] = [
    ("s", 1e0),
    ("ms", 1e-3),
    ("us", 1e-6),
    ("ns", 1e-9),
    ("ps", 1e-12),
    ("fs", 1e-15),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TimeScale {
    pub num: u32,
    pub unit: &'static str,
}

impl TimeScale {
    pub const DEFAULT: Self = Self { num: 1, unit: "ns" };

    /// Parse a VCD `$timescale` unit such as `b"ns"`.
    pub fn from_unit(num: u32, unit: &[u8]) -> Option<Self> {
        let unit = match unit {
            b"s" => "s",
            b"ms" => "ms",
            b"us" => "us",
            b"ns" => "ns",
            b"ps" => "ps",
            b"fs" => "fs",
            _ => return None,
        };
        Some(Self { num, unit })
    }

    pub fn secs_per_tick(&self) -> f64 {
        self.num as f64
            * UNIT_SECS
                .iter()
                .find(|(name, _)| *name == self.unit)
                .map(|(_, secs)| *secs)
                .unwrap_or(1e-9)
    }

    pub fn label(&self) -> String {
        format!("{}{}", self.num, self.unit)
    }
}

impl Default for TimeScale {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Display for TimeScale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

/// Render a tick count as a human readable time using the most fitting unit.
pub fn format_time(t: f64, ts: &TimeScale) -> String {
    let secs = t * ts.secs_per_tick();
    if secs == 0.0 {
        return "0".to_string();
    }
    let mut sel = UNIT_SECS.len() - 1;
    for (i, (_, secs_per_unit)) in UNIT_SECS.iter().enumerate() {
        // Allow a tiny rounding slack so e.g. 100 ticks of 10ps is shown as 1ns.
        if secs.abs() * (1.0 + 1e-12) >= *secs_per_unit {
            sel = i;
            break;
        }
    }
    let v = secs / UNIT_SECS[sel].1;
    let text = if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.3}");
        let t = s.trim_end_matches('0').trim_end_matches('.');
        if t.is_empty() || t == "-" {
            format!("{v:.2}")
        } else {
            t.to_string()
        }
    };
    format!("{}{}", text, UNIT_SECS[sel].0)
}

/// Pick a "nice" ruler step (1/2/5 * 10^n) of roughly `scale * 8` ticks.
pub fn nice_step(scale: f64, ts: &TimeScale) -> f64 {
    let spt = ts.secs_per_tick();
    let target = (scale * 8.0 * spt).max(1e-300);
    let k = 10f64.powf(target.log10().floor());
    let m = target / k;
    let m = if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    };
    m * k / spt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_scale_secs() {
        let ts = TimeScale { num: 1, unit: "ns" };
        assert_eq!(ts.secs_per_tick(), 1e-9);
        let ts10 = TimeScale {
            num: 10,
            unit: "ps",
        };
        assert_eq!(ts10.secs_per_tick(), 1e-11);
        assert_eq!(ts.label(), "1ns");
        assert_eq!(ts10.label(), "10ps");
    }

    #[test]
    fn from_unit_validates() {
        assert_eq!(
            TimeScale::from_unit(100, b"ps"),
            Some(TimeScale {
                num: 100,
                unit: "ps"
            })
        );
        assert_eq!(TimeScale::from_unit(1, b"xx"), None);
        assert_eq!(TimeScale::from_unit(1, b"NS"), None);
    }

    #[test]
    fn format_time_smoke() {
        let ns = TimeScale { num: 1, unit: "ns" };
        assert_eq!(format_time(0.0, &ns), "0");
        assert_eq!(format_time(5.0, &ns), "5ns");
        assert_eq!(format_time(1500.0, &ns), "1.5us");
        assert_eq!(format_time(250000.0, &ns), "250us");
        assert_eq!(format_time(1_000_000.0, &ns), "1ms");
        assert_eq!(
            format_time(
                100.0,
                &TimeScale {
                    num: 10,
                    unit: "ps"
                }
            ),
            "1ns"
        );
        assert_eq!(format_time(0.5, &TimeScale { num: 1, unit: "fs" }), "0.5fs");
    }

    #[test]
    fn nice_step_smoke() {
        let ns = TimeScale { num: 1, unit: "ns" };
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        assert!(close(nice_step(1000.0, &ns), 1.0e4));
        assert!(close(nice_step(500.0, &ns), 5.0e3));
        assert!(close(nice_step(150.0, &ns), 2.0e3));
    }
}
