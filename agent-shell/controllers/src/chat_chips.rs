//! Chat chips: persona (list from the core) and local folder (file dialog).

use crate::chat_view::{Chips, PersonaChoice};
use crate::controller::{CoreCommand, Effect};
use crate::controllers::chat::ChatController;
use crate::i18n::tr;

impl ChatController {
    pub(crate) fn chips(&self) -> Option<Chips> {
        self.session?;
        let persona = self.persona();
        Some(Chips {
            persona_label: persona.map_or_else(|| tr("chat-chip-persona-none"), |p| p.name.clone()),
            persona_color: persona.map(|p| p.color.clone()),
            path_label: self
                .info
                .path
                .clone()
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| tr("chat-chip-path-none")),
        })
    }

    pub(crate) fn persona_choices(&self) -> Option<Vec<PersonaChoice>> {
        if !self.persona_list_open || self.session.is_none() {
            return None;
        }
        let inherit = PersonaChoice {
            id: None,
            name: tr("chat-persona-inherit"),
            color: None,
            current: self.info.own_persona_id.is_none(),
        };
        let own = self.info.own_persona_id;
        let rest = self.personas.iter().map(|p| PersonaChoice {
            id: Some(p.id),
            name: p.name.clone(),
            color: Some(p.color.clone()),
            current: own == Some(p.id),
        });
        Some(std::iter::once(inherit).chain(rest).collect())
    }

    pub(crate) fn open_persona_list(&mut self) -> Vec<Effect> {
        if self.session.is_none() {
            return Vec::new();
        }
        self.persona_list_open = true;
        vec![Effect::Command(CoreCommand::ListPersonas)]
    }

    pub(crate) fn pick_persona(&mut self, persona_id: Option<i64>) -> Vec<Effect> {
        self.persona_list_open = false;
        match self.session {
            Some(session_id) if persona_id != self.info.own_persona_id => {
                vec![Effect::Command(CoreCommand::SetSessionPersona {
                    session_id,
                    persona_id,
                })]
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn pick_path(&mut self, path: Option<String>) -> Vec<Effect> {
        self.want_path_dialog = false;
        let path = path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
        match (self.session, path) {
            (Some(session_id), Some(path)) if self.info.path.as_deref() != Some(path.as_str()) => {
                vec![Effect::Command(CoreCommand::SetSessionPath {
                    session_id,
                    path,
                })]
            }
            _ => Vec::new(),
        }
    }
}
