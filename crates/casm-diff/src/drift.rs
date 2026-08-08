//! Module: `casm_diff::drift`
//! Purpose: Comparing a declared architecture against infrastructure that actually exists.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why drift is the hard half
//!
//! Every other check in CASM asks whether an architecture is *internally* coherent.
//! This one asks whether it is *true*. An architecture that nobody has compared against
//! running infrastructure is a diagram, and diagrams are wrong within a quarter.
//!
//! # Binding is explicit, with a convenience fallback
//!
//! The naïve approach — match a node to a resource when their names are equal — fails
//! immediately in practice. A node called `orders-db` is an `aws_db_instance` called
//! `orders`, or `primary`, or `main`. Reporting that as drift on every run would train
//! users to ignore the command.
//!
//! So a node may declare its binding:
//!
//! ```yaml
//! - name: orders-db
//!   type: database
//!   metadata:
//!     infrastructure-id: aws_db_instance.orders
//! ```
//!
//! Name equality remains as a fallback for the easy case. Anything CASM cannot bind is
//! reported rather than guessed at — a false "this is fine" is far more dangerous here
//! than a false alarm.
//!
//! # What a mismatch means
//!
//! A type mismatch is only reported when the observed resource's type is one CASM
//! recognises. An unmapped Terraform resource type asserts nothing about the node's type,
//! because the alternative is inventing a disagreement out of ignorance.

use casm_core::{Architecture, Node, NodeType};
use core::fmt::Write as _;
use serde::{Deserialize, Serialize};

/// The metadata key a node uses to bind itself to a real resource.
pub const BINDING_KEY: &str = "infrastructure-id";

/// A resource observed in real infrastructure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Resource {
    /// A stable identifier, such as `aws_db_instance.orders`.
    pub id: String,
    /// The human-facing name, used for the fallback binding.
    pub name: String,
    /// The CASM node type this resource corresponds to, when it is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_type: Option<NodeType>,
    /// The provider's own type string, kept for diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
}

/// A snapshot of what actually exists.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Inventory {
    /// Where the snapshot came from, for the report header.
    #[serde(default = "default_source")]
    pub source: String,
    /// Everything observed.
    #[serde(default)]
    pub resources: Vec<Resource>,
}

/// The inventory source used when a document does not name one.
fn default_source() -> String {
    "inventory".to_owned()
}

impl Inventory {
    /// Parses a native CASM inventory document.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message if the JSON does not match the schema.
    pub fn from_json(source: &str) -> Result<Self, String> {
        serde_json::from_str(source).map_err(|error| format!("invalid inventory: {error}"))
    }

    /// Projects a Terraform state file into an inventory.
    ///
    /// Only managed resources are considered: a `data` block describes something
    /// Terraform reads rather than something it owns, so its absence is not drift.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message if the state is not valid JSON.
    pub fn from_terraform_state(source: &str) -> Result<Self, String> {
        let state: serde_json::Value = serde_json::from_str(source)
            .map_err(|error| format!("invalid Terraform state: {error}"))?;

        let resources = state
            .get("resources")
            .and_then(serde_json::Value::as_array)
            .map(|entries| entries.iter().filter_map(terraform_resource).collect())
            .unwrap_or_default();

        Ok(Self {
            source: "terraform".to_owned(),
            resources,
        })
    }

    /// Finds the resource bound to `node`, if any.
    ///
    /// Prefers an explicit `infrastructure-id`; falls back to name equality.
    #[must_use]
    pub fn bound_to(&self, node: &Node) -> Option<&Resource> {
        if let Some(declared) = node.metadata().get(BINDING_KEY) {
            return self
                .resources
                .iter()
                .find(|resource| &resource.id == declared);
        }
        self.resources
            .iter()
            .find(|resource| resource.name == node.name().as_str())
    }
}

