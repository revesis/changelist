use std::path::Path;

use crate::error::GitError;
use crate::git::command::run_git;

/// One entry from `git status --porcelain=v2 -z`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    /// Present only for renames/copies: the path this entry was renamed from.
    pub orig_path: Option<String>,
    pub staged: ChangeKind,
    pub worktree: ChangeKind,
    pub untracked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Unmodified,
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
}

impl ChangeKind {
    fn from_char(c: u8) -> Self {
        match c {
            b'.' => ChangeKind::Unmodified,
            b'M' => ChangeKind::Modified,
            b'A' => ChangeKind::Added,
            b'D' => ChangeKind::Deleted,
            b'R' => ChangeKind::Renamed,
            b'C' => ChangeKind::Copied,
            b'T' => ChangeKind::TypeChanged,
            b'U' => ChangeKind::Unmerged,
            _ => ChangeKind::Unmodified,
        }
    }
}

pub fn git_status(repo_root: &Path) -> Result<Vec<StatusEntry>, GitError> {
    let raw = run_git(repo_root, &["status", "--porcelain=v2", "-z"])?;
    parse_porcelain_v2(&raw)
}

/// Parses NUL-separated `git status --porcelain=v2 -z` output.
///
/// Record shapes (space-separated fields, NUL-terminated record(s)):
///   `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`
///   `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>\0<origPath>`
///   `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`
///   `? <path>`
///   `! <path>` (only with --ignored, not requested here)
/// Renamed/copied records consume an extra NUL-terminated field (origPath).
pub fn parse_porcelain_v2(raw: &[u8]) -> Result<Vec<StatusEntry>, GitError> {
    let fields: Vec<&[u8]> = raw.split(|&b| b == 0).filter(|f| !f.is_empty()).collect();
    let mut entries = Vec::new();
    let mut i = 0;
    while i < fields.len() {
        let field = fields[i];
        let text = std::str::from_utf8(field)
            .map_err(|e| GitError::InvalidOutput(e.to_string()))?;
        let mut parts = text.split(' ');
        let kind = parts.next().unwrap_or("");
        match kind {
            "1" => {
                let xy = parts.next().ok_or_else(|| {
                    GitError::InvalidOutput(format!("malformed '1' record: {text}"))
                })?;
                // skip sub, mH, mI, mW, hH, hI (6 fields), remainder is path
                let path = text
                    .splitn(9, ' ')
                    .nth(8)
                    .ok_or_else(|| GitError::InvalidOutput(format!("malformed '1' record: {text}")))?;
                entries.push(StatusEntry {
                    path: path.to_string(),
                    orig_path: None,
                    staged: ChangeKind::from_char(xy.as_bytes()[0]),
                    worktree: ChangeKind::from_char(xy.as_bytes()[1]),
                    untracked: false,
                });
                i += 1;
            }
            "2" => {
                let xy = parts.next().ok_or_else(|| {
                    GitError::InvalidOutput(format!("malformed '2' record: {text}"))
                })?;
                // fields: 2 XY sub mH mI mW hH hI X<score> path  (10 fields before NUL, then origPath in next NUL field)
                let path = text
                    .splitn(10, ' ')
                    .nth(9)
                    .ok_or_else(|| GitError::InvalidOutput(format!("malformed '2' record: {text}")))?;
                let orig_path = fields.get(i + 1).ok_or_else(|| {
                    GitError::InvalidOutput("rename record missing origPath field".to_string())
                })?;
                let orig_path = std::str::from_utf8(orig_path)
                    .map_err(|e| GitError::InvalidOutput(e.to_string()))?
                    .to_string();
                entries.push(StatusEntry {
                    path: path.to_string(),
                    orig_path: Some(orig_path),
                    staged: ChangeKind::from_char(xy.as_bytes()[0]),
                    worktree: ChangeKind::from_char(xy.as_bytes()[1]),
                    untracked: false,
                });
                i += 2;
            }
            "u" => {
                let xy = parts.next().ok_or_else(|| {
                    GitError::InvalidOutput(format!("malformed 'u' record: {text}"))
                })?;
                let path = text
                    .splitn(11, ' ')
                    .nth(10)
                    .ok_or_else(|| GitError::InvalidOutput(format!("malformed 'u' record: {text}")))?;
                entries.push(StatusEntry {
                    path: path.to_string(),
                    orig_path: None,
                    staged: ChangeKind::from_char(xy.as_bytes()[0]),
                    worktree: ChangeKind::from_char(xy.as_bytes()[1]),
                    untracked: false,
                });
                i += 1;
            }
            "?" => {
                let path = text
                    .splitn(2, ' ')
                    .nth(1)
                    .ok_or_else(|| GitError::InvalidOutput(format!("malformed '?' record: {text}")))?;
                entries.push(StatusEntry {
                    path: path.to_string(),
                    orig_path: None,
                    staged: ChangeKind::Unmodified,
                    worktree: ChangeKind::Added,
                    untracked: true,
                });
                i += 1;
            }
            "!" => {
                // Ignored entries are never requested (no --ignored flag), skip defensively.
                i += 1;
            }
            other => {
                return Err(GitError::InvalidOutput(format!(
                    "unrecognized status record kind {other:?} in {text:?}"
                )));
            }
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ordinary_modified_entry() {
        let raw = b"1 .M N... 100644 100644 100644 ce01362 ce01362 a.txt\0";
        let entries = parse_porcelain_v2(raw).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "a.txt");
        assert_eq!(entries[0].staged, ChangeKind::Unmodified);
        assert_eq!(entries[0].worktree, ChangeKind::Modified);
        assert!(!entries[0].untracked);
        assert!(entries[0].orig_path.is_none());
    }

    #[test]
    fn parses_added_entry() {
        let raw = b"1 A. N... 000000 100644 100644 0000000000000000000000000000000000000000 587be6b sub/c.txt\0";
        let entries = parse_porcelain_v2(raw).unwrap();
        assert_eq!(entries[0].path, "sub/c.txt");
        assert_eq!(entries[0].staged, ChangeKind::Added);
        assert_eq!(entries[0].worktree, ChangeKind::Unmodified);
    }

    #[test]
    fn parses_deleted_entry() {
        let raw = b"1 D. N... 100644 000000 000000 ce01362 0000000000000000000000000000000000000000 a.txt\0";
        let entries = parse_porcelain_v2(raw).unwrap();
        assert_eq!(entries[0].path, "a.txt");
        assert_eq!(entries[0].staged, ChangeKind::Deleted);
    }

    #[test]
    fn parses_rename_entry_with_orig_path() {
        let raw = b"2 RM N... 100644 100644 100644 ce01362 ce01362 R100 renamed.txt\0a.txt\0";
        let entries = parse_porcelain_v2(raw).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "renamed.txt");
        assert_eq!(entries[0].orig_path, Some("a.txt".to_string()));
        assert_eq!(entries[0].staged, ChangeKind::Renamed);
        assert_eq!(entries[0].worktree, ChangeKind::Modified);
    }

    #[test]
    fn parses_untracked_entry() {
        let raw = b"? b.txt\0";
        let entries = parse_porcelain_v2(raw).unwrap();
        assert_eq!(entries[0].path, "b.txt");
        assert!(entries[0].untracked);
        assert_eq!(entries[0].worktree, ChangeKind::Added);
    }

    #[test]
    fn parses_mixed_entries() {
        let raw = b"1 D. N... 100644 000000 000000 ce01362 0000000000000000000000000000000000000000 a.txt\01 A. N... 000000 100644 100644 0000000000000000000000000000000000000000 bf24909 renamed.txt\01 A. N... 000000 100644 100644 0000000000000000000000000000000000000000 587be6b sub/c.txt\0? b.txt\0";
        let entries = parse_porcelain_v2(raw).unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].path, "a.txt");
        assert_eq!(entries[1].path, "renamed.txt");
        assert_eq!(entries[2].path, "sub/c.txt");
        assert_eq!(entries[3].path, "b.txt");
        assert!(entries[3].untracked);
    }
}
