//! Source file view of a module: highlighted lines and a text cursor.
//!
//! The highlighter is intentionally small: keywords, numbers, strings,
//! comments, `` `directives `` and identifiers that are declared signals of
//! the module (so the ones you can add to the waveform stand out).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use super::scan::{is_keyword, ModuleDef, RtlDb};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HlKind {
    Plain,
    Keyword,
    Number,
    String,
    Comment,
    Directive,
    /// Identifier that is a declared signal of the module.
    Signal,
}

#[derive(Clone, Debug)]
pub struct Span {
    pub text: String,
    pub kind: HlKind,
}

/// Loaded source of the instance's module with a (line, column) cursor.
pub struct SourceView {
    pub module: String,
    pub file: PathBuf,
    pub lines: Vec<String>,
    /// Module that owns each line; the active module outside module regions.
    pub line_modules: Vec<String>,
    pub spans: Vec<Vec<Span>>,
    pub scroll: usize,
    pub line: usize,
    pub col: usize,
    /// Character selection anchor: (line, column).
    pub anchor: Option<(usize, usize)>,
    /// Character selection range, normalized so start <= end.
    pub sel: Option<((usize, usize), (usize, usize))>,
}

impl SourceView {
    /// Load the module's file and classify its identifiers with the RTL AST:
    /// declared signals and instance-qualified references (`u_dut.count`).
    ///
    /// A file often holds several modules; every line is classified with the
    /// module that owns it, not with the active one.
    pub fn load(def: &ModuleDef, db: &RtlDb) -> Option<SourceView> {
        let text = fs::read_to_string(&def.file).ok()?;
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        if lines.is_empty() {
            return None;
        }
        let line_modules = module_lines(&lines, def, db);
        let names_by_module = module_signal_names(db);
        let qualified = qualified_references(&lines, &line_modules, db);
        let empty = HashSet::new();
        let mut spans = Vec::with_capacity(lines.len());
        let mut in_block = false;
        for (index, line) in lines.iter().enumerate() {
            let names = names_by_module
                .get(line_modules[index].as_str())
                .unwrap_or(&empty);
            spans.push(highlight_line(
                line,
                index,
                names,
                &qualified,
                &mut in_block,
            ));
        }
        let line = def.start.saturating_sub(1).min(lines.len() - 1);
        Some(SourceView {
            module: def.name.clone(),
            file: def.file.clone(),
            lines,
            line_modules,
            spans,
            scroll: line,
            line,
            col: 0,
            anchor: None,
            sel: None,
        })
    }

    /// Module whose code this line belongs to (the active module outside
    /// module regions and after the end of the file).
    pub fn module_at_line(&self, line: usize) -> &str {
        self.line_modules
            .get(line)
            .map(String::as_str)
            .unwrap_or(&self.module)
    }

    pub fn move_cursor(&mut self, delta_line: i64, delta_col: i64, rows: usize) {
        self.clear_selection();
        let last = self.lines.len().saturating_sub(1) as i64;
        self.line = (self.line as i64 + delta_line).clamp(0, last.max(0)) as usize;
        let len = self
            .lines
            .get(self.line)
            .map(|line| line.chars().count())
            .unwrap_or(0) as i64;
        self.col = (self.col as i64 + delta_col).clamp(0, len) as usize;
        self.ensure_visible(rows);
    }

    pub fn set_cursor(&mut self, line: usize, col: usize, rows: usize) {
        self.line = line.min(self.lines.len().saturating_sub(1));
        let len = self
            .lines
            .get(self.line)
            .map(|line| line.chars().count())
            .unwrap_or(0);
        self.col = col.min(len);
        self.ensure_visible(rows);
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
        self.sel = None;
    }

    /// Start a character selection at the cursor (mouse down / Shift+keys).
    pub fn begin_selection(&mut self) {
        self.anchor = Some((self.line, self.col));
        self.sel = Some(((self.line, self.col), (self.line, self.col)));
    }

    fn clamp_pos(&self, line: usize, col: usize) -> (usize, usize) {
        let line = line.min(self.lines.len().saturating_sub(1));
        let len = self
            .lines
            .get(line)
            .map(|text| text.chars().count())
            .unwrap_or(0);
        (line, col.min(len))
    }

    /// Extend the selection to a (line, column) position (mouse drag).
    pub fn extend_selection_to(&mut self, line: usize, col: usize) {
        let to = self.clamp_pos(line, col);
        let anchor = self.anchor.unwrap_or((self.line, self.col));
        self.line = to.0;
        self.col = to.1;
        self.sel = Some(if anchor <= to {
            (anchor, to)
        } else {
            (to, anchor)
        });
    }

