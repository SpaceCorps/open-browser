//! The action registry: the single place an action is declared.
//!
//! Every browser capability is one [`ActionSpec`] here, and the three surfaces are all derived from
//! this list rather than written three times:
//!
//! * the CLI builds a subcommand per action, with one flag per parameter ([`crate::actions::spec`]);
//! * the HTTP API dispatches `POST /api/actions/<id>` through the same [`Action::parse`];
//! * the agent-facing tool listing (`ob actions list --json`) is this list serialised.
//!
//! That is what makes "anything an agent can do in the browser has a CLI command" a property of the
//! build rather than a promise: a new action that reaches the browser has to be declared here, and
//! declaring it here is what gives it a command. `every_action_has_a_cli_command` in the CLI crate
//! fails otherwise.

mod spec;

pub use spec::{ActionSpec, ParamKind, ParamSpec};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A parsed, validated action ready to execute.
///
/// The variants carry owned, already-checked values; parsing happens once in [`Action::parse`] so
/// that the CLI and the HTTP API cannot disagree about what a parameter means.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum Action {
    Goto { url: String, wait: WaitUntil },
    Back,
    Forward,
    Reload,
    Click { selector: String, index: usize },
    Type { selector: String, text: String, clear: bool, enter: bool },
    Press { key: String },
    Select { selector: String, value: String },
    Check { selector: String, checked: bool },
    Hover { selector: String },
    Scroll { to: ScrollTarget },
    WaitFor { selector: Option<String>, text: Option<String>, timeout_ms: u64 },
    Text { selector: Option<String> },
    Html { selector: Option<String> },
    Attribute { selector: String, name: String },
    Links { pattern: Option<String> },
    Screenshot { path: Option<String>, full_page: bool, selector: Option<String> },
    Pdf { path: Option<String> },
    Eval { expression: String },
    Cookies { op: CookieAction },
    Upload { selector: String, paths: Vec<String> },
    Download { url: String, path: Option<String> },
    Url,
    Title,
}

/// How far a navigation must get before `goto` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WaitUntil {
    /// The document has been committed. Fast, and enough for a static page.
    Commit,
    /// The load event has fired. The default: it is what a person means by "the page is up".
    #[default]
    Load,
    /// No network request for 500ms. Needed for client-rendered pages, slow everywhere else.
    Idle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScrollTarget {
    Top,
    Bottom,
    Selector(String),
    Pixels(i64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CookieAction {
    List,
    Clear,
}

/// Every action this build can perform, in the order the CLI lists them.
pub fn registry() -> &'static [ActionSpec] {
    spec::REGISTRY
}

/// Look one up by its stable id.
pub fn find(id: &str) -> Option<&'static ActionSpec> {
    registry().iter().find(|spec| spec.id == id)
}

