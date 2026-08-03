//! Module: `casm_lsp::server`
//! Purpose: The only module that speaks JSON-RPC — a thin adapter over the pure analysis.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Thin on purpose
//!
//! Every handler here does the same three things: pull the document from the store, call
//! one pure function, convert the answer into protocol types. No decision about *what* to
//! say is made in this file, which is what keeps the interesting behaviour testable
//! without a mock editor.
//!
//! # Panic isolation
//!
//! Each handler runs inside a `catch_unwind` guard. A panic becomes an error response and a log
//! line; the session survives. See `docs/adr/0008-unwinding-for-lsp-panic-isolation.md`
//! for why release builds keep unwinding, and for why this is a last line of defence
//! rather than a licence to panic.

use casm_renderer::{Mermaid, Renderer as _};
use casm_validator::ValidatorConfig;
use std::panic::AssertUnwindSafe;
use std::sync::Mutex;
use tower_lsp_server::jsonrpc::{Error as RpcError, Result as RpcResult};
use tower_lsp_server::ls_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, Command, CompletionItem, CompletionItemKind,
    CompletionOptions, CompletionParams, CompletionResponse, Diagnostic, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse, Documentation,
    ExecuteCommandOptions, ExecuteCommandParams, GotoDefinitionParams, GotoDefinitionResponse,
    Hover, HoverContents, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
    InitializedParams, Location, MarkupContent, MarkupKind, MessageType, NumberOrString, OneOf,
    Position, Range, ReferenceParams, ServerCapabilities, ServerInfo, SymbolKind,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Uri, WorkspaceEdit,
};
use tower_lsp_server::{Client, LanguageServer};

use crate::actions::{self, ActionKind, GENERATE_DIAGRAM, VALIDATE_WORKSPACE};
use crate::completion::{self, ItemKind};
use crate::diagnostics::Severity;
use crate::documents::{DocumentStore, Limits};
use crate::hover;
use crate::navigation;
use crate::text::{Position as CasmPosition, Span};

/// The language server.
pub struct Backend {
    client: Client,
    store: Mutex<DocumentStore>,
}

impl Backend {
    /// Constructs a backend bound to `client`.
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self {
            client,
            store: Mutex::new(DocumentStore::new(
                Limits::default(),
                ValidatorConfig::default(),
            )),
        }
    }

    /// Analyses a document and pushes its diagnostics to the client.
    async fn refresh(&self, uri: &Uri, text: String, version: i32) {
        let published = {
            let Ok(mut store) = self.store.lock() else {
                // A poisoned lock means a previous panic was contained. Reporting it is
                // more useful than silently serving stale analyses.
                return;
            };
            let document = store.upsert(uri.as_str(), text, version);
            document
                .diagnostics()
                .iter()
                .map(convert_diagnostic)
                .collect::<Vec<_>>()
        };

        self.client
            .publish_diagnostics(uri.clone(), published, Some(version))
            .await;
    }

    /// Runs `operation` against the stored document for `uri`, if it is open.
    fn with_document<T>(
        &self,
        uri: &Uri,
        operation: impl FnOnce(&crate::documents::Document) -> T,
    ) -> Option<T> {
        let mut store = self.store.lock().ok()?;
        let document = store.get(uri.as_str())?;
        Some(operation(document))
    }
}

/// Runs `operation`, converting a panic into an error response instead of a dead session.
fn guarded<T>(operation_name: &'static str, operation: impl FnOnce() -> T) -> RpcResult<T> {
    std::panic::catch_unwind(AssertUnwindSafe(operation)).map_err(|_| {
        let mut error = RpcError::internal_error();
        error.message = format!(
            "casm-lsp: the '{operation_name}' handler panicked. The request failed; the \
             server is still running. Please report this with the document that triggered it."
        )
        .into();
        error
    })
}

