//! Module: `casm_cli::hook`
//! Purpose: Installing a pre-commit hook that validates architectures before they land.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # The only place CASM writes to a repository
//!
//! Everything else in `casm-git` is strictly read-only. Installing a hook is the
//! exception, and it is guarded accordingly: an existing hook is never overwritten
//! without `--force`, and the file carries a marker line so `uninstall` can tell its own
//! work from somebody else's.
//!
//! # Why the hook is lenient by default
//!
//! `casm validate` exits `1` on warnings and `2` on errors. A hook that blocked on
//! warnings would block on "this service should declare two security controls" — true,
//! worth fixing, and not worth refusing a commit over. Warnings get printed; only errors
//! stop the commit.
//!
//! The hook also exits cleanly when `casm` is not on `PATH`. A contributor who has not
//! installed the tool should still be able to commit; a hook that bricks a clone is a
//! hook that gets deleted.

use std::path::{Path, PathBuf};

/// Identifies a hook as CASM's, so `uninstall` never removes somebody else's.
pub(crate) const MARKER: &str = "# casm-hook v1 — managed by `casm hook install`";

/// The pre-commit script installed into a repository.
pub(crate) const SCRIPT: &str = r#"#!/bin/sh
# casm-hook v1 — managed by `casm hook install`
#
# Validates every staged CASM architecture file. Warnings are printed; only errors
# (exit code 2 or above) refuse the commit. Remove with `casm hook uninstall`.

if ! command -v casm >/dev/null 2>&1; then
  echo "casm: not installed, skipping architecture validation" >&2
  exit 0
fi

staged=$(git diff --cached --name-only --diff-filter=ACM \
  | grep -E '(architecture\.ya?ml|\.casm\.ya?ml)$' || true)

if [ -z "$staged" ]; then
  exit 0
fi

failed=0
for file in $staged; do
  casm validate "$file"
  code=$?
  if [ "$code" -ge 2 ]; then
    failed=1
  fi
done

if [ "$failed" -ne 0 ]; then
  echo "" >&2
  echo "casm: refusing the commit — the architecture has errors." >&2
  echo "      Fix them, or bypass this check with 'git commit --no-verify'." >&2
  exit 1
fi

exit 0
"#;

/// Where a repository keeps its hooks.
#[must_use]
pub(crate) fn hooks_directory(git_dir: &Path) -> PathBuf {
    git_dir.join("hooks")
}

/// The pre-commit hook's path within a repository.
#[must_use]
pub(crate) fn hook_path(git_dir: &Path) -> PathBuf {
    hooks_directory(git_dir).join("pre-commit")
}

/// What is currently installed at the hook path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    /// No pre-commit hook exists.
    Absent,
    /// CASM's hook is installed.
    Installed,
    /// Someone else's hook is installed.
    Foreign,
}

/// Inspects the hook at `git_dir`.
pub(crate) fn status(git_dir: &Path) -> Status {
    match std::fs::read_to_string(hook_path(git_dir)) {
        Err(_) => Status::Absent,
        Ok(contents) if contents.contains(MARKER) => Status::Installed,
        Ok(_) => Status::Foreign,
    }
}

/// Makes a file executable on Unix.
///
/// Git ignores a hook that is not executable, silently — so an install that skipped this
/// would appear to succeed and do nothing.
#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)
}

/// Windows has no executable bit; Git runs hooks through its bundled shell regardless.
#[cfg(not(unix))]
fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Writes the hook, refusing to clobber a foreign one unless `force` is set.
///
/// # Errors
///
/// Returns a human-readable message if a foreign hook is present without `force`, or if
/// the file cannot be written.
pub(crate) fn install(git_dir: &Path, force: bool) -> Result<Status, String> {
    let existing = status(git_dir);

    if existing == Status::Foreign && !force {
        return Err(format!(
            "'{}' already exists and was not written by CASM.\n\
             Inspect it first, then pass --force to replace it.",
            hook_path(git_dir).display()
        ));
    }

    let directory = hooks_directory(git_dir);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create '{}': {error}", directory.display()))?;

    let path = hook_path(git_dir);
    std::fs::write(&path, SCRIPT)
        .map_err(|error| format!("cannot write '{}': {error}", path.display()))?;
    make_executable(&path)
        .map_err(|error| format!("cannot make '{}' executable: {error}", path.display()))?;

    Ok(existing)
}

