//! Module: `casm_cli`
//! Purpose: The `casm` binary — CASM's primary human interface.
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
//!
//! Rule 10 (observability): every run is instrumented, whether or not anybody asked for
//! the telemetry. Recording costs a timestamp and a push; deciding at the top of `main`
//! whether to instrument would mean two code paths, one of which is never exercised.
//! `--telemetry <format>` chooses what to do with what was collected, not whether to
//! collect it.

#![forbid(unsafe_code)]

mod cli;
mod commands;
mod exit;
mod hook;

use casm_telemetry::{Outcome, Recorder, Resource, Severity, sink};
use clap::Parser as _;

use cli::{Cli, Command};
use commands::CommandResult;
use exit::ExitCode;

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let telemetry = cli.telemetry;
    let name = cli.command.name();

    let mut recorder = Recorder::new(
        Resource::new("casm", env!("CARGO_PKG_VERSION")).with_attribute("casm.command", name),
    );

    let span = recorder.start(name);
    let result = dispatch(cli, &mut recorder);

    // Recorded before the span closes, so it carries the span id and a log pipeline can
    // correlate the line with the trace.
    recorder.event(severity_of(&result), summarise(name, &result));
    recorder.finish(span, outcome_of(&result));

    if let Some(format) = telemetry {
        // Stderr: stdout carries the command's output, and a pipeline parsing it must not
        // receive timing data mixed in. A telemetry write that fails is reported and does
        // not change the command's own exit code — the work succeeded either way.
        if let Err(error) = sink::write(&recorder, format.as_sink_format(), &mut std::io::stderr())
        {
            eprintln!("warning: could not write telemetry: {error}");
        }
    }

    match result {
        Ok(code) => std::process::ExitCode::from(clamp(code.code())),
        Err(error) => {
            eprintln!("error: {}", error.0);
            std::process::ExitCode::from(clamp(ExitCode::Failure.code()))
        }
    }
}

/// How a command's result reads as a span outcome.
///
/// Validation findings are *not* an error: the command did its job and reported what it
/// found. Only a command that could not run is an error, which keeps a trace searchable
/// for real failures rather than for architectures that need work.
fn outcome_of(result: &CommandResult) -> Outcome {
    match result {
        Ok(_) => Outcome::Ok,
        Err(_) => Outcome::Error,
    }
}

/// How a command's result reads as an event severity.
///
/// Deliberately different from [`outcome_of`]. The span outcome answers "did the tool
/// work"; the event severity answers "what did it find", which is what somebody filtering
/// a log stream is asking. An architecture with errors is a healthy run of `casm` and an
/// `ERROR` line in the log, and both readings are correct.
fn severity_of(result: &CommandResult) -> Severity {
    match result {
        Ok(ExitCode::Success) => Severity::Info,
        Ok(ExitCode::Warnings) => Severity::Warn,
        Ok(ExitCode::ValidationErrors | ExitCode::Failure) | Err(_) => Severity::Error,
    }
}

/// One line describing how the command ended, for a log stream.
fn summarise(name: &'static str, result: &CommandResult) -> String {
    match result {
        Ok(code) => format!("{name} finished with exit code {}", code.code()),
        Err(error) => format!("{name} failed: {}", error.0),
    }
}

/// Narrows an exit code into the byte the operating system accepts.
///
/// Every code CASM produces is already in `0..=3`; the clamp exists so the conversion
/// is total and cannot panic (NASA Rule 3).
fn clamp(code: i32) -> u8 {
    u8::try_from(code).unwrap_or(u8::MAX)
}

