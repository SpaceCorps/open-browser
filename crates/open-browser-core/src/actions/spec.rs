//! The declarations themselves.
//!
//! A `const` table rather than a builder: the CLI needs this at argument-parsing time, before any
//! runtime exists, and a static table is what lets `ob --help` be generated from the same data the
//! executor dispatches on.

use serde::Serialize;
use std::collections::BTreeMap;

/// What a parameter accepts. Drives both the CLI's value hints and the validation in
/// [`ActionSpec::validate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParamKind {
    Text,
    /// A CSS selector. Separate from `Text` so the CLI can hint it and the web UI can offer a picker.
    Selector,
    Url,
    Path,
    Number,
    /// Present or absent. On the CLI a `--flag`; over HTTP the string "true".
    Flag,
    /// One of a fixed set. The first is the default.
    Choice(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ParamSpec {
    pub name: &'static str,
    pub kind: ParamKind,
    pub required: bool,
    /// Repeatable on the CLI; newline-joined into the single value the parser sees.
    pub repeatable: bool,
    pub help: &'static str,
}

impl ParamSpec {
    const fn new(name: &'static str, kind: ParamKind, required: bool, help: &'static str) -> Self {
        Self { name, kind, required, repeatable: false, help }
    }

    const fn repeatable(mut self) -> Self {
        self.repeatable = true;
        self
    }
}

/// One browser capability, in the form all three surfaces read.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ActionSpec {
    /// Stable id. The CLI subcommand name, the HTTP path segment, and what a script file names.
    pub id: &'static str,
    pub summary: &'static str,
    /// Grouping for `ob actions list` and the web UI's palette.
    pub group: &'static str,
    pub params: &'static [ParamSpec],
    /// Whether the action changes the page. Read-only actions are the ones safe to retry blindly,
    /// and the ones a `--dry-run` may still perform.
    pub mutates: bool,
    /// One copy-pasteable example, shown in `--help` and in the agent's tool listing. An agent with
    /// an example gets the quoting right; one with only a parameter list frequently does not.
    pub example: &'static str,
}

impl ActionSpec {
    /// Reject unknown parameters and bad choices before anything reaches the browser.
    ///
    /// Unknown names are an error rather than ignored: a typo'd `--seelctor` that silently did
    /// nothing would surface as a mysterious timeout deep in the run.
    pub fn validate(&self, params: &BTreeMap<String, String>) -> crate::Result<()> {
        for name in params.keys() {
            if !self.params.iter().any(|p| p.name == name) {
                let known = self.params.iter().map(|p| p.name).collect::<Vec<_>>().join(", ");
                return Err(crate::Error::BadParameter {
                    action: self.id.to_string(),
                    parameter: name.clone(),
                    reason: if known.is_empty() {
                        format!("{} takes no parameters", self.id)
                    } else {
                        format!("known parameters: {known}")
                    },
                });
            }
        }
        for param in self.params {
            let Some(value) = params.get(param.name) else { continue };
            if let ParamKind::Choice(choices) = param.kind {
                if !choices.contains(&value.as_str()) {
                    return Err(crate::Error::BadParameter {
                        action: self.id.to_string(),
                        parameter: param.name.to_string(),
                        reason: format!("'{value}' is not one of {}", choices.join(", ")),
                    });
                }
            }
        }
        Ok(())
    }

    pub fn required_params(&self) -> impl Iterator<Item = &ParamSpec> {
        self.params.iter().filter(|p| p.required)
    }
}

const SELECTOR: ParamKind = ParamKind::Selector;
const TEXT: ParamKind = ParamKind::Text;

