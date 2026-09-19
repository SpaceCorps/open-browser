//! The bridge to open-agents.
//!
//! open-browser does not talk to a model. It hands a task to `open-agents run`, and the promptware
//! that run is wrapped in tells the agent to drive the browser by shelling out to `ob`. That is why
//! "every browser action has a CLI command" is load-bearing rather than a nicety: the CLI *is* the
//! agent's tool surface, so an action with no command is an action no agent can perform.
//!
//! The consequence for this module is that it is small. It builds an argv, sets the two environment
//! variables that pin the agent to one session, and reports what came back. Everything an agent can
//! do is already reachable without it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use crate::actions::ActionSpec;
use crate::error::{Error, Result};

/// Names the session an agent's `ob` invocations attach to. Set in the agent's environment so it
/// cannot wander into another session's logged-in profile by forgetting a flag.
pub const SESSION_ENV: &str = "OB_SESSION";

/// What to run and with what.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTask {
    /// What the agent should accomplish, in prose.
    pub task: String,
    pub session: String,
    /// The promptware to wrap it in. Its memory is what makes repeated runs get better.
    pub promptware: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

/// How to reach open-agents and what to tell it about this machine.
#[derive(Debug, Clone)]
pub struct AgentRunner {
    /// The `open-agents` binary. A bare name is looked up on `$PATH`.
    pub binary: PathBuf,
    /// The `ob` binary the agent is told to call. Resolved to this process's own path where
    /// possible, so an agent launched from a checkout does not drive a different installed build.
    pub cli: PathBuf,
    pub home: PathBuf,
}

impl AgentRunner {
    pub fn new(binary: impl Into<PathBuf>, home: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            cli: std::env::current_exe().unwrap_or_else(|_| PathBuf::from(crate::CLI_NAME)),
            home: home.into(),
        }
    }

    /// The argv, without running anything. `ob agent run --dry-run` prints this, and the tests
    /// assert on it, because a wrong flag here is otherwise only visible as an agent that ignores
    /// its promptware.
    pub fn command_line(&self, task: &AgentTask) -> Vec<String> {
        let mut argv = vec![
            self.binary.display().to_string(),
            "run".to_string(),
            format!("--promptware={}", task.promptware),
            "--json".to_string(),
        ];
        if let Some(provider) = &task.provider {
            argv.push(format!("--provider={provider}"));
        }
        if let Some(model) = &task.model {
            argv.push(format!("--model={model}"));
        }
        if let Some(effort) = &task.effort {
            argv.push(format!("--effort={effort}"));
        }
        if let Some(seconds) = task.timeout_seconds {
            argv.push(format!("--timeout={seconds}"));
        }
        // The prompt is positional and last, so a task beginning with a dash cannot be read as a
        // flag by whatever parses this downstream.
        argv.push("--".to_string());
        argv.push(self.prompt(task));
        argv
    }

    /// What the agent is actually asked to do: the task, plus how to reach the browser.
    ///
    /// The tool listing is generated from the registry rather than written here, so an action added
    /// to the registry is one an agent knows about on the next run with no edit to this text.
    pub fn prompt(&self, task: &AgentTask) -> String {
        format!(
            "{task}\n\n\
             You drive a real browser by running `{cli}` in the shell. The browser is already open \
             in session '{session}' and stays open between commands, so cookies and logins persist. \
             Every command takes --json; read that rather than the human summary.\n\n\
             Commands:\n{tools}\n\n\
             Run `{cli} actions show <ID>` for one action's parameters, and `{cli} text` to see \
             what is on the page before deciding what to click.",
            task = task.task,
            cli = self.cli.display(),
            session = task.session,
            tools = tool_listing(crate::actions::registry()),
        )
    }

    fn environment(&self, task: &AgentTask) -> BTreeMap<String, String> {
        BTreeMap::from([
            (SESSION_ENV.to_string(), task.session.clone()),
            (crate::HOME_ENV.to_string(), self.home.display().to_string()),
        ])
    }

    /// Launch the agent, returning the child.
    ///
    /// Not awaited here: the caller records the pid before waiting, so a run started in one
    /// terminal can be cancelled from another.
    pub fn spawn(&self, task: &AgentTask, log: Option<&Path>) -> Result<tokio::process::Child> {
        let argv = self.command_line(task);
        let mut command = tokio::process::Command::new(&argv[0]);
        command.args(&argv[1..]);
        for (key, value) in self.environment(task) {
            command.env(key, value);
        }
        command.stdin(Stdio::null());
        match log {
            Some(path) => {
                let file = std::fs::File::create(path).map_err(|source| Error::Io {
                    context: format!("creating {}", path.display()),
                    source,
                })?;
                let errors = file.try_clone().map_err(|source| Error::Io {
                    context: format!("opening {} for stderr", path.display()),
                    source,
                })?;
                command.stdout(file).stderr(errors);
            }
            None => {
                command.stdout(Stdio::piped()).stderr(Stdio::piped());
            }
        }
        #[cfg(unix)]
        {
            // Its own process group: the agent spawns `ob` children, and cancelling the run has to
            // reach them too or the browser keeps being driven after the run is marked cancelled.
            command.process_group(0);
        }
        command.spawn().map_err(|source| Error::Io {
            context: format!(
                "launching {} -- install open-agents, or set openAgentsBin in config.yaml",
                argv[0]
            ),
            source,
        })
    }
}

