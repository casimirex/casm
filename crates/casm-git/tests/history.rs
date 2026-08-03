//! Integration tests against a real Git repository.
//!
//! The unit tests cover the arithmetic; these cover the claim the crate is built on —
//! that a commit which reformats an architecture is *not* a commit that changed it.
//! Nothing short of a real repository with a real history can demonstrate that.
//!
//! The fixture drives the `git` command line rather than `gix`, deliberately: writing the
//! history with the same library that reads it would let a shared misunderstanding of the
//! object format pass unnoticed. Tests skip if `git` is unavailable.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use casm_git::{GitError, HistoryOptions, Repository};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A throwaway repository with a controlled history.
struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    /// Creates an initialised repository, or `None` if `git` is not installed.
    fn new(label: &str) -> Option<Self> {
        if Command::new("git").arg("--version").output().is_err() {
            return None;
        }

        let unique = casm_core::NodeId::new();
        let directory = std::env::temp_dir().join(format!("casm-git-{label}-{unique}"));
        std::fs::create_dir_all(&directory).ok()?;

        let fixture = Self { directory };
        fixture.git(&["init", "--quiet", "--initial-branch=main"]);
        fixture.git(&["config", "user.name", "Test Author"]);
        fixture.git(&["config", "user.email", "test@example.com"]);
        fixture.git(&["config", "commit.gpgsign", "false"]);
        Some(fixture)
    }

    /// Runs a git subcommand in the fixture.
    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(&self.directory)
            .output()
            .expect("git should run");
        assert!(
            status.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    /// Writes `architecture.yaml` and commits it.
    fn commit(&self, contents: &str, message: &str) {
        std::fs::write(self.path(), contents).expect("write");
        self.git(&["add", "architecture.yaml"]);
        self.git(&["commit", "--quiet", "-m", message]);
    }

    /// The architecture file's path.
    fn path(&self) -> PathBuf {
        self.directory.join("architecture.yaml")
    }

    /// Opens the repository through `casm-git`.
    fn open(&self) -> Repository {
        Repository::discover(&self.directory).expect("the fixture is a repository")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).ok();
    }
}

const V1: &str = "\
name: checkout
version: 1.0.0
nodes:
  - name: api
    type: service
  - name: orders-db
    type: database
relationships:
  - source: api
    target: orders-db
    type: sync
    latency-budget-ms: 50
";

/// The same architecture with the node order swapped and comments added.
const V1_REFORMATTED: &str = "\
# The storefront's order capture path.
name: checkout
version: 1.0.0

nodes:
  # Declared second last time; identical meaning.
  - name: orders-db
    type: database

  - name: api
    type: service

relationships:
  - source: api
    target: orders-db
    type: sync
    latency-budget-ms: 50
";

/// `orders-db` becomes a `storage` node: a real change.
const V2: &str = "\
name: checkout
version: 1.0.0
nodes:
  - name: api
    type: service
  - name: orders-db
    type: storage
relationships:
  - source: api
    target: orders-db
    type: sync
    latency-budget-ms: 50
";

#[test]
fn reformatting_is_not_a_semantic_change() {
    // The central claim of Phase 8, and of ADR-0009.
    let Some(fixture) = Fixture::new("reformat") else {
        return;
    };
    fixture.commit(V1, "add the checkout architecture");
    fixture.commit(V1_REFORMATTED, "reorder nodes and add comments");

    let repository = fixture.open();
    let history = repository
        .semantic_history(&fixture.path(), HistoryOptions::default())
        .expect("history reads");

    assert_eq!(
        history.len(),
        1,
        "two commits, one meaning: {:?}",
        history.iter().map(|r| &r.summary).collect::<Vec<_>>()
    );
    assert_eq!(history[0].summary, "add the checkout architecture");
    assert!(history[0].introduced);
}

#[test]
fn a_real_change_is_reported() {
    let Some(fixture) = Fixture::new("change") else {
        return;
    };
    fixture.commit(V1, "add the checkout architecture");
    fixture.commit(V1_REFORMATTED, "reorder nodes");
    fixture.commit(V2, "move orders to object storage");

    let repository = fixture.open();
    let history = repository
        .semantic_history(&fixture.path(), HistoryOptions::default())
        .expect("history reads");

    assert_eq!(
        history.len(),
        2,
        "{:?}",
        history.iter().map(|r| &r.summary).collect::<Vec<_>>()
    );
    assert_eq!(
        history[0].summary, "move orders to object storage",
        "newest first"
    );
    assert_eq!(history[1].summary, "add the checkout architecture");
}

