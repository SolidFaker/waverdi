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