/// The registry rendered as the agent's tool list.
pub fn tool_listing(specs: &[ActionSpec]) -> String {
    let width = specs.iter().map(|spec| spec.id.len()).max().unwrap_or(0);
    specs
        .iter()
        .map(|spec| format!("  {:width$}  {}\n      {}", spec.id, spec.summary, spec.example))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> AgentTask {
        AgentTask {
            task: "read the latest email".into(),
            session: "work".into(),
            promptware: "Browser".into(),
            provider: Some("claude".into()),
            model: None,
            effort: None,
            timeout_seconds: Some(600),
        }
    }

    fn runner() -> AgentRunner {
        AgentRunner {
            binary: PathBuf::from("open-agents"),
            cli: PathBuf::from("ob"),
            home: PathBuf::from("/tmp/ob-home"),
        }
    }

    #[test]
    fn the_promptware_is_always_passed_because_it_is_what_gives_the_run_memory() {
        let argv = runner().command_line(&task());
        assert!(argv.contains(&"--promptware=Browser".to_string()), "{argv:?}");
        assert!(argv.contains(&"--json".to_string()), "{argv:?}");
    }

    #[test]
    fn omitted_settings_are_left_out_rather_than_sent_as_empty_flags() {
        let mut task = task();
        task.model = None;
        task.effort = None;
        let argv = runner().command_line(&task);
        assert!(!argv.iter().any(|arg| arg.starts_with("--model")), "{argv:?}");
        assert!(!argv.iter().any(|arg| arg.starts_with("--effort")), "{argv:?}");
    }

    #[test]
    fn the_prompt_is_last_and_behind_a_double_dash_so_a_task_cannot_look_like_a_flag() {
        let mut task = task();
        task.task = "--help me".into();
        let argv = runner().command_line(&task);
        assert_eq!(argv[argv.len() - 2], "--");
        assert!(argv.last().unwrap().starts_with("--help me"));
    }

    #[test]
    fn the_agent_is_pinned_to_one_session_through_the_environment() {
        let environment = runner().environment(&task());
        assert_eq!(environment.get(SESSION_ENV).unwrap(), "work");
        assert_eq!(environment.get(crate::HOME_ENV).unwrap(), "/tmp/ob-home");
    }

    #[test]
    fn every_action_appears_in_the_listing_the_agent_is_given() {
        // The point of the registry: an action that exists is an action the agent is told about,
        // with no second list to keep in step.
        let prompt = runner().prompt(&task());
        for spec in crate::actions::registry() {
            assert!(prompt.contains(spec.id), "{} is missing from the agent's tools", spec.id);
            assert!(prompt.contains(spec.example), "{} has no example in the prompt", spec.id);
        }
    }

    #[test]
    fn the_prompt_names_the_binary_this_build_would_actually_run() {
        let mut runner = runner();
        runner.cli = PathBuf::from("/opt/custom/ob");
        assert!(runner.prompt(&task()).contains("/opt/custom/ob"));
    }
}
