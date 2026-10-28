use super::{Capabilities, DumpSource, ReadResult};
use crate::dump::ParseOut;
use crate::waveform::Waveform;
use std::sync::Arc;

/// Eager sources already carry their values in the hierarchy and do not use
/// the request pipeline.
pub(super) struct EagerSource {
    hierarchy: Option<Waveform>,
    warnings: Vec<String>,
}

impl EagerSource {
    pub(super) fn new(out: ParseOut) -> Self {
        Self {
            hierarchy: Some(out.wf),
            warnings: out.warnings,
        }
    }
}

impl DumpSource for EagerSource {
    fn is_lazy(&self) -> bool {
        false
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            lazy: false,
            directions: false,
        }
    }

    fn var_count(&self) -> usize {
        self.hierarchy
            .as_ref()
            .map(|wf| wf.signals.len())
            .unwrap_or_default()
    }

    fn take_hierarchy(&mut self) -> Waveform {
        self.hierarchy.take().expect("hierarchy is taken once")
    }

    fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    fn read_signal(&mut self, _index: usize) -> ReadResult {
        (Arc::new(Vec::new()), Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eager_source_keeps_its_values_in_the_hierarchy() {
        let path = std::path::Path::new("waveform/counter.vcd");
        let out =
            crate::dump::parse_with_progress(path, &mut |_| true).expect("counter.vcd parses");
        let signals = out.wf.signals.len();
        let mut source = EagerSource::new(out);

        assert!(!source.is_lazy());
        assert_eq!(
            source.capabilities(),
            Capabilities {
                lazy: false,
                directions: false,
            }
        );
        assert_eq!(source.var_count(), signals);
        let hierarchy = source.take_hierarchy();
        assert_eq!(hierarchy.signals.len(), signals);
        assert!(hierarchy
            .signals
            .iter()
            .any(|signal| !signal.changes.is_empty()));
    }
}
