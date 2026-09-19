//! What ran, when, and what it produced.
//!
//! SQLite rather than a log file because the service, the desktop app and the CLI all read this
//! concurrently and the web UI wants to page through it; rather than a server database because the
//! whole thing has to work from `cargo install` with no daemon.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{db, Error, Result};

/// Bumped whenever [`migrate`] gains a step. Forward-only, numbered.
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Succeeded => "succeeded",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
        }
    }

    fn parse(raw: &str) -> Self {
        match raw {
            "running" => RunStatus::Running,
            "succeeded" => RunStatus::Succeeded,
            "cancelled" => RunStatus::Cancelled,
            _ => RunStatus::Failed,
        }
    }
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What kind of thing was run. Kept as a column so the UI can filter a fleet of agent runs from the
/// single actions a person fired by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunKind {
    Action,
    Automation,
    Agent,
}

impl RunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RunKind::Action => "action",
            RunKind::Automation => "automation",
            RunKind::Agent => "agent",
        }
    }

    fn parse(raw: &str) -> Self {
        match raw {
            "automation" => RunKind::Automation,
            "agent" => RunKind::Agent,
            _ => RunKind::Action,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    /// Zero-padded 5-digit id, so runs sort lexically as well as numerically.
    pub id: String,
    pub kind: RunKind,
    pub status: RunStatus,
    pub session: String,
    /// The action id, the automation name, or the agent's task. What the run was.
    pub subject: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    /// The outcome as JSON, once there is one.
    pub result: Option<String>,
    pub error: Option<String>,
    /// The agent process while it is running, so a run can be cancelled from another invocation.
    /// Cleared on finish; meaningless once `status` has left `running`.
    pub pid: Option<u32>,
}

pub struct RunStore {
    connection: Connection,
}

impl RunStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            crate::error::io(std::fs::create_dir_all(parent), || {
                format!("creating {}", parent.display())
            })?;
        }
        let connection = db(Connection::open(path), || format!("opening {}", path.display()))?;
        // Concurrent readers while a run is being written: the default rollback journal locks the
        // whole database for the duration of a write, which with a fleet of agents means the UI
        // spends its time waiting.
        db(connection.pragma_update(None, "journal_mode", "WAL"), || "enabling WAL".to_string())?;
        db(connection.pragma_update(None, "busy_timeout", 5000), || {
            "setting the busy timeout".to_string()
        })?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self> {
        let connection = db(Connection::open_in_memory(), || "opening memory".to_string())?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let current: i64 =
            db(self.connection.query_row("PRAGMA user_version", [], |row| row.get(0)), || {
                "reading the schema version".to_string()
            })?;
        if current >= SCHEMA_VERSION {
            return Ok(());
        }
        db(
            self.connection.execute_batch(
                "CREATE TABLE IF NOT EXISTS runs (
                    id           TEXT PRIMARY KEY,
                    kind         TEXT NOT NULL,
                    status       TEXT NOT NULL,
                    session      TEXT NOT NULL,
                    subject      TEXT NOT NULL,
                    created_at   TEXT NOT NULL,
                    finished_at  TEXT,
                    result       TEXT,
                    error        TEXT,
                    pid          INTEGER
                 );
                 CREATE INDEX IF NOT EXISTS runs_created ON runs (created_at DESC);
                 CREATE INDEX IF NOT EXISTS runs_session ON runs (session, created_at DESC);",
            ),
            || "creating the runs table".to_string(),
        )?;
        db(self.connection.pragma_update(None, "user_version", SCHEMA_VERSION), || {
            "recording the schema version".to_string()
        })
    }

    /// Start a run and return its record.
    ///
    /// The id is allocated inside the same transaction as the insert, so two processes starting a
    /// run at once cannot be handed the same one.
    pub fn start(&self, kind: RunKind, session: &str, subject: &str) -> Result<RunRecord> {
        let created_at = now();
        let id: String = db(
            self.connection.query_row(
                "INSERT INTO runs (id, kind, status, session, subject, created_at)
                 VALUES (
                    printf('%05d', COALESCE((SELECT CAST(MAX(id) AS INTEGER) FROM runs), 0) + 1),
                    ?1, ?2, ?3, ?4, ?5
                 )
                 RETURNING id",
                params![kind.as_str(), RunStatus::Running.as_str(), session, subject, created_at],
                |row| row.get(0),
            ),
            || "starting a run".to_string(),
        )?;
        Ok(RunRecord {
            id,
            kind,
            status: RunStatus::Running,
            session: session.to_string(),
            subject: subject.to_string(),
            created_at,
            finished_at: None,
            result: None,
            error: None,
            pid: None,
        })
    }

    pub fn set_pid(&self, id: &str, pid: u32) -> Result<()> {
        db(
            self.connection
                .execute("UPDATE runs SET pid = ?2 WHERE id = ?1", params![id, pid])
                .map(|_| ()),
            || format!("recording the pid of {id}"),
        )
    }

    /// Record a terminal status, and report whether this call was the one that did it.
    ///
    /// The `status = 'running'` guard is the whole point. A run can be finished from two places at
    /// once — the process supervising it, and a `runs cancel` in another terminal — and whichever
    /// claims the row first owns the verdict. A canceller must therefore claim *before* it kills,
    /// or the supervisor will notice the death and record a plain failure instead.
    pub fn finish(
        &self,
        id: &str,
        status: RunStatus,
        result: Option<&str>,
        error: Option<&str>,
    ) -> Result<bool> {
        let changed = db(
            self.connection.execute(
                "UPDATE runs
                    SET status = ?2, finished_at = ?3, result = ?4, error = ?5, pid = NULL
                  WHERE id = ?1 AND status = 'running'",
                params![id, status.as_str(), now(), result, error],
            ),
            || format!("finishing {id}"),
        )?;
        Ok(changed > 0)
    }

    pub fn get(&self, id: &str) -> Result<Option<RunRecord>> {
        db(
            self.connection
                .query_row(&format!("{SELECT} WHERE id = ?1"), params![id], read_row)
                .optional(),
            || format!("reading {id}"),
        )
    }

    /// The most recent runs, newest first.
    pub fn list(&self, session: Option<&str>, limit: usize) -> Result<Vec<RunRecord>> {
        // Owned values rather than `params![...]`: that macro borrows, and the bindings here would
        // be temporaries of the `match` arm they were built in.
        let limit = limit as i64;
        let (sql, bound): (String, Vec<Box<dyn rusqlite::ToSql>>) = match session {
            Some(session) => (
                format!("{SELECT} WHERE session = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2"),
                vec![Box::new(session.to_string()), Box::new(limit)],
            ),
            None => (
                format!("{SELECT} ORDER BY created_at DESC, id DESC LIMIT ?1"),
                vec![Box::new(limit)],
            ),
        };
        let mut statement = db(self.connection.prepare(&sql), || "listing runs".to_string())?;
        let rows =
            db(statement.query_map(rusqlite::params_from_iter(bound.iter()), read_row), || {
                "listing runs".to_string()
            })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|source| Error::Database { context: "listing runs".into(), source })
    }

    /// Runs still marked running. Used by `ob runs cancel --all` and by the service on startup.
    pub fn running(&self) -> Result<Vec<RunRecord>> {
        let mut statement = db(
            self.connection.prepare(&format!("{SELECT} WHERE status = 'running' ORDER BY id")),
            || "listing running runs".to_string(),
        )?;
        let rows = db(statement.query_map([], read_row), || "listing running runs".to_string())?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|source| Error::Database { context: "listing running runs".into(), source })
    }
}

