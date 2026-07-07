use std::path::Path;

use crate::error::{AppError, Result};
use crate::git::diff::diff_head_patch;
use crate::git::index_ops::{add, apply_patch, checkout_head, rm_force};
use crate::git::status::{git_status, ChangeKind, StatusEntry};
use crate::model::changelist::{new_changelist_id, now_rfc3339, Changelist};
use crate::model::shelf::{new_shelf_id, ShelfEntry, ShelfStore};
use crate::model::{ChangelistId, ChangelistStore};

#[derive(Debug)]
pub enum ShelveOutcome {
    Shelved { paths: Vec<String> },
    EmptyChangelist,
}

/// Shelves exactly the files belonging to `changelist_id` (IntelliJ-style
/// Shelve Changes): captures their working-tree state as a patch under
/// `.git/gitcl-shelf/`, then reverts those files to HEAD — without
/// disturbing the staged/unstaged state of any file in another changelist.
///
/// Same isolation invariant as `commit_changelist`: no `git add`, `git
/// checkout`, `git rm` or any pathspec-affecting command ever receives a
/// path outside the target changelist. Shared git quirks: untracked files
/// need a prior scoped `add` before `git diff HEAD` can see them; renames
/// need BOTH old and new path in the pathspec so the patch records the
/// deletion side. Like commit, a partially-staged file shelves its
/// working-tree content — the staged/unstaged split is not preserved.
///
/// Ordering is the safety property: the patch and shelf index are written
/// to disk BEFORE any revert command touches the working tree, so a
/// failure at any point leaves the changes in at least one place, never
/// zero.
pub fn shelve_changelist(
    repo_root: &Path,
    store: &ChangelistStore,
    shelf: &mut ShelfStore,
    changelist_id: &ChangelistId,
    shelf_name: &str,
) -> Result<ShelveOutcome> {
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
        return Ok(ShelveOutcome::EmptyChangelist);
    }

    let untracked: Vec<&str> = target_paths
        .iter()
        .filter(|p| by_path.get(p.as_str()).map(|e| e.untracked).unwrap_or(false))
        .map(|p| p.as_str())
        .collect();
    add(repo_root, &untracked).map_err(AppError::GitCommand)?;

    let mut pathspec: Vec<&str> = Vec::with_capacity(target_paths.len());
    for p in &target_paths {
        pathspec.push(p.as_str());
        if let Some(orig) = by_path.get(p.as_str()).and_then(|e| e.orig_path.as_deref()) {
            pathspec.push(orig);
        }
    }
    let patch = diff_head_patch(repo_root, &pathspec).map_err(AppError::GitCommand)?;
    if patch.is_empty() {
        return Ok(ShelveOutcome::EmptyChangelist);
    }

    let entry = ShelfEntry {
        id: new_shelf_id(),
        name: shelf_name.to_string(),
        created_at: now_rfc3339(),
        files: pathspec.iter().map(|p| p.to_string()).collect(),
    };
    let patch_path = ShelfStore::patch_path(repo_root, &entry.id);
    if let Some(parent) = patch_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| AppError::StoreWrite {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(&patch_path, &patch).map_err(|source| AppError::StoreWrite {
        path: patch_path.clone(),
        source,
    })?;
    shelf.entries.push(entry);
    shelf.save(repo_root)?;

    // Revert: paths that exist in HEAD are restored with a scoped checkout
    // (this also un-does deletions and clears their index state); paths not
    // in HEAD (untracked — just add'ed above —, index-added, or the new
    // side of a rename/copy) are force-removed from index and working tree.
    let mut in_head: Vec<&str> = Vec::new();
    let mut not_in_head: Vec<&str> = Vec::new();
    for p in &target_paths {
        let e = by_path[p.as_str()];
        let is_new = e.untracked
            || e.staged == ChangeKind::Added
            || e.worktree == ChangeKind::Added
            || matches!(e.staged, ChangeKind::Renamed | ChangeKind::Copied);
        if is_new {
            not_in_head.push(p.as_str());
        } else {
            in_head.push(p.as_str());
        }
        // The old side of a rename does exist in HEAD; restore it. (A
        // copy's origin is unchanged and must NOT be touched.)
        if e.staged == ChangeKind::Renamed {
            if let Some(orig) = e.orig_path.as_deref() {
                in_head.push(orig);
            }
        }
    }
    checkout_head(repo_root, &in_head).map_err(AppError::GitCommand)?;
    rm_force(repo_root, &not_in_head).map_err(AppError::GitCommand)?;

    Ok(ShelveOutcome::Shelved {
        paths: target_paths,
    })
}

