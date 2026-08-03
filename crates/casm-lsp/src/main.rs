//! Module: `casm-lsp` binary
//! Purpose: The process entry point — wire the server to stdio and serve.
//! Safety: `#![forbid(unsafe_code)]` — inherited from the library crate.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # Standard output belongs to the protocol
//!
//! LSP speaks JSON-RPC over stdin and stdout. A stray `println!` anywhere in the process
//! corrupts the framing, and the editor drops the connection with no useful error.
//!
//! Nothing in `casm-lsp` writes to stdout: logging goes to the client via
//! `window/logMessage`, and panic output goes to stderr, which editors capture
//! separately. This is also why the language server is a separate binary rather than a
//! `casm` subcommand — `casm validate` prints to stdout by design.

#![forbid(unsafe_code)]

use casm_lsp::Backend;
use tower_lsp_server::{LspService, Server};

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
