mod diff;
mod explore;
mod process;
mod status;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub use diff::{Diff, DiffKind, WhitespaceMode};
pub use explore::{
    BranchEntry, ConflictDetail, ConflictStage, LogEntry, RebaseState, RemoteEntry, StashEntry,
    SubmoduleEntry, UpstreamBranch, WorktreeEntry,
};

use crate::model::RepoSummary;

#[derive(Debug)]
pub struct GitError {
    message: String,
}

impl GitError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GitError {}

impl From<std::io::Error> for GitError {
    fn from(value: std::io::Error) -> Self {
        Self::new(value.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChangeId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeSection {
    Conflict,
    Staged,
    Unstaged,
    Untracked,
    Ignored,
}

impl ChangeSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Conflict => "Conflicts",
            Self::Staged => "Staged",
            Self::Unstaged => "Unstaged",
            Self::Untracked => "Untracked",
            Self::Ignored => "Ignored",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Ignored,
    Conflict,
}

impl ChangeKind {
    pub fn tag(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Copied => "C",
            Self::TypeChanged => "T",
            Self::Untracked => "?",
            Self::Ignored => "!",
            Self::Conflict => "U",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoId {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_git_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChangeKey {
    pub repo_id: Option<RepoId>,
    pub section: ChangeSection,
    pub kind: ChangeKind,
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub xy: String,
}

#[derive(Debug, Clone)]
pub struct Change {
    pub id: ChangeId,
    pub key: ChangeKey,
    pub section: ChangeSection,
    pub kind: ChangeKind,
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub xy: String,
}

impl Change {
    pub fn is_staged(&self) -> bool {
        self.section == ChangeSection::Staged
    }

    pub fn is_unstaged(&self) -> bool {
        self.section == ChangeSection::Unstaged
    }

    pub fn is_untracked(&self) -> bool {
        self.section == ChangeSection::Untracked
    }

    pub fn is_conflict(&self) -> bool {
        self.section == ChangeSection::Conflict
    }

    pub fn display_path(&self) -> String {
        if let Some(original) = &self.original_path {
            format!(
                "{} -> {}",
                original.to_string_lossy(),
                self.path.to_string_lossy()
            )
        } else {
            self.path.to_string_lossy().into_owned()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct StatusSnapshot {
    pub branch: String,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffTarget {
    Staged,
    Worktree,
}

#[derive(Debug, Clone)]
pub struct GitCli {
    root: PathBuf,
    git_dir: PathBuf,
    common_git_dir: PathBuf,
    repo_id: RepoId,
}

impl GitCli {
    pub fn discover(start: &Path) -> Result<Self, GitError> {
        let args = vec![
            OsString::from("-C"),
            start.as_os_str().to_os_string(),
            OsString::from("rev-parse"),
            OsString::from("--show-toplevel"),
            OsString::from("--git-dir"),
            OsString::from("--git-common-dir"),
            OsString::from("--is-inside-work-tree"),
        ];
        let out = process::git_output_os(None, &args, false)?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mut lines = text.lines();
        let root = lines
            .next()
            .ok_or_else(|| GitError::new("git did not return a repository root"))?;
        let git_dir = lines
            .next()
            .ok_or_else(|| GitError::new("git did not return a git directory"))?;
        let common_git_dir = lines
            .next()
            .ok_or_else(|| GitError::new("git did not return a common git directory"))?;
        let inside = lines.next().unwrap_or("false");

        if inside != "true" {
            return Err(GitError::new("not inside a Git work tree"));
        }

        let root = PathBuf::from(root);
        let git_dir = resolve_git_path(&root, git_dir);
        let common_git_dir = resolve_git_path(&root, common_git_dir);
        let repo_id = RepoId {
            root: root.clone(),
            git_dir: git_dir.clone(),
            common_git_dir: common_git_dir.clone(),
        };
        Ok(Self {
            root,
            git_dir,
            common_git_dir,
            repo_id,
        })
    }

    pub fn repo_summary(&self, snapshot: &StatusSnapshot) -> RepoSummary {
        let root_label = self
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.to_string_lossy().into_owned());

        RepoSummary {
            root_label,
            branch: snapshot.branch.clone(),
            ahead: snapshot.ahead,
            behind: snapshot.behind,
        }
    }

    pub fn snapshot(&self) -> Result<StatusSnapshot, GitError> {
        let out = process::git_output(
            Some(&self.root),
            &[
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
                "--untracked-files=all",
                "--renames",
            ],
            true,
        )?;
        let mut snapshot = status::parse_status(&out.stdout)?;
        self.attach_repo_id(&mut snapshot);
        Ok(snapshot)
    }

    fn attach_repo_id(&self, snapshot: &mut StatusSnapshot) {
        for change in &mut snapshot.changes {
            change.key.repo_id = Some(self.repo_id.clone());
        }
    }

    pub fn diff(&self, change: &Change) -> Result<Diff, GitError> {
        self.diff_with_options(change, 3, WhitespaceMode::Normal)
    }

    pub fn diff_with_options(
        &self,
        change: &Change,
        context_lines: u8,
        whitespace: WhitespaceMode,
    ) -> Result<Diff, GitError> {
        let target = match change.section {
            ChangeSection::Staged => DiffTarget::Staged,
            ChangeSection::Unstaged | ChangeSection::Untracked => DiffTarget::Worktree,
            ChangeSection::Conflict | ChangeSection::Ignored => DiffTarget::Worktree,
        };

        if change.kind == ChangeKind::Untracked {
            return self.untracked_diff(change, context_lines, whitespace);
        }

        let mut args = vec![
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--color=never"),
            OsString::from("--patch"),
            OsString::from("--find-renames"),
            OsString::from(format!("-U{context_lines}")),
        ];
        match whitespace {
            WhitespaceMode::Normal => {}
            WhitespaceMode::IgnoreSpaceChange => {
                args.push(OsString::from("--ignore-space-change"));
            }
            WhitespaceMode::IgnoreAllSpace => {
                args.push(OsString::from("--ignore-all-space"));
            }
        }
        if target == DiffTarget::Staged {
            args.push(OsString::from("--cached"));
        }
        args.push(OsString::from("--"));
        args.push(change.path.as_os_str().to_os_string());
        let out = process::git_output_os(Some(&self.root), &args, true)?;
        diff::parse_diff(target, &out.stdout, context_lines, whitespace)
    }

    fn untracked_diff(
        &self,
        change: &Change,
        context_lines: u8,
        whitespace: WhitespaceMode,
    ) -> Result<Diff, GitError> {
        let full_path = self.root.join(&change.path);
        let bytes = std::fs::read(&full_path)?;
        let text = String::from_utf8_lossy(&bytes);
        let mut patch = String::new();
        patch.push_str(&format!(
            "diff --git a/{0} b/{0}\n",
            change.path.to_string_lossy()
        ));
        patch.push_str("new file mode 100644\n");
        patch.push_str("index 0000000..0000000\n");
        patch.push_str("--- /dev/null\n");
        patch.push_str(&format!("+++ b/{}\n", change.path.to_string_lossy()));
        let line_count = text.lines().count().max(1);
        patch.push_str(&format!("@@ -0,0 +1,{line_count} @@\n"));
        for line in text.lines() {
            patch.push('+');
            patch.push_str(line);
            patch.push('\n');
        }
        diff::parse_diff(
            DiffTarget::Worktree,
            patch.as_bytes(),
            context_lines,
            whitespace,
        )
    }

    pub fn stage_hunk(
        &self,
        change: &Change,
        diff: &Diff,
        hunk_index: usize,
    ) -> Result<(), GitError> {
        self.apply_hunk(change, diff, hunk_index, false)
    }

    pub fn unstage_hunk(
        &self,
        change: &Change,
        diff: &Diff,
        hunk_index: usize,
    ) -> Result<(), GitError> {
        self.apply_hunk(change, diff, hunk_index, true)
    }

    fn apply_hunk(
        &self,
        change: &Change,
        diff: &Diff,
        hunk_index: usize,
        reverse: bool,
    ) -> Result<(), GitError> {
        if let Some(reason) = hunk_action_disabled_reason(change, diff, hunk_index, reverse) {
            return Err(GitError::new(reason));
        }

        let patch = diff
            .selected_hunk_patch(hunk_index)
            .ok_or_else(|| GitError::new("no hunk selected"))?;

        let mut check_args = vec![
            OsString::from("apply"),
            OsString::from("--cached"),
            OsString::from("--check"),
        ];
        if reverse {
            check_args.push(OsString::from("--reverse"));
        }
        process::git_output_os_with_stdin(Some(&self.root), &check_args, false, patch.clone())?;

        let mut apply_args = vec![OsString::from("apply"), OsString::from("--cached")];
        if reverse {
            apply_args.push(OsString::from("--reverse"));
        }
        process::git_output_os_with_stdin(Some(&self.root), &apply_args, false, patch)?;
        Ok(())
    }

    pub fn stage(&self, changes: &[Change]) -> Result<(), GitError> {
        for change in changes {
            match change.section {
                ChangeSection::Unstaged | ChangeSection::Untracked => {
                    let args = vec![
                        OsString::from("add"),
                        OsString::from("--"),
                        change.path.as_os_str().to_os_string(),
                    ];
                    process::git_status_os(Some(&self.root), &args, false)?;
                }
                ChangeSection::Staged => {}
                ChangeSection::Conflict | ChangeSection::Ignored => {
                    return Err(GitError::new(format!(
                        "cannot stage {} from the {} section yet",
                        change.display_path(),
                        change.section.label()
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn stage_all_visible(&self, changes: &[Change]) -> Result<(), GitError> {
        let paths: Vec<&Change> = changes
            .iter()
            .filter(|change| change.is_unstaged() || change.is_untracked())
            .collect();
        self.stage(&paths.into_iter().cloned().collect::<Vec<_>>())
    }

    pub fn unstage(&self, changes: &[Change]) -> Result<(), GitError> {
        for change in changes {
            if change.section == ChangeSection::Staged {
                let args = vec![
                    OsString::from("restore"),
                    OsString::from("--staged"),
                    OsString::from("--"),
                    change.path.as_os_str().to_os_string(),
                ];
                process::git_status_os(Some(&self.root), &args, false)?;
            }
        }
        Ok(())
    }

    pub fn discard(&self, changes: &[Change]) -> Result<(), GitError> {
        for change in changes {
            match change.section {
                ChangeSection::Unstaged => {
                    let args = vec![
                        OsString::from("restore"),
                        OsString::from("--worktree"),
                        OsString::from("--"),
                        change.path.as_os_str().to_os_string(),
                    ];
                    process::git_status_os(Some(&self.root), &args, false)?;
                }
                ChangeSection::Untracked => {
                    let args = vec![
                        OsString::from("clean"),
                        OsString::from("-f"),
                        OsString::from("--"),
                        change.path.as_os_str().to_os_string(),
                    ];
                    process::git_status_os(Some(&self.root), &args, false)?;
                }
                ChangeSection::Staged => {
                    return Err(GitError::new(format!(
                        "cannot discard staged change {}; unstage it first",
                        change.display_path()
                    )));
                }
                ChangeSection::Conflict | ChangeSection::Ignored => {
                    return Err(GitError::new(format!(
                        "discard is not available for {}",
                        change.display_path()
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn discard_fingerprint(&self, change: &Change) -> Result<u64, GitError> {
        match change.section {
            ChangeSection::Unstaged => {
                let args = vec![
                    OsString::from("diff"),
                    OsString::from("--no-ext-diff"),
                    OsString::from("--color=never"),
                    OsString::from("--patch"),
                    OsString::from("--"),
                    change.path.as_os_str().to_os_string(),
                ];
                let out = process::git_output_os(Some(&self.root), &args, true)?;
                Ok(hash_bytes(&out.stdout))
            }
            ChangeSection::Untracked => {
                let bytes = std::fs::read(self.root.join(&change.path))?;
                Ok(hash_bytes(&bytes))
            }
            _ => Err(GitError::new(
                "discard fingerprint requires an unstaged or untracked change",
            )),
        }
    }

    pub fn commit(&self, message: &str) -> Result<String, GitError> {
        let staged = self
            .snapshot()?
            .changes
            .iter()
            .any(|change| change.section == ChangeSection::Staged);
        if !staged {
            return Err(GitError::new("there are no staged changes to commit"));
        }

        let mut path = std::env::temp_dir();
        let file_name = format!(
            "gack-commit-{}-{}.txt",
            std::process::id(),
            monotonic_millis()
        );
        path.push(file_name);
        std::fs::write(&path, message)?;

        let args = vec![
            OsString::from("commit"),
            OsString::from("--file"),
            path.as_os_str().to_os_string(),
        ];
        let result = process::git_output_os(Some(&self.root), &args, false);
        let _ = std::fs::remove_file(&path);

        let _out = result?;
        let hash_out =
            process::git_output(Some(&self.root), &["rev-parse", "--short", "HEAD"], true)?;
        let hash = String::from_utf8_lossy(&hash_out.stdout).trim().to_string();
        Ok(hash)
    }

    #[allow(dead_code)]
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    pub fn common_git_dir(&self) -> &Path {
        &self.common_git_dir
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn resolve_git_path(root: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

pub fn hunk_action_disabled_reason(
    change: &Change,
    diff: &Diff,
    hunk_index: usize,
    reverse: bool,
) -> Option<String> {
    if diff.hunks.get(hunk_index).is_none() {
        return Some("no hunk selected".to_string());
    }
    if diff.is_binary {
        return Some("binary files do not support hunk staging".to_string());
    }
    if diff.whitespace != WhitespaceMode::Normal {
        return Some("hunk staging requires normal whitespace diff mode".to_string());
    }
    if change.kind != ChangeKind::Modified {
        return Some("hunk staging currently supports modified files only".to_string());
    }
    if reverse && change.section != ChangeSection::Staged {
        return Some("hunk unstaging requires a staged change".to_string());
    }
    if !reverse && change.section != ChangeSection::Unstaged {
        return Some("hunk staging requires an unstaged tracked change".to_string());
    }
    None
}

fn monotonic_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

pub fn changes_by_id(changes: &[Change], ids: &[ChangeId]) -> Vec<Change> {
    let map: BTreeMap<ChangeId, Change> = changes
        .iter()
        .map(|change| (change.id, change.clone()))
        .collect();
    ids.iter().filter_map(|id| map.get(id).cloned()).collect()
}

pub fn changes_by_key(changes: &[Change], keys: &[ChangeKey]) -> Vec<Change> {
    let map: BTreeMap<ChangeKey, Change> = changes
        .iter()
        .map(|change| (change.key.clone(), change.clone()))
        .collect();
    keys.iter()
        .filter_map(|key| map.get(key).cloned())
        .collect()
}
