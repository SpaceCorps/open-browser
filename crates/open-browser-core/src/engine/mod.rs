//! Driving a real browser over the Chrome DevTools Protocol.

mod chrome;
mod execute;

pub use chrome::{locate_chrome, start_detached, BrowserHandle, LaunchOptions, INSTALL_HINT};
pub use execute::{execute, ActionOutcome};
