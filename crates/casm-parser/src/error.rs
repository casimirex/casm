//! Module: `casm_parser::error`
//! Purpose: Diagnostic-grade parse failures with location, cause, and a suggested fix.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Design intent
//!
//! The bar here is the Rust compiler, not `serde`'s default output. A parse failure must
//! answer three questions: *where*, *what*, and *what should I type instead*. A
//! [`ParseError`] that cannot answer the third question is still acceptable, but the
//! type makes the omission visible rather than silently normal.

use core::fmt;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// A 1-indexed position within a source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    /// 1-indexed line number.
    pub line: usize,
    /// 1-indexed column number.
    pub column: usize,
}

impl Location {
    /// Constructs a location, clamping zero to 1 so output is never `0:0`.
    #[must_use]
    pub const fn new(line: usize, column: usize) -> Self {
        Self {
            line: if line == 0 { 1 } else { line },
            column: if column == 0 { 1 } else { column },
        }
    }

    /// The location of the very start of a file.
    #[must_use]
    pub const fn start() -> Self {
        Self { line: 1, column: 1 }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// Everything that can go wrong turning bytes into an [`casm_core::Architecture`].
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The file's format could not be determined from its extension or contents.
    #[error(
        "cannot determine the format of '{path}': \
         expected a .yaml, .yml, .json, or .toml extension"
    )]
    UnknownFormat {
        /// The file that could not be classified.
        path: PathBuf,
    },

    /// The document was not well-formed in its declared format.
    #[error("{path}:{location}: {message}")]
    Syntax {
        /// The offending file.
        path: PathBuf,
        /// Where the parser gave up.
        location: Location,
        /// What the underlying format parser reported.
        message: String,
        /// A concrete suggestion, when one can be inferred.
        suggestion: Option<String>,
    },

    /// The document parsed, but a value violated a domain rule.
    #[error("{path}: {message}")]
    Semantic {
        /// The offending file.
        path: PathBuf,
        /// What rule was violated.
        message: String,
        /// A concrete suggestion, when one can be inferred.
        suggestion: Option<String>,
    },

    /// A relationship referenced a node name or id that does not exist.
    #[error("{path}: relationship {endpoint} '{reference}' does not match any declared node")]
    UnresolvedReference {
        /// The offending file.
        path: PathBuf,
        /// Which end failed: `"source"` or `"target"`.
        endpoint: &'static str,
        /// The unresolvable reference as written.
        reference: String,
        /// The closest declared node name, if one is close enough to suggest.
        suggestion: Option<String>,
    },

    /// A pattern-conformance binding referenced a node that does not exist.
    #[error(
        "{path}: pattern '{pattern}' binds role '{role}' to '{reference}', \
         which does not match any declared node"
    )]
    UnresolvedBinding {
        /// The offending file.
        path: PathBuf,
        /// The pattern reference as written.
        pattern: String,
        /// The role that was bound.
        role: String,
        /// The unresolvable node reference as written.
        reference: String,
        /// The closest declared node name, if one is close enough to suggest.
        suggestion: Option<String>,
    },

    /// The document exceeded the configured size ceiling.
    ///
    /// NASA Rule 5: bounded allocation. Parsing is the point where an attacker-controlled
    /// byte count becomes an allocation, so the bound is enforced before, not during.
    #[error("'{path}' is {size} bytes, exceeding the {limit}-byte parse limit")]
    TooLarge {
        /// The offending file.
        path: PathBuf,
        /// Actual size in bytes.
        size: u64,
        /// The configured ceiling.
        limit: u64,
    },

    /// Two files in a pattern library defined the same `name@version`.
    #[error("{path}: pattern '{pattern}' is already defined by '{first}'")]
    DuplicatePattern {
        /// The file that redefined it.
        path: PathBuf,
        /// The colliding `name@version` reference.
        pattern: String,
        /// The file that defined it first.
        first: PathBuf,
    },

    /// A pattern library directory held more pattern files than the ceiling allows.
    ///
    /// NASA Rule 5: a directory is attacker-controlled input in the same way a file is.
    #[error("'{path}' holds {count} pattern files, exceeding the limit of {limit}")]
    TooManyPatterns {
        /// The offending directory.
        path: PathBuf,
        /// How many pattern files it holds.
        count: usize,
        /// The configured ceiling.
        limit: usize,
    },

    /// The file could not be read.
    #[error("cannot read '{path}': {message}")]
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying I/O error message.
        message: String,
    },

    /// Serialising an architecture back to text failed.
    #[error("cannot serialise architecture to {format}: {message}")]
    Emit {
        /// The target format.
        format: &'static str,
        /// The underlying serialiser message.
        message: String,
    },
}

