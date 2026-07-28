// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize};

use crate::{CommandLifecycle, ContractViolation, ValidateContract, finish, require_nonempty};

pub const WINDOWS_HOTSPOT_ACTION_ID: &str = "host.windows-mobile-hotspot";
pub const WINDOWS_HOTSPOT_RESOURCE_ID: &str = "windows.mobile-hotspot";
pub const WINDOWS_HOTSPOT_OWNER: &str = "rusty-hostess";
pub const WINDOWS_HOTSPOT_PROVIDER_FILE: &str = "rusty-hostess-hotspot-provider.exe";
pub const WINDOWS_HOTSPOT_PROVIDER_REQUEST_SCHEMA: &str =
    "rusty.hostess.windows_hotspot.provider_request.v1";
pub const WINDOWS_HOTSPOT_PROVIDER_RECEIPT_SCHEMA: &str =
    "rusty.hostess.windows_hotspot.provider_receipt.v1";
pub const WINDOWS_HOTSPOT_PREVIEW_REQUEST_SCHEMA: &str =
    "rusty.fleet.windows_hotspot_preview_request.v1";
pub const WINDOWS_HOTSPOT_EXECUTE_REQUEST_SCHEMA: &str =
    "rusty.fleet.windows_hotspot_execute_request.v1";
