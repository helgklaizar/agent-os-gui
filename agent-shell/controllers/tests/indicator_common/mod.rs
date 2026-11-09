//! Shared rig for `IndicatorController` tests: bus, clock, helpers.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use controllers::controllers::indicator::{BarView, Frame, IndicatorCfg, IndicatorController, Msg};
use controllers::indicators::State;
use controllers::{BusEvent, Controller, Effect, EventBus};

pub struct Rig {
    pub c: IndicatorController,
    bus: EventBus,
    seen: std::rc::Rc<std::cell::RefCell<Vec<BusEvent>>>,
    pub t0: Instant,
}

impl Rig {
    pub fn new() -> Rig {
        let bus = EventBus::new();
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let s = seen.clone();
        bus.subscribe(move |e| s.borrow_mut().push(e.clone()));
        Rig {
            c: IndicatorController::default(),
            bus,
            seen,
            t0: Instant::now(),
        }
    }

    pub fn send_at(&mut self, msg: Msg, secs: u64) {
        for e in self.c.update(msg, self.t0 + Duration::from_secs(secs)) {
            if let Effect::Publish(ev) = e {
                self.bus.publish(ev);
            }
        }
    }

    pub fn send(&mut self, msg: Msg) {
        self.send_at(msg, 0);
    }

    pub fn alerts(&self) -> Vec<(String, State)> {
        self.seen
            .borrow()
            .iter()
            .filter_map(|e| match e {
                BusEvent::IndicatorAlert { id, state } => Some((id.clone(), *state)),
                _ => None,
            })
            .collect()
    }

    pub fn config(&mut self, ids: &[&str]) {
        let cfg = ids
            .iter()
            .enumerate()
            .map(|(i, id)| IndicatorCfg {
                id: (*id).into(),
                enabled: true,
                position: i as i64,
            })
            .collect();
        self.send(Msg::Config(cfg));
        self.send(Msg::Width(2000));
    }

    pub fn frame(&mut self, id: &str, state: State, seq: u64) {
        self.send(Msg::Frame(Frame {
            id: id.into(),
            state,
            seq,
            value: None,
            text: None,
        }));
    }

    pub fn view(&self) -> BarView {
        self.c.view()
    }

    pub fn state_of(&self, id: &str) -> Option<State> {
        self.view()
            .visible
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.state)
    }
}
