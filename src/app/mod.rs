pub mod actions;
pub mod commit;
pub mod shelve;

use std::collections::HashSet;
use std::path::PathBuf;

use crate::error::{AppError, Result};
use crate::git::diff::{diff_staged, diff_worktree};
use crate::git::index_ops::{add as git_add, push as git_push, reset_path as git_reset_path};
use crate::git::status::{git_status, ChangeKind, StatusEntry};
use crate::model::{ChangelistId, ChangelistStore, ShelfStore};

pub use actions::{Action, Popup};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    Header(usize),
    File(usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Tree,
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
    WorkTree,
    Staged,
}

pub struct App {
    pub repo_root: PathBuf,
    pub store: ChangelistStore,
    pub shelf: ShelfStore,
    pub status: Vec<StatusEntry>,
    pub focused_pane: Pane,
    pub tree_rows: Vec<TreeRow>,
    pub tree_cursor: usize,
    pub collapsed: HashSet<usize>,
    pub tree_hscroll: u16,
    pub status_message: Option<String>,
    pub should_quit: bool,
    pub popup: Option<Popup>,
    pub diff_mode: DiffMode,
    pub show_help: bool,
    pub diff_scroll: u16,
    pub diff_hscroll: u16,
    /// When `Some(idx)`, the tree is in visual (batch-select) mode anchored
    /// at `idx` (an index into `tree_rows` pointing at a File row).
    pub visual_anchor: Option<usize>,
    /// Set after a batch push failed with a prompt-shaped error: the next
    /// `Action::Push` requests an interactive push instead of retrying in
    /// batch mode.
    pub interactive_push_hint: bool,
    /// Request flag read by the main loop, which owns the terminal and is
    /// the only place that can suspend the TUI to run the interactive push.
    pub wants_interactive_push: bool,
    diff_cache: Option<(String, DiffMode, std::result::Result<String, String>)>,
    push_pending: Option<std::sync::mpsc::Receiver<std::result::Result<(), String>>>,
    spinner_frame: usize,
}

const SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

impl App {
    pub fn new(repo_root: PathBuf) -> Result<Self> {
        let store = ChangelistStore::load(&repo_root)?;
        let shelf = ShelfStore::load(&repo_root)?;
        let mut app = App {
            repo_root,
            store,
            shelf,
            status: Vec::new(),
            focused_pane: Pane::Tree,
            tree_rows: Vec::new(),
            tree_cursor: 0,
            collapsed: HashSet::new(),
            tree_hscroll: 0,
            status_message: None,
            should_quit: false,
            popup: None,
            diff_mode: DiffMode::WorkTree,
            show_help: false,
            diff_scroll: 0,
            diff_hscroll: 0,
            visual_anchor: None,
            interactive_push_hint: false,
            wants_interactive_push: false,
            diff_cache: None,
            push_pending: None,
            spinner_frame: 0,
        };
        app.refresh()?;
        Ok(app)
    }

    pub fn refresh(&mut self) -> Result<()> {
        let entries = git_status(&self.repo_root).map_err(AppError::GitCommand)?;
        let dirty = self.store.reconcile(&entries);
        self.status = entries;
        if dirty {
            self.store.save(&self.repo_root)?;
        }
        self.rebuild_tree_rows();
        self.clamp_selection();
        self.diff_scroll = 0;
        self.diff_hscroll = 0;
        self.diff_cache = None;
        Ok(())
    }

    fn rebuild_tree_rows(&mut self) {
        self.tree_rows.clear();
        for (cl_idx, cl) in self.store.changelists.iter().enumerate() {
            self.tree_rows.push(TreeRow::Header(cl_idx));
            if !self.collapsed.contains(&cl_idx) {
                let count = self.store.files_in(&cl.id).len();
                for file_idx in 0..count {
                    self.tree_rows.push(TreeRow::File(cl_idx, file_idx));
                }
            }
        }
    }

    pub fn cursor_on_header(&self) -> bool {
        matches!(self.tree_rows.get(self.tree_cursor), Some(TreeRow::Header(_)))
    }

