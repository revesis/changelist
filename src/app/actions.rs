use crate::app::commit::{commit_changelist, CommitOutcome};
use crate::error::Result;
use crate::model::changelist::{new_changelist_id, Changelist, ChangelistId, DEFAULT_CHANGELIST_ID};

use super::{App, Pane};

#[derive(Debug, Clone)]
pub enum Popup {
    NewChangelist {
        buffer: String,
    },
    Rename {
        id: ChangelistId,
        buffer: String,
    },
    MoveFile {
        paths: Vec<String>,
        selected: usize,
    },
    ConfirmDelete {
        id: ChangelistId,
    },
    CommitMessage {
        id: ChangelistId,
        buffer: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum Action {
    CyclePane,
    CyclePaneBack,
    MoveSelection(i32),
    ScrollHorizontal(i32),
    EnterFilesPane,
    Refresh,
    Quit,
    OpenNewChangelist,
    OpenRename,
    OpenMove,
    OpenConfirmDelete,
    OpenCommit,
    SetActiveSelected,
    InputChar(char),
    Backspace,
    Confirm,
    Cancel,
    ToggleDiffMode,
    ToggleStage,
    ToggleHelp,
    ToggleVisualMode,
    ExitVisualMode,
}

impl App {
    pub fn dispatch(&mut self, action: Action) -> Result<()> {
        // If a popup is open, input-affecting actions are routed to it first;
        // navigation/quit/etc. are suppressed while a popup has focus.
        if self.popup.is_some() {
            return self.dispatch_popup(action);
        }
        if self.show_help {
            // Any key closes the help overlay.
            self.show_help = false;
            return Ok(());
        }

        match action {
            Action::CyclePane => self.cycle_pane(),
            Action::CyclePaneBack => self.cycle_pane_back(),
            Action::MoveSelection(delta) => self.move_selection(delta),
            Action::ScrollHorizontal(delta) => self.scroll_horizontal(delta),
            Action::EnterFilesPane => {
                if self.focused_pane == Pane::Changelists {
                    self.focused_pane = Pane::Files;
                }
            }
            Action::Refresh => {
                if let Err(e) = self.refresh() {
                    self.status_message = Some(format!("refresh failed: {e}"));
                } else {
                    self.status_message = None;
                }
            }
            Action::Quit => self.should_quit = true,
            Action::OpenNewChangelist => {
                self.popup = Some(Popup::NewChangelist {
                    buffer: String::new(),
                });
            }
            Action::OpenRename => {
                if self.focused_pane == Pane::Changelists {
                    if let Some(id) = self.selected_changelist_id() {
                        let name = self
                            .store
                            .changelist_by_id(&id)
                            .map(|c| c.name.clone())
                            .unwrap_or_default();
                        self.popup = Some(Popup::Rename { id, buffer: name });
                    }
                }
            }
            Action::OpenMove => {
                if self.focused_pane == Pane::Files {
                    let paths = self.selected_file_paths();
                    self.exit_visual_mode();
                    if !paths.is_empty() {
                        self.popup = Some(Popup::MoveFile { paths, selected: 0 });
                    }
                }
            }
            Action::OpenConfirmDelete => {
                if self.focused_pane == Pane::Changelists {
                    if let Some(id) = self.selected_changelist_id() {
                        if id != DEFAULT_CHANGELIST_ID {
                            self.popup = Some(Popup::ConfirmDelete { id });
                        } else {
                            self.status_message =
                                Some("cannot delete the Default changelist".to_string());
                        }
                    }
                }
            }
            Action::OpenCommit => {
                if self.focused_pane == Pane::Changelists {
                    if let Some(id) = self.selected_changelist_id() {
                        if self.store.files_in(&id).is_empty() {
                            self.status_message =
                                Some("nothing to commit: changelist is empty".to_string());
                        } else {
                            self.popup = Some(Popup::CommitMessage {
                                id,
                                buffer: String::new(),
                            });
                        }
                    }
                }
            }
            Action::SetActiveSelected => {
                if self.focused_pane == Pane::Changelists {
                    if let Some(id) = self.selected_changelist_id() {
                        self.store.active_changelist = id;
                        self.store.save(&self.repo_root)?;
                    }
                }
            }
            Action::ToggleHelp => self.show_help = true,
            Action::ToggleDiffMode => self.toggle_diff_mode(),
            Action::ToggleVisualMode => self.toggle_visual_mode(),
            Action::ExitVisualMode => self.exit_visual_mode(),
            Action::ToggleStage => {
                let paths = self.selected_file_paths();
                self.exit_visual_mode();
                if let Err(e) = self.toggle_stage_paths(&paths) {
                    self.status_message = Some(format!("stage/unstage failed: {e}"));
                }
            }
            Action::InputChar(_) | Action::Backspace | Action::Confirm | Action::Cancel => {}
        }
        Ok(())
    }

    fn dispatch_popup(&mut self, action: Action) -> Result<()> {
        let Some(popup) = self.popup.clone() else {
            return Ok(());
        };
        match action {
            Action::InputChar(c) => {
                if let Some(buf) = self.popup_buffer_mut() {
                    buf.push(c);
                }
            }
            Action::Backspace => {
                if let Some(buf) = self.popup_buffer_mut() {
                    buf.pop();
                }
            }
            Action::MoveSelection(delta) => {
                let len = self.store.changelists.len();
                if let Some(Popup::MoveFile { selected, .. }) = self.popup.as_mut() {
                    if len > 0 {
                        let new = (*selected as i32 + delta).clamp(0, len as i32 - 1);
                        *selected = new as usize;
                    }
                }
            }
            Action::Cancel => {
                self.popup = None;
            }
            Action::Confirm => {
                self.confirm_popup(popup)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn popup_buffer_mut(&mut self) -> Option<&mut String> {
        match self.popup.as_mut()? {
            Popup::NewChangelist { buffer } => Some(buffer),
            Popup::Rename { buffer, .. } => Some(buffer),
            Popup::CommitMessage { buffer, .. } => Some(buffer),
            _ => None,
        }
    }

    fn confirm_popup(&mut self, popup: Popup) -> Result<()> {
        match popup {
            Popup::NewChangelist { buffer } => {
                let name = buffer.trim().to_string();
                if !name.is_empty() {
                    let id = new_changelist_id();
                    self.store.changelists.push(Changelist::new(id, name, None));
                    self.store.save(&self.repo_root)?;
                }
                self.popup = None;
            }
            Popup::Rename { id, buffer } => {
                let name = buffer.trim().to_string();
                if !name.is_empty() {
                    if let Some(cl) = self.store.changelist_by_id_mut(&id) {
                        cl.name = name;
                    }
                    self.store.save(&self.repo_root)?;
                }
                self.popup = None;
            }
            Popup::MoveFile { paths, selected } => {
                if let Some(target) = self.store.changelists.get(selected) {
                    let target_id = target.id.clone();
                    for path in paths {
                        self.store.files.insert(path, target_id.clone());
                    }
                    self.store.save(&self.repo_root)?;
                }
                self.popup = None;
            }
            Popup::ConfirmDelete { id } => {
                for cl_id in self.store.files.values_mut() {
                    if *cl_id == id {
                        *cl_id = DEFAULT_CHANGELIST_ID.to_string();
                    }
                }
                self.store.changelists.retain(|c| c.id != id);
                if self.store.active_changelist == id {
                    self.store.active_changelist = DEFAULT_CHANGELIST_ID.to_string();
                }
                self.store.save(&self.repo_root)?;
                self.popup = None;
                self.clamp_selection();
            }
            Popup::CommitMessage { id, buffer } => {
                let message = buffer.trim().to_string();
                if message.is_empty() {
                    self.status_message = Some("commit message cannot be empty".to_string());
                    return Ok(());
                }
                match commit_changelist(&self.repo_root, &mut self.store, &id, &message) {
                    Ok(CommitOutcome::Committed { paths }) => {
                        self.status_message =
                            Some(format!("committed {} file(s)", paths.len()));
                        self.popup = None;
                        self.refresh()?;
                    }
                    Ok(CommitOutcome::EmptyChangelist) => {
                        self.status_message =
                            Some("nothing to commit: changelist is empty".to_string());
                        self.popup = None;
                    }
                    Err(e) => {
                        self.status_message = Some(format!("commit failed: {e}"));
                    }
                }
            }
        }
        Ok(())
    }
}
