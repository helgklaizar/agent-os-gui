//! Bar labels come from `en.ftl`, not from literals (`i18n_bar_strings_from_ftl`).

mod bar_common;

use bar_common::*;
use controllers::Controller;
use controllers::bar_types::SysFrame;
use controllers::controllers::metrics::{MetricKind, MetricsController, MetricsMsg};
use controllers::controllers::quota::{QuotaController, QuotaMsg};
use controllers::i18n::{EN_FTL, tr, tr_args};

fn ftl_value(key: &str) -> Option<String> {
    EN_FTL
        .lines()
        .find_map(|l| {
            l.strip_prefix(key)
                .and_then(|r| r.trim_start().strip_prefix('='))
        })
        .map(|v| v.trim().to_string())
}

#[test]
fn i18n_bar_strings_from_ftl() {
    for (key, text) in [
        ("metric-disk", "Disk"),
        ("metric-temp", "Temperature"),
        ("quota-eta", "Runs out in { $duration }"),
        ("quota-enough", "Enough until reset"),
    ] {
        assert_eq!(ftl_value(key).as_deref(), Some(text), "key {key} in en.ftl");
    }
    // the view takes the same strings through `tr`, not its own copies
    let mut m = MetricsController::new();
    m.update(
        MetricsMsg::Frame(SysFrame {
            disk: Some(1.0),
            temp: Some(1.0),
            ..SysFrame::default()
        }),
        t0(),
    );
    let v = m.view();
    let label = |k: MetricKind| {
        v.items
            .iter()
            .find(|i| i.kind == k)
            .map(|i| i.label.clone())
    };
    assert_eq!(label(MetricKind::Disk), Some(tr("metric-disk")));
    assert_eq!(label(MetricKind::Temp), Some(tr("metric-temp")));
    let mut q = QuotaController::new();
    send(&mut q, QuotaMsg::Clock(NOW));
    send(
        &mut q,
        QuotaMsg::Snapshot(Some(vec![
            api_key("cl1", "x", 0.62, 1.4, 97),
            api_key("cl2", "y", 0.62, 1.4, 60),
        ])),
    );
    let tips: Vec<_> = q.view().items.iter().map(|i| i.tooltip.clone()).collect();
    assert!(
        tips[0].contains(&tr_args("quota-eta", &[("duration", "1 h 21 min")])),
        "{}",
        tips[0]
    );
    assert!(tips[1].contains(&tr("quota-enough")), "{}", tips[1]);
}
