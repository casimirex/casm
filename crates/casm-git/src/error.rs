//! Module: `casm_git::error`
//! Purpose: What can go wrong reading history, in terms a user can act on.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! `gix` reports failures in its own vocabulary — object database errors, reference
//! resolution errors, decode errors. Most of those mean one of three things to somebody
//! running `casm log`: this is not a repository, that revision does not exist, or the
//! repository is damaged. The variants here say which.

use std::path::PathBuf;
use thiserror::Error;

/// Everything that can go wrong reading an architecture's history.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// No Git repository was found at or above the given path.
    #[error(
        "'{path}' is not inside a Git repository; \
         architecture history needs one to read from"
    )]
    NotARepository {
        /// Where the search started.
        path: PathBuf,
    },

    /// The repository has no commits yet.
    #[error("this repository has no commits yet, so there is no history to read")]
    NoCommits,

    /// A revision specification did not resolve.
    #[error("'{spec}' does not name a commit in this repository")]
    UnknownRevision {
        /// The specification as written.
        spec: String,
    },

    /// The file is not tracked at the requested revision.
    #[error("'{path}' does not exist at {revision}")]
    PathNotFound {
        /// The requested path.
        path: PathBuf,
        /// The revision it was looked for at.
        revision: String,
    },

    /// A blob was not valid UTF-8.
    #[error("'{path}' at {revision} is not valid UTF-8, so it cannot be an architecture")]
    NotText {
        /// The offending path.
        path: PathBuf,
        /// The revision it was read at.
        revision: String,
    },

    /// The object database could not be read.
    ///
    /// Usually a damaged or partially-fetched repository — a shallow clone, most often.
    #[error("cannot read the repository's object database: {message}")]
    ObjectDatabase {
        /// What `gix` reported.
        message: String,
    },
}

/// The canonical result type of `casm-git`.
pub type Result<T, E = GitError> = core::result::Result<T, E>;

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
    fn a_missing_repository_says_what_was_needed_and_why() {
        let error = GitError::NotARepository {
            path: PathBuf::from("/tmp/somewhere"),
        };
        let rendered = error.to_string();
        assert!(rendered.contains("/tmp/somewhere"), "{rendered}");
        assert!(rendered.contains("Git repository"), "{rendered}");
    }

    #[test]
    fn an_empty_repository_is_distinguished_from_a_missing_one() {
        // "no commits yet" and "not a repository" need different responses.
        assert_ne!(
            GitError::NoCommits.to_string(),
            GitError::NotARepository {
                path: PathBuf::from(".")
            }
            .to_string()
        );
    }

    #[test]
    fn a_missing_path_names_both_the_file_and_the_revision() {
        let error = GitError::PathNotFound {
            path: PathBuf::from("architecture.yaml"),
            revision: "HEAD~3".to_owned(),
        };
        let rendered = error.to_string();
        assert!(rendered.contains("architecture.yaml"), "{rendered}");
        assert!(rendered.contains("HEAD~3"), "{rendered}");
    }

    #[test]
    fn errors_are_comparable_for_exact_assertions() {
        let first = GitError::UnknownRevision {
            spec: "nope".to_owned(),
        };
        let second = GitError::UnknownRevision {
            spec: "nope".to_owned(),
        };
        assert_eq!(first, second);
    }
}
