use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use super::diff;
use super::process;
use super::{Diff, DiffTarget, GitCli, GitError, WhitespaceMode};

const FIELD_SEP: char = '\x1f';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    pub selector: String,
    pub short_hash: String,
    pub relative_time: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchEntry {
    pub full_ref: String,
    pub name: String,
    pub short_oid: String,
    pub upstream: Option<String>,
    pub subject: String,
    pub current: bool,
    pub remote: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub oid: String,
    pub short_oid: String,
    pub author: String,
    pub author_date: String,
    pub refs: Vec<String>,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    pub name: String,
    pub fetch_url: Option<String>,
    pub push_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamBranch {
    pub remote: String,
    pub branch: String,
    pub display: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: Option<String>,
    pub prunable: Option<String>,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmoduleEntry {
    pub name: String,
    pub path: PathBuf,
    pub url: Option<String>,
    pub recorded_oid: Option<String>,
    pub checked_out_oid: Option<String>,
    pub initialized: bool,
    pub dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictStage {
    pub mode: String,
    pub oid: String,
    pub stage: u8,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictDetail {
    pub path: PathBuf,
    pub stages: Vec<ConflictStage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseState {
    pub kind: String,
    pub head_name: Option<String>,
    pub onto: Option<String>,
    pub current: Option<String>,
    pub msgnum: Option<String>,
    pub end: Option<String>,
}

impl GitCli {
    pub fn stash_list(&self) -> Result<Vec<StashEntry>, GitError> {
        let out = process::git_output(
            Some(&self.root),
            &["stash", "list", "--format=%gd%x1f%h%x1f%cr%x1f%gs"],
            true,
        )?;
        Ok(parse_stash_list(&out.stdout))
    }

    pub fn stash_patch(&self, selector: &str) -> Result<Diff, GitError> {
        let args = vec![
            OsString::from("stash"),
            OsString::from("show"),
            OsString::from("--patch"),
            OsString::from("--include-untracked"),
            OsString::from("--no-ext-diff"),
            OsString::from("--color=never"),
            OsString::from("--find-renames"),
            OsString::from(selector),
        ];
        let out = process::git_output_os(Some(&self.root), &args, true)?;
        diff::parse_diff(DiffTarget::Worktree, &out.stdout, 3, WhitespaceMode::Normal)
    }

    pub fn branch_list(&self) -> Result<Vec<BranchEntry>, GitError> {
        let out = process::git_output(
            Some(&self.root),
            &[
                "for-each-ref",
                "--format=%(HEAD)%1f%(refname)%1f%(refname:short)%1f%(objectname:short)%1f%(upstream:short)%1f%(subject)",
                "refs/heads",
                "refs/remotes",
            ],
            true,
        )?;
        Ok(parse_branch_list(&out.stdout))
    }

    pub fn log_list(&self) -> Result<Vec<LogEntry>, GitError> {
        let out = process::git_output(
            Some(&self.root),
            &[
                "log",
                "--date=iso-strict",
                "--decorate=short",
                "--format=%H%x1f%h%x1f%an%x1f%ad%x1f%D%x1f%s",
                "-n",
                "200",
            ],
            true,
        );
        match out {
            Ok(out) => Ok(parse_log_list(&out.stdout)),
            Err(err) if err.to_string().contains("does not have any commits") => Ok(Vec::new()),
            Err(err) => Err(err),
        }
    }

    pub fn commit_patch(&self, oid: &str) -> Result<Diff, GitError> {
        let args = vec![
            OsString::from("show"),
            OsString::from("--patch"),
            OsString::from("--no-ext-diff"),
            OsString::from("--color=never"),
            OsString::from("--find-renames"),
            OsString::from("--format=fuller"),
            OsString::from(oid),
        ];
        let out = process::git_output_os(Some(&self.root), &args, true)?;
        diff::parse_diff(DiffTarget::Worktree, &out.stdout, 3, WhitespaceMode::Normal)
    }

    pub fn remote_list(&self) -> Result<Vec<RemoteEntry>, GitError> {
        let out = process::git_output(Some(&self.root), &["remote"], true)?;
        let names = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let mut remotes = Vec::new();
        for name in names {
            let fetch_url = self.remote_urls(&name, false)?.into_iter().next();
            let push_url = self.remote_urls(&name, true)?.into_iter().next();
            remotes.push(RemoteEntry {
                name,
                fetch_url: fetch_url.map(|url| redact_url(&url)),
                push_url: push_url.map(|url| redact_url(&url)),
            });
        }
        Ok(remotes)
    }

    pub fn worktree_list(&self) -> Result<Vec<WorktreeEntry>, GitError> {
        let out = process::git_output(
            Some(&self.root),
            &["worktree", "list", "--porcelain", "-z"],
            true,
        )?;
        Ok(parse_worktree_list(&out.stdout, &self.root))
    }

    pub fn submodule_list(&self) -> Result<Vec<SubmoduleEntry>, GitError> {
        let modules_path = self.root.join(".gitmodules");
        if !modules_path.exists() {
            return Ok(Vec::new());
        }

        let out = process::git_output(
            Some(&self.root),
            &[
                "config",
                "-z",
                "--file",
                ".gitmodules",
                "--get-regexp",
                "^submodule\\..*",
            ],
            true,
        )?;
        let mut submodules = parse_submodule_config(&out.stdout);
        for submodule in &mut submodules {
            let full_path = self.root.join(&submodule.path);
            submodule.initialized = full_path.join(".git").exists();
            submodule.recorded_oid = self.submodule_recorded_oid(&submodule.path).ok().flatten();
            if submodule.initialized {
                submodule.checked_out_oid = self.submodule_checked_out_oid(&submodule.path).ok();
                submodule.dirty = self.submodule_dirty(&submodule.path).unwrap_or(false);
            }
        }
        Ok(submodules)
    }

    pub fn conflict_detail(&self, path: &std::path::Path) -> Result<ConflictDetail, GitError> {
        let args = vec![
            OsString::from("ls-files"),
            OsString::from("-u"),
            OsString::from("-z"),
            OsString::from("--"),
            path.as_os_str().to_os_string(),
        ];
        let out = process::git_output_os(Some(&self.root), &args, true)?;
        Ok(ConflictDetail {
            path: path.to_path_buf(),
            stages: parse_conflict_stages(&out.stdout),
        })
    }

    pub fn stash_apply(&self, selector: &str) -> Result<(), GitError> {
        let args = vec![
            OsString::from("stash"),
            OsString::from("apply"),
            OsString::from(selector),
        ];
        process::git_status_os(Some(&self.root), &args, false)
    }

    pub fn stash_drop(&self, selector: &str) -> Result<(), GitError> {
        let args = vec![
            OsString::from("stash"),
            OsString::from("drop"),
            OsString::from(selector),
        ];
        process::git_status_os(Some(&self.root), &args, false)
    }

    pub fn switch_branch(&self, name: &str) -> Result<(), GitError> {
        let args = vec![
            OsString::from("switch"),
            OsString::from("--"),
            OsString::from(name),
        ];
        process::git_status_os(Some(&self.root), &args, false)
    }

    pub fn fetch_remote(&self, name: &str) -> Result<(), GitError> {
        let args = vec![
            OsString::from("fetch"),
            OsString::from("--porcelain"),
            OsString::from("--prune"),
            OsString::from(name),
        ];
        process::git_status_os(Some(&self.root), &args, false)
    }

    pub fn update_current_branch_ff_only(&self) -> Result<(), GitError> {
        let upstream = self.current_upstream()?;
        self.fetch_remote(&upstream.remote)?;
        let behind = self.rev_count("HEAD..@{u}")?;
        if behind == 0 {
            return Ok(());
        }
        process::git_status_os(
            Some(&self.root),
            &[
                OsString::from("merge"),
                OsString::from("--ff-only"),
                OsString::from("@{u}"),
            ],
            false,
        )
    }

    pub fn push_current_branch(&self) -> Result<(), GitError> {
        let upstream = self.current_upstream()?;
        let behind = self.rev_count("@{u}..HEAD")?;
        let upstream_ahead = self.rev_count("HEAD..@{u}")?;
        if upstream_ahead > 0 {
            return Err(GitError::new(format!(
                "refusing to push: {} is {} commit(s) ahead of HEAD; update first",
                upstream.display, upstream_ahead
            )));
        }
        if behind == 0 {
            return Ok(());
        }
        let dst = format!("HEAD:refs/heads/{}", upstream.branch);
        let args = vec![
            OsString::from("push"),
            OsString::from("--porcelain"),
            OsString::from(upstream.remote),
            OsString::from(dst),
        ];
        process::git_status_os(Some(&self.root), &args, false)
    }

    pub fn rev_count(&self, revspec: &str) -> Result<u32, GitError> {
        let out = process::git_output(Some(&self.root), &["rev-list", "--count", revspec], true)?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.trim()
            .parse()
            .map_err(|_| GitError::new(format!("git returned invalid rev count for {revspec}")))
    }

    pub fn current_upstream(&self) -> Result<UpstreamBranch, GitError> {
        let out = process::git_output(
            Some(&self.root),
            &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
            true,
        )?;
        let display = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let Some((remote, branch)) = display.split_once('/') else {
            return Err(GitError::new("current branch has no remote upstream"));
        };
        Ok(UpstreamBranch {
            remote: remote.to_string(),
            branch: branch.to_string(),
            display,
        })
    }

    pub fn rev_short(&self, rev: &str) -> Result<String, GitError> {
        let args = vec![
            OsString::from("rev-parse"),
            OsString::from("--short"),
            OsString::from(rev),
        ];
        let out = process::git_output_os(Some(&self.root), &args, true)?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    pub fn remove_worktree(&self, path: &std::path::Path) -> Result<(), GitError> {
        let args = vec![
            OsString::from("worktree"),
            OsString::from("remove"),
            OsString::from("--"),
            path.as_os_str().to_os_string(),
        ];
        process::git_status_os(Some(&self.root), &args, false)
    }

    pub fn submodule_sync(&self, path: &std::path::Path) -> Result<(), GitError> {
        let args = vec![
            OsString::from("submodule"),
            OsString::from("sync"),
            OsString::from("--"),
            path.as_os_str().to_os_string(),
        ];
        process::git_status_os(Some(&self.root), &args, false)
    }

    pub fn submodule_update_local(&self, path: &std::path::Path) -> Result<(), GitError> {
        let args = vec![
            OsString::from("submodule"),
            OsString::from("update"),
            OsString::from("--init"),
            OsString::from("--recursive"),
            OsString::from("--no-fetch"),
            OsString::from("--"),
            path.as_os_str().to_os_string(),
        ];
        process::git_status_os(Some(&self.root), &args, false)
    }

    pub fn stage_submodule_pointer(&self, path: &std::path::Path) -> Result<(), GitError> {
        let args = vec![
            OsString::from("add"),
            OsString::from("--"),
            path.as_os_str().to_os_string(),
        ];
        process::git_status_os(Some(&self.root), &args, false)
    }

    pub fn checkout_ours(&self, path: &std::path::Path) -> Result<(), GitError> {
        let args = vec![
            OsString::from("checkout"),
            OsString::from("--ours"),
            OsString::from("--"),
            path.as_os_str().to_os_string(),
        ];
        process::git_status_os(Some(&self.root), &args, false)
    }

    pub fn checkout_theirs(&self, path: &std::path::Path) -> Result<(), GitError> {
        let args = vec![
            OsString::from("checkout"),
            OsString::from("--theirs"),
            OsString::from("--"),
            path.as_os_str().to_os_string(),
        ];
        process::git_status_os(Some(&self.root), &args, false)
    }

    pub fn mark_resolved(&self, path: &std::path::Path) -> Result<(), GitError> {
        let args = vec![
            OsString::from("add"),
            OsString::from("--"),
            path.as_os_str().to_os_string(),
        ];
        process::git_status_os(Some(&self.root), &args, false)
    }

    pub fn rebase_state(&self) -> Result<Option<RebaseState>, GitError> {
        let merge_dir = self.git_path("rebase-merge")?;
        if merge_dir.is_dir() {
            return Ok(Some(RebaseState {
                kind: "merge".to_string(),
                head_name: read_optional_file(merge_dir.join("head-name")),
                onto: read_optional_file(merge_dir.join("onto")),
                current: read_optional_file(merge_dir.join("stopped-sha")),
                msgnum: read_optional_file(merge_dir.join("msgnum")),
                end: read_optional_file(merge_dir.join("end")),
            }));
        }

        let apply_dir = self.git_path("rebase-apply")?;
        if apply_dir.is_dir() {
            return Ok(Some(RebaseState {
                kind: "apply".to_string(),
                head_name: read_optional_file(apply_dir.join("head-name")),
                onto: read_optional_file(apply_dir.join("onto")),
                current: read_optional_file(apply_dir.join("next")),
                msgnum: read_optional_file(apply_dir.join("next")),
                end: read_optional_file(apply_dir.join("last")),
            }));
        }

        Ok(None)
    }

    pub fn rebase_continue(&self) -> Result<(), GitError> {
        process::git_status_os(
            Some(&self.root),
            &[OsString::from("rebase"), OsString::from("--continue")],
            false,
        )
    }

    pub fn rebase_abort(&self) -> Result<(), GitError> {
        process::git_status_os(
            Some(&self.root),
            &[OsString::from("rebase"), OsString::from("--abort")],
            false,
        )
    }

    pub fn rebase_skip(&self) -> Result<(), GitError> {
        process::git_status_os(
            Some(&self.root),
            &[OsString::from("rebase"), OsString::from("--skip")],
            false,
        )
    }

    fn git_path(&self, name: &str) -> Result<PathBuf, GitError> {
        let out = process::git_output(
            Some(&self.root),
            &["rev-parse", "--path-format=absolute", "--git-path", name],
            true,
        )?;
        Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
    }

    fn remote_urls(&self, name: &str, push: bool) -> Result<Vec<String>, GitError> {
        let mut args = vec![
            OsString::from("remote"),
            OsString::from("get-url"),
            OsString::from("--all"),
        ];
        if push {
            args.push(OsString::from("--push"));
        }
        args.push(OsString::from(name));

        let out = process::git_output_os(Some(&self.root), &args, true)?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_string)
            .collect())
    }

    fn submodule_recorded_oid(&self, path: &std::path::Path) -> Result<Option<String>, GitError> {
        let args = vec![
            OsString::from("ls-files"),
            OsString::from("--stage"),
            OsString::from("--"),
            path.as_os_str().to_os_string(),
        ];
        let out = process::git_output_os(Some(&self.root), &args, true)?;
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(text.split_whitespace().nth(1).map(|oid| oid.to_string()))
    }

    fn submodule_checked_out_oid(&self, path: &std::path::Path) -> Result<String, GitError> {
        let repo_path = self.root.join(path);
        let out = process::git_output(Some(&repo_path), &["rev-parse", "--short", "HEAD"], true)?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn submodule_dirty(&self, path: &std::path::Path) -> Result<bool, GitError> {
        let repo_path = self.root.join(path);
        let out = process::git_output(
            Some(&repo_path),
            &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
            true,
        )?;
        Ok(!out.stdout.is_empty())
    }
}

fn parse_stash_list(bytes: &[u8]) -> Vec<StashEntry> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(FIELD_SEP).collect();
            if parts.len() < 4 {
                return None;
            }
            Some(StashEntry {
                selector: parts[0].to_string(),
                short_hash: parts[1].to_string(),
                relative_time: parts[2].to_string(),
                subject: parts[3..].join(" "),
            })
        })
        .collect()
}

fn parse_branch_list(bytes: &[u8]) -> Vec<BranchEntry> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(FIELD_SEP).collect();
            if parts.len() < 6 {
                return None;
            }
            Some(BranchEntry {
                full_ref: parts[1].to_string(),
                name: parts[2].to_string(),
                short_oid: parts[3].to_string(),
                upstream: non_empty(parts[4]),
                subject: parts[5..].join(" "),
                current: parts[0] == "*",
                remote: parts[1].starts_with("refs/remotes/"),
            })
        })
        .collect()
}

