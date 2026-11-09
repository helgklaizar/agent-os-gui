//! Controllers. Each lives in its own file and does not reference the others (checked by a test).

pub mod connection;
pub mod shell;
pub mod theme;

pub mod bar;
pub mod browser;
pub mod chat;
pub mod dialog;
pub mod indicator;
pub mod metrics;
pub mod modules;
pub mod notifications;
pub mod quota;
pub mod search;
pub mod session_settings;
pub mod session_tree;
pub mod settings;
pub mod shortcut;
pub mod terminal;
pub mod tools;

pub use bar::BarController;
pub use browser::BrowserController;
pub use chat::ChatController;
pub use dialog::DialogController;
pub use indicator::IndicatorController;
pub use metrics::MetricsController;
pub use modules::ModulesController;
pub use notifications::NotificationsController;
pub use quota::QuotaController;
pub use search::SearchController;
pub use session_settings::SessionSettingsController;
pub use session_tree::SessionTreeController;
pub use settings::SettingsController;
pub use shortcut::ShortcutController;
pub use terminal::TerminalController;
pub use tools::ToolsController;
