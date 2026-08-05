//! Module: `casm_lsp::library`
//! Purpose: Finding the pattern library a workspace means, and saying so when it cannot.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why the server has to guess
//!
//! `casm validate --patterns <dir>` is explicit: the author names the directory. An editor
//! has no such moment. It opens a file and expects diagnostics, so the server must decide
//! for itself which library a document is being checked against.
//!
//! The order is: the `casm.patterns` setting if the client sent one, then `patterns/` at
//! each workspace root, then `.casm/patterns/`. First hit wins, and nothing is merged —
//! two directories holding different versions of the same pattern would otherwise make the
//! answer depend on which was scanned first.
//!
//! # Saying nothing is not an option
//!
//! Every outcome carries a [`Discovery::note`] the server logs. A library that failed to
//! load looks exactly like a library that was never configured — every claim reported as
//! unchecked — and the difference is the whole of what the author needs to know.
//!
//! This module is the one part of the crate that touches the filesystem, which is why it
//! is behind the `server` feature: the browser build has no disk, and gets its patterns
//! handed to it across the ABI instead.

use casm_core::Pattern;
use casm_parser::library::Library;
use std::path::{Path, PathBuf};

/// Directories searched under a workspace root, in order.
///
/// `patterns/` is what the CLI documentation uses and the repository dogfoods.
/// `.casm/patterns/` is for a workspace that would rather not spend a top-level name.
const CONVENTIONAL: &[&str] = &["patterns", ".casm/patterns"];

/// The result of looking for a pattern library.
///
/// Construction never fails. A missing directory, an unreadable file, and a malformed
/// pattern all produce an empty library and a note explaining which — never an error the
/// caller has to decide what to do about mid-session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Discovery {
    /// The patterns that loaded, empty if none did.
    pub patterns: Vec<Pattern>,
    /// The directory they came from, if one was found.
    pub directory: Option<PathBuf>,
    /// One line for the client's log, describing what happened.
    pub note: String,
}

impl Discovery {
    /// A discovery that found nothing, for the stated reason.
    fn empty(note: impl Into<String>) -> Self {
        Self {
            patterns: Vec::new(),
            directory: None,
            note: note.into(),
        }
    }
}

/// Finds and loads the library for a workspace.
///
/// `roots` are the workspace folders the client reported, most significant first.
/// `configured` is the `casm.patterns` setting, absolute or relative to the first root.
///
/// An explicit setting is never silently ignored: a `configured` path that does not exist
/// yields an empty library and says so, rather than falling back to a conventional
/// directory the author did not ask for.
#[must_use]
pub fn discover(roots: &[PathBuf], configured: Option<&str>) -> Discovery {
    if let Some(setting) = configured {
        let candidate = resolve(roots, setting);
        if !candidate.is_dir() {
            return Discovery::empty(format!(
                "casm.patterns is set to '{}', which is not a directory — conformance \
                 claims will be reported as unchecked",
                candidate.display()
            ));
        }
        return load(&candidate);
    }

    for root in roots {
        for suffix in CONVENTIONAL {
            let candidate = root.join(suffix);
            if candidate.is_dir() {
                return load(&candidate);
            }
        }
    }

    Discovery::empty(
        "no pattern library found — looked for 'patterns/' and '.casm/patterns/' in each \
         workspace folder. Conformance claims will be reported as unchecked; set \
         'casm.patterns' to point at one.",
    )
}

/// Loads a directory known to exist, turning any failure into a note.
fn load(directory: &Path) -> Discovery {
    match Library::load(directory) {
        Ok(library) => {
            let patterns: Vec<Pattern> = library.patterns().cloned().collect();
            Discovery {
                note: format!(
                    "loaded {} pattern(s) from {}",
                    patterns.len(),
                    directory.display()
                ),
                patterns,
                directory: Some(directory.to_path_buf()),
            }
        }
        // The directory is real but something in it is not a pattern. Reporting the
        // failure and carrying on with none beats refusing to serve the workspace.
        Err(error) => Discovery::empty(format!(
            "could not load the pattern library at {}: {}",
            directory.display(),
            error
        )),
    }
}

/// Resolves a configured path against the first workspace root.
fn resolve(roots: &[PathBuf], setting: &str) -> PathBuf {
    let path = Path::new(setting);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    roots
        .first()
        .map_or_else(|| path.to_path_buf(), |root| root.join(path))
}

