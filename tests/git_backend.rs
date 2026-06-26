use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use gack::action::Action;
use gack::app::App;
use gack::git::{ChangeSection, GitCli, WhitespaceMode};
use gack::model::{Modal, TerminalHandoffAfter};
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};

struct TestRepo {
    dir: TempDir,
}

impl TestRepo {
    fn init() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Self { dir };
        repo.git(&["init", "-b", "main"]);
        repo.git(&["config", "user.name", "Test"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_AUTHOR_DATE", "2001-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2001-01-01T00:00:00Z")
            .output()
            .expect("git command starts");
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(&self, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(args)
            .output()
            .expect("git command starts");
        assert!(
            output.status.success(),
            "git {:?} failed\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn git_output_in(path: &Path, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .expect("git command starts");
        assert!(
            output.status.success(),
            "git {:?} failed\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn write(&self, path: &str, contents: &str) {
        let full = self.path().join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("parent dirs");
        }
        fs::write(full, contents).expect("write file");
    }

    fn commit_all(&self, message: &str) {
        self.git(&["add", "--all"]);
        self.git(&["commit", "-m", message]);
    }
}

#[test]
fn discovers_from_subdirectory_and_reads_status() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub fn value() -> u8 { 1 }\n");
    repo.commit_all("initial");
    fs::create_dir_all(repo.path().join("src/nested")).unwrap();

    repo.write("src/lib.rs", "pub fn value() -> u8 { 2 }\n");

    let git = GitCli::discover(&repo.path().join("src/nested")).unwrap();
    assert!(git.common_git_dir().is_absolute());
    let snapshot = git.snapshot().unwrap();

    assert_eq!(snapshot.branch, "main");
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].section, ChangeSection::Unstaged);
    assert!(snapshot.changes[0].key.repo_id.is_some());
}

#[test]
fn same_file_staged_and_unstaged_rows_stage_and_unstage_safely() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");
    repo.commit_all("initial");

    repo.write("file.txt", "two\n");
    repo.git(&["add", "--", "file.txt"]);
    repo.write("file.txt", "three\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let snapshot = git.snapshot().unwrap();
    assert_eq!(snapshot.changes.len(), 2);
    assert!(
        snapshot
            .changes
            .iter()
            .any(|change| change.section == ChangeSection::Staged)
    );
    let unstaged = snapshot
        .changes
        .iter()
        .find(|change| change.section == ChangeSection::Unstaged)
        .cloned()
        .unwrap();

    git.stage(&[unstaged]).unwrap();
    let snapshot = git.snapshot().unwrap();
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].section, ChangeSection::Staged);

    git.unstage(&snapshot.changes).unwrap();
    let snapshot = git.snapshot().unwrap();
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].section, ChangeSection::Unstaged);
}

#[test]
fn stages_literal_pathspec_special_filenames() {
    let repo = TestRepo::init();
    repo.write("base.txt", "base\n");
    repo.commit_all("initial");

    for name in ["*.rs", "[abc].txt", ":(top)", "-dash.txt", "space name.txt"] {
        repo.write(name, "content\n");
    }

    let git = GitCli::discover(repo.path()).unwrap();
    let snapshot = git.snapshot().unwrap();
    git.stage_all_visible(&snapshot.changes).unwrap();

    let names = repo.git_output(&["diff", "--cached", "--name-only", "-z"]);
    for name in ["*.rs", "[abc].txt", ":(top)", "-dash.txt", "space name.txt"] {
        assert!(
            names
                .split(|byte| *byte == 0)
                .any(|entry| entry == name.as_bytes()),
            "missing staged path {name}"
        );
    }
}