/// Converts one Terraform state resource entry.
fn terraform_resource(entry: &serde_json::Value) -> Option<Resource> {
    if entry.get("mode").and_then(serde_json::Value::as_str) != Some("managed") {
        return None;
    }

    let provider_type = entry.get("type").and_then(serde_json::Value::as_str)?;
    let name = entry.get("name").and_then(serde_json::Value::as_str)?;

    Some(Resource {
        id: format!("{provider_type}.{name}"),
        name: name.to_owned(),
        node_type: terraform_type(provider_type),
        provider_type: Some(provider_type.to_owned()),
    })
}

/// Maps a Terraform resource type onto a CASM node type.
///
/// Deliberately incomplete. An unmapped type yields `None`, which suppresses type-mismatch
/// reporting for that resource — CASM would rather say nothing than invent a
/// disagreement from a type it does not understand.
#[must_use]
pub fn terraform_type(provider_type: &str) -> Option<NodeType> {
    match provider_type {
        "aws_db_instance"
        | "aws_rds_cluster"
        | "aws_dynamodb_table"
        | "google_sql_database_instance"
        | "google_spanner_database"
        | "azurerm_postgresql_server"
        | "azurerm_cosmosdb_account" => Some(NodeType::Database),

        "aws_s3_bucket"
        | "aws_efs_file_system"
        | "google_storage_bucket"
        | "azurerm_storage_account" => Some(NodeType::Storage),

        "aws_sqs_queue"
        | "aws_sns_topic"
        | "aws_msk_cluster"
        | "google_pubsub_topic"
        | "google_pubsub_subscription"
        | "azurerm_servicebus_queue" => Some(NodeType::Queue),

        "aws_elasticache_cluster"
        | "aws_elasticache_replication_group"
        | "google_redis_instance"
        | "azurerm_redis_cache" => Some(NodeType::Cache),

        "aws_lb"
        | "aws_alb"
        | "aws_api_gateway_rest_api"
        | "aws_apigatewayv2_api"
        | "aws_cloudfront_distribution"
        | "google_compute_global_forwarding_rule"
        | "azurerm_application_gateway" => Some(NodeType::Gateway),

        "aws_ecs_service"
        | "aws_lambda_function"
        | "aws_instance"
        | "google_cloud_run_service"
        | "google_cloudfunctions_function"
        | "kubernetes_deployment"
        | "azurerm_linux_web_app" => Some(NodeType::Service),

        _ => None,
    }
}

/// One way the declared architecture and reality disagree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "drift")]
// Not `#[non_exhaustive]`, per ADR-0005.
pub enum Drift {
    /// The architecture declares a node that the inventory does not contain.
    Missing {
        /// The node's name.
        node: String,
        /// The node's declared type.
        node_type: String,
    },
    /// The inventory contains a resource the architecture does not declare.
    Unexpected {
        /// The resource's identifier.
        resource: String,
        /// The node type it appears to be, when recognised.
        node_type: Option<String>,
    },
    /// A node and its resource are bound, but disagree about what they are.
    TypeMismatch {
        /// The node's name.
        node: String,
        /// What the architecture says.
        declared: String,
        /// What the infrastructure says.
        observed: String,
    },
}

impl Drift {
    /// Returns `true` if this finding means the architecture is describing something
    /// that does not exist.
    ///
    /// The dangerous direction: an unexpected resource is usually something nobody
    /// documented, whereas a missing node means a decision was written down and never
    /// built, or was quietly torn down.
    #[must_use]
    pub const fn is_missing(&self) -> bool {
        matches!(self, Self::Missing { .. })
    }
}

impl core::fmt::Display for Drift {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missing { node, node_type } => write!(
                f,
                "node '{node}' ({node_type}) is declared but was not found in the inventory"
            ),
            Self::Unexpected {
                resource,
                node_type,
            } => {
                let described = node_type.as_deref().unwrap_or("unrecognised type");
                write!(
                    f,
                    "resource '{resource}' ({described}) exists but is not declared"
                )
            }
            Self::TypeMismatch {
                node,
                declared,
                observed,
            } => write!(
                f,
                "node '{node}' is declared as {declared} but the infrastructure says {observed}"
            ),
        }
    }
}

