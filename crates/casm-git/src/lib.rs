//! Module: `casm_git`
//! Purpose: Reading an architecture's history from Git, semantically rather than textually.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # The question this crate answers
//!
//! `git log architecture.yaml` lists every commit that touched the file. Most of them did
//! not change the architecture: they reformatted it, reordered a list, fixed a typo in a
//! comment, or regenerated identifiers. Finding the commit that actually introduced a
//! dependency means reading all of them.
//!
//! `casm log` lists only the commits where the architecture's meaning changed, by walking
//! history and comparing [`casm_core::merkle`] fingerprints. Two commits with the same
//! fingerprint are the same architecture, whatever their bytes (ADR-0009), so the noise
//! disappears without any heuristics.
//!
//! The same walk, done per node instead of per architecture, is `casm blame`.
//!
//! # NASA compliance
//!
//! Rule 5 (bounded allocation): the walk holds at most two `MerkleTree`s at a time — the
//! commit being considered and its parent — regardless of how long the history is. The
//! number of commits examined is capped by [`HistoryOptions::max_commits`], so a
//! repository with a hundred thousand commits cannot turn `casm log` into a hang.
//!
//! Rule 8 (determinism): the same repository always yields the same result. Fingerprints
//! are pure functions of content, and commits are visited in Git's own ancestry order.
//!
//! **Nothing here writes.** The repository is opened for reading, no reference is moved,
//! no object is created, and no subprocess is spawned. `casm checkout` prints an
//! architecture to standard output rather than touching the working tree — a tool that
//! rewrote your files to answer a question about history would be a poor trade.

#![forbid(unsafe_code)]

pub mod error;
pub mod time;

use casm_core::merkle::{Fingerprint, MerkleTree};
use gix::bstr::ByteSlice as _;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub use error::{GitError, Result};
pub use time::DateTime;

/// How much history to examine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryOptions {
    /// The most commits to walk before stopping.
    ///
    /// A ceiling rather than a preference: without one, `casm log` on a monorepo would
    /// walk every commit ever made to find that an architecture changed twice.
    pub max_commits: usize,
    /// The most semantic changes to report.
    pub max_revisions: usize,
}

impl Default for HistoryOptions {
    fn default() -> Self {
        Self {
            max_commits: 10_000,
            max_revisions: 50,
        }
    }
}

impl HistoryOptions {
    /// Options with the given revision limit.
    #[must_use]
    pub const fn with_max_revisions(mut self, limit: usize) -> Self {
        self.max_revisions = limit;
        self
    }

    /// Options with the given commit-walk ceiling.
    #[must_use]
    pub const fn with_max_commits(mut self, limit: usize) -> Self {
        self.max_commits = limit;
        self
    }
}

/// A commit at which the architecture's meaning changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Revision {
    /// The full commit hash.
    pub commit: String,
    /// The commit author's name.
    pub author: String,
    /// The commit author's email address.
    pub email: String,
    /// Author time, in seconds since the Unix epoch.
    pub timestamp: i64,
    /// The first line of the commit message.
    pub summary: String,
    /// The architecture's fingerprint at this commit.
    pub fingerprint: Fingerprint,
    /// Node names whose meaning differs from the parent commit.
    pub changed_nodes: Vec<String>,
    /// `true` if this is the commit that introduced the file.
    pub introduced: bool,
}

impl Revision {
    /// The abbreviated commit hash, as Git would show it.
    #[must_use]
    pub fn short_commit(&self) -> String {
        self.commit.get(..7).unwrap_or(&self.commit).to_owned()
    }

    /// The author time as a UTC civil date and time.
    #[must_use]
    pub const fn dated(&self) -> DateTime {
        DateTime::from_unix(self.timestamp)
    }
}

/// A Git repository, opened for reading.
pub struct Repository {
    inner: gix::Repository,
    root: PathBuf,
}

impl Repository {
    /// Finds the repository containing `path` and opens it read-only.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::NotARepository`] if no repository encloses `path`.
    pub fn discover(path: &Path) -> Result<Self> {
        let inner = gix::discover(path).map_err(|_| GitError::NotARepository {
            path: path.to_path_buf(),
        })?;

        let root = inner
            .workdir()
            .unwrap_or_else(|| inner.git_dir())
            .to_path_buf();

        Ok(Self { inner, root })
    }

