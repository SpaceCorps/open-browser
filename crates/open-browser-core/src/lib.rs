//! Core of `open-browser`: drive a real browser from a declared set of actions.
//!
//! * [`actions`] is the registry — one declaration per capability, from which the CLI's
//!   subcommands, the HTTP API's routes and the agent's tool listing are all derived.
//! * [`engine`] launches Chrome over CDP and executes those actions against a page.
//! * [`session`] keeps a browser alive between invocations, so consecutive commands share cookies,
//!   logins and history.
//! * [`automation`] is a list of actions saved as a file, runnable by name.
//! * [`runs`] records what happened.

pub mod actions;
pub mod agents;
pub mod automation;
pub mod config;
pub mod engine;
pub mod error;
pub mod home;
pub mod runs;
pub mod session;

pub use error::{Error, Result};

/// The name of the shipped binary. Interpolated into the agent-facing tool listing, so a rename is
/// one edit rather than a find-and-replace through prompt text.
pub const CLI_NAME: &str = "ob";

pub const HOME_ENV: &str = "OPEN_BROWSER_HOME";

/// Points at the Chrome/Chromium binary to drive, overriding detection.
pub const CHROME_ENV: &str = "OPEN_BROWSER_CHROME";
