use crate::git::{
    BranchEntry, Change, ChangeKey, ConflictDetail, Diff, LogEntry, RebaseState, RemoteEntry,
    StashEntry, SubmoduleEntry, UpstreamBranch, WhitespaceMode, WorktreeEntry,
};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Changes,
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Changes,
    Stash,
    Branches,
    Log,
    Remotes,
    Repos,
}

impl Panel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Changes => "Changes",
            Self::Stash => "Stash",
            Self::Branches => "Branches",
            Self::Log => "Log",
            Self::Remotes => "Remotes",
            Self::Repos => "Repos",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    None,
    Help,
    Commit,
    Palette,
    Confirm,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingOperation {
    Discard(Vec<PendingDiscard>),
    ApplyStash(String),
    DropStash(String),
    SwitchBranch(String),
    FetchRemote(String),
    UpdateCurrentBranch,
    PushCurrentBranch,
    RemoveWorktree(PathBuf),
    SyncSubmodule(PathBuf),
    UpdateSubmodule(PathBuf),
    StageSubmodulePointer(PathBuf),
    RebaseAbort,
    RebaseSkip,
    CheckoutOurs(PathBuf),
    CheckoutTheirs(PathBuf),
    MarkResolved(PathBuf),
    StartInteractiveRebase(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDiscard {
    pub key: ChangeKey,
    pub fingerprint: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalHandoff {
    pub cwd: PathBuf,
    pub command: OsString,
    pub args: Vec<OsString>,
    pub label: String,
    pub after: TerminalHandoffAfter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalHandoffAfter {
    Refresh(String),
    LoadCommitMessage(PathBuf),
    Commit(PathBuf),
}

#[derive(Debug, Clone)]
pub struct RepoSummary {
    pub root_label: String,
    pub branch: String,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

#[derive(Debug)]
pub struct AppState {
    pub repo: RepoSummary,
    pub panel: Panel,
    pub changes: Vec<Change>,
    pub selected: usize,
    pub marked: BTreeSet<ChangeKey>,
    pub diff: Option<Diff>,
    pub diff_context: u8,
    pub whitespace_mode: WhitespaceMode,
    pub diff_scroll: usize,
    pub inspector_scroll: usize,
    pub diff_hunk: usize,
    pub focus: Focus,
    pub modal: Modal,
    pub commit_summary: String,
    pub commit_body: String,
    pub commit_body_focus: bool,
    pub palette_query: String,
    pub stashes: Vec<StashEntry>,
    pub selected_stash: usize,
    pub stash_patch: Option<Diff>,
    pub branches: Vec<BranchEntry>,
    pub selected_branch: usize,
    pub log: Vec<LogEntry>,
    pub selected_log: usize,
    pub log_patch: Option<Diff>,
    pub remotes: Vec<RemoteEntry>,
    pub selected_remote: usize,
    pub worktrees: Vec<WorktreeEntry>,
    pub selected_worktree: usize,
    pub submodules: Vec<SubmoduleEntry>,
    pub selected_submodule: usize,
    pub selected_repo: usize,
    pub conflict_detail: Option<ConflictDetail>,
    pub rebase_state: Option<RebaseState>,
    pub current_upstream: Option<UpstreamBranch>,
    pub pending_handoff: Option<TerminalHandoff>,
    pub retry_handoff: Option<TerminalHandoff>,
    pub confirm_title: String,
    pub confirm_body: String,
    pub pending_discard: Vec<ChangeKey>,
    pub pending_operation: Option<PendingOperation>,
    pub status_message: String,
    pub error_title: String,
    pub error_body: String,
    pub should_quit: bool,
}

impl AppState {
    pub fn new(repo: RepoSummary) -> Self {
        Self {
            repo,
            panel: Panel::Changes,
            changes: Vec::new(),
            selected: 0,
            marked: BTreeSet::new(),
            diff: None,
            diff_context: 3,
            whitespace_mode: WhitespaceMode::Normal,
            diff_scroll: 0,
            inspector_scroll: 0,
            diff_hunk: 0,
            focus: Focus::Changes,
            modal: Modal::None,
            commit_summary: String::new(),
            commit_body: String::new(),
            commit_body_focus: false,
            palette_query: String::new(),
            stashes: Vec::new(),
            selected_stash: 0,
            stash_patch: None,
            branches: Vec::new(),
            selected_branch: 0,
            log: Vec::new(),
            selected_log: 0,
            log_patch: None,
            remotes: Vec::new(),
            selected_remote: 0,
            worktrees: Vec::new(),
            selected_worktree: 0,
            submodules: Vec::new(),
            selected_submodule: 0,
            selected_repo: 0,
            conflict_detail: None,
            rebase_state: None,
            current_upstream: None,
            pending_handoff: None,
            retry_handoff: None,
            confirm_title: String::new(),
            confirm_body: String::new(),
            pending_discard: Vec::new(),
            pending_operation: None,
            status_message: "Ready".to_string(),
            error_title: String::new(),
            error_body: String::new(),
            should_quit: false,
        }
    }

    pub fn selected_change(&self) -> Option<&Change> {
        self.changes.get(self.selected)
    }

    pub fn selected_change_key(&self) -> Option<ChangeKey> {
        self.selected_change().map(|change| change.key.clone())
    }

    pub fn selected_or_marked_keys(&self) -> Vec<ChangeKey> {
        if self.marked.is_empty() {
            self.selected_change_key().into_iter().collect()
        } else {
            self.marked.iter().cloned().collect()
        }
    }

    pub fn staged_count(&self) -> usize {
        self.changes
            .iter()
            .filter(|change| change.is_staged())
            .count()
    }

    pub fn conflict_count(&self) -> usize {
        self.changes
            .iter()
            .filter(|change| change.is_conflict())
            .count()
    }

    pub fn clamp_selection(&mut self) {
        if self.changes.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.changes.len() {
            self.selected = self.changes.len() - 1;
        }
    }

    pub fn restore_selection_by_key(&mut self, key: Option<&ChangeKey>, fallback_index: usize) {
        if self.changes.is_empty() {
            self.selected = 0;
            return;
        }

        if let Some(key) = key
            && let Some(index) = self.changes.iter().position(|change| &change.key == key)
        {
            self.selected = index;
            return;
        }

        self.selected = fallback_index.min(self.changes.len() - 1);
    }

    pub fn retain_valid_marks(&mut self) {
        let keys: BTreeSet<ChangeKey> = self
            .changes
            .iter()
            .map(|change| change.key.clone())
            .collect();
        self.marked.retain(|key| keys.contains(key));
    }

    pub fn selected_stash(&self) -> Option<&StashEntry> {
        self.stashes.get(self.selected_stash)
    }

    pub fn selected_branch(&self) -> Option<&BranchEntry> {
        self.branches.get(self.selected_branch)
    }

    pub fn selected_log_entry(&self) -> Option<&LogEntry> {
        self.log.get(self.selected_log)
    }

    pub fn selected_remote(&self) -> Option<&RemoteEntry> {
        self.remotes.get(self.selected_remote)
    }

    pub fn selected_worktree(&self) -> Option<&WorktreeEntry> {
        self.worktrees.get(self.selected_worktree)
    }

    pub fn selected_submodule(&self) -> Option<&SubmoduleEntry> {
        self.submodules.get(self.selected_submodule)
    }

    pub fn selected_repo_worktree(&self) -> Option<&WorktreeEntry> {
        if self.selected_repo < self.worktrees.len() {
            self.worktrees.get(self.selected_repo)
        } else {
            None
        }
    }

    pub fn selected_repo_submodule(&self) -> Option<&SubmoduleEntry> {
        let index = self.selected_repo.checked_sub(self.worktrees.len())?;
        self.submodules.get(index)
    }

    pub fn clamp_panel_selection(&mut self) {
        self.selected_stash = clamp_index(self.selected_stash, self.stashes.len());
        self.selected_branch = clamp_index(self.selected_branch, self.branches.len());
        self.selected_log = clamp_index(self.selected_log, self.log.len());
        self.selected_remote = clamp_index(self.selected_remote, self.remotes.len());
        self.selected_worktree = clamp_index(self.selected_worktree, self.worktrees.len());
        self.selected_submodule = clamp_index(self.selected_submodule, self.submodules.len());
        self.selected_repo = clamp_index(
            self.selected_repo,
            self.worktrees.len() + self.submodules.len(),
        );
    }
}

fn clamp_index(index: usize, len: usize) -> usize {
    if len == 0 { 0 } else { index.min(len - 1) }
}