impl Action {
    /// The id of the spec this action came from.
    pub fn id(&self) -> &'static str {
        match self {
            Action::Goto { .. } => "goto",
            Action::Back => "back",
            Action::Forward => "forward",
            Action::Reload => "reload",
            Action::Click { .. } => "click",
            Action::Type { .. } => "type",
            Action::Press { .. } => "press",
            Action::Select { .. } => "select",
            Action::Check { .. } => "check",
            Action::Hover { .. } => "hover",
            Action::Scroll { .. } => "scroll",
            Action::WaitFor { .. } => "wait-for",
            Action::Text { .. } => "text",
            Action::Html { .. } => "html",
            Action::Attribute { .. } => "attribute",
            Action::Links { .. } => "links",
            Action::Screenshot { .. } => "screenshot",
            Action::Pdf { .. } => "pdf",
            Action::Eval { .. } => "eval",
            Action::Cookies { .. } => "cookies",
            Action::Upload { .. } => "upload",
            Action::Download { .. } => "download",
            Action::Url => "url",
            Action::Title => "title",
        }
    }

    /// Build an action from an id and a bag of string parameters.
    ///
    /// This is the one parser. The CLI fills the map from flags and the HTTP API from a JSON
    /// object, so a parameter cannot mean one thing on the command line and another over the wire.
    pub fn parse(id: &str, params: &BTreeMap<String, String>) -> crate::Result<Self> {
        let spec = find(id).ok_or_else(|| crate::Error::UnknownAction {
            id: id.to_string(),
            known: registry().iter().map(|s| s.id).collect::<Vec<_>>().join(", "),
        })?;
        spec.validate(params)?;

        let get = |name: &str| params.get(name).map(String::as_str);
        let required = |name: &str| -> crate::Result<String> {
            get(name).map(str::to_string).ok_or_else(|| crate::Error::MissingParameter {
                action: id.to_string(),
                parameter: name.to_string(),
            })
        };
        let flag = |name: &str| matches!(get(name), Some("true" | "1" | "yes"));
        let number = |name: &str, default: u64| -> crate::Result<u64> {
            match get(name) {
                None => Ok(default),
                Some(raw) => raw.parse().map_err(|_| crate::Error::BadParameter {
                    action: id.to_string(),
                    parameter: name.to_string(),
                    reason: format!("'{raw}' is not a number"),
                }),
            }
        };

        Ok(match id {
            "goto" => Action::Goto {
                url: normalize_url(&required("url")?),
                wait: match get("wait") {
                    None | Some("load") => WaitUntil::Load,
                    Some("commit") => WaitUntil::Commit,
                    Some("idle") => WaitUntil::Idle,
                    Some(other) => {
                        return Err(crate::Error::BadParameter {
                            action: id.to_string(),
                            parameter: "wait".into(),
                            reason: format!("'{other}' is not one of commit, load, idle"),
                        })
                    }
                },
            },
            "back" => Action::Back,
            "forward" => Action::Forward,
            "reload" => Action::Reload,
            "click" => Action::Click {
                selector: required("selector")?,
                index: number("index", 0)? as usize,
            },
            "type" => Action::Type {
                selector: required("selector")?,
                text: required("text")?,
                clear: flag("clear"),
                enter: flag("enter"),
            },
            "press" => Action::Press { key: required("key")? },
            "select" => {
                Action::Select { selector: required("selector")?, value: required("value")? }
            }
            "check" => Action::Check {
                selector: required("selector")?,
                // `--checked=false` is how you untick a box; absent means tick it.
                checked: get("checked").map(|v| v != "false").unwrap_or(true),
            },
            "hover" => Action::Hover { selector: required("selector")? },
            "scroll" => Action::Scroll {
                to: match get("to") {
                    None | Some("bottom") => ScrollTarget::Bottom,
                    Some("top") => ScrollTarget::Top,
                    Some(raw) => match raw.parse::<i64>() {
                        Ok(pixels) => ScrollTarget::Pixels(pixels),
                        Err(_) => ScrollTarget::Selector(raw.to_string()),
                    },
                },
            },
            "wait-for" => {
                let selector = get("selector").map(str::to_string);
                let text = get("text").map(str::to_string);
                if selector.is_none() && text.is_none() {
                    return Err(crate::Error::MissingParameter {
                        action: id.to_string(),
                        parameter: "selector or text".into(),
                    });
                }
                Action::WaitFor { selector, text, timeout_ms: number("timeout", 30_000)? }
            }
            "text" => Action::Text { selector: get("selector").map(str::to_string) },
            "html" => Action::Html { selector: get("selector").map(str::to_string) },
            "attribute" => {
                Action::Attribute { selector: required("selector")?, name: required("name")? }
            }
            "links" => Action::Links { pattern: get("pattern").map(str::to_string) },
            "screenshot" => Action::Screenshot {
                path: get("path").map(str::to_string),
                full_page: flag("full-page"),
                selector: get("selector").map(str::to_string),
            },
            "pdf" => Action::Pdf { path: get("path").map(str::to_string) },
            "eval" => Action::Eval { expression: required("expression")? },
            "cookies" => Action::Cookies {
                op: match get("do") {
                    None | Some("list") => CookieAction::List,
                    Some("clear") => CookieAction::Clear,
                    Some(other) => {
                        return Err(crate::Error::BadParameter {
                            action: id.to_string(),
                            parameter: "do".into(),
                            reason: format!("'{other}' is not one of list, clear"),
                        })
                    }
                },
            },
            "upload" => Action::Upload {
                selector: required("selector")?,
                // Repeatable flags arrive newline-joined; the HTTP API sends the same shape.
                paths: required("path")?.lines().map(str::to_string).collect(),
            },
            "download" => Action::Download {
                url: normalize_url(&required("url")?),
                path: get("path").map(str::to_string),
            },
            "url" => Action::Url,
            "title" => Action::Title,
            _ => unreachable!("registry and parse agree: {id}"),
        })
    }
}