#[cfg(unix)]
#[test]
fn stages_non_utf8_filename_without_lossy_round_trip() {
    let repo = TestRepo::init();
    repo.write("base.txt", "base\n");
    repo.commit_all("initial");

    let name = std::ffi::OsString::from_vec(b"bad-\xff-name.txt".to_vec());
    if let Err(err) = fs::write(repo.path().join(&name), b"content\n") {
        eprintln!("skipping non-UTF-8 filename test: filesystem rejected fixture: {err}");
        return;
    }

    let git = GitCli::discover(repo.path()).unwrap();
    let snapshot = git.snapshot().unwrap();
    git.stage_all_visible(&snapshot.changes).unwrap();

    let names = repo.git_output(&["diff", "--cached", "--name-only", "-z"]);
    assert!(
        names
            .split(|byte| *byte == 0)
            .any(|entry| entry == name.as_os_str().as_bytes())
    );
}

#[test]
fn commits_with_temp_message_file_and_returns_head_hash() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let snapshot = git.snapshot().unwrap();
    git.stage_all_visible(&snapshot.changes).unwrap();
    let hash = git.commit("initial commit").unwrap();

    assert!(!hash.is_empty());
    let snapshot = git.snapshot().unwrap();
    assert!(snapshot.changes.is_empty());
}

#[test]
fn focused_panel_changes_navigation_behavior() {
    let repo = TestRepo::init();
    repo.write("a.txt", "one\ntwo\nthree\n");
    repo.write("b.txt", "alpha\nbeta\ngamma\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();

    assert_eq!(app.state.selected, 0);
    app.handle(Action::MoveDown);
    assert_eq!(app.state.selected, 1);

    app.handle(Action::FocusDiff);
    app.handle(Action::MoveDown);

    assert_eq!(app.state.selected, 1);
    assert!(app.state.diff_scroll > 0);

    app.handle(Action::Close);
    app.handle(Action::MoveUp);
    assert_eq!(app.state.selected, 0);
}

#[test]
fn non_diff_inspectors_scroll_when_focused() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");
    repo.commit_all("initial");

    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();
    app.handle(Action::OpenBranches);
    let selected = app.state.selected_branch;

    app.handle(Action::FocusDiff);
    app.handle(Action::MoveDown);

    assert_eq!(app.state.selected_branch, selected);
    assert!(app.state.inspector_scroll > 0);
}

#[test]
fn visual_rail_row_selection_skips_section_headers() {
    let repo = TestRepo::init();
    repo.write("a.txt", "a\n");
    repo.write("b.txt", "b\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();

    app.select_rail_visual_row(0);
    assert_eq!(app.state.selected, 0);
    app.select_rail_visual_row(2);
    assert_eq!(app.state.selected, 1);

    app.handle(Action::OpenBranches);
    app.select_rail_visual_row(0);
    assert_eq!(app.state.selected_branch, 0);
}

#[test]
fn visible_rail_row_selection_accounts_for_scrolled_lists() {
    let repo = TestRepo::init();
    for index in 0..20 {
        repo.write(&format!("file-{index:02}.txt"), "content\n");
    }

    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();
    app.state.selected = 15;

    app.select_rail_visible_row(0, 5);

    assert!(
        app.state.selected >= 11,
        "clicked visible row should map into the scrolled viewport, got {}",
        app.state.selected
    );
}

#[test]
fn palette_does_not_run_disabled_command() {
    let repo = TestRepo::init();
    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();

    app.handle(Action::Palette);
    for ch in "commit".chars() {
        app.handle(Action::Text(ch));
    }
    app.handle(Action::SubmitCommit);

    assert_eq!(app.state.modal, Modal::None);
    assert!(app.state.status_message.contains("disabled"));
    assert!(app.state.status_message.contains("stage changes first"));
}

#[test]
fn commit_editor_handoff_loads_draft_without_committing() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();
    app.handle(Action::StageAllVisible);
    app.handle(Action::Commit);
    assert_eq!(app.state.modal, Modal::Commit);

    let draft = repo.path().join("COMMIT_EDITMSG.test");
    fs::write(&draft, "Use external editor\n\nDetailed body.\n").unwrap();
    app.complete_terminal_handoff(
        TerminalHandoffAfter::LoadCommitMessage(draft),
        true,
        "exit 0",
    );

    assert_eq!(app.state.modal, Modal::Commit);
    assert_eq!(app.state.commit_summary, "Use external editor");
    assert_eq!(app.state.commit_body, "Detailed body.");
    assert_eq!(app.state.staged_count(), 1);
}

