//! Module: `casm_core::interface`
//! Purpose: Protocol contracts exposed by a node, and their content-addressed hashes.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # NASA compliance
//!
//! Rule 6 (exhaustive matching): [`Protocol`] is deliberately **not**
//! `#[non_exhaustive]`, with [`Protocol::Custom`] as the explicit escape hatch instead.
//! Marking it non-exhaustive would force every downstream crate — the validator, each
//! renderer backend — to carry a `_ => …` arm, and a wildcard arm is precisely how a
//! newly-added protocol gets silently mishandled everywhere. Paying for exhaustiveness
//! with a major version bump per variant is the trade Rule 6 asks for: adding a protocol
//! is a compile error at every site that must care.
//!
//! Rule 8 (determinism): [`SchemaHash`] is SHA3-256 over the exact contract bytes, so
//! two architectures that declare the same contract produce byte-identical hashes on
//! any machine, on any run.

use core::fmt;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};

use crate::error::InterfaceError;
use crate::names::Name;

/// Width of a SHA3-256 digest in bytes.
const HASH_LEN: usize = 32;

/// The wire protocol an [`Interface`] speaks.
///
/// Serialises in `kebab-case`, so YAML reads `protocol: http2` or `protocol: grpc`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    /// HTTP/1.1.
    #[serde(rename = "http1.1", alias = "http11", alias = "http")]
    Http11,
    /// HTTP/2.
    #[serde(rename = "http2")]
    Http2,
    /// HTTP/3 over QUIC.
    #[serde(rename = "http3")]
    Http3,
    /// gRPC over HTTP/2.
    Grpc,
    /// Apache Kafka.
    Kafka,
    /// AMQP 0-9-1 / 1.0.
    Amqp,
    /// MQTT.
    Mqtt,
    /// GraphQL over HTTP.
    #[serde(rename = "graphql")]
    GraphQl,
    /// WebSocket.
    #[serde(rename = "websocket")]
    WebSocket,
    /// A raw TCP or UDP socket protocol.
    Tcp,
    /// A SQL wire protocol (`PostgreSQL`, `MySQL`, …).
    Sql,
    /// An escape hatch for protocols CASIMIR does not model natively.
    Custom(String),
}

impl Protocol {
    /// Returns the canonical lowercase label for this protocol.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Http11 => "http1.1",
            Self::Http2 => "http2",
            Self::Http3 => "http3",
            Self::Grpc => "grpc",
            Self::Kafka => "kafka",
            Self::Amqp => "amqp",
            Self::Mqtt => "mqtt",
            Self::GraphQl => "graphql",
            Self::WebSocket => "websocket",
            Self::Tcp => "tcp",
            Self::Sql => "sql",
            Self::Custom(label) => label,
        }
    }

    /// Returns `true` if this protocol is request/response by nature.
    ///
    /// Used by the validator to flag synchronous coupling across trust boundaries.
    #[must_use]
    // The `Custom` arm shares a body with the synchronous group, but merging them would
    // hide that "unknown protocols default to synchronous" is a deliberate, conservative
    // choice rather than an accident of grouping.
    #[allow(clippy::match_same_arms)]
    pub const fn is_synchronous(&self) -> bool {
        match self {
            Self::Http11
            | Self::Http2
            | Self::Http3
            | Self::Grpc
            | Self::GraphQl
            | Self::Sql
            | Self::Tcp => true,
            Self::Kafka | Self::Amqp | Self::Mqtt | Self::WebSocket => false,
            // An unmodelled protocol is assumed synchronous: the conservative choice,
            // because it makes the validator warn rather than stay silent.
            Self::Custom(_) => true,
        }
    }

    /// Validates a [`Protocol::Custom`] label.
    ///
    /// # Errors
    ///
    /// Returns [`InterfaceError::EmptyCustomProtocol`] if a custom label is blank.
    pub fn validate(&self) -> Result<(), InterfaceError> {
        match self {
            Self::Custom(label) if label.trim().is_empty() => {
                Err(InterfaceError::EmptyCustomProtocol)
            }
            _ => Ok(()),
        }
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A SHA3-256 digest identifying the exact bytes of an interface contract.
///
/// Content addressing means an interface's identity is its content: if the schema
/// changes by one byte, the hash changes, and every architecture pinning the old hash
/// fails validation loudly instead of drifting silently.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SchemaHash([u8; HASH_LEN]);

impl SchemaHash {
    /// Computes the SHA3-256 digest of `content`.
    #[must_use]
    pub fn of(content: impl AsRef<[u8]>) -> Self {
        let mut hasher = Sha3_256::new();
        hasher.update(content.as_ref());
        let digest = hasher.finalize();

        let mut bytes = [0_u8; HASH_LEN];
        bytes.copy_from_slice(&digest);
        Self(bytes)
    }

    /// Parses a digest from its 64-character lowercase hexadecimal form.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message if `raw` is not exactly 64 hex digits.
    pub fn parse_hex(raw: &str) -> Result<Self, String> {
        crate::hex::decode(raw)
            .map(Self)
            .map_err(|reason| format!("schema hash: {reason}"))
    }

    /// Borrows the raw digest bytes.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; HASH_LEN] {
        &self.0
    }

    /// Renders the digest as 64 lowercase hexadecimal characters.
    #[must_use]
    pub fn to_hex(&self) -> String {
        crate::hex::encode(&self.0)
    }
}

