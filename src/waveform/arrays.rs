//! Virtual signals for unpacked arrays.
//!
//! Dumps store every element of an unpacked array as its own variable
//! (`mem[0][7:0]`, `mem[1][7:0]`, ...). This module groups those elements
//! into a tree of signals linked through `Signal::parent`, with the children
//! recorded in `Signal::members`:
//!
//! * a parent signal prints the whole array as `{0, 1, 2, 3}`;
//! * multi-dimensional arrays nest one brace level per dimension,
//!   `{{0, 1, 2}, {2, 3, 4}, {1, 2, 3}}`;
//! * expanding a node in the Signal List reveals the next dimension.
//!
//! The brace text is never stored: synthesized signals keep an empty change
//! list and join their members at the requested time.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap};
use std::sync::Arc;

use super::{fmt_real, fmt_value, Radix, SigKind, SigState, Signal, Ticks, Value};

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
                    dir: String::new(),
                    scope: scope.to_vec(),
                    kind: SigKind::Str,
                    // Brace values are synthesized from the members on
                    // demand; nothing is pre-joined or stored here.
                    changes: Arc::new(Vec::new()),
                    min: f64::INFINITY,
                    max: f64::NEG_INFINITY,
                    parent: None,
                    members: children.clone(),
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
        let mut paths: Vec<(usize, Vec<String>)> = Vec::new();
        let mut stack = vec![(self.tree.root, Vec::<String>::new())];
        while let Some((id, path)) = stack.pop() {
            if !path.is_empty() {
                paths.push((id, path.clone()));
            }
            for &child in &self.tree.nodes[id].children {
                let mut child_path = path.clone();
                child_path.push(self.tree.nodes[child].name.clone());
                stack.push((child, child_path));
            }
        }
        for (_, path) in &paths {
            let Some(members) = by_scope.get(path) else {
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
            self.push_aggregate(name, scope, members.clone());
        }
        // Interface ports are recorded as references to the connected
        // instance (`module = tb/.if_inst/.mst`, no signals). Give each one
        // an aggregate named after the *port* (`ROM_IF`) that shares the
        // members of the connected scope, instead of showing the interface's
        // own instance/modport name.
        for (node, path) in &paths {
            if !self.tree.nodes[*node].signals.is_empty() {
                continue;
            }
            let module = self.tree.nodes[*node].module.clone();
            if !module.contains("/.") {
                continue;
            }
            let mut target = Vec::new();
            let mut cursor = self.tree.root;
            for step in module.split("/.") {
                let Some(child) = self.tree.nodes[cursor]
                    .children
                    .iter()
                    .copied()
                    .find(|&child| self.tree.nodes[child].name == step)
                else {
                    break;
                };
                cursor = child;
                target.push(step.to_string());
            }
            if target.is_empty() || target == *path {
                continue;
            }
            let members = match self.scope_aggregate(&target) {
                Some(aggregate) => self.signals[aggregate].members.clone(),
                None => self
                    .signals
                    .iter()
                    .enumerate()
                    .filter(|(_, signal)| {
                        signal.parent.is_none()
                            && signal.var_type != "aggregate"
                            && signal.scope == target
                    })
                    .map(|(index, _)| index)
                    .collect(),
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
            self.push_aggregate(name, scope, members);
        }
    }

    fn push_aggregate(&mut self, name: String, scope: Vec<String>, members: Vec<usize>) {
        let bits = members
            .iter()
            .map(|&member| self.signals[member].bits.max(1))
            .sum::<u32>()
            .max(1);
        self.signals.push(Signal {
            name,
            bits,
            var_type: "aggregate".to_string(),
            dir: String::new(),
            scope,
            kind: SigKind::Str,
            changes: Arc::new(Vec::new()),
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            parent: None,
            members,
            state: SigState::Lazy,
        });
    }

    /// Aggregate signal synthesized for a dump scope path, if any.
    pub fn scope_aggregate(&self, scope: &[String]) -> Option<usize> {
        let (name, parent) = scope.split_last()?;
        self.signals.iter().position(|signal| {
            signal.var_type == "aggregate" && signal.name == *name && signal.scope == parent
        })
    }

    /// True for the synthesized brace signals (unpacked arrays and
    /// aggregates); their values are computed from the members, never stored.
    pub fn is_synthesized(&self, index: usize) -> bool {
        self.signals.get(index).is_some_and(is_synthesized)
    }

    /// Children shown when a signal row is expanded: array elements, bit
    /// chunks and scope-aggregate members.
    pub fn children(&self, index: usize) -> Vec<usize> {
        let Some(signal) = self.signals.get(index) else {
            return Vec::new();
        };
        if !signal.members.is_empty() {
            return signal.members.clone();
        }
        self.signals
            .iter()
            .enumerate()
            .filter(|(_, child)| child.parent == Some(index))
            .map(|(child, _)| child)
            .collect()
    }

    /// Signal indices whose values must be loaded to build `index`: the
    /// element signals of an array subtree, or the signal itself.
    pub(crate) fn value_leaves(&self, index: usize) -> Vec<usize> {
        let Some(signal) = self.signals.get(index) else {
            return Vec::new();
        };
        if !is_synthesized(signal) {
            return vec![index];
        }
        let mut out = Vec::new();
        let mut stack = vec![index];
        while let Some(node) = stack.pop() {
            let signal = &self.signals[node];
            if is_synthesized(signal) && !signal.members.is_empty() {
                stack.extend(signal.members.iter().copied());
            } else {
                out.push(node);
            }
        }
        out.sort_unstable();
        out
    }

    /// Recompute one synthesized signal (array or scope aggregate) when all
    /// of its children are loaded. Returns true when it became `Ready`.
    pub(crate) fn recompute_aggregate(&mut self, index: usize) -> bool {
        let Some(signal) = self.signals.get(index) else {
            return false;
        };
        if signal.members.is_empty()
            || !signal
                .members
                .iter()
                .all(|&child| self.signals[child].state == SigState::Ready)
        {
            return false;
        }
        self.signals[index].changes = Arc::new(Vec::new());
        self.signals[index].state = SigState::Ready;
        self.set_cached_value_times(index, None);
        true
    }

    /// Recompute the brace text of every array or aggregate whose element
    /// values are all present. Used after lazily loading elements. Returns
    /// the indices that were recomputed.
    pub(crate) fn recompute_ready_arrays(&mut self) -> Vec<usize> {
        let mut updated = Vec::new();
        for index in 0..self.signals.len() {
            let signal = &self.signals[index];
            if !is_synthesized(signal) || signal.state == SigState::Ready {
                continue;
            }
            let ready = signal
                .members
                .iter()
                .all(|&child| self.signals[child].state == SigState::Ready);
            if !ready {
                continue;
            }
            self.signals[index].changes = Arc::new(Vec::new());
            self.signals[index].state = SigState::Ready;
            self.set_cached_value_times(index, None);
            updated.push(index);
        }
        updated
    }

    /// Record radix overrides without touching transition-time caches.
    pub fn rebuild_array_texts(&mut self, radix: &HashMap<usize, Radix>) -> Vec<usize> {
        self.radix = radix.clone();
        (0..self.signals.len())
            .filter(|&index| {
                is_synthesized(&self.signals[index]) && self.signals[index].state == SigState::Ready
            })
            .collect()
    }

    /// Value of signal `index` at time `t`. Plain signals read their stored
    /// changes; synthesized array/aggregate signals join the members' values
    /// at `t` into `{...}` instead of keeping a pre-joined change list. Like
    /// the old stored brace changes they are unknown before the first member
    /// change and while the members are still loading.
    pub fn value_at(&self, index: usize, t: Ticks) -> Option<Value> {
        let signal = self.signals.get(index)?;
        if !is_synthesized(signal) {
            return signal.value_at(t).cloned();
        }
        if signal.state != SigState::Ready || t < self.first_time(index)? {
            return None;
        }
        Some(Value::Str(self.element_text(index, t, None)))
    }

    /// Unknown kind of signal `index` at `t` with x precedence: `Some(2)`
    /// when any bit is x, `Some(3)` when only z bits are unknown, `None` when
    /// every bit is known. Real/string values carry no bits. Synthesized rows
    /// classify every member, because their brace text is joined on demand
    /// and collapses each unknown member; a member without a value yet reads
    /// as unknown `x`, exactly like its brace text.
    pub fn unknown_kind_at(&self, index: usize, t: Ticks) -> Option<u8> {
        let signal = self.signals.get(index)?;
        if !is_synthesized(signal) {
            if signal.kind != SigKind::Bits {
                return None;
            }
            return match signal.value_at(t) {
                Some(value) => value.unknown_kind(),
                None => Some(2),
            };
        }
        let mut any_z = false;
        for &child in &signal.members {
            match self.unknown_kind_at(child, t) {
                Some(2) => return Some(2),
                Some(3) => any_z = true,
                _ => {}
            }
        }
        any_z.then_some(3)
    }

    /// Transition times of a signal. Synthesized signals merge their members'
    /// times once the signal is `Ready` (unloaded ones keep an empty list,
    /// like their old empty change list); the union is thinned with the same
    /// stride rule the old merged change list used (every change counts, one
    /// time is emitted at the first multiple of the stride), so the list
    /// never grows past [`crate::dump::MAX_AGGREGATE_CHANGES`] values for a
    /// huge interface.
    ///
    /// The cached `Arc` is handed out directly: the draw path walks up to
    /// `MAX_AGGREGATE_CHANGES` times per row per frame and must not copy the
    /// slice first.
    pub fn value_times(&self, index: usize) -> Arc<[Ticks]> {
        let Some(signal) = self.signals.get(index) else {
            return Arc::from(Vec::new());
        };
        if !is_synthesized(signal) {
            return Arc::from(
                signal
                    .changes
                    .iter()
                    .map(|change| change.t)
                    .collect::<Vec<_>>(),
            );
        }
        if signal.state != SigState::Ready {
            return Arc::from(Vec::new());
        }
        if let Some(times) = self.cached_value_times(index) {
            return times;
        }
        self.compute_value_times(index)
    }

    /// Number of transitions of a signal, used by width and search code.
    pub fn value_len(&self, index: usize) -> usize {
        match self.signals.get(index) {
            Some(signal) if !is_synthesized(signal) => signal.changes.len(),
            _ => self.value_times(index).len(),
        }
    }

    /// Display text of signal `index` at `t` with `radix`: synthesized brace
    /// signals are joined from their members at `t`.
    pub fn display_value(&self, index: usize, t: Ticks, radix: Radix) -> String {
        let Some(signal) = self.signals.get(index) else {
            return "x".to_string();
        };
        if signal.state != SigState::Ready {
            return "…".to_string();
        }
        if is_synthesized(signal) {
            match self.value_at(index, t) {
                Some(value) => fmt_value(&value, radix),
                None => "x".to_string(),
            }
        } else {
            signal.display_value(t, radix)
        }
    }

    /// Render an `old→new` transition when the cursor column sits on an edge
    /// of a synthesized signal (plain signals use [`Signal::display_change_in`]).
    pub fn display_change_in(
        &self,
        index: usize,
        from: f64,
        to: f64,
        radix: Radix,
    ) -> Option<String> {
        let signal = self.signals.get(index)?;
        if !is_synthesized(signal) {
            return signal.display_change_in(from, to, radix);
        }
        let times = self.value_times(index);
        let i = times.partition_point(|&t| (t as f64) < from);
        let &t = times.get(i)?;
        if (t as f64) >= to {
            return None;
        }
        let previous = i
            .checked_sub(1)
            .and_then(|j| self.value_at(index, times[j]))?;
        let new = self.value_at(index, t)?;
        Some(format!(
            "{}→{}",
            fmt_value(&previous, radix),
            fmt_value(&new, radix)
        ))
    }

    /// Fill the cached transition times of every ready synthesized signal.
    /// Used when a whole waveform is installed; steady-state updates cache
    /// only the signals that turned ready.
    pub fn refresh_value_times(&mut self) {
        for index in 0..self.signals.len() {
            self.cache_value_times(index);
        }
    }

    /// Fill the cached transition times of one ready synthesized signal. The
    /// merge walks all members once per change batch instead of once per draw
    /// call, so drawing stays a cache hit.
    pub fn cache_value_times(&mut self, index: usize) {
        if !self.signals.get(index).is_some_and(is_synthesized)
            || self.signals[index].state != SigState::Ready
            || self.cached_value_times(index).is_some()
        {
            return;
        }
        let times = self.compute_value_times(index);
        self.set_cached_value_times(index, Some(times));
    }

    fn cached_value_times(&self, index: usize) -> Option<Arc<[Ticks]>> {
        self.value_times_cache.get(index).and_then(Option::clone)
    }

    fn set_cached_value_times(&mut self, index: usize, times: Option<Arc<[Ticks]>>) {
        if self.value_times_cache.len() <= index {
            self.value_times_cache.resize(index + 1, None);
        }
        self.value_times_cache[index] = times;
    }

    /// Brace text of a synthesized signal: its members at `t`, each formatted
    /// with its own radix or one inherited from the parent. Nested arrays
    /// nest one brace level per dimension, like the old stored texts.
    fn element_text(&self, index: usize, t: Ticks, inherited: Option<Radix>) -> String {
        let signal = &self.signals[index];
        if !is_synthesized(signal) {
            return element_text_radix(signal, t, inherited);
        }
        let current = self.radix.get(&index).copied().or(inherited);
        let parts: Vec<String> = signal
            .members
            .iter()
            .map(|&child| self.element_text(child, t, self.radix.get(&child).copied().or(current)))
            .collect();
        format!("{{{}}}", parts.join(", "))
    }

    /// Merged member times of a synthesized signal, with nested synthesized
    /// members resolved through their caches.
    fn compute_value_times(&self, index: usize) -> Arc<[Ticks]> {
        let sources: Vec<TimeSource> = self.signals[index]
            .members
            .iter()
            .map(|&child| {
                if let Some(times) = self.cached_value_times(child) {
                    TimeSource::Merged(times)
                } else if is_synthesized(&self.signals[child]) {
                    TimeSource::Merged(self.compute_value_times(child))
                } else {
                    TimeSource::Changes(self.signals[child].changes.as_slice())
                }
            })
            .collect();
        Arc::from(merged_times(&sources))
    }

    /// First transition of a signal: the minimum over its members, without
    /// merging their whole change lists.
    fn first_time(&self, index: usize) -> Option<Ticks> {
        let signal = &self.signals[index];
        if !is_synthesized(signal) {
            return signal.changes.first().map(|change| change.t);
        }
        if let Some(times) = self.cached_value_times(index) {
            return times.first().copied();
        }
        signal
            .members
            .iter()
            .filter_map(|&child| self.first_time(child))
            .min()
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

/// True for the signals whose `{...}` values are synthesized on demand.
fn is_synthesized(signal: &Signal) -> bool {
    matches!(signal.var_type.as_str(), "array" | "aggregate")
}

enum TimeSource<'a> {
    Changes(&'a [super::Change]),
    Merged(Arc<[Ticks]>),
}

impl TimeSource<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Changes(changes) => changes.len(),
            Self::Merged(times) => times.len(),
        }
    }

    fn time_at(&self, index: usize) -> Option<Ticks> {
        match self {
            Self::Changes(changes) => changes.get(index).map(|change| change.t),
            Self::Merged(times) => times.get(index).copied(),
        }
    }
}

