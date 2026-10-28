use std::collections::HashSet;

use super::db::RtlDb;
use super::model::{Location, SignalTrace};

impl RtlDb {
    /// Declaration / driver / load lines of a signal in a module.
    pub fn trace(&self, module: &str, signal: &str) -> Option<SignalTrace> {
        let m = self.modules.get(module)?;
        let mut trace = SignalTrace::default();
        let at = |line: usize| Location {
            file: m.file.clone(),
            line,
        };
        trace.decl = m.signal(signal).map(|decl| at(decl.line));
        for assign in &m.assigns {
            if assign.lhs == signal {
                trace.drivers.push(at(assign.line));
            }
            for (name, line) in &assign.uses {
                if name == signal {
                    trace.loads.push(at(*line));
                }
            }
        }
        for block in &m.always {
            for (name, line) in &block.drivers {
                if name == signal {
                    trace.drivers.push(at(*line));
                }
            }
            for (name, line) in block.uses.iter().chain(&block.sensitivity) {
                if name == signal {
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
}
