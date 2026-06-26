use crate::action::Action;
use crate::commands::{self, CommandId};
use crate::config::Config;
use crate::git::{
    Change, ChangeSection, GitCli, GitError, changes_by_key, hunk_action_disabled_reason,
};
use crate::model::{
    AppState, Focus, Modal, Panel, PendingDiscard, PendingOperation, TerminalHandoff,
    TerminalHandoffAfter,
};
use std::ffi::OsString;
use std::fs;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct App {
    git: GitCli,
    pub config: Config,
    pub state: AppState,
    refresh_job: Option<BackgroundRefreshJob>,
    refresh_generation: u64,
}

struct BackgroundRefreshJob {
    receiver: Receiver<BackgroundRefreshResult>,
}

struct BackgroundRefreshResult {
    generation: u64,
    message: String,
    result: Result<RefreshPayload, GitError>,
}

struct RefreshPayload {
    snapshot: crate::git::StatusSnapshot,
    stashes: Option<Vec<crate::git::StashEntry>>,
    branches: Option<Vec<crate::git::BranchEntry>>,
    log: Option<Vec<crate::git::LogEntry>>,
    remotes: Option<Vec<crate::git::RemoteEntry>>,
    worktrees: Option<Vec<crate::git::WorktreeEntry>>,
    submodules: Option<Vec<crate::git::SubmoduleEntry>>,
    rebase_state: Option<crate::git::RebaseState>,
    current_upstream: Option<crate::git::UpstreamBranch>,
}

impl App {
    pub fn new(git: GitCli) -> Result<Self, GitError> {
        Self::with_config(git, Config::default())
    }

    pub fn with_config(git: GitCli, config: Config) -> Result<Self, GitError> {
        let snapshot = git.snapshot()?;
        let repo = git.repo_summary(&snapshot);
        let mut state = AppState::new(repo);
        state.changes = snapshot.changes;
        state.status_message = "Loaded repository status".to_string();
        let mut app = Self {
            git,
            config,
            state,
            refresh_job: None,
            refresh_generation: 0,
        };
        app.load_read_only_panels();
        app.load_active_inspector();
        Ok(app)
    }

    pub fn handle(&mut self, action: Action) {
        if self.state.modal != Modal::None {
            self.handle_modal(action);
            return;
        }

        match action {
            Action::MoveUp => self.move_up(),
            Action::MoveDown => self.move_down(),
            Action::PageUp => self.page_up(),
            Action::PageDown => self.page_down(),
            Action::FocusNext => self.toggle_focus(),
            Action::FocusChanges => self.state.focus = Focus::Changes,
            Action::FocusDiff => self.focus_inspector(),
            Action::ToggleMark => self.toggle_mark(),
            Action::ClearMarks => {
                self.state.marked.clear();
                self.state.status_message = "Cleared marks".to_string();
            }
            Action::Stage => {
                if self.state.panel == Panel::Repos {
                    self.confirm_stage_submodule_pointer();
                } else {
                    self.stage_selection();
                }
            }
            Action::Unstage => self.unstage_selection(),
            Action::StageHunk => {
                self.state.focus = Focus::Diff;
                self.stage_selected_hunk();
            }
            Action::UnstageHunk => match self.state.panel {
                Panel::Branches | Panel::Remotes => self.confirm_update_current_branch(),
                Panel::Repos => self.confirm_update_submodule(),
                _ => {
                    self.state.focus = Focus::Diff;
                    self.unstage_selected_hunk();
                }
            },
            Action::StageAllVisible => self.stage_all_visible(),
            Action::Discard => {
                if self.state.panel == Panel::Repos {
                    self.confirm_remove_worktree();
                } else {
                    self.open_discard_confirmation();
                }
            }
            Action::ToggleWhitespace => {
                self.state.focus = Focus::Diff;
                self.state.whitespace_mode = self.state.whitespace_mode.next();
                self.reset_inspector_position();
                self.load_active_inspector();
                self.state.status_message =
                    format!("Whitespace mode: {}", self.state.whitespace_mode.label());
            }
            Action::IncreaseContext => {
                self.state.focus = Focus::Diff;
                self.state.diff_context = self.state.diff_context.saturating_add(1).min(12);
                self.reset_inspector_position();
                self.load_active_inspector();
                self.state.status_message = format!("Diff context: {}", self.state.diff_context);
            }
            Action::DecreaseContext => {
                self.state.focus = Focus::Diff;
                self.state.diff_context = self.state.diff_context.saturating_sub(1).max(1);
                self.reset_inspector_position();
                self.load_active_inspector();
                self.state.status_message = format!("Diff context: {}", self.state.diff_context);
            }
            Action::Commit => self.open_commit(),
            Action::OpenChanges => self.open_panel(Panel::Changes),
            Action::OpenStash => self.open_panel(Panel::Stash),
            Action::OpenBranches => self.open_panel(Panel::Branches),
            Action::OpenLog => self.open_panel(Panel::Log),
            Action::OpenRemotes => self.open_panel(Panel::Remotes),
            Action::OpenRepos => self.open_panel(Panel::Repos),
            Action::ApplyStash => {
                if self.selected_change_is_conflict() {
                    self.confirm_mark_resolved();
                } else {
                    self.confirm_apply_stash();
                }
            }
            Action::DropStash => self.confirm_drop_stash(),
            Action::SwitchBranch => self.switch_selected_branch(),
            Action::FetchRemote => self.confirm_fetch_remote(),
            Action::UpdateCurrentBranch => self.confirm_update_current_branch(),
            Action::PushCurrentBranch => self.confirm_push_current_branch(),
            Action::SyncSubmodule => self.confirm_sync_submodule(),
            Action::RebaseContinue => self.rebase_continue(),
            Action::RebaseAbort => self.confirm_rebase_abort(),
            Action::RebaseSkip => self.confirm_rebase_skip(),
            Action::ExternalEditor => self.open_external_editor(),
            Action::ChooseOurs => self.confirm_checkout_ours(),
            Action::ChooseTheirs => self.confirm_checkout_theirs(),
            Action::MarkResolved => self.confirm_mark_resolved(),
            Action::StartInteractiveRebase => self.confirm_start_interactive_rebase(),
            Action::Refresh => self.refresh("Refreshed repository status"),
            Action::Help => self.state.modal = Modal::Help,
            Action::Palette => {
                self.state.palette_query.clear();
                self.state.modal = Modal::Palette;
            }
            Action::Quit => self.state.should_quit = true,
            Action::NextHunk => self.jump_hunk(1),
            Action::PrevHunk => self.jump_hunk(-1),
            Action::ScrollDiffUp => self.scroll_diff(-8),
            Action::ScrollDiffDown => self.scroll_diff(8),
            Action::Close => self.state.focus = Focus::Changes,
            Action::None
            | Action::Text(_)
            | Action::Backspace
            | Action::Newline
            | Action::SubmitCommit => {}
        }
    }