impl fmt::Display for SchemaHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for SchemaHash {
    /// Abbreviated for log readability; full digest available via [`SchemaHash::to_hex`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hex = self.to_hex();
        let prefix = hex.get(..12).unwrap_or(&hex);
        write!(f, "SchemaHash({prefix}…)")
    }
}

impl TryFrom<String> for SchemaHash {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse_hex(&value)
    }
}

impl From<SchemaHash> for String {
    fn from(value: SchemaHash) -> Self {
        value.to_hex()
    }
}

/// A named, versioned protocol contract exposed by a [`crate::Node`].
///
/// # Examples
///
/// ```
/// use casm_core::{Interface, Protocol};
///
/// let iface = Interface::new("public-api", Protocol::Http2, "2.1.0")?
///     .with_schema(br#"{"openapi":"3.1.0"}"#);
///
/// assert_eq!(iface.version().major, 2);
/// assert!(iface.schema_hash().is_some());
/// # Ok::<(), casm_core::error::InterfaceError>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interface {
    name: Name,
    protocol: Protocol,
    version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    schema_hash: Option<SchemaHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

impl Interface {
    /// Constructs an interface, validating the name, protocol, and version.
    ///
    /// # Errors
    ///
    /// - [`InterfaceError::Name`] if the name violates the CASIMIR alphabet.
    /// - [`InterfaceError::InvalidVersion`] if `version` is not Semantic Versioning.
    /// - [`InterfaceError::EmptyCustomProtocol`] if a custom protocol label is blank.
    pub fn new(
        name: impl Into<String>,
        protocol: Protocol,
        version: &str,
    ) -> Result<Self, InterfaceError> {
        let name = Name::new(name)?;
        protocol.validate()?;

        let version = Version::parse(version).map_err(|error| InterfaceError::InvalidVersion {
            name: name.as_str().to_owned(),
            version: version.to_owned(),
            reason: error.to_string(),
        })?;

        Ok(Self {
            name,
            protocol,
            version,
            schema_hash: None,
            description: None,
        })
    }

    /// Attaches the SHA3-256 hash of `content` to this interface.
    #[must_use]
    pub fn with_schema(mut self, content: impl AsRef<[u8]>) -> Self {
        self.schema_hash = Some(SchemaHash::of(content));
        self
    }

    /// Attaches a pre-computed schema hash.
    #[must_use]
    pub const fn with_schema_hash(mut self, hash: SchemaHash) -> Self {
        self.schema_hash = Some(hash);
        self
    }

    /// Attaches a human-readable description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The interface's validated name.
    #[inline]
    #[must_use]
    pub const fn name(&self) -> &Name {
        &self.name
    }

    /// The wire protocol.
    #[inline]
    #[must_use]
    pub const fn protocol(&self) -> &Protocol {
        &self.protocol
    }

