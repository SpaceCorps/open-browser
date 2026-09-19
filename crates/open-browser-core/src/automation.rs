//! An automation: a list of actions saved as a file and run by name.
//!
//! The file is YAML whose steps are the same ids and parameters the CLI takes, so an automation is
//! readable as the commands it replaces, and a person can write one without having run anything. It
//! goes through [`Action::parse`] like everything else — a script cannot reach a capability the CLI
//! does not have, which is the same property from the other direction.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::actions::Action;
use crate::error::{io, Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Automation {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Defaults substituted into `{{placeholders}}`, overridable per run.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, String>,
    pub steps: Vec<Step>,
}

/// One step. `action` is the registry id; everything else is that action's parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub action: String,
    #[serde(default, flatten)]
    pub params: BTreeMap<String, String>,
    /// Keep going if this step fails. For the optional cookie banner that is usually not there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

impl Automation {
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::other(format!(
                    "no automation at {} -- list them with `ob automation list`",
                    path.display()
                )))
            }
            Err(source) => {
                return Err(Error::Io { context: format!("reading {}", path.display()), source })
            }
        };
        let automation: Self = serde_yaml::from_str(&text)
            .map_err(|error| Error::other(format!("{}: {error}", path.display())))?;
        automation.validate()?;
        Ok(automation)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            io(std::fs::create_dir_all(parent), || format!("creating {}", parent.display()))?;
        }
        let body = serde_yaml::to_string(self)
            .map_err(|error| Error::other(format!("serialising {}: {error}", self.name)))?;
        io(std::fs::write(path, body), || format!("writing {}", path.display()))
    }

    /// Check every step names a real action with parameters that action accepts.
    ///
    /// Done at load rather than at the failing step: a twenty-step automation that dies on step
    /// nineteen because of a typo has already sent nineteen steps' worth of side effects.
    pub fn validate(&self) -> Result<()> {
        if self.steps.is_empty() {
            return Err(Error::other(format!("{} has no steps", self.name)));
        }
        for (index, step) in self.steps.iter().enumerate() {
            let spec = crate::actions::find(&step.action).ok_or_else(|| Error::UnknownAction {
                id: format!("{} (step {})", step.action, index + 1),
                known: crate::actions::registry()
                    .iter()
                    .map(|s| s.id)
                    .collect::<Vec<_>>()
                    .join(", "),
            })?;
            spec.validate(&step.params)?;
        }
        Ok(())
    }

    /// Resolve every step into an [`Action`], substituting inputs.
    ///
    /// All the parsing happens up front for the same reason as `validate`: a bad selector on the
    /// last step should not be discovered after the first one has sent an email.
    pub fn plan(&self, overrides: &BTreeMap<String, String>) -> Result<Vec<PlannedStep>> {
        let mut values = self.inputs.clone();
        for (key, value) in overrides {
            values.insert(key.clone(), value.clone());
        }
        let mut planned = Vec::with_capacity(self.steps.len());
        for (index, step) in self.steps.iter().enumerate() {
            let mut params = BTreeMap::new();
            for (name, raw) in &step.params {
                params.insert(name.clone(), substitute(raw, &values, &self.name, index)?);
            }
            planned.push(PlannedStep {
                action: Action::parse(&step.action, &params)?,
                optional: step.optional,
            });
        }
        Ok(planned)
    }

    /// Every input a run must be given a value for.
    pub fn required_inputs(&self) -> Vec<String> {
        let mut needed = std::collections::BTreeSet::new();
        for step in &self.steps {
            for raw in step.params.values() {
                for name in placeholders(raw) {
                    if !self.inputs.contains_key(&name) {
                        needed.insert(name);
                    }
                }
            }
        }
        needed.into_iter().collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedStep {
    pub action: Action,
    pub optional: bool,
}

/// List the automations in a directory, by name.
pub fn list(dir: &Path) -> Result<Vec<String>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(Error::Io { context: format!("reading {}", dir.display()), source })
        }
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "yaml" || ext == "yml") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Replace `{{name}}` with the value of `name`.
///
/// An unknown placeholder is an error rather than an empty string: `ob goto {{site}}` with no
/// `site` should say so, not navigate to `https://`.
fn substitute(
    raw: &str,
    values: &BTreeMap<String, String>,
    automation: &str,
    step: usize,
) -> Result<String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            // An unclosed `{{` is literal text, not an error: a selector may legitimately contain
            // braces, and only a complete pair means substitution.
            out.push_str(&rest[start..]);
            return Ok(out);
        };
        let name = after[..end].trim();
        let value = values.get(name).ok_or_else(|| {
            Error::other(format!(
            "{automation} step {}: nothing was given for {{{{{name}}}}} -- pass --input {name}=...",
            step + 1
        ))
        })?;
        out.push_str(value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn placeholders(raw: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        found.push(after[..end].trim().to_string());
        rest = &after[end + 2..];
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn automation(yaml: &str) -> Automation {
        serde_yaml::from_str(yaml).unwrap()
    }

    const SEARCH: &str = r#"
name: search
inputs:
  site: duckduckgo.com
steps:
  - action: goto
    url: "{{site}}"
  - action: type
    selector: "input[name=q]"
    text: "{{query}}"
    enter: "true"
  - action: click
    selector: ".cookie-banner button"
    optional: true
  - action: text
"#;

    #[test]
    fn a_step_is_the_action_id_plus_that_actions_parameters() {
        let parsed = automation(SEARCH);
        assert_eq!(parsed.steps.len(), 4);
        assert_eq!(parsed.steps[1].action, "type");
        assert_eq!(parsed.steps[1].params.get("selector").unwrap(), "input[name=q]");
        assert!(parsed.steps[2].optional);
        assert!(parsed.steps[3].params.is_empty());
    }

    #[test]
    fn planning_substitutes_defaults_and_overrides() {
        let parsed = automation(SEARCH);
        let overrides = BTreeMap::from([("query".to_string(), "rust cdp".to_string())]);
        let plan = parsed.plan(&overrides).unwrap();
        assert_eq!(
            plan[0].action,
            Action::Goto {
                url: "https://duckduckgo.com".into(),
                wait: crate::actions::WaitUntil::Load,
            }
        );
        assert!(
            matches!(&plan[1].action, Action::Type { text, enter, .. } if text == "rust cdp" && *enter)
        );
    }

    #[test]
    fn a_placeholder_with_no_value_is_an_error_naming_the_flag_that_supplies_it() {
        let parsed = automation(SEARCH);
        let error = parsed.plan(&BTreeMap::new()).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("query"), "{message}");
        assert!(message.contains("--input"), "{message}");
    }

    #[test]
    fn required_inputs_lists_only_what_has_no_default() {
        let parsed = automation(SEARCH);
        assert_eq!(parsed.required_inputs(), vec!["query".to_string()]);
    }

    #[test]
    fn a_typo_in_a_late_step_is_caught_before_the_first_one_runs() {
        let bad = automation(
            "name: x\nsteps:\n  - action: goto\n    url: example.com\n  - action: clikc\n    selector: b\n",
        );
        let error = bad.validate().unwrap_err();
        assert!(error.to_string().contains("clikc"), "{error}");
        assert!(error.to_string().contains("step 2"), "{error}");
    }

    #[test]
    fn a_bad_parameter_in_a_late_step_is_also_caught_up_front() {
        let bad = automation(
            "name: x\nsteps:\n  - action: goto\n    url: example.com\n  - action: click\n    seelctor: b\n",
        );
        assert!(bad.validate().is_err());
    }

    #[test]
    fn an_automation_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search.yaml");
        let parsed = automation(SEARCH);
        parsed.save(&path).unwrap();
        assert_eq!(Automation::load(&path).unwrap(), parsed);
        assert_eq!(list(dir.path()).unwrap(), vec!["search".to_string()]);
    }

    #[test]
    fn a_missing_automation_says_how_to_find_the_real_ones() {
        let dir = tempfile::tempdir().unwrap();
        let error = Automation::load(&dir.path().join("nope.yaml")).unwrap_err();
        assert!(error.to_string().contains("ob automation list"), "{error}");
    }

    #[test]
    fn listing_an_absent_directory_is_empty_rather_than_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(&dir.path().join("never-created")).unwrap().is_empty());
    }

    #[test]
    fn an_unclosed_brace_stays_literal_because_selectors_may_contain_braces() {
        let values = BTreeMap::new();
        assert_eq!(substitute("a{{b", &values, "x", 0).unwrap(), "a{{b");
    }
}