    pub fn command_disabled_reason(&self, id: CommandId) -> Option<String> {
        match id {
            CommandId::Stage | CommandId::Unstage | CommandId::Discard => {
                if self.state.panel != Panel::Changes {
                    Some("open Changes first".to_string())
                } else if self.state.selected_change().is_none() {
                    Some("no change selected".to_string())
                } else {
                    None
                }
            }
            CommandId::StageAll => {
                if self.state.panel != Panel::Changes {
                    Some("open Changes first".to_string())
                } else if self.state.changes.is_empty() {
                    Some("no visible changes".to_string())
                } else {
                    None
                }
            }
            CommandId::StageHunk => {
                let Some(change) = self.state.selected_change() else {
                    return Some("no change selected".to_string());
                };
                let Some(diff) = self.state.diff.as_ref() else {
                    return Some("no diff loaded".to_string());
                };
                hunk_action_disabled_reason(change, diff, self.state.diff_hunk, false)
            }
            CommandId::UnstageHunk => {
                if matches!(
                    self.state.panel,
                    Panel::Branches | Panel::Remotes | Panel::Repos
                ) {
                    return None;
                }
                let Some(change) = self.state.selected_change() else {
                    return Some("no change selected".to_string());
                };
                let Some(diff) = self.state.diff.as_ref() else {
                    return Some("no diff loaded".to_string());
                };
                hunk_action_disabled_reason(change, diff, self.state.diff_hunk, true)
            }
            CommandId::Commit => {
                (self.state.staged_count() == 0).then(|| "stage changes first".to_string())
            }
            CommandId::ApplyStash | CommandId::DropStash => {
                if self.state.panel != Panel::Stash {
                    Some("open Stash first".to_string())
                } else if self.state.selected_stash().is_none() {
                    Some("no stash selected".to_string())
                } else {
                    None
                }
            }
            CommandId::SwitchBranch => {
                if self.state.panel != Panel::Branches {
                    Some("open Branches first".to_string())
                } else if let Some(branch) = self.state.selected_branch() {
                    if branch.remote {
                        Some("remote branches are read-only".to_string())
                    } else if branch.current {
                        Some("already on this branch".to_string())
                    } else {
                        None
                    }
                } else {
                    Some("no branch selected".to_string())
                }
            }
            CommandId::FetchRemote => {
                if self.state.panel != Panel::Remotes {
                    Some("open Remotes first".to_string())
                } else if self.state.selected_remote().is_none() {
                    Some("no remote selected".to_string())
                } else {
                    None
                }
            }
            CommandId::UpdateCurrentBranch => {
                if !self.state.changes.is_empty() {
                    Some("working tree is not clean".to_string())
                } else if self.state.current_upstream.is_none() {
                    Some("no upstream".to_string())
                } else {
                    None
                }
            }
            CommandId::PushCurrentBranch => {
                if self.state.repo.behind.unwrap_or(0) > 0 {
                    Some("branch is behind upstream".to_string())
                } else if self.state.current_upstream.is_none() {
                    Some("no upstream".to_string())
                } else {
                    None
                }
            }
            CommandId::SyncSubmodule => self
                .state
                .selected_repo_submodule()
                .is_none()
                .then(|| "select a submodule".to_string()),
            CommandId::RebaseContinue | CommandId::RebaseAbort | CommandId::RebaseSkip => self
                .state
                .rebase_state
                .is_none()
                .then(|| "no rebase in progress".to_string()),
            CommandId::ExternalEditor => {
                if self.state.retry_handoff.is_some()
                    || self.selected_change_is_conflict()
                    || self.state.modal == Modal::Commit
                {
                    None
                } else if self.state.panel == Panel::Changes {
                    Some("select a conflict file or open Commit".to_string())
                } else {
                    Some("available for commit messages and conflicts".to_string())
                }
            }
            CommandId::StartInteractiveRebase => {
                if !self.state.changes.is_empty() {
                    Some("working tree is not clean".to_string())
                } else if self.state.current_upstream.is_none() {
                    Some("no upstream base".to_string())
                } else {
                    None
                }
            }
            CommandId::Changes
            | CommandId::Stash
            | CommandId::Branches
            | CommandId::Log
            | CommandId::Remotes
            | CommandId::Repos
            | CommandId::Refresh
            | CommandId::Whitespace
            | CommandId::IncreaseContext
            | CommandId::DecreaseContext
            | CommandId::Help => None,
        }
    }

    fn handle_modal(&mut self, action: Action) {
        match self.state.modal {
            Modal::Help => match action {
                Action::Quit | Action::Close | Action::Help => self.state.modal = Modal::None,
                _ => {}
            },
            Modal::Error => match action {
                Action::ExternalEditor => self.start_retry_handoff(),
                Action::Quit | Action::Close | Action::Help => self.state.modal = Modal::None,
                _ => {}
            },
            Modal::Palette => self.handle_palette(action),
            Modal::Commit => self.handle_commit(action),
            Modal::Confirm => self.handle_confirmation(action),
            Modal::None => {}
        }
    }

    fn handle_palette(&mut self, action: Action) {
        match action {
            Action::Close | Action::Quit => self.state.modal = Modal::None,
            Action::Backspace => {
                self.state.palette_query.pop();
            }
            Action::Text(ch) => self.state.palette_query.push(ch),
            Action::Newline | Action::FocusDiff | Action::SubmitCommit => {
                let query = self.state.palette_query.clone();
                self.state.modal = Modal::None;
                if let Some(command) = commands::first_matching_command(&query) {
                    if let Some(reason) = self.command_disabled_reason(command.id) {
                        self.state.status_message = format!("{} disabled: {reason}", command.label);
                        return;
                    }
                    self.handle(command.action);
                } else if query.contains("commit") {
                    self.open_commit();
                } else {
                    self.state.status_message = "No matching command".to_string();
                }
            }
            _ => {}
        }
    }

    fn handle_commit(&mut self, action: Action) {
        match action {
            Action::Close | Action::Quit => self.state.modal = Modal::None,
            Action::FocusNext => self.state.commit_body_focus = !self.state.commit_body_focus,
            Action::Backspace => {
                if self.state.commit_body_focus {
                    self.state.commit_body.pop();
                } else {
                    self.state.commit_summary.pop();
                }
            }
            Action::Text(ch) => {
                if self.state.commit_body_focus {
                    self.state.commit_body.push(ch);
                } else {
                    self.state.commit_summary.push(ch);
                }
            }
            Action::Newline | Action::FocusDiff => {
                if self.state.commit_body_focus {
                    self.state.commit_body.push('\n');
                } else {
                    self.state.commit_body_focus = true;
                }
            }
            Action::SubmitCommit => self.submit_commit(),
            Action::ExternalEditor => self.open_commit_external_editor(),
            _ => {}
        }
    }

