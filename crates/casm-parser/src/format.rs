//! Module: `casm_parser::format`
//! Purpose: Detecting which concrete syntax a CASIMIR document is written in.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Detection order
//!
//! Extension first, contents second. The extension is the author's explicit statement of
//! intent, and honouring it means a malformed `.json` file reports a *JSON* syntax error
//! rather than a confusing YAML one — YAML is a superset of JSON, so sniffing first would
//! silently reinterpret the file and blame the wrong grammar.

use std::path::Path;

/// A concrete syntax a CASIMIR architecture can be written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Format {
    /// YAML 1.2 — the format intended for humans.
    Yaml,
    /// JSON — the format intended for machines and API payloads.
    Json,
    /// TOML — the format intended for configuration.
    Toml,
}

impl Format {
    /// Every format CASIMIR can read, in preference order.
    pub const ALL: [Self; 3] = [Self::Yaml, Self::Json, Self::Toml];

    /// Returns the canonical lowercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Yaml => "yaml",
            Self::Json => "json",
            Self::Toml => "toml",
        }
    }

    /// The file extensions conventionally associated with this format.
    #[must_use]
    pub const fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Yaml => &["yaml", "yml"],
            Self::Json => &["json"],
            Self::Toml => &["toml"],
        }
    }

    /// Classifies a path by its extension, case-insensitively.
    ///
    /// Returns `None` for an unrecognised or absent extension; callers fall back to
    /// [`Format::sniff`].
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|format| format.extensions().contains(&extension.as_str()))
    }

    /// Guesses a format from the document's first meaningful character.
    ///
    /// This is a heuristic of last resort, used only when the path carries no usable
    /// extension (a stdin pipe, for instance). It is deliberately conservative: a
    /// document whose shape is ambiguous between YAML and TOML is called YAML, because
    /// YAML is the authoring default and produces the more familiar error messages.
    #[must_use]
    pub fn sniff(source: &str) -> Self {
        let Some(first) = Self::first_meaningful_line(source) else {
            return Self::Yaml;
        };

        if first.starts_with('{') || first.starts_with('[') && first.ends_with(',') {
            return Self::Json;
        }

        // A TOML table header (`[section]`) is unambiguous; a YAML sequence entry
        // (`- item`) and a mapping (`key: value`) are not TOML at all.
        if first.starts_with('[') && first.ends_with(']') {
            return Self::Toml;
        }

        // `key = value` with no colon before the equals is TOML's assignment syntax.
        if let Some(equals) = first.find('=') {
            let before_equals = first.get(..equals).unwrap_or_default();
            if !before_equals.contains(':') && !before_equals.trim().is_empty() {
                return Self::Toml;
            }
        }

        Self::Yaml
    }

    /// Returns the first line that is neither blank nor a `#` comment.
    fn first_meaningful_line(source: &str) -> Option<&str> {
        source
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("---"))
    }

    /// Resolves the format for a path, falling back to sniffing its contents.
    #[must_use]
    pub fn resolve(path: &Path, source: &str) -> Self {
        Self::from_path(path).unwrap_or_else(|| Self::sniff(source))
    }
}

