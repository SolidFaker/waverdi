use super::Action;
use crate::settings::{KeyChord, KeymapSettings};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy)]
enum KeyPattern {
    Char(char),
    F(u8),
}

impl KeyPattern {
    fn matches(self, key: KeyEvent) -> bool {
        match self {
            Self::Char(expected) => key.code == KeyCode::Char(expected),
            Self::F(expected) => key.code == KeyCode::F(expected),
        }
    }
}

pub(crate) struct Binding {
    patterns: &'static [KeyPattern],
    pub(crate) action: Action,
    pub(crate) needs_waveform: bool,
    pub(crate) category: &'static str,
    pub(crate) keys: &'static str,
    pub(crate) description: &'static str,
}

const BINDINGS: &[Binding] = &[
    Binding {
        patterns: &[KeyPattern::Char('q')],
        action: Action::Quit,
        needs_waveform: false,
        category: "General",
        keys: "q",
        description: "quit",
    },
    Binding {
        patterns: &[KeyPattern::Char('o')],
        action: Action::Open,
        needs_waveform: false,
        category: "General",
        keys: "o",
        description: "open (system dialog when available)",
    },
    Binding {
        patterns: &[KeyPattern::Char('O')],
        action: Action::OpenTui,
        needs_waveform: false,
        category: "General",
        keys: "O",
        description: "open built-in browser",
    },
    Binding {
        patterns: &[KeyPattern::Char('A')],
        action: Action::AddSignals,
        needs_waveform: false,
        category: "General",
        keys: "Shift+A",
        description: "add signals (hierarchy picker)",
    },
    Binding {
        patterns: &[KeyPattern::F(2)],
        action: Action::Settings,
        needs_waveform: false,
        category: "General",
        keys: "F2",
        description: "settings",
    },
    Binding {
        patterns: &[KeyPattern::F(1), KeyPattern::Char('?')],
        action: Action::Keys,
        needs_waveform: false,
        category: "General",
        keys: "F1 / ?",
        description: "key bindings",
    },
    Binding {
        patterns: &[KeyPattern::Char(':')],
        action: Action::Goto,
        needs_waveform: true,
        category: "General",
        keys: ":",
        description: "go to time",
    },
    Binding {
        patterns: &[KeyPattern::Char('s')],
        action: Action::Find,
        needs_waveform: true,
        category: "Search",
        keys: "s",
        description: "search signals",
    },
    Binding {
        patterns: &[KeyPattern::Char('v')],
        action: Action::FindValue,
        needs_waveform: true,
        category: "Search",
        keys: "v",
        description: "find value",
    },
    Binding {
        patterns: &[KeyPattern::Char('n')],
        action: Action::FindNextValue,
        needs_waveform: true,
        category: "Search",
        keys: "n / N",
        description: "next / previous value match",
    },
    Binding {
        patterns: &[KeyPattern::Char('N')],
        action: Action::FindPrevValue,
        needs_waveform: true,
        category: "Search",
        keys: "n / N",
        description: "next / previous value match",
    },
    Binding {
        patterns: &[
            KeyPattern::Char('z'),
            KeyPattern::Char('='),
            KeyPattern::Char('+'),
        ],
        action: Action::ZoomIn,
        needs_waveform: true,
        category: "View",
        keys: "z / = / +",
        description: "zoom in",
    },
    Binding {
        patterns: &[KeyPattern::Char('Z'), KeyPattern::Char('-')],
        action: Action::ZoomOut,
        needs_waveform: true,
        category: "View",
        keys: "Z / -",
        description: "zoom out",
    },
    Binding {
        patterns: &[KeyPattern::Char('f')],
        action: Action::Fit,
        needs_waveform: true,
        category: "View",
        keys: "f",
        description: "fit time range",
    },
    Binding {
        patterns: &[KeyPattern::Char('c')],
        action: Action::Center,
        needs_waveform: true,
        category: "View",
        keys: "c",
        description: "center cursor",
    },
    Binding {
        patterns: &[KeyPattern::Char(',')],
        action: Action::Prev,
        needs_waveform: true,
        category: "View",
        keys: ", / .",
        description: "previous / next transition",
    },
    Binding {
        patterns: &[KeyPattern::Char('.')],
        action: Action::Next,
        needs_waveform: true,
        category: "View",
        keys: ", / .",
        description: "previous / next transition",
    },
];

pub(crate) fn bindings() -> &'static [Binding] {
    BINDINGS
}

pub fn resolve(key: KeyEvent, has_waveform: bool, settings: &KeymapSettings) -> Option<Action> {
    BINDINGS
        .iter()
        .enumerate()
        .find(|(index, binding)| {
            (has_waveform || !binding.needs_waveform)
                && match settings.get(*index) {
                    Some(chord) => chord.matches(key),
                    None => binding.patterns.iter().any(|pattern| pattern.matches(key)),
                }
        })
        .map(|(_, binding)| binding.action)
}

pub(crate) fn effective_keys(index: usize, settings: &KeymapSettings) -> String {
    settings
        .get(index)
        .map(KeyChord::label)
        .or_else(|| BINDINGS.get(index).map(|binding| binding.keys.to_string()))
        .unwrap_or_default()
}

pub(crate) fn validate_chord(
    index: usize,
    chord: KeyChord,
    settings: &KeymapSettings,
) -> Result<(), String> {
    let Some(binding) = BINDINGS.get(index) else {
        return Err("unknown key binding".to_string());
    };
    if is_reserved(chord) {
        return Err(format!(
            "{} is reserved by another pane or dialog",
            chord.label()
        ));
    }
    let key = KeyEvent::new(chord.code, chord.modifiers);
    if let Some((_, other)) = BINDINGS.iter().enumerate().find(|(other, binding)| {
        *other != index
            && match settings.get(*other) {
                Some(existing) => existing == chord,
                None => binding.patterns.iter().any(|pattern| pattern.matches(key)),
            }
    }) {
        return Err(format!(
            "{} is already used by {}",
            chord.label(),
            other.description
        ));
    }
    let _ = binding;
    Ok(())
}

