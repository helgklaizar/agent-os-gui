//! `SearchController`: the unified search palette `Super+K` / `Ctrl+K`. The query goes to the core as a command
//! ([`CoreCommand::Search`]), results are grouped by kind, Enter publishes [`BusEvent::OpenHit`].

use crate::time::Instant;

use crate::bus::BusEvent;
use crate::controller::{Controller, CoreCommand, Effect};
use crate::hits::{Hit, HitKind};

/// How many results to request from the core.
const LIMIT: u32 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    Open,
    Close,
    Query(String),
    /// Core reply to the `query` request (the upper layer parsed it with `hits::parse_hits`).
    Results {
        query: String,
        hits: Vec<Hit>,
    },
    /// The `query` request failed.
    Failed {
        query: String,
    },
    /// Cyclic selection shift.
    Move(i32),
    Activate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Idle,
    Loading,
    Done,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitGroup {
    pub kind: HitKind,
    pub hits: Vec<Hit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchView {
    pub open: bool,
    pub query: String,
    pub status: Status,
    pub groups: Vec<HitGroup>,
    /// Index in the flat result list (groups in sequence).
    pub selected: Option<usize>,
}

#[derive(Debug, Default)]
pub struct SearchController {
    view: SearchView,
}

impl SearchController {
    fn flat(&self) -> Vec<&Hit> {
        self.view
            .groups
            .iter()
            .flat_map(|g| g.hits.iter())
            .collect()
    }

    fn reset(&mut self) {
        self.view = SearchView::default();
    }
}

fn group(hits: Vec<Hit>) -> Vec<HitGroup> {
    let mut groups: Vec<HitGroup> = Vec::new();
    for h in hits {
        match groups.iter_mut().find(|g| g.kind == h.kind) {
            Some(g) => g.hits.push(h),
            None => groups.push(HitGroup {
                kind: h.kind,
                hits: vec![h],
            }),
        }
    }
    groups.sort_by_key(|g| g.kind);
    groups
}

impl Controller for SearchController {
    type Msg = Msg;
    type View = SearchView;

    fn update(&mut self, msg: Msg, _now: Instant) -> Vec<Effect> {
        match msg {
            Msg::Open => {
                self.view.open = true;
                Vec::new()
            }
            Msg::Close => {
                self.reset();
                Vec::new()
            }
            Msg::Query(q) => {
                if !self.view.open {
                    return Vec::new();
                }
                self.view.groups.clear();
                self.view.selected = None;
                if q.trim().is_empty() {
                    self.view.query = q;
                    self.view.status = Status::Idle;
                    return Vec::new();
                }
                self.view.query = q.clone();
                self.view.status = Status::Loading;
                vec![Effect::Command(CoreCommand::Search {
                    query: q,
                    limit: LIMIT,
                })]
            }
            Msg::Results { query, hits } => {
                if !self.view.open || query != self.view.query {
                    return Vec::new();
                }
                self.view.groups = group(hits);
                self.view.status = Status::Done;
                self.view.selected = (!self.view.groups.is_empty()).then_some(0);
                Vec::new()
            }
            Msg::Failed { query } => {
                if self.view.open && query == self.view.query {
                    self.view.status = Status::Error;
                    self.view.groups.clear();
                    self.view.selected = None;
                }
                Vec::new()
            }
            Msg::Move(delta) => {
                let n = self.flat().len();
                if let (Some(cur), true) = (self.view.selected, n > 0) {
                    let n = n as i64;
                    self.view.selected = Some(
                        (i64::try_from(cur).unwrap_or(0) + i64::from(delta)).rem_euclid(n) as usize,
                    );
                }
                Vec::new()
            }
            Msg::Activate => {
                let target = self
                    .view
                    .selected
                    .and_then(|i| self.flat().get(i).and_then(|h| h.target.clone()));
                let Some(target) = target else {
                    return Vec::new();
                };
                self.reset();
                vec![Effect::Publish(BusEvent::OpenHit(target))]
            }
        }
    }

    fn view(&self) -> SearchView {
        self.view.clone()
    }
}
