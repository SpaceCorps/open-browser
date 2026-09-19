//! The command tree.
//!
//! Only the management commands are declared here. The action commands — `goto`, `click`,
//! `screenshot` and the rest — are generated from the registry in [`crate::actions`] and grafted
//! on in [`build`], so this file has no list of them to fall out of date.

use std::path::PathBuf;

use clap::{Args, Command, Parser, Subcommand, ValueEnum};

pub const ABOUT: &str = "Drive a real browser from the shell, and let agents do the same";

const LONG_ABOUT: &str = "\
open-browser gives a live Chrome session a command per capability, so an agent that can run a \
shell can browse: `ob goto`, `ob click`, `ob text`, `ob screenshot`. A session is a persistent \
Chrome profile, so a login survives between commands. `ob agent run` hands a task to open-agents, \
which drives the browser through these same commands.";

#[derive(Debug, Parser)]
#[command(
    name = "ob",
    bin_name = "ob",
    version,
    about = ABOUT,
    long_about = LONG_ABOUT,
    propagate_version = true,
    // The action subcommands are added at runtime; without this, `ob` with no arguments would
    // print a help page missing every one of them.
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Override `$OPEN_BROWSER_HOME` (default `~/.open-browser`).
    #[arg(long, global = true, value_name = "DIR", env = "OPEN_BROWSER_HOME")]
    pub home: Option<PathBuf>,

    /// The session to act in. Defaults to `$OB_SESSION`, then the configured session.
    ///
    /// An agent launched by `ob agent run` inherits `$OB_SESSION`, which is what keeps a fleet of
    /// agents in their own browsers rather than fighting over one.
    #[arg(long, short, global = true, value_name = "NAME", env = "OB_SESSION")]
    pub session: Option<String>,

    /// Emit JSON instead of human-readable text.
    #[arg(long, global = true)]
    pub json: bool,

    /// Start the session if it is not already running, instead of failing.
    #[arg(long, global = true)]
    pub start: bool,

    /// Print more about what is happening. Repeat for debug tracing.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

/// The management commands, i.e. everything that is not an action.
#[derive(Debug, Subcommand)]
pub enum Manage {
    /// Start, list and stop browser sessions.
    #[command(subcommand)]
    Session(SessionCommand),

    /// Run and manage saved automations.
    #[command(subcommand, alias = "auto")]
    Automation(AutomationCommand),

    /// Hand a task to an agent, which drives the browser through these same commands.
    #[command(subcommand)]
    Agent(AgentCommand),

    /// Inspect what ran.
    #[command(subcommand, alias = "run")]
    Runs(RunsCommand),

    /// List the browser actions this build can perform.
    #[command(subcommand)]
    Actions(ActionsCommand),

    /// Serve the HTTP API and the web UI.
    Serve(ServeArgs),

    /// Read and write defaults in `config.yaml`.
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Check that Chrome, the home directory and open-agents are usable.
    Doctor,

    /// Print a shell completion script.
    Completions {
        #[arg(value_enum)]
        shell: CompletionShell,
    },
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Start a browser session, or report the one already running.
    Start(SessionStartArgs),
    /// List running sessions.
    #[command(alias = "ls")]
    List,
    /// Stop a session, leaving its profile (and its logins) on disk.
    Stop {
        /// Which session. Defaults to the global `--session`.
        name: Option<String>,
        /// Stop every running session.
        #[arg(long)]
        all: bool,
    },
    /// Delete a session's profile. This is what logs it out of everything.
    #[command(alias = "rm")]
    Remove {
        name: String,
        /// Skip the confirmation.
        #[arg(long, short)]
        force: bool,
    },
}

