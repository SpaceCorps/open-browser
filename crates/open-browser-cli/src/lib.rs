//! `ob`: one command per browser action, plus the session and agent plumbing around them.
//!
//! The command tree is half declared and half generated. The management commands are an enum in
//! [`cli`]; the action commands are built from [`open_browser_core::actions::registry`] in
//! [`actions`]. [`dispatch`] is where the two halves meet: an action id routes to the one execution
//! path, anything else is parsed as a management command.

pub mod actions;
pub mod cli;
mod commands;
mod context;
pub mod promptware;

use anyhow::Result;
use clap::FromArgMatches;

use cli::{Cli, Manage};
use context::Context;

pub fn main() -> std::process::ExitCode {
    let matches = cli::build().get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    init_tracing(cli.verbose);

    // One runtime for everything: the browser connection is async, and building a runtime for
    // `ob config path` costs microseconds, which is cheaper than two code paths.
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("error: could not start the async runtime: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };

    match runtime.block_on(dispatch(&cli, &matches)) {
        Ok(code) => code,
        Err(error) => {
            // The chain matters: the bottom of it is usually the selector or the path that was
            // wrong, and the top is only ever "the action failed".
            eprintln!("error: {error}");
            for cause in error.chain().skip(1) {
                eprintln!("  caused by: {cause}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}

async fn dispatch(cli: &Cli, matches: &clap::ArgMatches) -> Result<std::process::ExitCode> {
    let (name, sub) = matches
        .subcommand()
        .ok_or_else(|| anyhow::anyhow!("no command given; `ob --help` lists them"))?;

    // An action id wins over a management command of the same name. Nothing collides today, and
    // `every_management_command_avoids_an_action_id` fails if that ever changes — an action losing
    // its command to a management subcommand is exactly the failure this project must not have.
    if let Some(spec) = open_browser_core::actions::find(name) {
        let context = Context::new(cli)?;
        commands::action::run(&context, spec, sub).await?;
        return Ok(std::process::ExitCode::SUCCESS);
    }

    let command = Manage::from_arg_matches(matches)?;
    let context = Context::new(cli)?;
    match command {
        Manage::Session(command) => commands::session::execute(&context, command).await?,
        Manage::Automation(command) => {
            commands::automation::execute_command(&context, command).await?
        }
        Manage::Agent(command) => commands::agent::execute(&context, command).await?,
        Manage::Runs(command) => commands::runs::execute(&context, command)?,
        Manage::Actions(command) => commands::actions::execute(&context, command)?,
        Manage::Serve(args) => commands::serve::execute(&context, args).await?,
        Manage::Config(command) => commands::config::execute(&context, command)?,
        Manage::Doctor => return commands::doctor::execute(&context),
        Manage::Completions { shell } => commands::completions::execute(shell),
    }
    Ok(std::process::ExitCode::SUCCESS)
}

/// Tracing goes to stderr so that `--json` on stdout stays machine-readable at any verbosity.
fn init_tracing(verbosity: u8) {
    let default = match verbosity {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_env("OPEN_BROWSER_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dispatcher checks the registry first, so a management command sharing a name with an
    /// action would make that action unreachable from the shell — and an action an agent cannot
    /// reach from the shell is one it cannot perform at all.
    #[test]
    fn every_management_command_avoids_an_action_id() {
        let manage = <Manage as clap::Subcommand>::augment_subcommands(clap::Command::new("x"));
        for command in manage.get_subcommands() {
            let names: Vec<&str> =
                std::iter::once(command.get_name()).chain(command.get_all_aliases()).collect();
            for name in names {
                assert!(
                    open_browser_core::actions::find(name).is_none(),
                    "the management command '{name}' shadows the action of the same name"
                );
            }
        }
    }

    #[test]
    fn the_built_tree_carries_every_action_and_every_management_command() {
        let root = cli::build();
        let names: Vec<&str> = root.get_subcommands().map(|c| c.get_name()).collect();
        for spec in open_browser_core::actions::registry() {
            assert!(names.contains(&spec.id), "`ob {}` is missing from the tree", spec.id);
        }
        for expected in
            ["session", "automation", "agent", "runs", "actions", "serve", "config", "doctor"]
        {
            assert!(names.contains(&expected), "`ob {expected}` is missing from the tree");
        }
    }

    /// `ob --session work goto x` and `ob goto x --session work` must mean the same thing, because
    /// an agent writes the second and a person writes the first.
    #[test]
    fn a_global_flag_is_accepted_before_or_after_the_subcommand() {
        for argv in [
            vec!["ob", "--session", "work", "goto", "example.com"],
            vec!["ob", "goto", "example.com", "--session", "work"],
        ] {
            let matches = cli::build().try_get_matches_from(&argv).unwrap();
            let cli = Cli::from_arg_matches(&matches).unwrap();
            assert_eq!(cli.session.as_deref(), Some("work"), "{argv:?}");
        }
    }
}
