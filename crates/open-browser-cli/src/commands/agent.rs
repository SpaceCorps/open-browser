//! Handing a task to open-agents, which drives this browser through these same commands.
//!
//! Nothing here talks to a model. It launches `open-agents run` with a promptware and an
//! environment pointing at a session, and the agent browses by shelling out to `ob`. That is the
//! whole integration, and it is why an action with no CLI command is an action no agent can
//! perform.

use anyhow::{Context as _, Result};
use open_browser_core::agents::{AgentRunner, AgentTask};
use open_browser_core::runs::{RunKind, RunStatus};
use serde::Serialize;

use crate::cli::{AgentCommand, AgentRunArgs};
use crate::context::Context;
use crate::promptware;

pub async fn execute(context: &Context, command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::Run(args) => run(context, args).await,
        AgentCommand::Prompt(args) => {
            let task = task_from(context, &args, context.session.clone())?;
            let runner = runner(context);
            let prompt = runner.prompt(&task);
            context.emit(&prompt, || prompt.clone())
        }
        AgentCommand::Install { force } => install(context, force),
    }
}

#[derive(Debug, Serialize)]
struct InstalledPromptware {
    promptware: String,
    path: String,
    created: bool,
}

/// Put the bundled promptware where open-agents will look for it.
///
/// Separate from `run` so the program can be installed and then edited before the first run, and
/// so `--force` has somewhere to live: the automatic install can never overwrite, because doing so
/// silently mid-run would discard the user's tuning at the worst possible moment.
fn install(context: &Context, force: bool) -> Result<()> {
    let home = promptware::agents_home();
    let path = promptware::program_path(&home);
    if force && path.is_file() {
        std::fs::remove_file(&path).with_context(|| format!("replacing {}", path.display()))?;
    }
    let outcome = promptware::install(&home)?;
    let created = outcome == promptware::Installed::Created;
    let report = InstalledPromptware {
        promptware: promptware::NAME.to_string(),
        path: path.display().to_string(),
        created,
    };
    context.emit(&report, || {
        if created {
            format!("wrote the {} promptware to {}", promptware::NAME, path.display())
        } else {
            format!(
                "{} already exists and was left alone; `ob agent install --force` replaces it",
                path.display()
            )
        }
    })
}

#[derive(Debug, Serialize)]
struct Launched {
    run: String,
    session: String,
    pid: u32,
}

#[derive(Debug, Serialize)]
struct Finished {
    run: String,
    session: String,
    status: RunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

async fn run(context: &Context, args: AgentRunArgs) -> Result<()> {
    let prose = args.task.join(" ");
    if prose.trim().is_empty() {
        anyhow::bail!(
            "give the agent something to do, e.g. `ob agent run 'find my latest invoice'`"
        );
    }
    if args.count == 0 {
        anyhow::bail!("--count must be at least 1");
    }

    let runner = runner(context);

    // The promptware is what gives the run memory, and open-agents resolves it under its own home
    // rather than this one. Installing here means the first `ob agent run` on a fresh machine
    // works; it never overwrites, so a program the user has edited is safe.
    if args.promptware.is_none() || args.promptware.as_deref() == Some(promptware::NAME) {
        match promptware::install(&promptware::agents_home()) {
            Ok(promptware::Installed::Created) => {
                context.note(format!("installed the {} promptware", promptware::NAME));
            }
            Ok(promptware::Installed::Kept) => {}
            // Not fatal: open-agents may have its home somewhere this process cannot write, and
            // the run still works if the promptware is already there by some other route.
            Err(error) => context.note(format!("could not install the promptware: {error}")),
        }
    }

    if args.dry_run {
        let task = task_from(context, &args, context.session.clone())?;
        let argv = runner.command_line(&task);
        return context.emit(&argv, || shell_words::join(&argv));
    }

    // One session per agent. Sharing a browser between agents means two of them typing into the
    // same field, so a fleet gets `work-1`, `work-2`, … each with its own profile and its own
    // logins. A fleet of one keeps the plain name, so the common case stays in the session the
    // person already logged in.
    let sessions: Vec<String> = if args.count == 1 {
        vec![context.session.clone()]
    } else {
        (1..=args.count).map(|index| format!("{}-{index}", context.session)).collect()
    };

    let store = context.runs()?;
    let mut launched = Vec::new();
    for session in sessions {
        let member = context.for_session(session.clone())?;
        // Each agent needs its browser up before it starts issuing commands; starting it here
        // rather than letting the agent's first `ob` call do it means a launch failure is reported
        // to the person now, not buried in an agent transcript.
        member.require_session().await?;

        let task = task_from(context, &args, session.clone())?;
        let record = store.start(RunKind::Agent, &session, &task.task)?;
        let log = context.home.session_dir(&session).join(format!("agent-{}.log", record.id));
        std::fs::create_dir_all(log.parent().unwrap()).ok();

        let child = runner.spawn(&task, Some(&log))?;
        let pid = child.id().unwrap_or(0);
        store.set_pid(&record.id, pid)?;
        context.note(format!(
            "run {} started in session '{session}' (pid {pid}); log: {}",
            record.id,
            log.display()
        ));
        launched.push((record.id, session, pid, child));
    }

    if args.detach {
        let summary: Vec<Launched> = launched
            .iter()
            .map(|(id, session, pid, _)| Launched {
                run: id.clone(),
                session: session.clone(),
                pid: *pid,
            })
            .collect();
        // The children are deliberately not awaited. `ob runs list` is how you find them again,
        // and `ob runs cancel` is how you stop one.
        for (_, _, _, child) in launched {
            std::mem::forget(child);
        }
        return context.emit(&summary, || {
            summary
                .iter()
                .map(|entry| {
                    format!("run {} in '{}' (pid {})", entry.run, entry.session, entry.pid)
                })
                .collect::<Vec<_>>()
                .join("\n")
        });
    }

    let mut results = Vec::new();
    for (id, session, _, mut child) in launched {
        let status = child.wait().await.with_context(|| format!("waiting for run {id}"))?;
        let (run_status, error) = if status.success() {
            (RunStatus::Succeeded, None)
        } else {
            (RunStatus::Failed, Some(format!("the agent exited with {status}")))
        };
        // `finish` only writes if the row is still running. A `runs cancel` in another terminal
        // claims it first, and its verdict is the true one — this process merely observed the
        // death it caused.
        let claimed = store.finish(&id, run_status, None, error.as_deref())?;
        let final_status = if claimed {
            run_status
        } else {
            store.get(&id)?.map(|record| record.status).unwrap_or(run_status)
        };
        results.push(Finished {
            run: id,
            session,
            status: final_status,
            error: if claimed { error } else { None },
        });
    }

    let ok = results.iter().all(|result| result.status == RunStatus::Succeeded);
    context.emit(&results, || {
        results
            .iter()
            .map(|result| {
                format!(
                    "run {} in '{}': {}{}",
                    result.run,
                    result.session,
                    result.status,
                    result.error.as_deref().map(|e| format!(" — {e}")).unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

fn runner(context: &Context) -> AgentRunner {
    AgentRunner::new(&context.config.open_agents_bin, context.home.path())
}

fn task_from(context: &Context, args: &AgentRunArgs, session: String) -> Result<AgentTask> {
    Ok(AgentTask {
        task: args.task.join(" "),
        session,
        promptware: args.promptware.clone().unwrap_or_else(|| context.config.promptware.clone()),
        provider: args.provider.clone(),
        model: args.model.clone(),
        effort: args.effort.clone(),
        timeout_seconds: args.timeout,
    })
}
