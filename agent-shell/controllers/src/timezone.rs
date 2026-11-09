//! Bar clock time zone: the system one, `Asia/Jerusalem` when unavailable. Reads TZif (RFC 8536) from zoneinfo.
//!
//! Limitation: transitions come from the file; after the last transition (2037 for the "fat" Ubuntu and macOS files)
//! the last offset stays, the trailing POSIX TZ rule string is not parsed.

use std::path::{Component, Path};

/// Fallback zone.
pub const FALLBACK_ZONE: &str = "Asia/Jerusalem";

const ZONEINFO: &str = "/usr/share/zoneinfo";
const MAX_FILE: u64 = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    /// Transitions ascending: unix start time and type index.
    transitions: Vec<(i64, usize)>,
    /// Type offsets from UTC, seconds; type 0 applies until the first transition.
    offsets: Vec<i32>,
}

/// Where the zone came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneSource {
    System,
    Fallback,
    /// Neither a system nor a fallback file: UTC.
    Utc,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TzError {
    #[error("not a TZif file or the file is truncated")]
    Format,
}

/// Slice `bytes[start..start + len]` or a format error (no panic on a truncated file).
fn take(bytes: &[u8], start: usize, len: usize) -> Result<&[u8], TzError> {
    start
        .checked_add(len)
        .and_then(|end| bytes.get(start..end))
        .ok_or(TzError::Format)
}

fn be32(bytes: &[u8]) -> Result<u32, TzError> {
    let b: [u8; 4] = bytes.try_into().map_err(|_| TzError::Format)?;
    Ok(u32::from_be_bytes(b))
}

struct Counts {
    isut: usize,
    isstd: usize,
    leap: usize,
    time: usize,
    typ: usize,
    chars: usize,
}

fn counts(bytes: &[u8], at: usize) -> Result<Counts, TzError> {
    if take(bytes, at, 4)? != b"TZif" {
        return Err(TzError::Format);
    }
    let field = |i: usize| -> Result<usize, TzError> {
        usize::try_from(be32(take(bytes, at + 20 + 4 * i, 4)?)?).map_err(|_| TzError::Format)
    };
    Ok(Counts {
        isut: field(0)?,
        isstd: field(1)?,
        leap: field(2)?,
        time: field(3)?,
        typ: field(4)?,
        chars: field(5)?,
    })
}

impl Zone {
    pub fn fixed(offset_secs: i32) -> Zone {
        Zone {
            transitions: Vec::new(),
            offsets: vec![offset_secs],
        }
    }

    /// TZif parsing: version 1 from the 32-bit data, versions 2 and newer from the 64-bit block.
    pub fn parse_tzif(bytes: &[u8]) -> Result<Zone, TzError> {
        let first = counts(bytes, 0)?;
        let version = *bytes.get(4).ok_or(TzError::Format)?;
        let v1_len = first.time * 5
            + first.typ * 6
            + first.chars
            + first.leap * 8
            + first.isstd
            + first.isut;
        if version == 0 {
            return Zone::block(bytes, 44, &first, 4);
        }
        let second_at = 44 + v1_len;
        let second = counts(bytes, second_at)?;
        Zone::block(bytes, second_at + 44, &second, 8)
    }

    fn block(bytes: &[u8], at: usize, c: &Counts, time_size: usize) -> Result<Zone, TzError> {
        if c.typ == 0 {
            return Err(TzError::Format);
        }
        let times = take(bytes, at, c.time * time_size)?;
        let indexes = take(bytes, at + c.time * time_size, c.time)?;
        let types_at = at + c.time * (time_size + 1);
        let types = take(bytes, types_at, c.typ * 6)?;
        take(bytes, types_at + c.typ * 6, c.chars)?;
        let mut transitions = Vec::with_capacity(c.time);
        for (i, chunk) in times.chunks_exact(time_size).enumerate() {
            let t = if time_size == 8 {
                i64::from_be_bytes(chunk.try_into().map_err(|_| TzError::Format)?)
            } else {
                i64::from(i32::from_be_bytes(
                    chunk.try_into().map_err(|_| TzError::Format)?,
                ))
            };
            let idx = usize::from(indexes[i]);
            if idx >= c.typ {
                return Err(TzError::Format);
            }
            transitions.push((t, idx));
        }
        let offsets = types
            .as_chunks::<6>()
            .0
            .iter()
            .map(|t| be32(&t[..4]).map(|v| v as i32))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Zone {
            transitions,
            offsets,
        })
    }

    /// UTC offset in seconds at the moment `unix`.
    pub fn offset_at(&self, unix: i64) -> i32 {
        let after = self.transitions.partition_point(|(t, _)| *t <= unix);
        let index = match after.checked_sub(1) {
            Some(i) => self.transitions[i].1,
            None => 0,
        };
        self.offsets.get(index).copied().unwrap_or(0)
    }
}

/// A zone name is safe as a path inside zoneinfo: no `..`, no absolute path and no odd characters.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'+'))
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn read_zone(path: &Path) -> Option<Zone> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return None;
    }
    Zone::parse_tzif(&std::fs::read(path).ok()?).ok()
}

fn load(name: &str, dir: &Path) -> Option<Zone> {
    valid_name(name).then(|| read_zone(&dir.join(name)))?
}

/// System zone by name from `zoneinfo_dir`; no name or file - `Asia/Jerusalem`; not even that - UTC.
pub fn resolve(system_name: Option<&str>, zoneinfo_dir: &Path) -> (Zone, ZoneSource) {
    if let Some(zone) = system_name.and_then(|n| load(n, zoneinfo_dir)) {
        return (zone, ZoneSource::System);
    }
    match load(FALLBACK_ZONE, zoneinfo_dir) {
        Some(zone) => (zone, ZoneSource::Fallback),
        None => (Zone::fixed(0), ZoneSource::Utc),
    }
}

/// This machine's zone name: `TZ`, then `/etc/localtime` (a link into zoneinfo), then `/etc/timezone`.
fn system_zone_name() -> Option<String> {
    let from_env = std::env::var("TZ")
        .ok()
        .map(|v| v.trim_start_matches(':').to_string());
    let from_link = || {
        let target = std::fs::read_link("/etc/localtime").ok()?;
        let text = target.to_string_lossy().into_owned();
        text.rsplit_once("zoneinfo/")
            .map(|(_, name)| name.to_string())
    };
    let from_file = || {
        let text = std::fs::read_to_string("/etc/timezone").ok()?;
        text.lines().next().map(|l| l.trim().to_string())
    };
    from_env
        .filter(|n| !n.is_empty())
        .or_else(from_link)
        .or_else(from_file)
}

/// This machine's zone for the bar clock.
pub fn system_zone() -> (Zone, ZoneSource) {
    let dir = Path::new(ZONEINFO);
    let name = system_zone_name();
    if name.is_none()
        && let Some(zone) = read_zone(Path::new("/etc/localtime"))
    {
        return (zone, ZoneSource::System);
    }
    resolve(name.as_deref(), dir)
}