#[test]
fn failed_commit_editor_handoff_removes_draft() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();
    app.handle(Action::StageAllVisible);
    app.handle(Action::Commit);

    let draft = repo.path().join("COMMIT_EDITMSG.failed");
    fs::write(&draft, "will be removed\n").unwrap();
    app.complete_terminal_handoff(
        TerminalHandoffAfter::LoadCommitMessage(draft.clone()),
        false,
        "exit 1",
    );

    assert_eq!(app.state.modal, Modal::Commit);
    assert!(!draft.exists());
}

#[test]
fn commit_submit_uses_terminal_handoff_and_refreshes_after_success() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();
    app.handle(Action::StageAllVisible);
    app.handle(Action::Commit);
    for ch in "Use handoff".chars() {
        app.handle(Action::Text(ch));
    }
    app.handle(Action::SubmitCommit);

    let handoff = app.take_terminal_handoff().expect("commit handoff queued");
    assert_eq!(handoff.label, "git commit");
    let TerminalHandoffAfter::Commit(draft) = handoff.after.clone() else {
        panic!("expected commit completion");
    };
    assert_eq!(fs::read_to_string(&draft).unwrap(), "Use handoff\n");
    assert!(handoff.args.iter().any(|arg| arg == "commit"));
    assert!(handoff.args.iter().any(|arg| arg == "--file"));

    let status = Command::new(&handoff.command)
        .current_dir(&handoff.cwd)
        .args(&handoff.args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("commit handoff command starts");
    app.complete_terminal_handoff(handoff.after, status.success(), &status.to_string());

    assert_eq!(app.state.modal, Modal::None);
    assert!(app.state.status_message.starts_with("Committed "));
    assert!(app.state.changes.is_empty());
    assert!(!draft.exists());
}

#[test]
fn failed_commit_handoff_keeps_commit_modal_and_removes_draft() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();
    app.handle(Action::StageAllVisible);
    app.handle(Action::Commit);
    for ch in "Try commit".chars() {
        app.handle(Action::Text(ch));
    }
    app.handle(Action::SubmitCommit);

    let handoff = app.take_terminal_handoff().expect("commit handoff queued");
    let TerminalHandoffAfter::Commit(draft) = handoff.after.clone() else {
        panic!("expected commit completion");
    };

    app.complete_terminal_handoff(handoff.after, false, "exit status: 1");

    assert_eq!(app.state.modal, Modal::Commit);
    assert!(app.state.status_message.contains("Could not commit"));
    assert!(!draft.exists());
}

