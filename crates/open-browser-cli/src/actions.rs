//! Turning the action registry into clap subcommands, and clap matches back into actions.
//!
//! This module is why "anything an agent can do in the browser has a CLI command" holds. Nothing
//! here knows what `click` or `screenshot` are; it walks [`open_browser_core::actions::registry`]
//! and builds a subcommand for each entry. Adding an action to the registry adds its command, and
//! there is no way to add one that reaches the browser without going through the registry —
//! [`crate::actions::tests::every_action_has_a_cli_command`] fails if that ever stops being true.

use std::collections::BTreeMap;

use clap::{Arg, ArgAction, ArgMatches, Command};
use open_browser_core::actions::{registry, ActionSpec, ParamKind, ParamSpec};

/// How many leading parameters a given action takes positionally.
///
/// `ob click 'button'` has to work, because that is how a person and an agent both write it, but
/// `ob screenshot --path=x` must not silently accept a stray positional. So the rule is narrow:
/// the required parameters, in declaration order, may also be given positionally, and nothing else
/// may. `upload` is the exception — its second required parameter is repeatable, and a repeatable
/// positional would swallow the first one's value too.
fn positional_count(spec: &ActionSpec) -> usize {
    let required: Vec<&ParamSpec> = spec.required_params().collect();
    if required.iter().any(|param| param.repeatable) {
        // Take the run of required parameters up to the first repeatable one.
        required.iter().position(|param| param.repeatable).unwrap_or(0)
    } else {
        required.len()
    }
}

/// The value name shown in `--help`, from the parameter's kind.
fn value_name(param: &ParamSpec) -> String {
    match param.kind {
        ParamKind::Selector => "SELECTOR".into(),
        ParamKind::Url => "URL".into(),
        ParamKind::Path => "PATH".into(),
        ParamKind::Number => "N".into(),
        ParamKind::Choice(choices) => choices.join("|").to_uppercase(),
        ParamKind::Flag => "BOOL".into(),
        ParamKind::Text => param.name.to_uppercase().replace('-', "_"),
    }
}

/// One subcommand, built from one spec.
pub fn command_for(spec: &'static ActionSpec) -> Command {
    let positionals = positional_count(spec);
    let mut command = Command::new(spec.id)
        .about(spec.summary)
        // The example goes in `after_help` rather than the summary because getting the quoting
        // right is most of what someone reading `ob click --help` actually needs.
        .after_help(format!("Example:\n  {}", spec.example));

    let mut seen_positional = 0usize;
    for param in spec.params {
        let is_positional = param.required && seen_positional < positionals;
        let mut arg = Arg::new(param.name).help(param.help);

        arg = match param.kind {
            // A flag parameter reaches the parser as the string "true" when present. Choice
            // parameters stay as values even when they look boolean (`check --checked=false`),
            // because "absent" and "false" mean different things there.
            ParamKind::Flag => arg.long(param.name).action(ArgAction::SetTrue),
            ParamKind::Choice(choices) => {
                arg.long(param.name).value_parser(choices.to_vec()).value_name(value_name(param))
            }
            _ if is_positional => {
                seen_positional += 1;
                arg.value_name(value_name(param)).required(true).index(seen_positional)
            }
            _ => {
                let mut arg = arg.long(param.name).value_name(value_name(param));
                if param.repeatable {
                    arg = arg.action(ArgAction::Append);
                } else if param.required {
                    arg = arg.required(true);
                }
                arg
            }
        };
        command = command.arg(arg);
    }
    command
}

/// Every action as a subcommand. Handed to clap as-is.
pub fn commands() -> Vec<Command> {
    registry().iter().map(command_for).collect()
}