/// Applies the shelf's patch back onto the working tree (never the index),
/// assigns the restored files to a changelist named after the shelf
/// (reusing an existing changelist with that name, else creating one), and
/// removes the shelf entry. If `git apply` fails — e.g. the tree has since
/// diverged — the shelf entry and patch are kept untouched.
pub fn unshelve(
    repo_root: &Path,
    store: &mut ChangelistStore,
    shelf: &mut ShelfStore,
    shelf_id: &str,
) -> Result<ShelfEntry> {
    let pos = shelf
        .entries
        .iter()
        .position(|e| e.id == shelf_id)
        .ok_or(AppError::ShelfNotFound)?;
    let entry = shelf.entries[pos].clone();
    let patch_path = ShelfStore::patch_path(repo_root, &entry.id);
    apply_patch(repo_root, &patch_path).map_err(AppError::GitCommand)?;

    // Assign before the next reconcile runs, so restored paths land in the
    // shelf's changelist instead of falling back to the active one.
    let cl_id = match store.changelists.iter().find(|c| c.name == entry.name) {
        Some(cl) => cl.id.clone(),
        None => {
            let id = new_changelist_id();
            store
                .changelists
                .push(Changelist::new(id.clone(), entry.name.clone(), None));
            id
        }
    };
    for path in &entry.files {
        store.files.insert(path.clone(), cl_id.clone());
    }
    store.save(repo_root)?;

    shelf.entries.remove(pos);
    shelf.save(repo_root)?;
    let _ = std::fs::remove_file(&patch_path);
    Ok(entry)
}