fn parse_log_list(bytes: &[u8]) -> Vec<LogEntry> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(FIELD_SEP).collect();
            if parts.len() < 6 {
                return None;
            }
            let refs = parts[4]
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect();
            Some(LogEntry {
                oid: parts[0].to_string(),
                short_oid: parts[1].to_string(),
                author: parts[2].to_string(),
                author_date: parts[3].to_string(),
                refs,
                subject: parts[5..].join(" "),
            })
        })
        .collect()
}

fn parse_worktree_list(bytes: &[u8], current_root: &std::path::Path) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut current: Option<WorktreeEntry> = None;
    let fields = if bytes.contains(&0) {
        bytes
            .split(|byte| *byte == 0)
            .map(String::from_utf8_lossy)
            .map(|line| line.into_owned())
            .collect::<Vec<_>>()
    } else {
        String::from_utf8_lossy(bytes)
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>()
    };

    for line in fields {
        if line.is_empty() {
            continue;
        }
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            let path = PathBuf::from(path);
            current = Some(WorktreeEntry {
                current: same_path(&path, current_root),
                path,
                head: None,
                branch: None,
                detached: false,
                bare: false,
                locked: None,
                prunable: None,
            });
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        if let Some(head) = line.strip_prefix("HEAD ") {
            entry.head = Some(short_oid(head));
        } else if let Some(branch) = line.strip_prefix("branch ") {
            entry.branch = Some(branch.trim_start_matches("refs/heads/").to_string());
        } else if line == "detached" {
            entry.detached = true;
        } else if line == "bare" {
            entry.bare = true;
        } else if let Some(reason) = line.strip_prefix("locked") {
            entry.locked = Some(reason.trim().to_string());
        } else if let Some(reason) = line.strip_prefix("prunable") {
            entry.prunable = Some(reason.trim().to_string());
        }
    }

    if let Some(entry) = current {
        entries.push(entry);
    }

    entries
}

