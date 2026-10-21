//! SQLite storage: connection setup, versioned migrations, and small helpers.
//!
//! The database lives in the per-user app-data folder (on Windows:
//! `%LOCALAPPDATA%\com.kept.agent\kept.db`). It is protected by the Windows
//! user-account ACLs. OAuth tokens and API keys are *never* stored here — they
//! go to the OS credential store (Windows Credential Manager).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

/// Ordered list of migrations. Never edit a shipped migration; append a new one.
pub const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_initial", include_str!("../migrations/0001_initial.sql")),
    ("0002_more_connectors", include_str!("../migrations/0002_more_connectors.sql")),
];

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    // WAL gives crash-safety and lets the UI read while the agent writes.
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA secure_delete = ON;",
    )?;
    Ok(())
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            name TEXT PRIMARY KEY, applied_at TEXT NOT NULL)",
    )?;
    for (name, sql) in MIGRATIONS {
        let done: Option<String> = conn
            .query_row("SELECT name FROM schema_migrations WHERE name = ?1", [name], |r| r.get(0))
            .optional()?;
        if done.is_none() {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql).with_context(|| format!("migration {name}"))?;
            tx.execute(
                "INSERT INTO schema_migrations(name, applied_at) VALUES (?1, ?2)",
                params![name, Utc::now().to_rfc3339()],
            )?;
            tx.commit()?;
            tracing::info!("applied migration {name}");
        }
    }
    Ok(())
}

/// Copy the live database to `dest` using SQLite's online backup (safe while running).
pub fn backup_to(conn: &Connection, dest: &Path) -> Result<()> {
    let mut out = Connection::open(dest)?;
    let backup = rusqlite::backup::Backup::new(conn, &mut out)?;
    backup.run_to_completion(256, std::time::Duration::from_millis(5), None)?;
    Ok(())
}

/// Run `f` atomically using a SAVEPOINT, so it nests safely inside an outer transaction.
pub fn atomic<T>(conn: &Connection, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT kept_sp")?;
    match f(conn) {
        Ok(v) => {
            conn.execute_batch("RELEASE kept_sp")?;
            Ok(v)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO kept_sp; RELEASE kept_sp");
            Err(e)
        }
    }
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn now() -> DateTime<Utc> {
    Utc::now()
}

pub fn ts(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

pub fn opt_ts(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|s| parse_ts(&s))
}

/// Record an audit event (status transitions, user corrections, merges, deletions).
pub fn audit(
    conn: &Connection,
    entity_type: &str,
    entity_id: &str,
    action: &str,
    from: Option<&str>,
    to: Option<&str>,
    actor: &str,
    reason: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO audit_events(id, entity_type, entity_id, action, from_value, to_value, actor, reason, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![new_id(), entity_type, entity_id, action, from, to, actor, reason, ts(&now())],
    )?;
    Ok(())
}
