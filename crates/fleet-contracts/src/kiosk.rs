// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    AuthorizationState, CommandLifecycle, ContractViolation, EnablementState, FreshnessState,
    ReachabilityState, SupportState, ValidateContract, finish, require_nonempty,
};

pub const KIOSK_SHOW_CONTROLS_ACTION_ID: &str = "kiosk.show-controls";
pub const KIOSK_DIRECT_OPERATOR_SCHEMA: &str = "rusty.kiosk.direct_operator.v1";
pub const KIOSK_DIRECT_OPERATOR_REVISION: &str = "8954228f9ae67c5995a72569e3c9cdd3758f85c0";
pub const KIOSK_DIRECT_OPERATOR_CAPABILITY_ID: &str = "rusty-kiosk.direct-operator";
pub const KIOSK_DIRECT_OPERATOR_OWNER: &str = "rusty-kiosk";
pub const KIOSK_DIRECT_OPERATOR_REQUEST_AUTH: &str = "hmac-sha256-v1";
pub const KIOSK_DIRECT_OPERATOR_RESPONSE_AUTH: &str = "hmac-sha256-response-v1";
pub const KIOSK_DIRECT_OPERATOR_INVOKE_METHOD: &str = "POST";
pub const KIOSK_DIRECT_OPERATOR_INVOKE_TARGET: &str = "/v1/kiosk/invoke";
pub const KIOSK_DIRECT_OPERATOR_RESULT_METHOD: &str = "GET";
pub const KIOSK_DIRECT_OPERATOR_RESULT_TARGET: &str = "/v1/kiosk/result";
pub const KIOSK_DIRECT_OPERATOR_RESULT_REQUEST_ID_PARAMETER: &str = "request_id";
pub const KIOSK_DIRECT_OPERATOR_PORT: u16 = 39_873;
pub const KIOSK_DIRECT_OPERATOR_MAX_CLOCK_SKEW_SECONDS: u16 = 90;
pub const KIOSK_SHOW_CONTROLS_COMMAND: &str = "show-controls";
pub const KIOSK_CLI_RESULT_SCHEMA: &str = "rusty.kiosk.cli_result.v1";
pub const OPERATION_PREVIEW_REQUEST_SCHEMA: &str = "rusty.fleet.operation_preview_request.v1";
pub const OPERATION_EXECUTE_REQUEST_SCHEMA: &str = "rusty.fleet.operation_execute_request.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationPreviewRequest {
    pub schema: String,
    pub action_id: String,
    pub targets: BTreeMap<String, u64>,
}

