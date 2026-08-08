//! Module: `casm_parser::suggest`
//! Purpose: "Did you mean …?" hints for misspelled node references and enum variants.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # NASA compliance
//!
//! Rule 4 (statically provable loop bounds): the edit-distance computation is bounded by
//! the product of the two input lengths, and both are capped — candidates are CASM
//! [`casm_core::Name`]s, which cannot exceed 128 bytes. A hostile document therefore
//! cannot turn suggestion generation into a denial of service.
//!
//! Rule 5 (no unbounded allocation): the algorithm keeps two rows, not a full matrix.

use casm_core::MAX_NAME_LEN;

/// The largest edit distance still considered a plausible typo.
///
/// Beyond this the "suggestion" is noise: proposing `orders-db` for `payments` helps
/// nobody, and a confidently wrong hint is worse than none.
pub const MAX_PLAUSIBLE_DISTANCE: usize = 3;

/// Computes the Levenshtein edit distance between two strings.
///
/// Comparison is case-insensitive on ASCII, because `Service` versus `service` is a
/// capitalisation slip rather than a different word.
#[must_use]
pub fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().flat_map(char::to_lowercase).collect();
    let right: Vec<char> = right.chars().flat_map(char::to_lowercase).collect();

    if left.is_empty() {
        return right.len();
    }
    if right.is_empty() {
        return left.len();
    }

    // Two rolling rows rather than a full (n+1)×(m+1) matrix.
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current: Vec<usize> = vec![0; right.len() + 1];

    for (i, left_char) in left.iter().enumerate() {
        if let Some(slot) = current.first_mut() {
            *slot = i + 1;
        }

        for (j, right_char) in right.iter().enumerate() {
            let substitution_cost = usize::from(left_char != right_char);
            let deletion = previous
                .get(j + 1)
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(1);
            let insertion = current
                .get(j)
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(1);
            let substitution = previous
                .get(j)
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(substitution_cost);

            if let Some(slot) = current.get_mut(j + 1) {
                *slot = deletion.min(insertion).min(substitution);
            }
        }

        core::mem::swap(&mut previous, &mut current);
    }

    previous.last().copied().unwrap_or(0)
}

/// Returns the candidate closest to `input`, if any is close enough to be worth saying.
///
/// Returns `None` when the nearest candidate is further than
/// [`MAX_PLAUSIBLE_DISTANCE`], or when two candidates tie — an ambiguous suggestion
/// misleads more than it helps.
#[must_use]
pub fn closest<'a, I>(input: &str, candidates: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    // Rule 4: bound the work regardless of what the document contains.
    if input.len() > MAX_NAME_LEN {
        return None;
    }

    let mut best: Option<(&'a str, usize)> = None;
    let mut tied = false;

    for candidate in candidates {
        let distance = edit_distance(input, candidate);

        match best {
            Some((_, best_distance)) if distance < best_distance => {
                best = Some((candidate, distance));
                tied = false;
            }
            Some((_, best_distance)) if distance == best_distance => tied = true,
            Some(_) => {}
            None => best = Some((candidate, distance)),
        }
    }

    match best {
        Some((candidate, distance)) if distance <= MAX_PLAUSIBLE_DISTANCE && !tied => {
            Some(candidate)
        }
        _ => None,
    }
}

