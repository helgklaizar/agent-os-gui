//! `ModuleStore` (A6-03): installed modules in `modules` (docs/reference/db.md section 3.9) and a local catalog file (read only).

use std::path::PathBuf;
use std::sync::Arc;

use agentd_core::{Module, ModuleError, ModuleStore, PortFuture, validate_manifest};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Value, json};

use crate::{Db, DbError};

const COLS: &str = "id, name, manifest_json, installed, enabled, placement, config_json";
/// A larger catalog is not read.
const MAX_CATALOG_BYTES: u64 = 1024 * 1024;

pub struct ModuleRepo {
    db: Arc<Db>,
    catalog: PathBuf,
}

impl ModuleRepo {
    /// `root` is `~/.agentos`; the catalog is `modules/catalog.json`.
    #[must_use]
    pub fn new(db: Arc<Db>, root: impl Into<PathBuf>) -> ModuleRepo {
        ModuleRepo {
            db,
            catalog: root.into().join("modules/catalog.json"),
        }
    }
}

fn row(r: &Row<'_>) -> rusqlite::Result<Module> {
    let (manifest, config): (String, String) = (r.get(2)?, r.get(6)?);
    Ok(Module {
        id: r.get(0)?,
        name: r.get(1)?,
        manifest: serde_json::from_str(&manifest).unwrap_or(Value::Null),
        installed: r.get::<_, i64>(3)? != 0,
        enabled: r.get::<_, i64>(4)? != 0,
        placement: r.get(5)?,
        config: serde_json::from_str(&config).unwrap_or_else(|_| json!({})),
    })
}

fn get(c: &Connection, id: &str) -> rusqlite::Result<Option<Module>> {
    c.query_row(
        &format!("SELECT {COLS} FROM modules WHERE id = ?1"),
        [id],
        row,
    )
    .optional()
}

fn unavailable(e: &DbError) -> ModuleError {
    tracing::warn!(scope = "modules", event = "db_unavailable", error = %e, "modules DB unavailable");
    ModuleError::Unavailable
}

impl ModuleStore for ModuleRepo {
    fn list(&self) -> PortFuture<'_, Result<Vec<Module>, ModuleError>> {
        Box::pin(async move {
            self.db
                .read(|c| {
                    c.prepare(&format!("SELECT {COLS} FROM modules ORDER BY id"))?
                        .query_map([], row)?
                        .collect()
                })
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn get(&self, id: String) -> PortFuture<'_, Result<Module, ModuleError>> {
        Box::pin(async move {
            self.db
                .read(move |c| get(c, &id))
                .await
                .map_err(|e| unavailable(&e))?
                .ok_or(ModuleError::NotFound)
        })
    }

    fn insert(&self, m: Module) -> PortFuture<'_, Result<Module, ModuleError>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    if get(tx, &m.id)?.is_some() {
                        return Ok(Err(ModuleError::Exists));
                    }
                    tx.execute(
                        &format!("INSERT INTO modules({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
                        params![
                            m.id,
                            m.name,
                            m.manifest.to_string(),
                            i64::from(m.installed),
                            i64::from(m.enabled),
                            m.placement,
                            m.config.to_string()
                        ],
                    )?;
                    Ok(get(tx, &m.id)?.ok_or(ModuleError::NotFound))
                })
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn update(&self, m: Module) -> PortFuture<'_, Result<Module, ModuleError>> {
        Box::pin(async move {
            self.db
                .write(move |tx| {
                    let n = tx.execute(
                        "UPDATE modules SET enabled = ?1, placement = ?2, config_json = ?3 WHERE id = ?4",
                        params![i64::from(m.enabled), m.placement, m.config.to_string(), m.id],
                    )?;
                    if n == 0 {
                        return Ok(Err(ModuleError::NotFound));
                    }
                    Ok(get(tx, &m.id)?.ok_or(ModuleError::NotFound))
                })
                .await
                .map_err(|e| unavailable(&e))?
        })
    }

    fn delete(&self, id: String) -> PortFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            let n = self
                .db
                .write(move |tx| tx.execute("DELETE FROM modules WHERE id = ?1", [id]))
                .await
                .map_err(|e| unavailable(&e))?;
            if n == 0 {
                Err(ModuleError::NotFound)
            } else {
                Ok(())
            }
        })
    }

    fn catalog(&self) -> PortFuture<'_, Vec<Value>> {
        let path = self.catalog.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let Ok(md) = std::fs::symlink_metadata(&path) else {
                    return Vec::new();
                };
                if !md.is_file() || md.len() > MAX_CATALOG_BYTES {
                    return Vec::new();
                }
                let parsed: Option<Vec<Value>> = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok());
                parsed
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|m| validate_manifest(m).is_empty())
                    .collect()
            })
            .await
            .unwrap_or_default()
        })
    }
}