/// The result of comparing an architecture against an inventory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DriftReport {
    /// Where the inventory came from.
    pub source: String,
    /// Every disagreement found.
    pub drifts: Vec<Drift>,
    /// How many nodes matched a resource cleanly.
    pub matched: usize,
}

impl DriftReport {
    /// Returns `true` if the architecture and the inventory agree.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.drifts.is_empty()
    }

    /// A one-line summary.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.is_clean() {
            return format!(
                "no drift against {}: {} node(s) matched",
                self.source, self.matched
            );
        }
        format!(
            "{} drift(s) against {}: {} node(s) matched",
            self.drifts.len(),
            self.source,
            self.matched
        )
    }

    /// Renders every finding, one per line.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for drift in &self.drifts {
            // Writing to a `String` cannot fail; discarded deliberately, per NASA Rule 3.
            let _ = writeln!(out, "~ {drift}");
        }
        out
    }
}

/// Compares a declared architecture against an observed inventory.
///
/// External nodes and humans are exempt: `partner-bank` and `customer` are outside the
/// control boundary by definition, so expecting them in your own Terraform state would
/// report drift on every run.
#[must_use]
pub fn detect(architecture: &Architecture, inventory: &Inventory) -> DriftReport {
    let mut drifts = Vec::new();
    let mut matched = 0_usize;
    let mut bound_ids: Vec<&str> = Vec::new();

    for node in architecture.nodes() {
        if node.node_type().is_external() || node.node_type() == NodeType::Boundary {
            continue;
        }

        let Some(resource) = inventory.bound_to(node) else {
            drifts.push(Drift::Missing {
                node: node.name().as_str().to_owned(),
                node_type: node.node_type().to_string(),
            });
            continue;
        };

        bound_ids.push(&resource.id);
        matched = matched.saturating_add(1);

        if let Some(observed) = resource.node_type
            && observed != node.node_type()
        {
            drifts.push(Drift::TypeMismatch {
                node: node.name().as_str().to_owned(),
                declared: node.node_type().to_string(),
                observed: observed.to_string(),
            });
        }
    }

    for resource in &inventory.resources {
        if !bound_ids.contains(&resource.id.as_str()) {
            drifts.push(Drift::Unexpected {
                resource: resource.id.clone(),
                node_type: resource.node_type.map(|kind| kind.to_string()),
            });
        }
    }

    DriftReport {
        source: inventory.source.clone(),
        drifts,
        matched,
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
    use std::path::Path;

    fn parse(source: &str) -> Architecture {
        casm_parser::parse_str(source, Path::new("test.yaml")).expect("fixture parses")
    }

    const ARCHITECTURE: &str = "\
name: checkout
nodes:
  - name: api
    type: service
  - name: orders-db
    type: database
";

    fn inventory(resources: Vec<Resource>) -> Inventory {
        Inventory {
            source: "test".to_owned(),
            resources,
        }
    }

    fn resource(id: &str, name: &str, node_type: Option<NodeType>) -> Resource {
        Resource {
            id: id.to_owned(),
            name: name.to_owned(),
            node_type,
            provider_type: None,
        }
    }

    #[test]
    fn a_matching_inventory_reports_no_drift() {
        let report = detect(
            &parse(ARCHITECTURE),
            &inventory(vec![
                resource("api", "api", Some(NodeType::Service)),
                resource("orders-db", "orders-db", Some(NodeType::Database)),
            ]),
        );

        assert!(report.is_clean(), "{}", report.render());
        assert_eq!(report.matched, 2);
    }

    #[test]
    fn a_declared_node_with_no_resource_is_missing() {
        let report = detect(
            &parse(ARCHITECTURE),
            &inventory(vec![resource("api", "api", Some(NodeType::Service))]),
        );

        assert_eq!(report.drifts.len(), 1);
        assert!(report.drifts[0].is_missing());
        assert!(
            report.drifts[0].to_string().contains("orders-db"),
            "{}",
            report.render()
        );
    }

    #[test]
    fn an_undeclared_resource_is_unexpected() {
        let report = detect(
            &parse(ARCHITECTURE),
            &inventory(vec![
                resource("api", "api", Some(NodeType::Service)),
                resource("orders-db", "orders-db", Some(NodeType::Database)),
                resource("aws_s3_bucket.dumps", "dumps", Some(NodeType::Storage)),
            ]),
        );

        assert_eq!(report.drifts.len(), 1);
        assert!(!report.drifts[0].is_missing());
        assert!(
            report.drifts[0].to_string().contains("dumps"),
            "{}",
            report.render()
        );
    }

    #[test]
    fn a_bound_resource_of_the_wrong_type_is_a_mismatch() {
        let report = detect(
            &parse(ARCHITECTURE),
            &inventory(vec![
                resource("api", "api", Some(NodeType::Service)),
                resource("orders-db", "orders-db", Some(NodeType::Storage)),
            ]),
        );

        assert_eq!(report.drifts.len(), 1, "{}", report.render());
        assert!(matches!(report.drifts[0], Drift::TypeMismatch { .. }));
        assert_eq!(report.matched, 2, "a mismatch is still a match");
    }

    #[test]
    fn an_unrecognised_resource_type_asserts_nothing_about_the_node() {
        // The alternative — assuming a disagreement — would report drift for every
        // resource type CASM has not been taught.
        let report = detect(
            &parse(ARCHITECTURE),
            &inventory(vec![
                resource("api", "api", None),
                resource("orders-db", "orders-db", None),
            ]),
        );
        assert!(report.is_clean(), "{}", report.render());
    }

    #[test]
    fn an_explicit_binding_overrides_name_matching() {
        // The realistic case: the node and the resource are named differently.
        let architecture = parse(
            "\
name: checkout
nodes:
  - name: orders-db
    type: database
    metadata:
      infrastructure-id: aws_db_instance.primary
",
        );
        let report = detect(
            &architecture,
            &inventory(vec![resource(
                "aws_db_instance.primary",
                "primary",
                Some(NodeType::Database),
            )]),
        );

        assert!(report.is_clean(), "{}", report.render());
        assert_eq!(report.matched, 1);
    }

    #[test]
    fn an_explicit_binding_that_matches_nothing_is_missing_despite_a_name_collision() {
        // A declared binding is a statement of intent; silently falling back to the name
        // would hide that the resource it names is gone.
        let architecture = parse(
            "\
name: checkout
nodes:
  - name: orders-db
    type: database
    metadata:
      infrastructure-id: aws_db_instance.primary
",
        );
        let report = detect(
            &architecture,
            &inventory(vec![resource(
                "orders-db",
                "orders-db",
                Some(NodeType::Database),
            )]),
        );

        assert!(
            report.drifts.iter().any(Drift::is_missing),
            "{}",
            report.render()
        );
    }

    #[test]
    fn external_nodes_are_exempt() {
        // A partner's systems are not in your Terraform state, and never will be.
        let architecture = parse(
            "\
name: checkout
nodes:
  - name: partner-bank
    type: external-system
  - name: customer
    type: human
  - name: api
    type: service
",
        );
        let report = detect(
            &architecture,
            &inventory(vec![resource("api", "api", Some(NodeType::Service))]),
        );

        assert!(report.is_clean(), "{}", report.render());
        assert_eq!(report.matched, 1);
    }

    #[test]
    fn an_empty_inventory_reports_every_node_as_missing() {
        let report = detect(&parse(ARCHITECTURE), &inventory(Vec::new()));
        assert_eq!(report.drifts.len(), 2);
        assert!(report.drifts.iter().all(Drift::is_missing));
    }

    #[test]
    fn an_empty_architecture_reports_every_resource_as_unexpected() {
        let report = detect(
            &parse("name: empty\n"),
            &inventory(vec![resource("api", "api", None)]),
        );
        assert_eq!(report.drifts.len(), 1);
        assert!(!report.drifts[0].is_missing());
    }

    #[test]
    fn a_native_inventory_parses_from_json() {
        let json = r#"{
            "source": "manual",
            "resources": [
                { "id": "aws_db_instance.primary", "name": "primary", "node-type": "database" }
            ]
        }"#;
        let parsed = Inventory::from_json(json).unwrap();

        assert_eq!(parsed.source, "manual");
        assert_eq!(parsed.resources.len(), 1);
        assert_eq!(parsed.resources[0].node_type, Some(NodeType::Database));
    }

    #[test]
    fn a_malformed_inventory_is_rejected_with_a_message() {
        let error = Inventory::from_json("{ not json").unwrap_err();
        assert!(error.contains("invalid inventory"), "{error}");
    }

    #[test]
    fn terraform_state_projects_into_an_inventory() {
        let state = r#"{
            "version": 4,
            "resources": [
                { "mode": "managed", "type": "aws_db_instance", "name": "primary",
                  "instances": [{}] },
                { "mode": "managed", "type": "aws_s3_bucket", "name": "assets",
                  "instances": [{}] },
                { "mode": "data", "type": "aws_ami", "name": "ubuntu", "instances": [{}] }
            ]
        }"#;
        let parsed = Inventory::from_terraform_state(state).unwrap();

        assert_eq!(parsed.source, "terraform");
        assert_eq!(
            parsed.resources.len(),
            2,
            "the data source is not owned infrastructure"
        );
        assert_eq!(parsed.resources[0].id, "aws_db_instance.primary");
        assert_eq!(parsed.resources[0].node_type, Some(NodeType::Database));
        assert_eq!(parsed.resources[1].node_type, Some(NodeType::Storage));
    }

    #[test]
    fn terraform_state_with_no_resources_is_an_empty_inventory() {
        assert!(
            Inventory::from_terraform_state(r#"{"version":4}"#)
                .unwrap()
                .resources
                .is_empty()
        );
    }

    #[test]
    fn malformed_terraform_state_is_rejected() {
        assert!(Inventory::from_terraform_state("nope").is_err());
    }

    #[test]
    fn the_terraform_type_map_covers_each_node_type_it_claims_to() {
        let cases = [
            ("aws_db_instance", NodeType::Database),
            ("google_storage_bucket", NodeType::Storage),
            ("aws_sqs_queue", NodeType::Queue),
            ("aws_elasticache_cluster", NodeType::Cache),
            ("aws_alb", NodeType::Gateway),
            ("aws_lambda_function", NodeType::Service),
        ];
        for (provider, expected) in cases {
            assert_eq!(terraform_type(provider), Some(expected), "{provider}");
        }
        assert_eq!(
            terraform_type("aws_something_new"),
            None,
            "unmapped means unknown"
        );
    }

    #[test]
    fn a_report_summarises_itself_in_both_states() {
        let clean = detect(
            &parse(ARCHITECTURE),
            &inventory(vec![
                resource("api", "api", None),
                resource("orders-db", "orders-db", None),
            ]),
        );
        assert!(clean.summary().contains("no drift"), "{}", clean.summary());

        let dirty = detect(&parse(ARCHITECTURE), &inventory(Vec::new()));
        assert!(
            dirty.summary().contains("2 drift(s)"),
            "{}",
            dirty.summary()
        );
    }

    #[test]
    fn a_report_round_trips_through_json() {
        let report = detect(&parse(ARCHITECTURE), &inventory(Vec::new()));
        let json = serde_json::to_string(&report).unwrap();
        assert_eq!(serde_json::from_str::<DriftReport>(&json).unwrap(), report);
    }

    #[test]
    fn detection_is_deterministic() {
        let architecture = parse(ARCHITECTURE);
        let observed = inventory(vec![resource("api", "api", None)]);
        assert_eq!(
            detect(&architecture, &observed),
            detect(&architecture, &observed)
        );
    }
}
