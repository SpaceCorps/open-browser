//! The handlers. Each one is the HTTP shape of a thing `ob` already does.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use open_browser_core::actions::{find, registry, Action};
use open_browser_core::agents::{AgentRunner, AgentTask};
use open_browser_core::automation::Automation;
use open_browser_core::config::Config;
use open_browser_core::engine::{
    execute, start_detached, ActionOutcome, BrowserHandle, LaunchOptions,
};
use open_browser_core::home::{validate_session_name, Home};
use open_browser_core::runs::{RunKind, RunStatus, RunStore};
use open_browser_core::session::{terminate, SessionRecord, SessionRegistry};
use serde::{Deserialize, Serialize};

use crate::ServeConfig;

#[derive(Clone)]
pub struct AppState {
    pub home: Home,
    pub config: Arc<Config>,
}

impl AppState {
    pub fn new(config: ServeConfig) -> Self {
        Self { home: config.home, config: Arc::new(config.config) }
    }

    pub fn runs(&self) -> Result<RunStore, ApiError> {
        self.home.ensure()?;
        Ok(RunStore::open(&self.home.database_path())?)
    }

    fn sessions(&self) -> SessionRegistry {
        SessionRegistry::new(&self.home)
    }

    /// The session a request acts in, started if it is not up.
    ///
    /// Unlike the CLI, the API always starts: an HTTP client has no `--start` to pass and no
    /// terminal to be told about it in.
    async fn session(&self, name: Option<&str>) -> Result<SessionRecord, ApiError> {
        let name = validate_session_name(name.unwrap_or(&self.config.session))?;
        if let Some(record) = self.sessions().get(&name)? {
            return Ok(record);
        }
        self.home.ensure()?;
        let profile = self.home.profile_dir(&name);
        let mut options = LaunchOptions::new(&profile);
        options.headless = self.config.headless;
        options.window = self.config.window;
        options.args = self.config.chrome_args.clone();
        let (pid, endpoint) = start_detached(&options).await?;
        // Pinned while this is the only tab; see `BrowserHandle::page`.
        let target = match BrowserHandle::connect(&endpoint).await {
            Ok(browser) => {
                let target = browser.primary_target().await.ok();
                drop(browser);
                target
            }
            Err(_) => None,
        };
        let record = SessionRecord {
            name,
            endpoint,
            pid,
            target,
            profile,
            headless: options.headless,
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        self.sessions().insert(record.clone())?;
        Ok(record)
    }
}

/// An error with a status code, so a bad selector is a 400 and a dead browser is a 502.
///
/// Mapping these by hand rather than returning 500 for everything is what lets the web UI tell a
/// person "that selector matched nothing" instead of "internal server error".
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl From<open_browser_core::Error> for ApiError {
    fn from(error: open_browser_core::Error) -> Self {
        use open_browser_core::Error as E;
        let status = match &error {
            E::UnknownAction { .. } | E::NoSuchSession { .. } => StatusCode::NOT_FOUND,
            E::MissingParameter { .. } | E::BadParameter { .. } => StatusCode::BAD_REQUEST,
            E::NoSuchElement { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            E::SessionRunning { .. } => StatusCode::CONFLICT,
            E::Timeout { .. } => StatusCode::GATEWAY_TIMEOUT,
            E::NoBrowser { .. } | E::Browser { .. } => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self { status, message: error.to_string() }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: error.to_string() }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(error: std::io::Error) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: error.to_string() }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

pub async fn health(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    Ok(Json(serde_json::json!({
        "ok": true,
        "actions": registry().len(),
        "sessions": state.sessions().list().map(|list| list.len()).unwrap_or(0),
        "version": env!("CARGO_PKG_VERSION"),
    })))
}

pub async fn list_actions() -> ApiResult<&'static [open_browser_core::actions::ActionSpec]> {
    Ok(Json(registry()))
}

pub async fn show_action(
    Path(id): Path<String>,
) -> ApiResult<&'static open_browser_core::actions::ActionSpec> {
    find(&id).map(Json).ok_or_else(|| ApiError {
        status: StatusCode::NOT_FOUND,
        message: format!("no action called '{id}'"),
    })
}

#[derive(Debug, Deserialize)]
pub struct RunActionBody {
    #[serde(default)]
    session: Option<String>,
    /// The action's parameters, by the same names the CLI's flags use.
    #[serde(default)]
    params: BTreeMap<String, String>,
}

/// `POST /api/actions/<id>` — the HTTP form of one `ob` command.
pub async fn run_action(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RunActionBody>,
) -> ApiResult<ActionOutcome> {
    // The same parser the CLI uses. This is the join that makes the two surfaces one.
    let action = Action::parse(&id, &body.params)?;
    let session = state.session(body.session.as_deref()).await?;
    let store = state.runs()?;
    let record = store.start(RunKind::Action, &session.name, action.id())?;

    let outcome = perform(&state, &session, &action).await;
    match &outcome {
        Ok(outcome) => {
            let result = serde_json::to_string(outcome).ok();
            store.finish(&record.id, RunStatus::Succeeded, result.as_deref(), None)?;
        }
        Err(error) => {
            store.finish(&record.id, RunStatus::Failed, None, Some(&error.message))?;
        }
    }
    outcome.map(Json)
}

async fn perform(
    state: &AppState,
    session: &SessionRecord,
    action: &Action,
) -> Result<ActionOutcome, ApiError> {
    let browser = BrowserHandle::connect(&session.endpoint).await?;
    let page = browser.page(session.target.as_deref()).await?;
    let artifacts = state.home.session_dir(&session.name);
    std::fs::create_dir_all(&artifacts)?;
    let outcome = execute(&page, action, &artifacts).await?;
    // Dropped, never closed: closing would take down the session every other client is using.
    drop(browser);
    Ok(outcome)
}

pub async fn list_sessions(State(state): State<AppState>) -> ApiResult<Vec<SessionRecord>> {
    Ok(Json(state.sessions().list()?))
}

#[derive(Debug, Deserialize)]
pub struct StartSessionBody {
    name: Option<String>,
}

pub async fn start_session(
    State(state): State<AppState>,
    Json(body): Json<StartSessionBody>,
) -> ApiResult<SessionRecord> {
    Ok(Json(state.session(body.name.as_deref()).await?))
}

pub async fn stop_session(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<serde_json::Value> {
    let name = validate_session_name(&name)?;
    let registry = state.sessions();
    match registry.get(&name)? {
        Some(record) => {
            terminate(record.pid);
            registry.remove(&name)?;
            Ok(Json(serde_json::json!({ "stopped": name })))
        }
        None => Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: format!("'{name}' is not running"),
        }),
    }
}

