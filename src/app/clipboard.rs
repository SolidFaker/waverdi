use super::App;

pub fn osc52(text: &str) -> String {
    format!("\u{1b}]52;c;{}\u{7}", base64(text.as_bytes()))
}

pub fn take_clipboard(app: &mut App) -> Vec<String> {
    std::mem::take(&mut app.clipboard_writes)
}

impl App {
    pub(crate) fn copy_text(&mut self, text: String, description: &str) {
        if text.is_empty() {
            self.msg(format!("no {description} to copy"));
            return;
        }
        self.clipboard_writes.push(text.clone());
        self.msg(format!("copied {description}: {text}"));
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = (chunk[0] as u32) << 16
            | (chunk.get(1).copied().unwrap_or(0) as u32) << 8
            | chunk.get(2).copied().unwrap_or(0) as u32;
        out.push(ALPHABET[(value >> 18) as usize & 63] as char);
        out.push(ALPHABET[(value >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(value >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[value as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_encodes_utf8_as_base64() {
        assert_eq!(osc52("hello"), "\u{1b}]52;c;aGVsbG8=\u{7}");
        assert_eq!(osc52("f"), "\u{1b}]52;c;Zg==\u{7}");
        assert_eq!(osc52("fo"), "\u{1b}]52;c;Zm8=\u{7}");
        assert_eq!(osc52("foo"), "\u{1b}]52;c;Zm9v\u{7}");
    }
}
