//! `ob doctor`: can this machine actually do anything?
//!
//! Every check names the fix rather than only the symptom. The common first-run failures are no
//! Chrome and no open-agents, and both are one command away once you know which one.

use std::process::ExitCode;

use anyhow::Result;
use open_browser_core::engine::{locate_chrome, INSTALL_HINT};
use serde::Serialize;

use crate::context::Context;

#[derive(Debug, Serialize)]
struct Check {
    name: String,
    ok: bool,
    detail: String,
}

pub fn execute(context: &Context) -> Result<ExitCode> {
    let mut checks = Vec::new();

    checks.push(match locate_chrome() {
        Some(path) => Check { name: "chrome".into(), ok: true, detail: path.display().to_string() },
        None => Check {
            name: "chrome".into(),
            ok: false,
            detail: format!("no browser found. Install one: {INSTALL_HINT}"),
        },
    });

    let home = context.home.path().to_path_buf();
    checks.push(match context.home.ensure() {
        Ok(()) => Check { name: "home".into(), ok: true, detail: home.display().to_string() },
        Err(error) => Check {
            name: "home".into(),
            ok: false,
            detail: format!("{} is not writable: {error}", home.display()),
        },
    });

    checks.push(match context.runs() {
        Ok(_) => Check {
            name: "database".into(),
            ok: true,
            detail: context.home.database_path().display().to_string(),
        },
        Err(error) => Check { name: "database".into(), ok: false, detail: error.to_string() },
    });

    let binary = &context.config.open_agents_bin;
    let agents = std::process::Command::new(binary)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();
    checks.push(match agents {
        Ok(output) if output.status.success() => Check {
            name: "open-agents".into(),
            ok: true,
            detail: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        },
        _ => Check {
            name: "open-agents".into(),
            // Not fatal: every action works without it. Only `ob agent run` needs it, so this is
            // a warning that names what is unavailable rather than a failure.
            ok: false,
            detail: format!(
                "`{binary}` did not run. `ob agent run` needs it; every other command works without it. \
                 Install: https://github.com/SpaceCorps/open-agents"
            ),
        },
    });

    // Reported next to open-agents because it is the other half of the same requirement: the
    // binary without the promptware gives you an agent with no memory and no idea it has a browser.
    let program = crate::promptware::program_path(&crate::promptware::agents_home());
    checks.push(Check {
        name: "promptware".into(),
        ok: true,
        detail: if program.is_file() {
            program.display().to_string()
        } else {
            format!("not installed; `ob agent install` writes it to {}", program.display())
        },
    });

    let sessions = context.sessions().list().map(|list| list.len()).unwrap_or(0);
    checks.push(Check { name: "sessions".into(), ok: true, detail: format!("{sessions} running") });

    checks.push(Check {
        name: "actions".into(),
        ok: true,
        detail: format!("{} available", open_browser_core::actions::registry().len()),
    });

    // Only the first three are required to browse at all; open-agents is optional.
    let fatal = checks.iter().filter(|check| !check.ok && check.name != "open-agents").count();

    context.emit(&checks, || {
        let width = checks.iter().map(|check| check.name.len()).max().unwrap_or(0);
        checks
            .iter()
            .map(|check| {
                format!(
                    "{} {:width$}  {}",
                    if check.ok { "ok  " } else { "warn" },
                    check.name,
                    check.detail
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    })?;

    Ok(if fatal == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}
