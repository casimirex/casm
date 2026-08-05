//! Module: `casm_cli::commands`
//! Purpose: The implementation of each CASIMIR subcommand.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # I/O is confined to this module
//!
//! Everything below `casm-cli` is a pure function of its inputs. This module is where
//! files are read, standard output is written, and exit codes are decided. Keeping the
//! boundary sharp is what makes the rest of CASIMIR testable without a filesystem.

use casm_core::{Architecture, Pattern, conformance};
use casm_evidence::{Attribution, Pack, Provenance, render::Format as RenderFormat};
use casm_parser::{Format, Library, emit_str, parse_file};
use casm_telemetry::{Outcome, Recorder};
use casm_validator::{Report, Validator, ValidatorConfig, sarif};
use std::path::{Path, PathBuf};

use crate::cli::{
    DiagramFormat, DocumentFormat, EvidenceFormat, FormalTarget, HookAction, InventorySource,
    OutputFormat,
};
use crate::exit::ExitCode;
use crate::hook;
use casm_diff::Diff;

/// The template `casm init` scaffolds.
///
/// Deliberately a *valid* architecture that nonetheless produces warnings: a newcomer's
/// first `casm validate` should demonstrate what the tool is for, not print "all clear"
/// and teach them nothing.
const TEMPLATE: &str = r"# CASIMIR architecture
# Validate with:  casm validate
# Diagram with:   casm generate --format mermaid

name: {{NAME}}
version: 0.1.0
description: Describe what this system does, in one sentence.

nodes:
  - name: gateway
    type: gateway
    description: Public entry point.
    interfaces:
      - name: public-api
        protocol: http2
        version: 1.0.0
    controls:
      - type: security
        standard: OIDC
        description: All requests carry a validated OIDC token.
      - type: security
        standard: TLS1.3
        description: Terminated at the edge with TLS 1.3.

  - name: orders
    type: service
    description: Owns the order lifecycle.
    interfaces:
      - name: grpc
        protocol: grpc
        version: 1.0.0

  - name: orders-db
    type: database
    description: Durable order storage.
    interfaces:
      - name: sql
        protocol: sql
        version: 15.0.0

relationships:
  - source: gateway
    target: orders
    type: sync
    protocol: grpc
    latency-budget-ms: 100

  - source: orders
    target: orders-db
    type: sync
    protocol: sql
    latency-budget-ms: 50
";

/// A command failure carrying a message already formatted for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandError(pub String);

impl From<casm_parser::ParseError> for CommandError {
    fn from(error: casm_parser::ParseError) -> Self {
        Self(error.render())
    }
}

/// The result of running a subcommand.
pub(crate) type CommandResult = Result<ExitCode, CommandError>;

/// Scaffolds a new architecture file.
pub(crate) fn init(name: &str, output: &Path, force: bool) -> CommandResult {
    if output.exists() && !force {
        return Err(CommandError(format!(
            "'{}' already exists; pass --force to overwrite it",
            output.display()
        )));
    }

    let contents = TEMPLATE.replace("{{NAME}}", name);
    std::fs::write(output, &contents)
        .map_err(|error| CommandError(format!("cannot write '{}': {error}", output.display())))?;

    // Prove the template is valid rather than trusting it: a scaffold that fails its own
    // validator is the worst possible first impression.
    let architecture = parse_file(output)?;

    println!(
        "created '{}' ({} nodes)",
        output.display(),
        architecture.node_count()
    );
    println!("  next: casm validate {}", output.display());
    Ok(ExitCode::Success)
}

/// Builds a validator configuration from command-line overrides.
fn validator_config(
    allow: &[String],
    max_critical_path_ms: Option<u64>,
    min_security_controls: Option<usize>,
) -> ValidatorConfig {
    let mut config = ValidatorConfig::new();

    if let Some(ceiling) = max_critical_path_ms {
        config = config.max_critical_path_ms(ceiling);
    }
    if let Some(minimum) = min_security_controls {
        config = config.min_security_controls_per_service(minimum);
    }
    for rule in allow {
        config = config.allowing(rule.clone());
    }

    config
}

/// Validates one architecture file.
pub(crate) fn validate(
    file: &Path,
    format: OutputFormat,
    strict: bool,
    allow: &[String],
    max_critical_path_ms: Option<u64>,
    min_security_controls: Option<usize>,
    patterns: Option<&Path>,
) -> CommandResult {
    let architecture = parse_file(file)?;
    let config = validator_config(allow, max_critical_path_ms, min_security_controls);
    let report = Validator::with_config(config)
        .with_patterns(load_patterns(patterns)?)
        .validate(&architecture);

    match format {
        OutputFormat::Human => print_human(file, &architecture, &report),
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&report)
                .map_err(|error| CommandError(format!("cannot serialise report: {error}")))?;
            println!("{json}");
        }
        OutputFormat::Sarif => {
            let text =
                sarif::to_string(&report, &file.display().to_string()).map_err(CommandError)?;
            print!("{text}");
        }
    }

    Ok(exit_code_for(&report, strict))
}

/// Maps a report onto an exit code, honouring `--strict`.
fn exit_code_for(report: &Report, strict: bool) -> ExitCode {
    if strict && report.has_warnings() {
        return ExitCode::ValidationErrors;
    }
    ExitCode::from_report_code(report.exit_code())
}

/// Prints a validation report as prose.
fn print_human(file: &Path, architecture: &Architecture, report: &Report) {
    println!(
        "{}: {} v{} — {} node(s), {} relationship(s)",
        file.display(),
        architecture.name(),
        architecture.version(),
        architecture.node_count(),
        architecture.relationship_count()
    );

    if report.is_clean() {
        println!("\n{}", report.summary());
        return;
    }

    println!();
    for diagnostic in &report.diagnostics {
        println!("{}\n", diagnostic.render());
    }
    println!("{}", report.summary());
}

