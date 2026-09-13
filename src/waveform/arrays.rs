//! Virtual signals for unpacked arrays.
//!
//! Dumps store every element of an unpacked array as its own variable
//! (`mem[0][7:0]`, `mem[1][7:0]`, ...). This module groups those elements
//! into a tree of signals linked through `Signal::parent`:
//!
//! * a parent signal prints the whole array as `{0, 1, 2, 3}`;
//! * multi-dimensional arrays nest one brace level per dimension,
//!   `{{0, 1, 2}, {2, 3, 4}, {1, 2, 3}}`;
//! * expanding a node in the Signal List reveals the next dimension.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{fmt_real, fmt_value, Change, Radix, SigKind, SigState, Signal, Ticks, Value};

impl super::Waveform {
    /// Group per-element signals of unpacked arrays and add the synthesized
    /// parent signals. Leaves keep their indices in `signals`; new signals are
    /// appended, so existing indices stay valid.
    pub fn build_arrays(&mut self) {
        let mut groups: BTreeMap<(Vec<String>, String), BTreeMap<Vec<i64>, usize>> =
            BTreeMap::new();
        for (index, signal) in self.signals.iter().enumerate() {
            if signal.parent.is_some() {
                continue;
            }
            let Some((base, indices)) = split_element_name(&signal.name) else {
                continue;
            };
            groups
                .entry((signal.scope.clone(), base.to_string()))
                .or_default()
                .insert(indices, index);
        }

        for ((scope, base), members) in groups {
            let depth = members.keys().next().map(Vec::len).unwrap_or(0);
            if depth == 0 || members.keys().any(|key| key.len() != depth) {
                continue;
            }
            self.build_array_tree(&scope, &base, members);
        }
    }

    /// Build one array: leaves are the dump signals, every level above them is
    /// synthesized from its children.
    fn build_array_tree(
        &mut self,
        scope: &[String],
        base: &str,
        leaves: BTreeMap<Vec<i64>, usize>,
    ) {
        let depth = leaves.keys().next().map(Vec::len).unwrap_or(0);
        let mut level: BTreeMap<Vec<i64>, usize> = leaves;
        for prefix_len in (0..depth).rev() {
            let mut keys: BTreeMap<usize, Vec<i64>> = BTreeMap::new();
            let mut parents: BTreeMap<Vec<i64>, Vec<usize>> = BTreeMap::new();
            for (key, &index) in &level {
                keys.insert(index, key.clone());
                parents
                    .entry(key[..prefix_len].to_vec())
                    .or_default()
                    .push(index);
            }
            let mut next: BTreeMap<Vec<i64>, usize> = BTreeMap::new();
            for (key, mut children) in parents {
                children.sort_by_key(|child| keys[child].clone());
                let changes = array_changes(&self.signals, &children);
                let bits = children
                    .iter()
                    .map(|&child| self.signals[child].bits.max(1))
                    .sum::<u32>()
                    .max(1);
                let name = format!(
                    "{base}{}",
                    key.iter()
                        .map(|index| format!("[{index}]"))
                        .collect::<String>()
                );
                let index = self.signals.len();
                self.signals.push(Signal {
                    name,
                    bits,
                    var_type: "array".to_string(),
                    scope: scope.to_vec(),
                    kind: SigKind::Str,
                    changes,
                    min: f64::INFINITY,
                    max: f64::NEG_INFINITY,
                    parent: None,
                    members: Vec::new(),
                    state: SigState::Ready,
                });
                for &child in &children {
                    self.signals[child].parent = Some(index);
                }
                next.insert(key, index);
            }
            level = next;
        }
    }

