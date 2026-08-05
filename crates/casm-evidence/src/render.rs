//! Module: `casm_evidence::render`
//! Purpose: Writing a claims register out, in a form somebody will actually read.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # The wording is load-bearing
//!
//! Every heading here says *claimed*, *asserted*, or *outstanding*. None says *satisfied*,
//! *compliant*, or *verified*. That is not house style — it is the decision in
//! ADR-0013 made visible in the only place a reader will encounter it.
//!
//! A register handed to an auditor with the word "evidence" at the top and a list of
//! assertions underneath is a document that misrepresents itself. Every rendering here
//! opens with one sentence saying what CASIMIR checked and what it did not, and that
//! sentence is not optional or configurable.

use core::fmt::Write as _;

use crate::pack::Pack;

/// The preamble every rendering opens with.
///
/// Not configurable, and not omittable. A reader who sees only the tables must still be
/// unable to mistake this document for verified evidence.
const DISCLAIMER: &str = "This is a register of the controls this architecture CLAIMS, \
                          assembled from the file and its history. CASIMIR verified the \
                          structure, not the reality: nothing here is evidence that a \
                          control is implemented.";

/// How to render a register.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// Aligned plain text for a terminal.
    #[default]
    Human,
    /// GitHub-flavoured Markdown, for pasting into a ticket or a wiki.
    Markdown,
}

impl Format {
    /// Parses a format name, as a command-line flag would give it.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "human" | "text" => Some(Self::Human),
            "markdown" | "md" => Some(Self::Markdown),
            _ => None,
        }
    }

    /// Every accepted name, for an error message that lists the alternatives.
    #[must_use]
    pub const fn names() -> &'static [&'static str] {
        &["human", "markdown"]
    }
}

/// Renders `pack` in `format`.
#[must_use]
pub fn render(pack: &Pack, format: Format) -> String {
    match format {
        Format::Human => human(pack),
        Format::Markdown => markdown(pack),
    }
}

/// Aligned plain text.
fn human(pack: &Pack) -> String {
    let mut out = format!(
        "Control claims register — {} v{}\n\n{DISCLAIMER}\n\n",
        pack.architecture, pack.version
    );

    let _ = writeln!(out, "  fingerprint  {}", pack.fingerprint);
    let _ = writeln!(out, "  provenance   {}", pack.provenance.describe());
    if let Some(revisions) = pack.provenance.semantic_revisions {
        let _ = writeln!(out, "  history      {revisions} semantic revision(s)");
    }
    out.push('\n');

    if pack.standards.is_empty() {
        out.push_str("No controls are declared, so there is nothing to register.\n");
        return out;
    }

    for record in &pack.standards {
        let _ = writeln!(
            out,
            "{}  ({} claim(s), {} outstanding)",
            record.standard,
            record.claims.len(),
            record.outstanding
        );

        for claim in &record.claims {
            let marker = if claim.evidence_required { "!" } else { " " };
            let _ = writeln!(
                out,
                "  {marker} {:<20} {:<16} {}",
                claim.node, claim.control_type, claim.description
            );
        }

        for pattern in &record.corroborating_patterns {
            let _ = writeln!(out, "    corroborated by conformance to {pattern}");
        }
        out.push('\n');
    }

    out.push_str(&summary_lines(pack));
    out
}

/// GitHub-flavoured Markdown.
fn markdown(pack: &Pack) -> String {
    let mut out = format!(
        "# Control claims register — {} v{}\n\n> {DISCLAIMER}\n\n",
        pack.architecture, pack.version
    );

    let _ = writeln!(out, "| | |\n|---|---|");
    let _ = writeln!(out, "| Fingerprint | `{}` |", pack.fingerprint);
    let _ = writeln!(out, "| Provenance | {} |", pack.provenance.describe());
    if let Some(source) = &pack.provenance.source {
        let _ = writeln!(out, "| Source | `{source}` |");
    }
    if let Some(revisions) = pack.provenance.semantic_revisions {
        let _ = writeln!(out, "| Semantic revisions | {revisions} |");
    }
    out.push('\n');

    if pack.standards.is_empty() {
        out.push_str("No controls are declared, so there is nothing to register.\n");
        return out;
    }

    for record in &pack.standards {
        let _ = writeln!(
            out,
            "## {}\n\n{} claim(s), **{} outstanding**.\n",
            record.standard,
            record.claims.len(),
            record.outstanding
        );
        out.push_str("| Node | Type | Control | Claimed | Evidence |\n");
        out.push_str("|---|---|---|---|---|\n");

        for claim in &record.claims {
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} | {} |",
                claim.node,
                claim.node_type,
                claim.control_type,
                claim.description,
                if claim.evidence_required {
                    "**outstanding**"
                } else {
                    "not flagged"
                }
            );
        }
        out.push('\n');

        for pattern in &record.corroborating_patterns {
            let _ = writeln!(
                out,
                "Structurally corroborated by conformance to `{pattern}`.\n"
            );
        }
    }

    out.push_str("## Summary\n\n");
    out.push_str(&summary_lines(pack));
    out
}

