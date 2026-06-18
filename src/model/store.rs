use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::git::status::StatusEntry;
use crate::model::changelist::{Changelist, ChangelistId, DEFAULT_CHANGELIST_ID};

const STORE_VERSION: u32 = 1;
const STORE_FILE: &str = "changelist.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangelistStore {
    pub version: u32,
    pub active_changelist: ChangelistId,
    pub changelists: Vec<Changelist>,
    pub files: HashMap<String, ChangelistId>,
}

impl ChangelistStore {
    pub fn new_empty() -> Self {
        ChangelistStore {
            version: STORE_VERSION,
            active_changelist: DEFAULT_CHANGELIST_ID.to_string(),
            changelists: vec![Changelist::new_default()],
            files: HashMap::new(),
        }
    }

    fn store_path(repo_root: &Path) -> PathBuf {
        repo_root.join(".git").join(STORE_FILE)
    }

    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = Self::store_path(repo_root);
        if !path.exists() {
            return Ok(Self::new_empty());
        }
        let raw = std::fs::read_to_string(&path).map_err(|source| AppError::StoreRead {
            path: path.clone(),
            source,
        })?;
        let store: ChangelistStore = serde_json::from_str(&raw)?;
        Ok(store)
    }

    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let path = Self::store_path(repo_root);
        let tmp_path = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp_path, json).map_err(|source| AppError::StoreWrite {
            path: tmp_path.clone(),
            source,
        })?;
        std::fs::rename(&tmp_path, &path).map_err(|source| AppError::StoreWrite { path, source })?;
        Ok(())
    }

    pub fn changelist_by_id(&self, id: &ChangelistId) -> Option<&Changelist> {
        self.changelists.iter().find(|c| &c.id == id)
    }

    pub fn changelist_by_id_mut(&mut self, id: &ChangelistId) -> Option<&mut Changelist> {
        self.changelists.iter_mut().find(|c| &c.id == id)
    }

    pub fn files_in(&self, changelist_id: &ChangelistId) -> Vec<&str> {
        let mut paths: Vec<&str> = self
            .files
            .iter()
            .filter(|(_, cl)| *cl == changelist_id)
            .map(|(path, _)| path.as_str())
            .collect();
        paths.sort_unstable();
        paths
    }

    /// Reconciles the persisted file->changelist map against live `git status`
    /// output. Returns true if the store was modified (caller should persist).
    ///
    /// Order matters: renames are resolved first (so a renamed file keeps its
    /// changelist), then new paths try to inherit from a directory
    /// collapse/expand match (see `inherited_changelist_for`), falling back to
    /// the active changelist, then paths no longer reported by git status are
    /// pruned.
    pub fn reconcile(&mut self, entries: &[StatusEntry]) -> bool {
        let mut dirty = false;
        let current_paths: std::collections::HashSet<&str> =
            entries.iter().map(|e| e.path.as_str()).collect();

        // 1. Renames: carry the changelist assignment from orig_path to path.
        for entry in entries {
            if let Some(orig_path) = &entry.orig_path {
                if let Some(cl_id) = self.files.remove(orig_path) {
                    self.files.insert(entry.path.clone(), cl_id);
                    dirty = true;
                }
            }
        }

        // Snapshot paths about to vanish (after renames, before pruning) so
        // directory collapse/expand can inherit their changelist below. Git
        // reports a whole untracked directory as a single `dir/` entry once
        // it contains no tracked/staged files, and expands it back into
        // individual paths once one appears inside — without this, the
        // path identity change would otherwise look like an unrelated new
        // path and silently fall back to the active changelist.
        let vanishing: Vec<(String, ChangelistId)> = self
            .files
            .iter()
            .filter(|(p, _)| !current_paths.contains(p.as_str()))
            .map(|(p, cl)| (p.clone(), cl.clone()))
            .collect();

        // 2. New paths: inherit from a directory collapse/expand match if
        // one exists, otherwise auto-assign to the active changelist.
        for entry in entries {
            if !self.files.contains_key(&entry.path) {
                let cl_id = inherited_changelist_for(&entry.path, &vanishing)
                    .unwrap_or_else(|| self.active_changelist.clone());
                self.files.insert(entry.path.clone(), cl_id);
                dirty = true;
            }
        }

        // 3. Prune paths no longer present in git status.
        let stale: Vec<String> = self
            .files
            .keys()
            .filter(|p| !current_paths.contains(p.as_str()))
            .cloned()
            .collect();
        for path in stale {
            self.files.remove(&path);
            dirty = true;
        }

        dirty
    }
}

