//! Module: `casm_lsp::text`
//! Purpose: Positions and spans in the coordinate system the protocol actually uses.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # The UTF-16 trap
//!
//! LSP positions are **zero-based lines and UTF-16 code-unit offsets**, not byte offsets
//! and not character counts. Rust strings are UTF-8, so the three coincide only for
//! ASCII — which every CASIMIR [`casm_core::Name`] is, but which descriptions and
//! comments are not.
//!
//! Getting this wrong produces a bug that is invisible until someone writes an em-dash in
//! a description, at which point every highlight on that line silently shifts. The
//! conversion therefore lives in one place, [`utf16_len`], and every span in this crate is
//! constructed through it.

/// A zero-based position in a text document, in LSP coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    /// Zero-based line number.
    pub line: u32,
    /// Zero-based offset within the line, in UTF-16 code units.
    pub character: u32,
}

impl Position {
    /// Constructs a position.
    #[must_use]
    pub const fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }
}

/// A contiguous range within a single line, in LSP coordinates.
///
/// Single-line by construction: every symbol CASIMIR's grammar can produce — a key, a
/// scalar value, a node name — lives on one line. Multi-line ranges are built at the
/// protocol boundary when a diagnostic needs to span a block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    /// Zero-based line number.
    pub line: u32,
    /// Inclusive start offset, in UTF-16 code units.
    pub start: u32,
    /// Exclusive end offset, in UTF-16 code units.
    pub end: u32,
}

impl Span {
    /// Constructs a span.
    #[must_use]
    pub const fn new(line: u32, start: u32, end: u32) -> Self {
        Self { line, start, end }
    }

    /// An empty span at the start of `line`.
    #[must_use]
    pub const fn line_start(line: u32) -> Self {
        Self {
            line,
            start: 0,
            end: 0,
        }
    }

    /// Returns `true` if `position` falls within this span.
    ///
    /// The end is treated as inclusive. A cursor sitting immediately after a word is,
    /// to a user, still "on" that word — and hover that dies at the last character feels
    /// broken even though it is technically correct.
    #[must_use]
    pub const fn contains(&self, position: Position) -> bool {
        position.line == self.line
            && position.character >= self.start
            && position.character <= self.end
    }

