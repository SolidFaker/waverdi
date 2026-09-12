//! Source file view of a module: highlighted lines and a text cursor.
//!
//! The highlighter is intentionally small: keywords, numbers, strings,
//! comments, `` `directives `` and identifiers that are declared signals of
//! the module (so the ones you can add to the waveform stand out).

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use super::scan::{is_keyword, ModuleDef};

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
    pub spans: Vec<Vec<Span>>,
    pub scroll: usize,
    pub line: usize,
    pub col: usize,
    /// Line the current line selection is anchored at.
    pub anchor: Option<usize>,
    /// Selected line range (inclusive, ordered).
    pub sel: Option<(usize, usize)>,
    /// Selected identifier: (line, start column, end column).
    pub word: Option<(usize, usize, usize)>,
}

impl SourceView {
    pub fn load(def: &ModuleDef) -> Option<SourceView> {
        let text = fs::read_to_string(&def.file).ok()?;
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        if lines.is_empty() {
            return None;
        }
        let names: HashSet<&str> = def.signals.iter().map(|decl| decl.name.as_str()).collect();
        let mut spans = Vec::with_capacity(lines.len());
        let mut in_block = false;
        for line in &lines {
            spans.push(highlight_line(line, &names, &mut in_block));
        }
        let line = def.start.saturating_sub(1).min(lines.len() - 1);
        Some(SourceView {
            module: def.name.clone(),
            file: def.file.clone(),
            lines,
            spans,
            scroll: line,
            line,
            col: 0,
            anchor: None,
            sel: None,
            word: None,
        })
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
        self.word = None;
    }

    /// Start a line selection at the cursor (mouse down).
    pub fn begin_line_selection(&mut self) {
        self.anchor = Some(self.line);
        self.sel = Some((self.line, self.line));
        self.word = None;
    }

    /// Extend the line selection to `line`.
    pub fn select_lines(&mut self, line: usize) {
        let line = line.min(self.lines.len().saturating_sub(1));
        let anchor = self.anchor.unwrap_or(self.line);
        self.line = line;
        self.sel = Some((anchor.min(line), anchor.max(line)));
        self.word = None;
    }

    /// Extend the selection by moving the cursor one row (Shift+arrows).
    pub fn extend_line_selection(&mut self, delta_line: i64, rows: usize) {
        if self.anchor.is_none() {
            self.anchor = Some(self.line);
        }
        let last = self.lines.len().saturating_sub(1) as i64;
        self.line = (self.line as i64 + delta_line).clamp(0, last.max(0)) as usize;
        if let Some(anchor) = self.anchor {
            self.sel = Some((anchor.min(self.line), anchor.max(self.line)));
        }
        self.word = None;
        self.ensure_visible(rows);
    }

    /// Select the identifier under the cursor (double click).
    pub fn select_word(&mut self) {
        if let Some((start, end)) = self.word_span_at_cursor() {
            self.word = Some((self.line, start, end));
            self.sel = None;
            self.anchor = None;
        }
    }

    pub fn is_line_selected(&self, line: usize) -> bool {
        self.sel
            .map(|(start, end)| line >= start && line <= end)
            .unwrap_or(false)
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
        let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
        let mut start = self.col.min(chars.len());
        if start >= chars.len() || !is_word(chars[start]) {
            if start > 0 && is_word(chars[start - 1]) {
                start -= 1;
            } else {
                return None;
            }
        }
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        let mut end = start;
        while end < chars.len() && is_word(chars[end]) {
            end += 1;
        }
        (end > start).then_some((start, end))
    }
}

fn highlight_line(line: &str, names: &HashSet<&str>, in_block: &mut bool) -> Vec<Span> {
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
            } else if names.contains(word.as_str()) {
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
        let def = super::super::scan::parse_text_for_test(SOURCE, &file).expect("module");
        SourceView::load(&def).expect("view")
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