/// Formats a "did you mean" hint for a misspelled reference.
#[must_use]
pub fn did_you_mean(candidate: &str) -> String {
    format!("did you mean `{candidate}`?")
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
    fn distance_is_zero_for_identical_strings() {
        assert_eq!(edit_distance("service", "service"), 0);
    }

    #[test]
    fn distance_is_case_insensitive() {
        assert_eq!(edit_distance("Service", "service"), 0);
    }

    #[test]
    fn distance_counts_single_edits() {
        assert_eq!(edit_distance("srvice", "service"), 1, "one insertion");
        assert_eq!(edit_distance("services", "service"), 1, "one deletion");
        assert_eq!(edit_distance("servace", "service"), 1, "one substitution");
    }

    #[test]
    fn distance_handles_empty_strings() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", ""), 3);
    }

    #[test]
    fn distance_accumulates_edits_rather_than_multiplying_them() {
        // The inner recurrence adds one to a neighbouring cell. Replacing that `+` with
        // `*` survived: for distances of zero and one, `n + 1` and `n * 1` frequently
        // agree, and every existing case was one of those.
        assert_eq!(edit_distance("service", "srvice"), 1);
        assert_eq!(edit_distance("service", "srvic"), 2);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("abcdef", "uvwxyz"), 6);
        assert_eq!(edit_distance("", "abcd"), 4);

        // The mutated cell is the leading column of each row, which is the running cost
        // of deleting from the left string. It only shows when the left is *longer* than
        // the right, and every case above had it shorter or equal — so the first version
        // of this test left the mutant alive.
        assert_eq!(edit_distance("abc", "b"), 2);
        assert_eq!(edit_distance("abcd", "x"), 4);
        assert_eq!(edit_distance("service", "a"), 7);
        assert_eq!(edit_distance("ab", "b"), 1);
    }

    #[test]
    fn distance_is_symmetric() {
        assert_eq!(
            edit_distance("kitten", "sitting"),
            edit_distance("sitting", "kitten")
        );
        assert_eq!(
            edit_distance("kitten", "sitting"),
            3,
            "the textbook example"
        );
    }

    #[test]
    fn distance_handles_multibyte_characters_without_panicking() {
        // Char-based, not byte-based: this must not slice through a UTF-8 boundary.
        assert_eq!(edit_distance("café", "cafe"), 1);
        let _ = edit_distance("日本語", "日本");
    }

    #[test]
    fn closest_finds_an_obvious_typo() {
        let candidates = ["orders-db", "payments", "gateway"];
        assert_eq!(closest("orders-bd", candidates), Some("orders-db"));
    }

    #[test]
    fn the_suggestion_threshold_is_a_strict_comparison() {
        // `closest` refuses an input longer than `MAX_NAME_LEN` with `>`. Replacing it
        // with `>=` refuses one *at* the ceiling too, and with `==` refuses only that one
        // exact length — both survived, because nothing tested the boundary itself.
        let at_ceiling = "a".repeat(MAX_NAME_LEN);
        let candidate = at_ceiling.clone();

        assert_eq!(
            closest(&at_ceiling, [candidate.as_str()]),
            Some(candidate.as_str()),
            "a name exactly at the ceiling is still worth a suggestion"
        );

        let over = "a".repeat(MAX_NAME_LEN + 1);
        assert_eq!(
            closest(&over, [candidate.as_str()]),
            None,
            "one character further is refused"
        );
    }

    #[test]
    fn closest_declines_when_nothing_is_near() {
        let candidates = ["orders-db", "payments", "gateway"];
        assert_eq!(
            closest("zzzzzzzzzzzz", candidates),
            None,
            "a wrong hint is worse than none"
        );
    }

    #[test]
    fn closest_declines_on_a_tie() {
        // `ab` is distance 1 from both; proposing either would be a coin flip.
        assert_eq!(closest("ab", ["abc", "abd"]), None);
    }

    #[test]
    fn closest_handles_an_empty_candidate_set() {
        assert_eq!(closest("anything", []), None);
    }

    #[test]
    fn closest_refuses_pathologically_long_input() {
        let huge = "a".repeat(MAX_NAME_LEN + 1);
        assert_eq!(
            closest(&huge, ["api"]),
            None,
            "bounded work, per NASA Rule 4"
        );
    }

    #[test]
    fn closest_accepts_an_exact_match() {
        assert_eq!(closest("api", ["api", "gateway"]), Some("api"));
    }

    #[test]
    fn hint_is_formatted_for_a_terminal() {
        assert_eq!(did_you_mean("service"), "did you mean `service`?");
    }
}