pub(super) static REGISTRY: &[ActionSpec] = &[
    ActionSpec {
        id: "goto",
        summary: "Navigate to a URL",
        group: "navigate",
        params: &[
            ParamSpec::new("url", ParamKind::Url, true, "Where to go. A bare hostname gets https://"),
            ParamSpec::new("wait", ParamKind::Choice(&["load", "commit", "idle"]), false,
                "How far to get before returning. `idle` for client-rendered pages"),
        ],
        mutates: true,
        example: "ob goto news.ycombinator.com",
    },
    ActionSpec {
        id: "back",
        summary: "Go back one entry in history",
        group: "navigate",
        params: &[],
        mutates: true,
        example: "ob back",
    },
    ActionSpec {
        id: "forward",
        summary: "Go forward one entry in history",
        group: "navigate",
        params: &[],
        mutates: true,
        example: "ob forward",
    },
    ActionSpec {
        id: "reload",
        summary: "Reload the current page",
        group: "navigate",
        params: &[],
        mutates: true,
        example: "ob reload",
    },
    ActionSpec {
        id: "click",
        summary: "Click the element a selector matches",
        group: "interact",
        params: &[
            ParamSpec::new("selector", SELECTOR, true, "CSS selector for the element"),
            ParamSpec::new("index", ParamKind::Number, false,
                "Which match to click when the selector is not unique. 0-based"),
        ],
        mutates: true,
        example: "ob click 'button[type=submit]'",
    },
    ActionSpec {
        id: "type",
        summary: "Type text into an input",
        group: "interact",
        params: &[
            ParamSpec::new("selector", SELECTOR, true, "CSS selector for the field"),
            ParamSpec::new("text", TEXT, true, "What to type"),
            ParamSpec::new("clear", ParamKind::Flag, false, "Empty the field first"),
            ParamSpec::new("enter", ParamKind::Flag, false, "Press Enter afterwards"),
        ],
        mutates: true,
        example: "ob type 'input[name=q]' 'rust cdp' --enter",
    },
    ActionSpec {
        id: "press",
        summary: "Press a key on the focused element",
        group: "interact",
        params: &[ParamSpec::new("key", TEXT, true, "Key name, e.g. Enter, Tab, Escape, ArrowDown")],
        mutates: true,
        example: "ob press Escape",
    },
    ActionSpec {
        id: "select",
        summary: "Choose an option in a <select>",
        group: "interact",
        params: &[
            ParamSpec::new("selector", SELECTOR, true, "CSS selector for the <select>"),
            ParamSpec::new("value", TEXT, true, "The option's value attribute"),
        ],
        mutates: true,
        example: "ob select '#country' US",
    },
    ActionSpec {
        id: "check",
        summary: "Tick or untick a checkbox",
        group: "interact",
        params: &[
            ParamSpec::new("selector", SELECTOR, true, "CSS selector for the checkbox"),
            ParamSpec::new("checked", ParamKind::Choice(&["true", "false"]), false,
                "Defaults to true; pass false to untick"),
        ],
        mutates: true,
        example: "ob check '#accept-terms'",
    },
    ActionSpec {
        id: "hover",
        summary: "Hover the pointer over an element",
        group: "interact",
        params: &[ParamSpec::new("selector", SELECTOR, true, "CSS selector for the element")],
        mutates: true,
        example: "ob hover '.menu-trigger'",
    },
    ActionSpec {
        id: "scroll",
        summary: "Scroll the page",
        group: "interact",
        params: &[ParamSpec::new("to", TEXT, false,
            "`top`, `bottom`, a pixel offset, or a selector to scroll into view. Defaults to bottom")],
        mutates: true,
        example: "ob scroll --to=bottom",
    },
    ActionSpec {
        id: "wait-for",
        summary: "Wait until an element or some text appears",
        group: "interact",
        params: &[
            ParamSpec::new("selector", SELECTOR, false, "Wait for this element to exist"),
            ParamSpec::new("text", TEXT, false, "Wait for this text to appear in the body"),
            ParamSpec::new("timeout", ParamKind::Number, false, "Milliseconds before giving up. Default 30000"),
        ],
        mutates: false,
        example: "ob wait-for --selector='.results' --timeout=10000",
    },
    ActionSpec {
        id: "text",
        summary: "Read the visible text of the page or one element",
        group: "read",
        params: &[ParamSpec::new("selector", SELECTOR, false, "Limit to this element. Omit for the whole body")],
        mutates: false,
        example: "ob text --selector=article",
    },
    ActionSpec {
        id: "html",
        summary: "Read the HTML of the page or one element",
        group: "read",
        params: &[ParamSpec::new("selector", SELECTOR, false, "Limit to this element. Omit for the document")],
        mutates: false,
        example: "ob html --selector='#main'",
    },
    ActionSpec {
        id: "attribute",
        summary: "Read one property or attribute off an element",
        group: "read",
        params: &[
            ParamSpec::new("selector", SELECTOR, true, "CSS selector for the element"),
            ParamSpec::new(
                "name",
                TEXT,
                true,
                "Property or attribute name. The live DOM property wins where there is one, so \
                 `value` reads back what was typed rather than the served markup",
            ),
        ],
        mutates: false,
        example: "ob attribute 'a.next' href",
    },
    ActionSpec {
        id: "links",
        summary: "List the links on the page",
        group: "read",
        params: &[ParamSpec::new("pattern", TEXT, false, "Keep only hrefs containing this substring")],
        mutates: false,
        example: "ob links --pattern=/issues/",
    },
    ActionSpec {
        id: "screenshot",
        summary: "Capture the page as a PNG",
        group: "capture",
        params: &[
            ParamSpec::new("path", ParamKind::Path, false, "Where to write it. Defaults into the session directory"),
            ParamSpec::new("full-page", ParamKind::Flag, false, "Capture beyond the viewport"),
            ParamSpec::new("selector", SELECTOR, false, "Capture just this element"),
        ],
        mutates: false,
        example: "ob screenshot --full-page --path=./page.png",
    },
    ActionSpec {
        id: "pdf",
        summary: "Print the page to PDF",
        group: "capture",
        params: &[ParamSpec::new("path", ParamKind::Path, false, "Where to write it")],
        mutates: false,
        example: "ob pdf --path=./invoice.pdf",
    },
    ActionSpec {
        id: "eval",
        summary: "Evaluate a JavaScript expression in the page",
        group: "read",
        params: &[ParamSpec::new("expression", TEXT, true, "JavaScript, evaluated in the page's context")],
        mutates: true,
        example: "ob eval 'document.querySelectorAll(\"li\").length'",
    },
    ActionSpec {
        id: "cookies",
        summary: "List or clear the session's cookies",
        group: "session",
        params: &[ParamSpec::new("do", ParamKind::Choice(&["list", "clear"]), false, "Defaults to list")],
        mutates: false,
        example: "ob cookies --do=list",
    },
    ActionSpec {
        id: "upload",
        summary: "Attach files to a file input",
        group: "interact",
        params: &[
            ParamSpec::new("selector", SELECTOR, true, "CSS selector for the <input type=file>"),
            ParamSpec::new("path", ParamKind::Path, true, "File to attach. Repeatable").repeatable(),
        ],
        mutates: true,
        example: "ob upload 'input[type=file]' --path=./report.pdf",
    },
    ActionSpec {
        id: "download",
        summary: "Fetch a URL using the session's cookies and save it",
        group: "capture",
        params: &[
            ParamSpec::new("url", ParamKind::Url, true, "What to download"),
            ParamSpec::new("path", ParamKind::Path, false, "Where to write it"),
        ],
        mutates: false,
        example: "ob download https://example.com/export.csv --path=./export.csv",
    },
    ActionSpec {
        id: "url",
        summary: "Print the current URL",
        group: "read",
        params: &[],
        mutates: false,
        example: "ob url",
    },
    ActionSpec {
        id: "title",
        summary: "Print the page title",
        group: "read",
        params: &[],
        mutates: false,
        example: "ob title",
    },
];