    /// The span's width in UTF-16 code units.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// Returns `true` if the span covers no characters.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// Returns the length of `text` in UTF-16 code units.
///
/// This is the unit LSP counts in. A character outside the Basic Multilingual Plane —
/// an emoji, say — is two units, not one.
#[must_use]
pub fn utf16_len(text: &str) -> u32 {
    // A document long enough to overflow `u32` would exceed the parser's size ceiling
    // long before reaching here, but saturate rather than wrap regardless.
    u32::try_from(text.chars().map(char::len_utf16).sum::<usize>()).unwrap_or(u32::MAX)
}

/// Converts a byte offset within `line` into a UTF-16 code-unit offset.
///
/// An offset past the end of the line, or one that falls inside a multi-byte character,
/// yields the length of the whole line rather than panicking.
#[must_use]
pub fn byte_to_utf16(line: &str, byte_offset: usize) -> u32 {
    line.get(..byte_offset)
        .map_or_else(|| utf16_len(line), utf16_len)
}

/// Converts a UTF-16 code-unit offset within `line` into a byte offset.
///
/// The inverse of [`byte_to_utf16`], needed whenever the protocol hands us a cursor
/// position and we must slice the line at it. An offset past the end of the line clamps
/// to the line's length, so slicing with the result is always valid.
#[must_use]
pub fn utf16_to_byte(line: &str, utf16_offset: u32) -> usize {
    let mut consumed = 0_u32;

    for (byte_offset, character) in line.char_indices() {
        if consumed >= utf16_offset {
            return byte_offset;
        }
        // A cursor cannot sit inside a surrogate pair, so landing mid-character means the
        // client sent a position we cannot honour; take the character boundary before it.
        consumed = consumed.saturating_add(u32::try_from(character.len_utf16()).unwrap_or(1));
    }

    line.len()
}

/// Builds a span covering `[start_byte, end_byte)` of `line`.
#[must_use]
pub fn span_of_bytes(line_number: u32, line: &str, start_byte: usize, end_byte: usize) -> Span {
    Span::new(
        line_number,
        byte_to_utf16(line, start_byte),
        byte_to_utf16(line, end_byte),
    )
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
    fn utf16_length_matches_byte_length_for_ascii() {
        assert_eq!(utf16_len("payment-service"), 15);
        assert_eq!(utf16_len(""), 0);
    }

    #[test]
    fn utf16_length_counts_code_units_not_characters() {
        // 'é' is one UTF-16 unit but two UTF-8 bytes.
        assert_eq!(utf16_len("café"), 4);
        assert_eq!(
            "café".len(),
            5,
            "the byte length differs, which is the whole point"
        );

        // An emoji outside the BMP is a surrogate pair: two units.
        assert_eq!(utf16_len("🚀"), 2);
        assert_eq!(utf16_len("a🚀b"), 4);
    }

    #[test]
    fn byte_offsets_convert_to_utf16_offsets() {
        let line = "  name: café";
        assert_eq!(byte_to_utf16(line, 0), 0);
        assert_eq!(byte_to_utf16(line, 2), 2);
        assert_eq!(byte_to_utf16(line, 8), 8, "start of the value");
        assert_eq!(
            byte_to_utf16(line, line.len()),
            12,
            "'café' is 4 units, not 5"
        );
    }

    #[test]
    fn conversion_is_total_for_out_of_range_and_split_offsets() {
        let line = "café";
        // Past the end.
        assert_eq!(byte_to_utf16(line, 9_999), utf16_len(line));
        // Inside the two-byte 'é': `get` returns None, so we fall back to the full length.
        assert_eq!(byte_to_utf16(line, 4), utf16_len(line));
    }

    #[test]
    fn utf16_offsets_convert_back_to_byte_offsets() {
        let line = "  name: café";
        assert_eq!(utf16_to_byte(line, 0), 0);
        assert_eq!(utf16_to_byte(line, 8), 8);
        assert_eq!(
            utf16_to_byte(line, 12),
            line.len(),
            "past 'é' is the full byte length"
        );
    }

    #[test]
    fn utf16_to_byte_round_trips_with_byte_to_utf16() {
        for line in ["plain ascii", "  café au lait", "a🚀b", ""] {
            for (byte_offset, _) in line.char_indices() {
                let units = byte_to_utf16(line, byte_offset);
                assert_eq!(
                    utf16_to_byte(line, units),
                    byte_offset,
                    "round trip failed at byte {byte_offset} of {line:?}"
                );
            }
        }
    }

    #[test]
    fn utf16_to_byte_clamps_past_the_end_so_slicing_is_always_safe() {
        let line = "short";
        assert_eq!(utf16_to_byte(line, 9_999), line.len());
        assert!(line.get(..utf16_to_byte(line, 9_999)).is_some());
    }

    #[test]
    fn spans_are_built_from_byte_ranges() {
        let line = "  type: service";
        let span = span_of_bytes(3, line, 8, 15);
        assert_eq!(span, Span::new(3, 8, 15));
        assert_eq!(span.width(), 7);
    }

    #[test]
    fn spans_over_multibyte_text_are_narrower_than_their_byte_range() {
        let line = "  desc: café";
        let span = span_of_bytes(0, line, 8, 13);
        assert_eq!(span.width(), 4, "five bytes, four UTF-16 units");
    }

    #[test]
    fn containment_covers_the_span_inclusively_at_both_ends() {
        let span = Span::new(2, 4, 8);
        assert!(span.contains(Position::new(2, 4)), "the first character");
        assert!(span.contains(Position::new(2, 6)));
        assert!(
            span.contains(Position::new(2, 8)),
            "a cursor just past the word"
        );
        assert!(!span.contains(Position::new(2, 3)));
        assert!(!span.contains(Position::new(2, 9)));
    }

    #[test]
    fn containment_requires_the_same_line() {
        let span = Span::new(2, 4, 8);
        assert!(!span.contains(Position::new(1, 6)));
        assert!(!span.contains(Position::new(3, 6)));
    }

    #[test]
    fn an_empty_span_is_identified() {
        assert!(Span::line_start(5).is_empty());
        assert!(Span::new(0, 3, 3).is_empty());
        assert!(!Span::new(0, 3, 4).is_empty());
    }

    #[test]
    fn width_saturates_rather_than_underflowing() {
        // Nothing constructs this, but the arithmetic must be total.
        assert_eq!(Span::new(0, 10, 4).width(), 0);
    }

    #[test]
    fn positions_order_by_line_then_character() {
        assert!(Position::new(1, 5) < Position::new(2, 0));
        assert!(Position::new(1, 5) < Position::new(1, 6));
    }
}
