//! Module: `casm_lsp::documents`
//! Purpose: Holding open documents and their analyses, under a hard memory bound.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # NASA Rule 5, applied to a long-lived process
//!
//! A batch tool can be careless about memory; it exits. A language server runs for a
//! working day, and an editor may hand it every YAML file in a monorepo. Without a bound,
//! "CASIMIR ate 8 GB" is a matter of how large the repository is.
//!
//! So the store enforces two limits — a document count and a total byte budget — and
//! evicts least-recently-used entries to stay inside them. An evicted document is not
//! lost: the client still holds the text, and the next request re-analyses it.
//!
//! # A logical clock, not a wall clock
//!
//! The roadmap specifies dropping a document's AST after five minutes of inactivity.
//! This uses a monotonically increasing access counter instead, for two reasons: the
//! bound it enforces is on *memory*, which is what actually matters, and it is
//! deterministic, so eviction is testable without sleeping or injecting a clock.
//!
//! Time-based expiry would additionally free memory in an idle editor, which this does
//! not. That is a real difference, and the byte cap is what makes it acceptable: the
//! ceiling holds whether the server is busy or idle.

use casm_core::Pattern;
use casm_validator::ValidatorConfig;
use std::collections::HashMap;
use std::path::Path;

use crate::diagnostics::{Analysis, Diagnostic, analyse};
use crate::index::DocumentIndex;

/// How much the store may retain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most documents to keep analysed at once.
    pub max_documents: usize,
    /// The total source bytes to keep at once.
    pub max_total_bytes: usize,
}

impl Default for Limits {
    /// Generous for an architecture repository, still a firm ceiling.
    ///
    /// Sixty-four documents at 8 MiB total is far beyond any plausible working set — a
    /// large architecture file is a few kilobytes — while bounding the analysed state at
    /// roughly a few tens of megabytes once indices are counted.
    fn default() -> Self {
        Self {
            max_documents: 64,
            max_total_bytes: 8 * 1024 * 1024,
        }
    }
}

/// One open document and everything derived from it.
#[derive(Clone, Debug)]
pub struct Document {
    /// The full source, as the client last sent it.
    pub text: String,
    /// The client's version counter.
    pub version: i32,
    /// The position-aware index.
    pub index: DocumentIndex,
    /// The analysis: architecture and diagnostics.
    pub analysis: Analysis,
    /// Logical access time, for least-recently-used eviction.
    accessed: u64,
}

impl Document {
    /// The document's diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.analysis.diagnostics
    }

    /// How many source bytes this document occupies.
    #[must_use]
    pub fn size(&self) -> usize {
        self.text.len()
    }
}

/// The set of documents the server currently has analysed.
#[derive(Debug)]
pub struct DocumentStore {
    documents: HashMap<String, Document>,
    limits: Limits,
    config: ValidatorConfig,
    patterns: Vec<Pattern>,
    clock: u64,
}

impl DocumentStore {
    /// A store with the given limits and validator configuration.
    ///
    /// The pattern library starts empty, which is the honest state before a workspace has
    /// been resolved: every conformance claim is reported as unchecked until
    /// [`Self::load_patterns`] supplies one.
    #[must_use]
    pub fn new(limits: Limits, config: ValidatorConfig) -> Self {
        Self {
            documents: HashMap::new(),
            limits,
            config,
            patterns: Vec::new(),
            clock: 0,
        }
    }

    /// The validator configuration in force.
    #[must_use]
    pub const fn config(&self) -> &ValidatorConfig {
        &self.config
    }

    /// The pattern library in force.
    #[must_use]
    pub fn patterns(&self) -> &[Pattern] {
        &self.patterns
    }

    /// Replaces the validator configuration and re-analyses every open document.
    ///
    /// Called when the client changes settings. Re-analysing eagerly means the next
    /// keystroke shows diagnostics consistent with the new thresholds rather than stale
    /// ones.
    pub fn reconfigure(&mut self, config: ValidatorConfig) {
        self.config = config;
        self.reanalyse_all();
    }

    /// Replaces the pattern library and re-analyses every open document.
    ///
    /// Called when the workspace is resolved and again whenever a pattern file changes.
    /// Re-analysing matters more here than for a threshold change: editing a pattern moves
    /// findings in every document that claims it, none of which the author has touched.
    pub fn load_patterns(&mut self, patterns: Vec<Pattern>) {
        self.patterns = patterns;
        self.reanalyse_all();
    }

