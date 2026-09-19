//! `/api/events`: run status, pushed.
//!
//! A fleet of agents is the case this exists for. Polling `/api/runs` from a browser showing
//! twenty concurrent agents means twenty times the queries and a UI that is always a second
//! stale, so the server watches the database and pushes what changed.

use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use open_browser_core::runs::{RunRecord, RunStatus};

use crate::state::AppState;

/// How often the database is re-read. Fast enough that a finished run appears promptly, slow
/// enough that an idle UI is not a busy loop against SQLite.
const POLL: Duration = Duration::from_millis(750);

pub async fn events(upgrade: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    upgrade.on_upgrade(move |socket| pump(socket, state))
}

async fn pump(mut socket: WebSocket, state: AppState) {
    let mut known: BTreeMap<String, RunStatus> = BTreeMap::new();
    let mut first = true;

    loop {
        let Ok(store) = state.runs() else { break };
        let Ok(runs) = store.list(None, 100) else { break };

        let mut changed: Vec<&RunRecord> = Vec::new();
        for run in &runs {
            match known.get(&run.id) {
                Some(status) if *status == run.status => {}
                // On the first pass everything is "changed", which is how the UI gets its initial
                // state without a separate fetch.
                _ => changed.push(run),
            }
        }
        for run in &runs {
            known.insert(run.id.clone(), run.status);
        }

        if !changed.is_empty() || first {
            let payload = serde_json::json!({
                "type": if first { "snapshot" } else { "update" },
                "runs": changed,
            });
            let text = serde_json::to_string(&payload).unwrap_or_default();
            if socket.send(Message::Text(text.into())).await.is_err() {
                // The client went away. Not an error worth logging: closing the tab does this.
                break;
            }
            first = false;
        }

        tokio::select! {
            _ = tokio::time::sleep(POLL) => {}
            message = socket.recv() => match message {
                // A close frame or a dead socket ends the pump; anything else the client sends is
                // ignored, because this channel is one-directional by design.
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {}
            },
        }
    }
}
