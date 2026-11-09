#![deny(clippy::unwrap_used)]
//! GUI controllers without GTK: event bus, layout, theme, connection.
//!
//! Everything is synchronous and single-threaded (`!Send`): `controllers` knows nothing about GTK or `tokio`.
//! Controllers do not import each other and communicate only through [`bus::BusEvent`].

pub mod bar_cluster;
pub mod bar_types;
pub mod browser_addr;
pub mod browser_view;
pub mod bus;
pub mod chat_attach;
pub mod chat_bubbles;
pub mod chat_cards;
pub mod chat_chips;
pub mod chat_input;
pub mod chat_msg;
pub mod chat_observer;
pub mod chat_stream;
pub mod chat_types;
pub mod chat_view;
pub mod clock_fmt;
pub mod controller;
pub mod controllers;
pub mod dialog_form;
pub mod forecast;
pub mod form_types;
pub mod hits;
pub mod i18n;
pub mod indicators;
pub mod keys;
pub mod layout;
pub mod level;
pub mod manifest;
pub mod markdown;
pub mod module_manifest;
pub mod module_types;
pub mod notif_model;
pub mod notif_render;
pub mod notif_types;
pub mod notif_view;
pub mod permissions;
pub mod prefs;
pub mod secrets;
pub mod settings_actions;
pub mod settings_entities;
pub mod settings_fields;
pub mod settings_forms;
pub mod settings_lists;
pub mod settings_model;
pub mod settings_new;
pub mod settings_pages;
pub mod settings_parse;
pub mod settings_perms;
pub mod settings_policy;
pub mod settings_reply;
pub mod settings_save;
pub mod settings_specs;
pub mod settings_state;
pub mod settings_types;
pub mod settings_view;
pub mod ss_view;
pub mod terminal_view;
pub mod testkit;
pub mod text;
pub mod time;
pub mod timezone;
pub mod tool_types;
pub mod tree_forms;
pub mod tree_model;
pub mod tree_nav;
pub mod tree_render;
pub mod tree_types;
pub mod tree_view;
pub mod types;

pub use bus::{BusEvent, EventBus, SubscriptionId};
pub use controller::{CfgPatch, Controller, CoreCommand, Effect};
pub use types::{
    Column, ConnState, DialogKind, Fallback, FocusColumn, FocusTarget, Manual, MenuItem,
    ResolvedTheme, SettingsPage, ShortcutAction, Tab, Theme, TreeButton,
};