#[derive(Debug, Args)]
pub struct SessionStartArgs {
    /// Which session. Defaults to the global `--session`.
    pub name: Option<String>,
    /// Show the browser window. The way to log a session into something by hand.
    #[arg(long)]
    pub headed: bool,
    /// Keep the browser hidden, overriding a configured `headless: false`.
    #[arg(long, conflicts_with = "headed")]
    pub headless: bool,
    /// Window size, as WIDTHxHEIGHT.
    #[arg(long, value_name = "WxH")]
    pub window: Option<String>,
    /// Extra Chrome flags, repeatable.
    #[arg(long = "chrome-arg", value_name = "FLAG")]
    pub chrome_args: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum AutomationCommand {
    /// List saved automations.
    #[command(alias = "ls")]
    List,
    /// Show one automation's steps and the inputs it needs.
    Show { name: String },
    /// Write a starter automation to edit.
    New {
        name: String,
        /// Overwrite one that already exists.
        #[arg(long)]
        force: bool,
    },
    /// Run one.
    Run(AutomationRunArgs),
}

#[derive(Debug, Args)]
pub struct AutomationRunArgs {
    /// The saved automation's name, or a path to a `.yaml` file.
    pub name: String,
    /// Set an input, as `name=value`. Repeatable.
    #[arg(long = "input", short = 'i', value_name = "NAME=VALUE")]
    pub inputs: Vec<String>,
    /// Validate and print the steps without touching the browser.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Run one agent against this browser session.
    Run(AgentRunArgs),
    /// Print the prompt an agent would be given, including its tool listing.
    Prompt(AgentRunArgs),
    /// Write the bundled `Browser` promptware into open-agents' home.
    ///
    /// `ob agent run` does this for you. It is a command of its own so the program can be
    /// installed and then edited before the first run.
    Install {
        /// Overwrite an existing `Program.md`, discarding any edits made to it.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Args)]
pub struct AgentRunArgs {
    /// What the agent should accomplish, in prose.
    #[arg(value_name = "TASK")]
    pub task: Vec<String>,
    /// The promptware to wrap it in. Its memory is what makes repeated runs get better.
    #[arg(long, short = 'p', value_name = "NAME")]
    pub promptware: Option<String>,
    /// Which agent CLI to use, e.g. claude, codex, gemini.
    #[arg(long)]
    pub provider: Option<String>,
    #[arg(long)]
    pub model: Option<String>,
    /// Reasoning effort, passed through to the provider.
    #[arg(long)]
    pub effort: Option<String>,
    /// Give up after this many seconds.
    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<u64>,
    /// Run this many agents on the same task, each in its own session.
    ///
    /// The sessions are named `<session>-1`, `<session>-2` and so on, so a fleet does not fight
    /// over one browser.
    #[arg(long, short = 'n', default_value_t = 1, value_name = "N")]
    pub count: usize,
    /// Return as soon as the agents are launched, rather than waiting.
    #[arg(long)]
    pub detach: bool,
    /// Print the command that would run, and run nothing.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Subcommand)]
pub enum RunsCommand {
    /// List recent runs, newest first.
    #[command(alias = "ls")]
    List {
        /// Only this session's runs. Use `--all` for every session.
        #[arg(long)]
        all: bool,
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: usize,
    },
    /// Show one run, including its result.
    Show { id: String },
    /// Cancel a running run.
    Cancel { id: String },
    /// Print the agent log of a run.
    Log { id: String },
}

#[derive(Debug, Subcommand)]
pub enum ActionsCommand {
    /// List every action, with its example.
    #[command(alias = "ls")]
    List {
        /// Only this group, e.g. navigate, interact, read, capture, session.
        #[arg(long)]
        group: Option<String>,
    },
    /// Show one action's parameters.
    Show { id: String },
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Address to listen on. Defaults to the configured `bind`.
    #[arg(long, value_name = "ADDR")]
    pub bind: Option<String>,
    /// Serve this directory as the web UI.
    #[arg(long, value_name = "DIR")]
    pub ui: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the effective configuration.
    Show,
    /// Print the path to `config.yaml`.
    Path,
    /// Set a value, e.g. `ob config set headless false`.
    Set { key: String, value: String },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Elvish,
    #[value(name = "powershell")]
    PowerShell,
}

/// The full command tree: the declared management commands plus one per registered action.
///
/// Built by hand rather than by `Cli::command()` alone because the action subcommands do not exist
/// until the registry is walked. Everything downstream — `--help`, completions, parsing — uses
/// this, so the generated commands are first-class rather than a fallback path.
pub fn build() -> Command {
    let mut command = <Cli as clap::CommandFactory>::command();

    // Display order is set explicitly on both halves. Without it the two sets are each numbered
    // from zero and `--help` interleaves them — `goto` between `actions` and `serve` — which makes
    // a 30-command list unreadable. Actions come first because they are the point.
    for (index, action) in crate::actions::commands().into_iter().enumerate() {
        command = command.subcommand(action.display_order(index));
    }
    let offset = crate::actions::commands().len();
    for (index, manage) in
        <Manage as clap::Subcommand>::augment_subcommands(Command::new("__manage"))
            .get_subcommands()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
    {
        command = command.subcommand(manage.display_order(offset + index));
    }
    command
}
