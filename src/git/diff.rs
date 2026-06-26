use super::{DiffTarget, GitError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Header,
    Hunk,
    Add,
    Delete,
    Context,
    Metadata,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub start_line: usize,
    pub header: String,
    pub raw_start: usize,
    pub raw_end: usize,
}

#[derive(Debug, Clone)]
pub struct Diff {
    pub target: DiffTarget,
    pub context_lines: u8,
    pub whitespace: WhitespaceMode,
    pub lines: Vec<DiffLine>,
    pub hunks: Vec<DiffHunk>,
    pub is_binary: bool,
    pub additions: usize,
    pub deletions: usize,
    pub raw_patch: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WhitespaceMode {
    Normal,
    IgnoreSpaceChange,
    IgnoreAllSpace,
}

impl WhitespaceMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::IgnoreSpaceChange => "space-change",
            Self::IgnoreAllSpace => "all-space",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Normal => Self::IgnoreSpaceChange,
            Self::IgnoreSpaceChange => Self::IgnoreAllSpace,
            Self::IgnoreAllSpace => Self::Normal,
        }
    }
}

impl Diff {
    pub fn selected_hunk_patch(&self, index: usize) -> Option<Vec<u8>> {
        let hunk = self.hunks.get(index)?;
        let first_hunk = self.hunks.first()?;
        let mut patch = self.raw_patch[..first_hunk.raw_start].to_vec();
        patch.extend_from_slice(&self.raw_patch[hunk.raw_start..hunk.raw_end]);
        Some(patch)
    }
}

pub fn parse_diff(
    target: DiffTarget,
    bytes: &[u8],
    context_lines: u8,
    whitespace: WhitespaceMode,
) -> Result<Diff, GitError> {
    let mut lines = Vec::new();
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut is_binary = false;
    let mut additions = 0;
    let mut deletions = 0;

    let mut offset = 0;
    while offset < bytes.len() {
        let line_start = offset;
        let mut line_end = offset;
        while line_end < bytes.len() && bytes[line_end] != b'\n' {
            line_end += 1;
        }
        offset = if line_end < bytes.len() {
            line_end + 1
        } else {
            line_end
        };

        let mut display_end = line_end;
        if display_end > line_start && bytes[display_end - 1] == b'\r' {
            display_end -= 1;
        }
        let line_bytes = &bytes[line_start..display_end];
        let line = String::from_utf8_lossy(line_bytes);

        let kind = if line.starts_with("@@") {
            if let Some(previous) = hunks.last_mut() {
                previous.raw_end = line_start;
            }
            hunks.push(DiffHunk {
                start_line: lines.len(),
                header: line.to_string(),
                raw_start: line_start,
                raw_end: bytes.len(),
            });
            DiffKind::Hunk
        } else if line.starts_with("diff --git")
            || line.starts_with("+++")
            || line.starts_with("---")
        {
            DiffKind::Header
        } else if line.starts_with("Binary files") {
            is_binary = true;
            DiffKind::Metadata
        } else if line.starts_with('+') {
            additions += 1;
            DiffKind::Add
        } else if line.starts_with('-') {
            deletions += 1;
            DiffKind::Delete
        } else if line.starts_with(' ') {
            DiffKind::Context
        } else {
            DiffKind::Metadata
        };

        lines.push(DiffLine {
            kind,
            text: line.to_string(),
        });
    }

    if lines.is_empty() {
        lines.push(DiffLine {
            kind: DiffKind::Metadata,
            text: "No diff available for this selection.".to_string(),
        });
    }

    Ok(Diff {
        target,
        context_lines,
        whitespace,
        lines,
        hunks,
        is_binary,
        additions,
        deletions,
        raw_patch: bytes.to_vec(),
    })
}
