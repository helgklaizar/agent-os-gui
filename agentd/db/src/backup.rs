//! Consistent DB snapshot for backup (A7-09): `VACUUM INTO` through a separate reading connection; the core writer is not
//! blocked (WAL), the snapshot is one whole file without `-wal`.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::DbError;

/// Contents of the `sessions.db` snapshot; the temporary file lives in a directory the library removes on exit.
pub fn snapshot_bytes(db: &Path) -> Result<Vec<u8>, DbError> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let dir = tempfile::tempdir()?;
    let out = dir.path().join("snapshot.db");
    conn.execute("VACUUM INTO ?1", [out.to_string_lossy().as_ref()])?;
    Ok(std::fs::read(out)?)
}
