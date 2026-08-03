//! End-to-end tests that drive the server over real JSON-RPC framing.
//!
//! The unit tests elsewhere exercise the analysis directly, which is where the interesting
//! behaviour lives. These exist to prove the *wiring*: that capabilities are advertised,
//! that `didOpen` produces diagnostics, and that a request returns a well-formed response
//! over the actual transport rather than through a function call.
//!
//! Framing is `Content-Length: N\r\n\r\n{json}`, the same as an editor sends.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use casm_lsp::Backend;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tower_lsp_server::{LspService, Server};

/// Wraps a JSON-RPC message in LSP framing.
fn frame(message: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{message}", message.len())
}

/// Reads exactly one framed message from `reader`.
///
/// Used as a synchronisation barrier: the server dispatches requests concurrently, so
/// anything sent before the `initialize` *response* has been read races the handshake and
/// is rejected with "Server not initialized". Waiting for that one response is what makes
/// these tests deterministic rather than timing-dependent.
async fn read_message<R>(reader: &mut BufReader<R>) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut length = 0_usize;

    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).await.unwrap_or(0) == 0 {
            return String::new();
        }
        let header = header.trim();
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }

    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).await.expect("a complete body");
    String::from_utf8_lossy(&body).into_owned()
}

/// Runs `messages` through a fresh server and returns everything it wrote.
///
/// The first message must be `initialize`.
async fn exchange(messages: &[&str]) -> String {
    // A hang here means a protocol deadlock; failing fast beats a wedged CI job.
    tokio::time::timeout(std::time::Duration::from_secs(10), drive(messages))
        .await
        .expect("the server did not finish within 10s")
}

/// Feeds `messages` to a server over an in-memory duplex and collects its output.
async fn drive(messages: &[&str]) -> String {
    let (client, server) = tokio::io::duplex(1 << 20);
    let (server_read, server_write) = tokio::io::split(server);

    let (service, socket) = LspService::new(Backend::new);
    let serving = tokio::spawn(async move {
        Server::new(server_read, server_write, socket)
            .serve(service)
            .await;
    });

    let (client_read, mut client_write) = tokio::io::split(client);
    let mut reader = BufReader::new(client_read);

    let mut transcript = String::new();

    let Some((first, rest)) = messages.split_first() else {
        return transcript;
    };

    client_write
        .write_all(frame(first).as_bytes())
        .await
        .expect("write initialize");
    transcript.push_str(&read_message(&mut reader).await);

    for message in rest {
        client_write
            .write_all(frame(message).as_bytes())
            .await
            .expect("write");
    }

    // Drain until the server goes quiet.
    //
    // Neither EOF nor the `shutdown` reply works as a terminator: notifications such as
    // `publishDiagnostics` are pumped through a separate channel from request responses,
    // so a reply to a later request can overtake an earlier notification, and closing the
    // input ends the serve loop before pending notifications are flushed. Waiting for a
    // quiet period collects everything regardless of the order it was produced in.
    loop {
        match tokio::time::timeout(IDLE, read_message(&mut reader)).await {
            Ok(message) if !message.is_empty() => transcript.push_str(&message),
            _ => break,
        }
    }

    // `shutdown`, not `drop`: the halves of a split stream share one object, so dropping
    // the writer alone never signals EOF and the server would wait for input forever.
    client_write
        .shutdown()
        .await
        .expect("shutdown the write half");
    let _ = serving.await;

    transcript
}

/// How long the server must stay silent before a transcript is considered complete.
///
/// Generous: the work per message is microseconds, so this only ever elapses when there
/// is genuinely nothing left to send.
const IDLE: std::time::Duration = std::time::Duration::from_millis(750);

const INITIALIZE: &str =
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
/// Ending the input stream shuts the server down *after* it has answered everything in
/// flight. Sending `exit` instead would cancel pending requests, which is correct
/// protocol behaviour and useless for testing responses.
const SHUTDOWN: &str = r#"{"jsonrpc":"2.0","id":9000,"method":"shutdown"}"#;

