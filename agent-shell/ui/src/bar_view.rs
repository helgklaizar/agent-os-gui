//! Bar widgets: quotas, metrics, weather, date, clock, Agent OS menu. No logic here: values, colours and captions
//! arrive ready-made from `BarClusterView`, text is shown as plain text (no Pango markup).

use std::cell::RefCell;

use controllers::bar_cluster::BarClusterView;
use controllers::controllers::connection::ConnectionView as ControllerConnectionView;
use controllers::controllers::metrics::MetricKind;
use controllers::controllers::quota::QuotaView;
use controllers::i18n::tr;
use controllers::level::Level;
use controllers::{MenuItem, bar_types::QuotaKind};
use gtk4 as gtk;
use gtk4::prelude::*;

use crate::a11y;

const LEVEL_CLASSES: [&str; 5] = ["lvl-neutral", "lvl-ok", "lvl-warn", "lvl-bad", "lvl-nodata"];

fn level_class(level: Level) -> &'static str {
    match level {
        Level::Neutral => "lvl-neutral",
        Level::Ok => "lvl-ok",
        Level::Warn => "lvl-warn",
        Level::Bad => "lvl-bad",
        Level::NoData => "lvl-nodata",
    }
}

fn set_level(widget: &impl IsA<gtk::Widget>, level: Level) {
    for class in LEVEL_CLASSES {
        widget.remove_css_class(class);
    }
    widget.add_css_class(level_class(level));
}

fn plain_label(css: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_use_markup(false);
    label.add_css_class(css);
    label
}

/// One quota icon: a button (a click opens Settings -> Models) with an icon, percentage, bar and rate.
struct QuotaParts {
    account_id: i64,
    button: gtk::Button,
    icon: gtk::Image,
    pct: gtk::Label,
    bar: gtk::ProgressBar,
    rate: gtk::Label,
}

fn quota_parts(account_id: i64) -> QuotaParts {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    let icon = gtk::Image::new();
    icon.set_pixel_size(12);
    let pct = plain_label("quota-pct");
    let bar = gtk::ProgressBar::new();
    bar.add_css_class("quota-bar");
    bar.set_valign(gtk::Align::Center);
    bar.set_size_request(16, 3);
    let rate = plain_label("quota-rate");
    for w in [
        icon.upcast_ref::<gtk::Widget>(),
        pct.upcast_ref(),
        bar.upcast_ref(),
        rate.upcast_ref(),
    ] {
        row.append(w);
    }
    let button = gtk::Button::new();
    button.set_child(Some(&row));
    button.add_css_class("bar-btn");
    button.add_css_class("quota-btn");
    QuotaParts {
        account_id,
        button,
        icon,
        pct,
        bar,
        rate,
    }
}

fn update_quota(p: &QuotaParts, v: &QuotaView) {
    p.icon.set_icon_name(Some(match v.kind {
        QuotaKind::Local => "computer-symbolic",
        QuotaKind::ApiKey => "dialog-password-symbolic",
    }));
    let pct = v.remaining_pct.or(v.load_pct);
    p.pct
        .set_label(&pct.map_or_else(|| tr("value-none"), |n| format!("{n}%")));
    p.rate.set_label(v.rate_text.as_deref().unwrap_or(""));
    p.rate.set_visible(v.rate_text.is_some());
    p.bar.set_fraction(v.bar_fraction);
    set_level(&p.pct, v.level);
    set_level(&p.bar, v.level);
    set_level(&p.icon, v.level);
    set_level(&p.rate, v.rate_level);
    a11y::set_name(&p.button, &v.accessible_name);
    p.button.set_tooltip_text(Some(&v.tooltip));
}

pub struct BarWidgets {
    pub menu_button: gtk::Button,
    /// Menu layer over the window: an invisible backdrop catches a click "outside the menu", a card with items under the bar.
    pub menu_layer: gtk::Box,
    pub menu_card: gtk::Box,
    pub menu_items: RefCell<Vec<(MenuItem, gtk::Button)>>,
    pub quotas: gtk::Box,
    quota_parts: RefCell<Vec<QuotaParts>>,
    metrics: Vec<(MetricKind, gtk::Label)>,
    weather_box: gtk::Box,
    weather_icon: gtk::Image,
    weather_temp: gtk::Label,
    date: gtk::Label,
    time: gtk::Label,
    banner: gtk::Label,
    last: RefCell<Option<BarClusterView>>,
}

