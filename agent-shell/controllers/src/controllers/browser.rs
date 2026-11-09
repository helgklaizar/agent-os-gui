//! `BrowserController`: browser view, per-session history and bookmarks, quick links, agent commands.
//! WebKit loads pages in the view; the controller decides what and when to load and what to send to the core.

use crate::time::Instant;
use std::collections::{HashMap, VecDeque};

use crate::browser_addr::{Address, display_text, is_loadable, parse_address};
use crate::browser_view::{AgentLoad, BrowserMsg, BrowserView, LoadRequest};
use crate::controller::{Controller, CoreCommand, Effect};
use crate::tool_types::{BrowserCommand, Link, ToolResult};

const TITLE_MAX: usize = 200;
const ADDRESS_MAX: usize = 2048;
/// How many of the latest agent-command `request_id`s are remembered to guard against redelivery.
const RECENT: usize = 256;

#[derive(Debug, Default)]
struct Recent(VecDeque<String>);

impl Recent {
    fn has(&self, id: &str) -> bool {
        self.0.iter().any(|x| x == id)
    }

    fn add(&mut self, id: &str) {
        if self.0.len() == RECENT {
            self.0.pop_front();
        }
        self.0.push_back(id.to_string());
    }
}

/// State of a single session: history, bookmarks. The page lives in its own profile, so switching sessions does not lose it.
#[derive(Debug, Default)]
struct Tab {
    entries: Vec<(String, String)>,
    /// `None` means the start page.
    cursor: Option<usize>,
    /// Where "forward" returns after "Home".
    resume: Option<usize>,
    bookmarks: Vec<Link>,
    marks_rev: i64,
    marks_known: bool,
}

#[derive(Debug, Default)]
pub struct BrowserController {
    session: Option<i64>,
    tabs: HashMap<i64, Tab>,
    quick: Vec<Link>,
    quick_rev: i64,
    quick_asked: bool,
    seq: u64,
    load: Option<LoadRequest>,
    agent: Option<AgentLoad>,
    seen: Recent,
    answered: Recent,
}

impl BrowserController {
    pub fn new() -> Self {
        Self::default()
    }

    fn tab(&mut self) -> Option<(i64, &mut Tab)> {
        let id = self.session?;
        Some((id, self.tabs.entry(id).or_default()))
    }

    fn load(&mut self, session_id: i64, url: &str) {
        self.seq += 1;
        self.load = Some(LoadRequest {
            session_id,
            url: url.to_string(),
            seq: self.seq,
        });
    }

    fn navigate(&mut self, url: String) {
        let Some((id, tab)) = self.tab() else { return };
        let keep = tab.cursor.map_or(0, |c| c + 1);
        tab.entries.truncate(keep);
        tab.entries.push((url.clone(), String::new()));
        tab.cursor = Some(tab.entries.len() - 1);
        tab.resume = None;
        self.load(id, &url);
    }

    fn go_to(&mut self, index: Option<usize>) {
        let Some((id, tab)) = self.tab() else { return };
        tab.cursor = index;
        let url = index.and_then(|i| tab.entries.get(i)).map(|e| e.0.clone());
        if let Some(url) = url {
            tab.resume = None;
            self.load(id, &url);
        }
    }

    fn back(&mut self) {
        let Some((_, tab)) = self.tab() else { return };
        match tab.cursor {
            Some(0) => {
                tab.resume = Some(0);
                tab.cursor = None;
            }
            Some(i) => self.go_to(Some(i - 1)),
            None => {}
        }
    }

    fn forward(&mut self) {
        let Some((_, tab)) = self.tab() else { return };
        let next = match tab.cursor {
            Some(i) if i + 1 < tab.entries.len() => Some(i + 1),
            None => tab.resume.filter(|r| *r < tab.entries.len()),
            _ => None,
        };
        if next.is_some() {
            self.go_to(next);
        }
    }

