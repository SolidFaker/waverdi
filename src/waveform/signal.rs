use super::value::{fmt_bits, fmt_real, fmt_unknown, Radix, Value};
use super::Ticks;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SigKind {
    Bits,
    Real,
    Str,
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

    /// If the signal has a value change exactly at `t`, return the previous
    /// and new values.
    pub fn change_at(&self, t: Ticks) -> Option<(Option<&Value>, &Value)> {
        let i = self.changes.binary_search_by_key(&t, |c| c.t).ok()?;
        let previous = i.checked_sub(1).map(|j| &self.changes[j].v);
        Some((previous, &self.changes[i].v))
    }

    /// Render an `old→new` transition when the cursor sits on an edge.
    pub fn display_change(&self, t: Ticks, radix: Radix) -> Option<String> {
        let (previous, new) = self.change_at(t)?;
        let previous = previous?;
        Some(format!(
            "{}→{}",
            format_value(previous, radix),
            format_value(new, radix)
        ))
    }

    pub fn display_value(&self, t: Ticks, radix: Radix) -> String {
        match self.kind {
            SigKind::Bits => match self.value_at(t) {
                Some(Value::Bits(bits)) => fmt_bits(bits, radix),
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
    match value {
        Value::Bits(bits) => fmt_bits(bits, radix),
        Value::Real(real) => fmt_real(*real),
        Value::Str(s) => s.clone(),
    }
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
    fn change_at_reports_transitions() {
        let mut s = sig(SigKind::Bits, 4);
        s.changes.push(Change {
            t: 5,
            v: Value::Bits(vec![0, 0, 0, 1]),
        });
        s.changes.push(Change {
            t: 9,
            v: Value::Bits(vec![0, 1, 0, 1]),
        });
        assert_eq!(s.display_change(9, Radix::Hex), Some("h8→ha".to_string()));
        assert_eq!(
            s.display_change(9, Radix::Bin),
            Some("b1000→b1010".to_string())
        );
        assert_eq!(s.display_change(5, Radix::Hex), None);
        assert_eq!(s.display_change(7, Radix::Hex), None);
    }
}