/// The closing tally, shared by both formats so they can never disagree.
fn summary_lines(pack: &Pack) -> String {
    let mut out = format!(
        "{} claim(s) across {} standard(s); {} outstanding.\n",
        pack.total_claims(),
        pack.standards.len(),
        pack.outstanding()
    );

    if pack.is_silent() {
        out.push_str(
            "No claim is flagged as needing evidence. That is not the same as having \
             collected it: set `evidence-required: true` on the controls an auditor will \
             ask about.\n",
        );
    }

    if pack.has_unchecked_conformance() {
        out.push_str(
            "One or more claimed patterns were not in the supplied library, so they \
             corroborate nothing here. Re-run with `--patterns <dir>`.\n",
        );
    }

    if !pack.uncontrolled_nodes.is_empty() {
        let _ = writeln!(
            out,
            "{} node(s) declare no control at all: {}.",
            pack.uncontrolled_nodes.len(),
            pack.uncontrolled_nodes.join(", ")
        );
    }

    out
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
    use crate::provenance::{Attribution, Provenance};
    use casm_core::{Architecture, Control, ControlType, Node, NodeType};

    fn architecture() -> Architecture {
        Architecture::builder()
            .name("checkout")
            .version("1.4.0")
            .node(
                Node::builder()
                    .name("orders-db")
                    .node_type(NodeType::Database)
                    .control(
                        Control::new(
                            ControlType::Compliance,
                            "ISO27001-A.10.1",
                            "Encrypted at rest with a customer-managed key",
                        )
                        .unwrap()
                        .requiring_evidence(),
                    )
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap()
    }

    fn provenance() -> Provenance {
        Provenance::unknown()
            .from_source("architecture.yaml")
            .at(Attribution {
                commit: "eb263e4c1f0a94c4b844daf52c8694b70abcdef1".to_owned(),
                author: "Ada Lovelace".to_owned(),
                email: "ada@example.com".to_owned(),
                date: "2026-08-05".to_owned(),
                summary: "Encrypt the orders database".to_owned(),
            })
            .with_semantic_revisions(3)
    }

    fn pack() -> Pack {
        Pack::assemble(&architecture(), &[], provenance())
    }

    #[test]
    fn every_rendering_opens_with_the_disclaimer() {
        // The load-bearing sentence. A register that reads as verified evidence is the
        // one failure mode this crate exists to prevent.
        for format in [Format::Human, Format::Markdown] {
            let text = render(&pack(), format);
            assert!(text.contains(DISCLAIMER), "{format:?}:\n{text}");
        }
    }

    #[test]
    fn no_rendering_ever_says_a_control_is_satisfied() {
        // Scanned over the body, not the disclaimer — the disclaimer says "verified the
        // structure, not the reality", which is the distinction being enforced, not a
        // violation of it.
        for format in [Format::Human, Format::Markdown] {
            let rendered = render(&pack(), format);
            let body = rendered
                .split_once(DISCLAIMER)
                .map(|(_, rest)| rest)
                .expect("every rendering carries the disclaimer")
                .to_lowercase();

            // Whole words, not substrings: "provenance" contains "proven", and a
            // register that could not use the word would be a worse register.
            let words: Vec<&str> = body
                .split(|c: char| !c.is_ascii_alphabetic())
                .filter(|word| !word.is_empty())
                .collect();

            for forbidden in ["satisfied", "satisfies", "compliant", "verified", "proven"] {
                assert!(
                    !words.contains(&forbidden),
                    "{format:?} says {forbidden:?}:\n{body}"
                );
            }
        }
    }

    #[test]
    fn the_fingerprint_and_provenance_are_shown_so_a_reader_can_check_them() {
        for format in [Format::Human, Format::Markdown] {
            let text = render(&pack(), format);
            assert!(text.contains(&pack().fingerprint), "{format:?}");
            assert!(text.contains("Ada Lovelace"), "{format:?}");
            assert!(text.contains("eb263e4"), "{format:?}");
            assert!(text.contains('3'), "the revision count: {format:?}");
        }
    }

    #[test]
    fn an_outstanding_claim_is_marked_in_both_formats() {
        assert!(render(&pack(), Format::Human).contains("! orders-db"));
        assert!(render(&pack(), Format::Markdown).contains("**outstanding**"));
    }

    #[test]
    fn the_summary_tallies_the_same_numbers_in_both_formats() {
        // Two renderings that disagree about the count would make the document useless.
        for format in [Format::Human, Format::Markdown] {
            let text = render(&pack(), format);
            assert!(
                text.contains("1 claim(s) across 1 standard(s); 1 outstanding."),
                "{format:?}:\n{text}"
            );
        }
    }

    #[test]
    fn an_architecture_with_no_controls_says_so_plainly() {
        let bare = Architecture::builder()
            .name("bare")
            .node(
                Node::builder()
                    .name("api")
                    .node_type(NodeType::Service)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();
        let pack = Pack::assemble(&bare, &[], Provenance::unknown());

        for format in [Format::Human, Format::Markdown] {
            let text = render(&pack, format);
            assert!(text.contains("nothing to register"), "{format:?}:\n{text}");
        }
    }

    #[test]
    fn a_silent_register_is_told_what_the_silence_means() {
        let quiet = Architecture::builder()
            .name("quiet")
            .node(
                Node::builder()
                    .name("api")
                    .node_type(NodeType::Service)
                    .control(
                        Control::new(ControlType::Security, "OIDC", "Tokens are validated")
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();
        let pack = Pack::assemble(&quiet, &[], Provenance::unknown());

        let text = render(&pack, Format::Human);
        assert!(
            text.contains("not the same as having collected it"),
            "{text}"
        );
        assert!(text.contains("evidence-required"), "{text}");
    }

    #[test]
    fn unknown_provenance_is_stated_rather_than_hidden() {
        let pack = Pack::assemble(&architecture(), &[], Provenance::unknown());

        for format in [Format::Human, Format::Markdown] {
            let text = render(&pack, format);
            assert!(
                text.contains("not under version control"),
                "{format:?}:\n{text}"
            );
        }
    }

    #[test]
    fn uncontrolled_nodes_are_named_and_only_when_there_are_some() {
        // Deleting the `!` from this branch's condition survived the mutation sweep:
        // nothing asserted the line appears, so nothing noticed it appearing backwards.
        let bare = Architecture::builder()
            .name("mixed")
            .node(
                Node::builder()
                    .name("orders-db")
                    .node_type(NodeType::Database)
                    .control(
                        Control::new(ControlType::Compliance, "ISO27001-A.10.1", "Encrypted")
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .node(
                Node::builder()
                    .name("worker")
                    .node_type(NodeType::Service)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();

        let with_gap = Pack::assemble(&bare, &[], Provenance::unknown());
        for format in [Format::Human, Format::Markdown] {
            let text = render(&with_gap, format);
            assert!(
                text.contains("1 node(s) declare no control at all: worker."),
                "{format:?}:\n{text}"
            );
        }

        // And every node controlled means the line must be absent, not empty.
        for format in [Format::Human, Format::Markdown] {
            let text = render(&pack(), format);
            assert!(!text.contains("declare no control"), "{format:?}:\n{text}");
        }
    }

    #[test]
    fn rendering_is_deterministic() {
        for format in [Format::Human, Format::Markdown] {
            assert_eq!(
                render(&pack(), format),
                render(&pack(), format),
                "{format:?}"
            );
        }
    }

    #[test]
    fn markdown_tables_are_well_formed() {
        let text = render(&pack(), Format::Markdown);

        assert!(text.contains("| Node | Type | Control | Claimed | Evidence |"));
        assert!(text.contains("|---|---|---|---|---|"));
        assert!(text.starts_with("# Control claims register"));
    }

    #[test]
    fn format_names_round_trip_and_unknown_ones_are_refused() {
        for name in Format::names() {
            assert!(Format::parse(name).is_some(), "{name}");
        }

        assert_eq!(Format::parse("md"), Some(Format::Markdown));
        assert_eq!(Format::parse("pdf"), None);
        assert_eq!(Format::parse(""), None);
    }
}