    /// Synthesize one aggregate signal per dump scope: adding it to the
    /// waveform shows `{member, ...}`; expanding reveals the member signals.
    /// Struct variables and interface/module instances are dumped as scopes,
    /// so this is what makes them addable as a single aggregate row.
    pub fn build_scope_aggregates(&mut self) {
        // Scope path -> direct signals (array parents included, elements not).
        let mut by_scope: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
        for (index, signal) in self.signals.iter().enumerate() {
            if signal.parent.is_none() && signal.var_type != "aggregate" {
                by_scope
                    .entry(signal.scope.clone())
                    .or_default()
                    .push(index);
            }
        }
        let mut paths: Vec<Vec<String>> = Vec::new();
        let mut stack = vec![(self.tree.root, Vec::<String>::new())];
        while let Some((id, path)) = stack.pop() {
            if !path.is_empty() {
                paths.push(path.clone());
            }
            for &child in &self.tree.nodes[id].children {
                let mut child_path = path.clone();
                child_path.push(self.tree.nodes[child].name.clone());
                stack.push((child, child_path));
            }
        }
        for path in paths {
            let Some(members) = by_scope.get(&path) else {
                continue;
            };
            if members.is_empty() {
                continue;
            }
            let name = path.last().cloned().unwrap_or_default();
            let scope = path[..path.len() - 1].to_vec();
            if self
                .signals
                .iter()
                .any(|s| s.var_type == "aggregate" && s.name == name && s.scope == scope)
            {
                continue;
            }
            let bits = members
                .iter()
                .map(|&m| self.signals[m].bits.max(1))
                .sum::<u32>()
                .max(1);
            self.signals.push(Signal {
                name,
                bits,
                var_type: "aggregate".to_string(),
                scope,
                kind: SigKind::Str,
                changes: Vec::new(),
                min: f64::INFINITY,
                max: f64::NEG_INFINITY,
                parent: None,
                members: members.clone(),
                state: SigState::Lazy,
            });
        }
    }

    /// Aggregate signal synthesized for a dump scope path, if any.
    pub fn scope_aggregate(&self, scope: &[String]) -> Option<usize> {
        let (name, parent) = scope.split_last()?;
        self.signals.iter().position(|signal| {
            signal.var_type == "aggregate" && signal.name == *name && signal.scope == parent
        })
    }

    /// Child signal indices for every synthesized array signal.
    #[cfg_attr(not(fsdb_sdk), allow(dead_code))]
    pub(crate) fn array_children(&self) -> Vec<Vec<usize>> {
        let mut children = vec![Vec::new(); self.signals.len()];
        for (index, signal) in self.signals.iter().enumerate() {
            if let Some(parent) = signal.parent {
                children[parent].push(index);
            }
            if !signal.members.is_empty() {
                children[index].extend(signal.members.iter().copied());
            }
        }
        children
    }

    /// Children shown when a signal row is expanded: array elements, bit
    /// chunks and scope-aggregate members.
    pub fn children(&self, index: usize) -> Vec<usize> {
        let mut out: Vec<usize> = self
            .signals
            .iter()
            .enumerate()
            .filter(|(_, signal)| signal.parent == Some(index))
            .map(|(child, _)| child)
            .collect();
        if let Some(signal) = self.signals.get(index) {
            out.extend(signal.members.iter().copied());
        }
        out
    }

    /// Signal indices whose values must be loaded to build `index`: the
    /// element signals of an array subtree, or the signal itself.
    #[cfg_attr(not(fsdb_sdk), allow(dead_code))]
    pub(crate) fn value_leaves(&self, index: usize) -> Vec<usize> {
        let children = self.array_children();
        let mut out = Vec::new();
        let mut stack = vec![index];
        while let Some(node) = stack.pop() {
            if children[node].is_empty() {
                out.push(node);
            } else {
                stack.extend(children[node].iter().copied());
            }
        }
        out.sort_unstable();
        out
    }

    /// Recompute one synthesized signal (array or scope aggregate) when all
    /// of its children are loaded. Returns true when it became `Ready`.
    pub(crate) fn recompute_aggregate(&mut self, index: usize) -> bool {
        let children = self.array_children();
        let Some(list) = children.get(index) else {
            return false;
        };
        if list.is_empty() {
            return false;
        }
        if !list
            .iter()
            .all(|&child| self.signals[child].state == SigState::Ready)
        {
            return false;
        }
        let changes = array_changes(&self.signals, list);
        self.signals[index].changes = changes;
        self.signals[index].state = SigState::Ready;
        true
    }

    /// Recompute the brace text of every array or aggregate whose element
    /// values are all present. Used after lazily loading elements. Returns
    /// the indices that were recomputed.
    #[cfg_attr(not(fsdb_sdk), allow(dead_code))]
    pub(crate) fn recompute_ready_arrays(&mut self) -> Vec<usize> {
        let children = self.array_children();
        let synthesized: Vec<usize> = (0..self.signals.len())
            .filter(|&index| matches!(self.signals[index].var_type.as_str(), "array" | "aggregate"))
            .collect();
        let mut updated = Vec::new();
        for index in synthesized {
            if self.signals[index].state == SigState::Ready {
                continue;
            }
            let ready = children[index]
                .iter()
                .all(|&child| self.signals[child].state == SigState::Ready);
            if !ready {
                continue;
            }
            let changes = array_changes(&self.signals, &children[index]);
            self.signals[index].changes = changes;
            self.signals[index].state = SigState::Ready;
            updated.push(index);
        }
        updated
    }

