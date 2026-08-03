//! Module: `casm_validator::config`
//! Purpose: The thresholds a validation run is parameterised by.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Defaults are opinions
//!
//! Every value here encodes a judgement, and each is stated in its own documentation so
//! a team that disagrees knows exactly what they are overriding rather than discovering
//! the threshold from a failed build.

use serde::{Deserialize, Serialize};

/// The default end-to-end latency ceiling, in milliseconds.
///
/// One second is the widely-cited threshold beyond which an interaction stops feeling
/// responsive. An architecture whose critical path already exceeds it on paper cannot
/// meet it in production, where real networks are slower than budgets.
pub const DEFAULT_MAX_CRITICAL_PATH_MS: u64 = 1_000;

/// The default number of security controls required per service.
///
/// Two, not one: a single control is almost always "we have TLS", which says nothing
/// about authorisation. Requiring two forces a second, different thought.
pub const DEFAULT_MIN_SECURITY_CONTROLS: usize = 2;

/// Thresholds and toggles for a validation run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case", default)]
pub struct ValidatorConfig {
    /// The end-to-end latency ceiling for the critical path, in milliseconds.
    pub max_critical_path_ms: u64,

    /// How many `type: security` controls each service and gateway must declare.
    ///
    /// Set to `0` to disable the rule entirely.
    pub min_security_controls_per_service: usize,

    /// Rule identifiers to suppress for this run.
    ///
    /// Suppression is by explicit id rather than by severity, so silencing one noisy
    /// rule cannot accidentally silence an unrelated error.
    pub allow: Vec<String>,
}

impl Default for ValidatorConfig {
    fn default() -> Self {
        Self {
            max_critical_path_ms: DEFAULT_MAX_CRITICAL_PATH_MS,
            min_security_controls_per_service: DEFAULT_MIN_SECURITY_CONTROLS,
            allow: Vec::new(),
        }
    }
}

impl ValidatorConfig {
    /// A configuration with every threshold at its default.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the critical-path latency ceiling.
    #[must_use]
    pub const fn max_critical_path_ms(mut self, ceiling: u64) -> Self {
        self.max_critical_path_ms = ceiling;
        self
    }

    /// Sets how many security controls each service must declare.
    #[must_use]
    pub const fn min_security_controls_per_service(mut self, minimum: usize) -> Self {
        self.min_security_controls_per_service = minimum;
        self
    }

    /// Suppresses a rule by its identifier.
    #[must_use]
    pub fn allowing(mut self, rule_id: impl Into<String>) -> Self {
        let rule_id = rule_id.into();
        if !self.allow.contains(&rule_id) {
            self.allow.push(rule_id);
        }
        self
    }

    /// Returns `true` if `rule_id` has been suppressed.
    #[must_use]
    pub fn is_allowed(&self, rule_id: &str) -> bool {
        self.allow.iter().any(|allowed| allowed == rule_id)
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
    fn defaults_match_the_documented_constants() {
        let config = ValidatorConfig::default();
        assert_eq!(config.max_critical_path_ms, DEFAULT_MAX_CRITICAL_PATH_MS);
        assert_eq!(
            config.min_security_controls_per_service,
            DEFAULT_MIN_SECURITY_CONTROLS
        );
        assert!(config.allow.is_empty());
    }

    #[test]
    fn builders_override_individual_thresholds() {
        let config = ValidatorConfig::new()
            .max_critical_path_ms(250)
            .min_security_controls_per_service(1);
        assert_eq!(config.max_critical_path_ms, 250);
        assert_eq!(config.min_security_controls_per_service, 1);
    }

    #[test]
    fn suppression_is_by_exact_id() {
        let config = ValidatorConfig::new().allowing("no-isolated-nodes");
        assert!(config.is_allowed("no-isolated-nodes"));
        assert!(
            !config.is_allowed("no-isolated"),
            "must not be a prefix match"
        );
        assert!(!config.is_allowed("no-dependency-cycles"));
    }

    #[test]
    fn suppressions_are_deduplicated() {
        let config = ValidatorConfig::new()
            .allowing("a")
            .allowing("a")
            .allowing("b");
        assert_eq!(config.allow, ["a", "b"]);
    }

    #[test]
    fn config_round_trips_through_json_with_kebab_case_keys() {
        let config = ValidatorConfig::new()
            .max_critical_path_ms(500)
            .allowing("no-isolated-nodes");
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("max-critical-path-ms"), "{json}");

        let back: ValidatorConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, back);
    }

    #[test]
    fn omitted_fields_fall_back_to_defaults() {
        let config: ValidatorConfig =
            serde_json::from_str("{\"max-critical-path-ms\": 42}").unwrap();
        assert_eq!(config.max_critical_path_ms, 42);
        assert_eq!(
            config.min_security_controls_per_service,
            DEFAULT_MIN_SECURITY_CONTROLS
        );
    }
}
