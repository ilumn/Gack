use std::env;
use std::fmt;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

use gack::app::App;
use gack::config::Config;
use gack::git::GitCli;
use gack::tui;

#[derive(Debug, Default)]
struct Args {
    repo: Option<PathBuf>,
    config: Option<PathBuf>,
    no_config: bool,
    help: bool,
    version: bool,
}

#[derive(Debug)]
struct CliError {
    message: String,
}

impl CliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

fn main() {
    if let Err(err) = run() {
        eprintln!("gack: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args(env::args().skip(1))?;

    if args.help {
        print_help();
        return Ok(());
    }

    if args.version {
        println!("gack {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let requested_start = args.repo.unwrap_or(env::current_dir()?);
    let start = resolve_start_path(&requested_start)?;
    let git = GitCli::discover(&start)
        .map_err(|err| CliError::new(format_repo_discovery_error(&start, &err.to_string())))?;
    let config = Config::load_with_project(args.config, args.no_config, Some(git.root()))?;
    let mut app = App::with_config(git, config)?;

    if !io::stdout().is_terminal() {
        return Err("stdout is not a terminal".into());
    }

    tui::run(&mut app)?;
    Ok(())
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, Box<dyn std::error::Error>> {
    let mut parsed = Args::default();
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => parsed.help = true,
            "-V" | "--version" => parsed.version = true,
            "--repo" => {
                let Some(path) = args.next() else {
                    return Err("--repo requires a path".into());
                };
                parsed.repo = Some(PathBuf::from(path));
            }
            "--config" => {
                let Some(path) = args.next() else {
                    return Err("--config requires a path".into());
                };
                parsed.config = Some(PathBuf::from(path));
            }
            "--no-config" => parsed.no_config = true,
            value if value.starts_with("--repo=") => {
                parsed.repo = Some(PathBuf::from(value.trim_start_matches("--repo=")));
            }
            value if value.starts_with("--config=") => {
                parsed.config = Some(PathBuf::from(value.trim_start_matches("--config=")));
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(parsed)
}

fn resolve_start_path(path: &Path) -> Result<PathBuf, CliError> {
    let metadata = fs::metadata(path).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => CliError::new(format!(
            "repository path does not exist\n\nRequested path:\n  {}\n\nPass a directory inside a Git worktree:\n  gack --repo /path/to/repo",
            path.display()
        )),
        _ => CliError::new(format!(
            "could not read repository path\n\nRequested path:\n  {}\n\nDetails:\n{}",
            path.display(),
            indent_block(&err.to_string())
        )),
    })?;

    if metadata.is_dir() {
        return Ok(path.to_path_buf());
    }

    if metadata.is_file() {
        return path
            .parent()
            .map(Path::to_path_buf)
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| {
                CliError::new(format!(
                    "repository path is a file with no parent directory\n\nRequested path:\n  {}",
                    path.display()
                ))
            });
    }

    Err(CliError::new(format!(
        "repository path is not a directory or file\n\nRequested path:\n  {}\n\nPass a directory inside a Git worktree:\n  gack --repo /path/to/repo",
        path.display()
    )))
}

fn format_repo_discovery_error(start: &Path, detail: &str) -> String {
    let normalized = detail.to_ascii_lowercase();

    if normalized.contains("not a git repository")
        || normalized.contains("not inside a git work tree")
    {
        return format!(
            "no Git repository found\n\nLooked from:\n  {}\n\nRun `gack` from inside a Git worktree, or pass one explicitly:\n  gack --repo /path/to/repo\n\nIf this directory should be a new repository, initialize it first:\n  git init\n\nDetails:\n{}",
            start.display(),
            indent_block(detail)
        );
    }

    if normalized.contains("dubious ownership") {
        return format!(
            "Git refused to open this repository because of ownership checks\n\nRepository path:\n  {}\n\nCheck the directory owner, or mark it safe if you trust it:\n  git config --global --add safe.directory {}\n\nDetails:\n{}",
            start.display(),
            shell_arg(start),
            indent_block(detail)
        );
    }

    if normalized.contains("no such file or directory") || normalized.contains("os error 2") {
        return format!(
            "could not start Git\n\nGack shells out to the `git` executable. Install Git and make sure this works:\n  git --version\n\nDetails:\n{}",
            indent_block(detail)
        );
    }

    format!(
        "could not open Git repository\n\nLooked from:\n  {}\n\nDetails:\n{}",
        start.display(),
        indent_block(detail)
    )
}

fn indent_block(text: &str) -> String {
    text.lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn shell_arg(path: &Path) -> String {
    let text = path.to_string_lossy();
    if text
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':'))
    {
        text.into_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

fn print_help() {
    println!(
        "\
gack - a compact terminal Git panel

Usage:
  gack [--repo <path>]

Options:
  --repo <path>    Open the Git repository containing path
  --config <path>  Load a specific gack config file
  --no-config      Start with built-in defaults only
  -h, --help       Show this help
  -V, --version    Show version

Inside the TUI:
  Up/Down or j/k   Move selection
  Tab              Switch focus between changes and diff
  Space            Mark/unmark a change
  s                Stage marked changes, or focused change
  u                Unstage marked changes, or focused change
  a                Stage all visible unstaged changes
  c                Commit staged changes
  r                Refresh
  ?                Help
  q                Quit or close modal
"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn repo_discovery_error_explains_non_repo() {
        let detail = "git -C /tmp/example rev-parse --show-toplevel failed: fatal: not a git repository (or any of the parent directories): .git";

        let message = format_repo_discovery_error(Path::new("/tmp/example"), detail);

        assert!(message.contains("no Git repository found"));
        assert!(message.contains("Looked from:\n  /tmp/example"));
        assert!(message.contains("gack --repo /path/to/repo"));
        assert!(message.contains("git init"));
        assert!(message.contains("Details:\n  git -C /tmp/example"));
    }

    #[test]
    fn repo_discovery_error_explains_missing_git() {
        let message = format_repo_discovery_error(
            Path::new("/tmp/example"),
            "No such file or directory (os error 2)",
        );

        assert!(message.contains("could not start Git"));
        assert!(message.contains("git --version"));
    }

    #[test]
    fn start_path_can_be_a_file_inside_repo() {
        let temp = tempfile::tempdir().unwrap();
        let file_path = temp.path().join("README.md");
        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(file, "example").unwrap();

        let resolved = resolve_start_path(&file_path).unwrap();

        assert_eq!(resolved, temp.path());
    }

    #[test]
    fn missing_start_path_gets_actionable_message() {
        let missing = Path::new("/tmp/gack-path-that-should-not-exist");
        let error = resolve_start_path(missing).unwrap_err();
        let message = error.to_string();

        assert!(message.contains("repository path does not exist"));
        assert!(message.contains("gack --repo /path/to/repo"));
    }
}