    /// Re-format the brace text of every array/aggregate signal after radix
    /// changes. An override applies to the elements it contains; a leaf
    /// override only affects that element (and the parents that embed it).
    pub fn rebuild_array_texts(&mut self, radix: &HashMap<usize, Radix>) {
        let roots: Vec<usize> = (0..self.signals.len())
            .filter(|&index| {
                matches!(self.signals[index].var_type.as_str(), "array" | "aggregate")
                    && self.signals[index].parent.is_none()
            })
            .collect();
        let mut children: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (index, signal) in self.signals.iter().enumerate() {
            if let Some(parent) = signal.parent {
                children.entry(parent).or_default().push(index);
            }
            if !signal.members.is_empty() {
                children
                    .entry(index)
                    .or_default()
                    .extend(signal.members.iter().copied());
            }
        }
        for root in roots {
            self.rebuild_array_node(root, None, radix, &children);
        }
    }

    fn rebuild_array_node(
        &mut self,
        index: usize,
        inherited: Option<Radix>,
        radix: &HashMap<usize, Radix>,
        children: &BTreeMap<usize, Vec<usize>>,
    ) {
        let current = radix.get(&index).copied().or(inherited);
        let Some(list) = children.get(&index).cloned() else {
            return;
        };
        if list.is_empty() {
            return;
        }
        for &child in &list {
            if matches!(self.signals[child].var_type.as_str(), "array" | "aggregate") {
                self.rebuild_array_node(child, current, radix, children);
            }
        }
        let mut times: BTreeSet<Ticks> = BTreeSet::new();
        for &child in &list {
            for change in &self.signals[child].changes {
                times.insert(change.t);
            }
        }
        let mut changes: Vec<Change> = Vec::new();
        for t in times {
            let parts: Vec<String> = list
                .iter()
                .map(|&child| {
                    let signal = &self.signals[child];
                    element_text_radix(signal, t, radix.get(&child).copied().or(current))
                })
                .collect();
            let value = Value::Str(format!("{{{}}}", parts.join(", ")));
            if changes
                .last()
                .map(|change| change.v == value)
                .unwrap_or(false)
            {
                continue;
            }
            changes.push(Change { t, v: value });
        }
        self.signals[index].changes = changes;
    }
}

/// `mem[0][7:0]` -> `("mem", [0])`; `arr[1][2]` -> `("arr", [1, 2])`.
/// A name without array indices (`count[7:0]`) is not an array element.
fn split_element_name(name: &str) -> Option<(&str, Vec<i64>)> {
    let open = name.find('[')?;
    let base = &name[..open];
    if base.is_empty()
        || !base
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    {
        return None;
    }
    let mut rest = &name[open..];
    let mut indices = Vec::new();
    while !rest.is_empty() {
        let inner = rest.strip_prefix('[')?;
        let close = inner.find(']')?;
        let text = inner[..close].trim();
        if text.contains(':') {
            // The packed range may only be the final group.
            rest = &inner[close + 1..];
            if !rest.is_empty() {
                return None;
            }
            break;
        }
        indices.push(text.parse::<i64>().ok()?);
        rest = &inner[close + 1..];
    }
    (!indices.is_empty()).then_some((base, indices))
}

/// `{child, child, ...}` value changes over the union of the children's
/// change times.
fn array_changes(signals: &[Signal], children: &[usize]) -> Vec<Change> {
    let mut times: BTreeSet<Ticks> = BTreeSet::new();
    for &child in children {
        for change in &signals[child].changes {
            times.insert(change.t);
        }
    }
    let mut changes: Vec<Change> = Vec::new();
    for t in times {
        let parts: Vec<String> = children
            .iter()
            .map(|&child| element_text(&signals[child], t))
            .collect();
        let value = Value::Str(format!("{{{}}}", parts.join(", ")));
        if changes
            .last()
            .map(|change| change.v == value)
            .unwrap_or(false)
        {
            continue;
        }
        changes.push(Change { t, v: value });
    }
    changes
}