    /// The repository's working directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Expresses `path` relative to the repository root, as Git stores it.
    ///
    /// Git addresses blobs by repository-relative path, so a path given on the command
    /// line — absolute, or relative to wherever the user happened to be — has to be
    /// translated before any lookup will find it.
    #[must_use]
    pub fn relative_path(&self, path: &Path) -> PathBuf {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
        };

        // `strip_prefix` fails for a path outside the repository; keeping the original is
        // the honest fallback, and the lookup will then report it as not found.
        absolute
            .strip_prefix(&self.root)
            .map_or_else(|_| path.to_path_buf(), Path::to_path_buf)
    }

    /// Reads a file's contents at a given revision.
    ///
    /// # Errors
    ///
    /// - [`GitError::UnknownRevision`] if `revision` does not resolve.
    /// - [`GitError::PathNotFound`] if the file is not tracked there.
    /// - [`GitError::NotText`] if the blob is not valid UTF-8.
    pub fn read_at(&self, revision: &str, path: &Path) -> Result<String> {
        let id = self
            .inner
            .rev_parse_single(revision)
            .map_err(|_| GitError::UnknownRevision {
                spec: revision.to_owned(),
            })?;

        let commit = id
            .object()
            .map_err(|_| GitError::UnknownRevision {
                spec: revision.to_owned(),
            })?
            .peel_to_commit()
            .map_err(|_| GitError::UnknownRevision {
                spec: revision.to_owned(),
            })?;

        let relative = self.relative_path(path);
        let bytes = read_blob(&commit, &relative).ok_or_else(|| GitError::PathNotFound {
            path: relative.clone(),
            revision: revision.to_owned(),
        })?;

        String::from_utf8(bytes).map_err(|_| GitError::NotText {
            path: relative,
            revision: revision.to_owned(),
        })
    }

    /// Returns the commits at which the architecture's meaning changed, newest first.
    ///
    /// Commits that touched the file without changing its meaning are omitted, as are
    /// commits where the file did not exist or did not parse.
    ///
    /// # Errors
    ///
    /// - [`GitError::NoCommits`] if the repository has no history.
    /// - [`GitError::ObjectDatabase`] if the object database cannot be read.
    pub fn semantic_history(&self, path: &Path, options: HistoryOptions) -> Result<Vec<Revision>> {
        let relative = self.relative_path(path);
        let head = self.inner.head_commit().map_err(|_| GitError::NoCommits)?;

        let walk = head
            .ancestors()
            .all()
            .map_err(|error| GitError::ObjectDatabase {
                message: error.to_string(),
            })?;

        let mut revisions = Vec::new();
        // The sliding window is what bounds memory: two snapshots, never the whole history.
        let mut newer: Option<Snapshot> = None;
        let mut examined = 0_usize;

        for step in walk {
            if examined >= options.max_commits || revisions.len() >= options.max_revisions {
                break;
            }
            examined = examined.saturating_add(1);

            let Ok(info) = step else { continue };
            let Some(current) = self.snapshot(info.id, &relative) else {
                continue;
            };

            if let Some(previous) = newer.take()
                && let Some(revision) = previous.into_revision(Some(&current))
            {
                revisions.push(revision);
            }
            newer = Some(current);
        }

        // The oldest snapshot we hold has no parent in view, so it introduced the file.
        if revisions.len() < options.max_revisions
            && let Some(oldest) = newer
            && let Some(revision) = oldest.into_revision(None)
        {
            revisions.push(revision);
        }

        Ok(revisions)
    }

    /// Returns the commits at which a single node's meaning changed, newest first.
    ///
    /// # Errors
    ///
    /// As [`Repository::semantic_history`].
    pub fn blame_node(
        &self,
        path: &Path,
        node: &str,
        options: HistoryOptions,
    ) -> Result<Vec<Revision>> {
        Ok(self
            .semantic_history(path, options)?
            .into_iter()
            .filter(|revision| revision.changed_nodes.iter().any(|name| name == node))
            .collect())
    }

    /// Builds a snapshot of the architecture at one commit, if it parses there.
    fn snapshot(&self, id: gix::ObjectId, relative: &Path) -> Option<Snapshot> {
        let commit = self.inner.find_commit(id).ok()?;
        let bytes = read_blob(&commit, relative)?;
        let source = String::from_utf8(bytes).ok()?;

        // A commit where the file was mid-refactor and did not parse is not a semantic
        // change; it is a commit we cannot say anything about. Skipping it means
        // `casm log` compares the states either side of it, which is what a reader wants.
        let architecture = casm_parser::parse_str(&source, relative).ok()?;
        let author = commit.author().ok()?;
        let message = commit.message().ok()?;

        Some(Snapshot {
            commit: id.to_string(),
            author: author.name.to_string(),
            email: author.email.to_string(),
            timestamp: author.time().map(|time| time.seconds).unwrap_or_default(),
            summary: message.title.trim().to_str_lossy().into_owned(),
            tree: MerkleTree::of(&architecture),
        })
    }
}

