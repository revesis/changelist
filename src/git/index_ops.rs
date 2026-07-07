use std::path::Path;

use crate::error::GitError;
use crate::git::command::{run_git, run_git_interactive, run_git_with_env};

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

/// Restores `paths` (both index and working tree) to their HEAD state.
/// Only ever called with paths belonging to the changelist being shelved —
/// the same isolation rule as `commit_only`: never pass a path from
/// another changelist.
pub fn checkout_head(repo_root: &Path, paths: &[&str]) -> Result<(), GitError> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args = vec!["checkout", "-q", "HEAD", "--"];
    args.extend(paths.iter().copied());
    run_git(repo_root, &args)?;
    Ok(())
}

/// Force-removes `paths` from the index and working tree. Used to revert
/// not-in-HEAD paths (untracked/added/rename-new) after their content has
/// been captured in a shelf patch. `-r` because git can report a whole
/// untracked directory as a single `dir/` entry.
pub fn rm_force(repo_root: &Path, paths: &[&str]) -> Result<(), GitError> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args = vec!["rm", "-q", "-f", "-r", "--"];
    args.extend(paths.iter().copied());
    run_git(repo_root, &args)?;
    Ok(())
}

/// Applies a shelf patch to the working tree only (never the index), so
/// unshelved files come back as plain unstaged modifications / untracked
/// files. On conflict git leaves the tree untouched and this returns the
/// error — the caller keeps the shelf entry so nothing is lost.
pub fn apply_patch(repo_root: &Path, patch_file: &Path) -> Result<(), GitError> {
    let patch = patch_file
        .to_str()
        .ok_or_else(|| GitError::InvalidOutput("non-UTF-8 patch path".to_string()))?;
    run_git(repo_root, &["apply", "--whitespace=nowarn", patch])?;
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
    let ssh_command = batch_ssh_command(repo_root);
    run_git_with_env(
        repo_root,
        &["push"],
        &[
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_SSH_COMMAND", ssh_command.as_str()),
        ],
    )?;
    Ok(())
}

/// Interactive fallback for when the batch `push` failed because git or
/// ssh needed terminal input (credentials, key passphrase, host-key
/// confirmation): plain `git push` with inherited stdio and none of the
/// prompt-suppressing env, so both git and ssh prompt natively. Must only
/// run while the TUI has released the terminal (cooked mode, main screen).
pub fn push_interactive(repo_root: &Path) -> Result<(), GitError> {
    run_git_interactive(repo_root, &["push"])
}

/// Composes the ssh command git uses for the push, with BatchMode forced
/// on. `GIT_TERMINAL_PROMPT=0` and the null stdin only silence prompts
/// issued by git itself — ssh's own prompts (unknown-host-key
/// verification, key passphrases) open `/dev/tty` directly, so mid-TUI
/// they scribble over the raw-mode screen and race the event loop for
/// keystrokes. With BatchMode ssh fails fast ("Host key verification
/// failed") and the error lands in the status bar; accept a new host
/// out-of-band with a one-off `ssh <host>` in a real terminal.
///
/// The user's own ssh command is respected in git's precedence order
/// (`GIT_SSH_COMMAND` env > `core.sshCommand` config > `GIT_SSH` env) and
/// the option is appended to it — ssh accepts options before the host
/// argument git adds at the end.
fn batch_ssh_command(repo_root: &Path) -> String {
    let not_blank = |s: String| {
        let t = s.trim().to_string();
        (!t.is_empty()).then_some(t)
    };
    let configured = std::env::var("GIT_SSH_COMMAND")
        .ok()
        .and_then(not_blank)
        .or_else(|| {
            run_git(repo_root, &["config", "core.sshCommand"])
                .ok()
                .map(|raw| String::from_utf8_lossy(&raw).into_owned())
                .and_then(not_blank)
        })
        .or_else(|| std::env::var("GIT_SSH").ok().and_then(not_blank));
    format!("{} -o BatchMode=yes", configured.as_deref().unwrap_or("ssh"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// The "never hangs" property, end-to-end minus a real ssh server: a
    /// push to an ssh remote nobody is listening on must come back as a
    /// prompt-free error, not block waiting for terminal input.
    #[test]
    fn push_to_unreachable_ssh_remote_fails_fast() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let run = |args: &[&str]| {
            let status = Command::new("git").arg("-C").arg(root).args(args).status().unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "a@a.com"]);
        run(&["config", "user.name", "a"]);
        run(&["config", "push.default", "current"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-q", "-m", "init"]);
        // Port 1 on localhost: connection refused immediately, no prompt.
        run(&["remote", "add", "origin", "ssh://git@127.0.0.1:1/nope.git"]);

        let start = std::time::Instant::now();
        let result = push(root);
        assert!(result.is_err(), "push must fail, not hang");
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "push should fail fast, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn batch_ssh_command_defaults_to_plain_ssh() {
        // No core.sshCommand in a fresh repo; the env vars are unlikely to
        // be set in the test environment, but guard the assertion anyway.
        if std::env::var("GIT_SSH_COMMAND").is_ok() || std::env::var("GIT_SSH").is_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        Command::new("git").arg("-C").arg(root).args(["init", "-q"]).status().unwrap();
        assert_eq!(batch_ssh_command(root), "ssh -o BatchMode=yes");
    }

    #[test]
    fn batch_ssh_command_appends_to_core_ssh_command() {
        if std::env::var("GIT_SSH_COMMAND").is_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let run = |args: &[&str]| {
            Command::new("git").arg("-C").arg(root).args(args).status().unwrap();
        };
        run(&["init", "-q"]);
        run(&["config", "core.sshCommand", "ssh -i ~/.ssh/work_key"]);
        assert_eq!(
            batch_ssh_command(root),
            "ssh -i ~/.ssh/work_key -o BatchMode=yes"
        );
    }
}
