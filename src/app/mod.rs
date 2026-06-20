pub mod actions;
pub mod commit;

use std::path::PathBuf;

use crate::error::{AppError, Result};
use crate::git::diff::{diff_staged, diff_worktree};
use crate::git::index_ops::{add as git_add, push as git_push, reset_path as git_reset_path};
use crate::git::status::{git_status, ChangeKind, StatusEntry};
use crate::model::{ChangelistId, ChangelistStore};

pub use actions::{Action, Popup};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Changelists,
    Files,
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
    pub status: Vec<StatusEntry>,
    pub focused_pane: Pane,
    pub selected_changelist_idx: usize,
    pub selected_file_idx: usize,
    pub status_message: Option<String>,
    pub should_quit: bool,
    pub popup: Option<Popup>,
    pub diff_mode: DiffMode,
    pub show_help: bool,
    pub diff_scroll: u16,
    pub diff_hscroll: u16,
    pub changelist_hscroll: u16,
    pub file_hscroll: u16,
    /// When `Some(idx)`, the Files pane is in visual (batch-select) mode,
    /// anchored at `idx`; the selected range is `[anchor, selected_file_idx]`.
    pub visual_anchor: Option<usize>,
    /// Cache for the current file's diff, keyed on (path, mode) so a `git
    /// diff` subprocess is only spawned when the diff actually needs to
    /// change, not on every redraw (the render loop redraws continuously,
    /// independent of whether anything happened).
    diff_cache: Option<(String, DiffMode, std::result::Result<String, String>)>,
    /// Set while a `git push` runs on a background thread; polled
    /// non-blockingly from the main loop so the spinner animation in the
    /// status bar keeps redrawing instead of the whole TUI freezing for the
    /// duration of the (potentially slow, network-bound) push.
    push_pending: Option<std::sync::mpsc::Receiver<std::result::Result<(), String>>>,
    spinner_frame: usize,
}

const SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

