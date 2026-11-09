//! S2-04: metrics from `bar.sys`: values, disk changes, "no data" after three periods (6 s).

mod bar_common;

use bar_common::*;
use controllers::Controller;
use controllers::bar_types::SysFrame;
use controllers::controllers::metrics::{MetricKind, MetricsController, MetricsMsg, MetricsView};
use controllers::level::Level;

fn frame(cpu: f64, gpu: f64, ram: f64, disk: f64, temp: f64) -> SysFrame {
    SysFrame {
        cpu: Some(cpu),
        gpu: Some(gpu),
        ram: Some(ram),
        disk: Some(disk),
        temp: Some(temp),
    }
}

fn item(v: &MetricsView, kind: MetricKind) -> controllers::controllers::metrics::MetricView {
    v.items
        .iter()
        .find(|m| m.kind == kind)
        .cloned()
        .expect("metric in the view")
}

#[test]
fn metrics_view_has_five_in_order_with_labels_and_text() {
    let mut c = MetricsController::new();
    c.update(MetricsMsg::Frame(frame(14.0, 20.0, 66.0, 38.0, 38.0)), t0());
    let v = c.view();
    let kinds: Vec<_> = v.items.iter().map(|m| m.kind).collect();
    assert_eq!(
        kinds,
        [
            MetricKind::Cpu,
            MetricKind::Gpu,
            MetricKind::Ram,
            MetricKind::Disk,
            MetricKind::Temp
        ]
    );
    assert_eq!(item(&v, MetricKind::Disk).label, "Disk");
    assert_eq!(item(&v, MetricKind::Temp).label, "Temperature");
    assert_eq!(item(&v, MetricKind::Disk).text, "38%");
    assert_eq!(item(&v, MetricKind::Temp).text, "38°");
    assert_eq!(item(&v, MetricKind::Disk).accessible_name, "Disk: 38%");
    assert!(!v.no_data);
}

#[test]
fn metrics_disk_changes() {
    // prototype regression: disk is not a constant
    let mut c = MetricsController::new();
    let mut seen = Vec::new();
    for disk in [38.0, 39.0, 41.0, 40.0] {
        c.update(MetricsMsg::Frame(frame(14.0, 20.0, 66.0, disk, 38.0)), t0());
        seen.push(item(&c.view(), MetricKind::Disk).value);
    }
    assert_eq!(seen, [Some(38), Some(39), Some(41), Some(40)]);
}

#[test]
fn metrics_no_data_grey() {
    let mut c = MetricsController::new();
    assert!(c.view().no_data, "no data before the first frame");
    let start = t0();
    c.update(
        MetricsMsg::Frame(frame(14.0, 20.0, 66.0, 38.0, 38.0)),
        start,
    );
    c.update(MetricsMsg::Tick, later(start, 5_999));
    assert!(!c.view().no_data, "5.999 s without frames - still data");
    c.update(MetricsMsg::Tick, later(start, 6_000));
    let v = c.view();
    assert!(v.no_data, "6 s without frames - grey");
    assert!(
        v.items
            .iter()
            .all(|m| m.level == Level::NoData && m.text == "—" && m.value.is_none())
    );
    assert!(
        item(&v, MetricKind::Disk)
            .accessible_name
            .contains("no data")
    );
    // a frame brings the data back
    c.update(
        MetricsMsg::Frame(frame(15.0, 20.0, 66.0, 38.0, 38.0)),
        later(start, 6_100),
    );
    assert!(!c.view().no_data);
    assert_eq!(item(&c.view(), MetricKind::Cpu).value, Some(15));
}

#[test]
fn metrics_null_field_greys_only_that_metric() {
    let mut c = MetricsController::new();
    let mut f = frame(14.0, 20.0, 66.0, 38.0, 38.0);
    f.gpu = None;
    c.update(MetricsMsg::Frame(f), t0());
    let v = c.view();
    assert!(!v.no_data);
    assert_eq!(item(&v, MetricKind::Gpu).level, Level::NoData);
    assert_eq!(item(&v, MetricKind::Gpu).text, "—");
    assert_eq!(item(&v, MetricKind::Cpu).value, Some(14));
    c.update(MetricsMsg::Frame(SysFrame::default()), t0());
    assert!(c.view().no_data, "all fields null - no data");
}

#[test]
fn metrics_offline_greys_online_waits_for_frame() {
    let mut c = MetricsController::new();
    let start = t0();
    c.update(
        MetricsMsg::Frame(frame(14.0, 20.0, 66.0, 38.0, 38.0)),
        start,
    );
    c.update(MetricsMsg::Offline, later(start, 100));
    assert!(c.view().no_data);
    c.update(
        MetricsMsg::Frame(frame(14.0, 20.0, 66.0, 40.0, 38.0)),
        later(start, 200),
    );
    assert!(
        c.view().no_data,
        "while there is no connection, frames do not revive the bar"
    );
    c.update(MetricsMsg::Online, later(start, 300));
    c.update(
        MetricsMsg::Frame(frame(14.0, 20.0, 66.0, 41.0, 38.0)),
        later(start, 400),
    );
    assert_eq!(item(&c.view(), MetricKind::Disk).value, Some(41));
}
