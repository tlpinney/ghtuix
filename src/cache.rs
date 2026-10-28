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
const FORMAT: i32 = 10;

pub struct Cache {
    db: Connection,
}

impl Cache {
    /// Opens (or creates) the cache at `path`, readable only by this user: it holds
    /// private PR pages, diffs and logs.
    pub fn open(path: &Path) -> Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        if let Some(dir) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        // Create the file first so it starts private; SQLite gives its WAL files the same mode.
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
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

    /// Deletes everything saved for `login` on `host` (when another account signs in).
    pub fn forget(&self, host: &str, login: &str) -> Result<()> {
        let prefix = format!("{host}/{login}/");
        // `substr` rather than LIKE, so `_` and `%` in names match only themselves.
        self.db.execute(
            "DELETE FROM entries WHERE substr(key, 1, length(?1)) = ?1",
            params![prefix],
        )?;
        Ok(())
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

/// Where the diff between two commits of `repo` is stored (it never changes).
pub fn diff_key(host: &str, login: &str, repo: &str, base: &str, head: &str) -> String {
    format!("{host}/{login}/diff/{repo}/{base}...{head}")
}

/// Where a finished Actions job's log is stored (it no longer changes).
pub fn log_key(host: &str, login: &str, repo: &str, job_id: i64) -> String {
    format!("{host}/{login}/log/{repo}/{job_id}")
}

/// Where a REST read's last body and `ETag` are kept, for conditional reads.
pub fn etag_key(host: &str, login: &str, what: &str) -> String {
    format!("{host}/{login}/etag/{what}")
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

    #[test]
    fn the_cache_is_private_and_an_old_account_can_be_forgotten() {
        use std::os::unix::fs::PermissionsExt;
        let (cache, path) = cache();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        cache.put(&inbox_key("h", "old_one"), &1).unwrap();
        cache.put(&inbox_key("h", "oldXone"), &2).unwrap();
        cache.put(&login_key("h"), &"old_one").unwrap();
        cache.forget("h", "old_one").unwrap();
        assert!(cache.get::<i32>(&inbox_key("h", "old_one")).is_none());
        assert!(
            cache.get::<i32>(&inbox_key("h", "oldXone")).is_some(),
            "_ is literal"
        );
        assert!(cache.get::<String>(&login_key("h")).is_some());
    }
}