impl ValidateContract for OperationPreviewRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != OPERATION_PREVIEW_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.operation_preview_request.v1",
            ));
        }
        if self.action_id != KIOSK_SHOW_CONTROLS_ACTION_ID {
            failures.push(ContractViolation::new(
                "wrong_action",
                "action_id",
                "expected kiosk.show-controls",
            ));
        }
        if self.targets.is_empty() || self.targets.len() > 10_000 {
            failures.push(ContractViolation::new(
                "invalid_target_count",
                "targets",
                "operation previews require 1 through 10000 targets",
            ));
        }
        for (device_id, identity_revision) in &self.targets {
            if device_id.is_empty() || device_id.len() > 256 {
                failures.push(ContractViolation::new(
                    "invalid_device_id",
                    "targets",
                    "target device IDs contain 1 through 256 bytes",
                ));
            }
            if *identity_revision == 0 {
                failures.push(ContractViolation::new(
                    "invalid_identity_revision",
                    "targets",
                    "target identity revisions must be greater than zero",
                ));
            }
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationExecuteRequest {
    pub schema: String,
    pub operation_id: String,
    pub preview_id: String,
}

impl ValidateContract for OperationExecuteRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != OPERATION_EXECUTE_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.operation_execute_request.v1",
            ));
        }
        for (path, value) in [
            ("operation_id", self.operation_id.as_str()),
            ("preview_id", self.preview_id.as_str()),
        ] {
            if value.is_empty() || value.len() > 256 {
                failures.push(ContractViolation::new(
                    "invalid_identifier",
                    path,
                    "operation and preview IDs contain 1 through 256 bytes",
                ));
            }
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KioskOwnerContractBinding {
    pub owner_repo_id: String,
    pub owner_contract_schema: String,
    pub owner_contract_revision: String,
    pub capability_id: String,
    pub request_auth: String,
    pub response_auth: String,
    pub invoke_method: String,
    pub invoke_target: String,
    pub result_method: String,
    pub result_target: String,
    pub result_request_id_parameter: String,
    pub port: u16,
    pub max_clock_skew_seconds: u16,
    pub command: String,
    pub command_value: Option<String>,
    pub owner_result_schema: String,
}

impl ValidateContract for KioskOwnerContractBinding {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let expected = Self::show_controls_v1();
        let mut failures = Vec::new();
        for (path, actual, required) in [
            (
                "owner_repo_id",
                self.owner_repo_id.as_str(),
                expected.owner_repo_id.as_str(),
            ),
            (
                "owner_contract_schema",
                self.owner_contract_schema.as_str(),
                expected.owner_contract_schema.as_str(),
            ),
            (
                "owner_contract_revision",
                self.owner_contract_revision.as_str(),
                expected.owner_contract_revision.as_str(),
            ),
            (
                "capability_id",
                self.capability_id.as_str(),
                expected.capability_id.as_str(),
            ),
            (
                "request_auth",
                self.request_auth.as_str(),
                expected.request_auth.as_str(),
            ),
            (
                "response_auth",
                self.response_auth.as_str(),
                expected.response_auth.as_str(),
            ),
            (
                "invoke_method",
                self.invoke_method.as_str(),
                expected.invoke_method.as_str(),
            ),
            (
                "invoke_target",
                self.invoke_target.as_str(),
                expected.invoke_target.as_str(),
            ),
            (
                "result_method",
                self.result_method.as_str(),
                expected.result_method.as_str(),
            ),
            (
                "result_target",
                self.result_target.as_str(),
                expected.result_target.as_str(),
            ),
            (
                "result_request_id_parameter",
                self.result_request_id_parameter.as_str(),
                expected.result_request_id_parameter.as_str(),
            ),
            ("command", self.command.as_str(), expected.command.as_str()),
            (
                "owner_result_schema",
                self.owner_result_schema.as_str(),
                expected.owner_result_schema.as_str(),
            ),
        ] {
            if actual != required {
                failures.push(ContractViolation::new(
                    "owner_contract_mismatch",
                    path,
                    "Kiosk owner contract binding differs from the pinned show-controls surface",
                ));
            }
        }
        if self.command_value.is_some() {
            failures.push(ContractViolation::new(
                "owner_contract_mismatch",
                "command_value",
                "Kiosk show-controls accepts no command value",
            ));
        }
        if self.port != KIOSK_DIRECT_OPERATOR_PORT
            || self.max_clock_skew_seconds != KIOSK_DIRECT_OPERATOR_MAX_CLOCK_SKEW_SECONDS
        {
            failures.push(ContractViolation::new(
                "owner_contract_mismatch",
                "port",
                "Kiosk owner port and request-expiry window differ from direct operator v1",
            ));
        }
        finish(failures)
    }
}

