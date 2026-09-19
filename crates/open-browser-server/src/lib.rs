//! The HTTP API, and the web UI that sits on it.
//!
//! The API is the same three surfaces story as the CLI: `POST /api/actions/<id>` takes a JSON
//! object of parameters and hands it to [`Action::parse`] — the identical function `ob click`
//! calls — so an action cannot mean one thing over HTTP and another in the shell. Nothing here
//! enumerates actions; the routes come from the registry.

mod state;
mod ws;

use std::net::SocketAddr;

use anyhow::{Context as _, Result};
use axum::routing::{delete, get, post};
use axum::Router;
use open_browser_core::config::Config;
use open_browser_core::home::Home;
use std::path::PathBuf;

pub use state::AppState;

pub struct ServeConfig {
    pub home: Home,
    pub config: Config,
    pub bind: String,
    /// A directory of static files to serve at `/`. The built web UI, usually.
    pub ui: Option<PathBuf>,
}

/// A bound but not yet running server.
///
/// Splitting the bind from the serve exists for the desktop app. It starts the service in-process
/// on port 0 and has to know which port it got *before* the window opens, because the address is
/// injected into the webview as `window.__OPEN_BROWSER_API__`. Binding inside [`serve`] would mean
/// the port is only knowable once the future is already running and never returns.
pub struct Serving {
    pub address: SocketAddr,
    listener: tokio::net::TcpListener,
    router: Router,
}

impl Serving {
    /// Run until the process ends or the task is dropped.
    pub async fn run(self) -> Result<()> {
        axum::serve(self.listener, self.router).await.context("serving")?;
        Ok(())
    }
}

/// Claim the port. Port 0 in `bind` means "any free one", and [`Serving::address`] reports which.
pub async fn bind(config: ServeConfig) -> Result<Serving> {
    let requested = config.bind.clone();
    let requested: SocketAddr = requested
        .parse()
        .with_context(|| format!("'{requested}' is not an address; write it as 127.0.0.1:8787"))?;

    let listener = tokio::net::TcpListener::bind(requested)
        .await
        .with_context(|| format!("binding {requested}"))?;
    let address = listener.local_addr().unwrap_or(requested);
    Ok(Serving { address, listener, router: router(config) })
}

pub async fn serve(config: ServeConfig) -> Result<()> {
    let serving = bind(config).await?;
    println!("open-browser listening on http://{}", serving.address);
    serving.run().await
}

pub fn router(config: ServeConfig) -> Router {
    let ui = config.ui.clone();
    let state = AppState::new(config);

    let api = Router::new()
        .route("/health", get(state::health))
        .route("/actions", get(state::list_actions))
        .route("/actions/{id}", get(state::show_action).post(state::run_action))
        .route("/sessions", get(state::list_sessions).post(state::start_session))
        .route("/sessions/{name}", delete(state::stop_session))
        .route("/automations", get(state::list_automations))
        .route("/automations/{name}", get(state::show_automation))
        .route("/automations/{name}/run", post(state::run_automation))
        .route("/agents", post(state::run_agent))
        .route("/runs", get(state::list_runs))
        .route("/runs/{id}", get(state::show_run))
        .route("/runs/{id}/cancel", post(state::cancel_run))
        .route("/runs/{id}/log", get(state::run_log))
        .route("/config", get(state::show_config))
        .route("/events", get(ws::events))
        .with_state(state);

    let mut router = Router::new().nest("/api", api).layer(
        // The UI is served from this same origin in the normal case; CORS is permissive because
        // the alternative is a Vite dev server on another port being unable to talk to it, and
        // this binds to loopback by default.
        tower_http::cors::CorsLayer::permissive(),
    );

    if let Some(dir) = ui {
        // `fallback` rather than a nested route: the UI is a single-page app, so a deep link like
        // /runs/00007 has to return index.html instead of a 404.
        let index = dir.join("index.html");
        router = router.fallback_service(
            tower_http::services::ServeDir::new(dir)
                .not_found_service(tower_http::services::ServeFile::new(index)),
        );
    }
    router.layer(tower_http::trace::TraceLayer::new_for_http())
}