    /// Extend the selection by moving the cursor (Shift+arrows).
    pub fn extend_selection_by(&mut self, delta_line: i64, delta_col: i64, rows: usize) {
        if self.anchor.is_none() {
            self.anchor = Some((self.line, self.col));
        }
        let last = self.lines.len().saturating_sub(1) as i64;
        let line = (self.line as i64 + delta_line).clamp(0, last.max(0)) as usize;
        let len = self
            .lines
            .get(line)
            .map(|text| text.chars().count())
            .unwrap_or(0) as i64;
        let col = if delta_col == 0 {
            self.col.min(len as usize)
        } else {
            (self.col as i64 + delta_col).clamp(0, len) as usize
        };
        self.extend_selection_to(line, col);
        self.ensure_visible(rows);
    }

    /// Select the identifier under the cursor (double click / plain click).
    pub fn select_word(&mut self) {
        if let Some((start, end)) = self.word_span_at_cursor() {
            let line = self.line;
            self.anchor = Some((line, start));
            self.sel = Some(((line, start), (line, end)));
        }
    }

    /// Finish a mouse selection: a click that did not drag picks the word
    /// under the cursor instead of a whole line.
    pub fn finish_selection(&mut self) {
        if let Some((start, end)) = self.sel {
            if start == end {
                self.select_word();
            }
        }
    }

    /// Character interval selected on `line`, if the line is part of the
    /// selection: `[start, end)` in character columns.
    pub fn selection_interval(&self, line: usize) -> Option<(usize, usize)> {
        let ((l0, c0), (l1, c1)) = self.sel?;
        if line < l0 || line > l1 {
            return None;
        }
        let len = self
            .lines
            .get(line)
            .map(|text| text.chars().count())
            .unwrap_or(0);
        if l0 == l1 {
            return Some((c0.min(len), c1.min(len)));
        }
        if line == l0 {
            Some((c0.min(len), len))
        } else if line == l1 {
            Some((0, c1.min(len)))
        } else {
            Some((0, len))
        }
    }

    pub fn ensure_visible(&mut self, rows: usize) {
        let rows = rows.max(1);
        if self.line < self.scroll {
            self.scroll = self.line;
        } else if self.line >= self.scroll + rows {
            self.scroll = self.line + 1 - rows;
        }
        let max = self.lines.len().saturating_sub(rows);
        self.scroll = self.scroll.min(max);
    }

    pub fn scroll_by(&mut self, delta: i64, rows: usize) {
        let max = self.lines.len().saturating_sub(rows.max(1)) as i64;
        self.scroll = (self.scroll as i64 + delta).clamp(0, max.max(0)) as usize;
    }

    pub fn page(&mut self, down: bool, rows: usize) {
        let step = rows.max(1) as i64;
        self.move_cursor(if down { step } else { -step }, 0, rows);
    }

    /// Identifier under the cursor (or immediately left of it), ignoring
    /// keywords so only real signal names can be picked.
    pub fn word_at_cursor(&self) -> Option<String> {
        let (start, end) = self.word_span_at_cursor()?;
        let line = self.lines.get(self.line)?;
        let word: String = line.chars().skip(start).take(end - start).collect();
        if word.is_empty() || word.chars().next()?.is_ascii_digit() || is_keyword(&word) {
            return None;
        }
        Some(word)
    }

    /// Character span of the identifier under (or just left of) the cursor.
    pub fn word_span_at_cursor(&self) -> Option<(usize, usize)> {
        let line = self.lines.get(self.line)?;
        let chars: Vec<char> = line.chars().collect();
        let mut start = self.col.min(chars.len());
        if start >= chars.len() || !is_ident_char(chars[start]) {
            if start > 0 && is_ident_char(chars[start - 1]) {
                start -= 1;
            } else {
                return None;
            }
        }
        while start > 0 && is_ident_char(chars[start - 1]) {
            start -= 1;
        }
        let mut end = start;
        while end < chars.len() && is_ident_char(chars[end]) {
            end += 1;
        }
        (end > start).then_some((start, end))
    }

    /// Instance qualifiers directly before character `start` on `line`: for
    /// `u_dut.count` with `start` at `count` this returns `["u_dut"]`. The
    /// RTL AST decides whether those names really are instances.
    pub fn qualifier_chain(&self, line: usize, start: usize) -> Vec<String> {
        let Some(text) = self.lines.get(line) else {
            return Vec::new();
        };
        let chars: Vec<char> = text.chars().collect();
        identifier_chain(&chars, start)
    }

