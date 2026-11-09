//! Synchronous bus of internal GUI events. Not the same as the core's WS topics.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::form_types::FormRequest;
use crate::hits::HitTarget;
use crate::i18n::Lang;
use crate::indicators::State;
use crate::types::{
    Column, ConnState, DialogKind, FocusTarget, SettingsPage, ShortcutAction, Tab, Theme,
};

/// Internal shell events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusEvent {
    OpenSession {
        session_id: i64,
        tab: Tab,
    },
    OpenSettings(SettingsPage),
    LayoutChanged,
    ThemeChanged(Theme),
    /// The user picked a theme in Settings: `ThemeController` applies it and sends it to the core.
    SelectTheme(Theme),
    ConnectionChanged(ConnState),
    ShowToast {
        session_id: i64,
        text: String,
    },
    /// Open a dialog; `opener` gets focus back on close.
    OpenDialog {
        kind: DialogKind,
        opener: FocusTarget,
    },
    /// Open a tree form (create, move, rename, delete): the request was built by `SessionTreeController`.
    OpenForm {
        form: FormRequest,
        opener: FocusTarget,
    },
    /// Expand a column manually (click on the collapsed strip icon); the manual state wins over auto-collapse.
    ExpandColumn(Column),
    /// Dialog closed: return focus to the element that opened it.
    FocusRestore(FocusTarget),
    /// A module tried to escape the sandbox; `kind` is a stable violation kind name, no details on the bus.
    ModuleViolation {
        module_id: String,
        kind: String,
    },
    /// Interface language saved in Settings → Account: the shell applies it without a restart.
    LanguageChanged(Lang),
    /// Global keyboard shortcut.
    ShortcutInvoked(ShortcutAction),
    /// Open a result found in the search palette.
    OpenHit(HitTarget),
    /// An indicator turned yellow or red and must be noticed; quiet hours are already accounted for.
    IndicatorAlert {
        id: String,
        state: State,
    },
}

/// Subscription id for [`EventBus::unsubscribe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriptionId(u64);

type Handler = Rc<dyn Fn(&BusEvent)>;

#[derive(Default)]
struct Inner {
    next_id: u64,
    handlers: Vec<(SubscriptionId, Handler)>,
    queue: VecDeque<BusEvent>,
    dispatching: bool,
}

/// Clears the delivery flag even if a handler panicked.
struct DispatchGuard<'a>(&'a RefCell<Inner>);

impl Drop for DispatchGuard<'_> {
    fn drop(&mut self) {
        self.0.borrow_mut().dispatching = false;
    }
}

/// Bus: delivery is synchronous, in publish order, on one thread. `!Send` (holds an `Rc`).
///
/// Publishing from a handler is not recursive: the event is queued and delivered to all subscribers
/// after the current event has been delivered to all. A subscription made in a handler takes effect from the next event.
#[derive(Clone, Default)]
pub struct EventBus {
    inner: Rc<RefCell<Inner>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self, handler: impl Fn(&BusEvent) + 'static) -> SubscriptionId {
        let mut inner = self.inner.borrow_mut();
        inner.next_id += 1;
        let id = SubscriptionId(inner.next_id);
        inner.handlers.push((id, Rc::new(handler)));
        id
    }

    pub fn unsubscribe(&self, id: SubscriptionId) {
        self.inner.borrow_mut().handlers.retain(|(h, _)| *h != id);
    }

    pub fn publish(&self, event: BusEvent) {
        {
            let mut inner = self.inner.borrow_mut();
            inner.queue.push_back(event);
            if inner.dispatching {
                return;
            }
            inner.dispatching = true;
        }
        let _guard = DispatchGuard(&self.inner);
        loop {
            // the borrow is released before handlers are called: they may publish and subscribe
            let (event, handlers) = {
                let mut inner = self.inner.borrow_mut();
                let Some(event) = inner.queue.pop_front() else {
                    return;
                };
                let handlers: Vec<Handler> =
                    inner.handlers.iter().map(|(_, h)| Rc::clone(h)).collect();
                (event, handlers)
            };
            for handler in handlers {
                handler(&event);
            }
        }
    }
}