    pub fn cursor_on_file(&self) -> bool {
        matches!(self.tree_rows.get(self.tree_cursor), Some(TreeRow::File(_, _)))
    }

    /// The changelist the cursor is currently on (either its header, or the
    /// parent of the file row it's on).
    pub fn focused_changelist_id(&self) -> Option<ChangelistId> {
        let cl_idx = match self.tree_rows.get(self.tree_cursor)? {
            TreeRow::Header(i) | TreeRow::File(i, _) => *i,
        };
        self.store.changelists.get(cl_idx).map(|c| c.id.clone())
    }

    /// The file path the cursor is currently on, or `None` if on a header.
    pub fn focused_file_path(&self) -> Option<String> {
        let (cl_idx, file_idx) = match self.tree_rows.get(self.tree_cursor)? {
            TreeRow::File(ci, fi) => (*ci, *fi),
            TreeRow::Header(_) => return None,
        };
        let cl = self.store.changelists.get(cl_idx)?;
        self.store.files_in(&cl.id).get(file_idx).map(|s| s.to_string())
    }

    fn clamp_selection(&mut self) {
        let len = self.tree_rows.len();
        if len == 0 {
            self.tree_cursor = 0;
            self.visual_anchor = None;
        } else {
            if self.tree_cursor >= len {
                self.tree_cursor = len - 1;
            }
            if let Some(anchor) = self.visual_anchor {
                if anchor >= len {
                    self.visual_anchor = Some(len - 1);
                }
            }
        }
    }

    pub fn move_selection(&mut self, delta: i32) {
        match self.focused_pane {
            Pane::Tree => {
                let len = self.tree_rows.len();
                if len > 0 {
                    let anchor_cl = self.visual_anchor.and_then(|a| {
                        match self.tree_rows.get(a) {
                            Some(TreeRow::File(ci, _)) => Some(*ci),
                            _ => None,
                        }
                    });
                    self.tree_cursor = clamp_index(self.tree_cursor, delta, len);
                    // Exit visual mode if the cursor crosses into a different changelist
                    if let Some(acl) = anchor_cl {
                        let cursor_cl = match self.tree_rows.get(self.tree_cursor) {
                            Some(TreeRow::File(ci, _)) => Some(*ci),
                            Some(TreeRow::Header(ci)) => Some(*ci),
                            None => None,
                        };
                        if cursor_cl != Some(acl) {
                            self.visual_anchor = None;
                        }
                    }
                    self.diff_scroll = 0;
                    self.diff_hscroll = 0;
                }
            }
            Pane::Diff => self.scroll_diff(delta),
        }
    }

    pub fn scroll_horizontal(&mut self, delta: i32) {
        match self.focused_pane {
            Pane::Tree => {
                let max = self.max_tree_hscroll() as i32;
                self.tree_hscroll = (self.tree_hscroll as i32 + delta).clamp(0, max) as u16;
            }
            Pane::Diff => {
                let max = self.max_diff_hscroll() as i32;
                self.diff_hscroll = (self.diff_hscroll as i32 + delta).clamp(0, max) as u16;
            }
        }
    }

    fn max_tree_hscroll(&self) -> u16 {
        self.tree_rows
            .iter()
            .map(|row| match row {
                TreeRow::Header(ci) => {
                    let cl = &self.store.changelists[*ci];
                    let count = self.store.files_in(&cl.id).len();
                    // "▼ * name (count)" or "▶   name (count)"
                    4 + cl.name.chars().count() + 3 + count.to_string().len()
                }
                TreeRow::File(ci, fi) => {
                    let cl = &self.store.changelists[*ci];
                    let files = self.store.files_in(&cl.id);
                    let path = files.get(*fi).copied().unwrap_or("");
                    // "  [MW] filename  path"
                    2 + 4 + 1 + path.chars().count() + 2 + path.chars().count()
                }
            })
            .max()
            .unwrap_or(0)
            .saturating_sub(1) as u16
    }