/// Renders an architecture as a diagram.
pub(crate) fn generate(file: &Path, format: DiagramFormat, output: Option<&Path>) -> CommandResult {
    let architecture = parse_file(file)?;

    let backend = casm_renderer::by_id(format.renderer_id())
        .ok_or_else(|| CommandError(format!("no renderer named '{}'", format.renderer_id())))?;
    let diagram = backend.render(&architecture);

    match output {
        Some(path) => {
            std::fs::write(path, &diagram).map_err(|error| {
                CommandError(format!("cannot write '{}': {error}", path.display()))
            })?;
            println!("wrote {} ({} bytes)", path.display(), diagram.len());
        }
        None => print!("{diagram}"),
    }

    Ok(ExitCode::Success)
}

/// Shows the semantic difference between two architectures.
pub(crate) fn diff(old: &Path, new: &Path, fail_on_breaking: bool) -> CommandResult {
    let old_architecture = parse_file(old)?;
    let new_architecture = parse_file(new)?;

    let difference = Diff::compute(&old_architecture, &new_architecture);
    print!("{}", difference.render());

    if difference.has_breaking_changes() {
        println!("\nthis diff contains breaking changes");
        if fail_on_breaking {
            return Ok(ExitCode::ValidationErrors);
        }
    }

    Ok(ExitCode::Success)
}

/// Validates every architecture file found under a directory.
pub(crate) fn check(
    directory: &Path,
    strict: bool,
    patterns: Option<&Path>,
    recorder: &mut Recorder,
) -> CommandResult {
    let files = discover(directory)?;

    if files.is_empty() {
        println!(
            "no architecture files found under '{}'",
            directory.display()
        );
        return Ok(ExitCode::Success);
    }

    let validator = Validator::new().with_patterns(load_patterns(patterns)?);
    let mut worst = ExitCode::Success;
    let mut checked = 0_usize;

    for file in &files {
        // A span per file: this is the sweep where a slow document is worth finding, and
        // an aggregate over the whole directory would hide it.
        let span = recorder
            .start("check-file")
            .with_attribute("casm.file", file.display().to_string());
        let parsed = parse_file(file);
        recorder.finish(
            span,
            if parsed.is_ok() {
                Outcome::Ok
            } else {
                Outcome::Error
            },
        );

        match parsed {
            Ok(architecture) => {
                let report = validator.validate(&architecture);
                checked = checked.saturating_add(1);
                recorder.count("casm.documents.checked", 1);
                println!("{}: {}", file.display(), report.summary());

                for diagnostic in &report.diagnostics {
                    println!("  {}", diagnostic.render().replace('\n', "\n  "));
                }

                let code = exit_code_for(&report, strict);
                if code.code() > worst.code() {
                    worst = code;
                }
            }
            Err(error) => {
                // A file that fails to parse is reported and the sweep continues: a
                // single broken file must not hide findings in every other one.
                println!("{}: FAILED TO PARSE\n  {}", file.display(), error.render());
                worst = ExitCode::Failure;
            }
        }
    }

    println!("\nchecked {checked} of {} file(s)", files.len());
    Ok(worst)
}

/// Reformats or converts an architecture file.
pub(crate) fn fmt(file: &Path, format: DocumentFormat, write: bool) -> CommandResult {
    let architecture = parse_file(file)?;
    let rendered = emit_str(&architecture, format.as_parser_format())?;

    if !write {
        print!("{rendered}");
        return Ok(ExitCode::Success);
    }

    let target = target_path(file, format.as_parser_format());
    std::fs::write(&target, &rendered)
        .map_err(|error| CommandError(format!("cannot write '{}': {error}", target.display())))?;

    println!("wrote {}", target.display());
    Ok(ExitCode::Success)
}

/// Chooses the output path when `fmt --write` changes the format.
fn target_path(file: &Path, format: Format) -> PathBuf {
    let extension = match format {
        Format::Yaml => "yaml",
        Format::Json => "json",
        Format::Toml => "toml",
    };
    file.with_extension(extension)
}

/// Exports the architecture as a formal specification.
pub(crate) fn formal(file: &Path, target: FormalTarget, output: Option<&Path>) -> CommandResult {
    let architecture = parse_file(file)?;
    let model = casm_formal::FormalModel::of(&architecture);

    let mut written: Vec<(String, String)> = Vec::new();

    if matches!(target, FormalTarget::Tla | FormalTarget::All) {
        let tla = casm_formal::tla::emit(&model);
        // Names first: each accessor borrows `tla`, and moving a field out would end
        // the borrow before the next one is taken.
        let (specification, config, liveness) = (
            tla.specification_filename(),
            tla.config_filename(),
            tla.liveness_config_filename(),
        );
        written.push((specification, tla.specification));
        written.push((config, tla.config));
        written.push((liveness, tla.liveness_config));
    }

    if matches!(target, FormalTarget::Alloy | FormalTarget::All) {
        let alloy = casm_formal::alloy::emit(&model);
        let filename = alloy.filename();
        written.push((filename, alloy.model));
    }

    let Some(directory) = output else {
        // To standard output, each file introduced by its name so the stream can be
        // split apart again. Writing several files to a pipe otherwise loses which is
        // which.
        for (name, contents) in &written {
            println!("==> {name} <==");
            print!("{contents}");
            println!();
        }
        return Ok(ExitCode::Success);
    };

    std::fs::create_dir_all(directory).map_err(|error| {
        CommandError(format!("cannot create '{}': {error}", directory.display()))
    })?;

    for (name, contents) in &written {
        let path = directory.join(name);
        std::fs::write(&path, contents)
            .map_err(|error| CommandError(format!("cannot write '{}': {error}", path.display())))?;
        println!("wrote {}", path.display());
    }

    if matches!(target, FormalTarget::Tla | FormalTarget::All) {
        let module = casm_formal::tla::module_name(&model.name);
        println!("\n  check safety:   tlc {module}.tla");
        println!("  check liveness: tlc -config {module}Liveness.cfg {module}.tla");
    }
    if matches!(target, FormalTarget::Alloy | FormalTarget::All) {
        println!(
            "  check structure: alloy exec {}",
            casm_formal::alloy::module_name(&model.name)
        );
    }

    Ok(ExitCode::Success)
}

