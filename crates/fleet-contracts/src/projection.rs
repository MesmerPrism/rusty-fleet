// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ApplicationObservation, CapabilitySnapshot, ConditionFamily, ContractViolation, DeviceIdentity,
    FleetQuery, OperationLedger, PowerObservation, StatusCondition, StreamDescriptor,
    ValidateContract, finish, require_nonempty,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionFreshness {
    Fresh,
    Stale,
    Offline,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceRowProjection {
    pub schema: String,
    pub identity: DeviceIdentity,
    pub source_epoch: String,
    pub accepted_revision: u64,
    pub accepted_at_ms: i64,
    pub age_ms: i64,
    pub freshness: ProjectionFreshness,
    pub battery_percent: Option<u8>,
    pub charging: Option<bool>,
    pub foreground_app: Option<String>,
    pub agent: Option<ApplicationObservation>,
    pub power: Option<PowerObservation>,
    pub application: Option<ApplicationObservation>,
    pub kiosk_state: String,
    pub route: String,
    #[serde(default)]
    pub conditions: BTreeMap<ConditionFamily, StatusCondition>,
    pub capabilities: CapabilitySnapshot,
    pub stream_count: usize,
    pub active_work_count: usize,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl ValidateContract for DeviceRowProjection {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.device_row.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.device_row.v1",
            ));
        }
        if let Err(mut nested) = self.identity.validate() {
            failures.append(&mut nested);
        }
        require_nonempty(&mut failures, &self.source_epoch, "source_epoch");
        if self.accepted_revision == 0 || self.age_ms < 0 {
            failures.push(ContractViolation::new(
                "invalid_projection_revision_or_age",
                "accepted_revision",
                "accepted revision must be positive and age must be nonnegative",
            ));
        }
        if self.battery_percent.is_some_and(|percent| percent > 100) {
            failures.push(ContractViolation::new(
                "invalid_battery",
                "battery_percent",
                "battery percentage must be between 0 and 100",
            ));
        }
        if let Some(agent) = &self.agent
            && let Err(nested) = agent.validate()
        {
            failures.extend(nested.into_iter().map(|failure| ContractViolation {
                path: format!("agent.{}", failure.path),
                ..failure
            }));
        }
        if let Some(power) = &self.power
            && let Err(nested) = power.validate()
        {
            failures.extend(nested.into_iter().map(|failure| ContractViolation {
                path: format!("power.{}", failure.path),
                ..failure
            }));
        }
        if let Some(application) = &self.application
            && let Err(nested) = application.validate()
        {
            failures.extend(nested.into_iter().map(|failure| ContractViolation {
                path: format!("application.{}", failure.path),
                ..failure
            }));
        }
        if self.conditions.len() > 16 || self.extensions.len() > 64 {
            failures.push(ContractViolation::new(
                "row_projection_too_large",
                "conditions",
                "row projections support bounded condition and extension collections",
            ));
        }
        require_nonempty(&mut failures, &self.kiosk_state, "kiosk_state");
        require_nonempty(&mut failures, &self.route, "route");
        for (family, condition) in &self.conditions {
            if family != &condition.family {
                failures.push(ContractViolation::new(
                    "condition_key_mismatch",
                    &format!("conditions.{family:?}"),
                    "condition map key must match the condition family",
                ));
            }
            if let Err(nested) = condition.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("conditions.{family:?}.{}", failure.path),
                    ..failure
                }));
            }
        }
        if let Err(mut nested) = self.capabilities.validate() {
            failures.append(&mut nested);
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceInspectorProjection {
    pub schema: String,
    pub row: DeviceRowProjection,
    #[serde(default)]
    pub attention: Vec<StatusCondition>,
    #[serde(default)]
    pub streams: Vec<StreamDescriptor>,
    #[serde(default)]
    pub active_operations: Vec<OperationLedger>,
}

impl ValidateContract for DeviceInspectorProjection {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.device_inspector.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.device_inspector.v1",
            ));
        }
        if let Err(mut nested) = self.row.validate() {
            failures.append(&mut nested);
        }
        if self.attention.len() > 64
            || self.streams.len() > 32
            || self.active_operations.len() > 128
        {
            failures.push(ContractViolation::new(
                "inspector_projection_too_large",
                "inspector",
                "inspector collection limits were exceeded",
            ));
        }
        for (index, condition) in self.attention.iter().enumerate() {
            if let Err(nested) = condition.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("attention[{index}].{}", failure.path),
                    ..failure
                }));
            }
        }
        for (index, stream) in self.streams.iter().enumerate() {
            if let Err(nested) = stream.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("streams[{index}].{}", failure.path),
                    ..failure
                }));
            }
        }
        for (index, operation) in self.active_operations.iter().enumerate() {
            if let Err(nested) = operation.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("active_operations[{index}].{}", failure.path),
                    ..failure
                }));
            }
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceDetailProjection {
    pub schema: String,
    pub inspector: DeviceInspectorProjection,
    #[serde(default)]
    pub condition_history: Vec<StatusCondition>,
    #[serde(default)]
    pub operation_history: Vec<OperationLedger>,
}

