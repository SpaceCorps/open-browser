//! Inspecting and cancelling runs.

use anyhow::{Context as _, Result};
use open_browser_core::runs::{RunStatus, RunStore};
use open_browser_core::session::terminate;

use crate::cli::RunsCommand;
use crate::context::Context;

pub fn execute(context: &Context, command: RunsCommand) -> Result<()> {
    let store = context.runs()?;
    match command {
        RunsCommand::List { all, limit } => {
            let session = (!all).then_some(context.session.as_str());
            let runs = store.list(session, limit)?;
            context.emit(&runs, || {
                if runs.is_empty() {
                    return "nothing has run yet".into();
                }
                runs.iter()
                    .map(|run| {
                        format!(
                            "{}  {:<10} {:<10} {:<14} {}",
                            run.id,
                            run.kind.as_str(),
                            run.status.as_str(),
                            run.session,
                            truncate(&run.subject, 60)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        }
        RunsCommand::Show { id } => {
            let run = get(&store, &id)?;
            context.emit(&run, || {
                let mut lines = vec![
                    format!("{}  {}  {}", run.id, run.kind.as_str(), run.status),
                    format!("session: {}", run.session),
                    format!("subject: {}", run.subject),
                    format!("started: {}", run.created_at),
                ];
                if let Some(finished) = &run.finished_at {
                    lines.push(format!("finished: {finished}"));
                }
                if let Some(error) = &run.error {
                    lines.push(format!("error: {error}"));
                }
                if let Some(result) = &run.result {
                    lines.push(format!("result: {result}"));
                }
                lines.join("\n")
            })
        }
        RunsCommand::Cancel { id } => {
            let run = get(&store, &id)?;
            if run.status != RunStatus::Running {
                anyhow::bail!("run {id} already {}", run.status);
            }
            // Claim the row *before* killing anything. The process supervising the run notices the
            // death immediately and calls `finish` too; whichever claims first owns the verdict,
            // and killing first would record this cancellation as a plain failure.
            let claimed = store.finish(&run.id, RunStatus::Cancelled, None, Some("cancelled"))?;
            if !claimed {
                let current = store.get(&run.id)?.map(|run| run.status);
                anyhow::bail!(
                    "run {id} finished as {} while it was being cancelled",
                    current.map(|status| status.to_string()).unwrap_or_else(|| "unknown".into())
                );
            }
            if let Some(pid) = run.pid {
                terminate(pid);
            }
            context.emit(&run.id, || format!("cancelled run {}", run.id))
        }
        RunsCommand::Log { id } => {
            let run = get(&store, &id)?;
            let path = context.home.session_dir(&run.session).join(format!("agent-{}.log", run.id));
            match std::fs::read_to_string(&path) {
                Ok(text) => context.emit(&text, || text.clone()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    anyhow::bail!("run {id} has no log; only agent runs write one")
                }
                Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
            }
        }
    }
}

fn get(store: &RunStore, id: &str) -> Result<open_browser_core::runs::RunRecord> {
    // Ids are zero-padded to five digits, so `ob runs show 7` should find run 00007 rather than
    // telling someone their run does not exist.
    let padded = match id.parse::<u64>() {
        Ok(number) => format!("{number:05}"),
        Err(_) => id.to_string(),
    };
    store
        .get(&padded)?
        .ok_or_else(|| anyhow::anyhow!("no run called {id}. `ob runs list` shows the recent ones"))
}

fn truncate(text: &str, width: usize) -> String {
    let flat = text.replace('\n', " ");
    if flat.chars().count() <= width {
        return flat;
    }
    // Truncating by chars rather than bytes: a task written in any non-ASCII language would panic
    // on a byte slice that lands mid-codepoint.
    flat.chars().take(width - 1).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multibyte_subject_is_truncated_without_panicking() {
        let subject = "найти последнее письмо от бухгалтерии и скачать вложение как PDF немедленно";
        assert!(truncate(subject, 20).chars().count() <= 20);
    }
}