pub async fn list_automations(State(state): State<AppState>) -> ApiResult<Vec<String>> {
    Ok(Json(open_browser_core::automation::list(&state.home.automations_dir())?))
}

pub async fn show_automation(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<Automation> {
    let name = validate_session_name(&name)?;
    Ok(Json(Automation::load(&state.home.automation_path(&name))?))
}

#[derive(Debug, Deserialize)]
pub struct RunAutomationBody {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    inputs: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct AutomationRunResult {
    automation: String,
    ok: bool,
    steps: Vec<serde_json::Value>,
}

pub async fn run_automation(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<RunAutomationBody>,
) -> ApiResult<AutomationRunResult> {
    let name = validate_session_name(&name)?;
    let automation = Automation::load(&state.home.automation_path(&name))?;
    // Every step parses before the first one runs, for the same reason as on the CLI: a typo on
    // step nineteen should not be discovered after eighteen side effects.
    let planned = automation.plan(&body.inputs)?;

    let session = state.session(body.session.as_deref()).await?;
    let store = state.runs()?;
    let record = store.start(RunKind::Automation, &session.name, &automation.name)?;

    let browser = BrowserHandle::connect(&session.endpoint).await?;
    let page = browser.page(session.target.as_deref()).await?;
    let artifacts = state.home.session_dir(&session.name);
    std::fs::create_dir_all(&artifacts)?;

    let mut steps = Vec::new();
    let mut ok = true;
    for step in &planned {
        match execute(&page, &step.action, &artifacts).await {
            Ok(outcome) => steps.push(serde_json::to_value(&outcome).unwrap_or_default()),
            Err(error) if step.optional => steps.push(serde_json::json!({
                "action": step.action.id(),
                "skipped": true,
                "error": error.to_string(),
            })),
            Err(error) => {
                steps.push(serde_json::json!({
                    "action": step.action.id(),
                    "error": error.to_string(),
                }));
                ok = false;
                break;
            }
        }
    }
    drop(browser);

    let result = AutomationRunResult { automation: automation.name, ok, steps };
    let body = serde_json::to_string(&result).ok();
    let status = if ok { RunStatus::Succeeded } else { RunStatus::Failed };
    store.finish(&record.id, status, body.as_deref(), None)?;
    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
pub struct RunAgentBody {
    task: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    promptware: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    /// How many agents to fire at this task, each in its own session.
    #[serde(default = "one")]
    count: usize,
}

fn one() -> usize {
    1
}

#[derive(Debug, Serialize)]
pub struct LaunchedAgent {
    run: String,
    session: String,
    pid: u32,
}

/// Launch agents and return immediately.
///
/// Always detached: an HTTP request that waited for an agent would sit open for minutes, and the
/// UI follows progress on `/api/events` anyway.
pub async fn run_agent(
    State(state): State<AppState>,
    Json(body): Json<RunAgentBody>,
) -> ApiResult<Vec<LaunchedAgent>> {
    if body.task.trim().is_empty() {
        return Err(ApiError { status: StatusCode::BAD_REQUEST, message: "task is empty".into() });
    }
    if body.count == 0 || body.count > 64 {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "count must be between 1 and 64".into(),
        });
    }

    let base = validate_session_name(body.session.as_deref().unwrap_or(&state.config.session))?;
    let names: Vec<String> = if body.count == 1 {
        vec![base]
    } else {
        (1..=body.count).map(|index| format!("{base}-{index}")).collect()
    };

    let runner = AgentRunner::new(&state.config.open_agents_bin, state.home.path());
    let store = state.runs()?;
    let mut launched = Vec::new();
    for name in names {
        let session = state.session(Some(&name)).await?;
        let task = AgentTask {
            task: body.task.clone(),
            session: session.name.clone(),
            promptware: body.promptware.clone().unwrap_or_else(|| state.config.promptware.clone()),
            provider: body.provider.clone(),
            model: body.model.clone(),
            effort: body.effort.clone(),
            timeout_seconds: body.timeout_seconds,
        };
        let record = store.start(RunKind::Agent, &session.name, &task.task)?;
        let log = state.home.session_dir(&session.name).join(format!("agent-{}.log", record.id));
        std::fs::create_dir_all(log.parent().unwrap())?;
        let mut child = runner.spawn(&task, Some(&log))?;
        let pid = child.id().unwrap_or(0);
        store.set_pid(&record.id, pid)?;

        // The child is watched by a task rather than forgotten: something has to notice the exit
        // and write the terminal status, or every agent run would sit at `running` forever. The
        // `finish` guard means a cancel that got there first still owns the verdict.
        let store_for_watch = state.runs()?;
        let id = record.id.clone();
        tokio::spawn(async move {
            let status = child.wait().await;
            let (run_status, error) = match status {
                Ok(status) if status.success() => (RunStatus::Succeeded, None),
                Ok(status) => (RunStatus::Failed, Some(format!("the agent exited with {status}"))),
                Err(error) => (RunStatus::Failed, Some(error.to_string())),
            };
            let _ = store_for_watch.finish(&id, run_status, None, error.as_deref());
        });

        launched.push(LaunchedAgent { run: record.id, session: session.name, pid });
    }
    Ok(Json(launched))
}

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    session: Option<String>,
    limit: Option<usize>,
}