// Every handler below is `async` because the `LanguageServer` trait declares it so; the
// signatures are not ours to choose. Clippy's suggestion — return `std::future::ready`
// instead — cannot be taken for a trait method, so the lint is a false positive here.
// `unused_async_trait_impl` is nightly-only, so stable clippy would reject the attribute
// itself under `-D warnings`. `unknown_lints` makes the allow portable across both.
#[allow(unknown_lints, clippy::unused_async_trait_impl)]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> RpcResult<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "casm-lsp".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
            // `None` selects the protocol default, UTF-16, which is exactly what
            // `crate::text` computes. Declaring anything else here without changing that
            // module would shift every span on a line containing a non-ASCII character.
            offset_encoding: None,
            capabilities: ServerCapabilities {
                // Full sync: architecture documents are small, and re-analysing the whole
                // text is microseconds. Incremental sync would add a rope and a class of
                // desynchronisation bug for no measurable gain.
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    // `:` and space fire after a key; `-` fires on a new sequence entry.
                    trigger_characters: Some(vec![":".to_owned(), " ".to_owned(), "-".to_owned()]),
                    ..CompletionOptions::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![GENERATE_DIAGRAM.to_owned(), VALIDATE_WORKSPACE.to_owned()],
                    ..ExecuteCommandOptions::default()
                }),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "casm-lsp ready — architecture as code")
            .await;
    }

    async fn shutdown(&self) -> RpcResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let document = params.text_document;
        self.refresh(&document.uri, document.text, document.version)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // Full sync, so the last change carries the entire document.
        let Some(change) = params.content_changes.into_iter().next_back() else {
            return;
        };
        self.refresh(
            &params.text_document.uri,
            change.text,
            params.text_document.version,
        )
        .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        if let Ok(mut store) = self.store.lock() {
            store.close(params.text_document.uri.as_str());
        }
        // Clear the squiggles: a closed document has no findings to show.
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
    }

    async fn completion(&self, params: CompletionParams) -> RpcResult<Option<CompletionResponse>> {
        let location = params.text_document_position;
        let position = convert_position(location.position);

        guarded("completion", || {
            self.with_document(&location.text_document.uri, |document| {
                let result = completion::complete(&document.index, position);
                CompletionResponse::Array(result.items.iter().map(convert_completion).collect())
            })
        })
    }

    async fn hover(&self, params: HoverParams) -> RpcResult<Option<Hover>> {
        let location = params.text_document_position_params;
        let position = convert_position(location.position);

        guarded("hover", || {
            self.with_document(&location.text_document.uri, |document| {
                hover::hover(
                    &document.index,
                    document.analysis.architecture.as_ref(),
                    position,
                )
                .map(|found| Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: found.markdown,
                    }),
                    range: Some(convert_span(found.span)),
                })
            })
            .flatten()
        })
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> RpcResult<Option<GotoDefinitionResponse>> {
        let location = params.text_document_position_params;
        let uri = location.text_document.uri.clone();
        let position = convert_position(location.position);

        guarded("goto_definition", || {
            self.with_document(&uri, |document| {
                navigation::definition(&document.index, position).map(|span| {
                    GotoDefinitionResponse::Scalar(Location {
                        uri: uri.clone(),
                        range: convert_span(span),
                    })
                })
            })
            .flatten()
        })
    }

    async fn references(&self, params: ReferenceParams) -> RpcResult<Option<Vec<Location>>> {
        let location = params.text_document_position;
        let uri = location.text_document.uri.clone();
        let position = convert_position(location.position);
        let include_declaration = params.context.include_declaration;

        guarded("references", || {
            self.with_document(&uri, |document| {
                navigation::references(&document.index, position, include_declaration)
                    .into_iter()
                    .map(|span| Location {
                        uri: uri.clone(),
                        range: convert_span(span),
                    })
                    .collect::<Vec<_>>()
            })
        })
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> RpcResult<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;

        guarded("document_symbol", || {
            self.with_document(&uri, |document| {
                let symbols = navigation::outline(&document.index)
                    .into_iter()
                    .map(build_symbol)
                    .collect();
                DocumentSymbolResponse::Nested(symbols)
            })
        })
    }

    async fn code_action(&self, params: CodeActionParams) -> RpcResult<Option<CodeActionResponse>> {
        let uri = params.text_document.uri.clone();
        let requested = params.range;

        guarded("code_action", || {
            self.with_document(&uri, |document| {
                // The server's own diagnostics are used rather than the client's copy, so
                // a stale or lossy round-trip cannot suppress a fix.
                let mut offered: Vec<CodeActionOrCommand> = document
                    .diagnostics()
                    .iter()
                    .filter(|diagnostic| overlaps(diagnostic.span, requested))
                    .flat_map(|diagnostic| actions::quick_fixes(&document.index, diagnostic))
                    .map(|action| convert_action(&uri, action))
                    .collect();

                offered.extend(
                    actions::source_actions()
                        .into_iter()
                        .map(|action| convert_action(&uri, action)),
                );
                offered
            })
        })
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> RpcResult<Option<serde_json::Value>> {
        let uri = params
            .arguments
            .first()
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();

        match params.command.as_str() {
            GENERATE_DIAGRAM => {
                let diagram = self
                    .store
                    .lock()
                    .ok()
                    .and_then(|store| {
                        store.peek(uri).and_then(|document| {
                            document
                                .analysis
                                .architecture
                                .as_ref()
                                .map(|arch| Mermaid.render(arch))
                        })
                    })
                    .unwrap_or_else(|| {
                        "%% CASIMIR could not render: the document does not currently parse.\n"
                            .to_owned()
                    });

                Ok(Some(serde_json::Value::String(diagram)))
            }

            VALIDATE_WORKSPACE => {
                let summary = self.validate_workspace();
                Ok(Some(serde_json::Value::String(summary)))
            }

            other => {
                let mut error = RpcError::invalid_request();
                error.message = format!("casm-lsp: unknown command '{other}'").into();
                Err(error)
            }
        }
    }
}

