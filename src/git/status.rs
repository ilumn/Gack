use std::ffi::OsString;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

use super::{Change, ChangeId, ChangeKey, ChangeKind, ChangeSection, GitError, StatusSnapshot};

pub fn parse_status(bytes: &[u8]) -> Result<StatusSnapshot, GitError> {
    let mut snapshot = StatusSnapshot {
        branch: "HEAD".to_string(),
        ahead: None,
        behind: None,
        changes: Vec::new(),
    };

    let records: Vec<&[u8]> = bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect();

    let mut i = 0;
    while i < records.len() {
        let record = records[i];
        let record_text = String::from_utf8_lossy(record);
        if let Some(branch) = record_text.strip_prefix("# branch.head ") {
            snapshot.branch = branch.to_string();
            i += 1;
            continue;
        }
        if let Some(upstream) = record_text.strip_prefix("# branch.ab ") {
            parse_ahead_behind(upstream, &mut snapshot);
            i += 1;
            continue;
        }
        if record.starts_with(b"# ") {
            i += 1;
            continue;
        }
        if record.starts_with(b"1 ") {
            parse_ordinary(record, &mut snapshot)?;
            i += 1;
            continue;
        }
        if record.starts_with(b"2 ") {
            let original = records.get(i + 1).copied();
            parse_renamed(record, original, &mut snapshot)?;
            i += if original.is_some() { 2 } else { 1 };
            continue;
        }
        if record.starts_with(b"u ") {
            parse_unmerged(record, &mut snapshot)?;
            i += 1;
            continue;
        }
        if let Some(path) = record.strip_prefix(b"? ") {
            push_change(
                &mut snapshot,
                ChangeSection::Untracked,
                ChangeKind::Untracked,
                path,
                None,
                "??",
            );
            i += 1;
            continue;
        }
        if let Some(path) = record.strip_prefix(b"! ") {
            push_change(
                &mut snapshot,
                ChangeSection::Ignored,
                ChangeKind::Ignored,
                path,
                None,
                "!!",
            );
            i += 1;
            continue;
        }
        i += 1;
    }

    Ok(snapshot)
}

fn parse_ahead_behind(text: &str, snapshot: &mut StatusSnapshot) {
    for part in text.split_whitespace() {
        if let Some(ahead) = part.strip_prefix('+') {
            snapshot.ahead = ahead.parse().ok();
        } else if let Some(behind) = part.strip_prefix('-') {
            snapshot.behind = behind.parse().ok();
        }
    }
}

fn parse_ordinary(record: &[u8], snapshot: &mut StatusSnapshot) -> Result<(), GitError> {
    let parts = split_fields(record, 9);
    if parts.len() < 9 {
        let text = String::from_utf8_lossy(record);
        return Err(GitError::new(format!("invalid porcelain record: {text}")));
    }

    let xy = ascii_field(parts[1]);
    let path = parts[8];
    push_xy(snapshot, xy, path, None);
    Ok(())
}

fn parse_renamed(
    record: &[u8],
    original: Option<&[u8]>,
    snapshot: &mut StatusSnapshot,
) -> Result<(), GitError> {
    let parts = split_fields(record, 10);
    if parts.len() < 10 {
        let text = String::from_utf8_lossy(record);
        return Err(GitError::new(format!(
            "invalid rename porcelain record: {text}"
        )));
    }

    let xy = ascii_field(parts[1]);
    let path = parts[9];
    push_xy(snapshot, xy, path, original);
    Ok(())
}

fn parse_unmerged(record: &[u8], snapshot: &mut StatusSnapshot) -> Result<(), GitError> {
    let parts = split_fields(record, 11);
    if parts.len() < 11 {
        let text = String::from_utf8_lossy(record);
        return Err(GitError::new(format!(
            "invalid unmerged porcelain record: {text}"
        )));
    }

    let xy = ascii_field(parts[1]);
    let path = parts[10];
    push_change(
        snapshot,
        ChangeSection::Conflict,
        ChangeKind::Conflict,
        path,
        None,
        xy,
    );
    Ok(())
}

