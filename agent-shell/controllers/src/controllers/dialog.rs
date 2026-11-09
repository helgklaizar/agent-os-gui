//! `DialogController`: modal dialog stack, Esc, backdrop click, focus return, About.

use crate::time::Instant;

use crate::bus::BusEvent;
use crate::controller::{Controller, CoreCommand, Effect};
use crate::dialog_form::{FormState, kind_of};
use crate::form_types::FormRequest;
use crate::i18n::tr;
use crate::types::{DialogKind, FocusTarget};

#[derive(Debug, Clone, PartialEq)]
pub struct AboutView {
    pub name: String,
    pub version: String,
    pub platform: String,
    pub config_path: String,
}

/// A folder list row in the form: `indent` is the depth indentation in pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderOption {
    pub id: Option<i64>,
    pub label: String,
    pub indent: u32,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaOption {
    pub id: Option<i64>,
    pub label: String,
    pub color: Option<String>,
    pub selected: bool,
}

/// Tree dialog form: create, move, rename, delete confirmation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FormView {
    pub title: String,
    pub has_name: bool,
    pub name: String,
    /// Hint under the field when the name is empty after a submit attempt.
    pub name_error: Option<String>,
    pub folders: Option<Vec<FolderOption>>,
    pub personas: Option<Vec<PersonaOption>>,
    pub submit_label: String,
    /// "Create" and similar are available only with a non-empty name.
    pub can_submit: bool,
    /// Delete confirmation lines: what will be lost.
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DialogView {
    pub form: Option<FormView>,
    /// Bottom to top: the last one is the topmost.
    pub stack: Vec<DialogKind>,
    pub about: AboutView,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DialogMsg {
    Open {
        kind: DialogKind,
        opener: FocusTarget,
    },
    /// Esc closes the top dialog.
    Esc,
    /// A backdrop click closes the top dialog.
    Backdrop,
    Confirm,
    Cancel,
    /// Version from the core's answer; `None` means no data.
    AboutVersion(Option<String>),
    /// Tree form; `opener` receives focus on close.
    OpenForm {
        form: FormRequest,
        opener: FocusTarget,
    },
    SetName(String),
    SetFolder(Option<i64>),
    SetPersona(Option<i64>),
    Submit,
}

#[derive(Default)]
pub struct DialogController {
    stack: Vec<(DialogKind, FocusTarget)>,
    /// Form of the top dialog of the stack if it is a tree form (each level has its own).
    forms: Vec<(usize, FormState)>,
    version: Option<String>,
}

impl DialogController {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DialogController {
    /// Close the top dialog; focus returns to whoever opened it.
    fn close_top(&mut self) -> Vec<Effect> {
        self.forms
            .retain(|(level, _)| *level < self.stack.len().saturating_sub(1));
        match self.stack.pop() {
            Some((_, opener)) => vec![Effect::Publish(BusEvent::FocusRestore(opener))],
            None => Vec::new(),
        }
    }

    fn form_mut(&mut self) -> Option<&mut FormState> {
        let level = self.stack.len().checked_sub(1)?;
        self.forms
            .iter_mut()
            .find(|(l, _)| *l == level)
            .map(|(_, f)| f)
    }

    /// Form confirmation: a command to the core and focus return; an empty name keeps the form open.
    fn submit(&mut self) -> Vec<Effect> {
        let Some(command) = self.form_mut().and_then(FormState::submit) else {
            return Vec::new();
        };
        let mut fx = vec![Effect::Command(command)];
        fx.extend(self.close_top());
        fx
    }
}

impl Controller for DialogController {
    type Msg = DialogMsg;
    type View = DialogView;

    fn update(&mut self, msg: DialogMsg, _now: Instant) -> Vec<Effect> {
        match msg {
            DialogMsg::Open { kind, opener } => {
                if self.stack.iter().any(|(k, _)| *k == kind) {
                    return Vec::new();
                }
                self.stack.push((kind, opener));
                if kind == DialogKind::About {
                    return vec![Effect::Command(CoreCommand::FetchAbout)];
                }
                Vec::new()
            }
            DialogMsg::Esc | DialogMsg::Backdrop | DialogMsg::Cancel => self.close_top(),
            DialogMsg::Confirm | DialogMsg::Submit if self.form_mut().is_some() => self.submit(),
            DialogMsg::Confirm => match self.stack.last() {
                Some((DialogKind::ConfirmLogout, _)) => {
                    self.stack.pop();
                    vec![Effect::Command(CoreCommand::Logout)]
                }
                _ => Vec::new(),
            },
            DialogMsg::AboutVersion(version) => {
                self.version = version;
                Vec::new()
            }
            DialogMsg::OpenForm { form, opener } => {
                self.stack.push((kind_of(&form), opener));
                self.forms
                    .push((self.stack.len() - 1, FormState::new(form)));
                Vec::new()
            }
            DialogMsg::SetName(name) => {
                if let Some(f) = self.form_mut() {
                    f.set_name(name);
                }
                Vec::new()
            }
            DialogMsg::SetFolder(folder) => {
                if let Some(f) = self.form_mut() {
                    f.folder = folder;
                }
                Vec::new()
            }
            DialogMsg::SetPersona(persona) => {
                if let Some(f) = self.form_mut() {
                    f.persona = persona;
                }
                Vec::new()
            }
            DialogMsg::Submit => Vec::new(),
        }
    }

    fn view(&self) -> DialogView {
        let top = self.stack.len().checked_sub(1);
        DialogView {
            form: top
                .and_then(|l| self.forms.iter().find(|(i, _)| *i == l))
                .map(|(_, f)| f.view()),
            stack: self.stack.iter().map(|(k, _)| *k).collect(),
            about: AboutView {
                name: tr("app-name"),
                version: self.version.clone().unwrap_or_else(|| tr("bar-no-data")),
                platform: tr("about-platform"),
                config_path: tr("about-config-path"),
            },
        }
    }
}
