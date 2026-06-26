use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use super::GitError;

const MAX_CAPTURED_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

pub fn git_output(repo: Option<&Path>, args: &[&str], read_only: bool) -> Result<Output, GitError> {
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    git_output_os(repo, &args, read_only)
}

pub fn git_output_os(
    repo: Option<&Path>,
    args: &[OsString],
    read_only: bool,
) -> Result<Output, GitError> {
    let spec = GitCommandSpec {
        repo: repo.map(Path::to_path_buf),
        args: args.to_vec(),
        read_only,
        stdin: None,
    };
    spec.output()
}

pub fn git_output_os_with_stdin(
    repo: Option<&Path>,
    args: &[OsString],
    read_only: bool,
    stdin: Vec<u8>,
) -> Result<Output, GitError> {
    let spec = GitCommandSpec {
        repo: repo.map(Path::to_path_buf),
        args: args.to_vec(),
        read_only,
        stdin: Some(stdin),
    };
    spec.output()
}

#[derive(Debug, Clone)]
pub struct GitCommandSpec {
    pub repo: Option<std::path::PathBuf>,
    pub args: Vec<OsString>,
    pub read_only: bool,
    pub stdin: Option<Vec<u8>>,
}

impl GitCommandSpec {
    pub fn output(&self) -> Result<Output, GitError> {
        let mut command = self.command();
        let (stdout_path, stdout_file) = capture_file("stdout")?;
        let (stderr_path, stderr_file) = capture_file("stderr")?;
        command.stdout(Stdio::from(stdout_file));
        command.stderr(Stdio::from(stderr_file));
        if self.stdin.is_some() {
            command.stdin(Stdio::piped());
        }

        let mut child = command.spawn()?;
        if let Some(stdin) = &self.stdin {
            let mut child_stdin = child
                .stdin
                .take()
                .ok_or_else(|| GitError::new("failed to open git stdin"))?;
            child_stdin.write_all(stdin)?;
        }

        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() >= GIT_COMMAND_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                cleanup_capture_files(&stdout_path, &stderr_path);
                return Err(GitError::new(command_timeout_os(&self.args)));
            }
            std::thread::sleep(Duration::from_millis(20));
        };

        let output_len = capture_file_len(&stdout_path)? + capture_file_len(&stderr_path)?;
        if output_len > MAX_CAPTURED_OUTPUT_BYTES as u64 {
            cleanup_capture_files(&stdout_path, &stderr_path);
            return Err(GitError::new(command_output_too_large_os(&self.args)));
        }
        let output = Output {
            status,
            stdout: read_capture_file(&stdout_path)?,
            stderr: read_capture_file(&stderr_path)?,
        };
        cleanup_capture_files(&stdout_path, &stderr_path);
        if output.status.success() {
            Ok(output)
        } else {
            Err(GitError::new(command_error_os(&self.args, &output)))
        }
    }

    pub fn command(&self) -> Command {
        command(self.repo.as_deref(), &self.args, self.read_only)
    }
}

#[cfg(test)]
fn command_for_test(repo: Option<&Path>, args: &[OsString], read_only: bool) -> Command {
    command(repo, args, read_only)
}

pub fn git_status_os(
    repo: Option<&Path>,
    args: &[OsString],
    read_only: bool,
) -> Result<(), GitError> {
    let output = git_output_os(repo, args, read_only)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GitError::new(command_error_os(args, &output)))
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout_when_using_spawn_wrapper() {
        let output = git_output(None, &["--version"], true).unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).contains("git version"));
    }

    #[test]
    fn passes_stdin_to_git_command() {
        let args = vec![OsString::from("hash-object"), OsString::from("--stdin")];
        let output = git_output_os_with_stdin(None, &args, true, b"hello".to_vec()).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "b6fc4c620b67d95f953a5c1c1230aaab5db5a1b0"
        );
    }

    #[test]
    fn read_only_commands_get_optional_locks() {
        let args = vec![OsString::from("status")];
        let command = command_for_test(None, &args, true);
        let envs: Vec<_> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(
            envs.iter().any(|(key, value)| {
                key == "GIT_OPTIONAL_LOCKS" && value.as_deref() == Some("0")
            })
        );
    }

    #[test]
    fn mutating_commands_do_not_get_optional_locks() {
        let args = vec![OsString::from("add")];
        let command = command_for_test(None, &args, false);
        assert!(
            !command
                .get_envs()
                .any(|(key, _)| key == "GIT_OPTIONAL_LOCKS")
        );
    }

    #[test]
    fn redacts_url_userinfo_from_errors() {
        assert_eq!(
            redact_git_message("fatal: https://token@example.com/org/repo.git failed"),
            "fatal: https://<redacted>@example.com/org/repo.git failed"
        );
    }

    #[test]
    fn redacts_query_secret_parameters() {
        assert_eq!(
            redact_git_message(
                "fatal: https://example.com/org/repo.git?access_token=secret&x=1 failed"
            ),
            "fatal: https://example.com/org/repo.git?access_token=<redacted>&x=1 failed"
        );
    }

    #[test]
    fn truncates_extremely_long_error_lines() {
        let long = format!("fatal: {}", "x".repeat(700));
        let redacted = redact_git_message(&long);
        assert!(redacted.ends_with("...<truncated>"));
        assert!(redacted.len() < long.len());
    }
}