fn is_reserved(chord: KeyChord) -> bool {
    if matches!(
        chord.code,
        KeyCode::Esc
            | KeyCode::Enter
            | KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown
    ) {
        return true;
    }
    if chord.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(chord.code, KeyCode::Char('c' | 'q' | 'w' | 'a'))
    {
        return true;
    }
    matches!(
        chord.code,
        KeyCode::Char(
            'g' | 'r'
                | '0'
                | '$'
                | 'G'
                | 'd'
                | 'p'
                | 'V'
                | 'x'
                | 'j'
                | 'k'
                | 'J'
                | 'K'
                | ' '
                | 'h'
                | 'l'
                | 'w'
                | 'e'
                | 'b'
                | 'a'
        )
    )
}

const EXTRA_HELP: &[&str] = &[
    "General   -f filelist / File>Load Filelist: RTL sources",
    "Add       Tab cycle panes   ↑ ↓ select   Space toggle",
    "          Tree: Space opens a scope, ← → folds it",
    "          Instances: Enter opens it; f cycles the filter",
    "          a applies, Enter applies and closes, Esc cancels",
    "View      h / l move cursor   ← → / Shift+← → x10",
    "          j / k next / previous row   gg / G first / last",
    "          J / K move signal / group down / up   Space select",
    "          w / b next / previous edge (1-bit: rising)",
    "          e / ge next / previous falling edge",
    "          0 / $ start / end of time",
    "          Time button opens the time-base menu",
    "          Home / End start / end   Tab cycle focus",
    "Source    j / k / h / l move   Enter / a adds the signal",
    "          ⇧↑↓ or drag select   Ctrl+W adds the selection",
    "          Ctrl+A selects the active module",
    "          double click picks a name   right-click: add",
    "Signals   V visual   dd cut   p paste   x cut (nWave)",
    "          Esc clear   r cycle radix / rename group",
    "          h full names (Signal List)   right-click: menu",
    "          Shift/Alt/Space multi-select   Ctrl+click range",
    "Groups    new signals go to the cursor group; G0 is the default",
    "          adding to the newest group appends a fresh group",
    "          Enter / ← → collapse / expand   r: rename group",
    "          right-click: New Group / Rename / Expand / Remove",
    "          ↑ ↓ / PgUp PgDn navigate lists",
    "Mouse     menus / toolbar: click to execute",
    "          drag pane borders: resize panes",
    "          drag scrollbars: scroll / pan time",
    "          drag list rows: reorder signals",
    "          Instance: click selects, double click folds a scope",
    "          Source: drag selects lines, double click a name;",
    "          right-click: Add to Waveform",
    "          click ruler: cursor   drag waveform: range",
    "          click inside a selection: zoom to it",
    "          wheel: zoom (waveform) / scroll (lists); Shift+wheel: pan",
    "          middle click: zoom out; dialogs: ✕ closes",
];

pub fn help_lines(settings: &KeymapSettings) -> Vec<String> {
    BINDINGS
        .iter()
        .enumerate()
        .map(|(index, binding)| {
            format!(
                "{:<10} {:<16} {}",
                binding.category,
                effective_keys(index, settings),
                binding.description
            )
        })
        .chain(EXTRA_HELP.iter().map(|line| (*line).to_string()))
        .collect()
}

pub fn help_len() -> usize {
    BINDINGS.len() + EXTRA_HELP.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_respect_waveform_availability_and_aliases() {
        let settings = KeymapSettings::default();
        let help = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
        let zoom = KeyEvent::new(KeyCode::Char('='), KeyModifiers::NONE);
        assert_eq!(resolve(help, false, &settings), Some(Action::Keys));
        assert_eq!(resolve(zoom, false, &settings), None);
        assert_eq!(resolve(zoom, true, &settings), Some(Action::ZoomIn));
    }

    #[test]
    fn overrides_replace_defaults_and_update_help() {
        let mut settings = KeymapSettings::default();
        let index = BINDINGS
            .iter()
            .position(|binding| binding.action == Action::ZoomIn)
            .unwrap();
        let chord = KeyChord {
            code: KeyCode::F(5),
            modifiers: KeyModifiers::NONE,
        };
        assert!(settings.set(index, chord));
        assert_eq!(effective_keys(index, &settings), "F5");
        assert_eq!(
            resolve(KeyEvent::new(chord.code, chord.modifiers), true, &settings),
            Some(Action::ZoomIn)
        );
        assert_eq!(
            resolve(
                KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
                true,
                &settings
            ),
            None
        );
        assert!(help_lines(&settings).iter().any(|line| line.contains("F5")));
        assert!(validate_chord(index, chord, &settings).is_ok());
        assert!(validate_chord(index + 1, chord, &settings).is_err());
        assert!(validate_chord(
            index,
            KeyChord::from_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            &settings
        )
        .is_err());
        assert!(settings.reset(index));
        assert_eq!(effective_keys(index, &settings), "z / = / +");
    }

    #[test]
    fn help_is_derived_from_the_binding_metadata() {
        let settings = KeymapSettings::default();
        let lines = help_lines(&settings);
        for binding in BINDINGS {
            assert!(lines.iter().any(|line| line.contains(binding.keys)));
            assert!(lines.iter().any(|line| line.contains(binding.description)));
        }
        assert_eq!(lines.len(), help_len());
    }
}
