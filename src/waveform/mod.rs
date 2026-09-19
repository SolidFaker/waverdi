mod arrays;
mod signal;
mod time;
mod tree;
mod value;

pub use signal::{Change, SigKind, SigState, Signal};
pub use time::{format_time, format_time_base, nice_step, TimeBase, TimeScale};
pub use tree::ScopeTree;
pub use value::{fmt_real, fmt_value, value_number, Radix, Value};

use std::collections::HashMap;
use std::sync::Arc;

pub type Ticks = u64;

#[derive(Clone)]
pub struct Waveform {
    pub ts: TimeScale,
    pub start: Ticks,
    pub end: Ticks,
    pub signals: Vec<Signal>,
    pub tree: ScopeTree,
    /// Radix overrides of array elements, mirrored from the app so brace
    /// values can be synthesized with the same formatting as the old stored
    /// texts. Absent entries default to hex, inherited from the parent.
    pub radix: HashMap<usize, Radix>,
    /// Cached transition times of synthesized signals (`None` while invalid).
    /// Plain signals read their change list directly, so they need no entry.
    pub value_times_cache: Vec<Option<Arc<[Ticks]>>>,
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
