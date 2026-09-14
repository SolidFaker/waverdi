use ratatui::buffer::Buffer;
use ratatui::style::{Color, Style};

/// Truncate to `width` terminal cells, adding an ellipsis when clipped.
pub fn trunc(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if s.chars().count() <= width {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Keep the right end of `s` in `width` cells, adding a leading ellipsis.
/// Used where the tail carries the meaning (file names, signal names).
pub fn trunc_left(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let len = s.chars().count();
    if len <= width {
        return s.to_string();
    }
    let skip = len - (width - 1);
    let mut out = String::from('…');
    out.extend(s.chars().skip(skip));
    out
}

/// Word-wrap into display lines of at most `width` cells. Words longer than
/// the width are split; an empty input yields one empty line.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        let mut word = word;
        while word.chars().count() > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let split = word
                .char_indices()
                .nth(width)
                .map(|(index, _)| index)
                .unwrap_or(word.len());
            let (head, tail) = word.split_at(split);
            lines.push(head.to_string());
            word = tail;
        }
        let current_len = current.chars().count();
        if current_len == 0 {
            current.push_str(word);
        } else if current_len + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// Slice `s` for a horizontally scrolled viewport: `offset` characters are
/// skipped and the result is at most `width` cells. A leading ellipsis is
/// only added when actual content (not just indentation) is hidden on the
/// left.
pub fn scroll_slice(s: &str, offset: usize, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let len = s.chars().count();
    if offset == 0 && len <= width {
        return s.to_string();
    }
    let left = s.chars().take(offset).any(|c| !c.is_whitespace());
    let room = width.saturating_sub(usize::from(left));
    let mut out = String::new();
    if left {
        out.push('…');
    }
    out.extend(s.chars().skip(offset).take(room));
    out
}

/// Draw a string, clipped to the buffer's right edge.
pub fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style) {
    let area = *buf.area();
    if x >= area.right() || y >= area.bottom() {
        return;
    }
    let width = (area.right() - x) as usize;
    buf.set_string(x, y, trunc(text, width), style);
}

/// Set a single cell's symbol, foreground and background.
pub fn set_cell(buf: &mut Buffer, x: u16, y: u16, symbol: &str, fg: Color, bg: Color) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(symbol);
        cell.set_fg(fg);
        cell.set_bg(bg);
    }
}

#[cfg(test)]
mod tests {
    use super::{scroll_slice, trunc, trunc_left, wrap};

    #[test]
    fn wrap_breaks_at_spaces_and_splits_long_words() {
        assert_eq!(wrap("hello world", 20), vec!["hello world"]);
        assert_eq!(wrap("hello world", 5), vec!["hello", "world"]);
        assert_eq!(wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(wrap("a b c d", 3), vec!["a b", "c d"]);
        assert_eq!(wrap("", 4), vec![""]);
    }

    #[test]
    fn truncation_keeps_the_requested_end() {
        assert_eq!(trunc("abcdef", 4), "abc…");
        assert_eq!(trunc_left("abcdef", 4), "…def");
        assert_eq!(trunc_left("abcdef", 10), "abcdef");
        assert_eq!(trunc_left("abcdef", 1), "…");
        assert_eq!(trunc_left("abcdef", 0), "");
        assert_eq!(trunc_left("wave.fsdb", 9), "wave.fsdb");
    }

    #[test]
    fn truncation_at_zero_width_is_empty() {
        assert_eq!(trunc("abc", 0), "");
    }

    #[test]
    fn scroll_slice_marks_only_hidden_content() {
        // Scrolled-out indentation does not produce an ellipsis.
        assert_eq!(scroll_slice("    name", 2, 8), "  name");
        // Hidden identifiers do.
        assert_eq!(scroll_slice("scope.name", 3, 8), "…pe.name");
        assert_eq!(scroll_slice("abc", 0, 8), "abc");
        assert_eq!(scroll_slice("abcdefghij", 0, 4), "abcd");
    }
}