    fn handle_confirmation(&mut self, action: Action) {
        match action {
            Action::Close | Action::Quit => {
                self.state.pending_discard.clear();
                self.state.pending_operation = None;
                self.state.modal = Modal::None;
                self.state.status_message = "Operation canceled".to_string();
            }
            Action::Newline | Action::FocusDiff | Action::SubmitCommit => {
                self.run_pending_operation()
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.state.changes.is_empty() {
            return;
        }
        let max = self.state.changes.len() as isize - 1;
        let next = (self.state.selected as isize + delta).clamp(0, max);
        if self.state.selected != next as usize {
            self.state.selected = next as usize;
            self.reset_inspector_position();
            self.load_active_inspector();
        }
    }

    fn reset_inspector_position(&mut self) {
        self.state.diff_scroll = 0;
        self.state.inspector_scroll = 0;
        self.state.diff_hunk = 0;
    }

    fn move_rail_selection(&mut self, delta: isize) {
        match self.state.panel {
            Panel::Changes => self.move_selection(delta),
            Panel::Stash => {
                self.state.selected_stash =
                    move_index(self.state.selected_stash, self.state.stashes.len(), delta);
                self.reset_inspector_position();
                self.load_active_inspector();
            }
            Panel::Branches => {
                self.state.selected_branch =
                    move_index(self.state.selected_branch, self.state.branches.len(), delta);
                self.reset_inspector_position();
                self.load_active_inspector();
            }
            Panel::Log => {
                self.state.selected_log =
                    move_index(self.state.selected_log, self.state.log.len(), delta);
                self.reset_inspector_position();
                self.load_active_inspector();
            }
            Panel::Remotes => {
                self.state.selected_remote =
                    move_index(self.state.selected_remote, self.state.remotes.len(), delta);
                self.reset_inspector_position();
                self.load_active_inspector();
            }
            Panel::Repos => {
                self.state.selected_repo = move_index(
                    self.state.selected_repo,
                    self.state.worktrees.len() + self.state.submodules.len(),
                    delta,
                );
                self.reset_inspector_position();
                self.load_active_inspector();
            }
        }
    }

    fn move_up(&mut self) {
        match self.state.focus {
            Focus::Changes => self.move_rail_selection(-1),
            Focus::Diff => self.scroll_inspector(-1),
        }
    }

    fn move_down(&mut self) {
        match self.state.focus {
            Focus::Changes => self.move_rail_selection(1),
            Focus::Diff => self.scroll_inspector(1),
        }
    }

    fn page_up(&mut self) {
        match self.state.focus {
            Focus::Changes => self.move_rail_selection(-10),
            Focus::Diff => self.scroll_inspector(-12),
        }
    }

    fn page_down(&mut self) {
        match self.state.focus {
            Focus::Changes => self.move_rail_selection(10),
            Focus::Diff => self.scroll_inspector(12),
        }
    }

    fn toggle_focus(&mut self) {
        self.state.focus = match self.state.focus {
            Focus::Changes if self.inspector_has_scrollable_content() => Focus::Diff,
            Focus::Changes => {
                self.state.status_message = "Inspector is visible; rail remains active".to_string();
                Focus::Changes
            }
            Focus::Diff => Focus::Changes,
        };
    }

    fn focus_inspector(&mut self) {
        if self.inspector_has_scrollable_content() {
            self.state.focus = Focus::Diff;
        } else {
            self.state.focus = Focus::Changes;
            self.state.status_message = "Inspector is visible; rail remains active".to_string();
        }
    }

    fn inspector_has_scrollable_content(&self) -> bool {
        match self.state.panel {
            Panel::Changes => self.state.selected_change().is_some(),
            Panel::Stash => self.state.selected_stash().is_some(),
            Panel::Branches => self.state.selected_branch().is_some(),
            Panel::Log => self.state.selected_log_entry().is_some(),
            Panel::Remotes => self.state.selected_remote().is_some(),
            Panel::Repos => {
                self.state.selected_repo_worktree().is_some()
                    || self.state.selected_repo_submodule().is_some()
            }
        }
    }

    fn toggle_mark(&mut self) {
        if self.state.panel != Panel::Changes {
            self.state.status_message = "Marks apply to the Changes panel".to_string();
            return;
        }
        if let Some(key) = self.state.selected_change_key()
            && !self.state.marked.remove(&key)
        {
            self.state.marked.insert(key);
        }
    }

    fn stage_selection(&mut self) {
        if self.state.panel != Panel::Changes {
            self.state.status_message = "Open Changes to stage files".to_string();
            return;
        }
        let changes = self.selected_or_marked_changes();
        if changes.is_empty() {
            self.state.status_message = "No change selected".to_string();
            return;
        }
        match self.git.stage(&changes) {
            Ok(()) => self.refresh("Staged selected changes"),
            Err(err) => self.show_error("Could not stage changes", err),
        }
    }

    fn stage_all_visible(&mut self) {
        if self.state.panel != Panel::Changes {
            self.state.status_message = "Open Changes to stage files".to_string();
            return;
        }
        let changes = self.state.changes.clone();
        match self.git.stage_all_visible(&changes) {
            Ok(()) => self.refresh("Staged all visible changes"),
            Err(err) => self.show_error("Could not stage all changes", err),
        }
    }

    fn unstage_selection(&mut self) {
        if self.state.panel != Panel::Changes {
            self.state.status_message = "Open Changes to unstage files".to_string();
            return;
        }
        let changes = self.selected_or_marked_changes();
        if changes.is_empty() {
            self.state.status_message = "No change selected".to_string();
            return;
        }
        match self.git.unstage(&changes) {
            Ok(()) => self.refresh("Unstaged selected changes"),
            Err(err) => self.show_error("Could not unstage changes", err),
        }
    }

    fn stage_selected_hunk(&mut self) {
        if self.state.panel != Panel::Changes {
            self.state.status_message = "Open Changes to stage hunks".to_string();
            return;
        }
        let Some(change) = self.state.selected_change().cloned() else {
            self.state.status_message = "No change selected".to_string();
            return;
        };
        let Some(diff) = self.state.diff.clone() else {
            self.state.status_message = "No diff loaded".to_string();
            return;
        };
        if let Some(reason) =
            hunk_action_disabled_reason(&change, &diff, self.state.diff_hunk, false)
        {
            self.state.status_message = format!("S disabled: {reason}");
            return;
        }
        match self.git.stage_hunk(&change, &diff, self.state.diff_hunk) {
            Ok(()) => self.refresh("Staged selected hunk"),
            Err(err) => self.show_error("Could not stage hunk", err),
        }
    }

    fn unstage_selected_hunk(&mut self) {
        if self.state.panel != Panel::Changes {
            self.state.status_message = "Open Changes to unstage hunks".to_string();
            return;
        }
        let Some(change) = self.state.selected_change().cloned() else {
            self.state.status_message = "No change selected".to_string();
            return;
        };
        let Some(diff) = self.state.diff.clone() else {
            self.state.status_message = "No diff loaded".to_string();
            return;
        };
        if let Some(reason) =
            hunk_action_disabled_reason(&change, &diff, self.state.diff_hunk, true)
        {
            self.state.status_message = format!("U disabled: {reason}");
            return;
        }
        match self.git.unstage_hunk(&change, &diff, self.state.diff_hunk) {
            Ok(()) => self.refresh("Unstaged selected hunk"),
            Err(err) => self.show_error("Could not unstage hunk", err),
        }
    }

    fn open_commit(&mut self) {
        if self.state.staged_count() == 0 {
            self.state.status_message = "Stage at least one change before committing".to_string();
            return;
        }
        self.state.commit_summary.clear();
        self.state.commit_body.clear();
        self.state.commit_body_focus = false;
        self.state.modal = Modal::Commit;
    }

    fn open_discard_confirmation(&mut self) {
        if self.state.panel != Panel::Changes {
            self.state.status_message = "Open Changes to discard files".to_string();
            return;
        }
        let changes = self.selected_or_marked_changes();
        if changes.is_empty() {
            self.state.status_message = "No change selected".to_string();
            return;
        }

        let unsupported = changes.iter().find(|change| {
            !matches!(
                change.section,
                ChangeSection::Unstaged | ChangeSection::Untracked
            )
        });
        if let Some(change) = unsupported {
            self.state.status_message = format!(
                "Cannot discard {} from {}; unstage first if needed",
                change.display_path(),
                change.section.label()
            );
            return;
        }

        let mut pending = Vec::new();
        for change in &changes {
            match self.git.discard_fingerprint(change) {
                Ok(fingerprint) => pending.push(PendingDiscard {
                    key: change.key.clone(),
                    fingerprint,
                }),
                Err(err) => {
                    self.show_error("Could not prepare discard confirmation", err);
                    return;
                }
            }
        }
        self.state.pending_discard = changes.iter().map(|change| change.key.clone()).collect();
        let title = if changes.len() == 1 {
            "Discard selected change?".to_string()
        } else {
            format!("Discard {} selected changes?", changes.len())
        };
        let mut body = String::from("This cannot be undone by gack.\n\n");
        for change in changes.iter().take(8) {
            body.push_str(change.kind.tag());
            body.push(' ');
            body.push_str(&change.display_path());
            body.push('\n');
        }
        if changes.len() > 8 {
            body.push_str("...\n");
        }
        self.open_confirmation(PendingOperation::Discard(pending), title, body);
    }

    fn run_pending_operation(&mut self) {
        let Some(operation) = self.state.pending_operation.clone() else {
            self.state.modal = Modal::None;
            self.state.status_message = "Nothing to do".to_string();
            return;
        };

        match operation {
            PendingOperation::Discard(targets) => self.discard_pending(&targets),
            PendingOperation::ApplyStash(selector) => match self.git.stash_apply(&selector) {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.refresh(&format!("Applied {selector}"));
                }
                Err(err) => self.show_error("Could not apply stash", err),
            },
            PendingOperation::DropStash(selector) => match self.git.stash_drop(&selector) {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.refresh(&format!("Dropped {selector}"));
                }
                Err(err) => self.show_error("Could not drop stash", err),
            },
            PendingOperation::SwitchBranch(name) => match self.git.switch_branch(&name) {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.open_panel(Panel::Changes);
                    self.refresh(&format!("Switched to {name}"));
                }
                Err(err) => self.show_error("Could not switch branch", err),
            },
            PendingOperation::FetchRemote(name) => match self.git.fetch_remote(&name) {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.refresh(&format!("Fetched {name}"));
                }
                Err(err) => self.show_error_with_retry(
                    "Could not fetch remote",
                    err,
                    self.git_handoff(
                        "terminal fetch retry",
                        vec!["fetch", "--prune", &name],
                        "Returned from terminal fetch",
                    ),
                ),
            },
            PendingOperation::UpdateCurrentBranch => match self.git.update_current_branch_ff_only()
            {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.refresh("Updated current branch");
                }
                Err(err) => self.show_error_with_retry(
                    "Could not update current branch",
                    err,
                    self.git_handoff(
                        "terminal fast-forward update retry",
                        vec!["pull", "--ff-only"],
                        "Returned from terminal update",
                    ),
                ),
            },
            PendingOperation::PushCurrentBranch => match self.git.push_current_branch() {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.refresh("Pushed current branch");
                }
                Err(err) => self.show_error_with_retry(
                    "Could not push current branch",
                    err,
                    self.git_handoff(
                        "terminal push retry",
                        vec!["push"],
                        "Returned from terminal push",
                    ),
                ),
            },
            PendingOperation::RemoveWorktree(path) => match self.git.remove_worktree(&path) {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.refresh("Removed worktree");
                }
                Err(err) => self.show_error("Could not remove worktree", err),
            },
            PendingOperation::SyncSubmodule(path) => match self.git.submodule_sync(&path) {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.refresh("Synced submodule");
                }
                Err(err) => self.show_error("Could not sync submodule", err),
            },
            PendingOperation::UpdateSubmodule(path) => {
                match self.git.submodule_update_local(&path) {
                    Ok(()) => {
                        self.state.modal = Modal::None;
                        self.state.pending_operation = None;
                        self.refresh("Updated submodule from local objects");
                    }
                    Err(err) => self.show_error("Could not update submodule", err),
                }
            }
            PendingOperation::StageSubmodulePointer(path) => {
                match self.git.stage_submodule_pointer(&path) {
                    Ok(()) => {
                        self.state.modal = Modal::None;
                        self.state.pending_operation = None;
                        self.refresh("Staged submodule pointer");
                    }
                    Err(err) => self.show_error("Could not stage submodule pointer", err),
                }
            }
            PendingOperation::RebaseAbort => match self.git.rebase_abort() {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.refresh("Aborted rebase");
                }
                Err(err) => self.show_error("Could not abort rebase", err),
            },
            PendingOperation::RebaseSkip => match self.git.rebase_skip() {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.refresh("Skipped rebase commit");
                }
                Err(err) => self.show_error("Could not skip rebase commit", err),
            },
            PendingOperation::CheckoutOurs(path) => match self.git.checkout_ours(&path) {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.refresh("Checked out ours");
                }
                Err(err) => self.show_error("Could not check out ours", err),
            },
            PendingOperation::CheckoutTheirs(path) => match self.git.checkout_theirs(&path) {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.refresh("Checked out theirs");
                }
                Err(err) => self.show_error("Could not check out theirs", err),
            },
            PendingOperation::MarkResolved(path) => match self.git.mark_resolved(&path) {
                Ok(()) => {
                    self.state.modal = Modal::None;
                    self.state.pending_operation = None;
                    self.refresh("Marked conflict resolved");
                }
                Err(err) => self.show_error("Could not mark resolved", err),
            },
            PendingOperation::StartInteractiveRebase(base) => {
                self.state.modal = Modal::None;
                self.state.pending_operation = None;
                self.state.pending_handoff = Some(TerminalHandoff {
                    cwd: self.git.root().to_path_buf(),
                    command: OsString::from("git"),
                    args: vec![
                        OsString::from("-C"),
                        self.git.root().as_os_str().to_os_string(),
                        std::ffi::OsString::from("rebase"),
                        std::ffi::OsString::from("-i"),
                        std::ffi::OsString::from(base),
                    ],
                    label: "interactive rebase".to_string(),
                    after: TerminalHandoffAfter::Refresh(
                        "Returned from interactive rebase".to_string(),
                    ),
                });
                self.state.status_message = "Starting terminal handoff".to_string();
            }
        }
    }

    fn discard_pending(&mut self, targets: &[PendingDiscard]) {
        let keys = targets
            .iter()
            .map(|target| target.key.clone())
            .collect::<Vec<_>>();
        let changes = changes_by_key(&self.state.changes, &keys);
        if changes.is_empty() {
            self.state.pending_discard.clear();
            self.state.pending_operation = None;
            self.state.modal = Modal::None;
            self.state.status_message = "Nothing to discard".to_string();
            return;
        }

        for target in targets {
            let Some(change) = changes.iter().find(|change| change.key == target.key) else {
                self.cancel_stale_discard();
                return;
            };
            match self.git.discard_fingerprint(change) {
                Ok(fingerprint) if fingerprint == target.fingerprint => {}
                Ok(_) | Err(_) => {
                    self.cancel_stale_discard();
                    return;
                }
            }
        }

        match self.git.discard(&changes) {
            Ok(()) => {
                self.state.pending_discard.clear();
                self.state.pending_operation = None;
                self.state.modal = Modal::None;
                self.refresh("Discarded selected changes");
            }
            Err(err) => self.show_error("Could not discard changes", err),
        }
    }

    fn cancel_stale_discard(&mut self) {
        self.state.pending_discard.clear();
        self.state.pending_operation = None;
        self.state.modal = Modal::None;
        self.refresh("Discard canceled: selection changed after confirmation");
    }

    fn open_confirmation(&mut self, operation: PendingOperation, title: String, body: String) {
        self.state.pending_operation = Some(operation);
        self.state.confirm_title = title;
        self.state.confirm_body = body;
        self.state.modal = Modal::Confirm;
    }

    fn confirm_apply_stash(&mut self) {
        if self.state.panel != Panel::Stash {
            self.state.status_message = "Open Stash to apply a stash".to_string();
            return;
        }
        let Some(stash) = self.state.selected_stash().cloned() else {
            self.state.status_message = "No stash selected".to_string();
            return;
        };
        self.open_confirmation(
            PendingOperation::ApplyStash(stash.selector.clone()),
            format!("Apply {}?", stash.selector),
            format!(
                "{}\n\nApplying a stash changes the working tree and may create conflicts.",
                stash.subject
            ),
        );
    }

    fn confirm_drop_stash(&mut self) {
        if self.state.panel != Panel::Stash {
            self.state.status_message = "Open Stash to drop a stash".to_string();
            return;
        }
        let Some(stash) = self.state.selected_stash().cloned() else {
            self.state.status_message = "No stash selected".to_string();
            return;
        };
        self.open_confirmation(
            PendingOperation::DropStash(stash.selector.clone()),
            format!("Drop {}?", stash.selector),
            format!(
                "{}\n\nDropping a stash removes it from the stash list.",
                stash.subject
            ),
        );
    }

    fn switch_selected_branch(&mut self) {
        if self.state.panel != Panel::Branches {
            self.state.status_message = "Open Branches to switch branches".to_string();
            return;
        }
        let Some(branch) = self.state.selected_branch().cloned() else {
            self.state.status_message = "No branch selected".to_string();
            return;
        };
        if branch.remote {
            self.state.status_message = "Remote branches are read-only for now".to_string();
            return;
        }
        if branch.current {
            self.state.status_message = format!("Already on {}", branch.name);
            return;
        }
        self.open_confirmation(
            PendingOperation::SwitchBranch(branch.name.clone()),
            format!("Switch to {}?", branch.name),
            "Git will reject the switch if local changes would be overwritten.".to_string(),
        );
    }

    fn confirm_fetch_remote(&mut self) {
        if self.state.panel != Panel::Remotes {
            self.state.status_message = "Open Remotes to fetch a remote".to_string();
            return;
        }
        let Some(remote) = self.state.selected_remote().cloned() else {
            self.state.status_message = "No remote selected".to_string();
            return;
        };
        self.open_confirmation(
            PendingOperation::FetchRemote(remote.name.clone()),
            format!("Fetch {}?", remote.name),
            "This contacts the remote and prunes stale remote-tracking refs.".to_string(),
        );
    }

    fn confirm_update_current_branch(&mut self) {
        if !self.state.changes.is_empty() {
            self.state.status_message =
                "Update requires a clean working tree and index".to_string();
            return;
        }
        let Ok(upstream) = self.git.current_upstream() else {
            self.state.status_message = "Current branch has no upstream".to_string();
            return;
        };
        let head = self
            .git
            .rev_short("HEAD")
            .unwrap_or_else(|_| "HEAD".to_string());
        let upstream_head = self
            .git
            .rev_short("@{u}")
            .unwrap_or_else(|_| upstream.display.clone());
        let behind = self.state.repo.behind.unwrap_or(0);
        self.open_confirmation(
            PendingOperation::UpdateCurrentBranch,
            format!(
                "Fast-forward {} from {}?",
                self.state.repo.branch, upstream.display
            ),
            format!(
                "Current HEAD: {head}\nKnown upstream HEAD: {upstream_head}\nKnown behind: {behind}\n\n\
gack will fetch {} and re-check upstream before merging. It will only run a fast-forward merge and will not create a merge commit.",
                upstream.remote
            ),
        );
    }

    fn confirm_push_current_branch(&mut self) {
        if self.state.repo.behind.unwrap_or(0) > 0 {
            self.state.status_message = "Push disabled: branch is behind upstream".to_string();
            return;
        }
        let Ok(upstream) = self.git.current_upstream() else {
            self.state.status_message = "Current branch has no upstream".to_string();
            return;
        };
        let head = self
            .git
            .rev_short("HEAD")
            .unwrap_or_else(|_| "HEAD".to_string());
        let ahead = self.state.repo.ahead.unwrap_or(0);
        self.open_confirmation(
            PendingOperation::PushCurrentBranch,
            format!("Push {} to {}?", self.state.repo.branch, upstream.display),
            format!(
                "Source: {} at {head}\nDestination: refs/heads/{}\nKnown ahead: {ahead}\nForce: no\n\n\
gack re-checks the upstream ref before pushing and refuses if the remote is ahead.",
                self.state.repo.branch, upstream.branch
            ),
        );
    }

    fn confirm_remove_worktree(&mut self) {
        let Some(worktree) = self.state.selected_repo_worktree().cloned() else {
            self.state.status_message = "Select a worktree to remove".to_string();
            return;
        };
        if worktree.current {
            self.state.status_message = "Cannot remove the current worktree".to_string();
            return;
        }
        self.open_confirmation(
            PendingOperation::RemoveWorktree(worktree.path.clone()),
            format!("Remove worktree {}?", worktree.path.to_string_lossy()),
            "Git will reject removal if the worktree contains local changes. Force removal is not used."
                .to_string(),
        );
    }

    fn confirm_sync_submodule(&mut self) {
        let Some(submodule) = self.state.selected_repo_submodule().cloned() else {
            self.state.status_message = "Select a submodule to sync".to_string();
            return;
        };
        self.open_confirmation(
            PendingOperation::SyncSubmodule(submodule.path.clone()),
            format!("Sync submodule {}?", submodule.path.to_string_lossy()),
            "This updates local submodule URL metadata from .gitmodules.".to_string(),
        );
    }

    fn confirm_update_submodule(&mut self) {
        let Some(submodule) = self.state.selected_repo_submodule().cloned() else {
            self.state.status_message = "Select a submodule to update".to_string();
            return;
        };
        self.open_confirmation(
            PendingOperation::UpdateSubmodule(submodule.path.clone()),
            format!("Update submodule {}?", submodule.path.to_string_lossy()),
            "This runs submodule update with --no-fetch, so it only uses local objects."
                .to_string(),
        );
    }

    fn confirm_stage_submodule_pointer(&mut self) {
        let Some(submodule) = self.state.selected_repo_submodule().cloned() else {
            self.state.status_message = "Select a submodule to stage its pointer".to_string();
            return;
        };
        self.open_confirmation(
            PendingOperation::StageSubmodulePointer(submodule.path.clone()),
            format!(
                "Stage submodule pointer {}?",
                submodule.path.to_string_lossy()
            ),
            "This stages the superproject's recorded submodule commit.".to_string(),
        );
    }

    fn rebase_continue(&mut self) {
        if self.state.rebase_state.is_none() {
            self.state.status_message = "No rebase in progress".to_string();
            return;
        }
        match self.git.rebase_continue() {
            Ok(()) => self.refresh("Continued rebase"),
            Err(err) => self.show_error("Could not continue rebase", err),
        }
    }

    fn confirm_rebase_abort(&mut self) {
        if self.state.rebase_state.is_none() {
            self.state.status_message = "No rebase in progress".to_string();
            return;
        }
        self.open_confirmation(
            PendingOperation::RebaseAbort,
            "Abort rebase?".to_string(),
            "This returns the branch to the state before the rebase started.".to_string(),
        );
    }

    fn confirm_rebase_skip(&mut self) {
        let Some(rebase) = self.state.rebase_state.as_ref() else {
            self.state.status_message = "No rebase in progress".to_string();
            return;
        };
        let current = rebase.current.as_deref().unwrap_or("current commit");
        self.open_confirmation(
            PendingOperation::RebaseSkip,
            format!("Skip {current}?"),
            "This drops the current rebase commit and continues with the next command.".to_string(),
        );
    }

    fn selected_change_is_conflict(&self) -> bool {
        self.state.panel == Panel::Changes
            && self
                .state
                .selected_change()
                .is_some_and(|change| change.is_conflict())
    }

    fn selected_conflict_path(&self) -> Option<std::path::PathBuf> {
        self.state
            .selected_change()
            .filter(|change| change.is_conflict())
            .map(|change| change.path.clone())
    }

    fn confirm_checkout_ours(&mut self) {
        let Some(path) = self.selected_conflict_path() else {
            self.state.status_message = "Select a conflict file first".to_string();
            return;
        };
        self.open_confirmation(
            PendingOperation::CheckoutOurs(path.clone()),
            format!("Use ours for {}?", path.to_string_lossy()),
            "This replaces the worktree file with stage 2 for this path. Review before marking resolved."
                .to_string(),
        );
    }

    fn confirm_checkout_theirs(&mut self) {
        let Some(path) = self.selected_conflict_path() else {
            self.state.status_message = "Select a conflict file first".to_string();
            return;
        };
        self.open_confirmation(
            PendingOperation::CheckoutTheirs(path.clone()),
            format!("Use theirs for {}?", path.to_string_lossy()),
            "This replaces the worktree file with stage 3 for this path. Review before marking resolved."
                .to_string(),
        );
    }

    fn confirm_mark_resolved(&mut self) {
        let Some(path) = self.selected_conflict_path() else {
            self.state.status_message = "Select a conflict file first".to_string();
            return;
        };
        self.open_confirmation(
            PendingOperation::MarkResolved(path.clone()),
            format!("Mark {} resolved?", path.to_string_lossy()),
            "This stages the file as the conflict resolution.".to_string(),
        );
    }

    fn confirm_start_interactive_rebase(&mut self) {
        if !self.state.changes.is_empty() {
            self.state.status_message =
                "Interactive rebase requires a clean working tree and index".to_string();
            return;
        }
        let Ok(upstream) = self.git.current_upstream() else {
            self.state.status_message = "Current branch has no upstream rebase base".to_string();
            return;
        };
        self.open_confirmation(
            PendingOperation::StartInteractiveRebase(upstream.display.clone()),
            format!("Start interactive rebase after {}?", upstream.display),
            "gack will leave the TUI, run git rebase -i in your terminal, then restore and refresh."
                .to_string(),
        );
    }

    pub fn take_terminal_handoff(&mut self) -> Option<TerminalHandoff> {
        self.state.pending_handoff.take()
    }

    pub fn watch_paths(&self) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        (
            self.git.root().to_path_buf(),
            self.git.git_dir().to_path_buf(),
            self.git.common_git_dir().to_path_buf(),
        )
    }

    pub fn complete_terminal_handoff(
        &mut self,
        after: TerminalHandoffAfter,
        success: bool,
        detail: &str,
    ) {
        match after {
            TerminalHandoffAfter::Refresh(message) => {
                let message = if success {
                    message
                } else {
                    format!("{message}: {detail}")
                };
                self.refresh(&message);
            }
            TerminalHandoffAfter::LoadCommitMessage(path) => {
                if !success {
                    let _ = fs::remove_file(&path);
                    self.state.status_message =
                        format!("Editor did not update commit draft: {detail}");
                    self.state.modal = Modal::Commit;
                    return;
                }
                match fs::read_to_string(&path) {
                    Ok(text) => {
                        let (summary, body) = split_commit_message(&text);
                        self.state.commit_summary = summary;
                        self.state.commit_body = body;
                        self.state.commit_body_focus = false;
                        self.state.modal = Modal::Commit;
                        self.state.status_message =
                            "Loaded commit draft from external editor".to_string();
                    }
                    Err(err) => {
                        self.state.status_message =
                            format!("Could not read edited commit draft: {err}");
                        self.state.modal = Modal::Commit;
                    }
                }
                let _ = fs::remove_file(path);
            }
            TerminalHandoffAfter::Commit(path) => {
                let _ = fs::remove_file(path);
                if success {
                    self.state.modal = Modal::None;
                    let hash = self
                        .git
                        .rev_short("HEAD")
                        .unwrap_or_else(|_| "HEAD".to_string());
                    self.refresh(&format!("Committed {hash}"));
                } else {
                    self.state.modal = Modal::Commit;
                    self.state.status_message = format!("Could not commit: {detail}");
                }
            }
        }
    }

    fn open_external_editor(&mut self) {
        if self.state.retry_handoff.is_some() {
            self.start_retry_handoff();
            return;
        }
        if self.selected_change_is_conflict() {
            self.open_conflict_external_editor();
        } else {
            self.state.status_message =
                "External editor is available for commit drafts and conflict files".to_string();
        }
    }

    fn open_conflict_external_editor(&mut self) {
        let Some(path) = self.selected_conflict_path() else {
            self.state.status_message = "Select a conflict file first".to_string();
            return;
        };
        let target = self.git.root().join(&path);
        self.queue_external_editor(
            "conflict editor",
            target.as_os_str().to_os_string(),
            TerminalHandoffAfter::Refresh("Returned from conflict editor".to_string()),
        );
    }

    fn open_commit_external_editor(&mut self) {
        let path = temp_commit_message_path();
        let draft = self.commit_message_draft();
        if let Err(err) = create_commit_message_draft(&path, &draft) {
            self.state.status_message = format!("Could not prepare editor draft: {err}");
            return;
        }
        self.queue_external_editor(
            "commit editor",
            path.as_os_str().to_os_string(),
            TerminalHandoffAfter::LoadCommitMessage(path),
        );
    }

    fn queue_external_editor(
        &mut self,
        label: &str,
        target: OsString,
        after: TerminalHandoffAfter,
    ) {
        let Some((command, mut args)) = editor_command() else {
            self.state.status_message =
                "Set GIT_EDITOR, VISUAL, or EDITOR to use an external editor".to_string();
            return;
        };
        args.push(target);
        self.state.pending_handoff = Some(TerminalHandoff {
            cwd: self.git.root().to_path_buf(),
            command,
            args,
            label: label.to_string(),
            after,
        });
        self.state.status_message = format!("Starting {label}");
    }

    fn start_retry_handoff(&mut self) {
        let Some(handoff) = self.state.retry_handoff.take() else {
            self.state.status_message = "No terminal retry is available".to_string();
            return;
        };
        let label = handoff.label.clone();
        self.state.modal = Modal::None;
        self.state.pending_handoff = Some(handoff);
        self.state.status_message = format!("Starting {label}");
    }

    fn git_handoff(
        &self,
        label: &str,
        git_args: Vec<&str>,
        refresh_message: &str,
    ) -> TerminalHandoff {
        let mut args = vec![
            OsString::from("-C"),
            self.git.root().as_os_str().to_os_string(),
        ];
        args.extend(git_args.into_iter().map(OsString::from));
        TerminalHandoff {
            cwd: self.git.root().to_path_buf(),
            command: OsString::from("git"),
            args,
            label: label.to_string(),
            after: TerminalHandoffAfter::Refresh(refresh_message.to_string()),
        }
    }

    fn commit_message_draft(&self) -> String {
        let mut draft = self.state.commit_summary.clone();
        if !self.state.commit_body.is_empty() {
            if !draft.is_empty() {
                draft.push_str("\n\n");
            }
            draft.push_str(&self.state.commit_body);
        }
        draft.push('\n');
        draft
    }

    fn submit_commit(&mut self) {
        let summary = self.state.commit_summary.trim();
        if summary.is_empty() {
            self.state.status_message = "Commit summary cannot be empty".to_string();
            return;
        }
        if self.state.staged_count() == 0 {
            self.state.status_message = "Stage at least one change before committing".to_string();
            return;
        }

        let mut message = summary.to_string();
        let body = self.state.commit_body.trim();
        if !body.is_empty() {
            message.push_str("\n\n");
            message.push_str(body);
        }

        if !message.ends_with('\n') {
            message.push('\n');
        }

        let path = temp_commit_message_path();
        if let Err(err) = create_commit_message_draft(&path, &message) {
            self.state.status_message = format!("Could not create commit message file: {err}");
            return;
        }
        let mut args = vec![
            OsString::from("-C"),
            self.git.root().as_os_str().to_os_string(),
            OsString::from("commit"),
            OsString::from("--file"),
        ];
        args.push(path.as_os_str().to_os_string());
        self.state.pending_handoff = Some(TerminalHandoff {
            cwd: self.git.root().to_path_buf(),
            command: OsString::from("git"),
            args,
            label: "git commit".to_string(),
            after: TerminalHandoffAfter::Commit(path),
        });
        self.state.status_message = "Starting git commit".to_string();
    }

    pub fn refresh(&mut self, message: &str) {
        self.refresh_generation = self.refresh_generation.saturating_add(1);
        self.refresh_job = None;
        let selected_key = self.state.selected_change_key();
        let selected_index = self.state.selected;
        match self.git.snapshot() {
            Ok(snapshot) => {
                let payload = self.collect_refresh_payload_from_snapshot(snapshot);
                self.apply_refresh_payload(payload, selected_key.as_ref(), selected_index, message);
            }
            Err(err) => self.show_error("Could not refresh repository", err),
        }
    }

    pub fn request_background_refresh(&mut self, message: &str) -> bool {
        if self.refresh_job.is_some() {
            return false;
        }
        self.refresh_generation = self.refresh_generation.saturating_add(1);
        let generation = self.refresh_generation;
        let git = self.git.clone();
        let message = message.to_string();
        let show_status = !message.is_empty();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = collect_refresh_payload(&git);
            let _ = sender.send(BackgroundRefreshResult {
                generation,
                message,
                result,
            });
        });
        self.refresh_job = Some(BackgroundRefreshJob { receiver });
        if show_status {
            self.state.status_message = "Refreshing repository status...".to_string();
        }
        true
    }

    pub fn poll_background_refresh(&mut self) {
        let received = match self.refresh_job.as_ref() {
            Some(job) => match job.receiver.try_recv() {
                Ok(result) => Some(Ok(result)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(())),
            },
            None => None,
        };

        match received {
            Some(Ok(result)) => {
                self.refresh_job = None;
                if result.generation != self.refresh_generation {
                    return;
                }
                match result.result {
                    Ok(payload) => {
                        let selected_key = self.state.selected_change_key();
                        let selected_index = self.state.selected;
                        self.apply_refresh_payload(
                            payload,
                            selected_key.as_ref(),
                            selected_index,
                            &result.message,
                        );
                    }
                    Err(err) => self.show_error("Could not refresh repository", err),
                }
            }
            Some(Err(())) => {
                self.refresh_job = None;
                self.state.status_message = "Background refresh stopped".to_string();
            }
            None => {}
        }
    }

    fn collect_refresh_payload_from_snapshot(
        &self,
        snapshot: crate::git::StatusSnapshot,
    ) -> RefreshPayload {
        RefreshPayload {
            snapshot,
            stashes: self.git.stash_list().ok(),
            branches: self.git.branch_list().ok(),
            log: self.git.log_list().ok(),
            remotes: self.git.remote_list().ok(),
            worktrees: self.git.worktree_list().ok(),
            submodules: self.git.submodule_list().ok(),
            rebase_state: self.git.rebase_state().ok().flatten(),
            current_upstream: self.git.current_upstream().ok(),
        }
    }

    fn apply_refresh_payload(
        &mut self,
        payload: RefreshPayload,
        selected_key: Option<&crate::git::ChangeKey>,
        selected_index: usize,
        message: &str,
    ) {
        self.state.repo = self.git.repo_summary(&payload.snapshot);
        self.state.changes = payload.snapshot.changes;
        self.state
            .restore_selection_by_key(selected_key, selected_index);
        self.state.retain_valid_marks();
        if let Some(stashes) = payload.stashes {
            self.state.stashes = stashes;
        }
        if let Some(branches) = payload.branches {
            self.state.branches = branches;
        }
        if let Some(log) = payload.log {
            self.state.log = log;
        }
        if let Some(remotes) = payload.remotes {
            self.state.remotes = remotes;
        }
        if let Some(worktrees) = payload.worktrees {
            self.state.worktrees = worktrees;
        }
        if let Some(submodules) = payload.submodules {
            self.state.submodules = submodules;
        }
        self.state.rebase_state = payload.rebase_state;
        self.state.current_upstream = payload.current_upstream;
        self.state.clamp_panel_selection();
        if !message.is_empty() {
            self.state.status_message = message.to_string();
        }
        self.load_active_inspector();
    }

    fn load_read_only_panels(&mut self) {
        if let Ok(stashes) = self.git.stash_list() {
            self.state.stashes = stashes;
        }
        if let Ok(branches) = self.git.branch_list() {
            self.state.branches = branches;
        }
        if let Ok(log) = self.git.log_list() {
            self.state.log = log;
        }
        if let Ok(remotes) = self.git.remote_list() {
            self.state.remotes = remotes;
        }
        if let Ok(worktrees) = self.git.worktree_list() {
            self.state.worktrees = worktrees;
        }
        if let Ok(submodules) = self.git.submodule_list() {
            self.state.submodules = submodules;
        }
        if let Ok(rebase_state) = self.git.rebase_state() {
            self.state.rebase_state = rebase_state;
        }
        self.state.current_upstream = self.git.current_upstream().ok();
        self.state.clamp_panel_selection();
    }

    fn open_panel(&mut self, panel: Panel) {
        self.state.panel = panel;
        self.state.focus = Focus::Changes;
        self.reset_inspector_position();
        self.load_read_only_panels();
        self.load_active_inspector();
        self.state.status_message = format!("Opened {}", panel.label());
    }

    fn load_active_inspector(&mut self) {
        match self.state.panel {
            Panel::Changes => self.load_selected_diff(),
            Panel::Stash => self.load_selected_stash_patch(),
            Panel::Log => self.load_selected_log_patch(),
            Panel::Branches | Panel::Remotes | Panel::Repos => {
                self.state.diff = None;
                self.state.stash_patch = None;
                self.state.log_patch = None;
            }
        }
    }

    fn load_selected_diff(&mut self) {
        let Some(change) = self.state.selected_change().cloned() else {
            self.state.diff = None;
            return;
        };

        match self.git.diff_with_options(
            &change,
            self.state.diff_context,
            self.state.whitespace_mode,
        ) {
            Ok(diff) => self.state.diff = Some(diff),
            Err(err) => {
                self.state.diff = None;
                self.state.status_message = format!("Diff unavailable: {err}");
            }
        }
        self.state.conflict_detail = if change.is_conflict() {
            self.git.conflict_detail(&change.path).ok()
        } else {
            None
        };
    }

    fn load_selected_stash_patch(&mut self) {
        self.state.diff = None;
        self.state.log_patch = None;
        let Some(selector) = self
            .state
            .selected_stash()
            .map(|stash| stash.selector.clone())
        else {
            self.state.stash_patch = None;
            return;
        };
        match self.git.stash_patch(&selector) {
            Ok(diff) => self.state.stash_patch = Some(diff),
            Err(err) => {
                self.state.stash_patch = None;
                self.state.status_message = format!("Stash patch unavailable: {err}");
            }
        }
    }

    fn load_selected_log_patch(&mut self) {
        self.state.diff = None;
        self.state.stash_patch = None;
        let Some(oid) = self
            .state
            .selected_log_entry()
            .map(|entry| entry.oid.clone())
        else {
            self.state.log_patch = None;
            return;
        };
        match self.git.commit_patch(&oid) {
            Ok(diff) => self.state.log_patch = Some(diff),
            Err(err) => {
                self.state.log_patch = None;
                self.state.status_message = format!("Commit patch unavailable: {err}");
            }
        }
    }

    fn active_diff(&self) -> Option<&crate::git::Diff> {
        match self.state.panel {
            Panel::Changes => self.state.diff.as_ref(),
            Panel::Stash => self.state.stash_patch.as_ref(),
            Panel::Log => self.state.log_patch.as_ref(),
            Panel::Branches | Panel::Remotes | Panel::Repos => None,
        }
    }

    fn jump_hunk(&mut self, delta: isize) {
        let Some(diff) = self.active_diff() else {
            return;
        };
        if diff.hunks.is_empty() {
            return;
        }
        let max = diff.hunks.len() as isize - 1;
        let next = (self.state.diff_hunk as isize + delta).clamp(0, max) as usize;
        let start_line = diff.hunks[next].start_line;
        self.state.diff_hunk = next;
        self.state.diff_scroll = start_line;
        self.state.focus = Focus::Diff;
    }

    fn scroll_diff(&mut self, delta: isize) {
        let Some(diff) = self.active_diff() else {
            return;
        };
        let max_scroll = diff.lines.len().saturating_sub(1) as isize;
        let next = (self.state.diff_scroll as isize + delta).clamp(0, max_scroll);
        self.state.diff_scroll = next as usize;
        self.update_selected_hunk_from_scroll();
    }

    fn scroll_inspector(&mut self, delta: isize) {
        if self.inspector_uses_patch_scroll() {
            self.scroll_diff(delta);
            return;
        }

        let max_scroll = self.inspector_line_count().saturating_sub(1) as isize;
        let next = (self.state.inspector_scroll as isize + delta).clamp(0, max_scroll);
        self.state.inspector_scroll = next as usize;
    }

    fn inspector_uses_patch_scroll(&self) -> bool {
        match self.state.panel {
            Panel::Changes => self
                .state
                .selected_change()
                .is_some_and(|change| !change.is_conflict() && self.state.diff.is_some()),
            Panel::Stash => self.state.stash_patch.is_some(),
            Panel::Log => self.state.log_patch.is_some(),
            Panel::Branches | Panel::Remotes | Panel::Repos => false,
        }
    }

    fn inspector_line_count(&self) -> usize {
        match self.state.panel {
            Panel::Changes => self
                .state
                .conflict_detail
                .as_ref()
                .map(|detail| detail.stages.len() + 6)
                .unwrap_or(3),
            Panel::Branches => {
                if self.state.selected_branch().is_some() {
                    10
                } else {
                    3
                }
            }
            Panel::Remotes => {
                if self.state.selected_remote().is_some() {
                    6
                } else {
                    3
                }
            }
            Panel::Repos => {
                if self.state.selected_repo_worktree().is_some()
                    || self.state.selected_repo_submodule().is_some()
                {
                    10
                } else {
                    3
                }
            }
            Panel::Stash | Panel::Log => self
                .active_diff()
                .map(|diff| diff.lines.len() + 1)
                .unwrap_or(3),
        }
    }

    fn update_selected_hunk_from_scroll(&mut self) {
        let Some(diff) = self.active_diff() else {
            return;
        };
        if diff.hunks.is_empty() {
            self.state.diff_hunk = 0;
            return;
        }
        let scroll = self.state.diff_scroll;
        let index = diff
            .hunks
            .iter()
            .enumerate()
            .take_while(|(_, hunk)| hunk.start_line <= scroll)
            .map(|(index, _)| index)
            .last()
            .unwrap_or(0);
        self.state.diff_hunk = index;
    }

    fn selected_or_marked_changes(&self) -> Vec<Change> {
        let keys = self.state.selected_or_marked_keys();
        changes_by_key(&self.state.changes, &keys)
    }

    fn show_error(&mut self, title: &str, err: GitError) {
        self.state.error_title = title.to_string();
        self.state.error_body = err.to_string();
        self.state.retry_handoff = None;
        self.state.modal = Modal::Error;
    }

    fn show_error_with_retry(&mut self, title: &str, err: GitError, handoff: TerminalHandoff) {
        self.state.error_title = title.to_string();
        self.state.error_body = format!(
            "{err}\n\nPress e to retry in the terminal if authentication or credentials are required."
        );
        self.state.retry_handoff = Some(handoff);
        self.state.modal = Modal::Error;
    }

    pub fn section_counts(&self) -> (usize, usize, usize, usize) {
        let conflicts = self.state.conflict_count();
        let staged = self.state.staged_count();
        let unstaged = self
            .state
            .changes
            .iter()
            .filter(|change| change.section == ChangeSection::Unstaged)
            .count();
        let untracked = self
            .state
            .changes
            .iter()
            .filter(|change| change.section == ChangeSection::Untracked)
            .count();
        (conflicts, staged, unstaged, untracked)
    }

    pub fn select_rail_visual_row(&mut self, row: usize) {
        let selected = match self.state.panel {
            Panel::Changes => self.change_index_for_visual_row(row).map(|index| {
                self.state.selected = index;
            }),
            Panel::Stash => (row < self.state.stashes.len()).then(|| {
                self.state.selected_stash = row;
            }),
            Panel::Branches => self.branch_index_for_visual_row(row).map(|index| {
                self.state.selected_branch = index;
            }),
            Panel::Log => (row < self.state.log.len()).then(|| {
                self.state.selected_log = row;
            }),
            Panel::Remotes => (row < self.state.remotes.len()).then(|| {
                self.state.selected_remote = row;
            }),
            Panel::Repos => self.repo_index_for_visual_row(row).map(|index| {
                self.state.selected_repo = index;
            }),
        };

        if selected.is_some() {
            self.state.focus = Focus::Changes;
            self.reset_inspector_position();
            self.load_active_inspector();
        }
    }

    pub fn select_rail_visible_row(&mut self, visible_row: usize, viewport_height: usize) {
        let offset = self.rail_viewport_offset(viewport_height);
        self.select_rail_visual_row(offset + visible_row);
    }

    fn rail_viewport_offset(&self, viewport_height: usize) -> usize {
        let Some(selected_row) = self.selected_visual_row_for_panel() else {
            return 0;
        };
        if viewport_height == 0 {
            return 0;
        }
        selected_row.saturating_sub(viewport_height.saturating_sub(1))
    }

    fn selected_visual_row_for_panel(&self) -> Option<usize> {
        match self.state.panel {
            Panel::Changes => {
                let mut visual = 0;
                let mut last_section: Option<ChangeSection> = None;
                for (index, change) in self.state.changes.iter().enumerate() {
                    if last_section != Some(change.section) {
                        visual += 1;
                        last_section = Some(change.section);
                    }
                    if index == self.state.selected {
                        return Some(visual);
                    }
                    visual += 1;
                }
                None
            }
            Panel::Stash => Some(self.state.selected_stash),
            Panel::Branches => {
                let mut visual = 0;
                let mut last_remote: Option<bool> = None;
                for (index, branch) in self.state.branches.iter().enumerate() {
                    if last_remote != Some(branch.remote) {
                        visual += 1;
                        last_remote = Some(branch.remote);
                    }
                    if index == self.state.selected_branch {
                        return Some(visual);
                    }
                    visual += 1;
                }
                None
            }
            Panel::Log => Some(self.state.selected_log),
            Panel::Remotes => Some(self.state.selected_remote),
            Panel::Repos => {
                let selected = self.state.selected_repo;
                let mut visual = 0;
                if !self.state.worktrees.is_empty() {
                    visual += 1;
                    if selected < self.state.worktrees.len() {
                        return Some(visual + selected);
                    }
                    visual += self.state.worktrees.len();
                }
                if !self.state.submodules.is_empty() {
                    visual += 1;
                    let submodule_index = selected.checked_sub(self.state.worktrees.len())?;
                    if submodule_index < self.state.submodules.len() {
                        return Some(visual + submodule_index);
                    }
                }
                None
            }
        }
    }

    fn change_index_for_visual_row(&self, row: usize) -> Option<usize> {
        let mut visual = 0;
        let mut last_section: Option<ChangeSection> = None;
        for (index, change) in self.state.changes.iter().enumerate() {
            if last_section != Some(change.section) {
                if visual == row {
                    return None;
                }
                visual += 1;
                last_section = Some(change.section);
            }
            if visual == row {
                return Some(index);
            }
            visual += 1;
        }
        None
    }

    fn branch_index_for_visual_row(&self, row: usize) -> Option<usize> {
        let mut visual = 0;
        let mut last_remote: Option<bool> = None;
        for (index, branch) in self.state.branches.iter().enumerate() {
            if last_remote != Some(branch.remote) {
                if visual == row {
                    return None;
                }
                visual += 1;
                last_remote = Some(branch.remote);
            }
            if visual == row {
                return Some(index);
            }
            visual += 1;
        }
        None
    }

    fn repo_index_for_visual_row(&self, row: usize) -> Option<usize> {
        let mut visual = 0;
        if !self.state.worktrees.is_empty() {
            if visual == row {
                return None;
            }
            visual += 1;
            for index in 0..self.state.worktrees.len() {
                if visual == row {
                    return Some(index);
                }
                visual += 1;
            }
        }
        if !self.state.submodules.is_empty() {
            if visual == row {
                return None;
            }
            visual += 1;
            for index in 0..self.state.submodules.len() {
                if visual == row {
                    return Some(self.state.worktrees.len() + index);
                }
                visual += 1;
            }
        }
        None
    }
}

