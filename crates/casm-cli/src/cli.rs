//! Module: `casm_cli::cli`
//! Purpose: The command surface — every flag, subcommand, and its help text.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # NASA compliance
//!
//! Rule 10 (observability is not optional): every subcommand can emit machine-readable
//! output. A command that only speaks to a terminal cannot be monitored, and a pipeline
//! that scrapes human prose breaks the first time a message is reworded.

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// The default architecture file name, used when a command is given no path.
pub(crate) const DEFAULT_ARCHITECTURE_FILE: &str = "architecture.yaml";

/// CASIMIR — a NASA-grade, Rust-native Architecture-as-Code platform.
///
/// Exit codes: `0` success, `1` warnings, `2` validation errors, `3` command failure.
#[derive(Debug, Parser)]
#[command(
    name = "casm",
    version,
    about = "CASIMIR — architecture as code, validated like flight software",
    long_about = None,
    propagate_version = true
)]
pub(crate) struct Cli {
    /// The subcommand to run.
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// How a command should present its results.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputFormat {
    /// Prose for a terminal.
    Human,
    /// JSON for scripts and dashboards.
    Json,
    /// SARIF 2.1.0 for CI code-scanning integrations.
    Sarif,
}

/// The diagram backends `generate` can target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum DiagramFormat {
    /// Mermaid flowchart, renders natively on GitHub.
    Mermaid,
    /// Graphviz DOT.
    Dot,
    /// Plain text, readable in a CI log.
    Ascii,
}

impl DiagramFormat {
    /// The renderer identifier this format selects.
    #[must_use]
    pub(crate) const fn renderer_id(self) -> &'static str {
        match self {
            Self::Mermaid => "mermaid",
            Self::Dot => "dot",
            Self::Ascii => "ascii",
        }
    }
}

/// Where an inventory of real infrastructure comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum InventorySource {
    /// CASIMIR's own inventory schema.
    Native,
    /// A Terraform state file, projected through CASIMIR's resource-type map.
    Terraform,
}

/// The formal-methods tools `formal` can target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum FormalTarget {
    /// TLA+, for failure and recovery over time.
    Tla,
    /// Alloy, for static structure and counterexamples.
    Alloy,
    /// Both.
    All,
}

/// The serialisation formats `fmt` can emit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum DocumentFormat {
    /// YAML — the authoring default.
    Yaml,
    /// JSON.
    Json,
    /// TOML.
    Toml,
}

impl DocumentFormat {
    /// The parser format this selection maps onto.
    #[must_use]
    pub(crate) const fn as_parser_format(self) -> casm_parser::Format {
        match self {
            Self::Yaml => casm_parser::Format::Yaml,
            Self::Json => casm_parser::Format::Json,
            Self::Toml => casm_parser::Format::Toml,
        }
    }
}

