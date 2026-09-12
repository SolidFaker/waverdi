use ratatui::buffer::Buffer;
use ratatui::style::{Color, Style};

/// Truncate to `width` terminal cells, adding an ellipsis when clipped.
pub fn trunc(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        out
    }
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
    use super::wrap;

    #[test]
    fn wrap_breaks_at_spaces_and_splits_long_words() {
        assert_eq!(wrap("hello world", 20), vec!["hello world"]);
        assert_eq!(wrap("hello world", 5), vec!["hello", "world"]);
        assert_eq!(wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(wrap("a b c d", 3), vec!["a b", "c d"]);
        assert_eq!(wrap("", 4), vec![""]);
    }
}