impl ValidateContract for DeviceDetailProjection {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.device_detail.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.device_detail.v1",
            ));
        }
        if let Err(mut nested) = self.inspector.validate() {
            failures.append(&mut nested);
        }
        if self.condition_history.len() > 128 || self.operation_history.len() > 1_000 {
            failures.push(ContractViolation::new(
                "detail_projection_too_large",
                "detail",
                "detail history limits were exceeded",
            ));
        }
        for (index, condition) in self.condition_history.iter().enumerate() {
            if let Err(nested) = condition.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("condition_history[{index}].{}", failure.path),
                    ..failure
                }));
            }
        }
        for (index, operation) in self.operation_history.iter().enumerate() {
            if let Err(nested) = operation.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("operation_history[{index}].{}", failure.path),
                    ..failure
                }));
            }
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetSummaryProjection {
    pub schema: String,
    pub as_of_ms: i64,
    pub total: usize,
    pub fresh: usize,
    pub stale: usize,
    pub offline: usize,
    pub attention: usize,
    pub active_work: usize,
}

impl ValidateContract for FleetSummaryProjection {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.summary.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.summary.v1",
            ));
        }
        if self.fresh + self.stale + self.offline > self.total || self.attention > self.total {
            failures.push(ContractViolation::new(
                "invalid_summary_counts",
                "total",
                "summary category counts are inconsistent with the total",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FleetQueryResult {
    pub schema: String,
    pub query: FleetQuery,
    pub result_revision: u64,
    pub as_of_ms: i64,
    pub total_count: usize,
    pub window_offset: usize,
    pub window_count: usize,
    #[serde(default)]
    pub rows: Vec<DeviceRowProjection>,
}

impl ValidateContract for FleetQueryResult {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.query_result.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.query_result.v1",
            ));
        }
        if self.result_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_revision",
                "result_revision",
                "result revision must be greater than zero",
            ));
        }
        if self.window_count != self.rows.len()
            || self.window_offset.saturating_add(self.window_count) > self.total_count
        {
            failures.push(ContractViolation::new(
                "invalid_window",
                "rows",
                "query result window metadata does not match returned rows",
            ));
        }
        if let Err(mut nested) = self.query.validate() {
            failures.append(&mut nested);
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigationRestoration {
    pub selected_device_id: Option<String>,
    pub inspector_tab: Option<String>,
    pub scroll_anchor_device_id: Option<String>,
    #[serde(default)]
    pub focused_region: Option<String>,
    #[serde(default)]
    pub collapsed_groups: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedView {
    pub schema: String,
    pub view_id: String,
    pub name: String,
    pub query: FleetQuery,
    #[serde(default)]
    pub columns: Vec<String>,
    pub density: String,
    pub grouping: Option<String>,
    pub restoration: NavigationRestoration,
    pub schema_version: u32,
}

#[must_use]
pub fn is_valid_saved_view_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.split('.').all(|segment| {
            !segment.is_empty()
                && segment.chars().next().is_some_and(is_saved_view_id_edge)
                && segment.chars().last().is_some_and(is_saved_view_id_edge)
                && segment.chars().all(is_saved_view_id_body)
        })
}

fn is_saved_view_id_edge(value: char) -> bool {
    value.is_ascii_lowercase() || value.is_ascii_digit()
}

fn is_saved_view_id_body(value: char) -> bool {
    is_saved_view_id_edge(value) || value == '_' || value == '-'
}

impl ValidateContract for SavedView {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.saved_view.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.saved_view.v1",
            ));
        }
        require_nonempty(&mut failures, &self.view_id, "view_id");
        require_nonempty(&mut failures, &self.name, "name");
        require_nonempty(&mut failures, &self.density, "density");
        if !is_valid_saved_view_id(&self.view_id) {
            failures.push(ContractViolation::new(
                "invalid_saved_view_id",
                "view_id",
                "saved-view ID must use lowercase dotted-ID grammar",
            ));
        }
        if self.name.len() > 256
            || self.density.len() > 32
            || self
                .grouping
                .as_ref()
                .is_some_and(|value| value.len() > 128)
            || [
                self.restoration.selected_device_id.as_ref(),
                self.restoration.inspector_tab.as_ref(),
                self.restoration.scroll_anchor_device_id.as_ref(),
                self.restoration.focused_region.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(|value| value.len() > 128)
        {
            failures.push(ContractViolation::new(
                "saved_view_text_too_large",
                "saved_view",
                "saved-view identity, display, grouping, and restoration text is bounded",
            ));
        }
        if self
            .grouping
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
            || [
                self.restoration.selected_device_id.as_ref(),
                self.restoration.inspector_tab.as_ref(),
                self.restoration.scroll_anchor_device_id.as_ref(),
                self.restoration.focused_region.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(|value| value.trim().is_empty())
        {
            failures.push(ContractViolation::new(
                "invalid_saved_view_text",
                "saved_view",
                "optional saved-view grouping and restoration text must be nonempty when present",
            ));
        }
        if self.columns.len() > 64 || self.restoration.collapsed_groups.len() > 512 {
            failures.push(ContractViolation::new(
                "saved_view_too_large",
                "saved_view",
                "saved-view column and collapsed-group limits were exceeded",
            ));
        }
        if self
            .columns
            .iter()
            .any(|value| value.trim().is_empty() || value.len() > 128)
            || self
                .restoration
                .collapsed_groups
                .iter()
                .any(|value| value.trim().is_empty() || value.len() > 128)
        {
            failures.push(ContractViolation::new(
                "invalid_saved_view_item",
                "saved_view",
                "saved-view columns and collapsed groups must contain bounded nonempty text",
            ));
        }
        if self.columns.iter().collect::<BTreeSet<_>>().len() != self.columns.len()
            || self
                .restoration
                .collapsed_groups
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != self.restoration.collapsed_groups.len()
        {
            failures.push(ContractViolation::new(
                "duplicate_saved_view_item",
                "saved_view",
                "saved-view columns and collapsed groups must be unique",
            ));
        }
        if !matches!(
            self.density.as_str(),
            "compact" | "standard" | "comfortable"
        ) {
            failures.push(ContractViolation::new(
                "invalid_saved_view_density",
                "density",
                "saved-view density must be compact, standard, or comfortable",
            ));
        }
        if self.schema_version == 0 {
            failures.push(ContractViolation::new(
                "invalid_schema_version",
                "schema_version",
                "schema version must be greater than zero",
            ));
        }
        if let Err(mut nested) = self.query.validate() {
            for failure in &mut nested {
                failure.path = format!("query.{}", failure.path);
            }
            failures.append(&mut nested);
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedViewCollection {
    pub schema: String,
    pub revision: u64,
    pub views: Vec<SavedView>,
}

impl ValidateContract for SavedViewCollection {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.saved_view_collection.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.saved_view_collection.v1",
            ));
        }
        if self.revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_revision",
                "revision",
                "saved-view collection revision must be greater than zero",
            ));
        }
        if self.views.len() > 128 {
            failures.push(ContractViolation::new(
                "saved_view_collection_too_large",
                "views",
                "saved-view collections are limited to 128 entries",
            ));
        }
        let mut prior_id: Option<&str> = None;
        for (index, view) in self.views.iter().enumerate() {
            if let Err(mut nested) = view.validate() {
                for failure in &mut nested {
                    failure.path = format!("views[{index}].{}", failure.path);
                }
                failures.append(&mut nested);
            }
            if prior_id.is_some_and(|prior| prior >= view.view_id.as_str()) {
                failures.push(ContractViolation::new(
                    "saved_views_not_canonical",
                    "views",
                    "saved views must be unique and ordered by view_id",
                ));
            }
            prior_id = Some(&view.view_id);
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedViewMutationRequest {
    pub schema: String,
    pub expected_revision: u64,
    pub view: SavedView,
}

impl ValidateContract for SavedViewMutationRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.saved_view_mutation_request.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.saved_view_mutation_request.v1",
            ));
        }
        if self.expected_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_revision",
                "expected_revision",
                "expected saved-view revision must be greater than zero",
            ));
        }
        if let Err(mut nested) = self.view.validate() {
            for failure in &mut nested {
                failure.path = format!("view.{}", failure.path);
            }
            failures.append(&mut nested);
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedViewMutationReceipt {
    pub schema: String,
    pub view_id: String,
    pub previous_revision: u64,
    pub current_revision: u64,
    pub changed: bool,
    pub deleted: bool,
    pub view: Option<SavedView>,
}

impl ValidateContract for SavedViewMutationReceipt {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.saved_view_mutation_receipt.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.saved_view_mutation_receipt.v1",
            ));
        }
        require_nonempty(&mut failures, &self.view_id, "view_id");
        if !is_valid_saved_view_id(&self.view_id)
            || self.previous_revision == 0
            || self.current_revision < self.previous_revision
            || self.current_revision > self.previous_revision.saturating_add(1)
            || (self.changed && self.current_revision == self.previous_revision)
            || (!self.changed && self.current_revision != self.previous_revision)
            || (self.deleted && !self.changed)
            || self.deleted != self.view.is_none()
        {
            failures.push(ContractViolation::new(
                "invalid_saved_view_receipt",
                "receipt",
                "saved-view receipt revision, deletion, or payload state is inconsistent",
            ));
        }
        if let Some(view) = &self.view {
            if view.view_id != self.view_id {
                failures.push(ContractViolation::new(
                    "saved_view_identity_mismatch",
                    "view.view_id",
                    "receipt view identity must match view_id",
                ));
            }
            if let Err(mut nested) = view.validate() {
                for failure in &mut nested {
                    failure.path = format!("view.{}", failure.path);
                }
                failures.append(&mut nested);
            }
        }
        finish(failures)
    }
}
