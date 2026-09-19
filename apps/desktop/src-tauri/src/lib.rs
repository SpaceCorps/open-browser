//! The desktop shell: `ob serve` and the web UI in one window.
//!
//! There is no second implementation of anything here. The window loads the same `apps/web` bundle
//! a browser would, and it talks to the same `open-browser-server` the `ob serve` command runs. The
//! only thing this crate adds is starting that service in-process and telling the webview where it
//! ended up.

use std::net::SocketAddr;

use anyhow::{Context as _, Result};
use open_browser_core::config::Config;
use open_browser_core::home::Home;
use open_browser_server::{bind, ServeConfig};
use tauri::{WebviewUrl, WebviewWindowBuilder};

/// Start the service, then the window.
///
/// The order is the whole design. The webview learns the API's address from an initialisation
/// script, and a script cannot be injected into a page that has already loaded — so the port has to
/// be known before the window is created. [`open_browser_server::bind`] exists for this: it claims
/// the port and hands back the address without consuming the future that serves on it.
pub fn run() {
    init_tracing();

    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => fail(&format!("could not start the async runtime: {error}")),
    };

    // Held for the lifetime of `run`, which is the lifetime of the app: dropping the runtime would
    // stop the service the window is talking to.
    let address = match runtime.block_on(start_service()) {
        Ok(address) => address,
        Err(error) => {
            let mut message = format!("{error}");
            for cause in error.chain().skip(1) {
                message.push_str(&format!("\n  caused by: {cause}"));
            }
            fail(&message)
        }
    };
    let origin = format!("http://{address}");
    tracing::info!("the service is on {origin}");

    // `serde_json` rather than string interpolation because this value crosses into JavaScript. The
    // address comes from the OS today, but quoting it properly costs nothing and means a future
    // `--api` flag cannot turn a hostname into script.
    let script = format!(
        "window.__OPEN_BROWSER_API__ = {};",
        serde_json::to_string(&origin).expect("a string is always valid JSON")
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            // The window is built here rather than declared in `tauri.conf.json` because a window
            // from the config is created before this hook runs, and it would therefore load the
            // page without the script above.
            WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("open-browser")
                .inner_size(1280.0, 840.0)
                .min_inner_size(900.0, 560.0)
                .resizable(true)
                .initialization_script(&script)
                .build()?;
            Ok(())
        })
        // No `invoke_handler`: the UI reaches the service over HTTP like any other client, so the
        // app exposes no IPC command of its own for a page to call.
        .run(tauri::generate_context!())
        .expect("starting the desktop app");
}

/// Bind the service on a free loopback port and leave it running on the current runtime.
async fn start_service() -> Result<SocketAddr> {
    let home = Home::resolve(None).context("locating the open-browser home directory")?;
    home.ensure()?;
    let config = Config::load(&home.config_path())?;

    // Port 0, and loopback only. The desktop app's service is for the window in front of you; the
    // configured `bind` address belongs to `ob serve`, which is the deliberate way to expose it.
    let serving = bind(ServeConfig {
        home,
        config,
        bind: "127.0.0.1:0".to_string(),
        // Tauri serves the UI from `frontendDist`, so the service only answers `/api`.
        ui: None,
    })
    .await?;

    let address = serving.address;
    tokio::spawn(async move {
        if let Err(error) = serving.run().await {
            tracing::error!("the service stopped: {error:#}");
        }
    });
    Ok(address)
}

/// A bundled app has no terminal to print to, so the message goes to the log as well as stderr.
fn fail(message: &str) -> ! {
    tracing::error!("{message}");
    eprintln!("error: {message}");
    std::process::exit(1)
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("OPEN_BROWSER_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}
