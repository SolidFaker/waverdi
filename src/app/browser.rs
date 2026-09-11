use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EntryKind {
    Parent,
    Dir,
    File,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
}

/// Strip the Windows `\\?\` extended-length prefix for friendlier paths.
pub(crate) fn clean_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            if let Some(unc) = rest.strip_prefix("UNC\\") {
                return PathBuf::from(format!(r"\\{unc}"));
            }
            return PathBuf::from(rest);
        }
    }
    path
}

/// A small terminal file browser used when no system dialog is available.
pub struct FileBrowser {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub sel: usize,
    pub scroll: usize,
    pub error: Option<String>,
}

impl FileBrowser {
    pub fn new(start: &Path) -> Self {
        let start = if start.is_file() {
            start.parent().unwrap_or_else(|| Path::new("."))
        } else {
            start
        };
        let dir = start
            .canonicalize()
            .map(clean_path)
            .or_else(|_| std::env::current_dir())
            .unwrap_or_else(|_| PathBuf::from("."));
        let mut browser = Self {
            dir,
            entries: Vec::new(),
            sel: 0,
            scroll: 0,
            error: None,
        };
        browser.refresh();
        browser
    }

    pub fn refresh(&mut self) {
        self.entries.clear();
        self.error = None;

        if let Some(parent) = self.dir.parent() {
            self.entries.push(Entry {
                name: "..".to_string(),
                path: parent.to_path_buf(),
                kind: EntryKind::Parent,
            });
        }

        match fs::read_dir(&self.dir) {
            Ok(read) => {
                let mut dirs = Vec::new();
                let mut files = Vec::new();
                for item in read.flatten() {
                    let name = item.file_name().to_string_lossy().into_owned();
                    let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    let entry = Entry {
                        name,
                        path: item.path(),
                        kind: if is_dir {
                            EntryKind::Dir
                        } else {
                            EntryKind::File
                        },
                    };
                    if is_dir {
                        dirs.push(entry);
                    } else {
                        files.push(entry);
                    }
                }
                dirs.sort_by_key(|e| e.name.to_lowercase());
                files.sort_by(|a, b| {
                    let a_vcd = a.name.to_lowercase().ends_with(".vcd");
                    let b_vcd = b.name.to_lowercase().ends_with(".vcd");
                    b_vcd
                        .cmp(&a_vcd)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                self.entries.extend(dirs);
                self.entries.extend(files);
            }
            Err(e) => self.error = Some(format!("cannot read {}: {e}", self.dir.display())),
        }

        if self.sel >= self.entries.len() {
            self.sel = self.entries.len().saturating_sub(1);
        }
        self.scroll = self.scroll.min(self.sel);
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.entries.get(self.sel)
    }

    pub fn move_sel(&mut self, delta: i64, rows: usize) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() as i64 - 1;
        self.sel = (self.sel as i64 + delta).clamp(0, last) as usize;
        self.scroll_to_sel(rows);
    }

    pub fn select(&mut self, index: usize, rows: usize) {
        if index >= self.entries.len() {
            return;
        }
        self.sel = index;
        self.scroll_to_sel(rows);
    }

    pub fn scroll_to_sel(&mut self, rows: usize) {
        let rows = rows.max(1);
        if self.sel < self.scroll {
            self.scroll = self.sel;
        } else if self.sel >= self.scroll + rows {
            self.scroll = self.sel + 1 - rows;
        }
    }

    /// Go to the parent directory. Returns false at the filesystem root.
    pub fn go_parent(&mut self) -> bool {
        let Some(parent) = self.dir.parent().map(Path::to_path_buf) else {
            return false;
        };
        self.navigate(&parent);
        true
    }

    pub fn navigate(&mut self, path: &Path) {
        self.dir = path.to_path_buf();
        self.sel = 0;
        self.scroll = 0;
        self.refresh();
    }

    /// Enter on the selected row: navigate into directories, return files.
    pub fn activate(&mut self) -> Option<PathBuf> {
        let entry = self.selected()?.clone();
        match entry.kind {
            EntryKind::Parent | EntryKind::Dir => {
                self.navigate(&entry.path);
                None
            }
            EntryKind::File => Some(entry.path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("waverdi_browser_{tag}_{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("subdir")).unwrap();
            fs::write(dir.join("b.txt"), "x").unwrap();
            fs::write(dir.join("a.vcd"), "x").unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn lists_dirs_first_then_vcd_files() {
        let tmp = TempDir::new("list");
        let browser = FileBrowser::new(&tmp.0);
        let names: Vec<&str> = browser.entries.iter().map(|e| e.name.as_str()).collect();
        let subdir = names.iter().position(|n| *n == "subdir").unwrap();
        let a_vcd = names.iter().position(|n| *n == "a.vcd").unwrap();
        let b_txt = names.iter().position(|n| *n == "b.txt").unwrap();
        assert!(subdir < a_vcd && a_vcd < b_txt);
        assert_eq!(browser.entries[0].kind, EntryKind::Parent);
    }

    #[test]
    fn activate_navigates_and_returns_files() {
        let tmp = TempDir::new("activate");
        let mut browser = FileBrowser::new(&tmp.0);

        let subdir = browser
            .entries
            .iter()
            .position(|e| e.name == "subdir")
            .unwrap();
        browser.select(subdir, 10);
        assert_eq!(browser.activate(), None);
        assert_eq!(
            browser.dir,
            clean_path(tmp.0.join("subdir").canonicalize().unwrap())
        );

        assert!(browser.go_parent());
        let a_vcd = browser
            .entries
            .iter()
            .position(|e| e.name == "a.vcd")
            .unwrap();
        browser.select(a_vcd, 10);
        assert_eq!(browser.activate(), Some(tmp.0.join("a.vcd")));
    }

    #[test]
    fn selection_scrolls_into_view() {
        let tmp = TempDir::new("scroll");
        let mut browser = FileBrowser::new(&tmp.0);
        browser.sel = browser.entries.len().saturating_sub(1);
        browser.scroll_to_sel(3);
        assert!(browser.scroll + 3 > browser.sel);
        browser.move_sel(-1, 3);
        assert!(browser.scroll <= browser.sel);
    }
}
