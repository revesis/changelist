use std::path::Path;
use std::process::Command;

use crate::error::GitError;

/// Single chokepoint for all `git` subprocess invocations. Every other
/// module in `git/` must call through this — never spawn `git` directly
/// elsewhere, so the "CLI-only" invariant stays easy to audit.
pub fn run_git(repo_root: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
    run_git_with_env(repo_root, args, &[])
}

/// Like `run_git`, but with extra environment variables set on the child.
/// Used by commands (e.g. `push`) that may otherwise try to prompt for
/// credentials on a terminal — the TUI holds the real terminal in raw mode,
/// so the child's stdin isn't a usable interactive prompt and would just
/// hang forever waiting for input that can never arrive.
pub fn run_git_with_env(
    repo_root: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> Result<Vec<u8>, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .envs(envs.iter().copied())
        .stdin(std::process::Stdio::null())
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

/// Runs git with stdin/stdout/stderr inherited from the parent process,
/// for the rare command that is deliberately interactive (see
/// `index_ops::push_interactive`). Only valid while the TUI has handed the
/// real terminal back — the caller must have left raw mode and the
/// alternate screen first, and restore them afterwards. stderr is not
/// captured (the user watches it live), so the returned `NonZeroExit`
/// carries only the status code.
pub fn run_git_interactive(repo_root: &Path, args: &[&str]) -> Result<(), GitError> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .status()
        .map_err(GitError::Spawn)?;
    if !status.success() {
        return Err(GitError::NonZeroExit {
            status: status.code().unwrap_or(-1),
            stderr: String::new(),
        });
    }
    Ok(())
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