fn command(repo: Option<&Path>, args: &[OsString], read_only: bool) -> Command {
    let mut command = Command::new("git");
    if let Some(repo) = repo {
        command.arg("-C").arg(repo);
    }
    command.args(args);
    command.env("GIT_PAGER", "cat");
    command.env("PAGER", "cat");
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_LITERAL_PATHSPECS", "1");
    command.env("LC_ALL", "C");
    if read_only {
        command.env("GIT_OPTIONAL_LOCKS", "0");
    }
    command
}

fn command_error_os(args: &[OsString], output: &Output) -> String {
    let command = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    command_error_text(&command, output)
}

fn command_timeout_os(args: &[OsString]) -> String {
    let command = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    format!("git {command} timed out")
}

fn command_error_text(command: &str, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };

    if detail.is_empty() {
        format!("git {command} failed with {}", output.status)
    } else {
        format!("git {command} failed: {}", redact_git_message(detail))
    }
}

fn capture_file(label: &str) -> Result<(PathBuf, File), std::io::Error> {
    let base = std::env::temp_dir();
    for attempt in 0..1000 {
        let path = base.join(format!(
            "gack-{}-{}-{}-{attempt}.tmp",
            std::process::id(),
            monotonic_nanos(),
            label
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate capture file",
    ))
}

fn read_capture_file(path: &Path) -> Result<Vec<u8>, std::io::Error> {
    fs::read(path)
}

fn capture_file_len(path: &Path) -> Result<u64, std::io::Error> {
    fs::metadata(path).map(|metadata| metadata.len())
}

fn cleanup_capture_files(stdout_path: &Path, stderr_path: &Path) {
    let _ = fs::remove_file(stdout_path);
    let _ = fs::remove_file(stderr_path);
}

fn monotonic_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn command_output_too_large_os(args: &[OsString]) -> String {
    let command = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    format!("git {command} produced too much output")
}

pub fn redact_git_message(input: &str) -> String {
    input
        .lines()
        .map(|line| truncate_error_line(&redact_query_secrets(&redact_url_userinfo(line))))
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_url_userinfo(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(scheme_end) = rest.find("://") {
        let auth_start = scheme_end + 3;
        out.push_str(&rest[..auth_start]);
        let after_scheme = &rest[auth_start..];
        let end = after_scheme
            .find(|ch: char| {
                ch.is_whitespace() || ch == '"' || ch == '\'' || ch == '<' || ch == '>'
            })
            .unwrap_or(after_scheme.len());
        let candidate = &after_scheme[..end];
        if let Some(at) = candidate.find('@') {
            out.push_str("<redacted>");
            out.push_str(&candidate[at..]);
        } else {
            out.push_str(candidate);
        }
        rest = &after_scheme[end..];
    }
    out.push_str(rest);
    out
}

fn redact_query_secrets(input: &str) -> String {
    let mut out = input.to_string();
    for key in ["token", "access_token", "auth", "password", "passwd", "key"] {
        out = redact_query_key(&out, key);
    }
    out
}

fn redact_query_key(input: &str, key: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(index) = find_query_key(rest, key) {
        let value_start = index + key.len() + 1;
        out.push_str(&rest[..value_start]);
        out.push_str("<redacted>");
        let after_value = &rest[value_start..];
        let value_end = after_value
            .find(['&', '#', ' ', '\t', '\r', '\n', '"', '\''])
            .unwrap_or(after_value.len());
        rest = &after_value[value_end..];
    }
    out.push_str(rest);
    out
}

fn find_query_key(input: &str, key: &str) -> Option<usize> {
    for marker in ['?', '&'] {
        let needle = format!("{marker}{key}=");
        if let Some(index) = input.find(&needle) {
            return Some(index + 1);
        }
    }
    None
}

fn truncate_error_line(input: &str) -> String {
    const MAX_LINE: usize = 500;
    if input.chars().count() <= MAX_LINE {
        return input.to_string();
    }
    let mut out = input.chars().take(MAX_LINE).collect::<String>();
    out.push_str("...<truncated>");
    out
}
