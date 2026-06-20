use std::path::Path;

use crate::error::GitError;
use crate::git::command::{run_git, run_git_with_env};

pub fn add(repo_root: &Path, paths: &[&str]) -> Result<(), GitError> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args = vec!["add", "--"];
    args.extend(paths.iter().copied());
    run_git(repo_root, &args)?;
    Ok(())
}

pub fn reset_path(repo_root: &Path, path: &str) -> Result<(), GitError> {
    run_git(repo_root, &["reset", "--", path])?;
    Ok(())
}

/// Commits exactly `paths` (working-tree content, diffed against HEAD) on
/// top of HEAD, leaving the index untouched for every other path — this is
/// what makes per-changelist commit isolation possible with plain git.
/// Verified empirically against real git: handles modified/deleted paths
/// directly; renamed paths need BOTH the old and new path passed in
/// `paths` (the old path alone left behind otherwise won't be removed from
/// the resulting tree); untracked paths must already be staged via `add`
/// first (git refuses "--only" on a pathspec with no HEAD/index entry to
/// diff).
pub fn commit_only(repo_root: &Path, message: &str, paths: &[&str]) -> Result<(), GitError> {
    let mut args = vec!["commit", "--only", "-m", message, "--"];
    args.extend(paths.iter().copied());
    run_git(repo_root, &args)?;
    Ok(())
}

/// Pushes the current branch via plain `git push` (relies on the branch's
/// configured upstream; if none is set, surfaces git's own error asking the
/// user to set one rather than guessing a remote/branch to push to).
/// `GIT_TERMINAL_PROMPT=0` (plus a null stdin) makes git fail fast with a
/// "could not read Username" error instead of hanging: the TUI owns the
/// real terminal in raw mode, so an interactive credential prompt has no
/// usable stdin to read from and would otherwise block forever. Set up a
/// credential helper or SSH key beforehand so pushes don't need a prompt.
pub fn push(repo_root: &Path) -> Result<(), GitError> {
    run_git_with_env(repo_root, &["push"], &[("GIT_TERMINAL_PROMPT", "0")])?;
    Ok(())
}