    fn home(&mut self) {
        if let Some((_, tab)) = self.tab()
            && let Some(c) = tab.cursor
        {
            tab.resume = Some(c);
            tab.cursor = None;
        }
    }

    fn loaded(&mut self, session_id: i64, url: &str, title: &str) {
        if !is_loadable(url) {
            return;
        }
        let tab = self.tabs.entry(session_id).or_default();
        let Some(cur) = tab.cursor else { return };
        let title = display_text(title, TITLE_MAX);
        if tab.entries.get(cur).is_some_and(|e| e.0 == url) {
            tab.entries[cur].1 = title;
            return;
        }
        tab.entries.truncate(cur + 1);
        tab.entries.push((url.to_string(), title));
        tab.cursor = Some(tab.entries.len() - 1);
    }

    fn add_quick(&mut self, title: &str, url: &str, fx: &mut Vec<Effect>) {
        let Address::Url(url) = parse_address(url) else {
            return;
        };
        let title = display_text(title.trim(), TITLE_MAX);
        if title.is_empty() || self.quick.iter().any(|l| l.url == url) {
            return;
        }
        self.quick.push(Link { title, url });
        fx.push(Effect::Command(CoreCommand::PutQuickLinks {
            rev: self.quick_rev,
            items: self.quick.clone(),
        }));
    }

    fn put_marks(&self, session_id: i64, tab: &Tab) -> Effect {
        Effect::Command(CoreCommand::PutBookmarks {
            session_id,
            rev: tab.marks_rev,
            items: tab.bookmarks.clone(),
        })
    }

    fn add_mark(&mut self, fx: &mut Vec<Effect>) {
        let Some((id, tab)) = self.tab() else { return };
        let Some((url, title)) = tab.cursor.and_then(|c| tab.entries.get(c)).cloned() else {
            return;
        };
        if tab.bookmarks.iter().any(|l| l.url == url) {
            return;
        }
        let title = if title.is_empty() {
            display_text(&url, TITLE_MAX)
        } else {
            title
        };
        tab.bookmarks.push(Link { title, url });
        let effect = self.put_marks(id, &self.tabs[&id]);
        fx.push(effect);
    }

    fn remove_mark(&mut self, index: usize, fx: &mut Vec<Effect>) {
        let Some((id, tab)) = self.tab() else { return };
        if index >= tab.bookmarks.len() {
            return;
        }
        tab.bookmarks.remove(index);
        let effect = self.put_marks(id, &self.tabs[&id]);
        fx.push(effect);
    }

    fn agent_command(&mut self, session_id: i64, cmd: &BrowserCommand, fx: &mut Vec<Effect>) {
        if self.seen.has(&cmd.request_id) {
            return;
        }
        self.seen.add(&cmd.request_id);
        let refuse = |status: &str, why: &str| ToolResult::failed(&cmd.request_id, status, why);
        let result = if !matches!(cmd.command.as_str(), "open" | "navigate") {
            Some(refuse("error", "unsupported_command"))
        } else if !is_loadable(&cmd.url) {
            Some(refuse("denied", "url_scheme"))
        } else {
            None
        };
        match result {
            Some(result) => {
                self.answered.add(&cmd.request_id);
                fx.push(Effect::Command(CoreCommand::PostToolResult {
                    session_id,
                    result,
                }));
            }
            None => {
                self.agent = Some(AgentLoad {
                    session_id,
                    request_id: cmd.request_id.clone(),
                    url: cmd.url.clone(),
                });
            }
        }
    }

    fn agent_done(&mut self, session_id: i64, result: ToolResult, fx: &mut Vec<Effect>) {
        let rid = result.request_id.clone();
        if !self.seen.has(&rid) || self.answered.has(&rid) {
            return;
        }
        self.answered.add(&rid);
        if self.agent.as_ref().is_some_and(|a| a.request_id == rid) {
            self.agent = None;
        }
        fx.push(Effect::Command(CoreCommand::PostToolResult {
            session_id,
            result,
        }));
    }
}