fn merged_times(sources: &[TimeSource<'_>]) -> Vec<Ticks> {
    let total: usize = sources.iter().map(TimeSource::len).sum();
    let stride = total.div_ceil(crate::dump::MAX_AGGREGATE_CHANGES).max(1);
    let mut cursors = vec![0usize; sources.len()];
    let mut heap = BinaryHeap::new();
    for (index, source) in sources.iter().enumerate() {
        if let Some(time) = source.time_at(0) {
            heap.push(Reverse((time, index)));
        }
    }
    let mut applied = 0usize;
    let mut times = Vec::new();
    while let Some(entry) = heap.peek() {
        let time = entry.0 .0;
        let mut dirty = false;
        while heap.peek().is_some_and(|entry| entry.0 .0 == time) {
            let Reverse((_, source)) = heap.pop().unwrap();
            while sources[source].time_at(cursors[source]) == Some(time) {
                cursors[source] += 1;
                applied += 1;
                dirty = true;
            }
            if let Some(next) = sources[source].time_at(cursors[source]) {
                heap.push(Reverse((next, source)));
            }
        }
        if dirty && (times.is_empty() || applied.is_multiple_of(stride)) {
            times.push(time);
        }
    }
    times
}

/// Text of one element at time `t`: the bit vector in `radix` (decimal by
/// default, unknown values as `x`), the real number or the string. Nested
/// array levels are joined by the parent's `element_text` before this is
/// reached.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::{Change, ScopeTree, TimeScale, Waveform};

    fn leaf(name: &str, bits: &[u8], t: Ticks) -> Signal {
        Signal {
            name: name.to_string(),
            bits: bits.len() as u32,
            var_type: "wire".to_string(),
            dir: String::new(),
            scope: vec!["tb".to_string()],
            kind: SigKind::Bits,
            changes: Arc::new(vec![Change {
                t,
                v: Value::compact(bits.to_vec()),
            }]),
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
            radix: HashMap::new(),
            value_times_cache: Vec::new(),
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
        // The aggregate stores no brace text; it is joined on demand.
        assert!(wf.signals[aggregate].changes.is_empty());
        match wf.value_at(aggregate, 0) {
            Some(Value::Str(text)) => assert!(text.starts_with('{'), "{text}"),
            other => panic!("expected brace value, got {other:?}"),
        }
    }

    #[test]
    fn interface_port_aggregates_use_the_port_name() {
        let mut wf = waveform(vec![leaf("adr", &[0, 1], 0)]);
        let tb = wf
            .tree
            .add_scope(wf.tree.root, "tb".to_string(), String::new());
        let rom = wf
            .tree
            .add_scope(tb, "rom_if".to_string(), "prt_riscv_rom_if".to_string());
        wf.tree.nodes[rom].signals = vec![0];
        wf.signals[0].scope = vec!["tb".to_string(), "rom_if".to_string()];
        let cpu = wf
            .tree
            .add_scope(tb, "u_cpu".to_string(), "prt_riscv_cpu".to_string());
        // Interface port recorded as a reference to the connected instance.
        wf.tree
            .add_scope(cpu, "ROM_IF".to_string(), "tb/.rom_if/.mst".to_string());
        wf.build_scope_aggregates();

        let port = wf
            .scope_aggregate(&["tb".to_string(), "u_cpu".to_string(), "ROM_IF".to_string()])
            .expect("port aggregate");
        assert_eq!(wf.signals[port].name, "ROM_IF");
        assert_eq!(
            wf.signals[port].scope,
            vec!["tb".to_string(), "u_cpu".to_string()]
        );
        assert_eq!(wf.signals[port].members, vec![0]);
        assert!(wf.recompute_aggregate(port));
        // The connected instance keeps its own aggregate as well.
        assert!(wf
            .scope_aggregate(&["tb".to_string(), "rom_if".to_string()])
            .is_some());
    }

    #[test]
    fn rebuild_brace_texts_merges_children_in_time_order() {
        let mut wf = waveform(vec![
            leaf("mem[0][3:0]", &[1, 0, 0, 0], 0),
            leaf("mem[1][3:0]", &[0, 1, 0, 0], 0),
        ]);
        wf.build_arrays();
        // Element 0 changes at t=0 and t=2, element 1 at t=1 and t=2.
        wf.signals[0].changes = Arc::new(vec![
            Change {
                t: 0,
                v: Value::compact(vec![1, 0, 0, 0]),
            },
            Change {
                t: 2,
                v: Value::compact(vec![0, 1, 0, 0]),
            },
        ]);
        wf.signals[1].changes = Arc::new(vec![
            Change {
                t: 1,
                v: Value::compact(vec![1, 1, 0, 0]),
            },
            Change {
                t: 2,
                v: Value::compact(vec![0, 0, 1, 0]),
            },
        ]);
        let updated = wf.rebuild_array_texts(&std::collections::HashMap::new());
        assert!(updated.contains(&2), "{updated:?}");
        // The merged transition times of both elements, in time order.
        assert_eq!(&wf.value_times(2)[..], &[0, 1, 2][..]);
        let values: Vec<String> = wf
            .value_times(2)
            .iter()
            .copied()
            .map(|t| match wf.value_at(2, t) {
                Some(Value::Str(text)) => text,
                other => panic!("not text: {other:?}"),
            })
            .collect();
        // Both elements changing at t=2 produce one value, not two.
        assert_eq!(
            values,
            vec![
                "{1, x}".to_string(),
                "{1, 3}".to_string(),
                "{2, 4}".to_string()
            ]
        );
        assert!(wf.signals[2].changes.is_empty(), "brace texts stay lazy");
    }

    /// Synthesized signals keep no change list at all: the members are the
    /// only stored state and `value_at`/`value_times` join them on demand.
    #[test]
    fn array_values_are_synthesized_lazily() {
        let mut wf = waveform(vec![
            Signal {
                changes: Arc::new(vec![
                    Change {
                        t: 0,
                        v: Value::compact(vec![0, 0]),
                    },
                    Change {
                        t: 5,
                        v: Value::compact(vec![1, 0]),
                    },
                ]),
                ..leaf("a[0][1:0]", &[0, 0], 0)
            },
            leaf("a[1][1:0]", &[1, 0], 0),
        ]);
        wf.build_arrays();
        let root = wf.signals.iter().position(|s| s.name == "a").unwrap();
        assert!(wf.signals[root].changes.is_empty());
        assert_eq!(wf.value_at(root, 0), Some(Value::Str("{0, 1}".into())));
        assert_eq!(wf.value_at(root, 5), Some(Value::Str("{1, 1}".into())));
        assert_eq!(&wf.value_times(root)[..], &[0, 5][..]);
        assert_eq!(wf.value_len(root), 2);
        wf.refresh_value_times();
        let cached = wf.value_times(root);
        wf.rebuild_array_texts(&HashMap::new());
        assert!(Arc::ptr_eq(&cached, &wf.value_times(root)));
        assert_eq!(&cached[..], &[0, 5][..]);
    }

    /// The cached `Arc` must be shared, not copied, so the waveform pane can
    /// walk the merged times without reallocating them every frame.
    #[test]
    fn transition_times_are_shared_not_copied() {
        let mut wf = waveform(vec![
            leaf("a[0][1:0]", &[0, 0], 0),
            leaf("a[1][1:0]", &[1, 0], 0),
        ]);
        wf.build_arrays();
        let root = wf.signals.iter().position(|s| s.name == "a").unwrap();
        wf.refresh_value_times();
        let first = wf.value_times(root);
        let second = wf.value_times(root);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(&first[..], &[0][..]);
    }

    #[test]
    fn heap_merged_times_match_the_naive_reference() {
        fn reference(sources: &[TimeSource<'_>]) -> Vec<Ticks> {
            let total: usize = sources.iter().map(TimeSource::len).sum();
            let stride = total.div_ceil(crate::dump::MAX_AGGREGATE_CHANGES).max(1);
            let mut cursors = vec![0usize; sources.len()];
            let mut applied = 0usize;
            let mut times = Vec::new();
            loop {
                let next = sources
                    .iter()
                    .enumerate()
                    .filter_map(|(index, source)| source.time_at(cursors[index]))
                    .min();
                let Some(time) = next else { break };
                let mut dirty = false;
                for (index, source) in sources.iter().enumerate() {
                    while source.time_at(cursors[index]) == Some(time) {
                        cursors[index] += 1;
                        applied += 1;
                        dirty = true;
                    }
                }
                if dirty && (times.is_empty() || applied.is_multiple_of(stride)) {
                    times.push(time);
                }
            }
            times
        }

        let cases = vec![
            vec![vec![], vec![]],
            vec![vec![0, 0, 1, 5], vec![0, 1, 1, 2, 5, 9], vec![2, 2, 3]],
            (0..3)
                .map(|member| (0..70_000u64).map(|index| index * 3 + member).collect())
                .collect(),
        ];
        for raw in cases {
            let sources: Vec<TimeSource> = raw
                .into_iter()
                .map(|times| TimeSource::Merged(Arc::from(times)))
                .collect();
            assert_eq!(merged_times(&sources), reference(&sources));
        }
    }

    #[test]
    fn synthesized_values_are_unknown_before_the_first_member_change() {
        let mut wf = waveform(vec![
            leaf("a[0][1:0]", &[0, 0], 3),
            leaf("a[1][1:0]", &[1, 0], 7),
        ]);
        wf.build_arrays();
        let root = wf.signals.iter().position(|s| s.name == "a").unwrap();
        assert_eq!(wf.value_at(root, 2), None);
        assert_eq!(wf.value_at(root, 3), Some(Value::Str("{0, x}".into())));
        assert_eq!(wf.value_at(root, 7), Some(Value::Str("{0, 1}".into())));
    }

    #[test]
    fn groups_one_dimensional_arrays() {
        let mut wf = waveform(vec![
            leaf("mem[0][7:0]", &[0, 0, 0, 0, 0, 0, 0, 0], 0),
            leaf("mem[1][7:0]", &[0, 1, 0, 0, 0, 0, 0, 0], 0),
        ]);
        wf.build_arrays();
        assert_eq!(wf.signals.len(), 3);
        assert_eq!(wf.signals[2].name, "mem");
        assert_eq!(wf.signals[2].kind, SigKind::Str);
        assert_eq!(wf.value_at(2, 0), Some(Value::Str("{0, 2}".to_string())));
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
        assert_eq!(wf.value_at(sub0, 0), Some(Value::Str("{0, 1}".into())));
        assert_eq!(wf.value_at(sub1, 0), Some(Value::Str("{2, 3}".into())));
        assert_eq!(
            wf.value_at(root, 0),
            Some(Value::Str("{{0, 1}, {2, 3}}".to_string()))
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
            wf.value_at(root, 0),
            Some(Value::Str("{a, 10}".to_string()))
        );
        let mut radix = HashMap::new();
        radix.insert(root, Radix::Hex);
        wf.rebuild_array_texts(&radix);
        assert_eq!(
            wf.value_at(root, 0),
            Some(Value::Str("{a, 10}".to_string()))
        );
        // A leaf override only reformats that element.
        radix.clear();
        radix.insert(0, Radix::Bin);
        wf.rebuild_array_texts(&radix);
        assert_eq!(
            wf.value_at(root, 0),
            Some(Value::Str("{1010, 10}".to_string()))
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

    /// The Signal List (`display_value`) and the wave pane (`value_at`) read
    /// the same values for plain and synthesized signals when the cursor sits
    /// *between* two changes: the last change before the cursor must win, not
    /// `x`. Guards the lazy-aggregate rewrite, which moved the value column
    /// and bus sampling onto the shared helpers.
    #[test]
    fn display_matches_value_between_changes_for_plain_and_synthesized() {
        let mut wf = waveform(vec![
            Signal {
                changes: Arc::new(vec![
                    Change {
                        t: 0,
                        v: Value::compact(vec![0]),
                    },
                    Change {
                        t: 10,
                        v: Value::compact(vec![1]),
                    },
                ]),
                ..leaf("clk", &[0], 0)
            },
            Signal {
                changes: Arc::new(vec![
                    Change {
                        t: 0,
                        v: Value::compact(vec![0, 0, 0, 1, 0, 0, 0, 0]),
                    },
                    Change {
                        t: 20,
                        v: Value::compact(vec![0, 1, 0, 1, 0, 0, 0, 0]),
                    },
                ]),
                ..leaf("bus[7:0]", &[0; 8], 0)
            },
            leaf("mem[0][7:0]", &[0, 0, 0, 0, 0, 0, 0, 1], 0),
            Signal {
                changes: Arc::new(vec![
                    Change {
                        t: 0,
                        v: Value::compact(vec![0, 0, 0, 0, 0, 0, 1, 1]),
                    },
                    Change {
                        t: 20,
                        v: Value::compact(vec![0, 0, 0, 0, 0, 0, 0, 0]),
                    },
                ]),
                ..leaf("mem[1][7:0]", &[0, 0, 0, 0, 0, 0, 1, 1], 0)
            },
        ]);
        wf.build_arrays();
        let mem = wf.signals.iter().position(|s| s.name == "mem").unwrap();
        let cursor = 15;
        // Plain scalar: the change at t=10 still applies at t=15.
        assert_eq!(wf.value_at(0, cursor), Some(Value::Small(1, 1)));
        assert_eq!(wf.display_value(0, cursor, Radix::Hex), "h1");
        // Plain bus: the change at t=0 still applies before the one at t=20.
        let bus_value = wf.value_at(1, cursor).expect("bus value at cursor");
        assert_eq!(bus_value, Value::compact(vec![0, 0, 0, 1, 0, 0, 0, 0]));
        assert_eq!(
            wf.display_value(1, cursor, Radix::Hex),
            fmt_value(&bus_value, Radix::Hex)
        );
        // Synthesized array: the members are joined at the cursor, and the
        // merged transition times include the later member's change.
        let text = match wf.value_at(mem, cursor) {
            Some(Value::Str(text)) => text,
            other => panic!("expected brace value, got {other:?}"),
        };
        assert_eq!(text, "{80, c0}");
        assert_eq!(wf.display_value(mem, cursor, Radix::Hex), "{80, c0}");
        assert_eq!(&wf.value_times(mem)[..], &[0, 20][..]);
        // A later change moves both the joined value and the wave segment.
        assert_eq!(wf.display_value(mem, 20, Radix::Hex), "{80, 0}");
    }

    #[test]
    fn brace_values_track_changes() {
        let mut wf = waveform(vec![
            Signal {
                changes: Arc::new(vec![
                    Change {
                        t: 0,
                        v: Value::Bits(vec![0, 0]),
                    },
                    Change {
                        t: 5,
                        v: Value::Bits(vec![1, 0]),
                    },
                ]),
                ..leaf("a[0][1:0]", &[0, 0], 0)
            },
            leaf("a[1][1:0]", &[1, 0], 0),
        ]);
        wf.build_arrays();
        let root = wf.signals.iter().position(|s| s.name == "a").unwrap();
        let values: Vec<String> = wf
            .value_times(root)
            .iter()
            .copied()
            .map(|t| match wf.value_at(root, t) {
                Some(Value::Str(text)) => text,
                other => panic!("not text: {other:?}"),
            })
            .collect();
        assert_eq!(values, vec!["{0, 1}", "{1, 1}"]);
    }
}
