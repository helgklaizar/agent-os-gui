//! Session tree form requests (S3-04): what to show in the dialog and where to return focus. Built from the model, the dialog knows nothing about the tree.

use crate::controllers::session_tree::TreeRequest;
use crate::form_types::FormRequest;
use crate::tree_model::{TreeModel, shorten};
use crate::tree_types::NodeId;
use crate::types::{FocusTarget, TreeButton};

/// Form by call site and the element focus returns to; `None` - target not found.
pub fn form_for(m: &TreeModel, request: TreeRequest) -> Option<(FormRequest, FocusTarget)> {
    let node_focus = |n| FocusTarget::Node(n);
    Some(match request {
        TreeRequest::NewSession { folder } => (
            FormRequest::NewSession {
                folder,
                folders: m.folder_choices(),
                personas: m.persona_choices(),
            },
            FocusTarget::TreeButton(TreeButton::NewSession),
        ),
        TreeRequest::NewSessionInFolder { folder } => (
            FormRequest::NewSession {
                folder: Some(m.folder(folder)?.id),
                folders: m.folder_choices(),
                personas: m.persona_choices(),
            },
            node_focus(NodeId::Folder(folder)),
        ),
        TreeRequest::NewFolder { parent } => (
            FormRequest::NewFolder {
                parent,
                folders: m.folder_choices(),
                personas: m.persona_choices(),
            },
            FocusTarget::TreeButton(TreeButton::NewFolder),
        ),
        TreeRequest::NewSubfolder { parent } => (
            FormRequest::NewFolder {
                parent: Some(m.folder(parent)?.id),
                folders: m.folder_choices(),
                personas: m.persona_choices(),
            },
            node_focus(NodeId::Folder(parent)),
        ),
        TreeRequest::NewThread { parent } => {
            let p = m.session(parent).filter(|p| p.parent_id.is_none())?;
            (
                FormRequest::NewThread {
                    parent_id: p.id,
                    parent_name: shorten(&p.name),
                },
                node_focus(NodeId::Session(parent)),
            )
        }
        TreeRequest::Move { session } => {
            let s = m.session(session).filter(|s| s.parent_id.is_none())?;
            (
                FormRequest::MoveSession {
                    id: s.id,
                    name: shorten(&s.name),
                    folder: s.folder_id,
                    folders: m.folder_choices(),
                },
                node_focus(NodeId::Session(session)),
            )
        }
        TreeRequest::Rename { node } => {
            let name = match node {
                NodeId::Folder(id) => m.folder(id)?.name.clone(),
                NodeId::Session(id) => m.session(id)?.name.clone(),
            };
            (FormRequest::Rename { node, name }, node_focus(node))
        }
        TreeRequest::Delete {
            node: NodeId::Folder(id),
        } => (
            FormRequest::DeleteFolder {
                id,
                name: shorten(&m.folder(id)?.name),
            },
            node_focus(NodeId::Folder(id)),
        ),
        TreeRequest::Delete {
            node: NodeId::Session(id),
        } => {
            let s = m.session(id)?;
            (
                FormRequest::DeleteSession {
                    id,
                    name: shorten(&s.name),
                    threads: m.threads_of(id),
                },
                node_focus(NodeId::Session(id)),
            )
        }
    })
}