/// Deletes a shelf entry and its patch file without applying it. This is
/// the only operation that discards shelved changes permanently.
pub fn delete_shelf(repo_root: &Path, shelf: &mut ShelfStore, shelf_id: &str) -> Result<()> {
    let pos = shelf
        .entries
        .iter()
        .position(|e| e.id == shelf_id)
        .ok_or(AppError::ShelfNotFound)?;
    let entry = shelf.entries.remove(pos);
    shelf.save(repo_root)?;
    let _ = std::fs::remove_file(ShelfStore::patch_path(repo_root, &entry.id));
    Ok(())
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
            store.changelists.push(Changelist::new(
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

    fn porcelain(root: &Path) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["status", "--porcelain=v2"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
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

    fn shelve(
        root: &Path,
        store: &ChangelistStore,
        shelf: &mut ShelfStore,
        cl: &str,
        name: &str,
    ) -> ShelveOutcome {
        shelve_changelist(root, store, shelf, &cl.to_string(), name).unwrap()
    }

    #[test]
    fn shelve_reverts_target_and_leaves_other_changelist_untouched() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        write(root, "b.txt", "b1\n");
        commit_all(root, "init");
        write(root, "a.txt", "a1\na2\n");
        write(root, "b.txt", "b1\nb2\n");

        let store = store_with("default", &[("a.txt", "default"), ("b.txt", "feature")]);
        let mut shelf = ShelfStore::new_empty();
        let outcome = shelve(root, &store, &mut shelf, "default", "my shelf");
        assert!(matches!(outcome, ShelveOutcome::Shelved { .. }));

        let s = porcelain(root);
        assert!(!s.contains("a.txt"), "a.txt should be reverted: {s}");
        assert!(s.contains("b.txt"), "b.txt should still be modified: {s}");
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "a1\n"
        );
        assert_eq!(shelf.entries.len(), 1);
        assert!(ShelfStore::patch_path(root, &shelf.entries[0].id).exists());
    }

    #[test]
    fn shelve_preserves_other_staged_file_byte_identical() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        write(root, "b.txt", "b1\n");
        commit_all(root, "init");
        write(root, "a.txt", "a1\na2\n");
        write(root, "b.txt", "b1\nb2\n");
        run(root, &["add", "b.txt"]);
        let before = staged_diff(root, "b.txt");

        let store = store_with("default", &[("a.txt", "default"), ("b.txt", "feature")]);
        let mut shelf = ShelfStore::new_empty();
        shelve(root, &store, &mut shelf, "default", "s");

        let after = staged_diff(root, "b.txt");
        assert_eq!(before, after, "b.txt's staged diff must be untouched");
    }

    #[test]
    fn shelve_unshelve_roundtrip_restores_modification() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        commit_all(root, "init");
        write(root, "a.txt", "a1\na2\n");

        let mut store = store_with("default", &[("a.txt", "default")]);
        let mut shelf = ShelfStore::new_empty();
        shelve(root, &store, &mut shelf, "default", "Feature X");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "a1\n");

        let id = shelf.entries[0].id.clone();
        let entry = unshelve(root, &mut store, &mut shelf, &id).unwrap();
        assert_eq!(entry.name, "Feature X");
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "a1\na2\n"
        );
        assert!(shelf.entries.is_empty());
        assert!(!ShelfStore::patch_path(root, &id).exists());

        // Restored file assigned to a changelist named after the shelf.
        let cl_id = store.files.get("a.txt").unwrap();
        let cl = store.changelist_by_id(cl_id).unwrap();
        assert_eq!(cl.name, "Feature X");
    }

    #[test]
    fn shelve_untracked_removes_file_and_unshelve_restores() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        commit_all(root, "init");
        write(root, "new.txt", "fresh\n");

        let mut store = store_with("default", &[("new.txt", "default")]);
        let mut shelf = ShelfStore::new_empty();
        shelve(root, &store, &mut shelf, "default", "s");

        assert!(!root.join("new.txt").exists());
        // No index residue from the scoped `add` either.
        assert_eq!(porcelain(root), "", "repo should be fully clean");

        let id = shelf.entries[0].id.clone();
        unshelve(root, &mut store, &mut shelf, &id).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("new.txt")).unwrap(),
            "fresh\n"
        );
        // Restored to the working tree only: shows as untracked again.
        assert!(porcelain(root).contains("? new.txt"));
    }

    #[test]
    fn shelve_deletion_restores_file_then_unshelve_deletes_again() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        write(root, "b.txt", "b1\n");
        commit_all(root, "init");
        std::fs::remove_file(root.join("b.txt")).unwrap();

        let mut store = store_with("default", &[("b.txt", "default")]);
        let mut shelf = ShelfStore::new_empty();
        shelve(root, &store, &mut shelf, "default", "s");
        assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "b1\n");
        assert_eq!(porcelain(root), "");

        let id = shelf.entries[0].id.clone();
        unshelve(root, &mut store, &mut shelf, &id).unwrap();
        assert!(!root.join("b.txt").exists());
    }

    #[test]
    fn shelve_rename_roundtrip() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "old.txt", "content\n");
        commit_all(root, "init");
        run(root, &["mv", "old.txt", "new.txt"]);

        let mut store = store_with("default", &[("new.txt", "default")]);
        let mut shelf = ShelfStore::new_empty();
        shelve(root, &store, &mut shelf, "default", "s");

        assert_eq!(
            std::fs::read_to_string(root.join("old.txt")).unwrap(),
            "content\n"
        );
        assert!(!root.join("new.txt").exists());
        assert_eq!(porcelain(root), "");

        let id = shelf.entries[0].id.clone();
        let entry = unshelve(root, &mut store, &mut shelf, &id).unwrap();
        assert!(!root.join("old.txt").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("new.txt")).unwrap(),
            "content\n"
        );
        // Both sides of the rename were recorded and assigned.
        assert!(entry.files.contains(&"new.txt".to_string()));
        assert!(entry.files.contains(&"old.txt".to_string()));
    }

    #[test]
    fn shelve_binary_file_roundtrip() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        commit_all(root, "init");
        let bytes: Vec<u8> = vec![0, 159, 146, 150, 255, 10, 0, 1];
        std::fs::write(root.join("bin.dat"), &bytes).unwrap();

        let mut store = store_with("default", &[("bin.dat", "default")]);
        let mut shelf = ShelfStore::new_empty();
        shelve(root, &store, &mut shelf, "default", "s");
        assert!(!root.join("bin.dat").exists());

        let id = shelf.entries[0].id.clone();
        unshelve(root, &mut store, &mut shelf, &id).unwrap();
        assert_eq!(std::fs::read(root.join("bin.dat")).unwrap(), bytes);
    }

    #[test]
    fn shelve_working_tree_content_of_partially_staged_file() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "g.txt", "g1\n");
        commit_all(root, "init");
        write(root, "g.txt", "g1\ng2\n");
        run(root, &["add", "g.txt"]);
        write(root, "g.txt", "g1\ng2\ng3\n");

        let mut store = store_with("default", &[("g.txt", "default")]);
        let mut shelf = ShelfStore::new_empty();
        shelve(root, &store, &mut shelf, "default", "s");
        assert_eq!(std::fs::read_to_string(root.join("g.txt")).unwrap(), "g1\n");
        assert_eq!(porcelain(root), "");

        let id = shelf.entries[0].id.clone();
        unshelve(root, &mut store, &mut shelf, &id).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("g.txt")).unwrap(),
            "g1\ng2\ng3\n"
        );
    }

    #[test]
    fn unshelve_conflict_keeps_shelf_entry() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        commit_all(root, "init");
        write(root, "a.txt", "a1\na2\n");

        let mut store = store_with("default", &[("a.txt", "default")]);
        let mut shelf = ShelfStore::new_empty();
        shelve(root, &store, &mut shelf, "default", "s");

        // Diverge the file so the patch no longer applies.
        write(root, "a.txt", "completely different\n");

        let id = shelf.entries[0].id.clone();
        let result = unshelve(root, &mut store, &mut shelf, &id);
        assert!(result.is_err(), "apply should conflict");
        assert_eq!(shelf.entries.len(), 1, "shelf entry must survive");
        assert!(ShelfStore::patch_path(root, &id).exists());
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "completely different\n",
            "working tree must be untouched on conflict"
        );
    }

    #[test]
    fn shelve_empty_changelist_is_noop() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        commit_all(root, "init");

        let store = store_with("default", &[]);
        let mut shelf = ShelfStore::new_empty();
        let outcome = shelve(root, &store, &mut shelf, "default", "s");
        assert!(matches!(outcome, ShelveOutcome::EmptyChangelist));
        assert!(shelf.entries.is_empty());
    }

    #[test]
    fn delete_shelf_removes_entry_and_patch() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        commit_all(root, "init");
        write(root, "a.txt", "a1\na2\n");

        let store = store_with("default", &[("a.txt", "default")]);
        let mut shelf = ShelfStore::new_empty();
        shelve(root, &store, &mut shelf, "default", "s");
        let id = shelf.entries[0].id.clone();

        delete_shelf(root, &mut shelf, &id).unwrap();
        assert!(shelf.entries.is_empty());
        assert!(!ShelfStore::patch_path(root, &id).exists());
    }

    #[test]
    fn unshelve_reuses_existing_changelist_with_same_name() {
        let dir = init_repo();
        let root = dir.path();
        write(root, "a.txt", "a1\n");
        commit_all(root, "init");
        write(root, "a.txt", "a1\na2\n");

        let mut store = store_with("default", &[("a.txt", "default")]);
        store.changelists.push(Changelist::new(
            "feat1".to_string(),
            "Feature X".to_string(),
            None,
        ));
        let mut shelf = ShelfStore::new_empty();
        shelve(root, &store, &mut shelf, "default", "Feature X");

        let id = shelf.entries[0].id.clone();
        unshelve(root, &mut store, &mut shelf, &id).unwrap();
        assert_eq!(store.files.get("a.txt"), Some(&"feat1".to_string()));
        // No duplicate changelist created.
        let count = store
            .changelists
            .iter()
            .filter(|c| c.name == "Feature X")
            .count();
        assert_eq!(count, 1);
    }
}