fn push_xy(snapshot: &mut StatusSnapshot, xy: &str, path: &[u8], original: Option<&[u8]>) {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');

    if x != '.' && x != ' ' {
        push_change(
            snapshot,
            ChangeSection::Staged,
            kind_from_status(x),
            path,
            original,
            xy,
        );
    }

    if y != '.' && y != ' ' {
        push_change(
            snapshot,
            ChangeSection::Unstaged,
            kind_from_status(y),
            path,
            None,
            xy,
        );
    }
}

fn push_change(
    snapshot: &mut StatusSnapshot,
    section: ChangeSection,
    kind: ChangeKind,
    path: &[u8],
    original: Option<&[u8]>,
    xy: &str,
) {
    let id = ChangeId(snapshot.changes.len());
    let path = path_buf(path);
    let original_path = original.map(path_buf);
    let key = ChangeKey {
        repo_id: None,
        section,
        kind,
        path: path.clone(),
        original_path: original_path.clone(),
        xy: xy.to_string(),
    };
    snapshot.changes.push(Change {
        id,
        key,
        section,
        kind,
        path,
        original_path,
        xy: xy.to_string(),
    });
}

fn split_fields(record: &[u8], expected_fields: usize) -> Vec<&[u8]> {
    let mut fields = Vec::with_capacity(expected_fields);
    let mut start = 0;
    for index in 0..record.len() {
        if record[index] == b' ' && fields.len() + 1 < expected_fields {
            fields.push(&record[start..index]);
            start = index + 1;
        }
    }
    fields.push(&record[start..]);
    fields
}

fn ascii_field(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap_or("")
}

fn path_buf(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from(OsString::from_vec(bytes.to_vec()))
    }

    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

fn kind_from_status(ch: char) -> ChangeKind {
    match ch {
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'R' => ChangeKind::Renamed,
        'C' => ChangeKind::Copied,
        'T' => ChangeKind::TypeChanged,
        _ => ChangeKind::Modified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_staged_and_unstaged_same_file() {
        let raw = [
            b"# branch.head main\0".as_slice(),
            b"# branch.ab +2 -1\0".as_slice(),
            b"1 MM N... 100644 100644 100644 abc abc file.rs\0".as_slice(),
        ]
        .concat();
        let parsed = parse_status(&raw).unwrap();
        assert_eq!(parsed.branch, "main");
        assert_eq!(parsed.ahead, Some(2));
        assert_eq!(parsed.behind, Some(1));
        assert_eq!(parsed.changes.len(), 2);
        assert_eq!(parsed.changes[0].section, ChangeSection::Staged);
        assert_eq!(parsed.changes[1].section, ChangeSection::Unstaged);
    }

    #[test]
    fn parses_untracked() {
        let parsed = parse_status(b"? docs/read me.md\0").unwrap();
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.changes[0].kind, ChangeKind::Untracked);
        assert_eq!(parsed.changes[0].path, PathBuf::from("docs/read me.md"));
    }

    #[test]
    fn parses_rename_target_then_original() {
        let raw = b"2 R. N... 100644 100644 100644 abc abc R100 src/new name.rs\0src/old name.rs\0";
        let parsed = parse_status(raw).unwrap();
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.changes[0].kind, ChangeKind::Renamed);
        assert_eq!(parsed.changes[0].path, PathBuf::from("src/new name.rs"));
        assert_eq!(
            parsed.changes[0].original_path,
            Some(PathBuf::from("src/old name.rs"))
        );
    }

    #[test]
    fn parses_unmerged_as_single_conflict_row() {
        let raw = b"u UU N... 100644 100644 100644 100644 abc abc abc file.rs\0";
        let parsed = parse_status(raw).unwrap();
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.changes[0].kind, ChangeKind::Conflict);
        assert_eq!(parsed.changes[0].section, ChangeSection::Conflict);
    }
}
