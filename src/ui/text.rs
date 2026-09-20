use ratatui::buffer::Buffer;
use ratatui::style::{Color, Style};
use std::borrow::Cow;

/// Truncate to `width` terminal cells, adding an ellipsis when clipped.
/// Borrows `s` unchanged when it already fits.
pub fn trunc(s: &str, width: usize) -> Cow<'_, str> {
    if width == 0 {
        return Cow::Borrowed("");
    }
    if s.chars().count() <= width {
        Cow::Borrowed(s)
    } else {
        let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        Cow::Owned(out)
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
/// left. Borrows `s` unchanged when nothing is clipped.
pub fn scroll_slice(s: &str, offset: usize, width: usize) -> Cow<'_, str> {
    if width == 0 {
        return Cow::Borrowed("");
    }
    let len = s.chars().count();
    if offset == 0 && len <= width {
        return Cow::Borrowed(s);
    }
    let left = s.chars().take(offset).any(|c| !c.is_whitespace());
    let room = width.saturating_sub(usize::from(left));
    let mut out = String::new();
    if left {
        out.push('…');
    }
    out.extend(s.chars().skip(offset).take(room));
    Cow::Owned(out)
}

/// Draw a string, clipped to the buffer's right edge.
pub fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style) {
    let area = *buf.area();
    if x >= area.right() || y >= area.bottom() {
        return;
    }
    let width = (area.right() - x) as usize;
    // The common case is unclipped: write the borrowed text directly.
    if text.chars().count() <= width {
        buf.set_string(x, y, text, style);
    } else {
        buf.set_string(x, y, trunc(text, width), style);
    }
}

/// Set a single cell's symbol, foreground and background.
pub fn set_cell(buf: &mut Buffer, x: u16, y: u16, symbol: &str, fg: Color, bg: Color) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(symbol);
        cell.set_fg(fg);
        cell.set_bg(bg);
    }
}

/// Draw a formatted logic value with its unknown digits tinted individually:
/// `x` takes `x_color`, `z` takes `z_color` and every other character keeps
/// the `base` style. The Signal List value column and the bus trace labels
/// use this so a partially unknown value no longer paints the whole text red.
pub fn put_unknown_digits(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    text: &str,
    base: Style,
    x_color: Color,
    z_color: Color,
) {
    let area = *buf.area();
    if x >= area.right() || y >= area.bottom() {
        return;
    }
    let width = (area.right() - x) as usize;
    let visible: Cow<'_, str> = if text.chars().count() <= width {
        Cow::Borrowed(text)
    } else {
        trunc(text, width)
    };
    for (i, ch) in visible.chars().enumerate() {
        let style = match ch {
            'x' => base.fg(x_color),
            'z' => base.fg(z_color),
            _ => base,
        };
        if let Some(cell) = buf.cell_mut((x.saturating_add(i as u16), y)) {
            let mut encoded = [0u8; 4];
            cell.set_symbol(ch.encode_utf8(&mut encoded));
            cell.set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{scroll_slice, trunc, trunc_left, wrap};
    use std::borrow::Cow;

    #[test]
    fn unclipped_text_is_borrowed_instead_of_allocated() {
        assert!(matches!(trunc("abc", 8), Cow::Borrowed("abc")));
        assert!(matches!(scroll_slice("abc", 0, 8), Cow::Borrowed("abc")));
    }

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