fn collect_refresh_payload(git: &GitCli) -> Result<RefreshPayload, GitError> {
    let snapshot = git.snapshot()?;
    Ok(RefreshPayload {
        snapshot,
        stashes: git.stash_list().ok(),
        branches: git.branch_list().ok(),
        log: git.log_list().ok(),
        remotes: git.remote_list().ok(),
        worktrees: git.worktree_list().ok(),
        submodules: git.submodule_list().ok(),
        rebase_state: git.rebase_state().ok().flatten(),
        current_upstream: git.current_upstream().ok(),
    })
}

fn editor_command() -> Option<(OsString, Vec<OsString>)> {
    for name in ["GIT_EDITOR", "VISUAL", "EDITOR"] {
        let Ok(value) = std::env::var(name) else {
            continue;
        };
        if let Some(parts) = split_command_words(&value) {
            return Some(parts);
        }
    }
    Some((OsString::from("vi"), Vec::new()))
}

fn split_command_words(value: &str) -> Option<(OsString, Vec<OsString>)> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(quote_ch) = quote {
            if ch == quote_ch {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            ch if ch.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                while chars.peek().is_some_and(|next| next.is_whitespace()) {
                    chars.next();
                }
            }
            ch => current.push(ch),
        }
    }

    if escaped {
        current.push('\\');
    }
    if quote.is_some() {
        return None;
    }
    if !current.is_empty() {
        words.push(current);
    }

    let mut parts = words.into_iter();
    let command = parts.next()?;
    let args = parts.map(OsString::from).collect::<Vec<_>>();
    Some((OsString::from(command), args))
}