#[cfg(test)]
mod tests {
    use super::super::registry;
    use super::*;

    #[test]
    fn ids_are_unique_and_url_safe() {
        let mut seen = std::collections::BTreeSet::new();
        for spec in registry() {
            assert!(seen.insert(spec.id), "duplicate action id {}", spec.id);
            assert!(
                spec.id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{} is not a usable subcommand or path segment",
                spec.id
            );
        }
    }

    #[test]
    fn every_action_documents_itself_with_a_matching_example() {
        for spec in registry() {
            assert!(!spec.summary.is_empty(), "{} has no summary", spec.id);
            // The example is what an agent copies; one naming a different command is worse than none.
            assert!(
                spec.example.starts_with(&format!("ob {}", spec.id)),
                "{}'s example does not invoke it: {}",
                spec.id,
                spec.example
            );
            for param in spec.params {
                assert!(!param.help.is_empty(), "{}.{} has no help", spec.id, param.name);
            }
        }
    }

    #[test]
    fn unknown_parameters_are_rejected_rather_than_ignored() {
        let spec = super::super::find("click").unwrap();
        let params = BTreeMap::from([("seelctor".to_string(), "button".to_string())]);
        let error = spec.validate(&params).unwrap_err();
        assert!(error.to_string().contains("selector"), "{error}");
    }

    #[test]
    fn a_choice_outside_its_set_is_rejected() {
        let spec = super::super::find("goto").unwrap();
        let params = BTreeMap::from([
            ("url".to_string(), "example.com".to_string()),
            ("wait".to_string(), "eventually".to_string()),
        ]);
        assert!(spec.validate(&params).is_err());
    }

    #[test]
    fn an_action_with_no_parameters_says_so_plainly() {
        let spec = super::super::find("title").unwrap();
        let params = BTreeMap::from([("selector".to_string(), "h1".to_string())]);
        let error = spec.validate(&params).unwrap_err();
        assert!(error.to_string().contains("takes no parameters"), "{error}");
    }
}