impl App {
    pub fn new(repo_root: PathBuf) -> Result<Self> {
        let store = ChangelistStore::load(&repo_root)?;
        let mut app = App {
            repo_root,
            store,
            status: Vec::new(),
            focused_pane: Pane::Changelists,
            selected_changelist_idx: 0,
            selected_file_idx: 0,
            status_message: None,
            should_quit: false,
            popup: None,
            diff_mode: DiffMode::WorkTree,
            show_help: false,
            diff_scroll: 0,
            diff_hscroll: 0,
            changelist_hscroll: 0,
            file_hscroll: 0,
            visual_anchor: None,
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
        self.clamp_selection();
        self.diff_scroll = 0;
        self.diff_hscroll = 0;
        self.diff_cache = None;
        Ok(())
    }

    pub fn selected_changelist_id(&self) -> Option<ChangelistId> {
        self.store
            .changelists
            .get(self.selected_changelist_idx)
            .map(|c| c.id.clone())
    }

    pub fn files_in_selected_changelist(&self) -> Vec<&str> {
        match self.selected_changelist_id() {
            Some(id) => self.store.files_in(&id),
            None => Vec::new(),
        }
    }

    fn clamp_selection(&mut self) {
        let cl_len = self.store.changelists.len();
        if cl_len == 0 {
            self.selected_changelist_idx = 0;
        } else if self.selected_changelist_idx >= cl_len {
            self.selected_changelist_idx = cl_len - 1;
        }
        let file_len = self.files_in_selected_changelist().len();
        if file_len == 0 {
            self.selected_file_idx = 0;
            self.visual_anchor = None;
        } else {
            if self.selected_file_idx >= file_len {
                self.selected_file_idx = file_len - 1;
            }
            if let Some(anchor) = self.visual_anchor {
                if anchor >= file_len {
                    self.visual_anchor = Some(file_len - 1);
                }
            }
        }
    }

    pub fn move_selection(&mut self, delta: i32) {
        match self.focused_pane {
            Pane::Changelists => {
                let len = self.store.changelists.len();
                if len > 0 {
                    self.selected_changelist_idx =
                        clamp_index(self.selected_changelist_idx, delta, len);
                    self.selected_file_idx = 0;
                    self.visual_anchor = None;
                    self.diff_scroll = 0;
                    self.diff_hscroll = 0;
                }
            }
            Pane::Files => {
                let len = self.files_in_selected_changelist().len();
                if len > 0 {
                    self.selected_file_idx = clamp_index(self.selected_file_idx, delta, len);
                    self.diff_scroll = 0;
                    self.diff_hscroll = 0;
                }
            }
            Pane::Diff => self.scroll_diff(delta),
        }
    }

    /// Scrolls the currently focused pane horizontally (`h`/`l`, Left/Right) —
    /// useful when file paths or diff lines are wider than the pane.
    pub fn scroll_horizontal(&mut self, delta: i32) {
        match self.focused_pane {
            Pane::Changelists => {
                let max = self.max_changelist_hscroll() as i32;
                self.changelist_hscroll =
                    (self.changelist_hscroll as i32 + delta).clamp(0, max) as u16;
            }
            Pane::Files => {
                let max = self.max_file_hscroll() as i32;
                self.file_hscroll = (self.file_hscroll as i32 + delta).clamp(0, max) as u16;
            }
            Pane::Diff => {
                let max = self.max_diff_hscroll() as i32;
                self.diff_hscroll = (self.diff_hscroll as i32 + delta).clamp(0, max) as u16;
            }
        }
    }

    fn max_changelist_hscroll(&self) -> u16 {
        self.store
            .changelists
            .iter()
            .map(|c| c.name.chars().count())
            .max()
            .unwrap_or(0)
            .saturating_sub(1) as u16
    }

    fn max_file_hscroll(&self) -> u16 {
        self.files_in_selected_changelist()
            .iter()
            .map(|p| p.chars().count())
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
        if self.focused_pane == Pane::Files {
            self.visual_anchor = None;
        }
        self.focused_pane = match self.focused_pane {
            Pane::Changelists => Pane::Files,
            Pane::Files => Pane::Diff,
            Pane::Diff => Pane::Changelists,
        };
    }

    pub fn cycle_pane_back(&mut self) {
        if self.focused_pane == Pane::Files {
            self.visual_anchor = None;
        }
        self.focused_pane = match self.focused_pane {
            Pane::Changelists => Pane::Diff,
            Pane::Files => Pane::Changelists,
            Pane::Diff => Pane::Files,
        };
    }

    pub fn toggle_visual_mode(&mut self) {
        if self.focused_pane != Pane::Files {
            return;
        }
        self.visual_anchor = match self.visual_anchor {
            Some(_) => None,
            None => Some(self.selected_file_idx),
        };
    }

    pub fn exit_visual_mode(&mut self) {
        self.visual_anchor = None;
    }

    /// The range of file-pane row indices currently selected for a batch
    /// operation: just the cursor row in normal mode, or `[anchor,
    /// selected_file_idx]` (in either order) while in visual mode.
    pub fn visual_range(&self) -> Option<(usize, usize)> {
        self.visual_anchor
            .map(|a| (a.min(self.selected_file_idx), a.max(self.selected_file_idx)))
    }

    pub fn selected_file_path(&self) -> Option<String> {
        self.files_in_selected_changelist()
            .get(self.selected_file_idx)
            .map(|s| s.to_string())
    }

    /// All file paths targeted by the next action: the single selected file
    /// normally, or every file in the visual-mode range when active.
    pub fn selected_file_paths(&self) -> Vec<String> {
        let files = self.files_in_selected_changelist();
        match self.visual_range() {
            Some((lo, hi)) if !files.is_empty() => {
                let hi = hi.min(files.len() - 1);
                files[lo..=hi].iter().map(|s| s.to_string()).collect()
            }
            _ => self.selected_file_path().into_iter().collect(),
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

    /// Returns the diff for the currently selected file, spawning `git diff`
    /// only if the (path, mode) pair isn't already cached — the render loop
    /// calls this every frame regardless of whether anything changed, so
    /// without caching this would shell out to git continuously.
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

    /// Toggles stage/unstage for every path in `paths`. An untracked file or
    /// one with no staged changes gets staged; a fully-staged file gets
    /// unstaged. Partially-staged files are left as a working-tree-only
    /// `git add` (simplest, predictable behavior for v1). Each path is
    /// toggled according to its own current state, so a mixed batch selection
    /// stages some and unstages others as appropriate.
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

    /// Kicks off `git push` on a background thread (it's network-bound and
    /// can take a noticeable while) and returns immediately so the render
    /// loop keeps animating a spinner via `poll_push` instead of the whole
    /// TUI freezing for the duration of the push.
    pub fn start_push(&mut self) {
        if self.push_pending.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let repo_root = self.repo_root.clone();
        std::thread::spawn(move || {
            let result = git_push(&repo_root).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.push_pending = Some(rx);
        self.spinner_frame = 0;
    }

    /// Non-blocking check for a background push started by `start_push`.
    /// Called every iteration of the main loop so the spinner advances on
    /// every redraw tick (~250ms) regardless of key input, and the result
    /// (success/failure) lands in `status_message` as soon as it's ready.
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
                self.status_message = Some(format!("push failed: {e}"));
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
