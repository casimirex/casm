//! Module: `casm_core::hex`
//! Purpose: The one hexadecimal conversion, shared by every digest type.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! CASIMIR has two content digests — [`crate::SchemaHash`] for interface contracts and
//! [`crate::Fingerprint`] for whole architectures — and both are SHA3-256 rendered as 64
//! lowercase hexadecimal characters. Two copies of that conversion would be two places to
//! get an off-by-one wrong, so it lives here.

/// Renders `bytes` as lowercase hexadecimal.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        // Two hex digits per byte. `from_digit` cannot fail for a value below 16.
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    out
}

/// Parses exactly `N` bytes from a hexadecimal string.
///
/// An optional `algorithm:` prefix is accepted and ignored, so `sha3-256:abcd…` and
/// `abcd…` both parse.
///
/// # Errors
///
/// Returns a human-readable message naming the offending length or byte.
pub(crate) fn decode<const N: usize>(raw: &str) -> Result<[u8; N], String> {
    let raw = raw.rsplit_once(':').map_or(raw, |(_, digits)| digits);
    let expected = N.saturating_mul(2);

    if raw.len() != expected {
        return Err(format!(
            "expected {expected} hexadecimal digits, found {}",
            raw.len()
        ));
    }

    let mut bytes = [0_u8; N];
    for (index, slot) in bytes.iter_mut().enumerate() {
        let start = index.saturating_mul(2);
        let pair = raw
            .get(start..start.saturating_add(2))
            .ok_or_else(|| "value is not valid ASCII hexadecimal".to_owned())?;
        *slot = u8::from_str_radix(pair, 16)
            .map_err(|_| format!("'{pair}' at offset {start} is not a hexadecimal byte"))?;
    }

    Ok(bytes)
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
    fn encoding_pads_every_byte_to_two_digits() {
        assert_eq!(encode(&[0x00, 0x0f, 0xff]), "000fff");
        assert_eq!(encode(&[]), "");
    }

    #[test]
    fn encoding_is_lowercase() {
        let encoded = encode(&[0xab, 0xcd, 0xef]);
        assert_eq!(encoded, "abcdef");
        assert_eq!(encoded, encoded.to_lowercase());
    }

    #[test]
    fn decoding_round_trips_with_encoding() {
        let original = [0_u8, 1, 127, 128, 255, 16, 32, 64];
        let decoded: [u8; 8] = decode(&encode(&original)).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn decoding_accepts_an_algorithm_prefix() {
        let decoded: [u8; 2] = decode("sha3-256:00ff").unwrap();
        assert_eq!(decoded, [0x00, 0xff]);
    }

    #[test]
    fn decoding_accepts_uppercase_digits() {
        let decoded: [u8; 2] = decode("ABCD").unwrap();
        assert_eq!(decoded, [0xab, 0xcd]);
    }

    #[test]
    fn decoding_rejects_the_wrong_length_and_says_what_it_wanted() {
        let error = decode::<4>("abcd").unwrap_err();
        assert!(error.contains('8'), "{error}");
        assert!(error.contains('4'), "{error}");
    }

    #[test]
    fn decoding_rejects_non_hexadecimal_input() {
        assert!(decode::<2>("zzzz").is_err());
        assert!(decode::<2>("00 f").is_err());
    }

    #[test]
    fn decoding_is_total_on_arbitrary_input() {
        for raw in ["", ":", "🚀🚀", "sha3-256:", &"a".repeat(1000)] {
            let _ = decode::<32>(raw);
        }
    }
}
