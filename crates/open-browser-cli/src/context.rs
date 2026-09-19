//! What every command needs: the home, the config, the chosen session, and how to print.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use open_browser_core::config::Config;
use open_browser_core::home::{validate_session_name, Home};
use open_browser_core::runs::RunStore;
use open_browser_core::session::{SessionRecord, SessionRegistry};

use crate::cli::Cli;

pub struct Context {
    pub home: Home,
    pub config: Config,
    /// The session every command in this invocation acts in, already validated.
    pub session: String,
    pub json: bool,
    /// Start the session rather than failing when it is not running.
    pub start: bool,
}

impl Context {
    pub fn new(cli: &Cli) -> Result<Self> {
        let home = Home::resolve(cli.home.as_deref())?;
        let config = Config::load(&home.config_path())?;
        // Precedence: the flag (which clap has already filled from `$OB_SESSION`), then the
        // config. An agent inherits `$OB_SESSION` from its launcher, which is how a fleet member
        // stays in its own browser without being told again on every command it runs.
        let raw = cli.session.clone().unwrap_or_else(|| config.session.clone());
        let session = validate_session_name(&raw)?;
        Ok(Self { home, config, session, json: cli.json, start: cli.start })
    }

    /// A context aimed at a different session, for a fleet member.
    pub fn for_session(&self, session: String) -> Result<Self> {
        Ok(Self {
            home: self.home.clone(),
            config: self.config.clone(),
            session: validate_session_name(&session)?,
            json: self.json,
            start: self.start,
        })
    }

    pub fn sessions(&self) -> SessionRegistry {
        SessionRegistry::new(&self.home)
    }

    /// Open the run database, creating the home directory first.
    ///
    /// Opening it is a write, so this is also the point the home directory comes into being; a
    /// read-only command like `ob actions list` never calls it and leaves a fresh machine clean.
    pub fn runs(&self) -> Result<RunStore> {
        self.home.ensure()?;
        Ok(RunStore::open(&self.home.database_path())?)
    }

    /// Where captures and downloads land when no `--path` was given.
    pub fn artifacts(&self) -> Result<PathBuf> {
        let dir = self.home.session_dir(&self.session);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(dir)
    }

    /// The running session, started first if `--start` was given.
    pub async fn require_session(&self) -> Result<SessionRecord> {
        let registry = self.sessions();
        if let Some(record) = registry.get(&self.session)? {
            return Ok(record);
        }
        if !self.start {
            // The error names the command that fixes it, because "no such session" on its own
            // sends people looking for a config file that does not need editing.
            return Err(open_browser_core::Error::NoSuchSession { name: self.session.clone() })
                .with_context(|| {
                    format!("start it with `ob session start {}`, or pass --start", self.session)
                });
        }
        self.note(format!("starting session '{}'", self.session));
        crate::commands::session::start(self, None).await
    }

    /// Print `value` as JSON under `--json`, otherwise run `human`.
    pub fn emit<T, F>(&self, value: &T, human: F) -> Result<()>
    where
        T: serde::Serialize,
        F: FnOnce() -> String,
    {
        let mut out = std::io::stdout().lock();
        if self.json {
            serde_json::to_writer_pretty(&mut out, value)?;
            out.write_all(b"\n")?;
        } else {
            let text = human();
            if !text.is_empty() {
                out.write_all(text.as_bytes())?;
                if !text.ends_with('\n') {
                    out.write_all(b"\n")?;
                }
            }
        }
        out.flush()?;
        Ok(())
    }

    /// A status line for a human. Suppressed under `--json` so stdout stays parseable — an agent
    /// reads stdout, and a stray "starting session" line would break its parse.
    pub fn note(&self, message: impl AsRef<str>) {
        if !self.json {
            eprintln!("{}", message.as_ref());
        }
    }
}

/// Parse repeated `name=value` arguments.
pub fn key_values(raw: &[String]) -> Result<std::collections::BTreeMap<String, String>> {
    let mut map = std::collections::BTreeMap::new();
    for entry in raw {
        let (name, value) = entry
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("'{entry}' is not a name=value pair"))?;
        map.insert(name.trim().to_string(), value.to_string());
    }
    Ok(map)
}