#[test]
fn a_change_names_the_nodes_it_touched() {
    let Some(fixture) = Fixture::new("changed-nodes") else {
        return;
    };
    fixture.commit(V1, "initial");
    fixture.commit(V2, "change the datastore");

    let repository = fixture.open();
    let history = repository
        .semantic_history(&fixture.path(), HistoryOptions::default())
        .unwrap();

    assert_eq!(history[0].changed_nodes, ["orders-db"], "api was untouched");
}

#[test]
fn commit_metadata_is_carried_through() {
    let Some(fixture) = Fixture::new("metadata") else {
        return;
    };
    fixture.commit(V1, "initial");

    let repository = fixture.open();
    let history = repository
        .semantic_history(&fixture.path(), HistoryOptions::default())
        .unwrap();

    assert_eq!(history[0].author, "Test Author");
    assert_eq!(history[0].email, "test@example.com");
    assert_eq!(history[0].short_commit().len(), 7);
    assert!(history[0].timestamp > 1_577_836_800, "a plausible clock");
    assert!(history[0].dated().year >= 2020);
}

#[test]
fn blame_attributes_a_node_to_the_commit_that_changed_it() {
    let Some(fixture) = Fixture::new("blame") else {
        return;
    };
    fixture.commit(V1, "initial");
    fixture.commit(V1_REFORMATTED, "reformat");
    fixture.commit(V2, "change the datastore");

    let repository = fixture.open();

    let db = repository
        .blame_node(&fixture.path(), "orders-db", HistoryOptions::default())
        .unwrap();
    assert_eq!(db[0].summary, "change the datastore", "not the reformat");

    let api = repository
        .blame_node(&fixture.path(), "api", HistoryOptions::default())
        .unwrap();
    assert_eq!(api.len(), 1, "api changed once: when it was introduced");
    assert_eq!(api[0].summary, "initial");
}

