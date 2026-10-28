//! Format adapter used by the background loader.
//!
//! Every dump format is opened through [`DumpSource`]: the caller gets the
//! hierarchy first and pulls value changes per signal, so the loader, the
//! compute pool and the UI do not need to know which format is behind it.

mod eager;
#[cfg(fsdb_sdk)]
mod fsdb;
mod fst;
mod vcd;

use crate::waveform::{Change, Waveform};
use std::path::Path;
use std::sync::Arc;

/// What a dump format can provide; the UI uses this to gate features
/// uniformly instead of guessing per format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Values arrive on request instead of with the hierarchy.
    pub lazy: bool,
    /// Signals carry port directions (FSDB does, VCD/FST do not).
    pub directions: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            lazy: true,
            directions: false,
        }
    }
}

pub type ReadResult = (Arc<Vec<Change>>, Vec<String>);

/// Value decoding that can safely leave the reader thread. The closure owns
/// all of its input, including format-specific raw storage, so the session only
/// needs to execute it on the compute pool.
pub struct DeferredDecode {
    decode: Box<dyn FnOnce() -> ReadResult + Send>,
}

impl DeferredDecode {
    pub fn new(decode: impl FnOnce() -> ReadResult + Send + 'static) -> Self {
        Self {
            decode: Box::new(decode),
        }
    }

    pub fn ready(result: ReadResult) -> Self {
        Self::new(move || result)
    }

    pub fn decode(self) -> ReadResult {
        (self.decode)()
    }
}

/// One dump format as seen by the loader. Reads are serialized by the loader
/// (FFR cannot be used from two threads), so implementations may assume
/// `read_signal` is called from a single thread; the trait is deliberately
/// not `Send` so a source cannot be moved between reader threads.
pub trait DumpSource {
    /// Whether values still have to be fetched from the source on request.
    fn is_lazy(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            lazy: self.is_lazy(),
            directions: false,
        }
    }

    fn var_count(&self) -> usize;
    fn take_hierarchy(&mut self) -> Waveform;

    fn take_warnings(&mut self) -> Vec<String> {
        Vec::new()
    }

    fn read_signal(&mut self, index: usize) -> ReadResult;

    /// Read several signals in one pass; the results are in the input order.
    fn read_signals(&mut self, indexes: &[usize]) -> Vec<ReadResult> {
        indexes
            .iter()
            .map(|&index| self.read_signal(index))
            .collect()
    }

    /// Read several signals and return work that can be decoded without the
    /// backend. Formats with a raw path override this to move value decoding
    /// off the reader thread; other formats inherit the already-decoded path.
    fn read_signals_deferred(&mut self, indexes: &[usize]) -> Vec<DeferredDecode> {
        self.read_signals(indexes)
            .into_iter()
            .map(DeferredDecode::ready)
            .collect()
    }
}

/// Open `path` as a [`DumpSource`]: FSDB is read lazily through the Verdi FFR
/// session, FST reads its header first and decodes values on the first read,
/// VCD parses declarations only and scans values on demand. Anything else
/// keeps the eager parse.
pub(crate) fn open_source(
    path: &Path,
    progress: crate::dump::Progress,
) -> Result<Box<dyn DumpSource>, String> {
    #[cfg(fsdb_sdk)]
    if super::detect(path) == super::Format::Fsdb {
        return Ok(Box::new(fsdb::FsdbSource::open(path, progress)?));
    }
    match super::detect(path) {
        super::Format::Fst => Ok(Box::new(fst::FstSource::open(path)?)),
        super::Format::Vcd => Ok(Box::new(vcd::VcdSource::open(path)?)),
        _ => {
            let out = super::parse_with_progress(path, progress)?;
            Ok(Box::new(eager::EagerSource::new(out)))
        }
    }
}
