use super::{Capabilities, DeferredDecode, DumpSource, ReadResult};
use crate::waveform::Waveform;
use std::path::Path;
use std::sync::Arc;

/// Lazy FSDB source backed by an open Verdi FFR session.
pub(super) struct FsdbSource {
    hierarchy: Option<Waveform>,
    warnings: Vec<String>,
    session: crate::fsdb::FsdbSession,
}

impl FsdbSource {
    pub(super) fn open(path: &Path, progress: crate::dump::Progress) -> Result<Self, String> {
        let (out, session) = crate::fsdb::parse_fsdb_lazy(path, progress)?;
        Ok(Self {
            hierarchy: Some(out.wf),
            warnings: out.warnings,
            session,
        })
    }
}

impl DumpSource for FsdbSource {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            lazy: true,
            directions: self.session.has_directions(),
        }
    }

    fn var_count(&self) -> usize {
        self.session.var_count()
    }

    fn take_hierarchy(&mut self) -> Waveform {
        self.hierarchy.take().expect("hierarchy is taken once")
    }

    fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    fn read_signal(&mut self, index: usize) -> ReadResult {
        self.session.read_signal(index)
    }

    fn read_signals(&mut self, indexes: &[usize]) -> Vec<ReadResult> {
        self.session.read_signals(indexes)
    }

    fn read_signals_deferred(&mut self, indexes: &[usize]) -> Vec<DeferredDecode> {
        let names: Vec<String> = indexes
            .iter()
            .map(|&index| self.session.signal_name(index).to_string())
            .collect();
        self.session
            .read_signals_raw(indexes)
            .into_iter()
            .zip(names)
            .map(|((raw, mut warnings), name)| {
                DeferredDecode::new(move || {
                    let changes = crate::fsdb::decode_raw(raw, &name, &mut warnings);
                    (Arc::new(changes), warnings)
                })
            })
            .collect()
    }
}
