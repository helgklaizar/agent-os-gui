//! Test channel provider `scripted-channel` (`test-support` only): connection mode, send outcomes and inbound messages
//! are set by `POST /v1/_test/channel` commands (see `ChannelControl`).

use std::collections::VecDeque;
use std::sync::Mutex;

use agentd_core::{
    Capability, ChannelAccount, ChannelControl, ChannelProvider, ConnectKind, Flow, FlowStatus,
    Health, Incoming, PortFuture, SendOutcome,
};
use serde_json::{Value, json};

#[derive(Default)]
struct State {
    qr: bool,
    scanned: bool,
    reject_token: bool,
    health_down: bool,
    outcomes: VecDeque<String>,
    incoming: Vec<Incoming>,
    sends: u32,
}

#[derive(Default)]
pub struct ScriptedChannel {
    state: Mutex<State>,
    secrets: Option<std::sync::Arc<dyn agentd_core::SecretStore>>,
}

impl ScriptedChannel {
    #[must_use]
    pub fn new(secrets: std::sync::Arc<dyn agentd_core::SecretStore>) -> ScriptedChannel {
        ScriptedChannel {
            state: Mutex::new(State::default()),
            secrets: Some(secrets),
        }
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl ChannelProvider for ScriptedChannel {
    fn kind(&self) -> &'static str {
        "scripted-channel"
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::Send, Capability::Receive]
    }

    fn connect_kind(&self) -> ConnectKind {
        if self.st().qr {
            ConnectKind::Qr
        } else {
            ConnectKind::Token
        }
    }

    fn connect<'a>(&'a self, acc: &'a ChannelAccount) -> PortFuture<'a, Flow> {
        Box::pin(async move {
            let has_token = match &self.secrets {
                Some(s) => s.get(&format!("account.{}", acc.id)).await.is_some(),
                None => false,
            };
            let st = self.st();
            if st.qr {
                let mut f = Flow::new(
                    ConnectKind::Qr,
                    if st.scanned {
                        FlowStatus::Connected
                    } else {
                        FlowStatus::WaitingUser
                    },
                );
                if !st.scanned {
                    f.qr_data = Some(format!("scripted-qr-{}", acc.id));
                    f.expires_at = Some(i64::MAX / 2);
                }
                return f;
            }
            let status = match (has_token, st.reject_token) {
                (false, _) => FlowStatus::WaitingUser,
                (true, true) => FlowStatus::Failed,
                (true, false) => FlowStatus::Connected,
            };
            Flow::new(ConnectKind::Token, status)
        })
    }

    fn health<'a>(&'a self, _acc: &'a ChannelAccount) -> PortFuture<'a, Health> {
        let down = self.st().health_down;
        Box::pin(async move { if down { Health::Down } else { Health::Ok } })
    }

    fn send<'a>(
        &'a self,
        _acc: &'a ChannelAccount,
        _to: &'a str,
        _text: &'a str,
        key: &'a str,
    ) -> PortFuture<'a, SendOutcome> {
        let mut st = self.st();
        st.sends += 1;
        let out = match st.outcomes.pop_front().as_deref() {
            Some("rejected") => SendOutcome::Rejected("scripted"),
            Some("unknown") => SendOutcome::Unknown,
            _ => SendOutcome::Delivered {
                ext_id: Some(format!("sent-{key}")),
            },
        };
        Box::pin(async move { out })
    }

    fn poll<'a>(&'a self, _acc: &'a ChannelAccount) -> PortFuture<'a, Vec<Incoming>> {
        let taken = std::mem::take(&mut self.st().incoming);
        Box::pin(async move { taken })
    }
}

impl ChannelControl for ScriptedChannel {
    fn apply(&self, body: Value) -> PortFuture<'_, Value> {
        let mut st = self.st();
        if let Some(m) = body["mode"].as_str() {
            st.qr = m == "qr";
        }
        if let Some(v) = body["verify"].as_str() {
            st.reject_token = v == "rejected";
        }
        if let Some(b) = body["scanned"].as_bool() {
            st.scanned = b;
        }
        if let Some(b) = body["health_down"].as_bool() {
            st.health_down = b;
        }
        if let Some(list) = body["outcomes"].as_array() {
            st.outcomes = list
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
        }
        if let Some(list) = body["incoming"].as_array() {
            st.incoming = list
                .iter()
                .filter_map(|m| {
                    Some(Incoming {
                        ext_id: m["ext_id"].as_str()?.to_string(),
                        handle: m["handle"].as_str()?.to_string(),
                        body: m["body"].as_str()?.to_string(),
                    })
                })
                .collect();
        }
        let out = json!({"sends": st.sends, "qr": st.qr, "scanned": st.scanned});
        Box::pin(async move { out })
    }
}
