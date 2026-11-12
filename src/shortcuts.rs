//! Launcher shortcuts use control letters that terminals transmit without
//! requiring macOS function-key settings or Option/Meta configuration.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    FocusActions,
    Actions,
    SelectText,
    SearchConsole,
    Animations,
    PreviousConsole,
    NextConsole,
    Devices,
    StopAll,
    AddSession,
    CloseSession,
}

impl Shortcut {
    pub fn from_key(key: KeyEvent) -> Option<Self> {
        if key.modifiers != KeyModifiers::CONTROL {
            return None;
        }
        Some(match key.code {
            KeyCode::Char('g') => Self::FocusActions,
            KeyCode::Char('p') => Self::Actions,
            KeyCode::Char('e') => Self::SelectText,
            KeyCode::Char('s') => Self::SearchConsole,
            KeyCode::Char('a') => Self::Animations,
            KeyCode::Char('b') => Self::PreviousConsole,
            KeyCode::Char('f') => Self::NextConsole,
            KeyCode::Char('d') => Self::Devices,
            KeyCode::Char('x') => Self::StopAll,
            KeyCode::Char('n') => Self::AddSession,
            KeyCode::Char('w') => Self::CloseSession,
            _ => return None,
        })
    }

    fn binding(self) -> (char, &'static str) {
        match self {
            Self::FocusActions => ('g', "Ctrl+g"),
            Self::Actions => ('p', "Ctrl+p"),
            Self::SelectText => ('e', "Ctrl+e"),
            Self::SearchConsole => ('s', "Ctrl+s"),
            Self::Animations => ('a', "Ctrl+a"),
            Self::PreviousConsole => ('b', "Ctrl+b"),
            Self::NextConsole => ('f', "Ctrl+f"),
            Self::Devices => ('d', "Ctrl+d"),
            Self::StopAll => ('x', "Ctrl+x"),
            Self::AddSession => ('n', "Ctrl+n"),
            Self::CloseSession => ('w', "Ctrl+w"),
        }
    }

    pub fn key(self) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(self.binding().0), KeyModifiers::CONTROL)
    }

    pub fn label(self) -> &'static str {
        self.binding().1
    }
}