/// Manages the Git pre-commit hook.
pub(crate) fn manage_hook(action: &HookAction) -> CommandResult {
    let repository = casm_git::Repository::discover(Path::new("."))
        .map_err(|error| CommandError(error.to_string()))?;
    let git_dir = repository.root().join(".git");

    match action {
        HookAction::Install { force } => {
            let previous = hook::install(&git_dir, *force).map_err(CommandError)?;
            match previous {
                hook::Status::Installed => println!("hook already installed; refreshed it"),
                hook::Status::Foreign => println!("replaced an existing hook (--force)"),
                hook::Status::Absent => {
                    println!("installed {}", hook::hook_path(&git_dir).display());
                }
            }
            println!("  architectures are now validated before each commit");
        }
        HookAction::Uninstall => match hook::uninstall(&git_dir).map_err(CommandError)? {
            hook::Status::Installed => println!("removed {}", hook::hook_path(&git_dir).display()),
            hook::Status::Absent | hook::Status::Foreign => println!("nothing to remove"),
        },
        HookAction::Status => match hook::status(&git_dir) {
            hook::Status::Installed => {
                println!("installed at {}", hook::hook_path(&git_dir).display());
            }
            hook::Status::Absent => println!("not installed; run `casm hook install`"),
            hook::Status::Foreign => println!(
                "a pre-commit hook exists at {}, but CASIMIR did not write it",
                hook::hook_path(&git_dir).display()
            ),
        },
    }

    Ok(ExitCode::Success)
}

/// Compares a declared architecture against an observed inventory.
pub(crate) fn drift(
    file: &Path,
    inventory_path: &Path,
    from: InventorySource,
    format: OutputFormat,
    fail_on_drift: bool,
) -> CommandResult {
    let architecture = parse_file(file)?;

    let raw = std::fs::read_to_string(inventory_path).map_err(|error| {
        CommandError(format!(
            "cannot read '{}': {error}",
            inventory_path.display()
        ))
    })?;

    let inventory = match from {
        InventorySource::Native => casm_diff::Inventory::from_json(&raw),
        InventorySource::Terraform => casm_diff::Inventory::from_terraform_state(&raw),
    }
    .map_err(CommandError)?;

    let report = casm_diff::drift::detect(&architecture, &inventory);

    match format {
        OutputFormat::Json | OutputFormat::Sarif => {
            let json = serde_json::to_string_pretty(&report)
                .map_err(|error| CommandError(format!("cannot serialise report: {error}")))?;
            println!("{json}");
        }
        OutputFormat::Human => {
            println!(
                "{}: {} node(s) declared",
                file.display(),
                architecture.node_count()
            );
            if !report.is_clean() {
                println!();
                print!("{}", report.render());
            }
            println!("\n{}", report.summary());
        }
    }

    if fail_on_drift && !report.is_clean() {
        return Ok(ExitCode::ValidationErrors);
    }
    Ok(ExitCode::Success)
}

/// Loads a pattern library, or none if the caller supplied no directory.
///
/// A missing directory is an error rather than an empty library: an author who typed
/// `--patterns ptterns` should be told, not handed a silent all-clear.
fn load_patterns(directory: Option<&Path>) -> Result<Vec<Pattern>, CommandError> {
    let Some(directory) = directory else {
        return Ok(Vec::new());
    };

    let library = Library::load(directory).map_err(|error| CommandError(error.render()))?;
    Ok(library.patterns().cloned().collect())
}

/// Reports what an architecture must change to conform to a pattern.
///
/// Reports rather than rewrites. ADR-0012: a pattern is a shape, so migrating to a new
/// version is "here is what you do not yet satisfy" — a computation over two sets — and
/// not a three-way merge that could silently corrupt the file.
pub(crate) fn evolve(
    file: &Path,
    patterns: &Path,
    to: Option<&str>,
    strict: bool,
) -> CommandResult {
    let architecture = parse_file(file)?;
    let library = Library::load(patterns).map_err(|error| CommandError(error.render()))?;

    let claims = evolve_targets(&architecture, to)?;
    if claims.is_empty() {
        println!(
            "{}: claims no patterns, and none was requested with --to",
            file.display()
        );
        return Ok(ExitCode::Success);
    }

    println!("{}: {} pattern(s)\n", file.display(), claims.len());

    let mut unmet_total = 0_usize;
    for claim in &claims {
        let reference = claim.pattern().to_string();

        let Some(pattern) = library.get(claim.pattern()) else {
            println!("{reference}: not found in '{}'", patterns.display());
            if let Some(hint) = library.closest(claim.pattern()) {
                println!("  help: {hint}");
            }
            unmet_total = unmet_total.saturating_add(1);
            continue;
        };

        let report = conformance::check(&architecture, pattern, claim);
        unmet_total = unmet_total.saturating_add(report.unmet().len());
        print_conformance(&reference, &report, &architecture);
    }

    if unmet_total == 0 {
        println!("conforms to every pattern claimed");
        return Ok(ExitCode::Success);
    }

    println!("{unmet_total} requirement(s) unmet");
    if strict {
        return Ok(ExitCode::ValidationErrors);
    }
    Ok(ExitCode::Success)
}

