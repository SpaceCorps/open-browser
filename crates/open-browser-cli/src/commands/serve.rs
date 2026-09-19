//! `ob serve`: the HTTP API and the web UI.
//!
//! The server lives in its own crate so it can also be deployed on its own, but `ob serve` is how
//! you run it from the machine the browser is on — which is almost always the same machine.

use anyhow::Result;
use open_browser_server::{serve, ServeConfig};

use crate::cli::ServeArgs;
use crate::context::Context;

pub async fn execute(context: &Context, args: ServeArgs) -> Result<()> {
    context.home.ensure()?;
    let bind = args.bind.unwrap_or_else(|| context.config.bind.clone());
    let config = ServeConfig {
        home: context.home.clone(),
        config: context.config.clone(),
        bind,
        ui: args.ui,
    };
    serve(config).await
}
