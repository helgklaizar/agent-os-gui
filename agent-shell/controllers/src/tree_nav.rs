//! Keyboard navigation of the tree (S3-09): arrows move across visible rows, Left and Right collapse and expand folders.

use crate::tree_model::TreeModel;
use crate::tree_types::NodeId;
use crate::tree_view::NavKey;

/// Key result.
pub enum Nav {
    Focus(Option<NodeId>),
    Toggle(i64),
    Open(i64),
}

/// `nodes` - visible rows top to bottom, `focus` - the focused row.
pub fn navigate(model: &TreeModel, nodes: &[NodeId], focus: Option<NodeId>, key: NavKey) -> Nav {
    let at = focus.and_then(|f| nodes.iter().position(|n| *n == f));
    let to = |i: usize| nodes.get(i).copied();
    match key {
        NavKey::Down => Nav::Focus(at.map_or(to(0), |i| to((i + 1).min(nodes.len() - 1)))),
        NavKey::Up => Nav::Focus(at.map_or(nodes.last().copied(), |i| to(i.saturating_sub(1)))),
        NavKey::Home => Nav::Focus(to(0)),
        NavKey::End => Nav::Focus(nodes.last().copied()),
        NavKey::Enter => match focus {
            Some(NodeId::Session(id)) => Nav::Open(id),
            Some(NodeId::Folder(id)) => Nav::Toggle(id),
            None => Nav::Focus(None),
        },
        NavKey::Right => match focus {
            Some(NodeId::Folder(id)) if !model.is_expanded(id) => Nav::Toggle(id),
            Some(NodeId::Folder(_)) => {
                let child = at
                    .and_then(|i| to(i + 1))
                    .filter(|n| model.parent_node(*n) == focus);
                Nav::Focus(child.or(focus))
            }
            other => Nav::Focus(other),
        },
        NavKey::Left => match focus {
            Some(NodeId::Folder(id)) if model.is_expanded(id) => Nav::Toggle(id),
            Some(node) => Nav::Focus(model.parent_node(node).or(focus)),
            None => Nav::Focus(None),
        },
    }
}