/// Decides which claims `casm evolve` should report on.
///
/// With `--to`, exactly that pattern, whether or not the architecture already claims it —
/// that is the migration case. Without it, everything already claimed.
fn evolve_targets(
    architecture: &Architecture,
    to: Option<&str>,
) -> Result<Vec<casm_core::Conformance>, CommandError> {
    let Some(wanted) = to else {
        return Ok(architecture.conformance().cloned().collect());
    };

    let reference = casm_core::PatternRef::parse(wanted)
        .map_err(|error| CommandError(format!("--to: {error}")))?;

    // An existing claim carries the author's bindings, and discarding them would report
    // back an ambiguity they have already resolved. Any version of the same pattern will
    // do: migrating from 1.0.0 to 2.0.0 is precisely the case where the bindings written
    // for the old version are the ones worth reusing. Roles the new version does not have
    // are reported rather than dropped, which is the honest outcome — a binding that no
    // longer means anything is a thing the author should be told about.
    let inherited = architecture
        .conformance()
        .find(|claim| claim.pattern() == &reference)
        .or_else(|| {
            architecture
                .conformance()
                .find(|claim| claim.pattern().name() == reference.name())
        });

    let Some(existing) = inherited else {
        return Ok(vec![casm_core::Conformance::new(reference)]);
    };

    let mut claim = casm_core::Conformance::new(reference);
    for (role, node) in existing.bindings() {
        claim = claim
            .binding(role.as_str(), *node)
            .map_err(|error| CommandError(format!("--to: {error}")))?;
    }
    Ok(vec![claim])
}

/// Prints one conformance report.
fn print_conformance(
    reference: &str,
    report: &casm_core::ConformanceReport,
    architecture: &Architecture,
) {
    if report.conforms() {
        println!("{reference}: conforms");
        return;
    }

    println!("{reference}: {} unmet", report.unmet().len());

    for (role, id) in report.bindings() {
        let node = architecture
            .node(*id)
            .map_or_else(|| id.to_string(), |node| node.name().as_str().to_owned());
        println!("  {role} -> {node}");
    }

    // Mechanical failures first: they are the ones an author can act on immediately.
    let (mechanical, decisions): (Vec<_>, Vec<_>) = report
        .unmet()
        .iter()
        .partition(|unmet| unmet.is_mechanical());

    for unmet in mechanical {
        println!("  add: {}", unmet.message());
    }
    for unmet in decisions {
        println!("  decide: {}", unmet.message());
    }
    println!();
}

/// Opens the repository containing `file`, reporting failures in the user's terms.
fn repository_for(file: &Path) -> Result<casm_git::Repository, CommandError> {
    let search_from = file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    casm_git::Repository::discover(search_from.unwrap_or(Path::new(".")))
        .map_err(|error| CommandError(error.to_string()))
}

/// Assembles a register of the control claims an architecture makes.
///
/// Reports claims, never satisfaction. A control flagged `evidence-required` is listed as
/// outstanding, because CASIMIR does not hold the artefact behind it and saying otherwise
/// would launder an assertion into evidence — see ADR-0013.
pub(crate) fn evidence(
    file: &Path,
    format: EvidenceFormat,
    patterns: Option<&Path>,
    no_history: bool,
    strict: bool,
) -> CommandResult {
    let architecture = parse_file(file)?;
    let patterns = load_patterns(patterns)?;

    let provenance = if no_history {
        Provenance::unknown().from_source(file.display().to_string())
    } else {
        provenance_of(file)
    };

    let pack = Pack::assemble(&architecture, &patterns, provenance);

    match format {
        EvidenceFormat::Human => print!(
            "{}",
            casm_evidence::render::render(&pack, RenderFormat::Human)
        ),
        EvidenceFormat::Markdown => {
            print!(
                "{}",
                casm_evidence::render::render(&pack, RenderFormat::Markdown)
            );
        }
        EvidenceFormat::Json => {
            let json = serde_json::to_string_pretty(&pack)
                .map_err(|error| CommandError(format!("cannot serialise the register: {error}")))?;
            println!("{json}");
        }
    }

    if strict && pack.outstanding() > 0 {
        return Ok(ExitCode::ValidationErrors);
    }
    Ok(ExitCode::Success)
}

/// Reads whatever Git can tell us about where an architecture came from.
///
/// A file outside a repository, or one Git cannot read, yields
/// [`Provenance::unknown`] rather than an error: a register without attribution is still
/// worth having, and refusing to produce one would be the wrong trade.
fn provenance_of(file: &Path) -> Provenance {
    let base = Provenance::unknown().from_source(file.display().to_string());

    let Ok(repository) = repository_for(file) else {
        return base;
    };
    let Ok(revisions) = repository.semantic_history(file, casm_git::HistoryOptions::default())
    else {
        return base;
    };

    let base = base.with_semantic_revisions(revisions.len());
    let latest = revisions.first().map(attribution_of);
    let first = revisions.last().map(attribution_of);

    match (latest, first) {
        (Some(latest), Some(first)) => base.at(latest).introduced_by(first),
        (Some(latest), None) => base.at(latest),
        (None, _) => base,
    }
}

/// Projects a revision onto the attribution an evidence register carries.
fn attribution_of(revision: &casm_git::Revision) -> Attribution {
    Attribution {
        commit: revision.commit.clone(),
        author: revision.author.clone(),
        email: revision.email.clone(),
        date: revision.dated().to_date(),
        summary: revision.summary.clone(),
    }
}

