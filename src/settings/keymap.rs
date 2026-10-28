use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    pub fn from_event(key: KeyEvent) -> Self {
        Self {
            code: key.code,
            modifiers: key.modifiers,
        }
    }

    pub fn matches(self, key: KeyEvent) -> bool {
        self.code == key.code && self.modifiers == key.modifiers
    }

    pub fn label(self) -> String {
        let mut label = String::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            label.push_str("Ctrl+");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            label.push_str("Shift+");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            label.push_str("Alt+");
        }
        match self.code {
            KeyCode::Char(' ') => label.push_str("Space"),
            KeyCode::Char(c) => label.push(c),
            KeyCode::F(number) => label.push_str(&format!("F{number}")),
            KeyCode::Enter => label.push_str("Enter"),
            KeyCode::Esc => label.push_str("Esc"),
            KeyCode::Tab => label.push_str("Tab"),
            KeyCode::BackTab => label.push_str("BackTab"),
            KeyCode::Backspace => label.push_str("Backspace"),
            KeyCode::Delete => label.push_str("Delete"),
            KeyCode::Insert => label.push_str("Insert"),
            KeyCode::Home => label.push_str("Home"),
            KeyCode::End => label.push_str("End"),
            KeyCode::Up => label.push_str("Up"),
            KeyCode::Down => label.push_str("Down"),
            KeyCode::Left => label.push_str("Left"),
            KeyCode::Right => label.push_str("Right"),
            KeyCode::PageUp => label.push_str("PgUp"),
            KeyCode::PageDown => label.push_str("PgDn"),
            other => label.push_str(&format!("{other:?}")),
        }
        label
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeymapSettings {
    overrides: HashMap<usize, KeyChord>,
    pub revision: u64,
}

impl KeymapSettings {
    pub fn get(&self, index: usize) -> Option<KeyChord> {
        self.overrides.get(&index).copied()
    }

    pub fn set(&mut self, index: usize, chord: KeyChord) -> bool {
        if self.get(index) == Some(chord) {
            return false;
        }
        self.overrides.insert(index, chord);
        self.bump();
        true
    }

    pub fn reset(&mut self, index: usize) -> bool {
        if self.overrides.remove(&index).is_none() {
            return false;
        }
        self.bump();
        true
    }

    pub fn reset_all(&mut self) -> bool {
        if self.is_empty() {
            return false;
        }
        self.overrides.clear();
        self.bump();
        true
    }

    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }

    pub fn len(&self) -> usize {
        self.overrides.len()
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_track_revision_and_reset() {
        let mut settings = KeymapSettings::default();
        let chord = KeyChord {
            code: KeyCode::F(5),
            modifiers: KeyModifiers::CONTROL,
        };
        assert!(settings.set(3, chord));
        assert_eq!(settings.revision, 1);
        assert_eq!(settings.get(3), Some(chord));
        assert_eq!(chord.label(), "Ctrl+F5");
        assert!(!settings.set(3, chord));
        assert!(settings.reset(3));
        assert!(settings.is_empty());
        assert!(settings.set(2, chord));
        assert!(settings.reset_all());
        assert_eq!(settings.revision, 4);
    }
}