/// Every CASIMIR subcommand.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Scaffold a new architecture file.
    Init {
        /// The architecture's name.
        #[arg(short, long, default_value = "my-architecture")]
        name: String,

        /// Where to write the scaffolded file.
        #[arg(short, long, default_value = DEFAULT_ARCHITECTURE_FILE)]
        output: PathBuf,

        /// Overwrite the file if it already exists.
        #[arg(long)]
        force: bool,
    },

    /// Validate an architecture against the built-in rule library.
    Validate {
        /// The architecture file to validate.
        #[arg(default_value = DEFAULT_ARCHITECTURE_FILE)]
        file: PathBuf,

        /// How to present the findings.
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,

        /// Treat warnings as errors.
        #[arg(long)]
        strict: bool,

        /// Suppress a rule by its identifier. May be repeated.
        #[arg(long, value_name = "RULE")]
        allow: Vec<String>,

        /// The end-to-end latency ceiling for the critical path, in milliseconds.
        #[arg(long, value_name = "MS")]
        max_critical_path_ms: Option<u64>,

        /// How many security controls each service must declare.
        #[arg(long, value_name = "N")]
        min_security_controls: Option<usize>,
    },

    /// Generate a diagram from an architecture.
    Generate {
        /// The architecture file to render.
        #[arg(default_value = DEFAULT_ARCHITECTURE_FILE)]
        file: PathBuf,

        /// The diagram backend to use.
        #[arg(short, long, value_enum, default_value_t = DiagramFormat::Mermaid)]
        format: DiagramFormat,

        /// Where to write the diagram. Defaults to standard output.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Show the semantic difference between two architecture versions.
    Diff {
        /// The baseline architecture.
        old: PathBuf,

        /// The architecture to compare against the baseline.
        new: PathBuf,

        /// Exit with a failure code if any change is breaking.
        #[arg(long)]
        fail_on_breaking: bool,
    },

    /// Validate every architecture file under a directory.
    Check {
        /// The directory to search.
        #[arg(default_value = ".")]
        directory: PathBuf,

        /// Treat warnings as errors.
        #[arg(long)]
        strict: bool,
    },

    /// Reformat or convert an architecture file between YAML, JSON, and TOML.
    Fmt {
        /// The architecture file to reformat.
        #[arg(default_value = DEFAULT_ARCHITECTURE_FILE)]
        file: PathBuf,

        /// The format to emit.
        #[arg(short, long, value_enum, default_value_t = DocumentFormat::Yaml)]
        format: DocumentFormat,

        /// Rewrite the file in place instead of printing to standard output.
        #[arg(long)]
        write: bool,
    },

    /// Compare a declared architecture against infrastructure that actually exists.
    Drift {
        /// The architecture file to check.
        #[arg(default_value = DEFAULT_ARCHITECTURE_FILE)]
        file: PathBuf,

        /// The inventory of observed infrastructure.
        #[arg(short, long)]
        inventory: PathBuf,

        /// How to read the inventory.
        #[arg(long, value_enum, default_value_t = InventorySource::Native)]
        from: InventorySource,

        /// How to present the findings.
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,

        /// Exit with a failure code if any drift is found.
        #[arg(long)]
        fail_on_drift: bool,
    },

    /// Show the commits at which the architecture's meaning changed.
    ///
    /// Unlike `git log`, commits that only reformatted or reordered the file are omitted.
    Log {
        /// The architecture file whose history to read.
        #[arg(default_value = DEFAULT_ARCHITECTURE_FILE)]
        file: PathBuf,

        /// The most changes to report.
        #[arg(short, long, default_value_t = 20)]
        limit: usize,

        /// How to present the history.
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },

    /// Show the commits at which one node's meaning changed.
    Blame {
        /// The node's name.
        node: String,

        /// The architecture file whose history to read.
        #[arg(default_value = DEFAULT_ARCHITECTURE_FILE)]
        file: PathBuf,

        /// The most changes to report.
        #[arg(short, long, default_value_t = 20)]
        limit: usize,

        /// How to present the history.
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },

    /// Print an architecture as it was at a past revision.
    ///
    /// Writes to standard output; the working tree is never touched.
    Checkout {
        /// The revision to read, such as `HEAD~3`, a tag, or a commit hash.
        revision: String,

        /// The architecture file to read.
        #[arg(default_value = DEFAULT_ARCHITECTURE_FILE)]
        file: PathBuf,

        /// Validate the reconstructed architecture instead of printing it.
        #[arg(long)]
        validate: bool,
    },

    /// Export the architecture as a formal specification.
    ///
    /// TLA+ models failure propagation over time; Alloy models static structure.
    /// Both restate CASIMIR's own rules as machine-checked assertions.
    Formal {
        /// The architecture file to export.
        #[arg(default_value = DEFAULT_ARCHITECTURE_FILE)]
        file: PathBuf,

        /// Which tool to target.
        #[arg(short, long, value_enum, default_value_t = FormalTarget::All)]
        target: FormalTarget,

        /// Where to write the specifications. Defaults to standard output.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Manage the Git pre-commit hook that validates architectures.
    Hook {
        /// What to do with the hook.
        #[command(subcommand)]
        action: HookAction,
    },

    /// List the built-in validation rules.
    Rules {
        /// Emit the rule catalogue as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// What `casm hook` can do.
#[derive(Debug, Subcommand)]
pub(crate) enum HookAction {
    /// Install the pre-commit hook.
    Install {
        /// Replace an existing hook that CASIMIR did not write.
        #[arg(long)]
        force: bool,
    },
    /// Remove the pre-commit hook, if CASIMIR wrote it.
    Uninstall,
    /// Report whether the hook is installed.
    Status,
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
    use clap::CommandFactory;

    #[test]
    fn the_command_definition_is_internally_consistent() {
        // clap's own assertions catch conflicting flags, bad defaults, and duplicate
        // short options at test time rather than at a user's first invocation.
        Cli::command().debug_assert();
    }

    #[test]
    fn validate_defaults_to_the_conventional_file_and_human_output() {
        let cli = Cli::try_parse_from(["casm", "validate"]).unwrap();
        match cli.command {
            Command::Validate {
                file,
                format,
                strict,
                ..
            } => {
                assert_eq!(file, PathBuf::from(DEFAULT_ARCHITECTURE_FILE));
                assert_eq!(format, OutputFormat::Human);
                assert!(!strict);
            }
            other => panic!("expected Validate, got {other:?}"),
        }
    }

    #[test]
    fn validate_accepts_repeated_allow_flags() {
        let cli = Cli::try_parse_from([
            "casm",
            "validate",
            "--allow",
            "no-isolated-nodes",
            "--allow",
            "stateful-nodes-require-controls",
        ])
        .unwrap();

        match cli.command {
            Command::Validate { allow, .. } => assert_eq!(allow.len(), 2),
            other => panic!("expected Validate, got {other:?}"),
        }
    }

    #[test]
    fn validate_accepts_threshold_overrides() {
        let cli = Cli::try_parse_from([
            "casm",
            "validate",
            "--max-critical-path-ms",
            "250",
            "--min-security-controls",
            "1",
        ])
        .unwrap();

        match cli.command {
            Command::Validate {
                max_critical_path_ms,
                min_security_controls,
                ..
            } => {
                assert_eq!(max_critical_path_ms, Some(250));
                assert_eq!(min_security_controls, Some(1));
            }
            other => panic!("expected Validate, got {other:?}"),
        }
    }

    #[test]
    fn output_formats_parse_from_their_kebab_case_names() {
        for (flag, expected) in [
            ("human", OutputFormat::Human),
            ("json", OutputFormat::Json),
            ("sarif", OutputFormat::Sarif),
        ] {
            let cli = Cli::try_parse_from(["casm", "validate", "--format", flag]).unwrap();
            match cli.command {
                Command::Validate { format, .. } => assert_eq!(format, expected),
                other => panic!("expected Validate, got {other:?}"),
            }
        }
    }

    #[test]
    fn an_unknown_output_format_is_rejected() {
        assert!(Cli::try_parse_from(["casm", "validate", "--format", "xml"]).is_err());
    }

    #[test]
    fn diagram_formats_map_onto_renderer_ids() {
        assert_eq!(DiagramFormat::Mermaid.renderer_id(), "mermaid");
        assert_eq!(DiagramFormat::Dot.renderer_id(), "dot");
        assert_eq!(DiagramFormat::Ascii.renderer_id(), "ascii");
    }

    #[test]
    fn every_diagram_format_resolves_to_a_real_backend() {
        for format in [
            DiagramFormat::Mermaid,
            DiagramFormat::Dot,
            DiagramFormat::Ascii,
        ] {
            assert!(
                casm_renderer::by_id(format.renderer_id()).is_some(),
                "{format:?} names no backend"
            );
        }
    }

    #[test]
    fn document_formats_map_onto_parser_formats() {
        assert_eq!(
            DocumentFormat::Yaml.as_parser_format(),
            casm_parser::Format::Yaml
        );
        assert_eq!(
            DocumentFormat::Json.as_parser_format(),
            casm_parser::Format::Json
        );
        assert_eq!(
            DocumentFormat::Toml.as_parser_format(),
            casm_parser::Format::Toml
        );
    }

    #[test]
    fn diff_requires_both_operands() {
        assert!(Cli::try_parse_from(["casm", "diff"]).is_err());
        assert!(Cli::try_parse_from(["casm", "diff", "a.yaml"]).is_err());
        assert!(Cli::try_parse_from(["casm", "diff", "a.yaml", "b.yaml"]).is_ok());
    }

    #[test]
    fn check_defaults_to_the_current_directory() {
        let cli = Cli::try_parse_from(["casm", "check"]).unwrap();
        match cli.command {
            Command::Check { directory, .. } => assert_eq!(directory, PathBuf::from(".")),
            other => panic!("expected Check, got {other:?}"),
        }
    }

    #[test]
    fn init_defaults_to_a_named_scaffold_without_force() {
        let cli = Cli::try_parse_from(["casm", "init"]).unwrap();
        match cli.command {
            Command::Init {
                name,
                output,
                force,
            } => {
                assert_eq!(name, "my-architecture");
                assert_eq!(output, PathBuf::from(DEFAULT_ARCHITECTURE_FILE));
                assert!(!force, "init must not clobber by default");
            }
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["casm", "teleport"]).is_err());
    }

    #[test]
    fn every_subcommand_has_help_text() {
        for subcommand in Cli::command().get_subcommands() {
            assert!(
                subcommand.get_about().is_some(),
                "subcommand '{}' has no help text",
                subcommand.get_name()
            );
        }
    }
}
