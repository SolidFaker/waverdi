mod signal;
mod time;
mod tree;
mod value;

pub use signal::{Change, SigKind, Signal};
pub use time::{format_time, format_time_base, nice_step, TimeBase, TimeScale};
pub use tree::ScopeTree;
pub use value::{fmt_bits, fmt_real, Radix, Value};

pub type Ticks = u64;

pub struct Waveform {
    pub ts: TimeScale,
    pub start: Ticks,
    pub end: Ticks,
    pub signals: Vec<Signal>,
    pub tree: ScopeTree,
}

impl Waveform {
    pub fn total_ticks(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// One-line description used in status messages and `--list-signals`.
    pub fn summary(&self) -> String {
        format!(
            "{} signals, time range {} .. {} (timescale {})",
            self.signals.len(),
            format_time(self.start as f64, &self.ts),
            format_time(self.end as f64, &self.ts),
            self.ts.label()
        )
    }
}