pub const WINDOWS_HOTSPOT_OPERATION_SCHEMA: &str = "rusty.fleet.windows_hotspot_operation.v1";
pub const WINDOWS_HOTSPOT_LEASE_SCHEMA: &str = "rusty.fleet.host_resource_lease.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsHotspotAction {
    Status,
    Start,
    Ensure,
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsHotspotOwnership {
    None,
    Fleet,
    External,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsHotspotResult {
    Verified,
    Failed,
    Rejected,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsHotspotPreviewRequest {
    pub schema: String,
    pub action_id: String,
    pub action: WindowsHotspotAction,
}

impl ValidateContract for WindowsHotspotPreviewRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != WINDOWS_HOTSPOT_PREVIEW_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "unsupported schema",
            ));
        }
        if self.action_id != WINDOWS_HOTSPOT_ACTION_ID {
            failures.push(ContractViolation::new(
                "wrong_action_id",
                "action_id",
                "unsupported action",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsHotspotExecuteRequest {
    pub schema: String,
    pub operation_id: String,
    pub preview_id: String,
}

impl ValidateContract for WindowsHotspotExecuteRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != WINDOWS_HOTSPOT_EXECUTE_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "unsupported schema",
            ));
        }
        require_nonempty(&mut failures, &self.operation_id, "operation_id");
        require_nonempty(&mut failures, &self.preview_id, "preview_id");
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsHotspotPreflight {
    pub provider_ready: bool,
    pub lease_available: bool,
    pub active: bool,
    pub ownership: WindowsHotspotOwnership,
    pub ownership_generation: Option<String>,
    pub eligible: bool,
    pub reason_code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsHotspotPreview {
    pub schema: String,
    pub preview_id: String,
    pub operation_id: String,
    pub action_id: String,
    pub resource_id: String,
    pub owner_id: String,
    pub action: WindowsHotspotAction,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub fleet_revision: u64,
    pub preflight: WindowsHotspotPreflight,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsHotspotLease {
    pub schema: String,
    pub lease_id: String,
    pub resource_id: String,
    pub holder_operation_id: String,
    pub generation: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsHotspotProviderRequest {
    pub schema: String,
    pub request_id: String,
    pub operation_id: String,
    pub action: WindowsHotspotAction,
    pub expires_at_utc: String,
    pub timeout_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ownership_generation: Option<String>,
}

impl ValidateContract for WindowsHotspotProviderRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != WINDOWS_HOTSPOT_PROVIDER_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "unsupported schema",
            ));
        }
        for (value, path) in [
            (&self.request_id, "request_id"),
            (&self.operation_id, "operation_id"),
            (&self.expires_at_utc, "expires_at_utc"),
        ] {
            require_nonempty(&mut failures, value, path);
            if value.len() > 128 {
                failures.push(ContractViolation::new(
                    "identifier_too_long",
                    path,
                    "Hostess provider identifiers are bounded to 128 bytes",
                ));
            }
        }
        if !self.expires_at_utc.ends_with('Z') || !(100..=120_000).contains(&self.timeout_ms) {
            failures.push(ContractViolation::new(
                "invalid_deadline",
                "expires_at_utc",
                "provider deadline must be UTC and timeout must be bounded",
            ));
        }
        match self.action {
            WindowsHotspotAction::Stop => {
                if self
                    .ownership_generation
                    .as_deref()
                    .is_none_or(str::is_empty)
                {
                    failures.push(ContractViolation::new(
                        "generation_binding_invalid",
                        "ownership_generation",
                        "stop requires the exact Hostess ownership generation",
                    ));
                }
            }
            WindowsHotspotAction::Status | WindowsHotspotAction::Start => {
                if self.ownership_generation.is_some() {
                    failures.push(ContractViolation::new(
                        "generation_binding_invalid",
                        "ownership_generation",
                        "status and start cannot supply an ownership generation",
                    ));
                }
            }
            WindowsHotspotAction::Ensure => {}
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsHotspotProviderReceipt {
    pub schema: String,
    pub request_id: String,
    pub operation_id: String,
    pub action: WindowsHotspotAction,
    pub outcome: WindowsHotspotResult,
    pub reason: String,
    pub observed_at_utc: String,
    pub capability_available: bool,
    pub capability: String,
    pub operational_state: String,
    pub client_count: u32,
    pub max_client_count: u32,
    pub band: String,
    pub source_connectivity: String,
    pub ownership_generation: Option<String>,
}

impl ValidateContract for WindowsHotspotProviderReceipt {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != WINDOWS_HOTSPOT_PROVIDER_RECEIPT_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "unsupported schema",
            ));
        }
        for (value, path) in [
            (&self.request_id, "request_id"),
            (&self.operation_id, "operation_id"),
            (&self.reason, "reason"),
            (&self.observed_at_utc, "observed_at_utc"),
            (&self.capability, "capability"),
            (&self.operational_state, "operational_state"),
            (&self.band, "band"),
            (&self.source_connectivity, "source_connectivity"),
        ] {
            require_nonempty(&mut failures, value, path);
        }
        if self.request_id.len() > 128 || self.operation_id.len() > 128 {
            failures.push(ContractViolation::new(
                "identifier_too_long",
                "request_id",
                "Hostess provider identifiers are bounded to 128 bytes",
            ));
        }
        if !self.observed_at_utc.ends_with('Z') && !self.observed_at_utc.contains("+00:00") {
            failures.push(ContractViolation::new(
                "invalid_timestamp",
                "observed_at_utc",
                "provider receipt timestamp must be UTC",
            ));
        }
        if self.client_count > self.max_client_count && self.max_client_count != 0 {
            failures.push(ContractViolation::new(
                "invalid_client_count",
                "client_count",
                "client count cannot exceed the reported maximum",
            ));
        }
        if self
            .ownership_generation
            .as_ref()
            .is_some_and(|value| value.is_empty())
        {
            failures.push(ContractViolation::new(
                "generation_invalid",
                "ownership_generation",
                "ownership generation must not be empty",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsHotspotOperation {
    pub schema: String,
    pub operation_id: String,
    pub action_id: String,
    pub lifecycle: CommandLifecycle,
    pub preview: WindowsHotspotPreview,
    pub confirmed_at_ms: Option<i64>,
    pub lease: Option<WindowsHotspotLease>,
    pub invocation: Option<WindowsHotspotProviderRequest>,
    pub receipt: Option<WindowsHotspotProviderReceipt>,
    pub failure_code: Option<String>,
    pub updated_at_ms: i64,
}

impl ValidateContract for WindowsHotspotOperation {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != WINDOWS_HOTSPOT_OPERATION_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "unsupported schema",
            ));
        }
        if self.action_id != WINDOWS_HOTSPOT_ACTION_ID
            || self.operation_id != self.preview.operation_id
            || self.action_id != self.preview.action_id
            || self.preview.resource_id != WINDOWS_HOTSPOT_RESOURCE_ID
            || self.preview.owner_id != WINDOWS_HOTSPOT_OWNER
        {
            failures.push(ContractViolation::new(
                "operation_binding_mismatch",
                "operation_id",
                "operation does not bind its exact immutable preview",
            ));
        }
        if self.preview.expires_at_ms <= self.preview.created_at_ms {
            failures.push(ContractViolation::new(
                "invalid_expiry",
                "preview.expires_at_ms",
                "preview expiry must follow creation",
            ));
        }
        if self.preview.schema != "rusty.fleet.windows_hotspot_preview.v1"
            || self.preview.action_id != WINDOWS_HOTSPOT_ACTION_ID
            || self.preview.resource_id != WINDOWS_HOTSPOT_RESOURCE_ID
            || self.preview.owner_id != WINDOWS_HOTSPOT_OWNER
            || self.preview.fleet_revision == 0
        {
            failures.push(ContractViolation::new(
                "preview_invalid",
                "preview",
                "preview authority and revision are invalid",
            ));
        }
        if let Some(lease) = &self.lease
            && (lease.schema != WINDOWS_HOTSPOT_LEASE_SCHEMA
                || lease.resource_id != WINDOWS_HOTSPOT_RESOURCE_ID
                || lease.holder_operation_id != self.operation_id
                || lease.lease_id.is_empty()
                || lease.generation.is_empty()
                || lease.expires_at_ms <= lease.issued_at_ms
                || lease.expires_at_ms != self.preview.expires_at_ms)
        {
            failures.push(ContractViolation::new(
                "lease_binding_mismatch",
                "lease",
                "host lease does not bind the operation, resource, generation, and preview expiry",
            ));
        }
        if let Some(invocation) = &self.invocation {
            if let Err(mut nested) = invocation.validate() {
                failures.append(&mut nested);
            }
            if invocation.operation_id != self.operation_id
                || invocation.action != self.preview.action
            {
                failures.push(ContractViolation::new(
                    "invocation_binding_mismatch",
                    "invocation",
                    "invocation does not bind operation and action",
                ));
            }
        }
        if let Some(receipt) = &self.receipt {
            if let Err(mut nested) = receipt.validate() {
                failures.append(&mut nested);
            }
            if receipt.operation_id != self.operation_id
                || receipt.action != self.preview.action
                || self
                    .invocation
                    .as_ref()
                    .is_none_or(|request| request.request_id != receipt.request_id)
            {
                failures.push(ContractViolation::new(
                    "receipt_binding_mismatch",
                    "receipt",
                    "receipt does not bind the exact invocation",
                ));
            }
        }
        if matches!(
            self.lifecycle,
            CommandLifecycle::Accepted | CommandLifecycle::Dispatched
        ) && self.confirmed_at_ms.is_none()
        {
            failures.push(ContractViolation::new(
                "confirmation_missing",
                "confirmed_at_ms",
                "accepted and dispatched operations require explicit confirmation",
            ));
        }
        if self.lifecycle == CommandLifecycle::Dispatched && self.invocation.is_none() {
            failures.push(ContractViolation::new(
                "invocation_missing",
                "invocation",
                "dispatched operation requires an exact provider invocation",
            ));
        }
        finish(failures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        action: WindowsHotspotAction,
        generation: Option<&str>,
    ) -> WindowsHotspotProviderRequest {
        WindowsHotspotProviderRequest {
            schema: WINDOWS_HOTSPOT_PROVIDER_REQUEST_SCHEMA.to_owned(),
            request_id: "request.1".to_owned(),
            operation_id: "operation.1".to_owned(),
            action,
            expires_at_utc: "2026-07-27T12:00:00.0000000Z".to_owned(),
            timeout_ms: 30_000,
            ownership_generation: generation.map(str::to_owned),
        }
    }

    #[test]
    fn provider_requests_omit_absent_generation_and_use_only_exact_hostess_fields() {
        for (action, generation) in [
            (WindowsHotspotAction::Status, None),
            (WindowsHotspotAction::Start, None),
            (WindowsHotspotAction::Ensure, None),
            (WindowsHotspotAction::Ensure, Some("generation.1")),
            (WindowsHotspotAction::Stop, Some("generation.1")),
        ] {
            let request = request(action, generation);
            request.validate().expect("valid request");
            let object = serde_json::to_value(request)
                .expect("request JSON")
                .as_object()
                .expect("request object")
                .clone();
            assert_eq!(
                object
                    .keys()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>(),
                [
                    "schema",
                    "request_id",
                    "operation_id",
                    "action",
                    "expires_at_utc",
                    "timeout_ms",
                ]
                .into_iter()
                .chain(generation.map(|_| "ownership_generation"))
                .map(str::to_owned)
                .collect()
            );
        }
    }
}
