use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

const SHELF_VERSION: u32 = 1;
const SHELF_DIR: &str = "gitcl-shelf";
const SHELF_INDEX: &str = "shelf.json";

/// One shelved changelist: a named patch (vs the HEAD it was shelved on)
/// stored as `.git/gitcl-shelf/<id>.patch`, described by an entry in
/// `.git/gitcl-shelf/shelf.json`. Like `.git/changelist.json` this is
/// private per-clone state, never committed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShelfEntry {
    pub id: String,
    pub name: String,
    pub created_at: String,
    /// Status paths the patch touches at shelve time, including the old
    /// side of renames — used to assign unshelved files back to a
    /// changelist before the next reconcile runs.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShelfStore {
    pub version: u32,
    pub entries: Vec<ShelfEntry>,
}

impl ShelfStore {
    pub fn new_empty() -> Self {
        ShelfStore {
            version: SHELF_VERSION,
            entries: Vec::new(),
        }
    }

    fn dir(repo_root: &Path) -> PathBuf {
        repo_root.join(".git").join(SHELF_DIR)
    }

    fn index_path(repo_root: &Path) -> PathBuf {
        Self::dir(repo_root).join(SHELF_INDEX)
    }

    pub fn patch_path(repo_root: &Path, id: &str) -> PathBuf {
        Self::dir(repo_root).join(format!("{id}.patch"))
    }

    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = Self::index_path(repo_root);
        if !path.exists() {
            return Ok(Self::new_empty());
        }
        let raw = std::fs::read_to_string(&path).map_err(|source| AppError::StoreRead {
            path: path.clone(),
            source,
        })?;
        let store: ShelfStore = serde_json::from_str(&raw)?;
        Ok(store)
    }

    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let dir = Self::dir(repo_root);
        std::fs::create_dir_all(&dir).map_err(|source| AppError::StoreWrite {
            path: dir.clone(),
            source,
        })?;
        let path = Self::index_path(repo_root);
        let tmp_path = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp_path, json).map_err(|source| AppError::StoreWrite {
            path: tmp_path.clone(),
            source,
        })?;
        std::fs::rename(&tmp_path, &path).map_err(|source| AppError::StoreWrite { path, source })?;
        Ok(())
    }
}

pub fn new_shelf_id() -> String {
    uuid::Uuid::new_v4().to_string()[..8].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();

        let mut store = ShelfStore::new_empty();
        store.entries.push(ShelfEntry {
            id: "abc12345".to_string(),
            name: "Feature".to_string(),
            created_at: "2026-07-07T00:00:00Z".to_string(),
            files: vec!["a.txt".to_string()],
        });
        store.save(root).unwrap();

        let loaded = ShelfStore::load(root).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].name, "Feature");
        assert_eq!(loaded.entries[0].files, vec!["a.txt".to_string()]);
    }

    #[test]
    fn missing_index_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let loaded = ShelfStore::load(root).unwrap();
        assert!(loaded.entries.is_empty());
    }
}