pub async fn list_runs(
    State(state): State<AppState>,
    Query(query): Query<RunsQuery>,
) -> ApiResult<Vec<open_browser_core::runs::RunRecord>> {
    let store = state.runs()?;
    Ok(Json(store.list(query.session.as_deref(), query.limit.unwrap_or(50))?))
}

pub async fn show_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<open_browser_core::runs::RunRecord> {
    let store = state.runs()?;
    store.get(&id)?.map(Json).ok_or_else(|| ApiError {
        status: StatusCode::NOT_FOUND,
        message: format!("no run called {id}"),
    })
}

pub async fn cancel_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let store = state.runs()?;
    let run = store.get(&id)?.ok_or_else(|| ApiError {
        status: StatusCode::NOT_FOUND,
        message: format!("no run called {id}"),
    })?;
    // Claim first, kill second. The watcher task above sees the exit within milliseconds, and
    // whichever of the two claims the row decides whether this reads as cancelled or as failed.
    let claimed = store.finish(&id, RunStatus::Cancelled, None, Some("cancelled"))?;
    if !claimed {
        return Err(ApiError {
            status: StatusCode::CONFLICT,
            message: format!("run {id} had already finished"),
        });
    }
    if let Some(pid) = run.pid {
        terminate(pid);
    }
    Ok(Json(serde_json::json!({ "cancelled": id })))
}

pub async fn run_log(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<String, ApiError> {
    let store = state.runs()?;
    let run = store.get(&id)?.ok_or_else(|| ApiError {
        status: StatusCode::NOT_FOUND,
        message: format!("no run called {id}"),
    })?;
    let path = state.home.session_dir(&run.session).join(format!("agent-{id}.log"));
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

pub async fn show_config(State(state): State<AppState>) -> ApiResult<Config> {
    Ok(Json((*state.config).clone()))
}