    /// Whether the identifier at `start` follows the `.` of a named port
    /// connection (`.port`), rather than a field access (`sig.field`).
    pub fn dotted_port(&self, line: usize, start: usize) -> bool {
        let Some(text) = self.lines.get(line) else {
            return false;
        };
        let chars: Vec<char> = text.chars().collect();
        if start == 0 || chars.get(start - 1) != Some(&'.') {
            return false;
        }
        start < 2 || !is_ident_char(chars[start - 2])
    }

    /// Text of every `[...]` selector directly after `start` on `line`: for
    /// `arr[i][j]` this returns `["i", "j"]`, for `count[3:0]` `["3:0"]`.
    pub fn indices_after(&self, line: usize, start: usize) -> Vec<String> {
        let Some(text) = self.lines.get(line) else {
            return Vec::new();
        };
        let chars: Vec<char> = text.chars().collect();
        let mut i = start.min(chars.len());
        let mut out = Vec::new();
        loop {
            while chars.get(i) == Some(&' ') {
                i += 1;
            }
            if chars.get(i) != Some(&'[') {
                break;
            }
            let mut depth = 0i32;
            let mut group = String::new();
            let mut closed = false;
            while i < chars.len() {
                match chars[i] {
                    '[' => {
                        depth += 1;
                        if depth > 1 {
                            group.push('[');
                        }
                    }
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            closed = true;
                            break;
                        }
                        group.push(']');
                    }
                    c => group.push(c),
                }
                i += 1;
            }
            if !closed {
                break;
            }
            out.push(group);
        }
        out
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Identifier qualifiers directly before character `start`: for
/// `u_dut.count` with `start` at `count` returns `["u_dut"]`.
fn identifier_chain(chars: &[char], start: usize) -> Vec<String> {
    let mut i = start.min(chars.len());
    let mut chain = Vec::new();
    while i > 0 && chars[i - 1] == '.' {
        let end = i - 1;
        let mut begin = end;
        while begin > 0 && is_ident_char(chars[begin - 1]) {
            begin -= 1;
        }
        if begin == end {
            break;
        }
        chain.push(chars[begin..end].iter().collect::<String>());
        i = begin;
    }
    chain.reverse();
    chain
}

/// Module owning each line of the file: the module whose `module ... endmodule`
/// region contains the line, or the active module (`def`) outside every region.
fn module_lines(lines: &[String], def: &ModuleDef, db: &RtlDb) -> Vec<String> {
    let mut owners = vec![def.name.clone(); lines.len()];
    let mut regions: Vec<&ModuleDef> = db
        .modules
        .values()
        .filter(|module| module.file == def.file)
        .collect();
    regions.sort_by_key(|module| module.start);
    for module in regions {
        let start = module.start.saturating_sub(1).min(lines.len());
        let end = module.end.min(lines.len());
        for owner in owners.iter_mut().take(end).skip(start) {
            *owner = module.name.clone();
        }
    }
    owners
}

/// Declared signal names per module, for per-line classification.
fn module_signal_names(db: &RtlDb) -> HashMap<&str, HashSet<&str>> {
    db.modules
        .iter()
        .map(|(name, def)| {
            let names = def.signals.iter().map(|decl| decl.name.as_str()).collect();
            (name.as_str(), names)
        })
        .collect()
}

/// Character positions of identifiers that the RTL AST resolves to a signal
/// although the plain name is not declared in the line's module: instance
/// chains (`count` in `u_dut.count`) and named port connections (`.count`).
fn qualified_references(
    lines: &[String],
    line_modules: &[String],
    db: &RtlDb,
) -> HashSet<(usize, usize)> {
    let mut ports: HashMap<&str, HashSet<(usize, &str)>> = HashMap::new();
    for (name, def) in &db.modules {
        let entries = def
            .instances
            .iter()
            .flat_map(|inst| inst.ports.iter().map(|(port, line)| (*line, port.as_str())))
            .collect();
        ports.insert(name.as_str(), entries);
    }
    let mut qualified = HashSet::new();
    for (index, line) in lines.iter().enumerate() {
        let module = line_modules
            .get(index)
            .map(String::as_str)
            .unwrap_or_default();
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            if is_ident_char(chars[i]) && !chars[i].is_ascii_digit() {
                let start = i;
                while i < chars.len() && is_ident_char(chars[i]) {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                if !is_keyword(&word) {
                    if ports
                        .get(module)
                        .is_some_and(|set| set.contains(&(index + 1, word.as_str())))
                    {
                        qualified.insert((index, start));
                        continue;
                    }
                    let chain = identifier_chain(&chars, start);
                    if !chain.is_empty() && db.resolve_reference(module, &chain, &word).is_some() {
                        qualified.insert((index, start));
                    }
                }
            } else {
                i += 1;
            }
        }
    }
    qualified
}

