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

/// Produces a `git apply`-able patch of exactly `paths` (working-tree
/// content diffed against HEAD), used to shelve a changelist. `--binary`
/// so binary files roundtrip through `git apply`; `--no-ext-diff` and
/// `--no-textconv` so configured diff drivers can't turn the output into
/// something `git apply` refuses to consume.
pub fn diff_head_patch(repo_root: &Path, paths: &[&str]) -> Result<Vec<u8>, GitError> {
    let mut args = vec![
        "diff",
        "--binary",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "HEAD",
        "--",
    ];
    args.extend(paths.iter().copied());
    run_git(repo_root, &args)
}