/// The architecture at one commit, reduced to what history needs.
struct Snapshot {
    commit: String,
    author: String,
    email: String,
    timestamp: i64,
    summary: String,
    tree: MerkleTree,
}

impl Snapshot {
    /// Turns this snapshot into a [`Revision`] if it differs from its parent.
    ///
    /// `parent` is `None` when the snapshot is the oldest one examined, which means the
    /// file was introduced here as far as this walk can see.
    fn into_revision(self, parent: Option<&Self>) -> Option<Revision> {
        let changed_nodes = parent.map_or_else(
            || self.tree.nodes().keys().cloned().collect(),
            |older| self.tree.changed_nodes(&older.tree),
        );

        if let Some(older) = parent
            && older.tree.root() == self.tree.root()
        {
            return None;
        }

        Some(Revision {
            commit: self.commit,
            author: self.author,
            email: self.email,
            timestamp: self.timestamp,
            summary: self.summary,
            fingerprint: self.tree.root(),
            changed_nodes,
            introduced: parent.is_none(),
        })
    }
}

/// Reads a blob from a commit's tree by repository-relative path.
fn read_blob(commit: &gix::Commit<'_>, relative: &Path) -> Option<Vec<u8>> {
    let tree = commit.tree().ok()?;
    let entry = tree.lookup_entry_by_path(relative).ok()??;
    Some(entry.object().ok()?.data.clone())
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
    fn default_options_are_bounded() {
        let options = HistoryOptions::default();
        assert!(
            options.max_commits > 0,
            "an unbounded walk is a hang waiting to happen"
        );
        assert!(options.max_revisions > 0);
    }

    #[test]
    fn option_builders_override_the_limits() {
        let options = HistoryOptions::default()
            .with_max_commits(7)
            .with_max_revisions(3);
        assert_eq!(options.max_commits, 7);
        assert_eq!(options.max_revisions, 3);
    }

    #[test]
    fn discovering_outside_a_repository_says_so() {
        let outside = std::env::temp_dir().join("casm-definitely-not-a-repo");
        std::fs::create_dir_all(&outside).ok();
        assert!(matches!(
            Repository::discover(&outside),
            Err(GitError::NotARepository { .. })
        ));
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn a_short_commit_is_seven_characters() {
        let revision = Revision {
            commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            author: "a".to_owned(),
            email: "e".to_owned(),
            timestamp: 0,
            summary: "s".to_owned(),
            fingerprint: casm_core::merkle::Fingerprint::parse_hex(&"a".repeat(64)).unwrap(),
            changed_nodes: Vec::new(),
            introduced: false,
        };
        assert_eq!(revision.short_commit(), "0123456");
    }

    #[test]
    fn a_short_commit_does_not_panic_on_an_unexpectedly_short_hash() {
        let revision = Revision {
            commit: "abc".to_owned(),
            author: "a".to_owned(),
            email: "e".to_owned(),
            timestamp: 0,
            summary: "s".to_owned(),
            fingerprint: casm_core::merkle::Fingerprint::parse_hex(&"a".repeat(64)).unwrap(),
            changed_nodes: Vec::new(),
            introduced: false,
        };
        assert_eq!(revision.short_commit(), "abc");
    }

    #[test]
    fn a_revision_renders_its_author_time_as_utc() {
        let revision = Revision {
            commit: "a".repeat(40),
            author: "a".to_owned(),
            email: "e".to_owned(),
            timestamp: 1_700_000_000,
            summary: "s".to_owned(),
            fingerprint: casm_core::merkle::Fingerprint::parse_hex(&"a".repeat(64)).unwrap(),
            changed_nodes: Vec::new(),
            introduced: false,
        };
        assert_eq!(revision.dated().to_date(), "2023-11-14");
    }
}
