//! Module: `casm_cli::exit`
//! Purpose: The single authority on what each process exit code means.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why this is its own module
//!
//! Exit codes are the contract between CASM and every CI system that runs it. A shell
//! script cannot read a diagnostic; it reads `$?`. Defining the codes in one place, with
//! their meanings attached, is what stops `validate` and `check` from disagreeing about
//! whether warnings should fail a build.

/// Every exit status `casm` can return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub(crate) enum ExitCode {
    /// The command succeeded with no findings worth acting on.
    Success = 0,
    /// Validation produced warnings, but no errors.
    Warnings = 1,
    /// Validation produced errors: the architecture is not fit to build against.
    ValidationErrors = 2,
    /// The command itself failed: bad arguments, unreadable file, malformed document.
    ///
    /// Distinct from [`Self::ValidationErrors`] so that CI can tell "your architecture is
    /// wrong" apart from "the tool could not run", which need different responses.
    Failure = 3,
}

impl ExitCode {
    /// The numeric status to hand to the operating system.
    #[must_use]
    pub(crate) const fn code(self) -> i32 {
        self as i32
    }

    /// Maps a validator report's code onto an `ExitCode`.
    ///
    /// The mapping lives here rather than in the validator so that the CLI is the only
    /// place that knows about process semantics.
    #[must_use]
    pub(crate) const fn from_report_code(code: i32) -> Self {
        match code {
            0 => Self::Success,
            1 => Self::Warnings,
            _ => Self::ValidationErrors,
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

    #[test]
    fn codes_match_the_documented_contract() {
        assert_eq!(ExitCode::Success.code(), 0);
        assert_eq!(ExitCode::Warnings.code(), 1);
        assert_eq!(ExitCode::ValidationErrors.code(), 2);
        assert_eq!(ExitCode::Failure.code(), 3);
    }

    #[test]
    fn report_codes_map_onto_exit_codes() {
        assert_eq!(ExitCode::from_report_code(0), ExitCode::Success);
        assert_eq!(ExitCode::from_report_code(1), ExitCode::Warnings);
        assert_eq!(ExitCode::from_report_code(2), ExitCode::ValidationErrors);
    }

    #[test]
    fn an_unexpected_report_code_is_treated_as_an_error() {
        // Defensive: a future severity must never be silently downgraded to success.
        assert_eq!(ExitCode::from_report_code(99), ExitCode::ValidationErrors);
        assert_eq!(ExitCode::from_report_code(-1), ExitCode::ValidationErrors);
    }

    #[test]
    fn tool_failure_is_distinguishable_from_validation_failure() {
        // CI needs to tell "your architecture is wrong" from "the tool broke".
        assert_ne!(ExitCode::Failure.code(), ExitCode::ValidationErrors.code());
    }
}