/// A `didOpen` notification carrying `text`.
fn did_open(text: &str) -> String {
    let escaped = serde_json::to_string(text).expect("text serialises");
    format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///a.yaml","languageId":"casm","version":1,"text":{escaped}}}}}}}"#
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_advertises_the_capabilities_it_implements() {
    let response = exchange(&[INITIALIZE, SHUTDOWN]).await;

    for capability in [
        "completionProvider",
        "hoverProvider",
        "definitionProvider",
        "referencesProvider",
        "documentSymbolProvider",
        "codeActionProvider",
        "executeCommandProvider",
    ] {
        assert!(
            response.contains(capability),
            "{capability} was not advertised:\n{response}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_identifies_itself_with_its_version() {
    let response = exchange(&[INITIALIZE, SHUTDOWN]).await;
    assert!(response.contains("casm-lsp"), "{response}");
    assert!(response.contains(env!("CARGO_PKG_VERSION")), "{response}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_registered_commands_are_advertised() {
    let response = exchange(&[INITIALIZE, SHUTDOWN]).await;
    assert!(response.contains("casm.generateDiagram"), "{response}");
    assert!(response.contains("casm.validateWorkspace"), "{response}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_a_document_publishes_diagnostics() {
    let source = "name: x\nnodes:\n  - name: api\n    type: service\n";
    let response = exchange(&[INITIALIZE, INITIALIZED, &did_open(source), SHUTDOWN]).await;

    assert!(
        response.contains("publishDiagnostics"),
        "no diagnostics were pushed:\n{response}"
    );
    assert!(
        response.contains("services-require-security-controls"),
        "the expected rule did not reach the client:\n{response}"
    );
    assert!(response.contains("\"source\":\"casm\""), "{response}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_a_broken_document_publishes_a_syntax_diagnostic() {
    let source = "name: x\nnodes:\n  - name: api\n    type: srvice\n";
    let response = exchange(&[INITIALIZE, INITIALIZED, &did_open(source), SHUTDOWN]).await;

    assert!(response.contains("publishDiagnostics"), "{response}");
    assert!(response.contains("syntax"), "{response}");
    assert!(
        response.contains("did you mean"),
        "the suggestion must survive:\n{response}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_a_clean_document_publishes_an_empty_diagnostic_list() {
    // Publishing an empty list is how previous squiggles get cleared; staying silent
    // would leave stale errors on screen forever.
    let source = "\
name: x
nodes:
  - name: gateway
    type: gateway
    interfaces:
      - name: public
        protocol: http2
        version: 1.0.0
    controls:
      - type: security
        standard: OIDC
        description: tokens required
      - type: security
        standard: TLS
        description: mutual TLS
  - name: db
    type: database
    interfaces:
      - name: sql
        protocol: sql
        version: 16.0.0
    controls:
      - type: security
        standard: ENC
        description: encrypted at rest
relationships:
  - source: gateway
    target: db
    type: sync
    protocol: sql
    latency-budget-ms: 40
";
    let response = exchange(&[INITIALIZE, INITIALIZED, &did_open(source), SHUTDOWN]).await;

    assert!(response.contains("publishDiagnostics"), "{response}");
    assert!(
        response.contains("\"diagnostics\":[]"),
        "expected an empty list:\n{response}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completion_returns_node_types_over_the_wire() {
    let source = "name: x\nnodes:\n  - name: api\n    type: \n";
    let request = r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///a.yaml"},"position":{"line":3,"character":10}}}"#;

    let response = exchange(&[
        INITIALIZE,
        INITIALIZED,
        &did_open(source),
        request,
        SHUTDOWN,
    ])
    .await;

    assert!(
        response.contains("\"id\":2"),
        "no response to the request:\n{response}"
    );
    assert!(response.contains("database"), "{response}");
    assert!(response.contains("external-system"), "{response}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hover_returns_markdown_over_the_wire() {
    let source = "name: x\nnodes:\n  - name: api\n    type: service\n";
    let request = r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///a.yaml"},"position":{"line":2,"character":11}}}"#;

    let response = exchange(&[
        INITIALIZE,
        INITIALIZED,
        &did_open(source),
        request,
        SHUTDOWN,
    ])
    .await;

    assert!(response.contains("\"id\":3"), "{response}");
    assert!(response.contains("markdown"), "{response}");
    assert!(response.contains("**api**"), "{response}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn go_to_definition_resolves_an_endpoint_over_the_wire() {
    let source = "\
name: x
nodes:
  - name: api
    type: service
  - name: db
    type: database
relationships:
  - source: api
    target: db
    type: sync
";
    // Line 7 is `  - source: api`; character 13 is inside `api`.
    let request = r#"{"jsonrpc":"2.0","id":4,"method":"textDocument/definition","params":{"textDocument":{"uri":"file:///a.yaml"},"position":{"line":7,"character":13}}}"#;

    let response = exchange(&[
        INITIALIZE,
        INITIALIZED,
        &did_open(source),
        request,
        SHUTDOWN,
    ])
    .await;

    assert!(response.contains("\"id\":4"), "{response}");
    assert!(
        response.contains("\"line\":2"),
        "must jump to the declaration:\n{response}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_for_an_unopened_document_returns_null_rather_than_an_error() {
    let request = r#"{"jsonrpc":"2.0","id":5,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///never-opened.yaml"},"position":{"line":0,"character":0}}}"#;

    let response = exchange(&[INITIALIZE, INITIALIZED, request, SHUTDOWN]).await;

    assert!(response.contains("\"id\":5"), "{response}");
    assert!(
        !response.contains("\"error\""),
        "an absent document is not an error:\n{response}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_command_is_reported_as_an_error() {
    let request = r#"{"jsonrpc":"2.0","id":6,"method":"workspace/executeCommand","params":{"command":"casm.nonexistent","arguments":[]}}"#;

    let response = exchange(&[INITIALIZE, INITIALIZED, request, SHUTDOWN]).await;

    assert!(response.contains("\"id\":6"), "{response}");
    assert!(response.contains("error"), "{response}");
    assert!(
        response.contains("casm.nonexistent"),
        "the message should name it:\n{response}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_diagram_command_renders_the_open_document() {
    let source = "\
name: x
nodes:
  - name: api
    type: service
  - name: db
    type: database
relationships:
  - source: api
    target: db
    type: sync
";
    let request = r#"{"jsonrpc":"2.0","id":7,"method":"workspace/executeCommand","params":{"command":"casm.generateDiagram","arguments":["file:///a.yaml"]}}"#;

    let response = exchange(&[
        INITIALIZE,
        INITIALIZED,
        &did_open(source),
        request,
        SHUTDOWN,
    ])
    .await;

    assert!(response.contains("\"id\":7"), "{response}");
    assert!(response.contains("flowchart LR"), "{response}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_a_document_clears_its_diagnostics() {
    let source = "name: x\nnodes:\n  - name: api\n    type: service\n";
    let close = r#"{"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":"file:///a.yaml"}}}"#;

    let response = exchange(&[INITIALIZE, INITIALIZED, &did_open(source), close, SHUTDOWN]).await;

    assert!(
        response.contains("\"diagnostics\":[]"),
        "closing must publish an empty list:\n{response}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_survives_a_document_of_pathological_content() {
    // Nothing here should parse, and none of it should take the session down.
    for source in [
        "",
        ":::::",
        "\t\t\t",
        "🚀🚀🚀",
        "nodes:\n  - \n",
        &"a".repeat(10_000),
    ] {
        let response = exchange(&[INITIALIZE, INITIALIZED, &did_open(source), SHUTDOWN]).await;
        assert!(
            response.contains("publishDiagnostics"),
            "died on {source:?}"
        );
    }
}