fn parse_submodule_config(bytes: &[u8]) -> Vec<SubmoduleEntry> {
    #[derive(Default)]
    struct PartialSubmodule {
        path: Option<PathBuf>,
        url: Option<String>,
    }

    let mut partials: BTreeMap<String, PartialSubmodule> = BTreeMap::new();
    let records = if bytes.contains(&0) {
        bytes
            .split(|byte| *byte == 0)
            .map(String::from_utf8_lossy)
            .map(|line| line.into_owned())
            .collect::<Vec<_>>()
    } else {
        String::from_utf8_lossy(bytes)
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    for line in records {
        let Some((key, value)) = line.split_once('\n').or_else(|| line.split_once(' ')) else {
            continue;
        };
        let Some(rest) = key.strip_prefix("submodule.") else {
            continue;
        };
        let Some((name, field)) = rest.rsplit_once('.') else {
            continue;
        };
        let entry = partials.entry(name.to_string()).or_default();
        match field {
            "path" => entry.path = Some(PathBuf::from(value)),
            "url" => entry.url = Some(redact_url(value)),
            _ => {}
        }
    }

    partials
        .into_iter()
        .filter_map(|(name, partial)| {
            let path = partial.path?;
            Some(SubmoduleEntry {
                name,
                path,
                url: partial.url,
                recorded_oid: None,
                checked_out_oid: None,
                initialized: false,
                dirty: false,
            })
        })
        .collect()
}

fn parse_conflict_stages(bytes: &[u8]) -> Vec<ConflictStage> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let (meta, path) = text.split_once('\t')?;
            let mut fields = meta.split_whitespace();
            let mode = fields.next()?.to_string();
            let oid = fields.next()?.to_string();
            let stage = fields.next()?.parse().ok()?;
            Some(ConflictStage {
                mode,
                oid,
                stage,
                path: PathBuf::from(path),
            })
        })
        .collect()
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn short_oid(value: &str) -> String {
    value.chars().take(7).collect()
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn redact_url(value: &str) -> String {
    let Some(scheme_end) = value.find("://") else {
        return value.to_string();
    };
    let auth_start = scheme_end + 3;
    let Some(at) = value[auth_start..]
        .find('@')
        .map(|index| auth_start + index)
    else {
        return value.to_string();
    };
    format!("{}<redacted>{}", &value[..auth_start], &value[at..])
}

fn read_optional_file(path: PathBuf) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stash_list() {
        let parsed = parse_stash_list(b"stash@{0}\x1fabc123\x1f2 minutes ago\x1fWIP on main\n");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].selector, "stash@{0}");
        assert_eq!(parsed[0].subject, "WIP on main");
    }

    #[test]
    fn parses_branch_list() {
        let raw = b"*\x1frefs/heads/main\x1fmain\x1fabc1234\x1forigin/main\x1fInitial\n \x1frefs/remotes/origin/main\x1forigin/main\x1fabc1234\x1f\x1fInitial\n";
        let parsed = parse_branch_list(raw);
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].current);
        assert!(!parsed[0].remote);
        assert!(parsed[1].remote);
    }

    #[test]
    fn redacts_remote_userinfo() {
        assert_eq!(
            redact_url("https://token@example.com/org/repo.git"),
            "https://<redacted>@example.com/org/repo.git"
        );
    }

    #[test]
    fn parses_worktree_list() {
        let raw = b"worktree /tmp/repo\nHEAD abc123456789\nbranch refs/heads/main\n\nworktree /tmp/other\nHEAD def123456789\ndetached\nlocked reason\n";
        let parsed = parse_worktree_list(raw, std::path::Path::new("/tmp/repo"));
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].branch.as_deref(), Some("main"));
        assert!(parsed[1].detached);
        assert_eq!(parsed[1].locked.as_deref(), Some("reason"));
    }

    #[test]
    fn parses_submodule_config() {
        let raw = b"submodule.vendor/parser.path vendor/parser\nsubmodule.vendor/parser.url https://token@example.com/parser.git\n";
        let parsed = parse_submodule_config(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "vendor/parser");
        assert_eq!(parsed[0].path, PathBuf::from("vendor/parser"));
        assert_eq!(
            parsed[0].url.as_deref(),
            Some("https://<redacted>@example.com/parser.git")
        );
    }

    #[test]
    fn parses_conflict_stages() {
        let raw = [
            b"100644 abc123 1\tfile.txt\0".as_slice(),
            b"100644 def456 2\tfile.txt\0".as_slice(),
        ]
        .concat();
        let parsed = parse_conflict_stages(&raw);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].stage, 1);
        assert_eq!(parsed[1].oid, "def456");
    }
}