const SELECT: &str = "SELECT id, kind, status, session, subject, created_at, finished_at, result, error, pid FROM runs";

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRecord> {
    Ok(RunRecord {
        id: row.get(0)?,
        kind: RunKind::parse(&row.get::<_, String>(1)?),
        status: RunStatus::parse(&row.get::<_, String>(2)?),
        session: row.get(3)?,
        subject: row.get(4)?,
        created_at: row.get(5)?,
        finished_at: row.get(6)?,
        result: row.get(7)?,
        error: row.get(8)?,
        pid: row.get::<_, Option<i64>>(9)?.map(|pid| pid as u32),
    })
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_allocated_in_order_and_sort_lexically() {
        let store = RunStore::in_memory().unwrap();
        let first = store.start(RunKind::Action, "default", "goto").unwrap();
        let second = store.start(RunKind::Action, "default", "click").unwrap();
        assert_eq!(first.id, "00001");
        assert_eq!(second.id, "00002");
        assert!(first.id < second.id);
    }

    #[test]
    fn a_run_is_finished_exactly_once_and_the_loser_is_told() {
        let store = RunStore::in_memory().unwrap();
        let run = store.start(RunKind::Agent, "work", "read my email").unwrap();

        assert!(store.finish(&run.id, RunStatus::Cancelled, None, None).unwrap());
        // The supervising process arriving late must not overwrite the cancellation.
        assert!(!store.finish(&run.id, RunStatus::Failed, None, Some("killed")).unwrap());

        let after = store.get(&run.id).unwrap().unwrap();
        assert_eq!(after.status, RunStatus::Cancelled);
        assert!(after.error.is_none());
    }

    #[test]
    fn finishing_clears_the_pid_so_a_stale_one_is_never_signalled() {
        let store = RunStore::in_memory().unwrap();
        let run = store.start(RunKind::Agent, "work", "x").unwrap();
        store.set_pid(&run.id, 4242).unwrap();
        assert_eq!(store.get(&run.id).unwrap().unwrap().pid, Some(4242));

        store.finish(&run.id, RunStatus::Succeeded, Some("{}"), None).unwrap();
        assert_eq!(store.get(&run.id).unwrap().unwrap().pid, None);
    }

    #[test]
    fn listing_is_newest_first_and_filters_by_session() {
        let store = RunStore::in_memory().unwrap();
        store.start(RunKind::Action, "a", "one").unwrap();
        store.start(RunKind::Action, "b", "two").unwrap();
        let third = store.start(RunKind::Action, "a", "three").unwrap();

        let all = store.list(None, 10).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, third.id);

        let scoped = store.list(Some("a"), 10).unwrap();
        assert_eq!(scoped.len(), 2);
        assert!(scoped.iter().all(|run| run.session == "a"));
    }

    #[test]
    fn only_unfinished_runs_are_reported_as_running() {
        let store = RunStore::in_memory().unwrap();
        let one = store.start(RunKind::Agent, "a", "one").unwrap();
        store.start(RunKind::Agent, "a", "two").unwrap();
        store.finish(&one.id, RunStatus::Succeeded, None, None).unwrap();

        let running = store.running().unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].subject, "two");
    }

    #[test]
    fn a_store_survives_being_reopened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/runs.sqlite3");
        let id = {
            let store = RunStore::open(&path).unwrap();
            store.start(RunKind::Action, "default", "goto").unwrap().id
        };
        let store = RunStore::open(&path).unwrap();
        assert_eq!(store.get(&id).unwrap().unwrap().subject, "goto");
    }
}