/// Removes CASM's hook, leaving a foreign one alone.
///
/// # Errors
///
/// Returns a human-readable message if a foreign hook is present, or if removal fails.
pub(crate) fn uninstall(git_dir: &Path) -> Result<Status, String> {
    let existing = status(git_dir);

    match existing {
        Status::Absent => Ok(existing),
        Status::Foreign => Err(format!(
            "'{}' was not written by CASM; leaving it alone.",
            hook_path(git_dir).display()
        )),
        Status::Installed => {
            let path = hook_path(git_dir);
            std::fs::remove_file(&path)
                .map_err(|error| format!("cannot remove '{}': {error}", path.display()))?;
            Ok(existing)
        }
    }
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

    /// A throwaway `.git` directory.
    fn git_dir(label: &str) -> PathBuf {
        let unique = casm_core::NodeId::new();
        let directory = std::env::temp_dir().join(format!("casm-hook-{label}-{unique}"));
        std::fs::create_dir_all(&directory).expect("temp dir");
        directory
    }

    #[test]
    fn a_fresh_repository_has_no_hook() {
        let directory = git_dir("fresh");
        assert_eq!(status(&directory), Status::Absent);
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn installing_creates_the_hook_and_reports_it() {
        let directory = git_dir("install");

        assert_eq!(install(&directory, false), Ok(Status::Absent));
        assert_eq!(status(&directory), Status::Installed);
        assert!(hook_path(&directory).exists());

        std::fs::remove_dir_all(&directory).ok();
    }

    #[cfg(unix)]
    #[test]
    fn the_installed_hook_is_executable() {
        // Git ignores a non-executable hook silently, so this is the difference between
        // working and appearing to work.
        use std::os::unix::fs::PermissionsExt as _;
        let directory = git_dir("executable");
        install(&directory, false).unwrap();

        let mode = std::fs::metadata(hook_path(&directory))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "mode was {mode:o}");

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn installing_twice_is_idempotent() {
        let directory = git_dir("twice");
        install(&directory, false).unwrap();
        assert_eq!(install(&directory, false), Ok(Status::Installed));
        assert_eq!(status(&directory), Status::Installed);
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_foreign_hook_is_never_clobbered_without_force() {
        let directory = git_dir("foreign");
        std::fs::create_dir_all(hooks_directory(&directory)).unwrap();
        std::fs::write(
            hook_path(&directory),
            "#!/bin/sh\necho someone elses hook\n",
        )
        .unwrap();

        assert_eq!(status(&directory), Status::Foreign);
        let error = install(&directory, false).unwrap_err();
        assert!(error.contains("--force"), "{error}");
        assert!(
            std::fs::read_to_string(hook_path(&directory))
                .unwrap()
                .contains("someone elses"),
            "the existing hook was modified"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn force_replaces_a_foreign_hook() {
        let directory = git_dir("force");
        std::fs::create_dir_all(hooks_directory(&directory)).unwrap();
        std::fs::write(hook_path(&directory), "#!/bin/sh\nexit 0\n").unwrap();

        assert_eq!(install(&directory, true), Ok(Status::Foreign));
        assert_eq!(status(&directory), Status::Installed);

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn uninstalling_removes_only_our_own_hook() {
        let directory = git_dir("uninstall");
        install(&directory, false).unwrap();

        assert_eq!(uninstall(&directory), Ok(Status::Installed));
        assert_eq!(status(&directory), Status::Absent);
        assert!(!hook_path(&directory).exists());

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn uninstalling_refuses_to_remove_a_foreign_hook() {
        let directory = git_dir("uninstall-foreign");
        std::fs::create_dir_all(hooks_directory(&directory)).unwrap();
        std::fs::write(hook_path(&directory), "#!/bin/sh\nexit 0\n").unwrap();

        assert!(uninstall(&directory).is_err());
        assert!(
            hook_path(&directory).exists(),
            "somebody else's hook was deleted"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn uninstalling_when_absent_is_not_an_error() {
        let directory = git_dir("uninstall-absent");
        assert_eq!(uninstall(&directory), Ok(Status::Absent));
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn the_script_carries_the_marker_that_identifies_it() {
        assert!(
            SCRIPT.contains(MARKER),
            "install and uninstall would disagree"
        );
    }

    #[test]
    fn the_script_is_lenient_about_warnings_and_a_missing_binary() {
        // Both are deliberate: see the module documentation.
        assert!(SCRIPT.contains("-ge 2"), "warnings must not block a commit");
        assert!(
            SCRIPT.contains("command -v casm"),
            "a missing binary must not brick a clone"
        );
        assert!(
            SCRIPT.contains("--no-verify"),
            "the escape hatch must be discoverable"
        );
    }

    #[test]
    fn the_script_matches_the_filenames_the_extension_recognises() {
        // Kept in step with editors/vscode/package.json.
        assert!(SCRIPT.contains("architecture"), "{SCRIPT}");
        assert!(SCRIPT.contains("casm"), "{SCRIPT}");
    }

    #[cfg(unix)]
    #[test]
    fn the_installed_script_is_accepted_by_the_shell() {
        // A syntax error would only surface at commit time, on somebody else's machine.
        let directory = git_dir("shellcheck");
        install(&directory, false).unwrap();

        let checked = std::process::Command::new("sh")
            .arg("-n")
            .arg(hook_path(&directory))
            .output();

        if let Ok(result) = checked {
            assert!(
                result.status.success(),
                "the hook is not valid shell: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }

        std::fs::remove_dir_all(&directory).ok();
    }
}