impl Controller for BrowserController {
    type Msg = BrowserMsg;
    type View = BrowserView;

    fn update(&mut self, msg: BrowserMsg, _now: Instant) -> Vec<Effect> {
        let mut fx = Vec::new();
        match msg {
            BrowserMsg::Session(None) => self.session = None,
            BrowserMsg::Session(Some(id)) => {
                self.session = Some(id);
                if !std::mem::replace(&mut self.quick_asked, true) {
                    fx.push(Effect::Command(CoreCommand::FetchQuickLinks));
                }
                let tab = self.tabs.entry(id).or_default();
                if !std::mem::replace(&mut tab.marks_known, true) {
                    fx.push(Effect::Command(CoreCommand::FetchBookmarks {
                        session_id: id,
                    }));
                }
            }
            BrowserMsg::Submit(text) => match parse_address(&text) {
                Address::Empty => {}
                Address::Url(url) | Address::Search { url, .. } => self.navigate(url),
            },
            BrowserMsg::Back => self.back(),
            BrowserMsg::Forward => self.forward(),
            BrowserMsg::Reload => {
                if let Some((id, tab)) = self.tab()
                    && let Some(url) = tab
                        .cursor
                        .and_then(|c| tab.entries.get(c))
                        .map(|e| e.0.clone())
                {
                    self.load(id, &url);
                }
            }
            BrowserMsg::Home => self.home(),
            BrowserMsg::Loaded {
                session_id,
                url,
                title,
            } => self.loaded(session_id, &url, &title),
            BrowserMsg::Quick { items, rev } => {
                self.quick = items;
                self.quick_rev = rev;
            }
            BrowserMsg::Marks {
                session_id,
                items,
                rev,
            } => {
                let tab = self.tabs.entry(session_id).or_default();
                tab.bookmarks = items;
                tab.marks_rev = rev;
                tab.marks_known = true;
            }
            BrowserMsg::AddQuick { title, url } => self.add_quick(&title, &url, &mut fx),
            BrowserMsg::AddMark => self.add_mark(&mut fx),
            BrowserMsg::RemoveMark(i) => self.remove_mark(i, &mut fx),
            BrowserMsg::Stale { session_id: None } => {
                fx.push(Effect::Command(CoreCommand::FetchQuickLinks));
            }
            BrowserMsg::Stale {
                session_id: Some(session_id),
            } => fx.push(Effect::Command(CoreCommand::FetchBookmarks { session_id })),
            BrowserMsg::Agent {
                session_id,
                command,
            } => self.agent_command(session_id, &command, &mut fx),
            BrowserMsg::AgentDone { session_id, result } => {
                self.agent_done(session_id, result, &mut fx);
            }
        }
        fx
    }

    fn view(&self) -> BrowserView {
        let tab = self.session.and_then(|s| self.tabs.get(&s));
        let cur = tab.and_then(|t| t.cursor.and_then(|c| t.entries.get(c)));
        let (can_back, can_forward) = tab.map_or((false, false), |t| {
            let fwd = match t.cursor {
                Some(i) => i + 1 < t.entries.len(),
                None => t.resume.is_some_and(|r| r < t.entries.len()),
            };
            (t.cursor.is_some(), fwd)
        });
        BrowserView {
            session: self.session,
            start_page: cur.is_none(),
            address: cur.map_or_else(String::new, |e| display_text(&e.0, ADDRESS_MAX)),
            title: cur.map_or_else(String::new, |e| e.1.clone()),
            can_back,
            can_forward,
            load: self.load.clone(),
            agent: self.agent.clone(),
            quick: self.quick.clone(),
            bookmarks: tab.map_or_else(Vec::new, |t| t.bookmarks.clone()),
            bookmarked: cur
                .is_some_and(|e| tab.is_some_and(|t| t.bookmarks.iter().any(|l| l.url == e.0))),
        }
    }
}
