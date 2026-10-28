//! A local SQLite cache (`~/.cache/ghtuix/cache.sqlite3`) of JSON values by key.
//!
//! Private GitHub data is stored under keys that start with its host and account, so a
//! different account never sees it.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

/// The shape of what the cache stores. Bump it whenever a cached type changes meaning
/// (a field added, a body read differently): older entries are then dropped on open.
const FORMAT: i32 = 6;

pub struct Cache {
    db: Connection,
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS entries (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL,
                 saved_at INTEGER NOT NULL
             );",
        )?;
        let format: i32 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if format != FORMAT {
            db.execute("DELETE FROM entries", [])?;
            db.execute_batch(&format!("PRAGMA user_version = {FORMAT};"))?;
        }
        Ok(Self { db })
    }

    /// The value saved under `key` and when it was saved (Unix seconds), if any and still
    /// readable as `T`.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<(T, i64)> {
        let row: Option<(String, i64)> = self
            .db
            .query_row(
                "SELECT value, saved_at FROM entries WHERE key = ?1",
                params![key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .ok()
            .flatten();
        let (value, saved_at) = row?;
        Some((serde_json::from_str(&value).ok()?, saved_at))
    }

    pub fn put<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.db.execute(
            "INSERT INTO entries (key, value, saved_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, saved_at = excluded.saved_at",
            params![key, serde_json::to_string(value)?, crate::time::now()],
        )?;
        Ok(())
    }
}

/// `$XDG_CACHE_HOME/ghtuix/cache.sqlite3`, else `~/.cache/ghtuix/cache.sqlite3`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("ghtuix").join("cache.sqlite3"))
}

/// Where an account's Inbox is stored.
pub fn inbox_key(host: &str, login: &str) -> String {
    format!("{host}/{login}/inbox")
}

/// Where an account's copy of a PR page is stored.
pub fn pr_key(host: &str, login: &str, repo: &str, number: i64) -> String {
    format!("{host}/{login}/pr/{repo}#{number}")
}

/// Where the diff between two commits is stored (it never changes).
pub fn diff_key(host: &str, login: &str, base: &str, head: &str) -> String {
    format!("{host}/{login}/diff/{base}..{head}")
}

/// The account last signed in on `host`.
pub fn login_key(host: &str) -> String {
    format!("{host}/login")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> (Cache, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ghtuix-cache-{}", std::process::id()));
        let path = dir.join(format!("{:?}.sqlite3", std::thread::current().id()));
        let _ = std::fs::remove_file(&path);
        (Cache::open(&path).unwrap(), path)
    }

    #[test]
    fn a_new_format_drops_older_entries() {
        let (cache, path) = cache();
        cache.put("k", &1).unwrap();
        cache.db.execute_batch("PRAGMA user_version = 1;").unwrap();
        drop(cache);
        let reopened = Cache::open(&path).unwrap();
        assert!(reopened.get::<i32>("k").is_none());
    }

    #[test]
    fn values_round_trip_and_overwrite() {
        let (cache, path) = cache();
        assert!(cache.get::<Vec<String>>("k").is_none());
        cache.put("k", &vec!["a".to_string()]).unwrap();
        cache.put("k", &vec!["b".to_string()]).unwrap();
        let (value, saved_at) = cache.get::<Vec<String>>("k").unwrap();
        assert_eq!(value, ["b"]);
        assert!(saved_at > 0);
        drop(cache);
        let reopened = Cache::open(&path).unwrap();
        assert!(
            reopened.get::<Vec<String>>("k").is_some(),
            "persists across opens"
        );
    }
}