    /// Re-runs analysis over every open document, in place.
    fn reanalyse_all(&mut self) {
        let uris: Vec<String> = self.documents.keys().cloned().collect();
        for uri in uris {
            let Some(existing) = self.documents.get(&uri) else {
                continue;
            };
            let (text, version) = (existing.text.clone(), existing.version);
            self.upsert(&uri, text, version);
        }
    }

    /// Stores `text` for `uri`, analysing it.
    ///
    /// Analysis runs eagerly on every change rather than behind a debounce timer. It is a
    /// single pass over the lines plus one parse, which is microseconds for any document
    /// under the size cap — cheaper than the machinery a debounce would need, and it
    /// means diagnostics never lag the text.
    pub fn upsert(&mut self, uri: &str, text: String, version: i32) -> &Document {
        let index = DocumentIndex::build(&text);
        let path = uri_to_path(uri);
        let analysis = analyse(&text, &path, &index, &self.config, &self.patterns);

        self.clock = self.clock.saturating_add(1);
        let accessed = self.clock;

        self.documents.insert(
            uri.to_owned(),
            Document {
                text,
                version,
                index,
                analysis,
                accessed,
            },
        );

        self.enforce_limits(uri);

        // Present unless the document alone exceeds every limit, in which case an empty
        // placeholder is preferable to an unwrap.
        self.documents
            .entry(uri.to_owned())
            .or_insert_with(|| Document {
                text: String::new(),
                version,
                index: DocumentIndex::default(),
                analysis: Analysis::default(),
                accessed,
            })
    }

    /// Retrieves a document, marking it as recently used.
    pub fn get(&mut self, uri: &str) -> Option<&Document> {
        self.clock = self.clock.saturating_add(1);
        let accessed = self.clock;

        let document = self.documents.get_mut(uri)?;
        document.accessed = accessed;
        Some(document)
    }

    /// Retrieves a document together with the library its features need.
    ///
    /// Completion and hover both need the document *and* the pattern library, which live
    /// on different fields; handing back a pair is what lets a caller holding one lock
    /// borrow both.
    pub fn get_with_patterns(&mut self, uri: &str) -> Option<(&Document, &[Pattern])> {
        self.clock = self.clock.saturating_add(1);
        let accessed = self.clock;

        let document = self.documents.get_mut(uri)?;
        document.accessed = accessed;
        Some((document, &self.patterns))
    }

    /// Retrieves a document without affecting eviction order.
    #[must_use]
    pub fn peek(&self, uri: &str) -> Option<&Document> {
        self.documents.get(uri)
    }

    /// Forgets a document the client has closed.
    pub fn close(&mut self, uri: &str) -> Option<Document> {
        self.documents.remove(uri)
    }

    /// How many documents are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    /// Returns `true` if nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    /// The total source bytes held.
    #[must_use]
    pub fn total_bytes(&self) -> usize {
        self.documents.values().map(Document::size).sum()
    }

    /// The URIs currently held, in arbitrary order.
    #[must_use]
    pub fn uris(&self) -> Vec<String> {
        self.documents.keys().cloned().collect()
    }

    /// Evicts least-recently-used documents until both limits are satisfied.
    ///
    /// `protected` is never evicted: dropping the document the client just sent would be
    /// visibly broken, however large it is.
    fn enforce_limits(&mut self, protected: &str) {
        while self.documents.len() > self.limits.max_documents
            || self.total_bytes() > self.limits.max_total_bytes
        {
            let Some(victim) = self.least_recently_used(protected) else {
                break;
            };
            self.documents.remove(&victim);
        }
    }

    /// Finds the least recently used document, other than `protected`.
    fn least_recently_used(&self, protected: &str) -> Option<String> {
        self.documents
            .iter()
            .filter(|(uri, _)| uri.as_str() != protected)
            .min_by_key(|(uri, document)| (document.accessed, (*uri).clone()))
            .map(|(uri, _)| uri.clone())
    }
}

impl Default for DocumentStore {
    fn default() -> Self {
        Self::new(Limits::default(), ValidatorConfig::default())
    }
}

