//! Running one action against the session's browser.
//!
//! Every generated subcommand lands here. There is one code path from a command line to the
//! browser, and the HTTP API joins it at [`open_browser_core::actions::Action::parse`], so a
//! parameter cannot behave differently depending on which surface sent it.

use anyhow::Result;
use clap::ArgMatches;
use open_browser_core::actions::{Action, ActionSpec};
use open_browser_core::engine::{execute, ActionOutcome, BrowserHandle};
use open_browser_core::runs::{RunKind, RunStatus};

use crate::context::Context;

pub async fn run(context: &Context, spec: &'static ActionSpec, matches: &ArgMatches) -> Result<()> {
    let params = crate::actions::params_from(spec, matches);
    let action = Action::parse(spec.id, &params)?;
    let outcome = perform(context, &action).await?;

    context.emit(&outcome, || {
        // The human line is the summary; the value is for `--json`. Printing both would make the
        // common `ob text` case print the page twice.
        match &outcome.value {
            serde_json::Value::Null => outcome.summary.clone(),
            serde_json::Value::String(text) => text.clone(),
            other => {
                serde_json::to_string_pretty(other).unwrap_or_else(|_| outcome.summary.clone())
            }
        }
    })
}

/// Attach to the session, run the action, and record it.
///
/// Recorded even for a single hand-fired action, because the run log is what the web UI shows and
/// what makes an agent's browsing reviewable after the fact.
pub async fn perform(context: &Context, action: &Action) -> Result<ActionOutcome> {
    let session = context.require_session().await?;
    let store = context.runs()?;
    let record = store.start(RunKind::Action, &context.session, action.id())?;

    let outcome = attempt(context, &session, action).await;
    match &outcome {
        Ok(outcome) => {
            let result = serde_json::to_string(outcome).ok();
            store.finish(&record.id, RunStatus::Succeeded, result.as_deref(), None)?;
        }
        Err(error) => {
            store.finish(&record.id, RunStatus::Failed, None, Some(&error.to_string()))?;
        }
    }
    outcome
}

async fn attempt(
    context: &Context,
    session: &open_browser_core::session::SessionRecord,
    action: &Action,
) -> Result<ActionOutcome> {
    let browser = BrowserHandle::connect(&session.endpoint).await?;
    let page = browser.page(session.target.as_deref()).await?;
    let artifacts = context.artifacts()?;
    let outcome = execute(&page, action, &artifacts).await?;
    // Deliberately not `browser.close()`: closing would shut down the session everyone else is
    // attached to. Dropping the handle just ends this process's connection.
    drop(browser);
    Ok(outcome)
}