/// Text of one element at time `t`: the bit vector in `radix` (decimal by
/// default, unknown values as `x`), the real number, the string, or the nested
/// braces of a sub-array.
fn element_text_radix(signal: &Signal, t: Ticks, radix: Option<Radix>) -> String {
    match (signal.kind, signal.value_at(t)) {
        (SigKind::Bits, Some(value @ (Value::Small(..) | Value::Bits(_)))) => {
            if value.has_unknown() {
                "x".to_string()
            } else {
                // Hex is the default radix for arrays, like for buses.
                let radix = radix.unwrap_or(Radix::Hex);
                let text = fmt_value(value, radix);
                match radix {
                    // Drop the `b`/`o`/`d`/`h` prefix and pad zeros so braces
                    // stay compact.
                    Radix::Ascii => text,
                    _ => {
                        let digits: String = text.chars().skip(1).collect();
                        let trimmed = digits.trim_start_matches('0');
                        if trimmed.is_empty() {
                            "0".to_string()
                        } else {
                            trimmed.to_string()
                        }
                    }
                }
            }
        }
        (SigKind::Real, Some(Value::Real(real))) => fmt_real(*real),
        (SigKind::Str, Some(Value::Str(text))) => text.clone(),
        _ => "x".to_string(),
    }
}

fn element_text(signal: &Signal, t: Ticks) -> String {
    element_text_radix(signal, t, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::{ScopeTree, TimeScale, Waveform};

    fn leaf(name: &str, bits: &[u8], t: Ticks) -> Signal {
        Signal {
            name: name.to_string(),
            bits: bits.len() as u32,
            var_type: "wire".to_string(),
            scope: vec!["tb".to_string()],
            kind: SigKind::Bits,
            changes: vec![Change {
                t,
                v: Value::compact(bits.to_vec()),
            }],
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            parent: None,
            members: Vec::new(),
            state: SigState::Ready,
        }
    }

    fn waveform(signals: Vec<Signal>) -> Waveform {
        Waveform {
            ts: TimeScale::default(),
            start: 0,
            end: 10,
            signals,
            tree: ScopeTree::new(),
        }
    }

    #[test]
    fn scope_aggregates_group_members_with_brace_values() {
        let mut wf = waveform(vec![leaf("run", &[1], 0), leaf("pc", &[0, 1, 0, 1], 0)]);
        let tb = wf
            .tree
            .add_scope(wf.tree.root, "tb".to_string(), "m".to_string());
        let fetch = wf
            .tree
            .add_scope(tb, "clk_fetch".to_string(), "fetch_t".to_string());
        wf.tree.nodes[fetch].signals = vec![0, 1];
        for signal in &mut wf.signals {
            signal.scope = vec!["tb".to_string(), "clk_fetch".to_string()];
        }
        wf.build_scope_aggregates();
        let aggregate = wf
            .scope_aggregate(&["tb".to_string(), "clk_fetch".to_string()])
            .expect("aggregate");
        assert_eq!(wf.signals[aggregate].var_type, "aggregate");
        assert_eq!(wf.signals[aggregate].members, vec![0, 1]);
        // Members must be ready before the aggregate computes.
        for index in 0..2 {
            wf.signals[index].state = SigState::Lazy;
        }
        assert!(!wf.recompute_aggregate(aggregate));
        for index in 0..2 {
            wf.signals[index].state = SigState::Ready;
        }
        assert!(wf.recompute_aggregate(aggregate));
        match wf.signals[aggregate].changes.last().map(|c| &c.v) {
            Some(Value::Str(text)) => assert!(text.starts_with('{'), "{text}"),
            other => panic!("expected brace value, got {other:?}"),
        }
    }

    #[test]
    fn groups_one_dimensional_arrays() {
        let mut wf = waveform(vec![
            leaf("mem[0][7:0]", &[0, 0, 0, 0, 0, 0, 0, 0], 0),
            leaf("mem[1][7:0]", &[0, 1, 0, 0, 0, 0, 0, 0], 0),
        ]);
        wf.build_arrays();
        assert_eq!(wf.signals.len(), 3);
        let root = &wf.signals[2];
        assert_eq!(root.name, "mem");
        assert_eq!(root.kind, SigKind::Str);
        assert_eq!(root.value_at(0), Some(&Value::Str("{0, 2}".to_string())));
        assert_eq!(wf.signals[0].parent, Some(2));
        assert_eq!(wf.signals[1].parent, Some(2));
    }

    #[test]
    fn groups_multi_dimensional_arrays_level_by_level() {
        // Bits are stored LSB first: value 1 -> [1, 0], value 2 -> [0, 1].
        let mut wf = waveform(vec![
            leaf("arr[0][0][7:0]", &[0, 0, 0, 0, 0, 0, 0, 0], 0),
            leaf("arr[0][1][7:0]", &[1, 0, 0, 0, 0, 0, 0, 0], 0),
            leaf("arr[1][0][7:0]", &[0, 1, 0, 0, 0, 0, 0, 0], 0),
            leaf("arr[1][1][7:0]", &[1, 1, 0, 0, 0, 0, 0, 0], 0),
        ]);
        wf.build_arrays();
        assert_eq!(wf.signals.len(), 7);
        let sub0 = wf
            .signals
            .iter()
            .position(|signal| signal.name == "arr[0]")
            .unwrap();
        let sub1 = wf
            .signals
            .iter()
            .position(|signal| signal.name == "arr[1]")
            .unwrap();
        let root = wf
            .signals
            .iter()
            .position(|signal| signal.name == "arr")
            .unwrap();
        assert_eq!(
            wf.signals[sub0].value_at(0),
            Some(&Value::Str("{0, 1}".into()))
        );
        assert_eq!(
            wf.signals[sub1].value_at(0),
            Some(&Value::Str("{2, 3}".into()))
        );
        assert_eq!(
            wf.signals[root].value_at(0),
            Some(&Value::Str("{{0, 1}, {2, 3}}".to_string()))
        );
        assert_eq!(wf.signals[0].parent, Some(sub0));
        assert_eq!(wf.signals[2].parent, Some(sub1));
        assert_eq!(wf.signals[sub0].parent, Some(root));
        assert_eq!(wf.signals[sub1].parent, Some(root));
    }

    #[test]
    fn radix_overrides_reformat_the_brace_text() {
        // Element 0 = 10, element 1 = 16 (bits are LSB first).
        let mut wf = waveform(vec![
            leaf("arr[0][7:0]", &[0, 1, 0, 1, 0, 0, 0, 0], 0),
            leaf("arr[1][7:0]", &[0, 0, 0, 0, 1, 0, 0, 0], 0),
        ]);
        wf.build_arrays();
        let root = wf.signals.iter().position(|s| s.name == "arr").unwrap();
        // Hex is the default radix for arrays.
        assert_eq!(
            wf.signals[root].value_at(0),
            Some(&Value::Str("{a, 10}".to_string()))
        );
        let mut radix = HashMap::new();
        radix.insert(root, Radix::Hex);
        wf.rebuild_array_texts(&radix);
        assert_eq!(
            wf.signals[root].value_at(0),
            Some(&Value::Str("{a, 10}".to_string()))
        );
        // A leaf override only reformats that element.
        radix.clear();
        radix.insert(0, Radix::Bin);
        wf.rebuild_array_texts(&radix);
        assert_eq!(
            wf.signals[root].value_at(0),
            Some(&Value::Str("{1010, 10}".to_string()))
        );
    }

    #[test]
    fn single_element_arrays_are_grouped_too() {
        let mut wf = waveform(vec![
            leaf("count[7:0]", &[0, 0, 0, 0, 0, 0, 0, 1], 0),
            leaf("one[0][7:0]", &[0, 0, 0, 0, 0, 0, 0, 1], 0),
        ]);
        wf.build_arrays();
        // The plain bus stays a leaf; the 1-element array gains a parent.
        assert_eq!(wf.signals.len(), 3);
        let root = wf
            .signals
            .iter()
            .position(|signal| signal.name == "one")
            .expect("array root");
        assert_eq!(wf.signals[root].var_type, "array");
        assert_eq!(wf.signals[1].parent, Some(root));
        assert!(!wf.signals.iter().any(|signal| signal.name == "count"));
    }

    #[test]
    fn brace_values_track_changes() {
        let mut wf = waveform(vec![
            Signal {
                changes: vec![
                    Change {
                        t: 0,
                        v: Value::Bits(vec![0, 0]),
                    },
                    Change {
                        t: 5,
                        v: Value::Bits(vec![1, 0]),
                    },
                ],
                ..leaf("a[0][1:0]", &[0, 0], 0)
            },
            leaf("a[1][1:0]", &[1, 0], 0),
        ]);
        wf.build_arrays();
        let root = wf.signals.iter().position(|s| s.name == "a").unwrap();
        let values: Vec<String> = wf.signals[root]
            .changes
            .iter()
            .map(|change| match &change.v {
                Value::Str(text) => text.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(values, vec!["{0, 1}", "{1, 1}"]);
    }
}