impl Backend {
    /// Summarises the findings across every open document.
    ///
    /// Deliberately synchronous: it holds the store's `Mutex`, and holding a blocking
    /// lock across an `.await` is how an async runtime deadlocks itself.
    fn validate_workspace(&self) -> String {
        let Ok(store) = self.store.lock() else {
            return "casm-lsp: could not read the document store".to_owned();
        };

        let mut documents = 0_usize;
        let mut errors = 0_usize;
        let mut warnings = 0_usize;

        for uri in store.uris() {
            let Some(document) = store.peek(&uri) else {
                continue;
            };
            documents = documents.saturating_add(1);
            for diagnostic in document.diagnostics() {
                match diagnostic.severity {
                    Severity::Error => errors = errors.saturating_add(1),
                    Severity::Warning => warnings = warnings.saturating_add(1),
                    Severity::Info => {}
                }
            }
        }

        format!("checked {documents} open document(s): {errors} error(s), {warnings} warning(s)")
    }
}

/// Converts a protocol position into CASIMIR's.
fn convert_position(position: Position) -> CasmPosition {
    CasmPosition::new(position.line, position.character)
}

/// Converts a CASIMIR span into a protocol range.
fn convert_span(span: Span) -> Range {
    Range {
        start: Position {
            line: span.line,
            character: span.start,
        },
        end: Position {
            line: span.line,
            character: span.end,
        },
    }
}

/// Returns `true` if `span` intersects `range`, by line.
///
/// Line granularity is deliberate: an editor asks for actions at the cursor, and a fix
/// for the node on that line is what the author expects to be offered.
fn overlaps(span: Span, range: Range) -> bool {
    span.line >= range.start.line && span.line <= range.end.line
}

