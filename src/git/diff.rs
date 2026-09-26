use std::path::Path;

use crate::error::GitError;
use crate::git::command::{run_git, run_git_allowing};

pub fn diff_worktree(repo_root: &Path, path: &str) -> Result<String, GitError> {
    let raw = run_git(repo_root, &["diff", "--", path])?;
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

/// Diff of an untracked path as an all-added "new file". Plain `git diff`
/// compares against the index, where an untracked file has no entry, so it
/// prints nothing; `--no-index` against `/dev/null` synthesizes the diff
/// without touching the index. git reports a wholly-untracked directory as
/// a single `dir/` entry, so directories are expanded to their untracked
/// files (honoring .gitignore) and the per-file diffs concatenated.
pub fn diff_untracked(repo_root: &Path, path: &str) -> Result<String, GitError> {
    let files: Vec<String> = if path.ends_with('/') {
        let raw = run_git(
            repo_root,
            &["ls-files", "--others", "--exclude-standard", "-z", "--", path],
        )?;
        raw.split(|&b| b == 0)
            .filter(|f| !f.is_empty())
            .map(|f| String::from_utf8_lossy(f).into_owned())
            .collect()
    } else {
        vec![path.to_string()]
    };
    let mut out = String::new();
    for file in &files {
        let raw = run_git_allowing(
            repo_root,
            &["diff", "--no-index", "--", "/dev/null", file],
            &[0, 1],
        )?;
        out.push_str(&String::from_utf8_lossy(&raw));
    }
    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git").arg("-C").arg(dir.path()).args(args).status().unwrap().success());
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        fs::write(dir.path().join("tracked.txt"), "x\n").unwrap();
        git(&["add", "tracked.txt"]);
        git(&["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn untracked_file_diff_shows_new_content() {
        let dir = init_repo();
        fs::write(dir.path().join("new.txt"), "hello\nworld\n").unwrap();
        assert_eq!(diff_worktree(dir.path(), "new.txt").unwrap(), "");
        let d = diff_untracked(dir.path(), "new.txt").unwrap();
        assert!(d.contains("new file mode"), "{d}");
        assert!(d.contains("+hello") && d.contains("+world"), "{d}");
    }

    #[test]
    fn untracked_dir_entry_diffs_each_file_and_skips_ignored() {
        let dir = init_repo();
        fs::create_dir(dir.path().join("qwe")).unwrap();
        fs::write(dir.path().join("qwe/a.txt"), "aaa\n").unwrap();
        fs::write(dir.path().join("qwe/b.log"), "bbb\n").unwrap();
        fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();
        let d = diff_untracked(dir.path(), "qwe/").unwrap();
        assert!(d.contains("+aaa"), "{d}");
        assert!(!d.contains("+bbb"), "{d}");
    }
}