/// Accept `example.com` where a URL is wanted.
///
/// Every agent eventually passes a bare hostname, and a CDP navigation to one fails in a way that
/// reads like the site is down rather than like the input was wrong.
fn normalize_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.contains("://") || trimmed.starts_with("about:") || trimmed.starts_with("data:") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn every_registered_action_parses_and_reports_its_own_id() {
        // Guards the match in `Action::id` against a variant added without an arm, and the one in
        // `parse` against a spec added with no parser.
        for spec in registry() {
            let mut filled: BTreeMap<String, String> = spec
                .params
                .iter()
                .filter(|p| p.required)
                .map(|p| (p.name.to_string(), sample(p)))
                .collect();
            // `wait-for` needs one of two optional parameters, which `required` cannot express;
            // anything else reaching this list means a rule the spec does not describe.
            if spec.id == "wait-for" {
                filled.insert("selector".into(), "body".into());
            }
            let action = Action::parse(spec.id, &filled)
                .unwrap_or_else(|error| panic!("{} failed to parse: {error}", spec.id));
            assert_eq!(action.id(), spec.id);
        }
    }

    fn sample(param: &ParamSpec) -> String {
        match param.kind {
            ParamKind::Number => "1".into(),
            ParamKind::Flag => "true".into(),
            ParamKind::Choice(choices) => choices[0].into(),
            _ if param.name == "url" => "example.com".into(),
            _ => "body".into(),
        }
    }

    #[test]
    fn a_bare_hostname_becomes_an_https_url_and_a_scheme_is_left_alone() {
        let goto = Action::parse("goto", &params(&[("url", "example.com")])).unwrap();
        assert_eq!(goto, Action::Goto { url: "https://example.com".into(), wait: WaitUntil::Load });

        let http = Action::parse("goto", &params(&[("url", "http://localhost:5173")])).unwrap();
        assert!(matches!(http, Action::Goto { url, .. } if url == "http://localhost:5173"));
    }

    #[test]
    fn an_unknown_action_lists_the_known_ones() {
        let error = Action::parse("teleport", &params(&[])).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("teleport"), "{message}");
        assert!(message.contains("goto"), "{message}");
    }

    #[test]
    fn a_missing_required_parameter_names_it_rather_than_defaulting() {
        let error = Action::parse("click", &params(&[])).unwrap_err();
        assert!(error.to_string().contains("selector"), "{error}");
    }

    #[test]
    fn wait_for_needs_one_of_its_two_optional_parameters() {
        assert!(Action::parse("wait-for", &params(&[])).is_err());
        assert!(Action::parse("wait-for", &params(&[("text", "Done")])).is_ok());
    }

    #[test]
    fn check_ticks_by_default_and_unticks_only_when_told_to() {
        let on = Action::parse("check", &params(&[("selector", "#tos")])).unwrap();
        assert_eq!(on, Action::Check { selector: "#tos".into(), checked: true });
        let off =
            Action::parse("check", &params(&[("selector", "#tos"), ("checked", "false")])).unwrap();
        assert_eq!(off, Action::Check { selector: "#tos".into(), checked: false });
    }

    #[test]
    fn scroll_reads_a_number_as_pixels_and_anything_else_as_a_selector() {
        let by = Action::parse("scroll", &params(&[("to", "600")])).unwrap();
        assert_eq!(by, Action::Scroll { to: ScrollTarget::Pixels(600) });
        let into = Action::parse("scroll", &params(&[("to", "#footer")])).unwrap();
        assert_eq!(into, Action::Scroll { to: ScrollTarget::Selector("#footer".into()) });
    }

    #[test]
    fn an_unparseable_number_is_an_error_rather_than_a_silent_default() {
        let error =
            Action::parse("wait-for", &params(&[("text", "x"), ("timeout", "soon")])).unwrap_err();
        assert!(error.to_string().contains("soon"), "{error}");
    }
}