/// Read a subcommand's matches back into the flat parameter map [`Action::parse`] takes.
///
/// The map is the only thing that crosses into the core, so the CLI and the HTTP API converge here
/// rather than each building their own half-parsed action.
pub fn params_from(spec: &ActionSpec, matches: &ArgMatches) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    for param in spec.params {
        match param.kind {
            ParamKind::Flag => {
                // Only a present flag is recorded: an absent one must stay absent so the core's
                // defaults (`check` ticks unless told otherwise) still apply.
                if matches.get_flag(param.name) {
                    params.insert(param.name.to_string(), "true".to_string());
                }
            }
            _ if param.repeatable => {
                // Repeatable values arrive newline-joined, which is the shape `Action::parse`
                // splits on and the shape the HTTP API sends.
                if let Some(values) = matches.get_many::<String>(param.name) {
                    let joined = values.cloned().collect::<Vec<_>>().join("\n");
                    if !joined.is_empty() {
                        params.insert(param.name.to_string(), joined);
                    }
                }
            }
            _ => {
                if let Some(value) = matches.get_one::<String>(param.name) {
                    params.insert(param.name.to_string(), value.clone());
                }
            }
        }
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test the registry's module doc names. It is the mechanical form of the project's one
    /// hard rule: an agent's only way to touch the browser is a shell command, so an action with
    /// no command is an action no agent can perform.
    #[test]
    fn every_action_has_a_cli_command() {
        let built: Vec<String> = commands().iter().map(|c| c.get_name().to_string()).collect();
        for spec in registry() {
            assert!(
                built.contains(&spec.id.to_string()),
                "the action '{}' is in the registry but has no `ob {}` command",
                spec.id,
                spec.id
            );
        }
        assert_eq!(built.len(), registry().len(), "a command exists that no action declared");
    }

    /// Building the commands is also where a malformed spec would blow up, so this asserts clap
    /// accepts the whole tree rather than only that the names line up.
    #[test]
    fn the_whole_generated_tree_is_a_valid_clap_command() {
        let mut root = Command::new("ob");
        for command in commands() {
            root = root.subcommand(command);
        }
        root.debug_assert();
    }

    #[test]
    fn a_required_parameter_can_be_given_positionally_or_by_flag() {
        let spec = open_browser_core::actions::find("click").unwrap();
        let positional = command_for(spec).try_get_matches_from(["click", "button.go"]).unwrap();
        assert_eq!(params_from(spec, &positional).get("selector").unwrap(), "button.go");

        // The same value as a flag is the form an automation file and the agent both tend to use.
        let flagged =
            command_for(spec).try_get_matches_from(["click", "--index=2", "button.go"]).unwrap();
        let params = params_from(spec, &flagged);
        assert_eq!(params.get("index").unwrap(), "2");
    }

    #[test]
    fn a_missing_required_parameter_fails_at_parse_rather_than_in_the_browser() {
        let spec = open_browser_core::actions::find("click").unwrap();
        assert!(command_for(spec).try_get_matches_from(["click"]).is_err());
    }

    #[test]
    fn a_repeatable_parameter_collects_every_occurrence() {
        let spec = open_browser_core::actions::find("upload").unwrap();
        let matches = command_for(spec)
            .try_get_matches_from(["upload", "input[type=file]", "--path=a.pdf", "--path=b.pdf"])
            .unwrap();
        let params = params_from(spec, &matches);
        assert_eq!(params.get("path").unwrap(), "a.pdf\nb.pdf");
        assert_eq!(params.get("selector").unwrap(), "input[type=file]");
    }

    #[test]
    fn an_absent_flag_is_left_out_so_the_cores_default_still_applies() {
        let spec = open_browser_core::actions::find("type").unwrap();
        let matches = command_for(spec).try_get_matches_from(["type", "#q", "hello"]).unwrap();
        let params = params_from(spec, &matches);
        assert!(!params.contains_key("clear"), "{params:?}");
        assert!(!params.contains_key("enter"), "{params:?}");
    }

    /// Every generated command must produce parameters the core accepts. This is the join between
    /// the two halves: clap could happily build a flag the parser then rejects as unknown.
    #[test]
    fn every_generated_command_parses_into_an_action_of_the_same_id() {
        for spec in registry() {
            let mut argv = vec![spec.id.to_string()];
            let positionals = positional_count(spec);
            let mut used = 0usize;
            for param in spec.params {
                let value = sample(param);
                if param.required && used < positionals && !matches!(param.kind, ParamKind::Flag) {
                    argv.push(value);
                    used += 1;
                } else if param.required {
                    argv.push(format!("--{}={value}", param.name));
                }
            }
            // `wait-for` needs one of two optional parameters, which `required` cannot express.
            if spec.id == "wait-for" {
                argv.push("--selector=body".to_string());
            }
            let matches = command_for(spec)
                .try_get_matches_from(&argv)
                .unwrap_or_else(|error| panic!("`ob {}` did not parse: {error}", argv.join(" ")));
            let params = params_from(spec, &matches);
            let action = open_browser_core::actions::Action::parse(spec.id, &params)
                .unwrap_or_else(|error| {
                    panic!("`ob {}` did not become an action: {error}", argv.join(" "))
                });
            assert_eq!(action.id(), spec.id);
        }
    }

    fn sample(param: &ParamSpec) -> String {
        match param.kind {
            ParamKind::Url => "https://example.com".into(),
            ParamKind::Selector => "body".into(),
            ParamKind::Path => "/tmp/sample".into(),
            ParamKind::Number => "1".into(),
            ParamKind::Choice(choices) => choices[0].into(),
            ParamKind::Flag => "true".into(),
            ParamKind::Text => "sample".into(),
        }
    }
}