    fn max_diff_hscroll(&mut self) -> u16 {
        match self.selected_file_diff() {
            Some(Ok(text)) => text
                .lines()
                .map(|l| l.chars().count())
                .max()
                .unwrap_or(0)
                .saturating_sub(1) as u16,
            _ => 0,
        }
    }

    pub fn cycle_pane(&mut self) {
        if self.focused_pane == Pane::Tree {
            self.visual_anchor = None;
        }
        self.focused_pane = match self.focused_pane {
            Pane::Tree => Pane::Diff,
            Pane::Diff => Pane::Tree,
        };
    }

    pub fn cycle_pane_back(&mut self) {
        if self.focused_pane == Pane::Tree {
            self.visual_anchor = None;
        }
        self.focused_pane = match self.focused_pane {
            Pane::Tree => Pane::Diff,
            Pane::Diff => Pane::Tree,
        };
    }

    pub fn toggle_visual_mode(&mut self) {
        if !self.cursor_on_file() {
            return;
        }
        self.visual_anchor = match self.visual_anchor {
            Some(_) => None,
            None => Some(self.tree_cursor),
        };
    }

    pub fn exit_visual_mode(&mut self) {
        self.visual_anchor = None;
    }

    /// Returns the set of `tree_rows` indices that are File rows within the
    /// visual selection range, restricted to the same changelist as the anchor.
    pub fn visual_file_rows(&self) -> Option<std::collections::HashSet<usize>> {
        let anchor = self.visual_anchor?;
        let anchor_cl = match self.tree_rows.get(anchor)? {
            TreeRow::File(ci, _) => *ci,
            _ => return None,
        };
        let lo = anchor.min(self.tree_cursor);
        let hi = anchor.max(self.tree_cursor);
        let set = (lo..=hi)
            .filter(|&i| {
                matches!(self.tree_rows.get(i), Some(TreeRow::File(ci, _)) if *ci == anchor_cl)
            })
            .collect();
        Some(set)
    }

    pub fn selected_file_path(&self) -> Option<String> {
        self.focused_file_path()
    }

    /// All file paths targeted by the next action: the single selected file
    /// normally, or every file in the visual-mode range when active.
    pub fn selected_file_paths(&self) -> Vec<String> {
        if let Some(visual_rows) = self.visual_file_rows() {
            let mut paths: Vec<String> = visual_rows
                .into_iter()
                .filter_map(|i| {
                    let (ci, fi) = match self.tree_rows.get(i)? {
                        TreeRow::File(ci, fi) => (*ci, *fi),
                        _ => return None,
                    };
                    let cl = self.store.changelists.get(ci)?;
                    self.store.files_in(&cl.id).get(fi).map(|s| s.to_string())
                })
                .collect();
            paths.sort();
            paths
        } else {
            self.focused_file_path().into_iter().collect()
        }
    }

    pub fn status_entry_for(&self, path: &str) -> Option<&StatusEntry> {
        self.status.iter().find(|e| e.path == path)
    }

    pub fn toggle_diff_mode(&mut self) {
        self.diff_mode = match self.diff_mode {
            DiffMode::WorkTree => DiffMode::Staged,
            DiffMode::Staged => DiffMode::WorkTree,
        };
    }

    pub fn selected_file_diff(&mut self) -> Option<std::result::Result<String, String>> {
        let path = self.selected_file_path()?;
        if let Some((cached_path, cached_mode, cached_result)) = &self.diff_cache {
            if *cached_path == path && *cached_mode == self.diff_mode {
                return Some(cached_result.clone());
            }
        }
        let result = match self.diff_mode {
            DiffMode::WorkTree => diff_worktree(&self.repo_root, &path),
            DiffMode::Staged => diff_staged(&self.repo_root, &path),
        }
        .map_err(|e| e.to_string());
        self.diff_cache = Some((path, self.diff_mode, result.clone()));
        Some(result)
    }