fn temp_commit_message_path() -> std::path::PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("gack-commit-{}-{stamp}.txt", std::process::id()))
}

fn create_commit_message_draft(path: &std::path::Path, draft: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.write_all(draft.as_bytes())
}

fn split_commit_message(text: &str) -> (String, String) {
    let text = text.trim_end_matches(['\r', '\n']);
    let mut lines = text.lines();
    let summary = lines.next().unwrap_or("").trim_end().to_string();
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    (summary, body)
}

fn move_index(index: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let max = len as isize - 1;
    (index as isize + delta).clamp(0, max) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_simple_editor_commands_without_shelling_out() {
        let (command, args) = split_command_words("code --wait").unwrap();
        assert_eq!(command, OsString::from("code"));
        assert_eq!(args, vec![OsString::from("--wait")]);
    }

    #[test]
    fn splits_quoted_editor_commands_without_shelling_out() {
        let (command, args) = split_command_words(
            r#""/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code" --wait"#,
        )
        .unwrap();
        assert_eq!(
            command,
            OsString::from("/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code")
        );
        assert_eq!(args, vec![OsString::from("--wait")]);
    }

    #[test]
    fn splits_commit_message_from_editor_file() {
        let (summary, body) = split_commit_message("Update branch panel\n\nAdd detail rows.\n");
        assert_eq!(summary, "Update branch panel");
        assert_eq!(body, "Add detail rows.");
    }
}