#[test]
fn blame_of_an_unknown_node_is_empty() {
    let Some(fixture) = Fixture::new("blame-unknown") else {
        return;
    };
    fixture.commit(V1, "initial");

    let repository = fixture.open();
    assert!(
        repository
            .blame_node(&fixture.path(), "nonexistent", HistoryOptions::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_commit_where_the_file_did_not_parse_is_skipped() {
    // Mid-refactor commits are not semantic changes; they are commits we cannot read.
    // Skipping them means history compares the states either side.
    let Some(fixture) = Fixture::new("broken") else {
        return;
    };
    fixture.commit(V1, "initial");
    fixture.commit(
        "name: checkout\nnodes:\n  - name: api\n   type: service\n",
        "wip, broken",
    );
    fixture.commit(V1, "restore");

    let repository = fixture.open();
    let history = repository
        .semantic_history(&fixture.path(), HistoryOptions::default())
        .unwrap();

    assert_eq!(
        history.len(),
        1,
        "the broken commit is invisible, and the states either side are identical: {:?}",
        history.iter().map(|r| &r.summary).collect::<Vec<_>>()
    );
}

#[test]
fn reading_a_file_at_a_past_revision_returns_the_old_contents() {
    let Some(fixture) = Fixture::new("read-at") else {
        return;
    };
    fixture.commit(V1, "initial");
    fixture.commit(V2, "change");

    let repository = fixture.open();

    let head = repository.read_at("HEAD", &fixture.path()).unwrap();
    assert!(head.contains("type: storage"), "{head}");

    let previous = repository.read_at("HEAD~1", &fixture.path()).unwrap();
    assert!(previous.contains("type: database"), "{previous}");
    assert_eq!(previous, V1);
}

#[test]
fn reading_an_unknown_revision_is_reported_as_such() {
    let Some(fixture) = Fixture::new("bad-rev") else {
        return;
    };
    fixture.commit(V1, "initial");

    let repository = fixture.open();
    assert!(matches!(
        repository.read_at("no-such-ref", &fixture.path()),
        Err(GitError::UnknownRevision { .. })
    ));
}

#[test]
fn reading_an_untracked_path_is_reported_as_such() {
    let Some(fixture) = Fixture::new("bad-path") else {
        return;
    };
    fixture.commit(V1, "initial");

    let repository = fixture.open();
    let missing = fixture.directory.join("nope.yaml");
    assert!(matches!(
        repository.read_at("HEAD", &missing),
        Err(GitError::PathNotFound { .. })
    ));
}

#[test]
fn history_of_an_untracked_file_is_empty_rather_than_an_error() {
    let Some(fixture) = Fixture::new("untracked") else {
        return;
    };
    fixture.commit(V1, "initial");

    let repository = fixture.open();
    let history = repository
        .semantic_history(
            &fixture.directory.join("other.yaml"),
            HistoryOptions::default(),
        )
        .unwrap();
    assert!(history.is_empty());
}

#[test]
fn a_repository_with_no_commits_says_so() {
    let Some(fixture) = Fixture::new("empty") else {
        return;
    };

    let repository = fixture.open();
    assert!(matches!(
        repository.semantic_history(&fixture.path(), HistoryOptions::default()),
        Err(GitError::NoCommits)
    ));
}

#[test]
fn the_revision_limit_is_honoured() {
    let Some(fixture) = Fixture::new("limit") else {
        return;
    };
    for version in 1..=5 {
        fixture.commit(
            &V1.replace("version: 1.0.0", &format!("version: {version}.0.0")),
            "bump",
        );
    }

    let repository = fixture.open();
    let history = repository
        .semantic_history(
            &fixture.path(),
            HistoryOptions::default().with_max_revisions(2),
        )
        .unwrap();
    assert_eq!(history.len(), 2);
}

#[test]
fn the_commit_walk_ceiling_is_honoured() {
    let Some(fixture) = Fixture::new("ceiling") else {
        return;
    };
    for version in 1..=5 {
        fixture.commit(
            &V1.replace("version: 1.0.0", &format!("version: {version}.0.0")),
            "bump",
        );
    }

    let repository = fixture.open();
    let history = repository
        .semantic_history(
            &fixture.path(),
            HistoryOptions::default().with_max_commits(2),
        )
        .unwrap();
    assert!(
        history.len() <= 2,
        "walked further than permitted: {}",
        history.len()
    );
}

#[test]
fn history_is_deterministic_across_repeated_reads() {
    let Some(fixture) = Fixture::new("deterministic") else {
        return;
    };
    fixture.commit(V1, "initial");
    fixture.commit(V2, "change");

    let repository = fixture.open();
    let first = repository
        .semantic_history(&fixture.path(), HistoryOptions::default())
        .unwrap();
    let second = repository
        .semantic_history(&fixture.path(), HistoryOptions::default())
        .unwrap();
    assert_eq!(first, second);
}

#[test]
fn reading_history_never_modifies_the_repository() {
    // The crate promises to be read-only. This checks it against the working tree and
    // the ref that HEAD points at.
    let Some(fixture) = Fixture::new("readonly") else {
        return;
    };
    fixture.commit(V1, "initial");
    fixture.commit(V2, "change");

    let before = std::fs::read_to_string(fixture.path()).unwrap();
    let head_before = std::fs::read_to_string(fixture.directory.join(".git/refs/heads/main")).ok();

    let repository = fixture.open();
    let _ = repository.semantic_history(&fixture.path(), HistoryOptions::default());
    let _ = repository.read_at("HEAD~1", &fixture.path());
    let _ = repository.blame_node(&fixture.path(), "api", HistoryOptions::default());

    assert_eq!(
        std::fs::read_to_string(fixture.path()).unwrap(),
        before,
        "working tree changed"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.directory.join(".git/refs/heads/main")).ok(),
        head_before,
        "the branch ref moved"
    );
}

#[test]
fn a_path_relative_to_the_repository_root_resolves() {
    let Some(fixture) = Fixture::new("relative") else {
        return;
    };
    fixture.commit(V1, "initial");

    let repository = fixture.open();
    assert_eq!(
        repository.relative_path(&fixture.path()),
        Path::new("architecture.yaml")
    );
}

#[test]
fn a_nested_architecture_file_is_found() {
    let Some(fixture) = Fixture::new("nested") else {
        return;
    };
    let nested = fixture.directory.join("systems").join("payments");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("architecture.yaml"), V1).unwrap();
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "--quiet", "-m", "add payments"]);

    let repository = fixture.open();
    let history = repository
        .semantic_history(&nested.join("architecture.yaml"), HistoryOptions::default())
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].summary, "add payments");
}
