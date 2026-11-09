//! Tests of `ThemeController` (S1-07: `theme_auto_follows_system`, `theme_cache_offline_core_wins_online`).

use std::time::Instant;

use controllers::controllers::theme::{ThemeController, ThemeMsg};
use controllers::prefs::UiPrefs;
use controllers::{BusEvent, Controller, CoreCommand, Effect, ResolvedTheme, Theme};

fn persists_theme(effects: &[Effect]) -> Option<Theme> {
    effects.iter().find_map(|e| match e {
        Effect::Persist(UiPrefs { theme: Some(t), .. }) => Some(*t),
        _ => None,
    })
}

fn has_patch(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::Command(CoreCommand::PatchTheme(_))))
}

#[test]
fn theme_auto_follows_system() {
    let now = Instant::now();
    let mut c = ThemeController::new(true);
    assert_eq!(c.view().selected, Theme::Auto);
    assert_eq!(c.view().resolved, ResolvedTheme::Dark);

    let fx = c.update(ThemeMsg::SystemChanged { dark: false }, now);
    assert_eq!(c.view().resolved, ResolvedTheme::Light);
    assert!(fx.contains(&Effect::Publish(BusEvent::ThemeChanged(Theme::Auto))));

    c.update(ThemeMsg::SystemChanged { dark: true }, now);
    assert_eq!(c.view().resolved, ResolvedTheme::Dark);
}

#[test]
fn theme_explicit_choice_overrides_system_and_goes_to_core() {
    let now = Instant::now();
    let mut c = ThemeController::new(false);
    let fx = c.update(ThemeMsg::UserSelected(Theme::Dark), now);
    assert_eq!(c.view().selected, Theme::Dark);
    assert_eq!(c.view().resolved, ResolvedTheme::Dark);
    assert!(fx.contains(&Effect::Command(CoreCommand::PatchTheme(Theme::Dark))));
    assert!(fx.contains(&Effect::Publish(BusEvent::ThemeChanged(Theme::Dark))));
    assert_eq!(persists_theme(&fx), Some(Theme::Dark));

    // the system theme no longer matters
    let fx = c.update(ThemeMsg::SystemChanged { dark: true }, now);
    assert_eq!(c.view().resolved, ResolvedTheme::Dark);
    assert!(fx.is_empty(), "{fx:?}");
    c.update(ThemeMsg::SystemChanged { dark: false }, now);
    assert_eq!(c.view().resolved, ResolvedTheme::Dark);
}

#[test]
fn theme_cache_offline_core_wins_online() {
    let now = Instant::now();
    let mut c = ThemeController::new(false);

    // offline: theme from cache, nothing sent to the core, cache not rewritten
    let fx = c.update(ThemeMsg::CacheLoaded(Theme::Dark), now);
    assert_eq!(c.view().selected, Theme::Dark);
    assert_eq!(c.view().resolved, ResolvedTheme::Dark);
    assert!(!has_patch(&fx));
    assert_eq!(persists_theme(&fx), None);

    // connected: the core wins and overwrites the cache, no PATCH back
    let fx = c.update(ThemeMsg::CoreValue(Theme::Light), now);
    assert_eq!(c.view().selected, Theme::Light);
    assert_eq!(c.view().resolved, ResolvedTheme::Light);
    assert_eq!(persists_theme(&fx), Some(Theme::Light));
    assert!(fx.contains(&Effect::Publish(BusEvent::ThemeChanged(Theme::Light))));
    assert!(!has_patch(&fx));
}

#[test]
fn theme_core_value_equal_to_current_is_noop() {
    let now = Instant::now();
    let mut c = ThemeController::new(false);
    c.update(ThemeMsg::CoreValue(Theme::Dark), now);
    let fx = c.update(ThemeMsg::CoreValue(Theme::Dark), now);
    assert!(fx.is_empty(), "{fx:?}");
}

#[test]
fn theme_reselecting_same_theme_is_noop() {
    let now = Instant::now();
    let mut c = ThemeController::new(false);
    c.update(ThemeMsg::UserSelected(Theme::Dark), now);
    let fx = c.update(ThemeMsg::UserSelected(Theme::Dark), now);
    assert!(fx.is_empty(), "{fx:?}");
}
