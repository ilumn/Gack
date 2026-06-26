use crate::action::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    Stage,
    StageAll,
    Unstage,
    StageHunk,
    UnstageHunk,
    Discard,
    Commit,
    Refresh,
    Changes,
    Stash,
    Branches,
    Log,
    Remotes,
    Repos,
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
    StartInteractiveRebase,
    Whitespace,
    IncreaseContext,
    DecreaseContext,
    Help,
}

#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub id: CommandId,
    pub label: &'static str,
    pub key: &'static str,
    pub keywords: &'static [&'static str],
    pub action: Action,
}

pub fn command_catalog() -> &'static [CommandSpec] {
    &[
        CommandSpec {
            id: CommandId::Stage,
            label: "Stage selection",
            key: "s",
            keywords: &["add", "index"],
            action: Action::Stage,
        },
        CommandSpec {
            id: CommandId::StageAll,
            label: "Stage all visible",
            key: "a",
            keywords: &["add all", "index"],
            action: Action::StageAllVisible,
        },
        CommandSpec {
            id: CommandId::Unstage,
            label: "Unstage selection",
            key: "u",
            keywords: &["restore staged", "reset"],
            action: Action::Unstage,
        },
        CommandSpec {
            id: CommandId::StageHunk,
            label: "Stage hunk",
            key: "S",
            keywords: &["patch", "partial"],
            action: Action::StageHunk,
        },
        CommandSpec {
            id: CommandId::UnstageHunk,
            label: "Unstage hunk",
            key: "U",
            keywords: &["patch", "partial"],
            action: Action::UnstageHunk,
        },
        CommandSpec {
            id: CommandId::Discard,
            label: "Discard selection",
            key: "x",
            keywords: &["restore", "clean", "delete"],
            action: Action::Discard,
        },
        CommandSpec {
            id: CommandId::Commit,
            label: "Commit staged changes",
            key: "c",
            keywords: &["save"],
            action: Action::Commit,
        },
        CommandSpec {
            id: CommandId::Refresh,
            label: "Refresh repository",
            key: "r",
            keywords: &["reload", "status"],
            action: Action::Refresh,
        },
        CommandSpec {
            id: CommandId::Changes,
            label: "Open Changes panel",
            key: "0",
            keywords: &["files", "status"],
            action: Action::OpenChanges,
        },
        CommandSpec {
            id: CommandId::Stash,
            label: "Open Stash panel",
            key: "z",
            keywords: &["shelves", "wip"],
            action: Action::OpenStash,
        },
        CommandSpec {
            id: CommandId::Branches,
            label: "Open Branches panel",
            key: "B",
            keywords: &["refs", "switch"],
            action: Action::OpenBranches,
        },
        CommandSpec {
            id: CommandId::Log,
            label: "Open Log panel",
            key: "L",
            keywords: &["history", "commits"],
            action: Action::OpenLog,
        },
        CommandSpec {
            id: CommandId::Remotes,
            label: "Open Remotes panel",
            key: "F",
            keywords: &["fetch", "push", "origin"],
            action: Action::OpenRemotes,
        },
        CommandSpec {
            id: CommandId::Repos,
            label: "Open Repos panel",
            key: "R",
            keywords: &["worktrees", "submodules", "topology"],
            action: Action::OpenRepos,
        },
        CommandSpec {
            id: CommandId::ApplyStash,
            label: "Apply selected stash",
            key: "A",
            keywords: &["stash apply"],
            action: Action::ApplyStash,
        },
        CommandSpec {
            id: CommandId::DropStash,
            label: "Drop selected stash",
            key: "d",
            keywords: &["stash drop", "delete stash"],
            action: Action::DropStash,
        },
        CommandSpec {
            id: CommandId::SwitchBranch,
            label: "Switch to selected branch",
            key: "o",
            keywords: &["checkout", "branch switch"],
            action: Action::SwitchBranch,
        },
        CommandSpec {
            id: CommandId::FetchRemote,
            label: "Fetch selected remote",
            key: "f",
            keywords: &["remote fetch", "prune"],
            action: Action::FetchRemote,
        },
        CommandSpec {
            id: CommandId::UpdateCurrentBranch,
            label: "Update current branch ff-only",
            key: "U",
            keywords: &["pull", "fast forward", "remote update"],
            action: Action::UpdateCurrentBranch,
        },
        CommandSpec {
            id: CommandId::PushCurrentBranch,
            label: "Push current branch",
            key: "P",
            keywords: &["remote push", "publish commits"],
            action: Action::PushCurrentBranch,
        },
        CommandSpec {
            id: CommandId::SyncSubmodule,
            label: "Sync selected submodule",
            key: "y",
            keywords: &["submodule sync"],
            action: Action::SyncSubmodule,
        },
        CommandSpec {
            id: CommandId::RebaseContinue,
            label: "Rebase continue",
            key: "C",
            keywords: &["continue rebase"],
            action: Action::RebaseContinue,
        },
        CommandSpec {
            id: CommandId::RebaseAbort,
            label: "Rebase abort",
            key: "Q",
            keywords: &["abort rebase"],
            action: Action::RebaseAbort,
        },
        CommandSpec {
            id: CommandId::RebaseSkip,
            label: "Rebase skip",
            key: "K",
            keywords: &["skip rebase"],
            action: Action::RebaseSkip,
        },
        CommandSpec {
            id: CommandId::ExternalEditor,
            label: "Open external editor",
            key: "e / Ctrl+E",
            keywords: &["editor", "edit message", "edit conflict"],
            action: Action::ExternalEditor,
        },
        CommandSpec {
            id: CommandId::StartInteractiveRebase,
            label: "Start interactive rebase",
            key: "I",
            keywords: &["rebase interactive", "rewrite history"],
            action: Action::StartInteractiveRebase,
        },
        CommandSpec {
            id: CommandId::Whitespace,
            label: "Cycle diff whitespace",
            key: "w",
            keywords: &["ignore space"],
            action: Action::ToggleWhitespace,
        },
        CommandSpec {
            id: CommandId::IncreaseContext,
            label: "Increase diff context",
            key: "+",
            keywords: &["more context"],
            action: Action::IncreaseContext,
        },
        CommandSpec {
            id: CommandId::DecreaseContext,
            label: "Decrease diff context",
            key: "-",
            keywords: &["less context"],
            action: Action::DecreaseContext,
        },
        CommandSpec {
            id: CommandId::Help,
            label: "Open help",
            key: "?",
            keywords: &["keys", "shortcuts"],
            action: Action::Help,
        },
    ]
}

pub fn matching_commands(query: &str) -> Vec<CommandSpec> {
    let query = query.trim().to_lowercase();
    command_catalog()
        .iter()
        .copied()
        .filter(|command| query.is_empty() || command_matches(command, &query))
        .collect()
}

pub fn first_matching_command(query: &str) -> Option<CommandSpec> {
    matching_commands(query).into_iter().next()
}

fn command_matches(command: &CommandSpec, query: &str) -> bool {
    let label = command.label.to_lowercase();
    label.contains(query)
        || command
            .keywords
            .iter()
            .any(|keyword| keyword.to_lowercase().contains(query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unstage_query_prefers_unstage_over_stage() {
        let command = first_matching_command("unstage").unwrap();
        assert_eq!(command.id, CommandId::Unstage);
    }

    #[test]
    fn panel_queries_are_discoverable() {
        assert_eq!(
            first_matching_command("history").unwrap().id,
            CommandId::Log
        );
        assert_eq!(
            first_matching_command("submodules").unwrap().id,
            CommandId::Repos
        );
    }
}
