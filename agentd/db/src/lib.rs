//! The `agentd` core DB (docs/reference/db.md, docs/adr/0002-db-single-writer.md): migration runner, single writer, readers.
//! Only this crate declares `rusqlite` (docs/architecture.md section 4).

mod backup;
mod catalog;
mod chat;
mod error;
mod events;
mod export;
mod files;
mod gateway;
mod indicators;
mod integrity;
mod memory;
mod migrate;
mod modules;
mod perms;
mod quota;
mod retention;
mod search;
mod settings;
pub mod snapshot;
mod status;
mod storage;
mod store;
mod tasks;
mod tree;
mod triggers;
mod tunnels;

pub use backup::snapshot_bytes as backup_snapshot;
pub use catalog::MIGRATIONS;
pub use chat::ChatRepo;
pub use error::DbError;
pub use events::EventRepo;
pub use export::ExportRepo;
pub use files::FileRepo;
pub use gateway::GatewayRepo;
pub use indicators::IndicatorRepo;
pub use integrity::{
    BackupRecipients, Integrity, backup_recipients, check_conn, check_file, record_event,
};
pub use memory::MemoryRepo;
pub use migrate::{
    MigrateOptions, Migration, MigrationCheck, MigrationOutcome, RunnerEvent, check_migrations,
    migrate, user_version,
};
pub use modules::ModuleRepo;
pub use perms::PermRepo;
pub use quota::QuotaRepo;
pub use retention::{BATCH as RETENTION_BATCH, Retention, RetentionReport};
/// Write and read closure types: `agentd-db` consumers do not declare `rusqlite` themselves (docs/architecture.md section 4).
pub use rusqlite::{Connection, Result as SqlResult, Transaction, params};
pub use search::SearchRepo;
pub use settings::SettingsRepo;
pub use store::{Db, DbConfig};
pub use tasks::TaskRepo;
pub use tree::TreeRepo;
pub use triggers::TriggerRepo;
pub use tunnels::TunnelRepo;
