//! RTL source management: filelists and FSDB side-channel discovery.
//!
//! Verdi shows RTL sources from its KDB (`simv.daidir`), which stores a plain
//! text list of the compiled sources at `debug_dump/src_files_verilog`. When
//! the user does not pass a filelist we look for that database next to the
//! FSDB (or for the path embedded in the FSDB itself), and fall back to
//! scanning for `.v`/`.sv` files near the dump.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// RTL source files and compile options gathered from a filelist or a dump.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceSet {
    pub files: Vec<PathBuf>,
    pub incdirs: Vec<PathBuf>,
    pub defines: Vec<String>,
    /// Top module names (from the Verdi KDB when available).
    pub tops: Vec<String>,
    /// Where the set came from, for status messages.
    pub origin: String,
}

impl SourceSet {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Parse a VCS-style filelist (`-f`, `-v`, `-y`, `+incdir+`, `+define+`).
    pub fn from_filelist(path: &Path) -> io::Result<SourceSet> {
        let mut set = SourceSet {
            origin: path.display().to_string(),
            ..SourceSet::default()
        };
        let mut seen_lists = BTreeSet::new();
        parse_filelist(path, &mut set, &mut seen_lists, 0)?;
        dedup(&mut set.files);
        dedup(&mut set.incdirs);
        Ok(set)
    }

    /// Explicit file set (used by tests and by directory scans).
    pub fn from_files(files: Vec<PathBuf>, origin: impl Into<String>) -> SourceSet {
        let mut set = SourceSet {
            files,
            origin: origin.into(),
            ..SourceSet::default()
        };
        dedup(&mut set.files);
        set
    }

    /// Try to recover the source list for a dump: the Verdi KDB next to the
    /// dump, the KDB path embedded in the FSDB, or `.v`/`.sv` files nearby.
    pub fn discover_from_dump(dump: &Path) -> Option<SourceSet> {
        let dir = dump.parent()?;
        for base in [Some(dir), dir.parent()] {
            let Some(base) = base else { continue };
            if let Some(set) = daidir_in_dir(base) {
                return Some(set);
            }
        }
        if let Some(set) = daidir_embedded_in(dump) {
            return Some(set);
        }
        let files = scan_sources(dir, 2);
        if files.is_empty() {
            None
        } else {
            Some(SourceSet::from_files(
                files,
                format!("{} (directory scan)", dir.display()),
            ))
        }
    }
}

