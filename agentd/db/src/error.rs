//! DB layer errors. The texts contain no user data.

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("DB is newer than the code: user_version {db}, known {known}")]
    NewerThanCode { db: i64, known: i64 },
    #[error("schema_migrations has version {version} without a migration file")]
    MissingMigrationFile { version: i64 },
    #[error("version {version}: name in schema_migrations does not match the file")]
    MigrationMismatch { version: i64 },
    #[error("user_version {user_version} does not equal max(schema_migrations) {schema_max}")]
    VersionMismatch { user_version: i64, schema_max: i64 },
    #[error("foreign_key_check after migration {version} is not empty")]
    ForeignKeyViolation { version: i64 },
    #[error("migration set is unordered or has duplicates")]
    InvalidMigrationSet,
    #[error("low disk space: {free} bytes free, at least {need} needed")]
    LowDisk { free: u64, need: u64 },
    #[error("write queue is full")]
    QueueFull,
    #[error("DB writer stopped")]
    WriterGone,
    #[error("read task panicked")]
    ReaderPanicked,
    #[error("write closure panicked")]
    WriterPanicked,
    #[error("DB is already open by another agentd instance")]
    AlreadyRunning,
    #[error("pragma {name}: got {got}")]
    Pragma { name: &'static str, got: String },
}