impl KioskOwnerContractBinding {
    #[must_use]
    pub fn show_controls_v1() -> Self {
        Self {
            owner_repo_id: KIOSK_DIRECT_OPERATOR_OWNER.to_owned(),
            owner_contract_schema: KIOSK_DIRECT_OPERATOR_SCHEMA.to_owned(),
            owner_contract_revision: KIOSK_DIRECT_OPERATOR_REVISION.to_owned(),
            capability_id: KIOSK_DIRECT_OPERATOR_CAPABILITY_ID.to_owned(),
            request_auth: KIOSK_DIRECT_OPERATOR_REQUEST_AUTH.to_owned(),
            response_auth: KIOSK_DIRECT_OPERATOR_RESPONSE_AUTH.to_owned(),
            invoke_method: KIOSK_DIRECT_OPERATOR_INVOKE_METHOD.to_owned(),
            invoke_target: KIOSK_DIRECT_OPERATOR_INVOKE_TARGET.to_owned(),
            result_method: KIOSK_DIRECT_OPERATOR_RESULT_METHOD.to_owned(),
            result_target: KIOSK_DIRECT_OPERATOR_RESULT_TARGET.to_owned(),
            result_request_id_parameter: KIOSK_DIRECT_OPERATOR_RESULT_REQUEST_ID_PARAMETER
                .to_owned(),
            port: KIOSK_DIRECT_OPERATOR_PORT,
            max_clock_skew_seconds: KIOSK_DIRECT_OPERATOR_MAX_CLOCK_SKEW_SECONDS,
            command: KIOSK_SHOW_CONTROLS_COMMAND.to_owned(),
            command_value: None,
            owner_result_schema: KIOSK_CLI_RESULT_SCHEMA.to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KioskShowControlsTargetPreflight {
    pub device_id: String,
    pub identity_revision: u64,
    pub capability_id: String,
    pub capability_evidence_revision: u64,
    pub capability_owner: String,
    pub support: SupportState,
    pub enablement: EnablementState,
    pub authorization: AuthorizationState,
    pub reachability: ReachabilityState,
    pub freshness: FreshnessState,
    pub observed_at_ms: i64,
    pub fresh_until_ms: i64,
    pub evaluated_at_ms: i64,
    pub eligible: bool,
    pub reason_code: String,
    pub message: String,
}

impl KioskShowControlsTargetPreflight {
    #[must_use]
    pub fn expected_reason_code(&self) -> &'static str {
        match self.support {
            SupportState::Unsupported => "unsupported",
            SupportState::Unknown => "support_unknown",
            SupportState::Supported => match self.enablement {
                EnablementState::Disabled => "disabled",
                EnablementState::Unknown => "enablement_unknown",
                EnablementState::Enabled => match self.authorization {
                    AuthorizationState::Unauthorized => "unauthorized",
                    AuthorizationState::Restricted => "restricted",
                    AuthorizationState::Unknown => "authorization_unknown",
                    AuthorizationState::Authorized => match self.reachability {
                        ReachabilityState::Disconnected => "disconnected",
                        ReachabilityState::Unavailable => "unavailable",
                        ReachabilityState::Unknown => "reachability_unknown",
                        ReachabilityState::Reachable => {
                            if self.freshness == FreshnessState::Current
                                && self.evaluated_at_ms <= self.fresh_until_ms
                            {
                                "ready"
                            } else {
                                match self.freshness {
                                    FreshnessState::Unknown => "freshness_unknown",
                                    FreshnessState::Stale | FreshnessState::Current => "stale",
                                }
                            }
                        }
                    },
                },
            },
        }
    }

    #[must_use]
    pub fn expected_eligibility(&self) -> bool {
        self.expected_reason_code() == "ready"
    }
}

impl ValidateContract for KioskShowControlsTargetPreflight {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        require_nonempty(&mut failures, &self.device_id, "device_id");
        require_nonempty(&mut failures, &self.reason_code, "reason_code");
        require_nonempty(&mut failures, &self.message, "message");
        if self.identity_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_identity_revision",
                "identity_revision",
                "identity revision must be greater than zero",
            ));
        }
        if self.capability_evidence_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_capability_revision",
                "capability_evidence_revision",
                "capability evidence revision must be greater than zero",
            ));
        }
        if self.capability_id != KIOSK_DIRECT_OPERATOR_CAPABILITY_ID
            || self.capability_owner != KIOSK_DIRECT_OPERATOR_OWNER
        {
            failures.push(ContractViolation::new(
                "wrong_capability_owner",
                "capability_id",
                "show-controls preflight must use the Kiosk-owned direct-operator capability",
            ));
        }
        if self.fresh_until_ms < self.observed_at_ms || self.evaluated_at_ms < self.observed_at_ms {
            failures.push(ContractViolation::new(
                "invalid_preflight_window",
                "evaluated_at_ms",
                "preflight evaluation must follow observation and use a coherent freshness window",
            ));
        }
        let expected_reason = self.expected_reason_code();
        if self.eligible != self.expected_eligibility() || self.reason_code != expected_reason {
            failures.push(ContractViolation::new(
                "preflight_result_mismatch",
                "eligible",
                "preflight eligibility and reason must match the exact capability facts",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KioskShowControlsPreview {
    pub schema: String,
    pub preview_id: String,
    pub operation_id: String,
    pub action_id: String,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub fleet_revision: u64,
    pub owner_contract: KioskOwnerContractBinding,
    pub targets: Vec<KioskShowControlsTargetPreflight>,
}

impl ValidateContract for KioskShowControlsPreview {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.kiosk_show_controls_preview.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.kiosk_show_controls_preview.v1",
            ));
        }
        require_nonempty(&mut failures, &self.preview_id, "preview_id");
        require_nonempty(&mut failures, &self.operation_id, "operation_id");
        if self.action_id != KIOSK_SHOW_CONTROLS_ACTION_ID {
            failures.push(ContractViolation::new(
                "wrong_action",
                "action_id",
                "expected kiosk.show-controls",
            ));
        }
        if self.expires_at_ms <= self.created_at_ms {
            failures.push(ContractViolation::new(
                "invalid_expiry",
                "expires_at_ms",
                "preview expiry must follow creation",
            ));
        }
        if self.fleet_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_revision",
                "fleet_revision",
                "Fleet revision must be greater than zero",
            ));
        }
        if self.targets.is_empty() || self.targets.len() > 10_000 {
            failures.push(ContractViolation::new(
                "invalid_target_count",
                "targets",
                "show-controls previews require 1 through 10000 targets",
            ));
        }
        if let Err(mut nested) = self.owner_contract.validate() {
            failures.append(&mut nested);
        }
        let mut last_device_id: Option<&str> = None;
        for (index, target) in self.targets.iter().enumerate() {
            if let Err(nested) = target.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("targets[{index}].{}", failure.path),
                    ..failure
                }));
            }
            if target.evaluated_at_ms < self.created_at_ms
                || target.evaluated_at_ms > self.expires_at_ms
            {
                failures.push(ContractViolation::new(
                    "preflight_outside_preview",
                    &format!("targets[{index}].evaluated_at_ms"),
                    "preflight evaluation must occur inside the immutable preview window",
                ));
            }
            if last_device_id.is_some_and(|previous| previous >= target.device_id.as_str()) {
                failures.push(ContractViolation::new(
                    "noncanonical_targets",
                    &format!("targets[{index}].device_id"),
                    "preview targets must be unique and sorted by device_id",
                ));
            }
            last_device_id = Some(target.device_id.as_str());
        }
        finish(failures)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KioskRetryDisposition {
    NotEligible,
    NewOwnerRequestRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KioskCancelDisposition {
    CancelableBeforeDispatch,
    CancellationRequestedBeforeDispatch,
    CancelledBeforeDispatch,
    NotCancelableAfterDispatch,
    Terminal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KioskEffectiveReceipt {
    pub schema: String,
    pub receipt_id: String,
    pub operation_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub owner_contract: KioskOwnerContractBinding,
    pub owner_action_request_id: String,
    pub owner_result_transport_request_id: String,
    pub owner_command: String,
    pub response_status: u16,
    pub response_content_sha256: String,
    pub response_signature: String,
    pub response_auth_verified: bool,
    pub owner_result_schema: String,
    pub owner_accepted: bool,
    pub owner_completed: bool,
    pub owner_recorded_at_ms: i64,
    pub controls_open: bool,
    pub wrapped_at_ms: i64,
}

impl ValidateContract for KioskEffectiveReceipt {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.kiosk_effective_receipt.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.kiosk_effective_receipt.v1",
            ));
        }
        require_nonempty(&mut failures, &self.receipt_id, "receipt_id");
        require_nonempty(&mut failures, &self.operation_id, "operation_id");
        require_nonempty(&mut failures, &self.device_id, "device_id");
        if self.identity_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_identity_revision",
                "identity_revision",
                "identity revision must be greater than zero",
            ));
        }
        if let Err(mut nested) = self.owner_contract.validate() {
            failures.append(&mut nested);
        }
        if !is_owner_request_id(&self.owner_action_request_id)
            || !is_owner_request_id(&self.owner_result_transport_request_id)
        {
            failures.push(ContractViolation::new(
                "invalid_owner_request_id",
                "owner_action_request_id",
                "Kiosk action and signed result-transport request IDs contain 8 through 64 ASCII letters, digits, underscores, or hyphens",
            ));
        }
        if self.owner_action_request_id == self.owner_result_transport_request_id {
            failures.push(ContractViolation::new(
                "owner_request_id_reused",
                "owner_result_transport_request_id",
                "the stable owner action request ID and signed transport-envelope request ID must be distinct",
            ));
        }
        if self.response_status != 200
            || !is_lower_hex_sha256(&self.response_content_sha256)
            || !is_lower_hex_sha256(&self.response_signature)
            || !self.response_auth_verified
        {
            failures.push(ContractViolation::new(
                "unverified_owner_response",
                "response_auth_verified",
                "effective Kiosk receipts require a verified signed HTTP 200 response and exact lowercase SHA-256 values",
            ));
        }
        if self.owner_result_schema != KIOSK_CLI_RESULT_SCHEMA
            || self.owner_command != KIOSK_SHOW_CONTROLS_COMMAND
            || !self.owner_accepted
            || !self.owner_completed
            || !self.controls_open
        {
            failures.push(ContractViolation::new(
                "ineffective_owner_receipt",
                "owner_command",
                "effective Kiosk receipts require matching accepted, completed show-controls readback with controls_open=true",
            ));
        }
        if self.wrapped_at_ms < self.owner_recorded_at_ms {
            failures.push(ContractViolation::new(
                "invalid_receipt_time",
                "wrapped_at_ms",
                "Fleet wrapping time must not precede the owner receipt",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KioskShowControlsTargetLedger {
    pub device_id: String,
    pub identity_revision: u64,
    pub preflight: KioskShowControlsTargetPreflight,
    pub lifecycle: CommandLifecycle,
    pub dispatched_at_ms: Option<i64>,
    pub owner_deadline_at_ms: Option<i64>,
    pub attempt_count: u8,
    pub owner_request_ids: Vec<String>,
    pub owner_request_id: Option<String>,
    pub effective_receipt: Option<KioskEffectiveReceipt>,
    pub retry_disposition: KioskRetryDisposition,
    pub cancel_disposition: KioskCancelDisposition,
    pub reason_code: String,
    pub message: String,
    pub last_transition_ms: i64,
}

impl KioskShowControlsTargetLedger {
    fn validate_for_operation(
        &self,
        operation_id: &str,
        max_attempts: u8,
        preview_created_at_ms: i64,
        preview_expires_at_ms: i64,
    ) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        require_nonempty(&mut failures, &self.device_id, "device_id");
        require_nonempty(&mut failures, &self.reason_code, "reason_code");
        require_nonempty(&mut failures, &self.message, "message");
        if let Err(mut nested) = self.preflight.validate() {
            failures.append(&mut nested);
        }
        if self.device_id != self.preflight.device_id
            || self.identity_revision != self.preflight.identity_revision
        {
            failures.push(ContractViolation::new(
                "ledger_preflight_mismatch",
                "preflight",
                "target ledger identity must match its frozen preflight",
            ));
        }
        if self.attempt_count > max_attempts {
            failures.push(ContractViolation::new(
                "attempt_limit_exceeded",
                "attempt_count",
                "target attempt count exceeds the operation maximum",
            ));
        }
        if self.dispatched_at_ms.is_some_and(|timestamp| {
            timestamp < preview_created_at_ms
                || timestamp > preview_expires_at_ms
                || timestamp > self.last_transition_ms
        }) {
            failures.push(ContractViolation::new(
                "dispatch_outside_preview",
                "dispatched_at_ms",
                "owner dispatch must occur inside the immutable preview window and before the latest transition",
            ));
        }
        let valid_owner_deadline = match (self.dispatched_at_ms, self.owner_deadline_at_ms) {
            (None, None) => true,
            (Some(dispatched), Some(deadline)) => {
                deadline > dispatched && deadline <= preview_expires_at_ms
            }
            _ => false,
        };
        if !valid_owner_deadline {
            failures.push(ContractViolation::new(
                "invalid_owner_deadline",
                "owner_deadline_at_ms",
                "every owner dispatch retains one absolute deadline after dispatch and no later than preview expiry",
            ));
        }
        if self.owner_request_ids.len() != usize::from(self.attempt_count)
            || self.owner_request_ids.len() > usize::from(max_attempts)
        {
            failures.push(ContractViolation::new(
                "attempt_history_mismatch",
                "owner_request_ids",
                "attempt history must contain exactly one owner request ID per attempt",
            ));
        }
        let unique_request_ids = self
            .owner_request_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        if unique_request_ids.len() != self.owner_request_ids.len()
            || self
                .owner_request_ids
                .iter()
                .any(|request_id| !is_owner_request_id(request_id))
        {
            failures.push(ContractViolation::new(
                "owner_request_reused",
                "owner_request_ids",
                "every retry must use one new valid Kiosk owner request ID",
            ));
        }
        if self
            .owner_request_id
            .as_ref()
            .is_some_and(|request_id| !is_owner_request_id(request_id))
        {
            failures.push(ContractViolation::new(
                "invalid_owner_request_id",
                "owner_request_id",
                "owner request ID does not satisfy the Kiosk v1 contract",
            ));
        }
        if self.owner_request_id.as_ref() != self.owner_request_ids.last() {
            failures.push(ContractViolation::new(
                "current_attempt_mismatch",
                "owner_request_id",
                "current owner request ID must equal the final request in attempt history",
            ));
        }
        if matches!(
            self.lifecycle,
            CommandLifecycle::CleanupPending | CommandLifecycle::Cleaned
        ) {
            failures.push(ContractViolation::new(
                "cleanup_forbidden",
                "lifecycle",
                "show-controls has no temporary effect and never enters cleanup",
            ));
        }

        if !self.preflight.eligible {
            if self.lifecycle != CommandLifecycle::Rejected
                || self.attempt_count != 0
                || self.dispatched_at_ms.is_some()
                || self.owner_deadline_at_ms.is_some()
                || self.owner_request_id.is_some()
                || self.effective_receipt.is_some()
                || self.retry_disposition != KioskRetryDisposition::NotEligible
                || self.cancel_disposition != KioskCancelDisposition::Terminal
            {
                failures.push(ContractViolation::new(
                    "ineligible_target_dispatched",
                    "lifecycle",
                    "an ineligible target must remain rejected without dispatch, retry, cancellation, or receipt",
                ));
            }
            return finish(failures);
        }

        use CommandLifecycle::{
            Accepted, Applied, CancellationRequested, Cancelled, Dispatched, Expired, Failed,
            Proposed, Rejected, Running,
        };
        match self.lifecycle {
            Proposed | Accepted => {
                if self.attempt_count != 0
                    || self.dispatched_at_ms.is_some()
                    || self.owner_deadline_at_ms.is_some()
                    || self.owner_request_id.is_some()
                    || self.effective_receipt.is_some()
                    || self.retry_disposition != KioskRetryDisposition::NotEligible
                    || self.cancel_disposition != KioskCancelDisposition::CancelableBeforeDispatch
                {
                    failures.push(ContractViolation::new(
                        "invalid_pre_dispatch_state",
                        "lifecycle",
                        "eligible pre-dispatch targets are cancelable and have no owner request or receipt",
                    ));
                }
            }
            CancellationRequested => {
                if self.attempt_count != 0
                    || self.dispatched_at_ms.is_some()
                    || self.owner_deadline_at_ms.is_some()
                    || self.owner_request_id.is_some()
                    || self.effective_receipt.is_some()
                    || self.retry_disposition != KioskRetryDisposition::NotEligible
                    || self.cancel_disposition
                        != KioskCancelDisposition::CancellationRequestedBeforeDispatch
                {
                    failures.push(ContractViolation::new(
                        "unsafe_cancellation",
                        "cancel_disposition",
                        "show-controls cancellation may be requested only before dispatch",
                    ));
                }
            }
            Cancelled => {
                if self.attempt_count != 0
                    || self.dispatched_at_ms.is_some()
                    || self.owner_deadline_at_ms.is_some()
                    || self.owner_request_id.is_some()
                    || self.effective_receipt.is_some()
                    || self.retry_disposition != KioskRetryDisposition::NotEligible
                    || self.cancel_disposition != KioskCancelDisposition::CancelledBeforeDispatch
                {
                    failures.push(ContractViolation::new(
                        "unsafe_cancellation",
                        "cancel_disposition",
                        "cancelled show-controls targets must prove no owner dispatch occurred",
                    ));
                }
            }
            Dispatched | Running => {
                if self.attempt_count == 0
                    || self.dispatched_at_ms.is_none()
                    || self.owner_deadline_at_ms.is_none()
                    || self.owner_request_id.is_none()
                    || self.effective_receipt.is_some()
                    || self.retry_disposition != KioskRetryDisposition::NotEligible
                    || self.cancel_disposition != KioskCancelDisposition::NotCancelableAfterDispatch
                {
                    failures.push(ContractViolation::new(
                        "invalid_inflight_state",
                        "lifecycle",
                        "dispatched show-controls cannot be cancelled or retried while owner readback is pending",
                    ));
                }
            }
            Applied => {
                if self.attempt_count == 0
                    || self.dispatched_at_ms.is_none()
                    || self.owner_deadline_at_ms.is_none()
                    || self.owner_request_id.is_none()
                    || self.effective_receipt.is_none()
                    || self.retry_disposition != KioskRetryDisposition::NotEligible
                    || self.cancel_disposition != KioskCancelDisposition::Terminal
                    || self.reason_code != "owner_effective_receipt"
                {
                    failures.push(ContractViolation::new(
                        "applied_without_effective_receipt",
                        "effective_receipt",
                        "applied show-controls requires one matching effective Kiosk owner receipt",
                    ));
                }
            }
            Failed | Expired => {
                let expected_retry = if self.attempt_count < max_attempts {
                    KioskRetryDisposition::NewOwnerRequestRequired
                } else {
                    KioskRetryDisposition::NotEligible
                };
                if self.attempt_count == 0
                    || self.dispatched_at_ms.is_none()
                    || self.owner_deadline_at_ms.is_none()
                    || self.owner_request_id.is_none()
                    || self.effective_receipt.is_some()
                    || self.retry_disposition != expected_retry
                    || self.cancel_disposition != KioskCancelDisposition::Terminal
                {
                    failures.push(ContractViolation::new(
                        "unsafe_retry",
                        "retry_disposition",
                        "failed or expired targets may retry only below the attempt limit and always require a new owner request ID",
                    ));
                }
            }
            Rejected => failures.push(ContractViolation::new(
                "eligible_target_rejected",
                "lifecycle",
                "eligible targets cannot use the ineligible rejection state",
            )),
            CommandLifecycle::CleanupPending | CommandLifecycle::Cleaned => {}
        }

        if let Some(receipt) = &self.effective_receipt {
            if let Err(nested) = receipt.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("effective_receipt.{}", failure.path),
                    ..failure
                }));
            }
            if receipt.operation_id != operation_id
                || receipt.device_id != self.device_id
                || receipt.identity_revision != self.identity_revision
                || self.owner_request_id.as_deref()
                    != Some(receipt.owner_action_request_id.as_str())
            {
                failures.push(ContractViolation::new(
                    "effective_receipt_binding_mismatch",
                    "effective_receipt",
                    "effective owner receipt must bind the exact operation, target identity, and current owner request",
                ));
            }
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KioskShowControlsOperation {
    pub schema: String,
    pub operation_id: String,
    pub action_id: String,
    pub created_at_ms: i64,
    pub preview: KioskShowControlsPreview,
    pub lifecycle: CommandLifecycle,
    pub max_parallelism: u16,
    pub max_attempts_per_target: u8,
    pub cleanup_required: bool,
    pub targets: Vec<KioskShowControlsTargetLedger>,
}

impl KioskShowControlsOperation {
    #[must_use]
    pub fn derived_lifecycle(&self) -> CommandLifecycle {
        let eligible = self
            .targets
            .iter()
            .filter(|target| target.preflight.eligible)
            .collect::<Vec<_>>();
        if eligible.is_empty() {
            return CommandLifecycle::Rejected;
        }
        if eligible.iter().any(|target| {
            matches!(
                target.lifecycle,
                CommandLifecycle::Dispatched | CommandLifecycle::Running
            )
        }) {
            return CommandLifecycle::Running;
        }
        if eligible
            .iter()
            .any(|target| target.lifecycle == CommandLifecycle::CancellationRequested)
        {
            return CommandLifecycle::CancellationRequested;
        }
        if eligible
            .iter()
            .any(|target| target.lifecycle == CommandLifecycle::Proposed)
        {
            return CommandLifecycle::Proposed;
        }
        if eligible
            .iter()
            .any(|target| target.lifecycle == CommandLifecycle::Accepted)
        {
            return CommandLifecycle::Accepted;
        }
        if eligible
            .iter()
            .all(|target| target.lifecycle == CommandLifecycle::Applied)
        {
            return CommandLifecycle::Applied;
        }
        if eligible
            .iter()
            .all(|target| target.lifecycle == CommandLifecycle::Cancelled)
        {
            return CommandLifecycle::Cancelled;
        }
        if eligible
            .iter()
            .all(|target| target.lifecycle == CommandLifecycle::Expired)
        {
            return CommandLifecycle::Expired;
        }
        CommandLifecycle::Failed
    }
}

impl ValidateContract for KioskShowControlsOperation {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.kiosk_show_controls_operation.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.kiosk_show_controls_operation.v1",
            ));
        }
        require_nonempty(&mut failures, &self.operation_id, "operation_id");
        if self.action_id != KIOSK_SHOW_CONTROLS_ACTION_ID {
            failures.push(ContractViolation::new(
                "wrong_action",
                "action_id",
                "expected kiosk.show-controls",
            ));
        }
        if self.cleanup_required {
            failures.push(ContractViolation::new(
                "cleanup_forbidden",
                "cleanup_required",
                "show-controls creates no temporary effect and requires no cleanup",
            ));
        }
        if !(1..=64).contains(&self.max_parallelism) {
            failures.push(ContractViolation::new(
                "invalid_parallelism",
                "max_parallelism",
                "show-controls dispatch parallelism must be between 1 and 64",
            ));
        }
        if !(1..=8).contains(&self.max_attempts_per_target) {
            failures.push(ContractViolation::new(
                "invalid_attempt_limit",
                "max_attempts_per_target",
                "show-controls permits 1 through 8 explicitly bounded attempts per target",
            ));
        }
        if matches!(
            self.lifecycle,
            CommandLifecycle::CleanupPending | CommandLifecycle::Cleaned
        ) {
            failures.push(ContractViolation::new(
                "cleanup_forbidden",
                "lifecycle",
                "show-controls operation lifecycle never enters cleanup",
            ));
        }
        if let Err(nested) = self.preview.validate() {
            failures.extend(nested.into_iter().map(|failure| ContractViolation {
                path: format!("preview.{}", failure.path),
                ..failure
            }));
        }
        if self.preview.operation_id != self.operation_id
            || self.preview.action_id != self.action_id
            || self.preview.created_at_ms != self.created_at_ms
        {
            failures.push(ContractViolation::new(
                "preview_binding_mismatch",
                "preview",
                "operation must retain the exact preview identity, action, and creation time",
            ));
        }
        if self.targets.len() != self.preview.targets.len() {
            failures.push(ContractViolation::new(
                "preview_target_mismatch",
                "targets",
                "ledger must retain exactly one target for every frozen preview target",
            ));
        }
        let preview_targets = self
            .preview
            .targets
            .iter()
            .map(|target| (target.device_id.as_str(), target))
            .collect::<BTreeMap<_, _>>();
        let mut last_device_id: Option<&str> = None;
        for (index, target) in self.targets.iter().enumerate() {
            if let Err(nested) = target.validate_for_operation(
                &self.operation_id,
                self.max_attempts_per_target,
                self.preview.created_at_ms,
                self.preview.expires_at_ms,
            ) {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("targets[{index}].{}", failure.path),
                    ..failure
                }));
            }
            if preview_targets
                .get(target.device_id.as_str())
                .is_none_or(|preview| **preview != target.preflight)
            {
                failures.push(ContractViolation::new(
                    "preview_target_mismatch",
                    &format!("targets[{index}].preflight"),
                    "target ledger must preserve its complete immutable preview preflight",
                ));
            }
            if last_device_id.is_some_and(|previous| previous >= target.device_id.as_str()) {
                failures.push(ContractViolation::new(
                    "noncanonical_targets",
                    &format!("targets[{index}].device_id"),
                    "operation targets must remain unique and sorted by device_id",
                ));
            }
            last_device_id = Some(target.device_id.as_str());
        }
        if self.lifecycle != self.derived_lifecycle() {
            failures.push(ContractViolation::new(
                "operation_lifecycle_mismatch",
                "lifecycle",
                "operation lifecycle must be derived from the frozen eligible target ledgers",
            ));
        }
        let inflight = self
            .targets
            .iter()
            .filter(|target| {
                matches!(
                    target.lifecycle,
                    CommandLifecycle::Dispatched | CommandLifecycle::Running
                )
            })
            .count();
        if inflight > usize::from(self.max_parallelism) {
            failures.push(ContractViolation::new(
                "operation_parallelism_exceeded",
                "targets",
                "in-flight target count must not exceed the frozen operation parallelism",
            ));
        }
        finish(failures)
    }
}

fn is_owner_request_id(value: &str) -> bool {
    (8..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
