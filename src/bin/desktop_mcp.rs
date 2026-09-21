//! Serveur MCP stdio du morph ordinateur.
#![cfg(windows)]

use locaryn_morph_kit::mcp::serve;
use locaryn_plugin_desktop::sys;
use locaryn_plugin_desktop::tools::{tools_list, Desktop};
use std::sync::Arc;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    sys::enable_dpi_awareness();
    sys::restore_cursors(); // un plantage précédent a pu laisser le réticule
    let desktop = Arc::new(Desktop::new());
    serve("plugin-desktop", VERSION, tools_list(), |name, args| {
        let desktop = desktop.clone();
        async move { desktop.call(&name, args).await }
    })
    .await;
}
