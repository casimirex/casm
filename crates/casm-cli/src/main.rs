//! Module: `casm_cli`
//! Purpose: The `casm` binary — CASIMIR's primary human interface.
//! Safety: `#![forbid(unsafe_code)]` — verified via Miri in CI.
//! Complexity: Max 10 per function (enforced by clippy).
//! License: Apache-2.0
//!
//! # Exit codes are the contract
//!
//! | Code | Meaning |
//! |---|---|
//! | `0` | Success, or findings not worth acting on |
//! | `1` | Validation warnings |
//! | `2` | Validation errors — the architecture is not fit to build against |
//! | `3` | The command itself failed: bad arguments, unreadable file, malformed document |
//!
//! Codes `2` and `3` are deliberately distinct. "Your architecture is wrong" and "the
//! tool could not run" demand different responses from a pipeline, and collapsing them
//! into a single non-zero code makes that distinction unrecoverable.
//!
//! # NASA compliance
//!
//! Rule 3 (no panics): `main` never unwraps. Every failure path returns a
//! [`commands::CommandError`] carrying a message already formatted for the user, which
//! is printed to standard error before the process exits with code `3`.

#![forbid(unsafe_code)]

mod cli;
mod commands;
mod exit;
mod hook;

use clap::Parser as _;

use cli::{Cli, Command};
use commands::CommandResult;
use exit::ExitCode;

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    match dispatch(cli) {
        Ok(code) => std::process::ExitCode::from(clamp(code.code())),
        Err(error) => {
            eprintln!("error: {}", error.0);
            std::process::ExitCode::from(clamp(ExitCode::Failure.code()))
        }
    }
}

/// Narrows an exit code into the byte the operating system accepts.
///
/// Every code CASIMIR produces is already in `0..=3`; the clamp exists so the conversion
/// is total and cannot panic (NASA Rule 3).
fn clamp(code: i32) -> u8 {
    u8::try_from(code).unwrap_or(u8::MAX)
}

/// Routes a parsed command line to its implementation.
fn dispatch(cli: Cli) -> CommandResult {
    match cli.command {
        Command::Init {
            name,
            output,
            force,
        } => commands::init(&name, &output, force),

        Command::Validate {
            file,
            format,
            strict,
            allow,
            max_critical_path_ms,
            min_security_controls,
        } => commands::validate(
            &file,
            format,
            strict,
            &allow,
            max_critical_path_ms,
            min_security_controls,
        ),

        Command::Generate {
            file,
            format,
            output,
        } => commands::generate(&file, format, output.as_deref()),

        Command::Diff {
            old,
            new,
            fail_on_breaking,
        } => commands::diff(&old, &new, fail_on_breaking),

        Command::Check { directory, strict } => commands::check(&directory, strict),

        Command::Fmt {
            file,
            format,
            write,
        } => commands::fmt(&file, format, write),

        Command::Drift {
            file,
            inventory,
            from,
            format,
            fail_on_drift,
        } => commands::drift(&file, &inventory, from, format, fail_on_drift),

        Command::Log {
            file,
            limit,
            format,
        } => commands::log(&file, limit, format),

        Command::Blame {
            node,
            file,
            limit,
            format,
        } => commands::blame(&file, &node, limit, format),

        Command::Checkout {
            revision,
            file,
            validate,
        } => commands::checkout(&revision, &file, validate),

        Command::Hook { action } => commands::manage_hook(&action),

        Command::Rules { json } => commands::rules(json),
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
    fn every_exit_code_survives_the_narrowing_conversion() {
        for code in [
            ExitCode::Success,
            ExitCode::Warnings,
            ExitCode::ValidationErrors,
            ExitCode::Failure,
        ] {
            let narrowed = clamp(code.code());
            assert_eq!(i32::from(narrowed), code.code(), "{code:?} did not survive");
        }
    }

    #[test]
    fn clamping_is_total_for_out_of_range_input() {
        // Nothing produces these, but the conversion must never panic.
        assert_eq!(clamp(-1), u8::MAX);
        assert_eq!(clamp(i32::MAX), u8::MAX);
        assert_eq!(clamp(0), 0);
    }

    #[test]
    fn dispatch_routes_the_rules_command() {
        let cli = Cli::try_parse_from(["casm", "rules"]).unwrap();
        assert_eq!(dispatch(cli).map_or(-1, ExitCode::code), 0);
    }
}