    /// The contract's semantic version.
    #[inline]
    #[must_use]
    pub const fn version(&self) -> &Version {
        &self.version
    }

    /// The content hash of the contract, if one was declared.
    #[inline]
    #[must_use]
    pub const fn schema_hash(&self) -> Option<&SchemaHash> {
        self.schema_hash.as_ref()
    }

    /// The human-readable description, if any.
    #[inline]
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Returns `true` if a consumer pinned to `self` can safely talk to `other`.
    ///
    /// Compatibility is SemVer-major equality plus a minor-version floor, on the same
    /// protocol. This is the check that turns "we upgraded the API" from a production
    /// incident into a failed validation run.
    #[must_use]
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.protocol == other.protocol
            && self.version.major == other.version.major
            && other.version.minor >= self.version.minor
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
    fn schema_hash_is_deterministic_across_calls() {
        let a = SchemaHash::of(b"contract-bytes");
        let b = SchemaHash::of(b"contract-bytes");
        assert_eq!(a, b, "SHA3-256 must be a pure function of its input");
    }

    #[test]
    fn schema_hash_matches_the_published_sha3_256_vector() {
        // NIST FIPS 202 test vector: SHA3-256 of the empty string.
        let expected = "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a";
        assert_eq!(SchemaHash::of(b"").to_hex(), expected);
    }

    #[test]
    fn schema_hash_changes_with_a_single_byte() {
        let a = SchemaHash::of(b"contract-v1");
        let b = SchemaHash::of(b"contract-v2");
        assert_ne!(a, b);
    }

    #[test]
    fn schema_hash_round_trips_through_hex() {
        let hash = SchemaHash::of(b"payload");
        let back = SchemaHash::parse_hex(&hash.to_hex()).unwrap();
        assert_eq!(hash, back);
    }

    #[test]
    fn schema_hash_accepts_an_algorithm_prefix() {
        let hash = SchemaHash::of(b"payload");
        let prefixed = format!("sha3-256:{}", hash.to_hex());
        assert_eq!(SchemaHash::parse_hex(&prefixed).unwrap(), hash);
    }

    #[test]
    fn schema_hash_rejects_wrong_length_and_non_hex() {
        assert!(SchemaHash::parse_hex("abcd").is_err());
        assert!(SchemaHash::parse_hex(&"z".repeat(64)).is_err());
    }

    #[test]
    fn schema_hash_debug_is_abbreviated_not_full() {
        let hash = SchemaHash::of(b"payload");
        let debug = format!("{hash:?}");

        // Short *and* actually the hash. Asserting only the length let an impl that wrote
        // nothing at all pass, because an empty string is admirably short.
        assert!(
            debug.len() < 32,
            "debug output must stay log-friendly: {debug}"
        );
        assert!(!debug.is_empty());
        let prefix = hash.to_hex().get(..8).unwrap_or_default().to_owned();
        assert!(
            debug.contains(&prefix),
            "debug output must abbreviate the real value: {debug} does not contain {prefix}"
        );
    }

    #[test]
    fn interface_rejects_a_non_semver_version() {
        let err = Interface::new("api", Protocol::Http2, "v1").unwrap_err();
        assert!(matches!(err, InterfaceError::InvalidVersion { .. }));
    }

    #[test]
    fn interface_rejects_an_empty_custom_protocol() {
        let err = Interface::new("api", Protocol::Custom("  ".into()), "1.0.0").unwrap_err();
        assert_eq!(err, InterfaceError::EmptyCustomProtocol);
    }

    #[test]
    fn a_custom_protocol_is_accepted_when_its_label_is_not_blank() {
        // The existing test only proves a *blank* label is refused, so replacing the
        // guard with `true` — refusing every custom protocol — survived. Both sides of a
        // guard need a case.
        assert!(Protocol::Custom("amqp-1.0".to_owned()).validate().is_ok());
        assert!(Protocol::Custom("x".to_owned()).validate().is_ok());

        for blank in ["", " ", "\t", "\n  "] {
            assert!(
                matches!(
                    Protocol::Custom(blank.to_owned()).validate(),
                    Err(InterfaceError::EmptyCustomProtocol)
                ),
                "{blank:?} should be refused"
            );
        }

        // And a built-in protocol is never subject to the guard at all.
        assert!(Protocol::Http2.validate().is_ok());
    }