    fn max_diff_scroll(&mut self) -> u16 {
        match self.selected_file_diff() {
            Some(Ok(text)) => text.lines().count().saturating_sub(1) as u16,
            _ => 0,
        }
    }

    pub fn scroll_diff(&mut self, delta: i32) {
        let max = self.max_diff_scroll() as i32;
        let new = (self.diff_scroll as i32 + delta).clamp(0, max);
        self.diff_scroll = new as u16;
    }

    pub fn toggle_stage_paths(&mut self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        for path in paths {
            let is_staged = self
                .status_entry_for(path)
                .map(|e| e.staged != ChangeKind::Unmodified)
                .unwrap_or(false);
            if is_staged {
                git_reset_path(&self.repo_root, path).map_err(AppError::GitCommand)?;
            } else {
                git_add(&self.repo_root, &[path.as_str()]).map_err(AppError::GitCommand)?;
            }
        }
        self.refresh()
    }

    pub fn start_push(&mut self) {
        if self.push_pending.is_some() {
            return;
        }
        self.interactive_push_hint = false;
        let (tx, rx) = std::sync::mpsc::channel();
        let repo_root = self.repo_root.clone();
        std::thread::spawn(move || {
            let result = git_push(&repo_root).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.push_pending = Some(rx);
        self.spinner_frame = 0;
    }

    pub fn poll_push(&mut self) {
        let Some(rx) = &self.push_pending else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(())) => {
                self.status_message = Some("pushed".to_string());
                self.push_pending = None;
            }
            Ok(Err(e)) => {
                // git/ssh stderr is multi-line; the status bar shows a
                // single line, so flatten it or only "push failed:" and a
                // blank remainder would be visible.
                let flat = e.split_whitespace().collect::<Vec<_>>().join(" ");
                if push_error_needs_terminal(&flat) {
                    self.interactive_push_hint = true;
                    self.status_message = Some(format!(
                        "push needs terminal input — press Shift+P again to push interactively ({flat})"
                    ));
                } else {
                    self.status_message = Some(format!("push failed: {flat}"));
                }
                self.push_pending = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.status_message = Some("push failed: worker thread died".to_string());
                self.push_pending = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                let frame = SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()];
                self.status_message = Some(format!("{frame} pushing..."));
                self.spinner_frame = self.spinner_frame.wrapping_add(1);
            }
        }
    }
}

fn clamp_index(current: usize, delta: i32, len: usize) -> usize {
    let new = current as i32 + delta;
    new.clamp(0, len as i32 - 1) as usize
}

/// True when a push failure is prompt-shaped — the kind that a rerun with
/// a real terminal (interactive push) could resolve by letting git or ssh
/// ask the user something, rather than a genuine push error like a
/// non-fast-forward rejection.
fn push_error_needs_terminal(message: &str) -> bool {
    let m = message.to_lowercase();
    [
        // git with GIT_TERMINAL_PROMPT=0 and no cached HTTPS credentials
        "could not read username",
        "could not read password",
        "terminal prompts disabled",
        // ssh with BatchMode=yes
        "host key verification failed",
        "permission denied",
        "authentication failed",
    ]
    .iter()
    .any(|marker| m.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::push_error_needs_terminal;

    #[test]
    fn prompt_shaped_push_errors_are_detected() {
        assert!(push_error_needs_terminal(
            "git exited with 128: fatal: could not read Username for 'https://x': terminal prompts disabled"
        ));
        assert!(push_error_needs_terminal(
            "git exited with 128: Host key verification failed. fatal: Could not read from remote repository."
        ));
        assert!(push_error_needs_terminal(
            "git exited with 128: git@host: Permission denied (publickey,password)."
        ));
    }

    #[test]
    fn genuine_push_errors_are_not_prompt_shaped() {
        assert!(!push_error_needs_terminal(
            "git exited with 1: error: failed to push some refs to 'origin' (non-fast-forward)"
        ));
        assert!(!push_error_needs_terminal(
            "git exited with 128: fatal: The current branch main has no upstream branch."
        ));
    }
}