fn highlight_line(
    line: &str,
    index: usize,
    names: &HashSet<&str>,
    qualified: &HashSet<(usize, usize)>,
    in_block: &mut bool,
) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut i = 0usize;

    fn flush(spans: &mut Vec<Span>, plain: &mut String, kind: HlKind) {
        if !plain.is_empty() {
            spans.push(Span {
                text: std::mem::take(plain),
                kind,
            });
        }
    }
    fn push(spans: &mut Vec<Span>, text: String, kind: HlKind) {
        if !text.is_empty() {
            spans.push(Span { text, kind });
        }
    }

    while i < chars.len() {
        if *in_block {
            let start = i;
            let mut closed = false;
            while i < chars.len() {
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    i += 2;
                    closed = true;
                    *in_block = false;
                    break;
                }
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            push(&mut spans, text, HlKind::Comment);
            if !closed {
                break;
            }
            continue;
        }
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            flush(&mut spans, &mut plain, HlKind::Plain);
            let text: String = chars[i..].iter().collect();
            push(&mut spans, text, HlKind::Comment);
            break;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            flush(&mut spans, &mut plain, HlKind::Plain);
            *in_block = true;
            continue;
        }
        if c == '"' {
            flush(&mut spans, &mut plain, HlKind::Plain);
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(chars.len());
            let text: String = chars[start..i].iter().collect();
            push(&mut spans, text, HlKind::String);
            continue;
        }
        if c == '`' {
            flush(&mut spans, &mut plain, HlKind::Plain);
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            push(&mut spans, text, HlKind::Directive);
            continue;
        }
        if c.is_ascii_digit() {
            flush(&mut spans, &mut plain, HlKind::Plain);
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '\'' | '.' | '?'))
            {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            push(&mut spans, text, HlKind::Number);
            continue;
        }
        if c.is_alphanumeric() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let kind = if is_keyword(&word) {
                HlKind::Keyword
            } else if names.contains(word.as_str()) || qualified.contains(&(index, start)) {
                HlKind::Signal
            } else {
                HlKind::Plain
            };
            if kind == HlKind::Plain {
                plain.push_str(&word);
            } else {
                flush(&mut spans, &mut plain, HlKind::Plain);
                push(&mut spans, word, kind);
            }
            continue;
        }
        plain.push(c);
        i += 1;
    }
    flush(&mut spans, &mut plain, HlKind::Plain);
    if spans.is_empty() {
        spans.push(Span {
            text: String::new(),
            kind: HlKind::Plain,
        });
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "module counter(input logic clk, output logic [3:0] count);\n\
                          // count comment\n\
                          logic [3:0] next;\n\
                          assign count = next; /* block\n\
                          comment */\n\
                          endmodule\n";

    fn view(tag: &str) -> SourceView {
        let dir = std::env::temp_dir().join(format!("waverdi_view_{}_{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("counter.sv");
        fs::write(&file, SOURCE).unwrap();
        let set = crate::rtl::SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        let def = db.module("counter").expect("module");
        SourceView::load(def, &db).expect("view")
    }

    #[test]
    fn highlights_instance_qualified_signals_via_the_ast() {
        const TOP: &str = "module top;\n\
                            logic clk;\n\
                            counter u_dut(.clk(clk));\n\
                            assign x = u_dut.count;\n\
                            endmodule\n\
                            module counter(input logic clk, output logic [3:0] count);\n\
                            endmodule\n";
        let dir = std::env::temp_dir().join(format!("waverdi_view_ast_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("top.sv");
        fs::write(&file, TOP).unwrap();
        let set = crate::rtl::SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        let def = db.module("top").expect("module");
        let view = SourceView::load(def, &db).expect("view");
        // `count` is not declared in `top`, only reachable through `u_dut`.
        let line = view
            .lines
            .iter()
            .position(|line| line.contains("u_dut.count"))
            .unwrap();
        assert!(view.spans[line]
            .iter()
            .any(|span| span.kind == HlKind::Signal && span.text == "count"));
        assert_eq!(
            view.qualifier_chain(line, view.lines[line].find("count").unwrap()),
            vec!["u_dut".to_string()]
        );
    }

    #[test]
    fn highlights_keywords_comments_and_signals() {
        let view = view("hl");
        assert_eq!(view.module, "counter");
        let first = &view.spans[0];
        assert!(first
            .iter()
            .any(|s| s.kind == HlKind::Keyword && s.text == "module"));
        assert!(first
            .iter()
            .any(|s| s.kind == HlKind::Signal && s.text == "count"));
        assert!(view.spans[1]
            .iter()
            .any(|s| s.kind == HlKind::Comment && s.text.contains("count comment")));
        // The block comment spans two lines.
        assert!(view.spans[3].iter().any(|s| s.kind == HlKind::Comment));
        assert!(view.spans[4].iter().any(|s| s.kind == HlKind::Comment));
    }

    #[test]
    fn character_selection_intervals() {
        let mut view = view("sel");
        view.set_cursor(1, 2, 10);
        view.begin_selection();
        view.extend_selection_to(2, 3);
        assert_eq!(view.selection_interval(0), None);
        let line1_len = view.lines[1].chars().count();
        assert_eq!(view.selection_interval(1), Some((2, line1_len)));
        assert_eq!(view.selection_interval(2), Some((0, 3)));
        // A reversed selection is normalized.
        view.set_cursor(2, 3, 10);
        view.begin_selection();
        view.extend_selection_to(1, 2);
        assert_eq!(view.selection_interval(2), Some((0, 3)));
        assert_eq!(view.selection_interval(1), Some((2, line1_len)));
        // Finish without dragging turns a click into a word selection.
        let next_col = view.lines[3].find("next").unwrap();
        view.set_cursor(3, next_col, 10);
        view.begin_selection();
        view.extend_selection_to(3, next_col);
        view.finish_selection();
        assert_eq!(view.selection_interval(3), Some((next_col, next_col + 4)));
    }

    #[test]
    fn lines_belong_to_their_own_module() {
        const FILE: &str = "module a;\n\
                            logic only_a;\n\
                            endmodule\n\
                            module b;\n\
                            logic only_b;\n\
                            endmodule\n";
        let dir = std::env::temp_dir().join(format!("waverdi_view_multi_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("multi.sv");
        fs::write(&file, FILE).unwrap();
        let set = crate::rtl::SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        let view = SourceView::load(db.module("a").expect("a"), &db).expect("view");
        assert_eq!(view.module_at_line(1), "a");
        assert_eq!(view.module_at_line(4), "b");
        // `only_b` is a signal in b's own code even while a is active, and
        // `only_a` in a's.
        assert!(view.spans[1]
            .iter()
            .any(|span| span.kind == HlKind::Signal && span.text == "only_a"));
        assert!(view.spans[4]
            .iter()
            .any(|span| span.kind == HlKind::Signal && span.text == "only_b"));
    }

    #[test]
    fn modules_with_always_blocks_keep_their_own_lines() {
        const FILE: &str = "module pipe_reg(input logic clk, input logic rst_n, output logic q);\n\
                            logic r;\n\
                            always_ff @(posedge clk or negedge rst_n) begin\n\
                            if (!rst_n) begin\n\
                            r <= 1'b0;\n\
                            end else begin\n\
                            r <= ~r;\n\
                            end\n\
                            end\n\
                            assign q = r;\n\
                            endmodule\n\
                            module fifo(input logic clk);\n\
                            logic c;\n\
                            always_ff @(posedge clk) begin\n\
                            c <= c + 1'b1;\n\
                            end\n\
                            endmodule\n";
        let dir = std::env::temp_dir().join(format!("waverdi_view_always_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("common.sv");
        fs::write(&file, FILE).unwrap();
        let set = crate::rtl::SourceSet::from_files(vec![file], "test");
        let db = RtlDb::parse_sources(&set);
        let view = SourceView::load(db.module("fifo").expect("fifo"), &db).expect("view");
        // Every line of pipe_reg (including the assign after its always block)
        // stays inactive while fifo is the active module.
        let pipe = db.module("pipe_reg").unwrap();
        for line in pipe.start - 1..pipe.end {
            assert_eq!(view.module_at_line(line), "pipe_reg", "line {}", line + 1);
        }
        let fifo = db.module("fifo").unwrap();
        for line in fifo.start - 1..fifo.end {
            assert_eq!(view.module_at_line(line), "fifo", "line {}", line + 1);
        }
    }

    #[test]
    fn cursor_word_picking_skips_keywords() {
        let mut view = view("word");
        let col = view.lines[3].find("next").expect("next on line");
        view.set_cursor(3, col, 10);
        assert_eq!(view.word_at_cursor().as_deref(), Some("next"));
        view.set_cursor(3, col + 4, 10); // just after `next`
        assert_eq!(view.word_at_cursor().as_deref(), Some("next"));
        view.set_cursor(0, 1, 10); // `module` keyword
        assert_eq!(view.word_at_cursor(), None);
    }
}
