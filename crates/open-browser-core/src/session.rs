//! Sessions: a browser that outlives the command that started it.
//!
//! The reason this exists rather than launching a browser per command: logging in is expensive and
//! often interactive, and an automation that has to re-authenticate on every step is not an
//! automation. A session is a named Chrome profile plus a long-lived process listening on a CDP
//! port; `ob goto`, `ob click` and `ob text` then attach to it in turn.
//!
//! The registry is a JSON file rather than a row in the database because it is read by every
//! command, including ones that never touch SQLite, and because a stale entry has to be detectable
//! from outside the process that wrote it — hence the pid and the liveness check.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{io, Error, Result};
use crate::home::{validate_session_name, Home};

/// What is recorded about a running session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub name: String,
    /// The CDP endpoint, e.g. `http://127.0.0.1:9222`. What a command attaches to.
    pub endpoint: String,
    pub pid: u32,
    /// The tab this session drives, as a CDP target id.
    ///
    /// Recorded rather than rediscovered because "the browser's first tab" is not a stable thing
    /// to ask for — see [`crate::engine::BrowserHandle::page`]. Optional so that a registry
    /// written by an older build still loads; a session without one falls back to whatever tab is
    /// there, which is right for a browser with only one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub profile: PathBuf,
    pub headless: bool,
    pub started_at: String,
}

/// The on-disk registry of open sessions.
pub struct SessionRegistry {
    path: PathBuf,
}

impl SessionRegistry {
    pub fn new(home: &Home) -> Self {
        Self { path: home.path().join("sessions.json") }
    }

    fn read(&self) -> Result<BTreeMap<String, SessionRecord>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| {
                Error::other(format!("{} is not readable as JSON: {error}", self.path.display()))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(source) => {
                Err(Error::Io { context: format!("reading {}", self.path.display()), source })
            }
        }
    }

    fn write(&self, sessions: &BTreeMap<String, SessionRecord>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            io(std::fs::create_dir_all(parent), || format!("creating {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(sessions)
            .map_err(|error| Error::other(format!("serialising the session registry: {error}")))?;
        io(std::fs::write(&self.path, text), || format!("writing {}", self.path.display()))
    }

    /// Every session whose process is still alive.
    ///
    /// Dead entries are dropped as a side effect: a browser that crashed, or a machine that
    /// rebooted, otherwise leaves an entry that every later command tries and fails to attach to.
    pub fn list(&self) -> Result<Vec<SessionRecord>> {
        let mut sessions = self.read()?;
        let before = sessions.len();
        sessions.retain(|_, record| is_alive(record.pid));
        if sessions.len() != before {
            self.write(&sessions)?;
        }
        Ok(sessions.into_values().collect())
    }

    pub fn get(&self, name: &str) -> Result<Option<SessionRecord>> {
        let name = validate_session_name(name)?;
        Ok(self.list()?.into_iter().find(|record| record.name == name))
    }

    /// Look one up, or explain how to start it.
    pub fn require(&self, name: &str) -> Result<SessionRecord> {
        self.get(name)?.ok_or_else(|| Error::NoSuchSession { name: name.to_string() })
    }

    pub fn insert(&self, record: SessionRecord) -> Result<()> {
        let mut sessions = self.read()?;
        if let Some(existing) = sessions.get(&record.name) {
            if is_alive(existing.pid) {
                return Err(Error::SessionRunning { name: record.name.clone(), pid: existing.pid });
            }
        }
        sessions.insert(record.name.clone(), record);
        self.write(&sessions)
    }

    pub fn remove(&self, name: &str) -> Result<Option<SessionRecord>> {
        let mut sessions = self.read()?;
        let removed = sessions.remove(name);
        if removed.is_some() {
            self.write(&sessions)?;
        }
        Ok(removed)
    }
}

/// Whether a pid names a live process.
///
/// Signal 0 performs the permission and existence checks without delivering anything, which is the
/// standard way to ask this question.
pub fn is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        unsafe { kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// Terminate a session's browser.
#[cfg(unix)]
pub fn terminate(pid: u32) -> bool {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    // SIGTERM, not SIGKILL: Chrome flushes cookies and the session history to the profile on a
    // clean shutdown, and a profile killed mid-write is how a logged-in session stops being one.
    unsafe { kill(pid as i32, 15) == 0 }
}

#[cfg(not(unix))]
pub fn terminate(pid: u32) -> bool {
    std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, pid: u32) -> SessionRecord {
        SessionRecord {
            name: name.to_string(),
            target: None,
            endpoint: "http://127.0.0.1:9222".into(),
            pid,
            profile: PathBuf::from("/tmp/profile"),
            headless: true,
            started_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn registry() -> (tempfile::TempDir, SessionRegistry) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::resolve(Some(dir.path())).unwrap();
        home.ensure().unwrap();
        let registry = SessionRegistry::new(&home);
        (dir, registry)
    }

    #[test]
    fn a_session_round_trips_and_an_empty_registry_is_not_an_error() {
        let (_dir, registry) = registry();
        assert!(registry.list().unwrap().is_empty());

        registry.insert(record("work", std::process::id())).unwrap();
        assert_eq!(registry.get("work").unwrap().unwrap().name, "work");
        assert!(registry.remove("work").unwrap().is_some());
        assert!(registry.get("work").unwrap().is_none());
    }

    #[test]
    fn a_dead_session_is_forgotten_rather_than_returned() {
        let (_dir, registry) = registry();
        // pid 1 is init; a pid that is certainly not a browser this test started. Using a very
        // high pid instead would be racy on a busy machine.
        registry.insert(record("stale", 0x7FFF_FFF0)).unwrap();
        assert!(registry.list().unwrap().is_empty());
        assert!(registry.get("stale").unwrap().is_none());
    }

    #[test]
    fn starting_a_session_that_is_already_running_is_refused_with_its_pid() {
        let (_dir, registry) = registry();
        registry.insert(record("work", std::process::id())).unwrap();
        let error = registry.insert(record("work", std::process::id())).unwrap_err();
        assert!(matches!(error, Error::SessionRunning { .. }), "{error}");
    }

    #[test]
    fn a_dead_entry_does_not_block_restarting_that_session() {
        let (_dir, registry) = registry();
        registry.insert(record("work", 0x7FFF_FFF0)).unwrap();
        registry.insert(record("work", std::process::id())).unwrap();
        assert_eq!(registry.get("work").unwrap().unwrap().pid, std::process::id());
    }

    #[test]
    fn requiring_a_missing_session_says_how_to_start_it() {
        let (_dir, registry) = registry();
        let error = registry.require("work").unwrap_err();
        assert!(error.to_string().contains("ob session start work"), "{error}");
    }
}
