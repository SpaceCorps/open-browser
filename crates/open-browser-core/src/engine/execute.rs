//! Turning an [`Action`] into CDP calls.
//!
//! One `match`, one arm per action. Every arm returns an [`ActionOutcome`] whose `value` is the
//! JSON an agent reads and whose `summary` is the line a person reads — the CLI prints one or the
//! other, and the HTTP API returns both, so the two can never describe different things.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chromiumoxide::cdp::browser_protocol::dom::SetFileInputFilesParams;
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::Page;
use serde::Serialize;

use crate::actions::{Action, CookieAction, ScrollTarget, WaitUntil};
use crate::error::{cdp, io, Error, Result};

/// What an action produced.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionOutcome {
    pub action: String,
    /// Machine-readable result. `null` for actions whose only effect is on the page.
    pub value: serde_json::Value,
    /// One line for a human.
    pub summary: String,
    /// Any file the action wrote.
    pub artifact: Option<PathBuf>,
    pub duration_ms: u64,
}

/// How long to wait between polls when waiting for an element or some text.
///
/// Fast enough that a page appearing feels immediate, slow enough that a 30-second wait is 150
/// evaluations rather than thirty thousand.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

pub async fn execute(page: &Page, action: &Action, artifacts: &Path) -> Result<ActionOutcome> {
    let started = Instant::now();
    let id = action.id().to_string();
    let (value, summary, artifact) = run(page, action, artifacts).await?;
    Ok(ActionOutcome {
        action: id,
        value,
        summary,
        artifact,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

async fn run(
    page: &Page,
    action: &Action,
    artifacts: &Path,
) -> Result<(serde_json::Value, String, Option<PathBuf>)> {
    match action {
        Action::Goto { url, wait } => {
            cdp(page.goto(url.as_str()).await, || format!("navigating to {url}"))?;
            match wait {
                // `commit` is the navigation command returning; nothing further to await.
                WaitUntil::Commit => {}
                WaitUntil::Load => {
                    cdp(page.wait_for_navigation().await, || format!("loading {url}"))?;
                }
                WaitUntil::Idle => {
                    cdp(page.wait_for_navigation().await, || format!("loading {url}"))?;
                    wait_for_network_idle(page).await?;
                }
            }
            let landed = current_url(page).await?;
            Ok((json_url(&landed), format!("at {landed}"), None))
        }

        Action::Back => history(page, -1).await,
        Action::Forward => history(page, 1).await,

        Action::Reload => {
            cdp(page.reload().await, || "reloading".to_string())?;
            let url = current_url(page).await?;
            Ok((json_url(&url), format!("reloaded {url}"), None))
        }

        Action::Click { selector, index } => {
            let elements = find_all(page, selector).await?;
            let element = elements.get(*index).ok_or_else(|| Error::BadParameter {
                action: "click".into(),
                parameter: "index".into(),
                reason: format!("only {} element(s) match '{selector}'", elements.len()),
            })?;
            cdp(element.click().await.map(|_| ()), || format!("clicking '{selector}'"))?;
            Ok((serde_json::Value::Null, format!("clicked '{selector}'"), None))
        }

        Action::Type { selector, text, clear, enter } => {
            let element = find_one(page, selector).await?;
            cdp(element.click().await.map(|_| ()), || format!("focusing '{selector}'"))?;
            if *clear {
                // Select-all then type: CDP has no "clear", and deleting character by character is
                // both slower and wrong for a field with a non-trivial value.
                cdp(element.call_js_fn(CLEAR_FN, false).await.map(|_| ()), || {
                    format!("clearing '{selector}'")
                })?;
            }
            cdp(element.type_str(text).await.map(|_| ()), || format!("typing into '{selector}'"))?;
            if *enter {
                cdp(element.press_key("Enter").await.map(|_| ()), || "pressing Enter".to_string())?;
            }
            Ok((
                serde_json::Value::Null,
                format!("typed {} chars into '{selector}'", text.len()),
                None,
            ))
        }

        Action::Press { key } => {
            let element = find_one(page, "body").await?;
            cdp(element.press_key(key).await.map(|_| ()), || format!("pressing {key}"))?;
            Ok((serde_json::Value::Null, format!("pressed {key}"), None))
        }

        Action::Select { selector, value } => {
            let element = find_one(page, selector).await?;
            cdp(
                element
                    .call_js_fn(js_fn_with(&[value.as_str().into()], SELECT_BODY), false)
                    .await
                    .map(|_| ()),
                || format!("selecting '{value}' in '{selector}'"),
            )?;
            Ok((serde_json::Value::Null, format!("selected '{value}'"), None))
        }

        Action::Check { selector, checked } => {
            let element = find_one(page, selector).await?;
            cdp(
                element
                    .call_js_fn(js_fn_with(&[(*checked).into()], CHECK_BODY), false)
                    .await
                    .map(|_| ()),
                || format!("setting '{selector}'"),
            )?;
            Ok((
                serde_json::json!({ "checked": checked }),
                format!("{} '{selector}'", if *checked { "checked" } else { "unchecked" }),
                None,
            ))
        }

        Action::Hover { selector } => {
            let element = find_one(page, selector).await?;
            cdp(element.hover().await.map(|_| ()), || format!("hovering '{selector}'"))?;
            Ok((serde_json::Value::Null, format!("hovered '{selector}'"), None))
        }

        Action::Scroll { to } => {
            let description = match to {
                ScrollTarget::Top => {
                    eval_unit(page, "window.scrollTo(0,0)").await?;
                    "top".to_string()
                }
                ScrollTarget::Bottom => {
                    eval_unit(page, "window.scrollTo(0,document.body.scrollHeight)").await?;
                    "bottom".to_string()
                }
                ScrollTarget::Pixels(by) => {
                    eval_unit(page, &format!("window.scrollBy(0,{by})")).await?;
                    format!("{by}px")
                }
                ScrollTarget::Selector(selector) => {
                    let element = find_one(page, selector).await?;
                    cdp(element.scroll_into_view().await.map(|_| ()), || {
                        format!("scrolling to '{selector}'")
                    })?;
                    format!("'{selector}'")
                }
            };
            Ok((serde_json::Value::Null, format!("scrolled to {description}"), None))
        }

        Action::WaitFor { selector, text, timeout_ms } => {
            let deadline = Instant::now() + Duration::from_millis(*timeout_ms);
            let what = match (selector, text) {
                (Some(s), _) => format!("'{s}'"),
                (_, Some(t)) => format!("the text '{t}'"),
                _ => unreachable!("parse rejects a wait-for with neither"),
            };
            loop {
                let present = match (selector, text) {
                    (Some(selector), _) => page.find_element(selector.as_str()).await.is_ok(),
                    (_, Some(needle)) => body_text(page).await.unwrap_or_default().contains(needle),
                    _ => unreachable!(),
                };
                if present {
                    return Ok((
                        serde_json::json!({ "found": true }),
                        format!("{what} appeared"),
                        None,
                    ));
                }
                if Instant::now() >= deadline {
                    return Err(Error::Timeout { what, seconds: timeout_ms / 1000 });
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }

        Action::Text { selector } => {
            let text = match selector {
                Some(selector) => {
                    let element = find_one(page, selector).await?;
                    cdp(element.inner_text().await, || format!("reading '{selector}'"))?
                        .unwrap_or_default()
                }
                None => body_text(page).await?,
            };
            let summary = text.clone();
            Ok((serde_json::Value::String(text), summary, None))
        }

        Action::Html { selector } => {
            let html = match selector {
                Some(selector) => {
                    let element = find_one(page, selector).await?;
                    cdp(element.outer_html().await, || format!("reading '{selector}'"))?
                        .unwrap_or_default()
                }
                None => cdp(page.content().await, || "reading the document".to_string())?,
            };
            let summary = html.clone();
            Ok((serde_json::Value::String(html), summary, None))
        }

        Action::Attribute { selector, name } => {
            let element = find_one(page, selector).await?;
            let value = read_property(&element, selector, name).await?;
            let summary = value.clone().unwrap_or_else(|| format!("'{selector}' has no {name}"));
            Ok((
                value.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null),
                summary,
                None,
            ))
        }

        Action::Links { pattern } => {
            let script = "Array.from(document.querySelectorAll('a[href]')).map(a=>({text:(a.innerText||'').trim().slice(0,120),href:a.href}))";
            let mut links: Vec<Link> = eval_into(page, script).await?;
            if let Some(pattern) = pattern {
                links.retain(|link| link.href.contains(pattern.as_str()));
            }
            let summary = links
                .iter()
                .map(|link| format!("{}\t{}", link.href, link.text))
                .collect::<Vec<_>>()
                .join("\n");
            Ok((serde_json::to_value(&links).unwrap_or(serde_json::Value::Null), summary, None))
        }

        Action::Screenshot { path, full_page, selector } => {
            let target = resolve_artifact(path.as_deref(), artifacts, "png")?;
            let bytes = match selector {
                Some(selector) => {
                    let element = find_one(page, selector).await?;
                    cdp(element.screenshot(CaptureScreenshotFormat::Png).await, || {
                        format!("capturing '{selector}'")
                    })?
                }
                None => {
                    let params = ScreenshotParams::builder()
                        .format(CaptureScreenshotFormat::Png)
                        .full_page(*full_page)
                        .build();
                    cdp(page.screenshot(params).await, || "capturing the page".to_string())?
                }
            };
            io(std::fs::write(&target, &bytes), || format!("writing {}", target.display()))?;
            Ok((
                serde_json::json!({ "path": target, "bytes": bytes.len() }),
                format!("wrote {} ({} bytes)", target.display(), bytes.len()),
                Some(target),
            ))
        }

        Action::Pdf { path } => {
            let target = resolve_artifact(path.as_deref(), artifacts, "pdf")?;
            let bytes = cdp(page.pdf(Default::default()).await, || "printing to PDF".to_string())?;
            io(std::fs::write(&target, &bytes), || format!("writing {}", target.display()))?;
            Ok((
                serde_json::json!({ "path": target, "bytes": bytes.len() }),
                format!("wrote {} ({} bytes)", target.display(), bytes.len()),
                Some(target),
            ))
        }

        Action::Eval { expression } => {
            let result = cdp(page.evaluate(expression.as_str()).await, || {
                format!("evaluating {expression}")
            })?;
            let value = result.into_value::<serde_json::Value>().unwrap_or(serde_json::Value::Null);
            let summary = match &value {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            Ok((value, summary, None))
        }

        Action::Cookies { op } => match op {
            CookieAction::List => {
                let cookies = cdp(page.get_cookies().await, || "reading cookies".to_string())?;
                let rows: Vec<_> = cookies
                    .iter()
                    .map(|c| serde_json::json!({ "name": c.name, "domain": c.domain, "path": c.path }))
                    .collect();
                let summary = cookies
                    .iter()
                    .map(|c| format!("{}\t{}", c.domain, c.name))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok((serde_json::Value::Array(rows), summary, None))
            }
            CookieAction::Clear => {
                let cookies = cdp(page.get_cookies().await, || "reading cookies".to_string())?;
                let count = cookies.len();
                for cookie in cookies {
                    let _ = page.delete_cookie(
                        chromiumoxide::cdp::browser_protocol::network::DeleteCookiesParams::builder()
                            .name(cookie.name)
                            .domain(cookie.domain)
                            .build()
                            .map_err(Error::Other)?,
                    ).await;
                }
                Ok((
                    serde_json::json!({ "cleared": count }),
                    format!("cleared {count} cookie(s)"),
                    None,
                ))
            }
        },

        Action::Upload { selector, paths } => {
            let element = find_one(page, selector).await?;
            let mut absolute = Vec::new();
            for path in paths {
                let resolved = PathBuf::from(path)
                    .canonicalize()
                    .map_err(|source| Error::Io { context: format!("resolving {path}"), source })?;
                absolute.push(resolved.display().to_string());
            }
            // There is no wrapper for this one: `DOM.setFileInputFiles` is the only way to put a
            // file into an `<input type=file>`, because a synthetic click would open the OS picker
            // and nothing in the page is allowed to set `.files`.
            let params = SetFileInputFilesParams::builder()
                .files(absolute.clone())
                .backend_node_id(element.backend_node_id)
                .build()
                .map_err(Error::Other)?;
            cdp(page.execute(params).await.map(|_| ()), || {
                format!("attaching {} file(s) to '{selector}'", absolute.len())
            })?;
            Ok((
                serde_json::json!({ "files": absolute }),
                format!("attached {} file(s)", absolute.len()),
                None,
            ))
        }

        Action::Download { url, path } => {
            let target = resolve_artifact(path.as_deref(), artifacts, "bin")?;
            // Fetched from inside the page rather than with an HTTP client: the point of
            // downloading through a session is to use the cookies that session is logged in with.
            let script = format!(
                "(async()=>{{const r=await fetch({url});const b=new Uint8Array(await r.arrayBuffer());\
                 let s='';for(const x of b)s+=String.fromCharCode(x);return {{status:r.status,body:btoa(s)}}}})()",
                url = serde_json::Value::String(url.clone())
            );
            let fetched: Fetched = eval_into(page, &script).await?;
            use base64::Engine as _;
            let bytes =
                base64::engine::general_purpose::STANDARD.decode(fetched.body.as_bytes()).map_err(
                    |error| Error::other(format!("decoding the downloaded body: {error}")),
                )?;
            io(std::fs::write(&target, &bytes), || format!("writing {}", target.display()))?;
            Ok((
                serde_json::json!({ "path": target, "bytes": bytes.len(), "status": fetched.status }),
                format!(
                    "wrote {} ({} bytes, HTTP {})",
                    target.display(),
                    bytes.len(),
                    fetched.status
                ),
                Some(target),
            ))
        }

        Action::Url => {
            let url = current_url(page).await?;
            Ok((json_url(&url), url, None))
        }

        Action::Title => {
            let title = cdp(page.get_title().await, || "reading the title".to_string())?
                .unwrap_or_default();
            Ok((serde_json::Value::String(title.clone()), title, None))
        }
    }
}

/// `Element::call_js_fn` in chromiumoxide 0.9 takes a declaration and nothing else — there is no
/// argument list — so a value has to travel inside the source. It goes in as a JSON literal, which
/// is a subset of JavaScript expression syntax, so a selector value containing a quote or a newline
/// is escaped rather than closing the string and running as code.
fn js_fn_with(args: &[serde_json::Value], body: &str) -> String {
    let bindings = args
        .iter()
        .enumerate()
        .map(|(index, value)| format!("const a{index}={value};"))
        .collect::<String>();
    format!("function(){{{bindings}{body}}}")
}

/// Read one named thing off an element, preferring the live DOM property to the HTML attribute.
///
/// The two are not the same, and the difference is exactly the case that matters most here: after
/// `ob type '#q' hello`, the input's `value` *property* is "hello" while its `value` *attribute* is
/// whatever the page was served with — usually nothing at all. Reading back a field an agent just
/// filled is the commonest use of this action, so the property wins when there is one.
///
/// Only string, number and boolean properties count. `style` and `form` are objects, `click` is a
/// function, and none of those are what someone asking for an attribute meant; those fall through
/// to `getAttribute`, which is also what happens for `data-*` and any other name the DOM does not
/// reflect onto a property.
const PROPERTY_BODY: &str = "const p=this[a0];\
if(typeof p==='string'||typeof p==='number'||typeof p==='boolean')return String(p);\
const v=this.getAttribute(a0);return v===null?null:String(v);";

async fn read_property(
    element: &chromiumoxide::Element,
    selector: &str,
    name: &str,
) -> Result<Option<String>> {
    let returns =
        cdp(element.call_js_fn(js_fn_with(&[name.into()], PROPERTY_BODY), false).await, || {
            format!("reading {name} of '{selector}'")
        })?;
    // A JS `null` arrives as `Some(Value::Null)` and a function returning nothing as `None`; both
    // mean the element does not have this one.
    Ok(match returns.result.value {
        Some(serde_json::Value::String(value)) => Some(value),
        Some(serde_json::Value::Null) | None => None,
        Some(other) => Some(other.to_string()),
    })
}

const SELECT_BODY: &str = "this.value=a0;this.dispatchEvent(new Event('change',{bubbles:true}));";
const CHECK_BODY: &str = "if(this.checked!==a0){this.click()}return this.checked;";
const CLEAR_FN: &str =
    "function(){this.value='';this.dispatchEvent(new Event('input',{bubbles:true}))}";

#[derive(serde::Deserialize, Serialize)]
struct Link {
    text: String,
    href: String,
}

#[derive(serde::Deserialize)]
struct Fetched {
    status: u16,
    body: String,
}

async fn history(page: &Page, delta: i64) -> Result<(serde_json::Value, String, Option<PathBuf>)> {
    eval_unit(page, &format!("window.history.go({delta})")).await?;
    // The navigation is asynchronous and `history.go` resolves before it commits; without this the
    // URL read below is reliably the old one.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let url = current_url(page).await?;
    Ok((json_url(&url), format!("at {url}"), None))
}

/// No network request for 500ms, giving up after 10 seconds.
///
/// A page that polls on an interval never goes idle, so this has to be a bounded wait that returns
/// success rather than an error — the caller asked to wait for idle, not to fail without it.
async fn wait_for_network_idle(page: &Page) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let script = "performance.getEntriesByType('resource').length";
    let mut last: i64 = eval_into(page, script).await.unwrap_or(0);
    let mut quiet_since = Instant::now();
    while Instant::now() < deadline {
        tokio::time::sleep(POLL_INTERVAL).await;
        let now: i64 = eval_into(page, script).await.unwrap_or(last);
        if now != last {
            last = now;
            quiet_since = Instant::now();
        } else if quiet_since.elapsed() >= Duration::from_millis(500) {
            return Ok(());
        }
    }
    Ok(())
}

async fn find_one(page: &Page, selector: &str) -> Result<chromiumoxide::Element> {
    page.find_element(selector)
        .await
        .map_err(|_| Error::NoSuchElement { selector: selector.to_string() })
}

async fn find_all(page: &Page, selector: &str) -> Result<Vec<chromiumoxide::Element>> {
    let found = page
        .find_elements(selector)
        .await
        .map_err(|_| Error::NoSuchElement { selector: selector.to_string() })?;
    if found.is_empty() {
        return Err(Error::NoSuchElement { selector: selector.to_string() });
    }
    Ok(found)
}

async fn body_text(page: &Page) -> Result<String> {
    eval_into(page, "document.body ? document.body.innerText : ''").await
}

async fn current_url(page: &Page) -> Result<String> {
    Ok(cdp(page.url().await, || "reading the URL".to_string())?.unwrap_or_default())
}

fn json_url(url: &str) -> serde_json::Value {
    serde_json::json!({ "url": url })
}

async fn eval_unit(page: &Page, script: &str) -> Result<()> {
    cdp(page.evaluate(script).await.map(|_| ()), || format!("evaluating {script}"))
}

async fn eval_into<T: serde::de::DeserializeOwned>(page: &Page, script: &str) -> Result<T> {
    let result = cdp(page.evaluate(script).await, || format!("evaluating {script}"))?;
    result
        .into_value::<T>()
        .map_err(|error| Error::other(format!("unexpected result from the page: {error}")))
}

/// Where an artifact goes: the `--path` the user gave, or a timestamped name in the session's
/// directory so that repeated captures in one session do not overwrite each other.
fn resolve_artifact(path: Option<&str>, artifacts: &Path, extension: &str) -> Result<PathBuf> {
    let target = match path {
        Some(path) => PathBuf::from(path),
        None => {
            let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3f");
            artifacts.join(format!("{stamp}.{extension}"))
        }
    };
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            io(std::fs::create_dir_all(parent), || format!("creating {}", parent.display()))?;
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_with_quotes_is_escaped_into_the_function_rather_than_ending_the_string() {
        let nasty = "\"; window.stolen = 1; //";
        let source = js_fn_with(&[nasty.into()], SELECT_BODY);
        assert!(source.contains("window.stolen"), "{source}");
        // The payload survives as data inside a string literal; what matters is that the literal
        // is still closed before the body runs.
        assert!(source.contains(r#"const a0="\"; window.stolen = 1; //";"#), "{source}");
    }

    #[test]
    fn reading_a_name_prefers_the_property_and_keeps_get_attribute_as_the_fallback() {
        let source = js_fn_with(&["value".into()], PROPERTY_BODY);
        assert!(source.contains(r#"const a0="value";"#), "{source}");
        // Both halves have to be there: the property alone misses `data-*`, the attribute alone
        // misses everything an agent just typed.
        assert!(source.contains("this[a0]"), "{source}");
        assert!(source.contains("this.getAttribute(a0)"), "{source}");
        // An object-valued property such as `style` must not be stringified as "[object Object]".
        assert!(!source.contains("typeof p==='object'"), "{source}");
    }

    #[test]
    fn a_boolean_argument_reaches_the_page_as_a_boolean() {
        assert!(js_fn_with(&[true.into()], CHECK_BODY).contains("const a0=true;"));
    }

    #[test]
    fn an_explicit_path_is_used_as_given_and_its_parent_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = dir.path().join("nested/deeper/shot.png");
        let got = resolve_artifact(Some(wanted.to_str().unwrap()), dir.path(), "png").unwrap();
        assert_eq!(got, wanted);
        assert!(wanted.parent().unwrap().is_dir());
    }

    #[test]
    fn without_a_path_captures_are_timestamped_into_the_session_directory() {
        let dir = tempfile::tempdir().unwrap();
        let first = resolve_artifact(None, dir.path(), "png").unwrap();
        assert!(first.starts_with(dir.path()));
        assert_eq!(first.extension().unwrap(), "png");
        // Millisecond precision: two captures in the same second must not collide.
        std::thread::sleep(Duration::from_millis(2));
        let second = resolve_artifact(None, dir.path(), "png").unwrap();
        assert_ne!(first, second);
    }
}
