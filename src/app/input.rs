#[derive(Clone, Default)]
pub struct InputState {
    buf: Vec<char>,
    pos: usize,
}

impl InputState {
    pub fn clear(&mut self) {
        self.buf.clear();
        self.pos = 0;
    }

    pub fn set(&mut self, text: &str) {
        self.buf = text.chars().collect();
        self.pos = self.buf.len();
    }

    pub fn as_string(&self) -> String {
        self.buf.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.pos
    }

    pub fn insert(&mut self, c: char) {
        self.buf.insert(self.pos, c);
        self.pos += 1;
    }

    pub fn backspace(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
            self.buf.remove(self.pos);
        }
    }

    pub fn delete(&mut self) {
        if self.pos < self.buf.len() {
            self.buf.remove(self.pos);
        }
    }

    pub fn left(&mut self) {
        self.pos = self.pos.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.pos = (self.pos + 1).min(self.buf.len());
    }

    pub fn home(&mut self) {
        self.pos = 0;
    }

    pub fn end(&mut self) {
        self.pos = self.buf.len();
    }
}

/// Parse a time entered in the "Go to Time" dialog.
///
/// Accepts a bare number (interpreted as ticks) or a value with a unit
/// suffix such as `1.5us` / `250 ns`. Returns seconds.
pub fn parse_time_spec(s: &str) -> Result<f64, ()> {
    let s = s.trim();
    if s.is_empty() {
        return Err(());
    }
    if let Ok(v) = s.parse::<f64>() {
        return Ok(v);
    }
    let units = [
        ("us", 1e-6),
        ("ns", 1e-9),
        ("ps", 1e-12),
        ("ms", 1e-3),
        ("fs", 1e-15),
        ("s", 1.0),
    ];
    for (suffix, factor) in units {
        if let Some(digits) = s.strip_suffix(suffix) {
            if let Ok(v) = digits.trim().parse::<f64>() {
                return Ok(v * factor);
            }
        }
    }
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_operations() {
        let mut input = InputState::default();
        input.insert('a');
        input.insert('c');
        input.left();
        input.insert('b');
        assert_eq!(input.as_string(), "abc");
        assert_eq!(input.cursor(), 2);
        input.backspace();
        assert_eq!(input.as_string(), "ac");
        input.home();
        input.insert('0');
        assert_eq!(input.as_string(), "0ac");
        input.home();
        input.delete();
        assert_eq!(input.as_string(), "ac");
        input.delete();
        assert_eq!(input.as_string(), "c");
    }

    #[test]
    fn time_specs() {
        assert_eq!(parse_time_spec("1500"), Ok(1500.0));
        assert_eq!(parse_time_spec("1"), Ok(1.0));
        assert_eq!(parse_time_spec("1.5us"), Ok(1.5e-6));
        assert_eq!(parse_time_spec("2ms"), Ok(2e-3));
        let spaced = parse_time_spec("250 ns").unwrap();
        assert!((spaced - 2.5e-7).abs() < 1e-18, "got {spaced}");
        assert!(parse_time_spec("").is_err());
        assert!(parse_time_spec("abc").is_err());
        assert!(parse_time_spec("us").is_err());
    }
}
