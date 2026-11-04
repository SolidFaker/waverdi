use super::{Signal, Ticks, Value};

pub const MAX_DIFFERENCES: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comparison {
    pub total: usize,
    pub differences: Vec<Difference>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Difference {
    pub time: Ticks,
    pub left: Option<Value>,
    pub right: Option<Value>,
}

pub fn compare_signals(left: &Signal, right: &Signal, start: Ticks, end: Ticks) -> Comparison {
    let mut result = Comparison::default();
    if start > end {
        return result;
    }
    let (mut left_index, mut right_index) = (0usize, 0usize);
    let mut time = start;
    let mut was_different = false;
    let mut previous: (Option<&Value>, Option<&Value>) = (None, None);
    loop {
        while left
            .changes
            .get(left_index)
            .is_some_and(|change| change.t <= time)
        {
            left_index += 1;
        }
        while right
            .changes
            .get(right_index)
            .is_some_and(|change| change.t <= time)
        {
            right_index += 1;
        }
        let left_value = left_index
            .checked_sub(1)
            .map(|index| &left.changes[index].v);
        let right_value = right_index
            .checked_sub(1)
            .map(|index| &right.changes[index].v);
        let different = !values_equal(left_value, right_value);
        if different
            && (!was_different
                || !values_equal(previous.0, left_value)
                || !values_equal(previous.1, right_value))
        {
            result.total += 1;
            if result.differences.len() < MAX_DIFFERENCES {
                result.differences.push(Difference {
                    time,
                    left: left_value.cloned(),
                    right: right_value.cloned(),
                });
            }
        }
        previous = (left_value, right_value);
        was_different = different;
        let next_left = left.changes.get(left_index).map(|change| change.t);
        let next_right = right.changes.get(right_index).map(|change| change.t);
        let next = match (next_left, next_right) {
            (Some(left), Some(right)) => left.min(right),
            (Some(time), None) | (None, Some(time)) => time,
            (None, None) => break,
        };
        if next > end {
            break;
        }
        time = next;
    }
    result
}

fn values_equal(left: Option<&Value>, right: Option<&Value>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(Value::Real(left)), Some(Value::Real(right))) => left.to_bits() == right.to_bits(),
        (Some(left), Some(right))
            if matches!(left, Value::Small(..) | Value::Bits(_))
                && matches!(right, Value::Small(..) | Value::Bits(_)) =>
        {
            left.bits_len() == right.bits_len()
                && (0..left.bits_len()).all(|index| left.bit(index) == right.bit(index))
        }
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::{Change, SigKind, SigState};
    use std::sync::Arc;

    fn signal(changes: &[(u64, u8)]) -> Signal {
        Signal {
            name: "clk".to_string(),
            bits: 1,
            var_type: "wire".to_string(),
            dir: String::new(),
            scope: vec!["top".to_string()],
            kind: SigKind::Bits,
            changes: Arc::new(
                changes
                    .iter()
                    .map(|&(time, value)| Change {
                        t: time,
                        v: Value::compact(vec![value]),
                    })
                    .collect(),
            ),
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            parent: None,
            members: Vec::new(),
            state: SigState::Ready,
        }
    }

    #[test]
    fn compares_values_at_the_union_of_change_times() {
        let left = signal(&[(5, 0), (10, 1), (20, 0), (30, 1)]);
        let right = signal(&[(0, 1), (10, 0), (20, 0), (30, 0)]);
        let comparison = compare_signals(&left, &right, 0, 20);
        assert_eq!(comparison.total, 3);
        assert_eq!(
            comparison
                .differences
                .iter()
                .map(|difference| difference.time)
                .collect::<Vec<_>>(),
            vec![0, 5, 10]
        );
        assert_eq!(comparison.differences[0].left, None);
        assert_eq!(
            comparison.differences[0].right,
            Some(Value::compact(vec![1]))
        );
    }

    #[test]
    fn redundant_change_times_do_not_create_differences() {
        let left = signal(&[(0, 0), (5, 0), (10, 1)]);
        let right = signal(&[(0, 0), (10, 1)]);
        assert_eq!(compare_signals(&left, &right, 0, 20).total, 0);
    }

    #[test]
    fn logic_values_compare_independently_of_their_storage() {
        let mut left = signal(&[(0, 1)]);
        let mut right = signal(&[(0, 1)]);
        Arc::make_mut(&mut left.changes)[0].v = Value::Small(1, 1);
        Arc::make_mut(&mut right.changes)[0].v = Value::Bits(vec![1]);
        assert_eq!(compare_signals(&left, &right, 0, 1).total, 0);
        Arc::make_mut(&mut left.changes)[0].v = Value::Real(f64::NAN);
        Arc::make_mut(&mut right.changes)[0].v = Value::Real(f64::NAN);
        assert_eq!(compare_signals(&left, &right, 0, 1).total, 0);
    }
}
