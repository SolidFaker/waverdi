use crate::waveform::Waveform;
use std::path::Path;

/// A parsed waveform plus non-fatal parser warnings.
pub struct ParseOut {
    pub wf: Waveform,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Vcd,
    Fst,
    Fsdb,
    Unknown,
}

/// Detect the dump format from the file extension.
pub fn detect(path: &Path) -> Format {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("vcd") => Format::Vcd,
        Some("fst") => Format::Fst,
        Some("fsdb") => Format::Fsdb,
        _ => Format::Unknown,
    }
}

/// Parse a waveform dump, dispatching on the file format.
pub fn parse(path: &Path) -> Result<ParseOut, String> {
    match detect(path) {
        Format::Vcd => crate::vcd::parse_vcd(path),
        Format::Fst => crate::fst::parse_fst(path),
        Format::Fsdb => parse_fsdb(path),
        Format::Unknown => Err(format!(
            "{}: unsupported dump type (supported: .vcd, .fst; .fsdb requires Verdi FFR)",
            path.display()
        )),
    }
}

/// FSDB is read through the Verdi FSDB Reader (FFR) when the SDK was
/// available at build time (`VERDI_HOME` set).
#[cfg(fsdb_sdk)]
fn parse_fsdb(path: &Path) -> Result<ParseOut, String> {
    crate::fsdb::parse_fsdb(path)
}

#[cfg(not(fsdb_sdk))]
fn parse_fsdb(path: &Path) -> Result<ParseOut, String> {
    Err(format!(
        "{}: FSDB is a proprietary Synopsys format and can only be read with the \
         Verdi FSDB Reader (FFR) library (nffr.dll / libnffr.so from VERDI_HOME). \
         Build with VERDI_HOME set (source ~/synopsys/env.sh) to enable FSDB support, \
         or convert the dump to VCD or FST (e.g. `fsdb2vcd`) and open that instead.",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_formats_by_extension() {
        assert_eq!(detect(Path::new("a.vcd")), Format::Vcd);
        assert_eq!(detect(Path::new("a.FST")), Format::Fst);
        assert_eq!(detect(Path::new("a.fsdb")), Format::Fsdb);
        assert_eq!(detect(Path::new("a.txt")), Format::Unknown);
    }

    #[test]
    #[cfg(not(fsdb_sdk))]
    fn fsdb_reports_verdi_hint() {
        let err = match parse(Path::new("missing.fsdb")) {
            Ok(_) => panic!("fsdb must not parse without the Verdi FFR library"),
            Err(err) => err,
        };
        assert!(err.contains("Verdi"), "{err}");
        assert!(err.contains("FFR"), "{err}");
    }

    #[test]
    #[cfg(fsdb_sdk)]
    fn fsdb_without_file_errors() {
        assert!(parse(Path::new("missing.fsdb")).is_err());
    }
}
