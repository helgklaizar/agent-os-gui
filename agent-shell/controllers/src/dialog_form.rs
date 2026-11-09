//! Tree dialog forms: field state, view and the confirm command.

use crate::controller::CoreCommand;
use crate::controllers::dialog::{FolderOption, FormView, PersonaOption};
use crate::form_types::FormRequest;
use crate::i18n::{tr, tr_args};
use crate::tree_model::INDENT_STEP;
use crate::tree_types::NodeId;
use crate::types::DialogKind;

pub struct FormState {
    pub request: FormRequest,
    pub name: String,
    pub folder: Option<i64>,
    pub persona: Option<i64>,
    /// An attempt was made to submit an empty name.
    pub rejected: bool,
}

pub fn kind_of(form: &FormRequest) -> DialogKind {
    match form {
        FormRequest::NewSession { .. } => DialogKind::NewSession,
        FormRequest::NewFolder { .. } => DialogKind::NewFolder,
        FormRequest::NewThread { .. } => DialogKind::NewThread,
        FormRequest::MoveSession { .. } => DialogKind::MoveSession,
        FormRequest::Rename { .. } => DialogKind::Rename,
        FormRequest::DeleteSession { .. } => DialogKind::ConfirmDeleteSession,
        FormRequest::DeleteFolder { .. } => DialogKind::ConfirmDeleteFolder,
    }
}

/// English form of "thread" by count: 1 thread, otherwise threads.
fn threads_line(n: usize) -> String {
    let count = n.to_string();
    let args = [("count", count.as_str())];
    if n == 1 {
        tr_args("delete-session-threads-one", &args)
    } else {
        tr_args("delete-session-threads-many", &args)
    }
}

impl FormState {
    pub fn new(request: FormRequest) -> FormState {
        let (name, folder) = match &request {
            FormRequest::NewSession { folder, .. } => (String::new(), *folder),
            FormRequest::NewFolder { parent, .. } => (String::new(), *parent),
            FormRequest::MoveSession { folder, .. } => (String::new(), *folder),
            FormRequest::Rename { name, .. } => (name.clone(), None),
            _ => (String::new(), None),
        };
        FormState {
            request,
            name,
            folder,
            persona: None,
            rejected: false,
        }
    }

    fn has_name(&self) -> bool {
        matches!(
            self.request,
            FormRequest::NewSession { .. }
                | FormRequest::NewFolder { .. }
                | FormRequest::NewThread { .. }
                | FormRequest::Rename { .. }
        )
    }

    fn can_submit(&self) -> bool {
        !self.has_name() || !self.name.trim().is_empty()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
        if self.can_submit() {
            self.rejected = false;
        }
    }

    /// Confirm command; an empty name is rejected (`None`, the form stays with a hint).
    pub fn submit(&mut self) -> Option<CoreCommand> {
        if !self.can_submit() {
            self.rejected = true;
            return None;
        }
        let name = self.name.trim().to_string();
        Some(match &self.request {
            FormRequest::NewSession { .. } => CoreCommand::CreateSession {
                name,
                folder_id: self.folder,
                persona_id: self.persona,
            },
            FormRequest::NewFolder { .. } => CoreCommand::CreateFolder {
                name,
                parent_id: self.folder,
                persona_id: self.persona,
            },
            FormRequest::NewThread { parent_id, .. } => CoreCommand::CreateThread {
                parent_id: *parent_id,
                name,
            },
            FormRequest::MoveSession { id, .. } => CoreCommand::MoveSession {
                id: *id,
                folder_id: self.folder,
            },
            FormRequest::Rename {
                node: NodeId::Session(id),
                ..
            } => CoreCommand::RenameSession { id: *id, name },
            FormRequest::Rename {
                node: NodeId::Folder(id),
                ..
            } => CoreCommand::RenameFolder { id: *id, name },
            FormRequest::DeleteSession { id, .. } => CoreCommand::DeleteSession { id: *id },
            FormRequest::DeleteFolder { id, .. } => CoreCommand::DeleteFolder { id: *id },
        })
    }

    fn folders(&self) -> Option<Vec<FolderOption>> {
        let list = match &self.request {
            FormRequest::NewSession { folders, .. }
            | FormRequest::NewFolder { folders, .. }
            | FormRequest::MoveSession { folders, .. } => folders,
            _ => return None,
        };
        Some(
            list.iter()
                .map(|c| FolderOption {
                    id: c.id,
                    label: c.name.clone(),
                    indent: c.depth as u32 * INDENT_STEP,
                    selected: c.id == self.folder,
                })
                .collect(),
        )
    }

    fn personas(&self) -> Option<Vec<PersonaOption>> {
        let (FormRequest::NewSession { personas, .. } | FormRequest::NewFolder { personas, .. }) =
            &self.request
        else {
            return None;
        };
        Some(
            personas
                .iter()
                .map(|p| PersonaOption {
                    id: p.id,
                    label: p.name.clone(),
                    color: p.color.clone(),
                    selected: p.id == self.persona,
                })
                .collect(),
        )
    }

    pub fn view(&self) -> FormView {
        let (title, submit_label) = match &self.request {
            FormRequest::NewSession { .. } => (tr("dialog-new-session-title"), tr("dialog-create")),
            FormRequest::NewFolder { .. } => (tr("dialog-new-folder-title"), tr("dialog-create")),
            FormRequest::NewThread { .. } => (tr("dialog-new-thread-title"), tr("dialog-create")),
            FormRequest::MoveSession { .. } => (tr("dialog-move-title"), tr("dialog-move")),
            FormRequest::Rename { .. } => (tr("dialog-rename-title"), tr("dialog-save")),
            FormRequest::DeleteSession { .. } => (tr("delete-session-title"), tr("dialog-delete")),
            FormRequest::DeleteFolder { .. } => (tr("delete-folder-title"), tr("dialog-delete")),
        };
        let lines = match &self.request {
            FormRequest::DeleteSession { name, threads, .. } => {
                let mut lines = vec![tr_args("delete-session-line", &[("name", name)])];
                if *threads > 0 {
                    lines.push(threads_line(*threads));
                }
                lines
            }
            FormRequest::DeleteFolder { name, .. } => vec![
                tr_args("delete-folder-line", &[("name", name)]),
                tr("delete-folder-lines"),
            ],
            _ => Vec::new(),
        };
        FormView {
            title,
            has_name: self.has_name(),
            name: self.name.clone(),
            name_error: self.rejected.then(|| tr("dialog-name-required")),
            folders: self.folders(),
            personas: self.personas(),
            submit_label,
            can_submit: self.can_submit(),
            lines,
        }
    }
}