impl ParseError {
    /// The file this error concerns, if it concerns one.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::UnknownFormat { path }
            | Self::Syntax { path, .. }
            | Self::Semantic { path, .. }
            | Self::UnresolvedReference { path, .. }
            | Self::UnresolvedBinding { path, .. }
            | Self::DuplicatePattern { path, .. }
            | Self::TooManyPatterns { path, .. }
            | Self::TooLarge { path, .. }
            | Self::Io { path, .. } => Some(path),
            Self::Emit { .. } => None,
        }
    }

    /// Where in the file the failure occurred, when that is known.
    #[must_use]
    pub const fn location(&self) -> Option<Location> {
        match self {
            Self::Syntax { location, .. } => Some(*location),
            Self::UnknownFormat { .. }
            | Self::Semantic { .. }
            | Self::UnresolvedReference { .. }
            | Self::UnresolvedBinding { .. }
            | Self::DuplicatePattern { .. }
            | Self::TooManyPatterns { .. }
            | Self::TooLarge { .. }
            | Self::Io { .. }
            | Self::Emit { .. } => None,
        }
    }

    /// The actionable fix hint, when one could be inferred.
    #[must_use]
    pub fn suggestion(&self) -> Option<&str> {
        match self {
            Self::Syntax { suggestion, .. }
            | Self::Semantic { suggestion, .. }
            | Self::UnresolvedReference { suggestion, .. }
            | Self::UnresolvedBinding { suggestion, .. } => suggestion.as_deref(),
            Self::UnknownFormat { .. }
            | Self::DuplicatePattern { .. }
            | Self::TooManyPatterns { .. }
            | Self::TooLarge { .. }
            | Self::Io { .. }
            | Self::Emit { .. } => None,
        }
    }

    /// Renders the error the way a compiler would: location, message, then a hint.
    ///
    /// ```text
    /// architecture.yaml:14:5: unknown variant `srvice`, expected one of `service`, …
    ///   help: did you mean `service`?
    /// ```
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = self.to_string();
        if let Some(hint) = self.suggestion() {
            out.push_str("\n  help: ");
            out.push_str(hint);
        }
        out
    }
}

/// The canonical result type of `casm-parser`.
pub type Result<T, E = ParseError> = core::result::Result<T, E>;

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
    fn location_clamps_zero_to_one() {
        // A `0:0` in compiler output always looks like a bug, because it usually is one.
        assert_eq!(Location::new(0, 0), Location { line: 1, column: 1 });
        assert_eq!(Location::new(0, 5).line, 1);
        assert_eq!(Location::new(5, 0).column, 1);
    }

    #[test]
    fn location_preserves_real_positions() {
        assert_eq!(Location::new(14, 5).to_string(), "14:5");
    }

    #[test]
    fn syntax_errors_lead_with_path_and_position() {
        let err = ParseError::Syntax {
            path: PathBuf::from("architecture.yaml"),
            location: Location::new(14, 5),
            message: "unknown variant `srvice`".into(),
            suggestion: Some("did you mean `service`?".into()),
        };
        assert!(err.to_string().starts_with("architecture.yaml:14:5:"));
    }

    #[test]
    fn render_appends_the_help_line() {
        let err = ParseError::Syntax {
            path: PathBuf::from("a.yaml"),
            location: Location::new(1, 1),
            message: "bad".into(),
            suggestion: Some("try `service`".into()),
        };
        assert_eq!(err.render(), "a.yaml:1:1: bad\n  help: try `service`");
    }

    #[test]
    fn render_omits_the_help_line_when_there_is_no_hint() {
        let err = ParseError::Io {
            path: PathBuf::from("a.yaml"),
            message: "not found".into(),
        };
        assert!(!err.render().contains("help:"));
    }

    #[test]
    fn a_syntax_error_reports_where_it_happened() {
        // The existing accessor test only asserts the *absent* case, so `location`
        // returning `None` for everything survived — and an editor anchors its squiggle
        // on exactly this.
        let err = ParseError::Syntax {
            path: PathBuf::from("a.yaml"),
            location: Location::new(7, 22),
            message: "unexpected character".into(),
            suggestion: None,
        };

        let location = err.location().expect("a syntax error knows its position");
        assert_eq!(location.line, 7);
        assert_eq!(location.column, 22);
    }

    #[test]
    fn accessors_agree_with_the_variant() {
        let err = ParseError::UnresolvedReference {
            path: PathBuf::from("a.yaml"),
            endpoint: "target",
            reference: "orders-bd".into(),
            suggestion: Some("orders-db".into()),
        };
        assert_eq!(err.path(), Some(Path::new("a.yaml")));
        assert_eq!(
            err.location(),
            None,
            "reference errors have no single position"
        );
        assert_eq!(err.suggestion(), Some("orders-db"));
    }
}
