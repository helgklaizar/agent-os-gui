//! S2-10: quota and metric color thresholds on both sides of each boundary, rounding before comparison (D10).

mod bar_common;

use bar_common::*;
use controllers::Controller;
use controllers::bar_types::SysFrame;
use controllers::controllers::metrics::{MetricKind, MetricsController, MetricsMsg};
use controllers::controllers::quota::{QuotaController, QuotaMsg};
use controllers::level::{
    Level, load_level, local_load_level, quota_rate_level, quota_remaining_level, round_pct,
    temp_level,
};

fn quota_view(
    item: controllers::bar_types::QuotaItem,
) -> controllers::controllers::quota::QuotaView {
    let mut c = QuotaController::new();
    send(&mut c, QuotaMsg::Clock(NOW));
    send(&mut c, QuotaMsg::Snapshot(Some(vec![item])));
    c.view().items.remove(0)
}

fn metric(kind: MetricKind, value: f64) -> (Option<i64>, Level) {
    let mut c = MetricsController::new();
    let mut frame = SysFrame::default();
    match kind {
        MetricKind::Cpu => frame.cpu = Some(value),
        MetricKind::Gpu => frame.gpu = Some(value),
        MetricKind::Ram => frame.ram = Some(value),
        MetricKind::Disk => frame.disk = Some(value),
        MetricKind::Temp => frame.temp = Some(value),
    }
    send(&mut c, MetricsMsg::Frame(frame));
    let v = c.view();
    let m = v.items.iter().find(|m| m.kind == kind).cloned();
    m.map(|m| (m.value, m.level))
        .unwrap_or((None, Level::NoData))
}

#[test]
fn quota_color_remaining() {
    for (pct, want) in [
        (41, Level::Ok),
        (40, Level::Warn),
        (16, Level::Warn),
        (15, Level::Bad),
        (14, Level::Bad),
    ] {
        let v = quota_view(with_remaining(f64::from(pct), 1.0));
        assert_eq!(v.remaining_pct, Some(i64::from(pct)), "remainder {pct}");
        assert_eq!(v.remaining_level, want, "remainder {pct}%");
        assert_eq!(
            quota_remaining_level(f64::from(pct)),
            want,
            "threshold function, {pct}%"
        );
    }
}

#[test]
fn quota_color_rate() {
    for (rate, want) in [
        (1.0, Level::Ok),
        (1.01, Level::Warn),
        (1.49, Level::Warn),
        (1.5, Level::Bad),
    ] {
        let v = quota_view(with_remaining(60.0, rate));
        assert_eq!(v.rate_level, want, "rate {rate}");
        assert_eq!(quota_rate_level(rate), want);
    }
}

#[test]
fn metric_color_load() {
    for kind in [
        MetricKind::Cpu,
        MetricKind::Gpu,
        MetricKind::Ram,
        MetricKind::Disk,
    ] {
        for (v, want) in [
            (49.0, Level::Neutral),
            (50.0, Level::Warn),
            (79.0, Level::Warn),
            (80.0, Level::Bad),
        ] {
            assert_eq!(metric(kind, v).1, want, "{kind:?} {v}%");
        }
    }
    assert_eq!(load_level(49.0), Level::Neutral);
    assert_eq!(load_level(80.0), Level::Bad);
}

#[test]
fn metric_color_temp() {
    for (v, want) in [
        (59.0, Level::Neutral),
        (60.0, Level::Warn),
        (79.0, Level::Warn),
        (80.0, Level::Bad),
    ] {
        assert_eq!(metric(MetricKind::Temp, v).1, want, "temperature {v}°");
    }
    assert_eq!(temp_level(59.0), Level::Neutral);
    assert_eq!(temp_level(80.0), Level::Bad);
}

#[test]
fn metric_color_local_model_load() {
    // local model GPU/VRAM load: 79/80/94/95
    for (v, want) in [
        (79.0, Level::Ok),
        (80.0, Level::Warn),
        (94.0, Level::Warn),
        (95.0, Level::Bad),
    ] {
        assert_eq!(local_load_level(v), want, "load {v}%");
        let view = quota_view(local("ol1", v / 100.0));
        assert_eq!(view.load_level, want, "load {v}% in the quota view");
    }
}

#[test]
fn quota_remaining_rounded_before_compare() {
    // D10: integers first (half-up), then comparison
    for (raw, pct, want) in [
        (40.4, 40, Level::Warn),
        (40.5, 41, Level::Ok),
        (40.6, 41, Level::Ok),
        (15.4, 15, Level::Bad),
        (15.5, 16, Level::Warn),
        (15.6, 16, Level::Warn),
    ] {
        let v = quota_view(with_remaining(raw, 1.0));
        assert_eq!(
            (v.remaining_pct, v.remaining_level),
            (Some(pct), want),
            "remainder {raw}"
        );
        assert_eq!(quota_remaining_level(raw), want);
    }
    assert_eq!(round_pct(40.5), 41);
    assert_eq!(round_pct(40.49), 40);
    assert_eq!(round_pct(0.0), 0);
}

#[test]
fn metric_value_is_rounded_to_whole_percent() {
    assert_eq!(
        metric(MetricKind::Cpu, 49.6),
        (Some(50), Level::Warn),
        "49.6 → 50 → yellow"
    );
    assert_eq!(metric(MetricKind::Cpu, 49.4), (Some(49), Level::Neutral));
}