/// Finds a vanishing path whose directory relationship to `new_path` implies
/// continuity rather than an unrelated new file:
/// - directory collapse: `new_path` is a directory (`dir/`) and a vanishing
///   path was inside it (e.g. `qwe/321` -> `qwe/`)
/// - directory expand: a vanishing path is a directory and `new_path` is now
///   inside it (e.g. `qwe/` -> `qwe/321`)
fn inherited_changelist_for(
    new_path: &str,
    vanishing: &[(String, ChangelistId)],
) -> Option<ChangelistId> {
    vanishing.iter().find_map(|(old_path, cl)| {
        let collapse = new_path.ends_with('/') && old_path.starts_with(new_path);
        let expand = old_path.ends_with('/') && new_path.starts_with(old_path.as_str());
        (collapse || expand).then(|| cl.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::status::ChangeKind;

    fn entry(path: &str, staged: ChangeKind, worktree: ChangeKind, untracked: bool) -> StatusEntry {
        StatusEntry {
            path: path.to_string(),
            orig_path: None,
            staged,
            worktree,
            untracked,
        }
    }

    #[test]
    fn new_path_auto_assigned_to_active_changelist() {
        let mut store = ChangelistStore::new_empty();
        store.active_changelist = "default".to_string();
        let entries = vec![entry("a.txt", ChangeKind::Unmodified, ChangeKind::Modified, false)];
        let dirty = store.reconcile(&entries);
        assert!(dirty);
        assert_eq!(store.files.get("a.txt"), Some(&"default".to_string()));
    }

    #[test]
    fn vanished_path_is_pruned() {
        let mut store = ChangelistStore::new_empty();
        store.files.insert("a.txt".to_string(), "default".to_string());
        let dirty = store.reconcile(&[]);
        assert!(dirty);
        assert!(store.files.is_empty());
    }

    #[test]
    fn existing_assignment_for_still_present_path_is_untouched() {
        let mut store = ChangelistStore::new_empty();
        store.files.insert("a.txt".to_string(), "feature".to_string());
        let entries = vec![entry("a.txt", ChangeKind::Unmodified, ChangeKind::Modified, false)];
        let dirty = store.reconcile(&entries);
        assert!(!dirty);
        assert_eq!(store.files.get("a.txt"), Some(&"feature".to_string()));
    }

    #[test]
    fn rename_carries_over_changelist_assignment() {
        let mut store = ChangelistStore::new_empty();
        store.files.insert("old.txt".to_string(), "feature".to_string());
        let mut e = entry("new.txt", ChangeKind::Renamed, ChangeKind::Modified, false);
        e.orig_path = Some("old.txt".to_string());
        let dirty = store.reconcile(&[e]);
        assert!(dirty);
        assert_eq!(store.files.get("new.txt"), Some(&"feature".to_string()));
        assert!(!store.files.contains_key("old.txt"));
    }

    #[test]
    fn directory_collapse_inherits_changelist() {
        // qwe/321 was the only untracked file in qwe/; after unstaging it,
        // git collapses the status entry to the bare directory "qwe/".
        let mut store = ChangelistStore::new_empty();
        store.active_changelist = "default".to_string();
        store.files.insert("qwe/321".to_string(), "custom".to_string());
        let entries = vec![entry("qwe/", ChangeKind::Unmodified, ChangeKind::Added, true)];
        let dirty = store.reconcile(&entries);
        assert!(dirty);
        assert_eq!(store.files.get("qwe/"), Some(&"custom".to_string()));
        assert!(!store.files.contains_key("qwe/321"));
    }

    #[test]
    fn directory_expand_inherits_changelist() {
        // qwe/ (the whole directory, untracked) expands back into individual
        // file entries once git can see more than one path inside it.
        let mut store = ChangelistStore::new_empty();
        store.active_changelist = "default".to_string();
        store.files.insert("qwe/".to_string(), "custom".to_string());
        let entries = vec![
            entry("qwe/321", ChangeKind::Unmodified, ChangeKind::Added, true),
            entry("qwe/789", ChangeKind::Unmodified, ChangeKind::Added, true),
        ];
        let dirty = store.reconcile(&entries);
        assert!(dirty);
        assert_eq!(store.files.get("qwe/321"), Some(&"custom".to_string()));
        assert_eq!(store.files.get("qwe/789"), Some(&"custom".to_string()));
        assert!(!store.files.contains_key("qwe/"));
    }

    #[test]
    fn unrelated_new_path_still_falls_back_to_active_changelist() {
        let mut store = ChangelistStore::new_empty();
        store.active_changelist = "default".to_string();
        store.files.insert("qwe/321".to_string(), "custom".to_string());
        // "other.txt" has no directory relationship to the vanishing path.
        let entries = vec![
            entry("qwe/321", ChangeKind::Unmodified, ChangeKind::Modified, false),
            entry("other.txt", ChangeKind::Unmodified, ChangeKind::Added, true),
        ];
        store.reconcile(&entries);
        assert_eq!(store.files.get("other.txt"), Some(&"default".to_string()));
    }

    #[test]
    fn no_changes_means_not_dirty() {
        let mut store = ChangelistStore::new_empty();
        store.files.insert("a.txt".to_string(), "default".to_string());
        let entries = vec![entry("a.txt", ChangeKind::Unmodified, ChangeKind::Modified, false)];
        let dirty = store.reconcile(&entries);
        assert!(!dirty);
    }
}
