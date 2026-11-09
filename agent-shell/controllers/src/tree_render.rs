//! Collapsed column rows and strip (S3-03, S3-08): the model plus selection and focus turn into a view.

use crate::i18n::{tr, tr_args};
use crate::tree_model::{INDENT_STEP, TreeModel, shorten};
use crate::tree_types::NodeId;
use crate::tree_view::{RowKind, Strip, StripItem, TreeRow};

pub fn badge_text(n: i64) -> Option<String> {
    match n {
        i64::MIN..=0 => None,
        1..=99 => Some(n.to_string()),
        _ => Some("99+".to_string()),
    }
}

pub struct Render<'a> {
    pub model: &'a TreeModel,
    pub selected: Option<i64>,
    pub focus: Option<NodeId>,
}

impl Render<'_> {
    pub fn row(&self, node: NodeId, depth: usize) -> Option<TreeRow> {
        let m = self.model;
        let (name, color, unread, kind, expanded, sel) = match node {
            NodeId::Folder(id) => {
                let f = m.folder(id)?;
                (
                    &f.name,
                    m.color_of(f.persona_id),
                    0,
                    RowKind::Folder,
                    Some(m.is_expanded(id)),
                    false,
                )
            }
            NodeId::Session(id) => {
                let s = m.session(id)?;
                let kind = if m.parent_of(s).is_some() {
                    RowKind::Thread
                } else {
                    RowKind::Session
                };
                (
                    &s.name,
                    m.color_of(m.effective_persona(s)),
                    s.unread.max(0),
                    kind,
                    None,
                    self.selected == Some(id),
                )
            }
        };
        let kind_name = match kind {
            RowKind::Folder => tr("tree-a11y-folder"),
            RowKind::Session => tr("tree-a11y-session"),
            RowKind::Thread => tr("tree-a11y-thread"),
        };
        let mut parts = vec![format!("{kind_name} {name}")];
        match expanded {
            Some(true) => parts.push(tr("tree-a11y-expanded")),
            Some(false) => parts.push(tr("tree-a11y-collapsed")),
            None => {}
        }
        if sel {
            parts.push(tr("tree-a11y-selected"));
        }
        if unread > 0 {
            parts.push(tr_args(
                "tree-a11y-unread",
                &[("count", &unread.to_string())],
            ));
        }
        Some(TreeRow {
            node,
            kind,
            depth,
            indent: depth as u32 * INDENT_STEP,
            name: shorten(name),
            persona_color: color,
            unread,
            badge: badge_text(unread),
            selected: sel,
            focused: self.focus == Some(node),
            expanded,
            a11y_name: parts.join(", "),
        })
    }

    pub fn strip(&self) -> Strip {
        let items: Vec<StripItem> = self
            .model
            .visible(false)
            .into_iter()
            .filter_map(|(n, _)| match n {
                NodeId::Session(id) => self.model.session(id),
                NodeId::Folder(_) => None,
            })
            .filter(|s| s.unread > 0)
            .map(|s| StripItem {
                session_id: s.id,
                initial: s
                    .name
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().collect())
                    .unwrap_or_default(),
                unread: s.unread,
                badge: badge_text(s.unread).unwrap_or_default(),
                label: format!(
                    "{}, {}",
                    shorten(&s.name),
                    tr_args("tree-a11y-unread", &[("count", &s.unread.to_string())])
                ),
            })
            .collect();
        let total: i64 = self.model.sessions.iter().map(|s| s.unread.max(0)).sum();
        Strip {
            total_unread: total,
            badge: badge_text(total),
            items,
            a11y_name: tr_args("tree-strip-a11y", &[("count", &total.to_string())]),
        }
    }
}
