use crossterm::event::{KeyCode, KeyModifiers};

use crate::app::{Action, App, Popup};

/// Translates a raw key event into an `Action`, depending on whether a popup
/// currently has input focus (Input mode) or not (Normal mode).
pub fn map_key(app: &App, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    if let Some(popup) = &app.popup {
        // The move-target popup is a picker (list of changelists), not a
        // text field, so it also accepts j/k like the rest of the app.
        if matches!(popup, Popup::MoveFile { .. }) {
            if let Some(action) = map_picker_key(code) {
                return Some(action);
            }
        }
        return map_input_key(code);
    }
    map_normal_key(code, modifiers)
}

fn map_picker_key(code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Char('j') => Some(Action::MoveSelection(1)),
        KeyCode::Char('k') => Some(Action::MoveSelection(-1)),
        _ => None,
    }
}

fn map_normal_key(code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    match code {
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Quit),
        KeyCode::Tab => Some(Action::CyclePane),
        KeyCode::BackTab => Some(Action::CyclePaneBack),
        KeyCode::Char('j') | KeyCode::Down => Some(Action::MoveSelection(1)),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::MoveSelection(-1)),
        KeyCode::Char('l') | KeyCode::Right => Some(Action::ScrollHorizontal(1)),
        KeyCode::Char('h') | KeyCode::Left => Some(Action::ScrollHorizontal(-1)),
        KeyCode::Char('r') if modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Refresh),
        KeyCode::F(5) => Some(Action::Refresh),
        KeyCode::Enter => Some(Action::EnterFilesPane),
        KeyCode::Char('n') => Some(Action::OpenNewChangelist),
        KeyCode::Char('r') => Some(Action::OpenRename),
        KeyCode::Char('m') => Some(Action::OpenMove),
        KeyCode::Char('d') => Some(Action::OpenConfirmDelete),
        KeyCode::Char('c') => Some(Action::OpenCommit),
        KeyCode::Char('a') => Some(Action::SetActiveSelected),
        KeyCode::Char('v') => Some(Action::ToggleDiffMode),
        KeyCode::Char(' ') => Some(Action::ToggleStage),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        KeyCode::Char('V') => Some(Action::ToggleVisualMode),
        KeyCode::Esc => Some(Action::ExitVisualMode),
        _ => None,
    }
}

fn map_input_key(code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Up => Some(Action::MoveSelection(-1)),
        KeyCode::Down => Some(Action::MoveSelection(1)),
        KeyCode::Char(c) => Some(Action::InputChar(c)),
        _ => None,
    }
}