#[test]
fn refresh_preserves_selection_and_marks_by_stable_key() {
    let repo = TestRepo::init();
    repo.write("a.txt", "a\n");
    repo.write("b.txt", "b\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();
    app.handle(Action::ToggleMark);
    let marked = app.state.selected_change().unwrap().key.clone();

    app.refresh("manual refresh");

    assert!(app.state.marked.contains(&marked));
    assert_eq!(app.state.selected_change().unwrap().key, marked);
}

#[test]
fn background_refresh_applies_snapshot_without_blocking_call_site() {
    let repo = TestRepo::init();
    repo.write("a.txt", "a\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();
    assert!(app.request_background_refresh("background done"));

    for _ in 0..100 {
        app.poll_background_refresh();
        if app.state.status_message == "background done" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert_eq!(app.state.status_message, "background done");
    assert_eq!(app.state.changes.len(), 1);
}

#[test]
fn diff_controls_reload_selected_diff() {
    let repo = TestRepo::init();
    repo.write("a.txt", "one\ntwo\nthree\n");
    repo.commit_all("initial");
    repo.write("a.txt", "one\nTWO\nthree\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();

    assert_eq!(app.state.diff.as_ref().unwrap().context_lines, 3);
    app.handle(Action::IncreaseContext);
    assert_eq!(app.state.diff_context, 4);
    assert_eq!(app.state.diff.as_ref().unwrap().context_lines, 4);

    app.handle(Action::ToggleWhitespace);
    assert_eq!(app.state.whitespace_mode, WhitespaceMode::IgnoreSpaceChange);
    assert_eq!(
        app.state.diff.as_ref().unwrap().whitespace,
        WhitespaceMode::IgnoreSpaceChange
    );
}

#[test]
fn stages_and_unstages_selected_hunk() {
    let repo = TestRepo::init();
    let initial = numbered_lines(&[]);
    repo.write("file.txt", &initial);
    repo.commit_all("initial");

    let modified = numbered_lines(&[(2, "line 2 changed"), (18, "line 18 changed")]);
    repo.write("file.txt", &modified);

    let git = GitCli::discover(repo.path()).unwrap();
    let snapshot = git.snapshot().unwrap();
    let change = snapshot
        .changes
        .iter()
        .find(|change| change.section == ChangeSection::Unstaged)
        .unwrap();
    let diff = git
        .diff_with_options(change, 3, WhitespaceMode::Normal)
        .unwrap();
    assert_eq!(diff.hunks.len(), 2);

    git.stage_hunk(change, &diff, 0).unwrap();

    let cached = String::from_utf8_lossy(&repo.git_output(&["diff", "--cached"])).to_string();
    let worktree = String::from_utf8_lossy(&repo.git_output(&["diff"])).to_string();
    assert!(cached.contains("line 2 changed"));
    assert!(!cached.contains("line 18 changed"));
    assert!(worktree.contains("line 18 changed"));

    let snapshot = git.snapshot().unwrap();
    let staged = snapshot
        .changes
        .iter()
        .find(|change| change.section == ChangeSection::Staged)
        .unwrap();
    let staged_diff = git
        .diff_with_options(staged, 3, WhitespaceMode::Normal)
        .unwrap();
    git.unstage_hunk(staged, &staged_diff, 0).unwrap();

    let cached = String::from_utf8_lossy(&repo.git_output(&["diff", "--cached"])).to_string();
    let worktree = String::from_utf8_lossy(&repo.git_output(&["diff"])).to_string();
    assert!(!cached.contains("line 2 changed"));
    assert!(worktree.contains("line 2 changed"));
    assert!(worktree.contains("line 18 changed"));
}

#[test]
fn discard_unstaged_change_preserves_staged_index() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");
    repo.commit_all("initial");

    repo.write("file.txt", "two\n");
    repo.git(&["add", "--", "file.txt"]);
    repo.write("file.txt", "three\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let snapshot = git.snapshot().unwrap();
    let unstaged = snapshot
        .changes
        .iter()
        .find(|change| change.section == ChangeSection::Unstaged)
        .cloned()
        .unwrap();

    git.discard(&[unstaged]).unwrap();

    assert_eq!(
        fs::read_to_string(repo.path().join("file.txt")).unwrap(),
        "two\n"
    );
    let cached = String::from_utf8_lossy(&repo.git_output(&["diff", "--cached"])).to_string();
    assert!(cached.contains("two"));
}

#[test]
fn discard_untracked_file_uses_clean() {
    let repo = TestRepo::init();
    repo.write("base.txt", "base\n");
    repo.commit_all("initial");
    repo.write("scratch.txt", "scratch\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let snapshot = git.snapshot().unwrap();
    let untracked = snapshot
        .changes
        .iter()
        .find(|change| change.section == ChangeSection::Untracked)
        .cloned()
        .unwrap();

    git.discard(&[untracked]).unwrap();

    assert!(!repo.path().join("scratch.txt").exists());
}

#[test]
fn stale_discard_confirmation_does_not_remove_newer_work() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");
    repo.commit_all("initial");
    repo.write("file.txt", "two\n");

    let git = GitCli::discover(repo.path()).unwrap();
    let mut app = App::new(git).unwrap();
    app.handle(Action::Discard);
    repo.write("file.txt", "three\n");
    app.handle(Action::SubmitCommit);

    assert_eq!(
        fs::read_to_string(repo.path().join("file.txt")).unwrap(),
        "three\n"
    );
    assert!(app.state.status_message.contains("Discard canceled"));
}

#[test]
fn explorer_read_models_load_stash_branch_log_remote_and_worktree_data() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");
    repo.commit_all("initial");
    repo.git(&["branch", "feature"]);

    let bare = tempfile::tempdir().unwrap();
    let output = Command::new("git")
        .arg("-C")
        .arg(bare.path())
        .args(["init", "--bare"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    repo.git(&["remote", "add", "origin", bare.path().to_str().unwrap()]);

    let worktree_parent = tempfile::tempdir().unwrap();
    let worktree_path = worktree_parent.path().join("feature-worktree");
    repo.git(&[
        "worktree",
        "add",
        worktree_path.to_str().unwrap(),
        "feature",
    ]);

    repo.write("file.txt", "two\n");
    repo.write("scratch.txt", "scratch\n");
    repo.git(&["stash", "push", "-u", "-m", "panel stash"]);

    let git = GitCli::discover(repo.path()).unwrap();

    let stashes = git.stash_list().unwrap();
    assert_eq!(stashes.len(), 1);
    assert!(stashes[0].subject.contains("panel stash"));
    assert!(git.stash_patch(&stashes[0].selector).unwrap().additions > 0);

    let branches = git.branch_list().unwrap();
    assert!(branches.iter().any(|branch| branch.name == "main"));
    assert!(branches.iter().any(|branch| branch.name == "feature"));

    let log = git.log_list().unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "initial");
    assert!(!git.commit_patch(&log[0].oid).unwrap().lines.is_empty());

    let remotes = git.remote_list().unwrap();
    assert_eq!(remotes.len(), 1);
    assert_eq!(remotes[0].name, "origin");

    let worktrees = git.worktree_list().unwrap();
    assert!(worktrees.iter().any(|entry| entry.current));
    assert!(
        worktrees
            .iter()
            .any(|entry| entry.branch.as_deref() == Some("feature"))
    );
}

#[test]
fn stash_apply_and_drop_are_guarded_git_operations() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");
    repo.commit_all("initial");
    repo.write("file.txt", "two\n");
    repo.git(&["stash", "push", "-m", "apply me"]);

    let git = GitCli::discover(repo.path()).unwrap();
    let stash = git.stash_list().unwrap().remove(0);
    git.stash_apply(&stash.selector).unwrap();

    assert_eq!(
        fs::read_to_string(repo.path().join("file.txt")).unwrap(),
        "two\n"
    );

    git.stash_drop(&stash.selector).unwrap();
    assert!(git.stash_list().unwrap().is_empty());
}

#[test]
fn switches_to_selected_local_branch() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");
    repo.commit_all("initial");
    repo.git(&["switch", "-c", "feature"]);
    repo.write("feature.txt", "feature\n");
    repo.commit_all("feature work");
    repo.git(&["switch", "main"]);

    let git = GitCli::discover(repo.path()).unwrap();
    git.switch_branch("feature").unwrap();

    let branch = String::from_utf8_lossy(&TestRepo::git_output_in(
        repo.path(),
        &["branch", "--show-current"],
    ))
    .trim()
    .to_string();
    assert_eq!(branch, "feature");
}

#[test]
fn remote_fetch_update_and_push_use_upstream_safely() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");
    repo.commit_all("initial");

    let bare = tempfile::tempdir().unwrap();
    let output = Command::new("git")
        .arg("-C")
        .arg(bare.path())
        .args(["init", "--bare"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    repo.git(&["remote", "add", "origin", bare.path().to_str().unwrap()]);
    repo.git(&["push", "-u", "origin", "main"]);

    let peer = tempfile::tempdir().unwrap();
    let output = Command::new("git")
        .arg("clone")
        .arg("-b")
        .arg("main")
        .arg(bare.path())
        .arg(peer.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    TestRepo::git_output_in(peer.path(), &["config", "user.name", "Peer"]);
    TestRepo::git_output_in(peer.path(), &["config", "user.email", "peer@example.com"]);
    TestRepo::git_output_in(peer.path(), &["config", "commit.gpgsign", "false"]);
    fs::write(peer.path().join("remote.txt"), "remote\n").unwrap();
    TestRepo::git_output_in(peer.path(), &["add", "remote.txt"]);
    TestRepo::git_output_in(peer.path(), &["commit", "-m", "remote work"]);
    TestRepo::git_output_in(peer.path(), &["push", "origin", "main"]);

    let git = GitCli::discover(repo.path()).unwrap();
    let upstream = git.current_upstream().unwrap();
    assert_eq!(upstream.display, "origin/main");
    git.fetch_remote("origin").unwrap();
    git.update_current_branch_ff_only().unwrap();
    assert_eq!(
        fs::read_to_string(repo.path().join("remote.txt")).unwrap(),
        "remote\n"
    );

    repo.write("local.txt", "local\n");
    repo.commit_all("local work");
    git.push_current_branch().unwrap();

    let output = Command::new("git")
        .arg("--git-dir")
        .arg(bare.path())
        .args(["show", "main:local.txt"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "local\n");
}

#[test]
fn divergent_remote_update_is_rejected() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");
    repo.commit_all("initial");

    let bare = tempfile::tempdir().unwrap();
    let output = Command::new("git")
        .arg("-C")
        .arg(bare.path())
        .args(["init", "--bare"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    repo.git(&["remote", "add", "origin", bare.path().to_str().unwrap()]);
    repo.git(&["push", "-u", "origin", "main"]);

    let peer = tempfile::tempdir().unwrap();
    let output = Command::new("git")
        .arg("clone")
        .arg("-b")
        .arg("main")
        .arg(bare.path())
        .arg(peer.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    TestRepo::git_output_in(peer.path(), &["config", "user.name", "Peer"]);
    TestRepo::git_output_in(peer.path(), &["config", "user.email", "peer@example.com"]);
    TestRepo::git_output_in(peer.path(), &["config", "commit.gpgsign", "false"]);
    fs::write(peer.path().join("peer.txt"), "peer\n").unwrap();
    TestRepo::git_output_in(peer.path(), &["add", "peer.txt"]);
    TestRepo::git_output_in(peer.path(), &["commit", "-m", "peer work"]);
    TestRepo::git_output_in(peer.path(), &["push", "origin", "main"]);

    repo.write("local.txt", "local\n");
    repo.commit_all("local work");

    let git = GitCli::discover(repo.path()).unwrap();
    assert!(git.update_current_branch_ff_only().is_err());
    assert!(repo.path().join("local.txt").exists());
    assert!(!repo.path().join("peer.txt").exists());
}

#[test]
fn removes_clean_non_current_worktree() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");
    repo.commit_all("initial");
    repo.git(&["branch", "feature"]);

    let worktree_parent = tempfile::tempdir().unwrap();
    let worktree_path = worktree_parent.path().join("feature-worktree");
    repo.git(&[
        "worktree",
        "add",
        worktree_path.to_str().unwrap(),
        "feature",
    ]);

    let git = GitCli::discover(repo.path()).unwrap();
    let canonical_worktree = worktree_path.canonicalize().unwrap();
    assert!(
        git.worktree_list()
            .unwrap()
            .iter()
            .any(|entry| entry.path.canonicalize().ok().as_ref() == Some(&canonical_worktree))
    );

    git.remove_worktree(&worktree_path).unwrap();

    assert!(!worktree_path.exists());
    assert!(
        !git.worktree_list()
            .unwrap()
            .iter()
            .any(|entry| entry.path.canonicalize().ok().as_ref() == Some(&canonical_worktree))
    );
}

#[test]
fn dirty_worktree_remove_is_rejected_by_git() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");
    repo.commit_all("initial");
    repo.git(&["branch", "feature"]);

    let worktree_parent = tempfile::tempdir().unwrap();
    let worktree_path = worktree_parent.path().join("feature-worktree");
    repo.git(&[
        "worktree",
        "add",
        worktree_path.to_str().unwrap(),
        "feature",
    ]);
    fs::write(worktree_path.join("dirty.txt"), "dirty\n").unwrap();

    let git = GitCli::discover(repo.path()).unwrap();
    assert!(git.remove_worktree(&worktree_path).is_err());
    assert!(worktree_path.exists());
}

#[test]
fn submodule_sync_and_stage_pointer_are_local_operations() {
    let sub = TestRepo::init();
    sub.write("lib.txt", "one\n");
    sub.commit_all("sub initial");

    let parent = TestRepo::init();
    parent.write("root.txt", "root\n");
    parent.commit_all("parent initial");

    let output = Command::new("git")
        .arg("-C")
        .arg(parent.path())
        .args(["-c", "protocol.file.allow=always", "submodule", "add"])
        .arg(sub.path())
        .arg("vendor/parser")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    parent.commit_all("add submodule");

    let git = GitCli::discover(parent.path()).unwrap();
    let submodules = git.submodule_list().unwrap();
    assert_eq!(submodules.len(), 1);
    assert_eq!(
        submodules[0].path,
        std::path::PathBuf::from("vendor/parser")
    );
    git.submodule_sync(&submodules[0].path).unwrap();

    let submodule_path = parent.path().join("vendor/parser");
    TestRepo::git_output_in(&submodule_path, &["config", "user.name", "Sub"]);
    TestRepo::git_output_in(
        &submodule_path,
        &["config", "user.email", "sub@example.com"],
    );
    TestRepo::git_output_in(&submodule_path, &["config", "commit.gpgsign", "false"]);
    fs::write(submodule_path.join("lib.txt"), "two\n").unwrap();
    TestRepo::git_output_in(&submodule_path, &["add", "lib.txt"]);
    TestRepo::git_output_in(&submodule_path, &["commit", "-m", "sub update"]);

    git.stage_submodule_pointer(std::path::Path::new("vendor/parser"))
        .unwrap();
    let staged = String::from_utf8_lossy(&parent.git_output(&["diff", "--cached", "--submodule"]))
        .to_string();
    assert!(staged.contains("Submodule vendor/parser"));
}

#[test]
fn conflict_helper_can_choose_ours_and_mark_resolved() {
    let repo = TestRepo::init();
    repo.write("file.txt", "one\n");
    repo.commit_all("initial");
    repo.git(&["switch", "-c", "feature"]);
    repo.write("file.txt", "feature\n");
    repo.commit_all("feature change");
    repo.git(&["switch", "main"]);
    repo.write("file.txt", "main\n");
    repo.commit_all("main change");

    let output = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["merge", "feature"])
        .output()
        .unwrap();
    assert!(!output.status.success());

    let git = GitCli::discover(repo.path()).unwrap();
    let snapshot = git.snapshot().unwrap();
    let conflict = snapshot
        .changes
        .iter()
        .find(|change| change.is_conflict())
        .unwrap();
    let detail = git.conflict_detail(&conflict.path).unwrap();
    assert!(detail.stages.iter().any(|stage| stage.stage == 2));
    assert!(detail.stages.iter().any(|stage| stage.stage == 3));

    git.checkout_ours(&conflict.path).unwrap();
    assert_eq!(
        fs::read_to_string(repo.path().join("file.txt")).unwrap(),
        "main\n"
    );
    git.mark_resolved(&conflict.path).unwrap();
    assert!(
        git.snapshot()
            .unwrap()
            .changes
            .iter()
            .all(|change| !change.is_conflict())
    );
}

fn numbered_lines(overrides: &[(usize, &str)]) -> String {
    let mut lines = String::new();
    for index in 1..=24 {
        let value = overrides
            .iter()
            .find(|(line, _)| *line == index)
            .map(|(_, text)| *text)
            .unwrap_or_else(|| match index {
                2 => "line 2",
                18 => "line 18",
                _ => "",
            });
        if value.is_empty() {
            lines.push_str(&format!("line {index}\n"));
        } else {
            lines.push_str(value);
            lines.push('\n');
        }
    }
    lines
}