/// Converts a document or workspace URI into a path.
///
/// Total. A URI that does not decode cleanly is used verbatim rather than failing: for a
/// document the path only reaches parse-error messages, and for a workspace folder a
/// nonsensical path simply finds no pattern library.
///
/// Two details are not cosmetic. A Windows URI is `file:///C:/dir`, and stripping the
/// scheme leaves `/C:/dir`, which no Windows API will open — the leading slash has to go
/// when a drive letter follows it. And a folder whose name contains a space arrives
/// percent-encoded, so `%20` must come back before the path is used.
#[must_use]
pub fn uri_to_path(uri: &str) -> std::path::PathBuf {
    let trimmed = uri.strip_prefix("file://").unwrap_or(uri);

    let trimmed = match trimmed.strip_prefix('/') {
        Some(rest) if has_drive_letter(rest) => rest,
        _ => trimmed,
    };

    Path::new(&percent_decode(trimmed)).to_path_buf()
}

/// Returns `true` if `path` opens with a Windows drive specification such as `C:`.
fn has_drive_letter(path: &str) -> bool {
    let mut characters = path.chars();
    let Some(letter) = characters.next() else {
        return false;
    };
    letter.is_ascii_alphabetic() && characters.next().is_some_and(|next| next == ':')
}

/// Decodes `%XX` escapes, leaving anything malformed exactly as it was found.
///
/// Deliberately not a general URI decoder: `+` is not a space in a path, and a truncated
/// or non-hexadecimal escape is likelier to be a literal `%` in a filename than an error
/// worth refusing the workspace over.
fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }

    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0_usize;

    // Bounded by the input length: every arm advances `index`.
    while let Some(&byte) = bytes.get(index) {
        let decoded = if byte == b'%' {
            bytes
                .get(index.saturating_add(1)..index.saturating_add(3))
                .and_then(|pair| std::str::from_utf8(pair).ok())
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        } else {
            None
        };

        if let Some(value) = decoded {
            out.push(value);
            index = index.saturating_add(3);
        } else {
            out.push(byte);
            index = index.saturating_add(1);
        }
    }

    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
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

    const DOC: &str = "name: x\nnodes:\n  - name: api\n    type: service\n";

    fn store() -> DocumentStore {
        DocumentStore::default()
    }

    #[test]
    fn a_document_is_stored_and_analysed_on_open() {
        let mut store = store();
        let document = store.upsert("file:///a.yaml", DOC.to_owned(), 1);

        assert_eq!(document.version, 1);
        assert!(document.analysis.architecture.is_some());
        assert_eq!(document.index.node_names(), ["api"]);
    }

    #[test]
    fn diagnostics_are_available_immediately_after_a_change() {
        let mut store = store();
        let document = store.upsert("file:///a.yaml", DOC.to_owned(), 1);
        assert!(
            document
                .diagnostics()
                .iter()
                .any(|d| d.code == "services-require-security-controls"),
            "analysis runs eagerly, so diagnostics never lag the text"
        );
    }

    #[test]
    fn a_change_replaces_the_previous_analysis() {
        let mut store = store();
        store.upsert("file:///a.yaml", DOC.to_owned(), 1);

        let updated = DOC.replace("api", "gateway");
        let document = store.upsert("file:///a.yaml", updated, 2);

        assert_eq!(document.version, 2);
        assert_eq!(document.index.node_names(), ["gateway"]);
        assert_eq!(store.len(), 1, "an update is not a second document");
    }

    #[test]
    fn a_broken_document_is_stored_with_a_syntax_diagnostic_and_no_architecture() {
        let mut store = store();
        let document = store.upsert(
            "file:///a.yaml",
            "nodes:\n  - name: a\n   type: b\n".into(),
            1,
        );

        assert!(document.analysis.architecture.is_none());
        assert!(document.diagnostics().iter().any(|d| d.code == "syntax"));
        assert!(
            !document.index.node_names().is_empty(),
            "the index still works"
        );
    }

    #[test]
    fn closing_a_document_forgets_it() {
        let mut store = store();
        store.upsert("file:///a.yaml", DOC.to_owned(), 1);
        assert!(store.close("file:///a.yaml").is_some());
        assert!(store.is_empty());
        assert!(
            store.close("file:///a.yaml").is_none(),
            "closing twice is harmless"
        );
    }

    #[test]
    fn the_document_count_limit_is_enforced() {
        let limits = Limits {
            max_documents: 3,
            max_total_bytes: usize::MAX,
        };
        let mut store = DocumentStore::new(limits, ValidatorConfig::default());

        for n in 0..10 {
            store.upsert(&format!("file:///{n}.yaml"), DOC.to_owned(), 1);
        }
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn the_byte_limit_is_enforced() {
        let limits = Limits {
            max_documents: usize::MAX,
            max_total_bytes: 200,
        };
        let mut store = DocumentStore::new(limits, ValidatorConfig::default());

        for n in 0..10 {
            store.upsert(&format!("file:///{n}.yaml"), DOC.to_owned(), 1);
        }
        assert!(
            store.total_bytes() <= 200,
            "held {} bytes",
            store.total_bytes()
        );
    }

    #[test]
    fn eviction_drops_the_least_recently_used_document() {
        let limits = Limits {
            max_documents: 2,
            max_total_bytes: usize::MAX,
        };
        let mut store = DocumentStore::new(limits, ValidatorConfig::default());

        store.upsert("file:///old.yaml", DOC.to_owned(), 1);
        store.upsert("file:///mid.yaml", DOC.to_owned(), 1);

        // Touching `old` makes `mid` the least recently used.
        assert!(store.get("file:///old.yaml").is_some());

        store.upsert("file:///new.yaml", DOC.to_owned(), 1);

        assert!(
            store.peek("file:///old.yaml").is_some(),
            "recently used, so retained"
        );
        assert!(store.peek("file:///new.yaml").is_some(), "just written");
        assert!(store.peek("file:///mid.yaml").is_none(), "idle, so evicted");
    }

    #[test]
    fn the_document_just_written_is_never_the_eviction_victim() {
        // Dropping what the client just sent would be visibly broken.
        let limits = Limits {
            max_documents: 1,
            max_total_bytes: 1,
        };
        let mut store = DocumentStore::new(limits, ValidatorConfig::default());

        store.upsert("file:///a.yaml", DOC.to_owned(), 1);
        assert!(store.peek("file:///a.yaml").is_some());
        assert_eq!(
            store.len(),
            1,
            "even though it exceeds the byte budget on its own"
        );
    }

    #[test]
    fn peek_does_not_change_the_eviction_order() {
        let limits = Limits {
            max_documents: 2,
            max_total_bytes: usize::MAX,
        };
        let mut store = DocumentStore::new(limits, ValidatorConfig::default());

        store.upsert("file:///first.yaml", DOC.to_owned(), 1);
        store.upsert("file:///second.yaml", DOC.to_owned(), 1);
        assert!(
            store.peek("file:///first.yaml").is_some(),
            "peeking must not touch"
        );

        store.upsert("file:///third.yaml", DOC.to_owned(), 1);
        assert!(
            store.peek("file:///first.yaml").is_none(),
            "peek did not protect it"
        );
    }

    #[test]
    fn eviction_is_deterministic_when_access_times_tie() {
        // A HashMap iterates arbitrarily, so ties must be broken by URI or the victim
        // varies between runs.
        let choose = || {
            let limits = Limits {
                max_documents: 2,
                max_total_bytes: usize::MAX,
            };
            let mut store = DocumentStore::new(limits, ValidatorConfig::default());
            store.upsert("file:///a.yaml", DOC.to_owned(), 1);
            store.upsert("file:///b.yaml", DOC.to_owned(), 1);
            store.upsert("file:///c.yaml", DOC.to_owned(), 1);
            store.uris()
        };

        let mut first = choose();
        first.sort();
        for _ in 0..8 {
            let mut again = choose();
            again.sort();
            assert_eq!(first, again, "eviction must not depend on hash order");
        }
    }

    #[test]
    fn reconfiguring_re_analyses_every_open_document() {
        let mut store = store();
        store.upsert("file:///a.yaml", DOC.to_owned(), 1);
        assert!(
            store
                .peek("file:///a.yaml")
                .is_some_and(|d| !d.diagnostics().is_empty()),
            "the strict default reports something"
        );

        store.reconfigure(ValidatorConfig::new().min_security_controls_per_service(0));

        let document = store.peek("file:///a.yaml").expect("still open");
        assert!(
            !document
                .diagnostics()
                .iter()
                .any(|d| d.code == "services-require-security-controls"),
            "stale diagnostics survived a settings change"
        );
        assert_eq!(
            document.version, 1,
            "the version is preserved across re-analysis"
        );
    }

    #[test]
    fn a_missing_document_is_absent_rather_than_a_panic() {
        let mut store = store();
        assert!(store.get("file:///nope.yaml").is_none());
        assert!(store.peek("file:///nope.yaml").is_none());
    }

    #[test]
    fn uris_convert_to_paths_for_error_attribution() {
        assert_eq!(uri_to_path("file:///tmp/a.yaml"), Path::new("/tmp/a.yaml"));
        assert_eq!(uri_to_path("/tmp/a.yaml"), Path::new("/tmp/a.yaml"));
        assert_eq!(
            uri_to_path("untitled:Untitled-1"),
            Path::new("untitled:Untitled-1")
        );
    }

    #[test]
    fn the_store_never_panics_on_arbitrary_content() {
        let mut store = store();
        for (n, text) in ["", ":::", "🚀", "nodes:\n  - \n"].into_iter().enumerate() {
            let uri = format!("file:///{n}.yaml");
            store.upsert(&uri, text.to_owned(), 1);
            let _ = store.get(&uri);
        }
    }

    #[test]
    fn a_store_starts_with_no_patterns() {
        assert!(store().patterns().is_empty());
    }

    #[test]
    fn loading_patterns_re_analyses_every_open_document() {
        // The author has not touched these files; editing the library is what changed
        // their findings, so publishing stale ones would be wrong.
        const CLAIMING: &str = "\
name: checkout
version: 1.0.0
nodes:
  - name: edge-gateway
    type: gateway
patterns:
  - pattern: secure-web-tier@1.0.0
";
        const PATTERN: &str = "\
name: secure-web-tier
version: 1.0.0
requires:
  - role: edge
    type: gateway
";

        let mut store = store();
        store.upsert("file:///a.yaml", CLAIMING.to_owned(), 1);
        assert!(
            store
                .peek("file:///a.yaml")
                .expect("stored")
                .diagnostics()
                .iter()
                .any(|d| d.code == "patterns-are-satisfied"),
            "unchecked before the library loads"
        );

        let pattern = casm_parser::library::parse_pattern_str(PATTERN, Path::new("p.yaml"))
            .expect("the fixture pattern parses");
        store.load_patterns(vec![pattern]);

        assert_eq!(store.patterns().len(), 1);
        assert!(
            !store
                .peek("file:///a.yaml")
                .expect("stored")
                .diagnostics()
                .iter()
                .any(|d| d.code == "patterns-are-satisfied"),
            "satisfied once it loads, without the client touching the file"
        );
    }

    #[test]
    fn a_document_and_the_library_are_retrieved_together() {
        let mut store = store();
        store.upsert("file:///a.yaml", DOC.to_owned(), 1);

        let (document, patterns) = store
            .get_with_patterns("file:///a.yaml")
            .expect("the document is open");
        assert_eq!(document.version, 1);
        assert!(patterns.is_empty());
        assert!(store.get_with_patterns("file:///missing.yaml").is_none());
    }

    #[test]
    fn a_posix_uri_becomes_its_path() {
        assert_eq!(
            uri_to_path("file:///home/eng/architecture.yaml"),
            Path::new("/home/eng/architecture.yaml")
        );
    }

    #[test]
    fn a_windows_uri_loses_the_slash_before_its_drive_letter() {
        // `/C:/w` is not a path any Windows API will open, and this is what a client
        // actually sends. Asserted on every platform because the string transformation is
        // the same one either way.
        assert_eq!(
            uri_to_path("file:///C:/w/architecture.yaml")
                .display()
                .to_string(),
            "C:/w/architecture.yaml"
        );
    }

    #[test]
    fn a_leading_slash_survives_when_no_drive_letter_follows() {
        assert_eq!(uri_to_path("file:///srv/a.yaml"), Path::new("/srv/a.yaml"));
        assert_eq!(uri_to_path("file:///1:/odd"), Path::new("/1:/odd"));
    }

    #[test]
    fn percent_escapes_are_decoded() {
        assert_eq!(
            uri_to_path("file:///home/My%20Work/a.yaml"),
            Path::new("/home/My Work/a.yaml")
        );
    }

    #[test]
    fn a_malformed_escape_is_left_alone_rather_than_refused() {
        // Likelier a literal `%` in a filename than a URI worth rejecting the folder over.
        assert_eq!(
            uri_to_path("file:///tmp/100%/a.yaml"),
            Path::new("/tmp/100%/a.yaml")
        );
        assert_eq!(uri_to_path("file:///tmp/%zz"), Path::new("/tmp/%zz"));
        assert_eq!(uri_to_path("file:///tmp/%4"), Path::new("/tmp/%4"));
    }

    #[test]
    fn decoding_is_total_on_arbitrary_input() {
        for uri in ["", "%", "%%%%", "file://", "not a uri", "%e2%82"] {
            let _ = uri_to_path(uri);
        }
    }
}
