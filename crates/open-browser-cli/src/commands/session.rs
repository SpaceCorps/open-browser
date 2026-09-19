//! Starting, listing and stopping browser sessions.

use anyhow::{Context as _, Result};
use open_browser_core::engine::{start_detached, BrowserHandle, LaunchOptions};
use open_browser_core::session::{is_alive, terminate, SessionRecord};

use crate::cli::{SessionCommand, SessionStartArgs};
use crate::context::Context;

pub async fn execute(context: &Context, command: SessionCommand) -> Result<()> {
    match command {
        SessionCommand::Start(args) => {
            let record = start(context, Some(&args)).await?;
            context.emit(&record, || {
                format!(
                    "session '{}' is up on {} (pid {}, {})",
                    record.name,
                    record.endpoint,
                    record.pid,
                    if record.headless { "headless" } else { "headed" }
                )
            })
        }
        SessionCommand::List => {
            let sessions = context.sessions().list()?;
            context.emit(&sessions, || {
                if sessions.is_empty() {
                    return "no sessions are running. `ob session start` opens one".into();
                }
                sessions
                    .iter()
                    .map(|record| {
                        format!(
                            "{:<16} {:<24} pid {:<8} {}",
                            record.name,
                            record.endpoint,
                            record.pid,
                            if record.headless { "headless" } else { "headed" }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        }
        SessionCommand::Stop { name, all } => {
            let registry = context.sessions();
            let targets = if all {
                registry.list()?
            } else {
                let name = name.unwrap_or_else(|| context.session.clone());
                registry.get(&name)?.into_iter().collect()
            };
            let mut stopped = Vec::new();
            for record in targets {
                terminate(record.pid);
                registry.remove(&record.name)?;
                stopped.push(record.name);
            }
            context.emit(&stopped, || match stopped.len() {
                0 => "nothing to stop".to_string(),
                _ => format!("stopped {}", stopped.join(", ")),
            })
        }
        SessionCommand::Remove { name, force } => {
            let registry = context.sessions();
            if let Some(record) = registry.get(&name)? {
                terminate(record.pid);
                registry.remove(&name)?;
            }
            let profile = context.home.profile_dir(&name);
            if !profile.exists() {
                return context.emit(&name, || format!("'{name}' has no profile on disk"));
            }
            if !force {
                // Deleting a profile logs the session out of everything it was signed into, and
                // that is not recoverable. Requiring --force is cheaper than the alternative.
                anyhow::bail!(
                    "deleting {} logs '{name}' out of everything it is signed into. \
                     Pass --force if that is what you want",
                    profile.display()
                );
            }
            std::fs::remove_dir_all(&profile)
                .with_context(|| format!("removing {}", profile.display()))?;
            context.emit(&name, || format!("removed the profile for '{name}'"))
        }
    }
}

/// Start the context's session and register it.
///
/// Shared with `Context::require_session`, so `--start` and `ob session start` cannot end up
/// launching browsers configured differently.
pub async fn start(context: &Context, args: Option<&SessionStartArgs>) -> Result<SessionRecord> {
    let name = args.and_then(|args| args.name.clone()).unwrap_or_else(|| context.session.clone());
    let name = open_browser_core::home::validate_session_name(&name)?;

    let registry = context.sessions();
    if let Some(existing) = registry.get(&name)? {
        // Already up is a success, not an error: `ob --start goto ...` twice in a row should work,
        // and so should two agents starting the same shared session at once.
        if is_alive(existing.pid) {
            return Ok(existing);
        }
    }

    context.home.ensure()?;
    let profile = context.home.profile_dir(&name);
    let mut options = LaunchOptions::new(&profile);
    options.headless = match args {
        Some(args) if args.headed => false,
        Some(args) if args.headless => true,
        _ => context.config.headless,
    };
    options.window = context.config.window;
    if let Some(raw) = args.and_then(|args| args.window.as_deref()) {
        options.window = parse_window(raw)?;
    }
    options.args = context.config.chrome_args.clone();
    if let Some(args) = args {
        options.args.extend(args.chrome_args.iter().cloned());
    }

    let (pid, endpoint) = start_detached(&options).await?;
    // Resolve and pin the tab now, while this is the only tab there is. Doing it later — from a
    // command that finds several — would be a guess.
    let target = match BrowserHandle::connect(&endpoint).await {
        Ok(browser) => {
            let target = browser.primary_target().await.ok();
            drop(browser);
            target
        }
        // A session whose tab could not be resolved is still usable; the next command falls back
        // to whatever tab is open, which in a browser with one tab is the right one anyway.
        Err(error) => {
            context.note(format!("could not pin the session's tab: {error}"));
            None
        }
    };
    let record = SessionRecord {
        name: name.clone(),
        endpoint,
        pid,
        target,
        profile,
        headless: options.headless,
        started_at: chrono::Utc::now().to_rfc3339(),
    };
    registry.insert(record.clone())?;
    Ok(record)
}

fn parse_window(raw: &str) -> Result<(u32, u32)> {
    let (width, height) = raw
        .split_once(['x', 'X'])
        .ok_or_else(|| anyhow::anyhow!("'{raw}' is not a size; write it as 1280x800"))?;
    Ok((
        width.trim().parse().with_context(|| format!("'{width}' is not a width"))?,
        height.trim().parse().with_context(|| format!("'{height}' is not a height"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_size_is_read_as_width_by_height_and_anything_else_is_refused() {
        assert_eq!(parse_window("1440x900").unwrap(), (1440, 900));
        assert_eq!(parse_window("1440X900").unwrap(), (1440, 900));
        assert!(parse_window("1440").is_err());
        assert!(parse_window("widexhigh").is_err());
    }
}
