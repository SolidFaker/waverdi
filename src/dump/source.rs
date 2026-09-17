//! Format adapter used by the background loader.
//!
//! Every dump format is opened through [`DumpSource`]: the caller gets the
//! hierarchy first and pulls value changes per signal, so the loader, the
//! compute pool and the UI do not need to know which format is behind it.
//!
//! Laziness lives here too, on a spectrum:
//! - formats with random access (FSDB through the Verdi reader) read a signal
//!   from the file the moment it is requested ([`FsdbSource`]);
//! - sequential formats (VCD/FST) cannot seek, so they are parsed once and
//!   their values wait in the source until requested ([`EagerSource`]).
//!
//! Either way the loader sees the same interface: hierarchy, then values on
//! demand, with the shared change budget and decimation applied by the
//! parsers (`crate::dump::limit_changes`).

use crate::waveform::{Change, Waveform};
use std::path::Path;

/// One dump format as seen by the loader. Reads are serialized by the loader
/// (FFR cannot be used from two threads), so implementations may assume
/// `read_signal` is called from a single thread; the trait is deliberately
/// not `Send` so a source cannot be moved between reader threads.
pub trait DumpSource {
    /// Whether values still have to be fetched from the source on request.
    /// Eager sources deliver their values with the hierarchy and never need
    /// the request pipeline (there is nothing to gain and a clone to lose).
    fn is_lazy(&self) -> bool {
        true
    }
    /// Number of variables the source can serve.
    fn var_count(&self) -> usize;
    /// Take the parsed hierarchy (signals may be lazy). Called once.
    fn take_hierarchy(&mut self) -> Waveform;
    /// Take parser warnings collected while building the hierarchy. Called once.
    fn take_warnings(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// Read one signal's value changes on demand.
    fn read_signal(&mut self, index: usize) -> (Vec<Change>, Vec<String>);
}

/// Lazy FSDB source backed by an open Verdi FFR session.
#[cfg(fsdb_sdk)]
struct FsdbSource {
    hierarchy: Option<Waveform>,
    warnings: Vec<String>,
    session: crate::fsdb::FsdbSession,
}

#[cfg(fsdb_sdk)]
impl DumpSource for FsdbSource {
    fn var_count(&self) -> usize {
        self.session.var_count()
    }

    fn take_hierarchy(&mut self) -> Waveform {
        self.hierarchy.take().expect("hierarchy is taken once")
    }

    fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    fn read_signal(&mut self, index: usize) -> (Vec<Change>, Vec<String>) {
        self.session.read_signal(index)
    }
}

/// Eager source: formats without random access are parsed up front, values
/// and all. They do not use the request pipeline (`is_lazy` is false), so the
/// UI gets the complete waveform in one event exactly like before the adapter
/// layer existed - a second copy of every value would only cost time and
/// memory.
struct EagerSource {
    hierarchy: Option<Waveform>,
    warnings: Vec<String>,
}

impl DumpSource for EagerSource {
    fn is_lazy(&self) -> bool {
        false
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

    fn read_signal(&mut self, _index: usize) -> (Vec<Change>, Vec<String>) {
        // The loader never requests values from an eager source.
        (Vec::new(), Vec::new())
    }
}

/// Open `path` as a [`DumpSource`]. FSDB is read lazily through the Verdi FFR
/// session; every other format is parsed up front and reported as eager, so
/// the loader can skip the request pipeline for it.
pub(crate) fn open_source(
    path: &Path,
    progress: crate::dump::Progress,
) -> Result<Box<dyn DumpSource>, String> {
    #[cfg(fsdb_sdk)]
    if super::detect(path) == super::Format::Fsdb {
        let (out, session) = crate::fsdb::parse_fsdb_lazy(path, progress)?;
        let super::ParseOut { wf, warnings } = out;
        return Ok(Box::new(FsdbSource {
            hierarchy: Some(wf),
            warnings,
            session,
        }));
    }
    let out = super::parse_with_progress(path, progress)?;
    Ok(Box::new(EagerSource {
        hierarchy: Some(out.wf),
        warnings: out.warnings,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dump::ParseOut;
    use std::path::Path;

    /// The eager adapter hands the parsed dump over whole: it is not lazy, so
    /// the values stay in the hierarchy and the loader skips the request
    /// pipeline for it.
    #[test]
    fn eager_source_keeps_its_values_in_the_hierarchy() {
        let path = Path::new("waveform/counter.vcd");
        let ParseOut { wf, warnings } =
            super::super::parse_with_progress(path, &mut |_| true).expect("counter.vcd parses");
        let signals = wf.signals.len();
        let mut source = EagerSource {
            hierarchy: Some(wf),
            warnings,
        };

        assert!(!source.is_lazy());
        assert_eq!(source.var_count(), signals);
        let hierarchy = source.take_hierarchy();
        assert_eq!(hierarchy.signals.len(), signals);
        assert!(hierarchy
            .signals
            .iter()
            .any(|signal| !signal.changes.is_empty()));
    }
}
