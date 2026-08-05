# Commands

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Clean |
| `1` | Warnings |
| `2` | Validation errors |
| `3` | The command itself failed |

`2` and `3` are deliberately distinct: "your architecture is wrong" and "the tool could
not run" need different responses from a pipeline.

## `casm init`

Scaffold a new architecture.

`--name <name>` · `--output <path>` · `--force`

## `casm validate [file]`

Run the rule library.

`--format human|json|sarif` · `--strict` · `--allow <rule>` (repeatable) ·
`--max-critical-path-ms <ms>` · `--min-security-controls <n>` · `--patterns <dir>`

## `casm generate [file]`

Render a diagram.

`--format mermaid|dot|ascii` · `--output <path>`

## `casm diff <old> <new>`

Semantic diff. Reordering and reformatting produce no output.

`--fail-on-breaking`

## `casm check [directory]`

Validate every architecture file found, identified by content rather than filename.

`--strict` · `--patterns <dir>`

## `casm evidence [file]`

Assemble a register of the control claims the architecture makes, grouped by standard, with
provenance from Git.

Reports claims, never satisfaction: a control flagged `evidence-required` is listed as
outstanding. See [ADR-0013](decisions.md).

`--format human|markdown|json` · `--patterns <dir>` · `--no-history` · `--strict`

## `casm evolve [file]`

Report what the architecture must change to conform to a pattern. Reports; does not
rewrite.

`--patterns <dir>` (default `patterns`) · `--to <name@version>` · `--strict`

## `casm fmt [file]`

Reformat, or convert between formats.

`--format yaml|json|toml` · `--write`

## `casm log [file]`

Commits where the architecture's meaning changed.

`--limit <n>` · `--format human|json`

## `casm blame <node> [file]`

Commits where one node's meaning changed.

`--limit <n>` · `--format human|json`

## `casm checkout <revision> [file]`

Print an architecture as it was. Writes to standard output; the working tree is never
touched.

`--validate`

## `casm drift [file]`

Compare against real infrastructure.

`--inventory <path>` (required) · `--from native|terraform` · `--format human|json` ·
`--fail-on-drift`

## `casm formal [file]`

Export a formal specification.

`--target tla|alloy|all` · `--output <dir>`

## `casm hook <install|uninstall|status>`

Manage the pre-commit hook. `install` takes `--force`.

## `casm rules`

List the built-in rules. `--json`