impl BarWidgets {
    /// Builds the bar content in `root`: menu on the left, quotas and metrics in the centre, weather, date, clock and theme on the right.
    pub fn build(root: &gtk::Box, theme_button: &gtk::Button) -> BarWidgets {
        let logo = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        logo.add_css_class("logo");
        logo.set_valign(gtk::Align::Center);
        logo.set_margin_start(10);
        logo.set_margin_end(4);
        // the caption is a child Label, not the `label` property: otherwise GtkButton overwrites the accessibility name with the caption text
        let menu_button = gtk::Button::new();
        menu_button.set_child(Some(&plain_label("menu-title")));
        if let Some(title) = menu_button.child().and_downcast::<gtk::Label>() {
            title.set_label(&tr("app-name"));
        }
        menu_button.add_css_class("bar-btn");
        a11y::set_name(&menu_button, &tr("bar-menu-button"));
        // the menu is a layer inside the window, not a popup surface: Wayland hands out a popup only on an input event,
        // while a layer works the same in kiosk, under a headless compositor and for accessibility technologies
        let menu_layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        menu_layer.add_css_class("menu-dim");
        menu_layer.set_hexpand(true);
        menu_layer.set_vexpand(true);
        menu_layer.set_visible(false);
        let menu_card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .accessible_role(gtk::AccessibleRole::Menu)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .build();
        menu_card.add_css_class("menu-card");
        menu_card.set_margin_top(crate::shell_view::BAR_HEIGHT);
        menu_card.set_margin_start(8);
        a11y::set_name(&menu_card, &tr("bar-menu-button"));
        menu_layer.append(&menu_card);
        let banner = plain_label("bar-banner");
        banner.set_label(&tr("bar-banner-offline"));
        banner.add_css_class("lvl-bad");
        banner.set_visible(false);

        let quotas = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        quotas.add_css_class("quotas");
        let sep = gtk::Separator::new(gtk::Orientation::Vertical);
        sep.set_margin_top(5);
        sep.set_margin_bottom(5);
        sep.set_margin_start(6);
        sep.set_margin_end(6);
        let metrics_box = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        let metrics: Vec<(MetricKind, gtk::Label)> = [
            MetricKind::Cpu,
            MetricKind::Gpu,
            MetricKind::Ram,
            MetricKind::Disk,
            MetricKind::Temp,
        ]
        .into_iter()
        .map(|k| (k, plain_label("metric")))
        .collect();
        for (_, label) in &metrics {
            metrics_box.append(label);
        }
        let center = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        center.set_hexpand(true);
        center.set_halign(gtk::Align::Center);
        center.append(&quotas);
        center.append(&sep);
        center.append(&metrics_box);

        let weather_box = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        let weather_icon = gtk::Image::new();
        weather_icon.set_pixel_size(12);
        let weather_temp = plain_label("weather");
        weather_box.append(&weather_icon);
        weather_box.append(&weather_temp);
        weather_box.set_margin_end(6);
        let date = plain_label("bar-date");
        date.set_margin_end(6);
        let time = plain_label("bar-time");
        time.set_margin_end(4);

        for w in [
            logo.upcast_ref::<gtk::Widget>(),
            menu_button.upcast_ref(),
            banner.upcast_ref(),
            center.upcast_ref(),
            weather_box.upcast_ref(),
            date.upcast_ref(),
            time.upcast_ref(),
            theme_button.upcast_ref(),
        ] {
            root.append(w);
        }
        BarWidgets {
            menu_button,
            menu_layer,
            menu_card,
            menu_items: RefCell::new(Vec::new()),
            quotas,
            quota_parts: RefCell::new(Vec::new()),
            metrics,
            weather_box,
            weather_icon,
            weather_temp,
            date,
            time,
            banner,
            last: RefCell::new(None),
        }
    }

    /// Account quota icon button (for signals and tests).
    pub fn quota_button(&self, account_id: i64) -> Option<gtk::Button> {
        let parts = self.quota_parts.borrow();
        parts
            .iter()
            .find(|p| p.account_id == account_id)
            .map(|p| p.button.clone())
    }

    pub fn menu_button_for(&self, item: MenuItem) -> Option<gtk::Button> {
        self.menu_items
            .borrow()
            .iter()
            .find(|(i, _)| *i == item)
            .map(|(_, b)| b.clone())
    }

    /// Update widgets from the view; an identical view touches nothing (otherwise focus and tooltip would flicker every second).
    pub fn apply(&self, view: &BarClusterView) {
        if self.last.borrow().as_ref() == Some(view) {
            return;
        }
        self.apply_quotas(view);
        for (kind, label) in &self.metrics {
            if let Some(m) = view.metrics.items.iter().find(|m| m.kind == *kind) {
                let caption = if *kind == MetricKind::Temp {
                    m.caption.clone()
                } else {
                    format!("{} ", m.caption)
                };
                label.set_label(&format!("{caption}{}", m.text));
                set_level(label, m.level);
                a11y::set_name(label, &m.accessible_name);
                label.set_tooltip_text(Some(&m.label));
            }
        }
        let w = &view.bar.weather;
        self.weather_icon.set_icon_name(Some(w.icon));
        self.weather_temp.set_label(&w.temp);
        set_level(&self.weather_temp, w.level);
        set_level(&self.weather_icon, w.level);
        self.weather_box.set_tooltip_text(Some(&w.tooltip));
        // AT-SPI does not show a container's name: it is set on the caption
        a11y::set_name(
            &self.weather_temp,
            &format!("{}: {}", tr("weather-name"), w.tooltip),
        );
        self.date.set_label(&view.bar.date);
        self.time.set_label(&view.bar.time);
        *self.last.borrow_mut() = Some(view.clone());
    }

    /// Connection banner from the `ConnectionController` view: "No connection" or "Core does not start".
    pub fn apply_connection(&self, view: &ControllerConnectionView) {
        self.banner.set_label(&if view.banner_core_unavailable {
            tr("bar-banner-core-unavailable")
        } else {
            tr("bar-banner-offline")
        });
        self.banner
            .set_visible(view.banner_offline || view.banner_core_unavailable);
    }

    fn apply_quotas(&self, view: &BarClusterView) {
        let ids: Vec<i64> = view.quotas.items.iter().map(|q| q.account_id).collect();
        let mut parts = self.quota_parts.borrow_mut();
        if parts.iter().map(|p| p.account_id).ne(ids.iter().copied()) {
            while let Some(child) = self.quotas.first_child() {
                self.quotas.remove(&child);
            }
            *parts = ids.iter().map(|id| quota_parts(*id)).collect();
            for p in parts.iter() {
                self.quotas.append(&p.button);
            }
        }
        for (p, v) in parts.iter().zip(&view.quotas.items) {
            update_quota(p, v);
        }
    }
}
