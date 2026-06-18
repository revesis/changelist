use std::path::Path;
use std::process::Command;

use crate::error::GitError;

/// Single chokepoint for all `git` subprocess invocations. Every other
/// module in `git/` must call through this — never spawn `git` directly
/// elsewhere, so the "CLI-only" invariant stays easy to audit.
pub fn run_git(repo_root: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .map_err(GitError::Spawn)?;

    if !output.status.success() {
        return Err(GitError::NonZeroExit {
            status: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    Ok(output.stdout)
}

pub fn discover_repo_root(start: &Path) -> Option<std::path::PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(start)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(s))
    }
}
