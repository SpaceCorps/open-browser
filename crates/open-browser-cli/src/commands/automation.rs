//! Saved automations: a list of steps that are the same actions the CLI takes.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use open_browser_core::automation::{self, Automation};
use open_browser_core::engine::{execute, ActionOutcome, BrowserHandle};
use open_browser_core::runs::{RunKind, RunStatus};
use serde::Serialize;

use crate::cli::{AutomationCommand, AutomationRunArgs};
use crate::context::{key_values, Context};

const STARTER: &str = "\
# Steps are the same actions `ob` takes, by their ids. `ob actions list` shows them all.
# {{placeholders}} are filled from `inputs` below, or from `--input name=value` at run time.
name: NAME
description: What this does
inputs:
  query: rust cdp
steps:
  - action: goto
    url: duckduckgo.com
  - action: type
    selector: input[name=q]
    text: '{{query}}'
    enter: 'true'
  - action: wait-for
    selector: '[data-testid=result]'
  - action: text
    selector: '[data-testid=result]'
";

pub async fn execute_command(context: &Context, command: AutomationCommand) -> Result<()> {
    match command {
        AutomationCommand::List => {
            let names = automation::list(&context.home.automations_dir())?;
            context.emit(&names, || {
                if names.is_empty() {
                    format!(
                        "no automations yet. `ob automation new <name>` writes one into {}",
                        context.home.automations_dir().display()
                    )
                } else {
                    names.join("\n")
                }
            })
        }
        AutomationCommand::Show { name } => {
            let automation = load(context, &name)?;
            context.emit(&automation, || {
                let mut lines = vec![automation.name.clone()];
                if let Some(description) = &automation.description {
                    lines.push(description.clone());
                }
                let needed = automation.required_inputs();
                if !needed.is_empty() {
                    lines.push(format!("needs: {}", needed.join(", ")));
                }
                for (index, step) in automation.steps.iter().enumerate() {
                    let params = step
                        .params
                        .iter()
                        .map(|(name, value)| format!("{name}={value}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    lines.push(format!(
                        "  {:>2}. {} {}{}",
                        index + 1,
                        step.action,
                        params,
                        if step.optional { "  (optional)" } else { "" }
                    ));
                }
                lines.join("\n")
            })
        }
        AutomationCommand::New { name, force } => {
            context.home.ensure()?;
            let path = context.home.automation_path(&name);
            if path.exists() && !force {
                anyhow::bail!("{} already exists. Pass --force to overwrite it", path.display());
            }
            std::fs::write(&path, STARTER.replace("name: NAME", &format!("name: {name}")))
                .with_context(|| format!("writing {}", path.display()))?;
            context.emit(&path, || format!("wrote {}", path.display()))
        }
        AutomationCommand::Run(args) => run(context, args).await,
    }
}

/// The result of one automation run: every step's outcome, in order.
#[derive(Debug, Serialize)]
pub struct AutomationOutcome {
    pub automation: String,
    pub steps: Vec<StepOutcome>,
    /// Whether every non-optional step succeeded.
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub struct StepOutcome {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ActionOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// A failed optional step is reported rather than hidden; it just does not stop the run.
    pub skipped: bool,
}

async fn run(context: &Context, args: AutomationRunArgs) -> Result<()> {
    let automation = load(context, &args.name)?;
    let overrides = key_values(&args.inputs)?;

    // Planning parses every step before any of them runs. A twenty-step automation that dies on
    // step nineteen because of a typo has already sent nineteen steps' worth of side effects.
    let planned = automation.plan(&overrides)?;

    if args.dry_run {
        let plan: Vec<String> = planned
            .iter()
            .map(|step| {
                format!("{}{}", step.action.id(), if step.optional { " (optional)" } else { "" })
            })
            .collect();
        return context.emit(&plan, || {
            format!("{} step(s), all valid:\n  {}", plan.len(), plan.join("\n  "))
        });
    }

    let session = context.require_session().await?;
    let store = context.runs()?;
    let record = store.start(RunKind::Automation, &context.session, &automation.name)?;

    let browser = BrowserHandle::connect(&session.endpoint).await?;
    let page = browser.page(session.target.as_deref()).await?;
    let artifacts = context.artifacts()?;

    let mut steps = Vec::new();
    let mut ok = true;
    for planned in &planned {
        let id = planned.action.id().to_string();
        match execute(&page, &planned.action, &artifacts).await {
            Ok(outcome) => {
                context.note(format!("  {id}: {}", outcome.summary));
                steps.push(StepOutcome {
                    action: id,
                    outcome: Some(outcome),
                    error: None,
                    skipped: false,
                });
            }
            Err(error) if planned.optional => {
                context.note(format!("  {id}: skipped ({error})"));
                steps.push(StepOutcome {
                    action: id,
                    outcome: None,
                    error: Some(error.to_string()),
                    skipped: true,
                });
            }
            Err(error) => {
                steps.push(StepOutcome {
                    action: id,
                    outcome: None,
                    error: Some(error.to_string()),
                    skipped: false,
                });
                ok = false;
                break;
            }
        }
    }
    drop(browser);

    let outcome = AutomationOutcome { automation: automation.name.clone(), steps, ok };
    let body = serde_json::to_string(&outcome).ok();
    let status = if ok { RunStatus::Succeeded } else { RunStatus::Failed };
    let error = outcome.steps.last().filter(|_| !ok).and_then(|step| step.error.clone());
    store.finish(&record.id, status, body.as_deref(), error.as_deref())?;

    context.emit(&outcome, || {
        let last = outcome
            .steps
            .iter()
            .rev()
            .find_map(|step| step.outcome.as_ref())
            .map(|outcome| outcome.summary.clone())
            .unwrap_or_default();
        if ok {
            format!("{} finished: {last}", outcome.automation)
        } else {
            format!("{} failed at step {}", outcome.automation, outcome.steps.len())
        }
    })?;

    if !ok {
        // A failed automation must exit non-zero, or a shell script driving `ob` — or an agent
        // checking `$?` — reads a partial run as a completed one.
        std::process::exit(1);
    }
    Ok(())
}

/// Load by saved name, or by path when the argument looks like one.
fn load(context: &Context, name: &str) -> Result<Automation> {
    let path = if name.ends_with(".yaml") || name.ends_with(".yml") || name.contains('/') {
        PathBuf::from(name)
    } else {
        context.home.automation_path(name)
    };
    Ok(Automation::load(&path)?)
}
