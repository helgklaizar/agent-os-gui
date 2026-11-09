//! Bridge from core-client events to the tool controllers: `session.<id>` frames (`browser.command`, `terminal.*`),
//! `sessions`, `bar.quota` (session models). No GTK: works on macOS too, so it is covered by integration tests.

use std::time::Instant;

use controllers::bar_types::parse_quota_snapshot;
use controllers::browser_view::BrowserMsg;
use controllers::controllers::browser::BrowserController;
use controllers::controllers::session_settings::SessionSettingsController;
use controllers::controllers::terminal::TerminalController;
use controllers::ss_view::{ModelChoice, SsMsg};
use controllers::terminal_view::TermMsg;
use controllers::tool_types::{parse_browser_command, parse_term_frame};
use controllers::tree_types::{parse_delta, parse_snapshot};
use controllers::{Controller, Effect};
use core_client::CoreEvent;
use core_client::api::FrameKind;

use crate::tools_exec::ToolsReply;

fn session_of(topic: &str) -> Option<i64> {
    topic.strip_prefix("session.")?.parse().ok()
}

/// Tool controllers that live together.
pub struct ToolsCtls {
    pub browser: BrowserController,
    pub terminal: TerminalController,
    pub settings: SessionSettingsController,
}

impl ToolsCtls {
    pub fn new(home: std::path::PathBuf) -> ToolsCtls {
        ToolsCtls {
            browser: BrowserController::new(),
            terminal: TerminalController::new(home),
            settings: SessionSettingsController::new(),
        }
    }

    /// A session is opened (`None` means closed): all three tabs learn about it at once.
    pub fn open(&mut self, session: Option<i64>, now: Instant) -> Vec<Effect> {
        let mut fx = self.browser.update(BrowserMsg::Session(session), now);
        fx.extend(self.terminal.update(TermMsg::Session(session), now));
        fx.extend(self.settings.update(SsMsg::Open(session), now));
        fx
    }

    /// Core client event -> controllers. A frame of the wrong shape and an unknown `kind` are skipped (api.md section 2).
    pub fn route(&mut self, event: &CoreEvent, now: Instant) -> Vec<Effect> {
        let CoreEvent::Frame(f) = event else {
            return Vec::new();
        };
        let mut fx = Vec::new();
        match f.topic.as_str() {
            "sessions" => match f.kind {
                FrameKind::Snapshot => {
                    if let Some((_, sessions)) = parse_snapshot(&f.data) {
                        fx.extend(
                            self.terminal
                                .update(TermMsg::Snapshot(sessions.clone()), now),
                        );
                        fx.extend(self.settings.update(SsMsg::Snapshot(sessions), now));
                    }
                }
                FrameKind::Delta => {
                    if let Some(frame) = parse_delta(&f.data) {
                        fx.extend(self.terminal.update(TermMsg::Tree(frame.clone()), now));
                        fx.extend(self.settings.update(SsMsg::Tree(frame), now));
                    }
                }
                FrameKind::Event => {}
            },
            "bar.quota" if f.kind == FrameKind::Snapshot => {
                if let Some(items) = parse_quota_snapshot(&f.data) {
                    let models = items
                        .into_iter()
                        .map(|q| ModelChoice {
                            account_id: q.account_id,
                            label: q.label,
                        })
                        .collect();
                    fx.extend(self.settings.update(SsMsg::Models(models), now));
                }
            }
            topic => {
                let Some(session_id) = session_of(topic) else {
                    return fx;
                };
                if f.kind != FrameKind::Event {
                    return fx;
                }
                match f.data.get("kind").and_then(|k| k.as_str()) {
                    Some("browser.command") => {
                        if let Some(command) = parse_browser_command(&f.data) {
                            fx.extend(self.browser.update(
                                BrowserMsg::Agent {
                                    session_id,
                                    command,
                                },
                                now,
                            ));
                        }
                    }
                    Some(k) if k.starts_with("terminal.") => {
                        if let Some(frame) = parse_term_frame(&f.data) {
                            fx.extend(
                                self.terminal
                                    .update(TermMsg::Agent { session_id, frame }, now),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        fx
    }

    /// Core reply to a tool command -> controllers.
    pub fn reply(&mut self, reply: ToolsReply, now: Instant) -> Vec<Effect> {
        match reply {
            ToolsReply::Quick { items, rev } => {
                self.browser.update(BrowserMsg::Quick { items, rev }, now)
            }
            ToolsReply::Marks {
                session_id,
                items,
                rev,
            } => self.browser.update(
                BrowserMsg::Marks {
                    session_id,
                    items,
                    rev,
                },
                now,
            ),
            ToolsReply::Stale { session_id } => {
                self.browser.update(BrowserMsg::Stale { session_id }, now)
            }
            ToolsReply::Perms(p) => self.settings.update(SsMsg::Perms(p), now),
            ToolsReply::PermRejected => self.settings.update(SsMsg::PermRejected, now),
            ToolsReply::Tasks { session_id, items } => self
                .settings
                .update(SsMsg::Tasks { session_id, items }, now),
            ToolsReply::TaskChanged(t) => self.settings.update(SsMsg::TaskChanged(t), now),
            ToolsReply::Runs { task_id, items } => {
                self.settings.update(SsMsg::Runs { task_id, items }, now)
            }
            ToolsReply::Exported { path, size } => {
                self.settings.update(SsMsg::Exported { path, size }, now)
            }
        }
    }
}