/// Routes a parsed command line to its implementation.
fn dispatch(cli: Cli, recorder: &mut Recorder) -> CommandResult {
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
            patterns,
        } => commands::validate(
            &file,
            format,
            strict,
            &allow,
            max_critical_path_ms,
            min_security_controls,
            patterns.as_deref(),
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

        Command::Check {
            directory,
            strict,
            patterns,
        } => commands::check(&directory, strict, patterns.as_deref(), recorder),

        Command::Evolve {
            file,
            patterns,
            to,
            strict,
        } => commands::evolve(&file, &patterns, to.as_deref(), strict),

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

        Command::Formal {
            file,
            target,
            output,
        } => commands::formal(&file, target, output.as_deref()),

        Command::Evidence {
            file,
            format,
            patterns,
            no_history,
            strict,
        } => commands::evidence(&file, format, patterns.as_deref(), no_history, strict),

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

    fn recorder() -> Recorder {
        Recorder::new(Resource::new("casm", "test"))
    }

    #[test]
    fn dispatch_routes_the_rules_command() {
        let cli = Cli::try_parse_from(["casm", "rules"]).unwrap();
        assert_eq!(dispatch(cli, &mut recorder()).map_or(-1, ExitCode::code), 0);
    }

    #[test]
    fn every_run_is_instrumented_even_without_the_flag() {
        // Instrumentation is unconditional; the flag only decides what is done with it.
        // Two code paths, one of them never exercised, is how telemetry rots.
        let cli = Cli::try_parse_from(["casm", "rules"]).unwrap();
        assert!(cli.telemetry.is_none());

        let mut recorder = recorder();
        let span = recorder.start(cli.command.name());
        let result = dispatch(cli, &mut recorder);
        recorder.finish(span, outcome_of(&result));

        assert_eq!(recorder.spans().len(), 1);
        assert_eq!(recorder.spans()[0].name, "rules");
    }

    #[test]
    fn the_telemetry_flag_is_accepted_before_or_after_the_subcommand() {
        // `global = true`, so a user does not have to remember which side it goes on.
        for arguments in [
            ["casm", "--telemetry", "summary", "rules"],
            ["casm", "rules", "--telemetry", "summary"],
        ] {
            let cli = Cli::try_parse_from(arguments).unwrap();
            assert!(cli.telemetry.is_some(), "{arguments:?}");
        }
    }

    #[test]
    fn every_run_records_one_event_correlated_with_its_span() {
        // Without this the logs signal is empty, and "the collector accepted our logs"
        // would be a vacuous claim — an OTLP receiver ignores unknown fields, so an empty
        // request and a completely wrong encoding both answer 200.
        let cli = Cli::try_parse_from(["casm", "rules"]).unwrap();
        let mut recorder = recorder();

        let span = recorder.start(cli.command.name());
        let result = dispatch(cli, &mut recorder);
        recorder.event(severity_of(&result), summarise("rules", &result));
        recorder.finish(span, outcome_of(&result));

        assert_eq!(recorder.events().len(), 1);
        assert_eq!(
            recorder.events()[0].span_id,
            Some(recorder.spans()[0].span_id)
        );
        assert!(recorder.events()[0].message.contains("rules finished"));
    }

    #[test]
    fn event_severity_reports_findings_where_the_span_outcome_reports_health() {
        // The two answer different questions, and a run that found errors is a healthy
        // run of the tool.
        assert_eq!(severity_of(&Ok(ExitCode::Success)), Severity::Info);
        assert_eq!(severity_of(&Ok(ExitCode::Warnings)), Severity::Warn);
        assert_eq!(
            severity_of(&Ok(ExitCode::ValidationErrors)),
            Severity::Error
        );
        assert_eq!(outcome_of(&Ok(ExitCode::ValidationErrors)), Outcome::Ok);
    }

    #[test]
    fn a_failing_command_is_an_error_outcome_but_findings_are_not() {
        // A trace searched for failures must not surface every architecture that has
        // warnings; those are the command working correctly.
        assert_eq!(outcome_of(&Ok(ExitCode::Success)), Outcome::Ok);
        assert_eq!(outcome_of(&Ok(ExitCode::ValidationErrors)), Outcome::Ok);
        assert_eq!(
            outcome_of(&Err(commands::CommandError("unreadable".to_owned()))),
            Outcome::Error
        );
    }
}
