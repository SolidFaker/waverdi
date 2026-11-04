use std::collections::HashSet;

use super::db::RtlDb;
use super::model::{reference_base, Location, SignalTrace};

impl RtlDb {
    /// Declaration / driver / load lines of a signal in a module.
    pub fn trace(&self, module: &str, signal: &str) -> Option<SignalTrace> {
        let m = self.modules.get(module)?;
        let mut trace = SignalTrace::default();
        let at = |line: usize| Location {
            file: m.file.clone(),
            line,
        };
        trace.decl = m
            .signal(signal)
            .or_else(|| m.signal(signal.split('.').next().unwrap_or(signal)))
            .map(|decl| at(decl.line));
        for assign in &m.assigns {
            if trace_matches(&assign.lhs, signal) {
                trace.drivers.push(at(assign.line));
            }
            for (name, line) in &assign.uses {
                if trace_matches(name, signal) {
                    trace.loads.push(at(*line));
                }
            }
        }
        for block in &m.always {
            for (name, line) in &block.drivers {
                if trace_matches(name, signal) {
                    trace.drivers.push(at(*line));
                }
            }
            for (name, line) in block.uses.iter().chain(&block.sensitivity) {
                if trace_matches(name, signal) {
                    trace.loads.push(at(*line));
                }
            }
        }
        trace.drivers.sort();
        trace.drivers.dedup();
        let driver_lines: HashSet<usize> = trace.drivers.iter().map(|loc| loc.line).collect();
        trace.loads.retain(|loc| !driver_lines.contains(&loc.line));
        trace.loads.sort();
        trace.loads.dedup();
        Some(trace)
    }

    pub fn trace_at_scope(&self, scope: &[String], signal: &str) -> Option<(String, SignalTrace)> {
        let (owner, def) = self.signal_owner(scope)?;
        let suffix = &scope[owner.len()..];
        let reference = if suffix.first().is_some_and(|root| def.declares(root)) {
            let mut parts = suffix.to_vec();
            parts.push(signal.to_string());
            parts.join(".")
        } else {
            signal.to_string()
        };
        self.trace(&def.name, &reference)
            .map(|trace| (def.name.clone(), trace))
    }
}

fn trace_matches(candidate: &str, wanted: &str) -> bool {
    let candidate = reference_base(candidate);
    let wanted = reference_base(wanted);
    candidate == wanted
        || (!wanted.contains('.') && candidate.split('.').next() == Some(wanted.as_str()))
}