/// Converts a CASIMIR diagnostic into a protocol one.
fn convert_diagnostic(diagnostic: &crate::diagnostics::Diagnostic) -> Diagnostic {
    Diagnostic {
        range: convert_span(diagnostic.span),
        severity: Some(match diagnostic.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
            Severity::Info => DiagnosticSeverity::INFORMATION,
        }),
        code: Some(NumberOrString::String(diagnostic.code.clone())),
        code_description: None,
        source: Some("casm".to_owned()),
        message: diagnostic.message.clone(),
        related_information: None,
        tags: None,
        data: None,
    }
}

/// Converts an outline entry into a protocol document symbol.
///
/// `range` covers the whole node item so the editor can highlight the block, while
/// `selection_range` is just the name so "reveal" lands on it rather than on the `- `.
#[allow(deprecated)] // `DocumentSymbol::deprecated` is deprecated but not optional.
fn build_symbol(entry: navigation::Outline) -> DocumentSymbol {
    let full_line_end = Position {
        line: entry.end_line,
        character: u32::MAX,
    };

    DocumentSymbol {
        name: entry.name,
        detail: Some(entry.detail),
        kind: SymbolKind::CLASS,
        tags: None,
        deprecated: None,
        range: Range {
            start: Position {
                line: entry.start_line,
                character: 0,
            },
            end: full_line_end,
        },
        selection_range: convert_span(entry.selection_span),
        children: None,
    }
}

/// Converts a CASIMIR completion into a protocol item.
fn convert_completion(item: &completion::Completion) -> CompletionItem {
    CompletionItem {
        label: item.label.clone(),
        kind: Some(match item.kind {
            ItemKind::Field => CompletionItemKind::FIELD,
            ItemKind::Value => CompletionItemKind::ENUM_MEMBER,
            ItemKind::Reference => CompletionItemKind::REFERENCE,
        }),
        detail: Some(item.detail.clone()),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: item.documentation.clone(),
        })),
        insert_text: Some(item.insert_text.clone()),
        ..CompletionItem::default()
    }
}

