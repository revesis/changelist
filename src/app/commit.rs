use std::path::Path;

use crate::error::{AppError, Result};
use crate::git::index_ops::{add, commit_only};
use crate::git::status::{git_status, StatusEntry};
use crate::model::{ChangelistId, ChangelistStore};

#[derive(Debug)]
pub enum CommitOutcome {
    Committed { paths: Vec<String> },
    EmptyChangelist,
}

/// Commits exactly the files belonging to `changelist_id`, without
/// disturbing the staged/unstaged state of any file in another changelist.
///
/// Invariant that makes this safe: this function never calls `git add`,
/// `git reset`, or any pathspec-affecting command on a path outside
/// `target_paths`. See `git::index_ops::commit_only` for why `git commit
/// --only` is the load-bearing primitive — verified empirically against
/// real git (modify/delete diff correctly against HEAD; renames need both
/// old and new path in the pathspec or the old tree entry is left behind;
/// untracked paths need a prior scoped `add`; partially-staged target files
/// commit their current working-tree content, not the stale staged content
/// — an intentional limitation, not a bug).
pub fn commit_changelist(
    repo_root: &Path,
    store: &mut ChangelistStore,
    changelist_id: &ChangelistId,
    message: &str,
) -> Result<CommitOutcome> {
    let entries = git_status(repo_root).map_err(AppError::GitCommand)?;
    let by_path: std::collections::HashMap<&str, &StatusEntry> =
        entries.iter().map(|e| (e.path.as_str(), e)).collect();

    let target_paths: Vec<String> = store
        .files_in(changelist_id)
        .into_iter()
        .filter(|p| by_path.contains_key(*p))
        .map(|p| p.to_string())
        .collect();

    if target_paths.is_empty() {
        return Ok(CommitOutcome::EmptyChangelist);
    }

    let untracked: Vec<&str> = target_paths
        .iter()
        .filter(|p| by_path.get(p.as_str()).map(|e| e.untracked).unwrap_or(false))
        .map(|p| p.as_str())
        .collect();
    add(repo_root, &untracked).map_err(AppError::GitCommand)?;

    // For renames, `commit --only` needs BOTH the old and new path in its
    // pathspec to actually remove the old tree entry — passing only the new
    // path silently leaves the old path in HEAD (verified empirically: the
    // commit then just looks like an unrelated add, not a rename).
    let mut pathspec: Vec<&str> = Vec::with_capacity(target_paths.len());
    for p in &target_paths {
        pathspec.push(p.as_str());
        if let Some(orig) = by_path.get(p.as_str()).and_then(|e| e.orig_path.as_deref()) {
            pathspec.push(orig);
        }
    }
    commit_only(repo_root, message, &pathspec).map_err(AppError::GitCommand)?;

    // Committed paths vanish from `git status`; reconcile prunes them from
    // the store automatically on the next refresh (caller's responsibility).
    Ok(CommitOutcome::Committed { paths: target_paths })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run(root, &["init", "-q"]);
        run(root, &["config", "user.email", "a@a.com"]);
        run(root, &["config", "user.name", "a"]);
        dir
    }

    fn run(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(root: &Path, path: &str, contents: &str) {
        std::fs::write(root.join(path), contents).unwrap();
    }

    fn commit_all(root: &Path, msg: &str) {
        run(root, &["add", "-A"]);
        run(root, &["commit", "-q", "-m", msg]);
    }

    fn store_with(active: &str, files: &[(&str, &str)]) -> ChangelistStore {
        let mut store = ChangelistStore::new_empty();
        store.active_changelist = active.to_string();
        if active != "default" {
            store.changelists.push(crate::model::changelist::Changelist::new(
                active.to_string(),
                "Other".to_string(),
                None,
            ));
        }
        for (path, cl) in files {
            store.files.insert(path.to_string(), cl.to_string());
        }
        store
    }

    fn staged_diff(root: &Path, path: &str) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["diff", "--cached", "--", path])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn log_count(root: &Path) -> usize {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["log", "--oneline"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).lines().count()
    }

    #[test]
    fn commits_target_leaves_other_unstaged_file_untouched() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        write(root, "b.txt", "b1\n");
        commit_all(root, "init");
        write(root, "a.txt", "a1\na2\n");
        write(root, "b.txt", "b1\nb2\n");

        let mut store = store_with("default", &[("a.txt", "default"), ("b.txt", "feature")]);
        let outcome = commit_changelist(root, &mut store, &"default".to_string(), "msg").unwrap();
        assert!(matches!(outcome, CommitOutcome::Committed { .. }));

        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["status", "--porcelain=v2"])
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&status.stdout);
        assert!(s.contains("b.txt"), "b.txt should still show as modified: {s}");
        assert!(!s.contains("a.txt"), "a.txt should be clean after commit: {s}");
    }

    #[test]
    fn other_changelists_staged_file_preserved_byte_identical() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        write(root, "b.txt", "b1\n");
        commit_all(root, "init");
        write(root, "a.txt", "a1\na2\n");
        write(root, "b.txt", "b1\nb2\n");
        run(root, &["add", "b.txt"]);
        let before = staged_diff(root, "b.txt");

        let mut store = store_with("default", &[("a.txt", "default"), ("b.txt", "feature")]);
        commit_changelist(root, &mut store, &"default".to_string(), "msg").unwrap();

        let after = staged_diff(root, "b.txt");
        assert_eq!(before, after, "b.txt's staged diff must be untouched");
    }

    #[test]
    fn commits_new_untracked_file() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        commit_all(root, "init");
        write(root, "new.txt", "fresh\n");

        let mut store = store_with("default", &[("new.txt", "default")]);
        commit_changelist(root, &mut store, &"default".to_string(), "add new").unwrap();

        let show = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["show", "HEAD:new.txt"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&show.stdout), "fresh\n");
    }

    #[test]
    fn commits_deletion() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        write(root, "b.txt", "b1\n");
        commit_all(root, "init");
        std::fs::remove_file(root.join("b.txt")).unwrap();

        let mut store = store_with("default", &[("b.txt", "default")]);
        commit_changelist(root, &mut store, &"default".to_string(), "delete b").unwrap();

        let ls = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["ls-tree", "HEAD", "--", "b.txt"])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&ls.stdout).is_empty());
    }

    #[test]
    fn commits_rename() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "old.txt", "content\n");
        commit_all(root, "init");
        // Plain `mv` is not detected as a rename by `git status` (it shows
        // up as an unrelated delete + untracked add) — only a staged rename
        // (`git mv`, or `git add -A` after a manual move) is reported as a
        // single rename record that `commit_only` can act on by new path
        // alone. This mirrors what reconcile() would see in practice.
        run(root, &["mv", "old.txt", "new.txt"]);

        let mut store = store_with("default", &[("new.txt", "default")]);
        commit_changelist(root, &mut store, &"default".to_string(), "rename").unwrap();

        let ls = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["ls-tree", "HEAD", "--name-only"])
            .output()
            .unwrap();
        let names = String::from_utf8_lossy(&ls.stdout);
        assert!(names.contains("new.txt"));
        assert!(!names.contains("old.txt"));
    }

    #[test]
    fn empty_changelist_is_a_noop() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        commit_all(root, "init");
        let before = log_count(root);

        let mut store = store_with("default", &[]);
        let outcome = commit_changelist(root, &mut store, &"default".to_string(), "msg").unwrap();
        assert!(matches!(outcome, CommitOutcome::EmptyChangelist));
        assert_eq!(log_count(root), before, "no new commit should be created");
    }

    #[test]
    fn commits_working_tree_content_of_partially_staged_file() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "g.txt", "g1\n");
        commit_all(root, "init");
        write(root, "g.txt", "g1\ng2\n");
        run(root, &["add", "g.txt"]);
        write(root, "g.txt", "g1\ng2\ng3\n");

        let mut store = store_with("default", &[("g.txt", "default")]);
        commit_changelist(root, &mut store, &"default".to_string(), "commit working tree")
            .unwrap();

        let show = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["show", "HEAD:g.txt"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&show.stdout), "g1\ng2\ng3\n");
    }
}
