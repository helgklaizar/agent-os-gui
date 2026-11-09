//! Core replies to Settings dialog commands: series outcome, error at the field or as a banner, input is not lost.

use crate::bus::BusEvent;
use crate::controller::Effect;
use crate::i18n::{Lang, tr};
use crate::settings_fields::doc_field_id;
use crate::settings_pages::spec_by_id;
use crate::settings_save::{banner, cmd, error_banner};
use crate::settings_state::{OpenFile, State};
use crate::settings_types::{Cmd, Op, Reply, SaveError};
use crate::settings_view::BannerKind;

fn finish(st: &mut State) {
    if st.pending == 0 && !st.failed {
        st.banner = Some(banner(BannerKind::Info, tr("set-saved")));
    }
}

fn fail(st: &mut State, op: &Op, e: &SaveError) -> Vec<Effect> {
    st.failed = true;
    let mut fx = Vec::new();
    let mut attached = false;
    if let Op::Patch(doc) = op {
        for path in &e.fields {
            let id = doc_field_id(doc, path);
            if spec_by_id(st, &id).is_some() {
                st.errors.insert(id, tr("set-err-field"));
                attached = true;
            }
        }
        if e.stale() {
            fx.push(cmd(Cmd::FetchPage(doc.clone())));
        }
    }
    if let Op::Secret(reference) = op {
        st.errors
            .insert(format!("s:{reference}"), tr("set-err-field"));
        attached = true;
    }
    if e.stale() && !matches!(op, Op::Patch(_)) {
        fx.push(cmd(Cmd::FetchAll));
    }
    st.banner = Some(error_banner(e));
    if attached && !e.stale() {
        st.banner = Some(banner(BannerKind::Error, tr("set-err-invalid")));
    }
    fx
}

/// Core reply to a dialog command.
pub fn reply(st: &mut State, reply: Reply) -> Vec<Effect> {
    st.model.apply(&reply);
    let mut fx = Vec::new();
    // language saved in the core: the shell applies it without a restart (only codes the GUI supports)
    if let Reply::Page {
        doc,
        value,
        saved: true,
        ..
    } = &reply
        && doc == "account"
        && let Some(lang) = value
            .get("language")
            .and_then(serde_json::Value::as_str)
            .and_then(Lang::from_code)
    {
        fx.push(Effect::Publish(BusEvent::LanguageChanged(lang)));
    }
    match reply {
        Reply::Page {
            doc, saved: true, ..
        } => {
            st.pending = st.pending.saturating_sub(1);
            st.clear_saved(&Op::Patch(doc.clone()));
            if doc == "indicators" {
                st.indicators = None;
            }
            finish(st);
        }
        Reply::File { path, text, rev } => {
            if st.file.as_ref().is_some_and(|f| f.path == path) {
                st.file = Some(OpenFile {
                    path,
                    rev,
                    text,
                    loaded: true,
                });
            }
        }
        Reply::FileSaved { path, rev } => {
            st.pending = st.pending.saturating_sub(1);
            if let Some(f) = st.file.as_mut().filter(|f| f.path == path) {
                f.rev = rev;
                if let Some(t) = st.sent.get("f:text") {
                    f.text = t.clone();
                }
            }
            st.clear_saved(&Op::File);
            fx.push(cmd(Cmd::FetchAll));
            finish(st);
        }
        Reply::Done(op) => {
            st.pending = st.pending.saturating_sub(1);
            if let Op::Secret(reference) = &op {
                st.secrets.remove(&format!("s:{reference}"));
                st.model.secrets.insert(reference.clone());
            }
            st.clear_saved(&op);
            fx.push(cmd(Cmd::FetchAll));
            finish(st);
        }
        Reply::ModulePending => {
            st.pending = st.pending.saturating_sub(1);
            st.banner = Some(banner(BannerKind::Info, tr("set-module-pending")));
        }
        Reply::Failed { op, error } => {
            if op != Op::Fetch {
                st.pending = st.pending.saturating_sub(1);
            }
            fx.extend(fail(st, &op, &error));
        }
        _ => {}
    }
    fx
}
