use std::path::Path;

use crate::error::GitError;
use crate::git::command::run_git;

pub fn diff_worktree(repo_root: &Path, path: &str) -> Result<String, GitError> {
    let raw = run_git(repo_root, &["diff", "--", path])?;
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

pub fn diff_staged(repo_root: &Path, path: &str) -> Result<String, GitError> {
    let raw = run_git(repo_root, &["diff", "--cached", "--", path])?;
    Ok(String::from_utf8_lossy(&raw).into_owned())
}
