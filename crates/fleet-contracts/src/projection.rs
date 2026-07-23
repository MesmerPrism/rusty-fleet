// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    CapabilitySnapshot, ConditionFamily, ContractViolation, DeviceIdentity, FleetQuery,
    OperationLedger, StatusCondition, StreamDescriptor, ValidateContract, finish, require_nonempty,
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
        if self.columns.len() > 64 || self.restoration.collapsed_groups.len() > 512 {
            failures.push(ContractViolation::new(
                "saved_view_too_large",
                "saved_view",
                "saved-view column and collapsed-group limits were exceeded",
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
            failures.append(&mut nested);
        }
        finish(failures)
    }
}