/// Shows the commits at which the architecture's meaning changed.
pub(crate) fn log(file: &Path, limit: usize, format: OutputFormat) -> CommandResult {
    let repository = repository_for(file)?;
    let options = casm_git::HistoryOptions::default().with_max_revisions(limit);

    let revisions = repository
        .semantic_history(file, options)
        .map_err(|error| CommandError(error.to_string()))?;

    render_revisions(file, &revisions, format, None)
}

/// Shows the commits at which one node's meaning changed.
pub(crate) fn blame(file: &Path, node: &str, limit: usize, format: OutputFormat) -> CommandResult {
    let repository = repository_for(file)?;
    let options = casm_git::HistoryOptions::default().with_max_revisions(limit);

    let revisions = repository
        .blame_node(file, node, options)
        .map_err(|error| CommandError(error.to_string()))?;

    render_revisions(file, &revisions, format, Some(node))
}

/// Renders a list of revisions in the requested format.
fn render_revisions(
    file: &Path,
    revisions: &[casm_git::Revision],
    format: OutputFormat,
    node: Option<&str>,
) -> CommandResult {
    match format {
        OutputFormat::Json | OutputFormat::Sarif => {
            // SARIF describes findings, not history, so JSON is the only machine format
            // that means anything here.
            let json = serde_json::to_string_pretty(revisions)
                .map_err(|error| CommandError(format!("cannot serialise history: {error}")))?;
            println!("{json}");
        }
        OutputFormat::Human => print_revisions(file, revisions, node),
    }

    Ok(ExitCode::Success)
}

/// Prints revisions as prose.
fn print_revisions(file: &Path, revisions: &[casm_git::Revision], node: Option<&str>) {
    let subject = node.map_or_else(
        || file.display().to_string(),
        |name| format!("node '{name}' in {}", file.display()),
    );

    if revisions.is_empty() {
        println!("no semantic changes recorded for {subject}");
        return;
    }

    println!("{subject}\n");
    for revision in revisions {
        println!(
            "{}  {}  {}",
            revision.short_commit(),
            revision.dated().to_date(),
            revision.summary
        );
        println!("    {} <{}>", revision.author, revision.email);
        println!("    fingerprint {}", revision.fingerprint.abbreviated(12));

        if revision.introduced {
            println!("    introduced here");
        } else if !revision.changed_nodes.is_empty() {
            println!("    nodes: {}", revision.changed_nodes.join(", "));
        }
        println!();
    }

    println!("{} semantic change(s)", revisions.len());
}

/// Prints an architecture as it was at a past revision.
pub(crate) fn checkout(revision: &str, file: &Path, validate_it: bool) -> CommandResult {
    let repository = repository_for(file)?;
    let source = repository
        .read_at(revision, file)
        .map_err(|error| CommandError(error.to_string()))?;

    if !validate_it {
        print!("{source}");
        return Ok(ExitCode::Success);
    }

    let architecture = casm_parser::parse_str(&source, file)?;
    let report = Validator::new().validate(&architecture);
    print_human(file, &architecture, &report);
    Ok(exit_code_for(&report, false))
}

/// Lists the built-in validation rules.
pub(crate) fn rules(json: bool) -> CommandResult {
    let catalogue = casm_validator::rules::built_in();

    if json {
        let entries: Vec<serde_json::Value> = catalogue
            .iter()
            .map(|rule| serde_json::json!({ "id": rule.id(), "description": rule.description() }))
            .collect();
        let text = serde_json::to_string_pretty(&entries)
            .map_err(|error| CommandError(format!("cannot serialise rules: {error}")))?;
        println!("{text}");
        return Ok(ExitCode::Success);
    }

    println!("{} built-in rule(s):\n", catalogue.len());
    for rule in &catalogue {
        println!("  {}\n      {}", rule.id(), rule.description());
    }

    Ok(ExitCode::Success)
}

/// Finds every plausible architecture file under `directory`.
///
/// The walk is depth-bounded and skips `target/`, `node_modules/`, and dot-directories,
/// because sweeping a build directory finds nothing and takes forever.
fn discover(directory: &Path) -> Result<Vec<PathBuf>, CommandError> {
    let mut found = Vec::new();
    walk(directory, 0, &mut found)?;
    found.sort();
    Ok(found)
}

/// The maximum directory depth `check` will descend.
///
/// NASA Rule 4: the recursion needs a statically provable bound. Eight levels reaches
/// any sanely-organised repository and cannot be driven into a symlink loop.
const MAX_WALK_DEPTH: usize = 8;

/// Recursively collects architecture files, bounded by [`MAX_WALK_DEPTH`].
fn walk(directory: &Path, depth: usize, found: &mut Vec<PathBuf>) -> Result<(), CommandError> {
    if depth > MAX_WALK_DEPTH {
        return Ok(());
    }

    let entries = std::fs::read_dir(directory)
        .map_err(|error| CommandError(format!("cannot read '{}': {error}", directory.display())))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }

        if path.is_dir() {
            walk(&path, depth.saturating_add(1), found)?;
        } else if Format::from_path(&path).is_some() && is_architecture_file(&path) {
            found.push(path);
        }
    }

    Ok(())
}