impl core::fmt::Display for Format {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.label())
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
    use std::path::PathBuf;

    #[test]
    fn extensions_are_recognised() {
        let cases = [
            ("a.yaml", Format::Yaml),
            ("a.yml", Format::Yaml),
            ("a.json", Format::Json),
            ("a.toml", Format::Toml),
        ];
        for (path, expected) in cases {
            assert_eq!(
                Format::from_path(&PathBuf::from(path)),
                Some(expected),
                "{path}"
            );
        }
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        assert_eq!(
            Format::from_path(&PathBuf::from("A.YAML")),
            Some(Format::Yaml)
        );
        assert_eq!(
            Format::from_path(&PathBuf::from("A.Json")),
            Some(Format::Json)
        );
    }

    #[test]
    fn unknown_and_absent_extensions_are_unclassified() {
        assert_eq!(Format::from_path(&PathBuf::from("a.txt")), None);
        assert_eq!(Format::from_path(&PathBuf::from("architecture")), None);
    }

    #[test]
    fn sniff_detects_json_by_its_opening_brace() {
        assert_eq!(Format::sniff("{\"name\": \"x\"}"), Format::Json);
        assert_eq!(Format::sniff("\n\n  {\"name\": \"x\"}"), Format::Json);
    }

    #[test]
    fn every_format_renders_its_own_label() {
        // `label` feeds `Display`, which reaches error messages and `casm fmt --format`.
        // Both could have returned a constant undetected.
        assert_eq!(Format::Yaml.label(), "yaml");
        assert_eq!(Format::Json.label(), "json");
        assert_eq!(Format::Toml.label(), "toml");

        assert_eq!(Format::Yaml.to_string(), "yaml");
        assert_eq!(Format::Json.to_string(), "json");
        assert_eq!(Format::Toml.to_string(), "toml");

        let labels: std::collections::BTreeSet<&str> =
            Format::ALL.iter().map(|format| format.label()).collect();
        assert_eq!(labels.len(), Format::ALL.len(), "labels must be distinct");
    }

    #[test]
    fn a_table_header_needs_both_brackets_to_be_toml() {
        // `first.starts_with('[') && first.ends_with(']')` — replacing the `&&` with `||`
        // claims a YAML sequence entry is TOML, because `- [a, b]` ends with a bracket
        // and a flow sequence begins with one.
        assert_eq!(Format::sniff("[package]\nname = \"x\"\n"), Format::Toml);

        // Opening bracket only. `[a, b, c]` would not do: it satisfies *both* halves and
        // is correctly read as a table header, which is how the first version of this
        // test managed to fail against unmutated code.
        assert_ne!(Format::sniff("[unterminated\n"), Format::Toml);

        // Closing bracket only.
        assert_ne!(Format::sniff("nodes: []\n"), Format::Toml);
        assert_ne!(Format::sniff("name: x]\n"), Format::Toml);
    }

    #[test]
    fn sniff_detects_toml_by_a_table_header() {
        assert_eq!(Format::sniff("[architecture]\nname = \"x\""), Format::Toml);
    }

    #[test]
    fn sniff_detects_toml_by_bare_assignment() {
        assert_eq!(Format::sniff("name = \"checkout\""), Format::Toml);
    }

    #[test]
    fn sniff_treats_yaml_mappings_as_yaml() {
        assert_eq!(
            Format::sniff("name: checkout\nversion: 1.0.0"),
            Format::Yaml
        );
    }

    #[test]
    fn sniff_is_not_fooled_by_a_yaml_value_containing_equals() {
        // `command: a = b` has a colon before the equals: still YAML.
        assert_eq!(Format::sniff("command: a = b"), Format::Yaml);
    }

    #[test]
    fn sniff_skips_comments_blank_lines_and_document_markers() {
        let source = "# CASIMIR architecture\n\n---\nname: checkout\n";
        assert_eq!(Format::sniff(source), Format::Yaml);
    }

    #[test]
    fn sniff_defaults_to_yaml_on_an_empty_document() {
        assert_eq!(Format::sniff(""), Format::Yaml);
        assert_eq!(Format::sniff("\n\n   \n"), Format::Yaml);
    }

    #[test]
    fn resolve_prefers_the_extension_over_the_contents() {
        // The file claims JSON; honouring that yields a JSON error, not a YAML one.
        let path = PathBuf::from("a.json");
        assert_eq!(Format::resolve(&path, "name: checkout"), Format::Json);
    }

    #[test]
    fn resolve_falls_back_to_sniffing_without_an_extension() {
        let path = PathBuf::from("stdin");
        assert_eq!(Format::resolve(&path, "{\"name\":\"x\"}"), Format::Json);
    }
}