    #[test]
    fn protocols_and_hashes_render_the_text_they_are_read_back_from() {
        // `Display` feeds data paths, not only logs: an evidence register and a diagram
        // both render these. An impl returning nothing survived until now.
        assert_eq!(Protocol::Http2.to_string(), "http2");
        assert_eq!(Protocol::Sql.to_string(), "sql");
        assert_eq!(
            Protocol::Custom("amqp-1.0".to_owned()).to_string(),
            "amqp-1.0"
        );

        let hash = SchemaHash::of(b"payload");
        assert_eq!(hash.to_string(), hash.to_hex());
        assert_eq!(hash.to_string().len(), 64);
    }

    #[test]
    fn an_interfaces_description_is_returned_as_written() {
        let plain = Interface::new("rest", Protocol::Http2, "1.0.0").expect("valid");
        assert_eq!(plain.description(), None);

        let described = Interface::new("rest", Protocol::Http2, "1.0.0")
            .expect("valid")
            .with_description("The public API");
        assert_eq!(described.description(), Some("The public API"));
    }

    #[test]
    fn compatibility_requires_matching_major_version() {
        let consumer = Interface::new("api", Protocol::Http2, "1.2.0").unwrap();
        let provider_v2 = Interface::new("api", Protocol::Http2, "2.0.0").unwrap();
        assert!(!consumer.is_compatible_with(&provider_v2));
    }

    #[test]
    fn compatibility_allows_a_forward_minor_version() {
        let consumer = Interface::new("api", Protocol::Http2, "1.2.0").unwrap();
        let provider = Interface::new("api", Protocol::Http2, "1.7.3").unwrap();
        assert!(consumer.is_compatible_with(&provider));
    }

    #[test]
    fn compatibility_rejects_a_backward_minor_version() {
        let consumer = Interface::new("api", Protocol::Http2, "1.5.0").unwrap();
        let provider = Interface::new("api", Protocol::Http2, "1.1.0").unwrap();
        assert!(
            !consumer.is_compatible_with(&provider),
            "provider is missing newer features"
        );
    }

    #[test]
    fn compatibility_requires_matching_protocol() {
        let consumer = Interface::new("api", Protocol::Http2, "1.0.0").unwrap();
        let provider = Interface::new("api", Protocol::Grpc, "1.0.0").unwrap();
        assert!(!consumer.is_compatible_with(&provider));
    }

    #[test]
    fn protocols_are_classified_by_synchrony() {
        assert!(Protocol::Grpc.is_synchronous());
        assert!(Protocol::Http2.is_synchronous());
        assert!(!Protocol::Kafka.is_synchronous());
        assert!(!Protocol::Mqtt.is_synchronous());
        assert!(
            Protocol::Custom("proprietary".into()).is_synchronous(),
            "conservative default"
        );
    }

    #[test]
    fn protocol_deserialises_from_friendly_yaml_aliases() {
        let http: Protocol = serde_json::from_str("\"http\"").unwrap();
        assert_eq!(http, Protocol::Http11);
        let http11: Protocol = serde_json::from_str("\"http1.1\"").unwrap();
        assert_eq!(http11, Protocol::Http11);
    }

    #[test]
    fn interface_round_trips_through_json() {
        let original = Interface::new("public-api", Protocol::Grpc, "3.1.4")
            .unwrap()
            .with_schema(b"proto-bytes")
            .with_description("Public gRPC surface");

        let json = serde_json::to_string(&original).unwrap();
        let back: Interface = serde_json::from_str(&json).unwrap();
        assert_eq!(original, back);
    }

    #[test]
    fn interface_rejects_unknown_fields() {
        let json = r#"{"name":"api","protocol":"grpc","version":"1.0.0","typo":true}"#;
        let parsed: Result<Interface, _> = serde_json::from_str(json);
        assert!(
            parsed.is_err(),
            "deny_unknown_fields must catch author typos"
        );
    }
}
