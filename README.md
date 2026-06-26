# Gack

Gack is a compact Rust terminal UI for everyday Git work. It is designed to feel closer to an IDE Git panel than a pile of porcelain commands: status, diffs, staging, unstaging, commits, stash, branches, remotes, worktrees, submodules, and conflict helpers in one terminal interface.

The name is inspired by the exclamation one makes when they forget a niche git command and have to google it.

<img  height="512" alt="image" src="https://github.com/user-attachments/assets/19bb2dfe-d96c-4a18-9dbe-14631302680c" />


## Features

- Dense Changes panel with staged, unstaged, untracked, deleted, renamed, copied, and conflicted files.
- Right-side inspector for diffs, stash patches, commit patches, branch details, remote details, worktrees, submodules, and conflict stages.
- File-level staging, unstaging, stage-all-visible, and guarded discard.
- Hunk stage/unstage for supported modified-file diffs.
- Commit modal with staged-file review, summary-length warning, and external editor handoff.
- Stash apply/drop, local branch switching, remote fetch/update/push, and terminal retry for credential prompts.
- Rebase recovery commands plus interactive rebase terminal handoff.
- Background refresh, filesystem watcher refresh, and periodic fallback.
- Mouse support for focus, wheel scrolling, and row selection.
- Minimal runtime dependencies: `crossterm`, `ratatui`, and the installed `git` CLI.

## Install From Source

```sh
cargo install --path .
```

## Run

Run Gack from inside the Git repository you want to inspect:

```sh
gack
```

Or point it at a repository:

```sh
gack --repo /path/to/repo
```

## Common Keys

| Key | Action |
| --- | --- |
| `0` | Changes panel |
| `z` | Stash panel |
| `B` | Branches panel |
| `L` | Log panel |
| `F` | Remotes panel |
| `R` | Repos panel |
| `j/k` or arrows | Move selection or scroll focused inspector |
| `Tab` / `Enter` | Focus inspector |
| `Esc` | Return focus or close modal |
| `Space` | Mark/unmark a change |
| `s` / `u` | Stage / unstage selection |
| `S` / `U` | Stage / unstage selected hunk, where supported |
| `x` | Discard or remove selected safe target, with confirmation |
| `c` | Commit staged changes |
| `Ctrl+S` | Submit commit from the commit modal |
| `Ctrl+E` | Open commit message in external editor |
| `:` or `Ctrl+P` | Command palette |
| `?` | Help |
| `q` | Quit |

## Configuration

Gack loads a global config, then a restricted project-local `.gack.toml` when present. Project config is limited to non-executable settings.

Example:

```toml
[ui]
mouse = "auto"              # auto | always | never

[git]
auto_refresh_ms = 2500      # fallback interval when file watching is unavailable
filesystem_watch = "auto"   # auto | always | never
```

Global config location:

- macOS: `~/Library/Application Support/gack/config.toml`
- Linux and other Unix: `$XDG_CONFIG_HOME/gack/config.toml` or `~/.config/gack/config.toml`

You can also pass `--config <path>` or `--no-config`.

## Refresh Behavior

Gack starts a native filesystem watcher by default and uses it to trigger quiet background refreshes when the worktree or Git metadata changes. If native watching cannot be started or does not deliver a probe event in the current environment, Gack falls back to a bounded polling watcher. If that cannot run, it uses periodic refresh as a last resort.

Successful automatic refreshes do not replace the status line. Manual actions and watcher errors still report status normally.

## Safety Model

- Git commands are invoked through structured process arguments, not shell strings.
- Destructive actions require confirmation.
- Discard confirmation stores fingerprints and cancels if the selected work changes before confirmation.
- Remote update and push operations re-check refs and avoid force behavior.
- Terminal handoff is used for editors, interactive rebase, and credential prompts.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

## License

Licensed under the MIT License. See [LICENSE-MIT](LICENSE-MIT).
