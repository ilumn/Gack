use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    MoveUp,
    MoveDown,
    PageUp,
    PageDown,
    FocusNext,
    FocusChanges,
    FocusDiff,
    ToggleMark,
    ClearMarks,
    Stage,
    Unstage,
    StageHunk,
    UnstageHunk,
    StageAllVisible,
    Discard,
    ToggleWhitespace,
    IncreaseContext,
    DecreaseContext,
    Commit,
    SubmitCommit,
    OpenChanges,
    OpenStash,
    OpenBranches,
    OpenLog,
    OpenRemotes,
    OpenRepos,
    ApplyStash,
    DropStash,
    SwitchBranch,
    FetchRemote,
    UpdateCurrentBranch,
    PushCurrentBranch,
    SyncSubmodule,
    RebaseContinue,
    RebaseAbort,
    RebaseSkip,
    ExternalEditor,
    ChooseOurs,
    ChooseTheirs,
    MarkResolved,
    StartInteractiveRebase,
    Refresh,
    Help,
    Palette,
    Close,
    Quit,
    NextHunk,
    PrevHunk,
    ScrollDiffUp,
    ScrollDiffDown,
    Text(char),
    Backspace,
    Newline,
    None,
}

pub fn key_to_action(key: KeyEvent) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => Action::Quit,
            KeyCode::Char('p') => Action::Palette,
            _ => Action::None,
        };
    }

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => Action::MoveUp,
        KeyCode::Down | KeyCode::Char('j') => Action::MoveDown,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::Tab => Action::FocusNext,
        KeyCode::BackTab => Action::FocusNext,
        KeyCode::Enter => Action::FocusDiff,
        KeyCode::Char(' ') | KeyCode::Char('m') => Action::ToggleMark,
        KeyCode::Char('M') => Action::ClearMarks,
        KeyCode::Char('s') => Action::Stage,
        KeyCode::Char('S') => Action::StageHunk,
        KeyCode::Char('u') => Action::Unstage,
        KeyCode::Char('U') => Action::UnstageHunk,
        KeyCode::Char('a') => Action::StageAllVisible,
        KeyCode::Char('x') => Action::Discard,
        KeyCode::Char('c') => Action::Commit,
        KeyCode::Char('0') => Action::OpenChanges,
        KeyCode::Char('z') => Action::OpenStash,
        KeyCode::Char('B') => Action::OpenBranches,
        KeyCode::Char('L') => Action::OpenLog,
        KeyCode::Char('F') => Action::OpenRemotes,
        KeyCode::Char('R') => Action::OpenRepos,
        KeyCode::Char('A') => Action::ApplyStash,
        KeyCode::Char('d') => Action::DropStash,
        KeyCode::Char('o') => Action::SwitchBranch,
        KeyCode::Char('f') => Action::FetchRemote,
        KeyCode::Char('P') => Action::PushCurrentBranch,
        KeyCode::Char('y') => Action::SyncSubmodule,
        KeyCode::Char('C') => Action::RebaseContinue,
        KeyCode::Char('Q') => Action::RebaseAbort,
        KeyCode::Char('K') => Action::RebaseSkip,
        KeyCode::Char('e') => Action::ExternalEditor,
        KeyCode::Char('1') => Action::ChooseOurs,
        KeyCode::Char('2') => Action::ChooseTheirs,
        KeyCode::Char('I') => Action::StartInteractiveRebase,
        KeyCode::Char('r') => Action::Refresh,
        KeyCode::Char('?') => Action::Help,
        KeyCode::Char(':') => Action::Palette,
        KeyCode::Esc => Action::Close,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('n') | KeyCode::Char(']') => Action::NextHunk,
        KeyCode::Char('p') | KeyCode::Char('[') => Action::PrevHunk,
        KeyCode::Char('w') => Action::ToggleWhitespace,
        KeyCode::Char('+') | KeyCode::Char('=') => Action::IncreaseContext,
        KeyCode::Char('-') => Action::DecreaseContext,
        KeyCode::Char('h') | KeyCode::Left => Action::ScrollDiffUp,
        KeyCode::Char('l') | KeyCode::Right => Action::ScrollDiffDown,
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Char(ch) => Action::Text(ch),
        _ => Action::None,
    }
}

pub fn key_to_text_action(key: KeyEvent) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => Action::Close,
            KeyCode::Char('e') => Action::ExternalEditor,
            KeyCode::Char('s') => Action::SubmitCommit,
            _ => Action::None,
        };
    }

    match key.code {
        KeyCode::Esc => Action::Close,
        KeyCode::Tab | KeyCode::BackTab => Action::FocusNext,
        KeyCode::Enter => Action::Newline,
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Char(ch) => Action::Text(ch),
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_mode_maps_uppercase_hunk_commands() {
        assert_eq!(
            key_to_action(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)),
            Action::StageHunk
        );
        assert_eq!(
            key_to_action(KeyEvent::new(KeyCode::Char('U'), KeyModifiers::SHIFT)),
            Action::UnstageHunk
        );
    }

    #[test]
    fn text_mode_keeps_shortcut_letters_as_text() {
        assert_eq!(
            key_to_text_action(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
            Action::Text('s')
        );
        assert_eq!(
            key_to_text_action(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)),
            Action::Text('S')
        );
        assert_eq!(
            key_to_text_action(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Action::Text('q')
        );
    }

    #[test]
    fn ctrl_s_submits_commit_in_text_mode() {
        assert_eq!(
            key_to_text_action(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Action::SubmitCommit
        );
    }

    #[test]
    fn plain_enter_still_adds_commit_body_newline() {
        assert_eq!(
            key_to_text_action(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Action::Newline
        );
        assert_eq!(
            key_to_text_action(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)),
            Action::None
        );
    }
}
