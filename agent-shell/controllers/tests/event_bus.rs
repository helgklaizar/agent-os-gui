//! Tests of `EventBus` (S1-07: `event_bus_ordered_sync`).

use std::cell::RefCell;
use std::rc::Rc;

use controllers::{BusEvent, ConnState, EventBus, Theme};

fn log_events(bus: &EventBus) -> Rc<RefCell<Vec<BusEvent>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&log);
    bus.subscribe(move |e| sink.borrow_mut().push(e.clone()));
    log
}

#[test]
fn event_bus_ordered_sync() {
    let bus = EventBus::new();
    let a = log_events(&bus);
    let b = log_events(&bus);
    let events = vec![
        BusEvent::ThemeChanged(Theme::Dark),
        BusEvent::ConnectionChanged(ConnState::Connected),
        BusEvent::LayoutChanged,
    ];
    for e in &events {
        bus.publish(e.clone());
    }
    // delivery is synchronous: by the time publish returns everything is received, order preserved
    assert_eq!(*a.borrow(), events);
    assert_eq!(*b.borrow(), events);
}

#[test]
fn event_bus_publish_from_handler_is_queued_not_recursive() {
    let bus = EventBus::new();
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));

    let inner_bus = bus.clone();
    let s1 = Rc::clone(&seen);
    bus.subscribe(move |e| {
        s1.borrow_mut().push(format!("first:{e:?}"));
        if *e == BusEvent::LayoutChanged {
            inner_bus.publish(BusEvent::ThemeChanged(Theme::Light));
        }
    });
    let s2 = Rc::clone(&seen);
    bus.subscribe(move |e| s2.borrow_mut().push(format!("second:{e:?}")));

    bus.publish(BusEvent::LayoutChanged);

    // the second subscriber sees LayoutChanged before the nested publication (no recursion)
    assert_eq!(
        *seen.borrow(),
        vec![
            "first:LayoutChanged".to_string(),
            "second:LayoutChanged".to_string(),
            "first:ThemeChanged(Light)".to_string(),
            "second:ThemeChanged(Light)".to_string(),
        ]
    );
}

#[test]
fn event_bus_unsubscribe_stops_delivery() {
    let bus = EventBus::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&log);
    let id = bus.subscribe(move |e| sink.borrow_mut().push(e.clone()));
    bus.publish(BusEvent::LayoutChanged);
    bus.unsubscribe(id);
    bus.publish(BusEvent::LayoutChanged);
    assert_eq!(log.borrow().len(), 1);
}

#[test]
fn event_bus_subscribe_inside_handler_does_not_panic() {
    let bus = EventBus::new();
    let late = Rc::new(RefCell::new(Vec::new()));
    let b2 = bus.clone();
    let l2 = Rc::clone(&late);
    bus.subscribe(move |e| {
        if *e == BusEvent::LayoutChanged {
            let l3 = Rc::clone(&l2);
            b2.subscribe(move |e| l3.borrow_mut().push(e.clone()));
        }
    });
    bus.publish(BusEvent::LayoutChanged);
    bus.publish(BusEvent::ThemeChanged(Theme::Dark));
    // a subscriber added during delivery receives only later events
    assert_eq!(*late.borrow(), vec![BusEvent::ThemeChanged(Theme::Dark)]);
}

#[test]
fn event_bus_without_subscribers_is_noop() {
    EventBus::new().publish(BusEvent::LayoutChanged);
}

/// The bus is `!Send`: if `EventBus: Send`, the expression below is ambiguous and the test does not compile.
#[test]
fn event_bus_is_not_send() {
    trait AmbiguousIfSend<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}
    <EventBus as AmbiguousIfSend<_>>::check();
}
