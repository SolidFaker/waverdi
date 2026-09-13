use super::value::{fmt_real, fmt_unknown, Radix, Value};
use super::Ticks;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SigKind {
    Bits,
    Real,
    Str,
}

/// Whether a signal's value changes are materialized in memory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SigState {
    /// Changes are loaded (or the dump has none for this signal).
    Ready,
    /// Changes exist in the dump backend but have not been read yet.
    Lazy,
    /// A background load for this signal is in flight.
    Loading,
}

#[derive(Clone, Debug)]
pub struct Change {
    pub t: Ticks,
    pub v: Value,
}

#[derive(Clone)]
pub struct Signal {
    pub name: String,
    pub bits: u32,
    pub var_type: String,
    pub scope: Vec<String>,
    pub kind: SigKind,
    pub changes: Vec<Change>,
    pub min: f64,
    pub max: f64,
    /// Signal this one was expanded from (bit/chunk of a bus), if any.
    pub parent: Option<usize>,
    /// Members of a synthesized scope aggregate (struct/interface/instance).
    /// Kept separate from `parent` so a signal can be an array element and a
    /// scope member at the same time.
    pub members: Vec<usize>,
    /// Lazy loading state; always `Ready` for fully parsed dumps.
    pub state: SigState,
}

impl Signal {
    pub fn full_name(&self) -> String {
        if self.scope.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.scope.join("."), self.name)
        }
    }

    pub fn value_at(&self, t: Ticks) -> Option<&Value> {
        let i = self.changes.partition_point(|c| c.t <= t);
        if i == 0 {
            None
        } else {
            Some(&self.changes[i - 1].v)
        }
    }

    /// If the signal has a value change inside the time range `[from, to)`,
    /// return the previous and new values. Used with the column the TUI
    /// actually draws the cursor in, so hitting "near" an edge counts.
    pub fn change_in(&self, from: f64, to: f64) -> Option<(Option<&Value>, &Value)> {
        let i = self.changes.partition_point(|c| (c.t as f64) < from);
        let change = self.changes.get(i)?;
        if (change.t as f64) >= to {
            return None;
        }
        let previous = i.checked_sub(1).map(|j| &self.changes[j].v);
        Some((previous, &change.v))
    }

    /// Render an `old→new` transition when the cursor column sits on an edge.
    pub fn display_change_in(&self, from: f64, to: f64, radix: Radix) -> Option<String> {
        let (previous, new) = self.change_in(from, to)?;
        let previous = previous?;
        Some(format!(
            "{}→{}",
            format_value(previous, radix),
            format_value(new, radix)
        ))
    }

    pub fn display_value(&self, t: Ticks, radix: Radix) -> String {
        if self.state != SigState::Ready {
            return "…".to_string();
        }
        match self.kind {
            SigKind::Bits => match self.value_at(t) {
                Some(value) if matches!(value, Value::Small(..) | Value::Bits(_)) => {
                    super::value::fmt_value(value, radix)
                }
                _ => fmt_unknown(self.bits as usize, radix),
            },
            SigKind::Real => match self.value_at(t) {
                Some(Value::Real(v)) => fmt_real(*v),
                _ => "x".to_string(),
            },
            SigKind::Str => match self.value_at(t) {
                Some(Value::Str(s)) => s.clone(),
                _ => "x".to_string(),
            },
        }
    }
}

fn format_value(value: &Value, radix: Radix) -> String {
    super::value::fmt_value(value, radix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(kind: SigKind, bits: u32) -> Signal {
        Signal {
            name: "s".into(),
            bits,
            var_type: String::new(),
            scope: vec![],
            kind,
            changes: vec![],
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            parent: None,
            members: Vec::new(),
            state: SigState::Ready,
        }
    }

    #[test]
    fn unknown_value_uses_radix_width() {
        let s = sig(SigKind::Bits, 8);
        assert_eq!(s.display_value(10, Radix::Hex), "hxx");
        assert_eq!(s.display_value(10, Radix::Bin), "bxxxxxxxx");
    }

    #[test]
    fn value_at_returns_latest_change() {
        let mut s = sig(SigKind::Real, 64);
        s.changes.push(Change {
            t: 5,
            v: Value::Real(1.0),
        });
        s.changes.push(Change {
            t: 9,
            v: Value::Real(2.0),
        });
        assert_eq!(s.value_at(4), None);
        assert_eq!(s.value_at(5).unwrap().as_real(), Some(1.0));
        assert_eq!(s.value_at(100).unwrap().as_real(), Some(2.0));
    }

    #[test]
    fn change_in_matches_display_column() {
        let mut s = sig(SigKind::Bits, 4);
        s.changes.push(Change {
            t: 5,
            v: Value::Bits(vec![0, 0, 0, 1]),
        });
        s.changes.push(Change {
            t: 9,
            v: Value::Bits(vec![0, 1, 0, 1]),
        });
        // A column covering ticks 8..10 sees the change at 9.
        assert_eq!(
            s.display_change_in(8.0, 10.0, Radix::Hex),
            Some("h8→ha".to_string())
        );
        assert_eq!(
            s.display_change_in(9.0, 11.0, Radix::Hex),
            Some("h8→ha".to_string())
        );
        // Column before/after the edge: no transition.
        assert_eq!(s.display_change_in(6.0, 8.0, Radix::Hex), None);
        assert_eq!(s.display_change_in(10.0, 12.0, Radix::Hex), None);
        // The first change has no previous value.
        assert_eq!(s.display_change_in(4.0, 6.0, Radix::Hex), None);
    }
}