/// Returns `true` if `path` is a file that could change what the library holds.
///
/// Used to decide whether a `workspace/didChangeWatchedFiles` notification is worth a
/// reload. A client may watch more than the server cares about, and re-reading the library
/// on every YAML file in the repository would make every keystroke in an architecture file
/// pay for it.
#[must_use]
pub fn is_pattern_file(path: &Path, directory: Option<&Path>) -> bool {
    let Some(directory) = directory else {
        // With no library loaded, any file under a conventionally named directory could be
        // the one that creates it.
        return path
            .parent()
            .is_some_and(|parent| CONVENTIONAL.iter().any(|name| parent.ends_with(name)));
    };
    path.starts_with(directory)
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
    use std::fs;

    const PATTERN: &str = "name: secure-tier\nversion: 1.0.0\nrequires:\n  - role: edge\n    \
                           type: gateway\n";

    /// A directory removed when the test ends, however it ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("casm-lsp-library-{label}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_library(root: &Path, suffix: &str) -> PathBuf {
        let directory = root.join(suffix);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("secure-tier.yaml"), PATTERN).unwrap();
        directory
    }

    #[test]
    fn a_conventional_directory_is_found() {
        let temp = TempDir::new("conventional");
        write_library(temp.path(), "patterns");

        let found = discover(&[temp.path().to_path_buf()], None);

        assert_eq!(found.patterns.len(), 1);
        assert_eq!(
            found.directory.as_deref(),
            Some(temp.path().join("patterns").as_path())
        );
        assert!(found.note.contains("loaded 1 pattern"), "{}", found.note);
    }

    #[test]
    fn the_dotted_directory_is_the_fallback() {
        let temp = TempDir::new("dotted");
        write_library(temp.path(), ".casm/patterns");

        let found = discover(&[temp.path().to_path_buf()], None);

        assert_eq!(found.patterns.len(), 1);
    }

    #[test]
    fn the_first_hit_wins_rather_than_merging() {
        // Two libraries would otherwise make the answer depend on scan order.
        let temp = TempDir::new("both");
        write_library(temp.path(), "patterns");
        write_library(temp.path(), ".casm/patterns");

        let found = discover(&[temp.path().to_path_buf()], None);

        assert_eq!(found.patterns.len(), 1);
        assert_eq!(
            found.directory.as_deref(),
            Some(temp.path().join("patterns").as_path())
        );
    }

    #[test]
    fn a_workspace_with_no_library_says_where_it_looked() {
        let temp = TempDir::new("bare");

        let found = discover(&[temp.path().to_path_buf()], None);

        assert!(found.patterns.is_empty());
        assert!(found.directory.is_none());
        assert!(found.note.contains("patterns/"), "{}", found.note);
        assert!(found.note.contains("unchecked"), "{}", found.note);
    }

    #[test]
    fn a_setting_is_resolved_against_the_first_root() {
        let temp = TempDir::new("setting");
        write_library(temp.path(), "shapes");

        let found = discover(&[temp.path().to_path_buf()], Some("shapes"));

        assert_eq!(found.patterns.len(), 1);
    }

    #[test]
    fn an_absolute_setting_ignores_the_roots() {
        let temp = TempDir::new("absolute");
        let directory = write_library(temp.path(), "shapes");

        let found = discover(&[], Some(&directory.display().to_string()));

        assert_eq!(found.patterns.len(), 1);
    }

    #[test]
    fn a_setting_pointing_nowhere_does_not_fall_back() {
        // The conventional directory exists and must still not be used: an author who
        // named a directory has said which one they mean.
        let temp = TempDir::new("nowhere");
        write_library(temp.path(), "patterns");

        let found = discover(&[temp.path().to_path_buf()], Some("does-not-exist"));

        assert!(found.patterns.is_empty());
        assert!(found.note.contains("not a directory"), "{}", found.note);
    }

    #[test]
    fn a_malformed_pattern_is_reported_not_swallowed() {
        let temp = TempDir::new("malformed");
        let directory = temp.path().join("patterns");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("broken.yaml"), "name: [unclosed\n").unwrap();

        let found = discover(&[temp.path().to_path_buf()], None);

        assert!(found.patterns.is_empty());
        assert!(found.note.contains("could not load"), "{}", found.note);
    }

    #[test]
    fn only_files_in_the_loaded_directory_trigger_a_reload() {
        let directory = Path::new("/w/patterns");

        assert!(is_pattern_file(
            Path::new("/w/patterns/secure-tier.yaml"),
            Some(directory)
        ));
        assert!(!is_pattern_file(
            Path::new("/w/examples/storefront.yaml"),
            Some(directory)
        ));
    }

    #[test]
    fn with_no_library_a_conventional_directory_still_triggers_one() {
        // The library that does not exist yet is exactly the one worth noticing.
        assert!(is_pattern_file(Path::new("/w/patterns/first.yaml"), None));
        assert!(is_pattern_file(
            Path::new("/w/.casm/patterns/first.yaml"),
            None
        ));
        assert!(!is_pattern_file(Path::new("/w/src/main.rs"), None));
    }
}