/// Converts a CASIMIR code action into a protocol one.
fn convert_action(uri: &Uri, action: actions::CodeAction) -> CodeActionOrCommand {
    let edit = (!action.edits.is_empty()).then(|| {
        let edits: Vec<TextEdit> = action
            .edits
            .iter()
            .map(|edit| TextEdit {
                range: convert_span(edit.span),
                new_text: edit.new_text.clone(),
            })
            .collect();

        WorkspaceEdit {
            changes: Some([(uri.clone(), edits)].into_iter().collect()),
            ..WorkspaceEdit::default()
        }
    });

    CodeActionOrCommand::CodeAction(CodeAction {
        title: action.title.clone(),
        kind: Some(match action.kind {
            ActionKind::QuickFix => CodeActionKind::QUICKFIX,
            ActionKind::Source => CodeActionKind::SOURCE,
        }),
        edit,
        command: action.command.map(|command| Command {
            title: command.title,
            command: command.id,
            arguments: Some(vec![serde_json::Value::String(uri.as_str().to_owned())]),
        }),
        ..CodeAction::default()
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    #[test]
    fn positions_convert_in_both_directions_without_shifting() {
        let protocol = Position {
            line: 4,
            character: 11,
        };
        let casm = convert_position(protocol);
        assert_eq!(casm, CasmPosition::new(4, 11));

        let range = convert_span(Span::new(4, 11, 14));
        assert_eq!(range.start, protocol);
        assert_eq!(
            range.end,
            Position {
                line: 4,
                character: 14
            }
        );
    }

    #[test]
    fn a_span_converts_to_a_single_line_range() {
        let range = convert_span(Span::new(7, 2, 9));
        assert_eq!(
            range.start.line, range.end.line,
            "CASIMIR symbols never span lines"
        );
    }

    #[test]
    fn overlap_is_tested_by_line() {
        let range = Range {
            start: Position {
                line: 3,
                character: 0,
            },
            end: Position {
                line: 5,
                character: 0,
            },
        };
        assert!(overlaps(Span::new(3, 0, 1), range), "the first line");
        assert!(overlaps(Span::new(4, 0, 1), range), "a line inside");
        assert!(overlaps(Span::new(5, 0, 1), range), "the last line");
        assert!(!overlaps(Span::new(2, 0, 1), range));
        assert!(!overlaps(Span::new(6, 0, 1), range));
    }

    #[test]
    fn severities_map_onto_the_protocol() {
        let build = |severity| crate::diagnostics::Diagnostic {
            span: Span::new(0, 0, 1),
            severity,
            code: "r".to_owned(),
            message: "m".to_owned(),
            related: Vec::new(),
        };

        assert_eq!(
            convert_diagnostic(&build(Severity::Error)).severity,
            Some(DiagnosticSeverity::ERROR)
        );
        assert_eq!(
            convert_diagnostic(&build(Severity::Warning)).severity,
            Some(DiagnosticSeverity::WARNING)
        );
        assert_eq!(
            convert_diagnostic(&build(Severity::Info)).severity,
            Some(DiagnosticSeverity::INFORMATION)
        );
    }

    #[test]
    fn a_diagnostic_carries_the_rule_id_as_its_code_and_casm_as_its_source() {
        // The client shows these; a wrong source means findings are attributed elsewhere.
        let diagnostic = convert_diagnostic(&crate::diagnostics::Diagnostic {
            span: Span::new(2, 4, 7),
            severity: Severity::Warning,
            code: "no-isolated-nodes".to_owned(),
            message: "m".to_owned(),
            related: Vec::new(),
        });

        assert_eq!(diagnostic.source.as_deref(), Some("casm"));
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("no-isolated-nodes".to_owned()))
        );
    }

    #[test]
    fn completion_kinds_map_onto_the_protocol() {
        let build = |kind| completion::Completion {
            label: "x".to_owned(),
            insert_text: "x".to_owned(),
            detail: "d".to_owned(),
            documentation: "doc".to_owned(),
            kind,
        };

        assert_eq!(
            convert_completion(&build(ItemKind::Field)).kind,
            Some(CompletionItemKind::FIELD)
        );
        assert_eq!(
            convert_completion(&build(ItemKind::Value)).kind,
            Some(CompletionItemKind::ENUM_MEMBER)
        );
        assert_eq!(
            convert_completion(&build(ItemKind::Reference)).kind,
            Some(CompletionItemKind::REFERENCE)
        );
    }

    #[test]
    fn a_completion_carries_markdown_documentation_and_its_insert_text() {
        let item = convert_completion(&completion::Completion {
            label: "service".to_owned(),
            insert_text: "service".to_owned(),
            detail: "stateless".to_owned(),
            documentation: "**docs**".to_owned(),
            kind: ItemKind::Value,
        });

        assert_eq!(item.insert_text.as_deref(), Some("service"));
        assert!(matches!(
            item.documentation,
            Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                ..
            }))
        ));
    }

    #[test]
    fn guarded_returns_a_value_when_nothing_goes_wrong() {
        assert_eq!(guarded("test", || 42), Ok(42));
    }

    #[test]
    fn guarded_converts_a_panic_into_an_error_response() {
        // The whole point: a bug costs one request, not the session.
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = guarded("test", || panic!("boom"));
        std::panic::set_hook(previous);

        let error = result.unwrap_err();
        assert!(error.message.contains("panicked"), "{}", error.message);
        assert!(error.message.contains("still running"), "{}", error.message);
        assert!(
            error.message.contains("'test'"),
            "the handler is named: {}",
            error.message
        );
    }

    #[test]
    fn guarded_contains_an_index_out_of_bounds_panic() {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = guarded("indexing", || {
            let empty: Vec<u8> = Vec::new();
            empty[3]
        });
        std::panic::set_hook(previous);
        assert!(result.is_err());
    }
}
