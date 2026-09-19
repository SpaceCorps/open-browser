//! One error type for the whole crate.
//!
//! The variants exist where a caller might reasonably do something different: a missing browser is
//! fixable by installing one, an unknown action by reading a list, a timeout by waiting longer.
//! Everything else collapses into [`Error::Browser`] or [`Error::Io`], both of which carry the
//! context needed to say which operation failed.

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Chrome or Chromium found. Set $OPEN_BROWSER_CHROME to its path, or install one:\n  {hint}")]
    NoBrowser { hint: &'static str },

    #[error("unknown action '{id}'. Known actions: {known}")]
    UnknownAction { id: String, known: String },

    #[error("{action} needs --{parameter}")]
    MissingParameter { action: String, parameter: String },

    #[error("{action}: --{parameter} is wrong: {reason}")]
    BadParameter { action: String, parameter: String, reason: String },

    #[error("no element matches '{selector}'")]
    NoSuchElement { selector: String },

    #[error("timed out after {seconds}s waiting for {what}")]
    Timeout { what: String, seconds: u64 },

    #[error("session '{name}' is not open. Start it with `ob session start {name}`")]
    NoSuchSession { name: String },

    #[error("session '{name}' is already running (pid {pid})")]
    SessionRunning { name: String, pid: u32 },

    #[error("browser: {context}: {source}")]
    Browser {
        context: String,
        #[source]
        source: chromiumoxide::error::CdpError,
    },

    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{context}: {source}")]
    Database {
        context: String,
        #[source]
        source: rusqlite::Error,
    },

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other(message: impl Into<String>) -> Self {
        Error::Other(message.into())
    }
}

/// Attach context to a `std::io` result.
pub fn io<T>(result: std::io::Result<T>, context: impl FnOnce() -> String) -> Result<T> {
    result.map_err(|source| Error::Io { context: context(), source })
}

/// Attach context to a CDP result.
pub fn cdp<T>(
    result: std::result::Result<T, chromiumoxide::error::CdpError>,
    context: impl FnOnce() -> String,
) -> Result<T> {
    result.map_err(|source| Error::Browser { context: context(), source })
}

pub fn db<T>(
    result: std::result::Result<T, rusqlite::Error>,
    context: impl FnOnce() -> String,
) -> Result<T> {
    result.map_err(|source| Error::Database { context: context(), source })
}

impl From<PathBuf> for Error {
    fn from(path: PathBuf) -> Self {
        Error::Other(format!("bad path: {}", path.display()))
    }
}