/// Returns `true` if `path` looks like a CASIMIR architecture rather than any old YAML.
///
/// Checked by content, not by filename: a repository is full of YAML that is not an
/// architecture, and `check` reporting parse failures for every CI workflow file would
/// make it useless.
fn is_architecture_file(path: &Path) -> bool {
    let Ok(source) = std::fs::read_to_string(path) else {
        return false;
    };
    source.contains("nodes") && (source.contains("relationships") || source.contains("name"))
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
    use casm_parser::parse_str;

    /// Creates a uniquely-named temporary directory for a test.
    fn temp_dir(label: &str) -> PathBuf {
        let unique = casm_core::NodeId::new();
        let dir = std::env::temp_dir().join(format!("casm-cli-{label}-{unique}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn the_scaffold_template_parses_and_validates() {
        // The template ships to every new user; if it is invalid, nothing else matters.
        let source = TEMPLATE.replace("{{NAME}}", "test-arch");
        let architecture =
            parse_str(&source, Path::new("architecture.yaml")).expect("template must parse");

        assert_eq!(architecture.name().as_str(), "test-arch");
        assert_eq!(architecture.node_count(), 3);
        assert_eq!(architecture.relationship_count(), 2);

        let report = Validator::new().validate(&architecture);
        assert!(
            !report.has_errors(),
            "template must not produce errors:\n{}",
            report.render()
        );
    }

    #[test]
    fn init_writes_a_file_and_refuses_to_clobber_it() {
        let dir = temp_dir("init");
        let target = dir.join("architecture.yaml");

        assert!(init("demo", &target, false).is_ok());
        assert!(target.exists());

        let refused = init("demo", &target, false);
        assert!(refused.is_err(), "init must not overwrite without --force");

        assert!(
            init("demo", &target, true).is_ok(),
            "--force must overwrite"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn strict_mode_promotes_warnings_to_a_failing_exit_code() {
        let mut report = Report::new();
        report.push(casm_validator::Diagnostic::new(
            "r",
            casm_validator::Severity::Warning,
            casm_validator::Subject::Architecture,
            "a warning",
        ));

        assert_eq!(exit_code_for(&report, false), ExitCode::Warnings);
        assert_eq!(exit_code_for(&report, true), ExitCode::ValidationErrors);
    }

    #[test]
    fn strict_mode_does_not_promote_a_clean_report() {
        assert_eq!(exit_code_for(&Report::new(), true), ExitCode::Success);
    }

    #[test]
    fn command_line_overrides_reach_the_validator_config() {
        let config = validator_config(&["no-isolated-nodes".to_owned()], Some(250), Some(1));
        assert_eq!(config.max_critical_path_ms, 250);
        assert_eq!(config.min_security_controls_per_service, 1);
        assert!(config.is_allowed("no-isolated-nodes"));
    }

    #[test]
    fn omitted_overrides_leave_the_defaults_alone() {
        let config = validator_config(&[], None, None);
        assert_eq!(config, ValidatorConfig::default());
    }

    #[test]
    fn fmt_target_path_follows_the_chosen_format() {
        let file = Path::new("systems/checkout.yaml");
        assert_eq!(
            target_path(file, Format::Json),
            PathBuf::from("systems/checkout.json")
        );
        assert_eq!(
            target_path(file, Format::Toml),
            PathBuf::from("systems/checkout.toml")
        );
        assert_eq!(
            target_path(file, Format::Yaml),
            PathBuf::from("systems/checkout.yaml")
        );
    }

    #[test]
    fn discovery_finds_architecture_files_and_ignores_unrelated_yaml() {
        let dir = temp_dir("discover");
        std::fs::write(
            dir.join("architecture.yaml"),
            TEMPLATE.replace("{{NAME}}", "a"),
        )
        .unwrap();
        std::fs::write(
            dir.join("ci.yaml"),
            "on: push\njobs:\n  build:\n    steps: []\n",
        )
        .unwrap();

        let found = discover(&dir).unwrap();
        assert_eq!(found.len(), 1, "found {found:?}");
        assert!(found[0].ends_with("architecture.yaml"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discovery_skips_build_and_dot_directories() {
        let dir = temp_dir("skip");
        for ignored in ["target", "node_modules", ".git"] {
            let nested = dir.join(ignored);
            std::fs::create_dir_all(&nested).unwrap();
            std::fs::write(
                nested.join("architecture.yaml"),
                TEMPLATE.replace("{{NAME}}", "x"),
            )
            .unwrap();
        }

        assert!(discover(&dir).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discovery_descends_into_subdirectories() {
        let dir = temp_dir("nested");
        let nested = dir.join("systems").join("payments");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            nested.join("architecture.yaml"),
            TEMPLATE.replace("{{NAME}}", "p"),
        )
        .unwrap();

        assert_eq!(discover(&dir).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discovery_output_is_sorted_for_determinism() {
        let dir = temp_dir("sorted");
        for name in ["zeta.yaml", "alpha.yaml", "mu.yaml"] {
            std::fs::write(dir.join(name), TEMPLATE.replace("{{NAME}}", "x")).unwrap();
        }

        let found = discover(&dir).unwrap();
        let mut sorted = found.clone();
        sorted.sort();
        assert_eq!(found, sorted);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discovery_reports_an_unreadable_directory() {
        assert!(discover(Path::new("/nonexistent/casm/path")).is_err());
    }

    #[test]
    fn check_returns_success_when_nothing_is_found() {
        let dir = temp_dir("empty");
        let mut recorder = Recorder::new(casm_telemetry::Resource::new("casm", "test"));
        assert_eq!(
            check(&dir, false, None, &mut recorder).map_or(-1, ExitCode::code),
            0
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn check_reports_a_failure_for_an_unparseable_file() {
        let dir = temp_dir("broken");
        std::fs::write(
            dir.join("architecture.yaml"),
            "name: x\nnodes: [not-a-node]\n",
        )
        .unwrap();

        let mut recorder = Recorder::new(casm_telemetry::Resource::new("casm", "test"));
        let code = check(&dir, false, None, &mut recorder).map_or(-1, ExitCode::code);
        assert_eq!(code, ExitCode::Failure.code());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn generate_writes_a_diagram_to_disk() {
        let dir = temp_dir("generate");
        let source = dir.join("architecture.yaml");
        let target = dir.join("diagram.mmd");
        std::fs::write(&source, TEMPLATE.replace("{{NAME}}", "demo")).unwrap();

        assert!(generate(&source, DiagramFormat::Mermaid, Some(&target)).is_ok());
        let diagram = std::fs::read_to_string(&target).unwrap();
        assert!(diagram.starts_with("flowchart LR"), "{diagram}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fmt_converts_between_formats_on_disk() {
        let dir = temp_dir("fmt");
        let source = dir.join("architecture.yaml");
        std::fs::write(&source, TEMPLATE.replace("{{NAME}}", "demo")).unwrap();

        assert!(fmt(&source, DocumentFormat::Json, true).is_ok());
        let json = dir.join("architecture.json");
        assert!(json.exists());

        // The converted file must still be a loadable architecture.
        let reparsed = parse_file(&json).unwrap();
        assert_eq!(reparsed.node_count(), 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_reports_a_missing_file_as_a_command_error() {
        let result = validate(
            Path::new("/nonexistent/architecture.yaml"),
            OutputFormat::Human,
            false,
            &[],
            None,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn rules_listing_succeeds_in_both_formats() {
        assert!(rules(false).is_ok());
        assert!(rules(true).is_ok());
    }

    /// A pattern the scaffold template satisfies: one gateway, one service, one sync hop.
    const SCAFFOLD_PATTERN: &str = r"
name: scaffold-shape
version: 1.0.0
requires:
  - role: edge
    type: gateway
    min-security-controls: 2
  - role: application
    type: service
relationships:
  - source: edge
    target: application
    type: sync
";

    /// Writes an architecture and a pattern library into one temporary directory.
    fn evolve_fixture(label: &str, claim: bool) -> (PathBuf, PathBuf, PathBuf) {
        let dir = temp_dir(label);
        let patterns = dir.join("patterns");
        std::fs::create_dir_all(&patterns).unwrap();
        std::fs::write(patterns.join("scaffold-shape.yaml"), SCAFFOLD_PATTERN).unwrap();

        let mut source = TEMPLATE.replace("{{NAME}}", "demo");
        if claim {
            source.push_str("\npatterns:\n  - pattern: scaffold-shape@1.0.0\n");
        }
        let file = dir.join("architecture.yaml");
        std::fs::write(&file, source).unwrap();

        (dir, file, patterns)
    }

    #[test]
    fn evolve_reports_conformance_for_a_claim_the_architecture_satisfies() {
        let (dir, file, patterns) = evolve_fixture("evolve-ok", true);
        let code = evolve(&file, &patterns, None, true).map_or(-1, ExitCode::code);
        assert_eq!(code, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn evolve_with_no_claims_and_no_target_succeeds_quietly() {
        let (dir, file, patterns) = evolve_fixture("evolve-none", false);
        let code = evolve(&file, &patterns, None, true).map_or(-1, ExitCode::code);
        assert_eq!(code, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn evolve_checks_a_pattern_the_architecture_does_not_yet_claim() {
        // The migration case: "what would it take to conform to this?"
        let (dir, file, patterns) = evolve_fixture("evolve-to", false);
        let code =
            evolve(&file, &patterns, Some("scaffold-shape@1.0.0"), true).map_or(-1, ExitCode::code);
        assert_eq!(code, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn evolve_fails_under_strict_when_a_requirement_is_unmet() {
        let (dir, file, patterns) = evolve_fixture("evolve-unmet", false);
        std::fs::write(
            patterns.join("scaffold-shape.yaml"),
            // The scaffold template has no queue, so this cannot be satisfied.
            "name: scaffold-shape\nversion: 1.0.0\nrequires:\n  - role: bus\n    type: queue\n",
        )
        .unwrap();

        let code =
            evolve(&file, &patterns, Some("scaffold-shape@1.0.0"), true).map_or(-1, ExitCode::code);
        assert_eq!(code, ExitCode::ValidationErrors.code());

        // The same run without --strict reports the same thing and exits zero.
        let lenient = evolve(&file, &patterns, Some("scaffold-shape@1.0.0"), false)
            .map_or(-1, ExitCode::code);
        assert_eq!(lenient, 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn evolve_reports_a_pattern_the_library_does_not_hold() {
        let (dir, file, patterns) = evolve_fixture("evolve-missing", false);
        let code =
            evolve(&file, &patterns, Some("nonexistent@1.0.0"), true).map_or(-1, ExitCode::code);
        assert_eq!(code, ExitCode::ValidationErrors.code());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn evolve_rejects_a_malformed_pattern_reference() {
        let (dir, file, patterns) = evolve_fixture("evolve-malformed", false);
        assert!(evolve(&file, &patterns, Some("no-version"), false).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_pattern_directory_is_an_error_rather_than_an_empty_library() {
        // An author who typed `--patterns ptterns` must be told, not handed all-clear.
        assert!(load_patterns(Some(Path::new("/nonexistent/patterns"))).is_err());
        assert!(load_patterns(None).unwrap().is_empty());
    }

    #[test]
    fn validate_checks_conformance_when_patterns_are_supplied() {
        let (dir, file, patterns) = evolve_fixture("validate-patterns", true);
        let code = validate(
            &file,
            OutputFormat::Human,
            false,
            &[],
            None,
            None,
            Some(&patterns),
        )
        .map_or(-1, ExitCode::code);

        // The scaffold template warns about other things by design; what matters is that
        // the conformance claim itself did not add an error.
        assert_ne!(code, ExitCode::Failure.code());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn evolve_to_a_new_version_reuses_the_bindings_written_for_the_old_one() {
        // The migration case: an ambiguity resolved for 1.0.0 should not be reported back
        // when moving to 2.0.0.
        let dir = temp_dir("evolve-migrate");
        let patterns = dir.join("patterns");
        std::fs::create_dir_all(&patterns).unwrap();
        std::fs::write(
            patterns.join("v2.yaml"),
            "name: shape\nversion: 2.0.0\nrequires:\n  - role: application\n    type: service\n",
        )
        .unwrap();

        // Two services, so `application` is ambiguous unless something says which.
        let two_services = "\
name: demo
nodes:
  - name: orders
    type: service
  - name: payments
    type: service
";
        let file = dir.join("architecture.yaml");
        std::fs::write(&file, two_services).unwrap();

        let unbound =
            evolve(&file, &patterns, Some("shape@2.0.0"), true).map_or(-1, ExitCode::code);
        assert_eq!(
            unbound,
            ExitCode::ValidationErrors.code(),
            "two services must be ambiguous without a binding"
        );

        // Claim 1.0.0 with a binding, then migrate to 2.0.0.
        let claiming = format!(
            "{two_services}patterns:\n  - pattern: shape@1.0.0\n    bind:\n      application: orders\n"
        );
        std::fs::write(&file, claiming).unwrap();

        let bound = evolve(&file, &patterns, Some("shape@2.0.0"), true).map_or(-1, ExitCode::code);
        assert_eq!(bound, 0, "the 1.0.0 binding should carry over to 2.0.0");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// An architecture with one flagged and one unflagged control.
    const CLAIMING: &str = "\
name: audited
version: 1.0.0
nodes:
  - name: orders-db
    type: database
    controls:
      - type: compliance
        standard: ISO27001-A.10.1
        description: Encrypted at rest
        evidence-required: true
      - type: operational
        standard: PITR-35D
        description: Point-in-time recovery for 35 days
";

    fn write_architecture(label: &str, source: &str) -> (PathBuf, PathBuf) {
        let dir = temp_dir(label);
        let file = dir.join("architecture.yaml");
        std::fs::write(&file, source).expect("write the fixture");
        (dir, file)
    }

    #[test]
    fn the_evidence_register_lists_claims_and_counts_what_is_outstanding() {
        let (dir, file) = write_architecture("evidence", CLAIMING);

        let code = evidence(&file, EvidenceFormat::Human, None, true, false)
            .expect("assembling a register cannot fail");

        assert_eq!(code, ExitCode::Success);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn strict_evidence_fails_when_a_claim_is_outstanding() {
        // The CI use: a control that says an artefact exists, which nobody has produced,
        // should be able to fail a pipeline.
        let (dir, file) = write_architecture("evidence-strict", CLAIMING);

        let code = evidence(&file, EvidenceFormat::Json, None, true, true)
            .expect("assembling a register cannot fail");

        assert_eq!(code, ExitCode::ValidationErrors);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn strict_evidence_passes_when_nothing_is_flagged() {
        let quiet = CLAIMING.replace("        evidence-required: true\n", "");
        let (dir, file) = write_architecture("evidence-quiet", &quiet);

        let code = evidence(&file, EvidenceFormat::Human, None, true, true)
            .expect("assembling a register cannot fail");

        assert_eq!(code, ExitCode::Success);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_architecture_outside_a_repository_still_produces_a_register() {
        // Provenance is unavailable, which is a fact to report rather than a failure.
        let (dir, file) = write_architecture("evidence-nogit", CLAIMING);

        let provenance = provenance_of(&file);
        assert_eq!(
            provenance.source.as_deref(),
            Some(file.display().to_string().as_str())
        );

        let code = evidence(&file, EvidenceFormat::Markdown, None, false, false)
            .expect("a missing repository is not an error");
        assert_eq!(code, ExitCode::Success);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn no_history_skips_git_entirely() {
        let (dir, file) = write_architecture("evidence-nohistory", CLAIMING);

        let code = evidence(&file, EvidenceFormat::Json, None, true, false)
            .expect("assembling a register cannot fail");

        assert_eq!(code, ExitCode::Success);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_unreadable_architecture_fails_rather_than_producing_an_empty_register() {
        // A register assembled from nothing would be the most dangerous possible output.
        let (dir, file) = write_architecture(
            "evidence-broken",
            "name: x\nnodes:\n  - name: a\n    type: srvice\n",
        );

        assert!(evidence(&file, EvidenceFormat::Human, None, true, false).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_check_sweep_records_a_span_for_every_file() {
        let dir = temp_dir("check-telemetry");
        for name in ["a.yaml", "b.yaml"] {
            std::fs::write(dir.join(name), CLAIMING).expect("write the fixture");
        }

        let mut recorder = Recorder::new(casm_telemetry::Resource::new("casm", "test"));
        let code = check(&dir, false, None, &mut recorder).expect("the sweep runs");

        assert_eq!(code, ExitCode::Success);
        assert_eq!(recorder.spans().len(), 2, "one span per file");
        assert!(
            recorder
                .spans()
                .iter()
                .all(|span| span.name == "check-file")
        );
        assert_eq!(
            recorder
                .metrics()
                .iter()
                .find(|m| m.name == "casm.documents.checked")
                .map(|m| m.sum),
            Some(2.0)
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_file_that_does_not_parse_is_an_error_outcome_on_its_span() {
        let dir = temp_dir("check-telemetry-broken");
        std::fs::write(
            dir.join("broken.yaml"),
            "name: x\nnodes:\n  - name: a\n    type: srvice\n",
        )
        .expect("write the fixture");

        let mut recorder = Recorder::new(casm_telemetry::Resource::new("casm", "test"));
        let _ = check(&dir, false, None, &mut recorder);

        assert_eq!(recorder.spans().len(), 1);
        assert_eq!(recorder.spans()[0].outcome, Outcome::Error);
        std::fs::remove_dir_all(dir).ok();
    }
}