fn parse_filelist(
    path: &Path,
    set: &mut SourceSet,
    seen: &mut BTreeSet<PathBuf>,
    depth: usize,
) -> io::Result<()> {
    if depth > 8 {
        return Ok(());
    }
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canonical) {
        return Ok(());
    }
    let text = fs::read_to_string(path)?;
    let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
    // Join backslash continuations before tokenising.
    let joined = text.replace("\\\r\n", " ").replace("\\\n", " ");
    let mut tokens = joined
        .lines()
        .flat_map(|line| {
            let line = strip_comment(line);
            line.split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .peekable();

    while let Some(token) = tokens.next() {
        if let Some(dirs) = token.strip_prefix("+incdir+") {
            for dir in dirs.split('+').filter(|d| !d.is_empty()) {
                set.incdirs.push(resolve(&base, dir));
            }
        } else if let Some(def) = token
            .strip_prefix("+define+")
            .or_else(|| token.strip_prefix("-D"))
        {
            if !def.is_empty() {
                set.defines.push(def.to_string());
            }
        } else if token == "-f" || token == "-F" || token == "-file" {
            if let Some(next) = tokens.next() {
                let _ = parse_filelist(&resolve(&base, &next), set, seen, depth + 1);
            }
        } else if token == "-v" || token == "-y" {
            if let Some(next) = tokens.next() {
                let resolved = resolve(&base, &next);
                if token == "-y" {
                    set.incdirs.push(resolved);
                } else if resolved.is_file() {
                    set.files.push(resolved);
                }
            }
        } else if token.starts_with('+') || token.starts_with('-') {
            // Other compiler options are irrelevant for browsing.
        } else {
            let file = resolve(&base, &token);
            if file.is_file() {
                set.files.push(file);
            }
        }
    }
    Ok(())
}

fn strip_comment(line: &str) -> &str {
    let cut = line
        .find("//")
        .into_iter()
        .chain(line.find('#'))
        .min()
        .unwrap_or(line.len());
    &line[..cut]
}

fn resolve(base: &Path, token: &str) -> PathBuf {
    let path = Path::new(token);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn dedup(paths: &mut Vec<PathBuf>) {
    let mut seen = BTreeSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

/// Source list recorded by the Verdi KDB in `base/*.daidir`.
fn daidir_in_dir(base: &Path) -> Option<SourceSet> {
    let entries = fs::read_dir(base).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        if !name.to_string_lossy().ends_with(".daidir") {
            continue;
        }
        if let Some(set) = read_daidir_sources(&path) {
            return Some(set);
        }
    }
    None
}

/// Read `debug_dump/src_files_verilog` (+ `topmodules`) from a Verdi KDB.
fn read_daidir_sources(daidir: &Path) -> Option<SourceSet> {
    let list = daidir.join("debug_dump/src_files_verilog");
    let text = fs::read_to_string(&list).ok()?;
    let files: Vec<PathBuf> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .collect();
    if files.is_empty() {
        return None;
    }
    let tops = fs::read_to_string(daidir.join("debug_dump/topmodules"))
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some(SourceSet {
        files,
        tops,
        origin: format!("{} (Verdi KDB)", daidir.display()),
        ..SourceSet::default()
    })
}

/// Verdi stores the KDB path in the FSDB header; find `*.daidir` there.
fn daidir_embedded_in(dump: &Path) -> Option<SourceSet> {
    let mut file = fs::File::open(dump).ok()?;
    let mut head = vec![0u8; 8 * 1024 * 1024];
    let read = file.read(&mut head).ok()?;
    head.truncate(read);
    let text = String::from_utf8_lossy(&head);
    for (index, _) in text.match_indices(".daidir") {
        let start = text[..index]
            .rfind(['\0', '\n', '"', '\''])
            .map(|i| i + 1)
            .unwrap_or(0);
        let end = index + ".daidir".len();
        let candidate = PathBuf::from(text[start..end].trim());
        if candidate.is_absolute() && candidate.is_dir() {
            if let Some(set) = read_daidir_sources(&candidate) {
                return Some(set);
            }
        }
    }
    None
}

/// Scan a directory tree for RTL source files.
fn scan_sources(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                out.extend(scan_sources(&path, depth - 1));
            }
        } else if is_rtl_source(&path) {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn is_rtl_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("v" | "sv" | "vh" | "svh")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("waverdi_rtl_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_nested_filelists_and_options() {
        let dir = temp_dir("filelist");
        fs::write(dir.join("top.sv"), "module top; endmodule\n").unwrap();
        fs::write(dir.join("sub.sv"), "module sub; endmodule\n").unwrap();
        fs::create_dir_all(dir.join("inc")).unwrap();
        fs::write(dir.join("inc/incl.svh"), "// include\n").unwrap();
        fs::write(dir.join("sub.f"), "sub.sv\n+define+SUB=1\n").unwrap();
        fs::write(
            dir.join("files.f"),
            "// comment\n\
             top.sv\n\
             +incdir+inc\n\
             +define+WIDTH=8\n\
             -f sub.f\n\
             missing.sv\n",
        )
        .unwrap();
        let set = SourceSet::from_filelist(&dir.join("files.f")).unwrap();
        assert_eq!(set.files.len(), 2);
        assert!(set.files.iter().any(|f| f.ends_with("top.sv")));
        assert!(set.files.iter().any(|f| f.ends_with("sub.sv")));
        assert_eq!(set.incdirs.len(), 1);
        assert!(set.incdirs[0].ends_with("inc"));
        assert_eq!(set.defines, vec!["WIDTH=8", "SUB=1"]);
    }

    #[test]
    fn discovers_the_verdi_kdb_next_to_a_dump() {
        let dir = temp_dir("daidir");
        let daidir = dir.join("simv.daidir/debug_dump");
        fs::create_dir_all(&daidir).unwrap();
        let src = dir.join("counter.sv");
        fs::write(&src, "module counter; endmodule\n").unwrap();
        fs::write(
            daidir.join("src_files_verilog"),
            format!("{}\n", src.display()),
        )
        .unwrap();
        fs::write(daidir.join("topmodules"), "tb\n").unwrap();
        let dump = dir.join("counter.fsdb");
        fs::write(&dump, "not really an fsdb\n").unwrap();
        let set = SourceSet::discover_from_dump(&dump).expect("sources");
        assert_eq!(set.files, vec![src]);
        assert_eq!(set.tops, vec!["tb"]);
    }

    #[test]
    fn falls_back_to_scanning_the_dump_directory() {
        let dir = temp_dir("scan");
        fs::write(dir.join("a.sv"), "module a; endmodule\n").unwrap();
        fs::write(dir.join("notes.txt"), "ignore me\n").unwrap();
        let dump = dir.join("dump.vcd");
        fs::write(&dump, "nope\n").unwrap();
        let set = SourceSet::discover_from_dump(&dump).expect("sources");
        assert_eq!(set.files.len(), 1);
        assert!(set.files[0].ends_with("a.sv"));
    }
}
