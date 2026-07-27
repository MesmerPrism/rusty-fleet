// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    AuthorizationState, CommandLifecycle, ContractViolation, EnablementState, FreshnessState,
    ReachabilityState, SupportState, ValidateContract, finish, require_nonempty,
};

pub const QUEST_AWAKE_ACTION_ID: &str = "quest.awake-control";
pub const QUEST_AWAKE_CAPABILITY_ID: &str = "questionable-file-manager.quest-awake-provider";
pub const QUEST_AWAKE_OWNER: &str = "questionable-file-manager";
pub const QUEST_AWAKE_PROVIDER_CONTRACT: &str = "questionable.file_manager.fleet_awake_provider.v1";
pub const QUEST_AWAKE_RECEIPT_SCHEMA: &str = "questionable.file_manager.quest_awake_receipt.v1";
pub const QUEST_AWAKE_PREVIEW_REQUEST_SCHEMA: &str = "rusty.fleet.quest_awake_preview_request.v1";
pub const QUEST_AWAKE_EXECUTE_REQUEST_SCHEMA: &str = "rusty.fleet.quest_awake_execute_request.v1";
pub const QUEST_AWAKE_OPERATION_SCHEMA: &str = "rusty.fleet.quest_awake_operation.v1";
pub const QUEST_AWAKE_MIN_DURATION_MS: u32 = 60_000;
pub const QUEST_AWAKE_MAX_DURATION_MS: u32 = 28_800_000;
pub const QUEST_AWAKE_MIN_WATCHDOG_INTERVAL_MS: u32 = 1_000;
pub const QUEST_AWAKE_MAX_WATCHDOG_INTERVAL_MS: u32 = 60_000;
pub const QUEST_AWAKE_DEFAULT_WATCHDOG_INTERVAL_MS: u32 = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestAwakeAction {
    Status,
    ApplyBounded,
    StartWindowsWatchdog,
    StartDeviceWatchdog,
    StopWatchdogs,
    RestoreNormal,
}

impl QuestAwakeAction {
    #[must_use]
    pub const fn provider_action(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::ApplyBounded => "applyBounded",
            Self::StartWindowsWatchdog => "repairOnce",
            Self::StartDeviceWatchdog => "startDeviceWatchdog",
            Self::StopWatchdogs => "stopWatchdogs",
            Self::RestoreNormal => "restoreNormal",
        }
    }

    #[must_use]
    pub const fn is_watchdog(self) -> bool {
        matches!(self, Self::StartWindowsWatchdog | Self::StartDeviceWatchdog)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAwakePreviewRequest {
    pub schema: String,
    pub action_id: String,
    pub action: QuestAwakeAction,
    pub duration_ms: u32,
    pub watchdog_interval_ms: u32,
    pub targets: BTreeMap<String, u64>,
}

impl ValidateContract for QuestAwakePreviewRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != QUEST_AWAKE_PREVIEW_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.quest_awake_preview_request.v1",
            ));
        }
        if self.action_id != QUEST_AWAKE_ACTION_ID {
            failures.push(ContractViolation::new(
                "wrong_action",
                "action_id",
                "expected quest.awake-control",
            ));
        }
        validate_policy(self.duration_ms, self.watchdog_interval_ms, &mut failures);
        validate_targets(&self.targets, &mut failures);
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAwakeExecuteRequest {
    pub schema: String,
    pub operation_id: String,
    pub preview_id: String,
}

impl ValidateContract for QuestAwakeExecuteRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != QUEST_AWAKE_EXECUTE_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.quest_awake_execute_request.v1",
            ));
        }
        for (path, value) in [
            ("operation_id", self.operation_id.as_str()),
            ("preview_id", self.preview_id.as_str()),
        ] {
            if !is_portable_id(value, 256) {
                failures.push(ContractViolation::new(
                    "invalid_identifier",
                    path,
                    "operation and preview IDs must be bounded portable identifiers",
                ));
            }
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAwakeOwnerBinding {
    pub owner_repo_id: String,
    pub capability_id: String,
    pub provider_contract: String,
    pub receipt_schema: String,
    pub transport: String,
    pub application_proof: String,
}

impl QuestAwakeOwnerBinding {
    #[must_use]
    pub fn file_manager_v1() -> Self {
        Self {
            owner_repo_id: QUEST_AWAKE_OWNER.to_owned(),
            capability_id: QUEST_AWAKE_CAPABILITY_ID.to_owned(),
            provider_contract: QUEST_AWAKE_PROVIDER_CONTRACT.to_owned(),
            receipt_schema: QUEST_AWAKE_RECEIPT_SCHEMA.to_owned(),
            transport: "pinned_local_subprocess".to_owned(),
            application_proof: "fresh_effective_power_and_watchdog_readback".to_owned(),
        }
    }
}

impl ValidateContract for QuestAwakeOwnerBinding {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        if self == &Self::file_manager_v1() {
            Ok(())
        } else {
            Err(vec![ContractViolation::new(
                "owner_contract_mismatch",
                "owner",
                "Quest awake owner differs from the pinned File Manager provider contract",
            )])
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAwakeTargetPreflight {
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

impl QuestAwakeTargetPreflight {
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
                        ReachabilityState::Unavailable => "provider_unavailable",
                        ReachabilityState::Unknown => "reachability_unknown",
                        ReachabilityState::Reachable => {
                            if self.freshness == FreshnessState::Current
                                && self.evaluated_at_ms <= self.fresh_until_ms
                            {
                                "ready"
                            } else if self.freshness == FreshnessState::Unknown {
                                "freshness_unknown"
                            } else {
                                "stale"
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

impl ValidateContract for QuestAwakeTargetPreflight {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        require_nonempty(&mut failures, &self.device_id, "device_id");
        require_nonempty(&mut failures, &self.message, "message");
        if self.identity_revision == 0 || self.capability_evidence_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_revision",
                "identity_revision",
                "identity and capability revisions must be greater than zero",
            ));
        }
        if self.capability_id != QUEST_AWAKE_CAPABILITY_ID
            || self.capability_owner != QUEST_AWAKE_OWNER
        {
            failures.push(ContractViolation::new(
                "wrong_capability_owner",
                "capability_id",
                "Quest awake preflight must use the File Manager owner capability",
            ));
        }
        if self.fresh_until_ms < self.observed_at_ms || self.evaluated_at_ms < self.observed_at_ms {
            failures.push(ContractViolation::new(
                "invalid_preflight_window",
                "evaluated_at_ms",
                "preflight timestamps are not coherent",
            ));
        }
        if self.eligible != self.expected_eligibility()
            || self.reason_code != self.expected_reason_code()
        {
            failures.push(ContractViolation::new(
                "preflight_result_mismatch",
                "eligible",
                "eligibility and reason must match the frozen capability facts",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAwakePreview {
    pub schema: String,
    pub preview_id: String,
    pub operation_id: String,
    pub action_id: String,
    pub action: QuestAwakeAction,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub fleet_revision: u64,
    pub duration_ms: u32,
    pub watchdog_interval_ms: u32,
    pub watchdog_generation: String,
    pub owner: QuestAwakeOwnerBinding,
    pub targets: Vec<QuestAwakeTargetPreflight>,
}

impl ValidateContract for QuestAwakePreview {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.quest_awake_preview.v1"
            || !is_portable_id(&self.preview_id, 256)
            || !is_portable_id(&self.operation_id, 256)
            || !is_portable_id(&self.watchdog_generation, 256)
            || self.action_id != QUEST_AWAKE_ACTION_ID
            || self.created_at_ms < 0
            || self.expires_at_ms <= self.created_at_ms
            || self.fleet_revision == 0
        {
            failures.push(ContractViolation::new(
                "invalid_preview_header",
                "preview",
                "Quest awake preview identity, action, time, or revision is invalid",
            ));
        }
        validate_policy(self.duration_ms, self.watchdog_interval_ms, &mut failures);
        if let Err(mut nested) = self.owner.validate() {
            failures.append(&mut nested);
        }
        let mut devices = BTreeSet::new();
        if self.targets.is_empty() || self.targets.len() > 10_000 {
            failures.push(ContractViolation::new(
                "invalid_target_count",
                "targets",
                "Quest awake preview must contain 1 through 10,000 targets",
            ));
        }
        for target in &self.targets {
            if !devices.insert(target.device_id.clone()) {
                failures.push(ContractViolation::new(
                    "duplicate_target",
                    "targets",
                    "Quest awake preview targets must be unique",
                ));
            }
            if let Err(mut nested) = target.validate() {
                failures.append(&mut nested);
            }
            if target.evaluated_at_ms != self.created_at_ms {
                failures.push(ContractViolation::new(
                    "preflight_time_mismatch",
                    "targets.evaluated_at_ms",
                    "target preflight must bind the preview time",
                ));
            }
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAwakeOwnerInvocation {
    pub schema: String,
    pub request_id: String,
    pub operation_id: String,
    pub preview_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub action: QuestAwakeAction,
    pub duration_ms: u32,
    pub watchdog_interval_ms: u32,
    pub watchdog_generation: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

impl ValidateContract for QuestAwakeOwnerInvocation {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.quest_awake_owner_invocation.v1"
            || self.identity_revision == 0
            || self.issued_at_ms < 0
            || self.expires_at_ms <= self.issued_at_ms
        {
            failures.push(ContractViolation::new(
                "invalid_invocation_header",
                "invocation",
                "Quest awake invocation header is invalid",
            ));
        }
        for (path, value) in [
            ("request_id", self.request_id.as_str()),
            ("operation_id", self.operation_id.as_str()),
            ("preview_id", self.preview_id.as_str()),
            ("device_id", self.device_id.as_str()),
            ("watchdog_generation", self.watchdog_generation.as_str()),
        ] {
            if !is_portable_id(value, 256) {
                failures.push(ContractViolation::new(
                    "invalid_identifier",
                    path,
                    "invocation identifiers must be bounded and portable",
                ));
            }
        }
        validate_policy(self.duration_ms, self.watchdog_interval_ms, &mut failures);
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAwakePowerReadback {
    pub wakefulness: String,
    pub display_state: String,
    pub stay_on: bool,
    pub auto_sleep_disabled: Option<bool>,
    pub proximity_state: String,
    pub proximity_hold_duration_ms: Option<u32>,
    pub proximity_hold_remaining_ms: Option<u32>,
    pub captured_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAwakeWatchdogReadback {
    pub reported_active: bool,
    pub fresh: bool,
    pub generation: String,
    pub boot_id_sha256: String,
    pub interval_ms: u32,
    pub last_poll_ms: i64,
    pub proximity_repair_count: u32,
    pub stay_on_repair_count: u32,
    pub wake_repair_count: u32,
    pub last_action: String,
    pub last_error: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAwakeOwnerReceipt {
    pub schema: String,
    pub request_id: String,
    pub operation_id: String,
    pub preview_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub action: QuestAwakeAction,
    pub watchdog_generation: String,
    pub requested_duration_ms: u32,
    pub requested_watchdog_interval_ms: u32,
    pub stay_on_effective: bool,
    pub proximity_hold_effective: bool,
    pub wake_effective: bool,
    pub windows_watchdog_effective: bool,
    pub device_watchdog_effective: bool,
    pub settings_restored: bool,
    pub effective: bool,
    pub settings_left_unchanged: bool,
    pub outcome: String,
    pub repair_count: u32,
    pub power: QuestAwakePowerReadback,
    pub device_watchdog: QuestAwakeWatchdogReadback,
    pub evidence_sha256: String,
    pub observed_at_ms: i64,
}

impl ValidateContract for QuestAwakeOwnerReceipt {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != QUEST_AWAKE_RECEIPT_SCHEMA || self.identity_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_receipt_header",
                "receipt",
                "Quest awake owner receipt schema or identity revision is invalid",
            ));
        }
        for (path, value) in [
            ("request_id", self.request_id.as_str()),
            ("operation_id", self.operation_id.as_str()),
            ("preview_id", self.preview_id.as_str()),
            ("device_id", self.device_id.as_str()),
            ("watchdog_generation", self.watchdog_generation.as_str()),
        ] {
            if !is_portable_id(value, 256) {
                failures.push(ContractViolation::new(
                    "invalid_identifier",
                    path,
                    "receipt identifiers must be bounded and portable",
                ));
            }
        }
        validate_policy(
            self.requested_duration_ms,
            self.requested_watchdog_interval_ms,
            &mut failures,
        );
        if self.evidence_sha256.len() != 64
            || !self
                .evidence_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            failures.push(ContractViolation::new(
                "invalid_evidence_digest",
                "evidence_sha256",
                "receipt evidence digest must be lowercase SHA-256",
            ));
        }
        if self.observed_at_ms < 0
            || self.power.captured_at_ms < 0
            || self.power.captured_at_ms > self.observed_at_ms
            || self.observed_at_ms - self.power.captured_at_ms > 30_000
            || self.power.wakefulness.is_empty()
            || self.power.wakefulness.len() > 128
            || self.power.display_state.is_empty()
            || self.power.display_state.len() > 128
            || self.power.proximity_state.is_empty()
            || self.power.proximity_state.len() > 128
            || self.device_watchdog.boot_id_sha256.len() != 64
            || (self.device_watchdog.reported_active
                && self.device_watchdog.boot_id_sha256
                    == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            || !self
                .device_watchdog
                .boot_id_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || self.device_watchdog.generation.len() > 256
            || self.device_watchdog.last_action.len() > 256
            || self.device_watchdog.last_error.len() > 512
            || self.outcome.is_empty()
            || self.outcome.len() > 256
        {
            failures.push(ContractViolation::new(
                "invalid_readback",
                "receipt",
                "power and watchdog readbacks must be bounded, coherent, and freshly captured",
            ));
        }
        let expected_stay_on = self.power.stay_on;
        let expected_proximity_hold = self
            .power
            .proximity_state
            .trim()
            .eq_ignore_ascii_case("CLOSE")
            && self.power.proximity_hold_duration_ms == Some(self.requested_duration_ms)
            && self
                .power
                .proximity_hold_remaining_ms
                .is_some_and(|remaining| remaining > 0);
        let expected_wake = self.power.wakefulness.eq_ignore_ascii_case("Awake")
            && (self.power.display_state.eq_ignore_ascii_case("ON")
                || self.power.display_state.eq_ignore_ascii_case("ON_SUSPEND"));
        let watchdog_poll_window_ms = i64::from(self.requested_watchdog_interval_ms)
            .saturating_mul(3)
            .max(15_000);
        let expected_device_watchdog = self.device_watchdog.reported_active
            && self.device_watchdog.fresh
            && self.device_watchdog.generation == self.watchdog_generation
            && self.device_watchdog.interval_ms == self.requested_watchdog_interval_ms
            && self.device_watchdog.last_poll_ms > 0
            && self.device_watchdog.last_poll_ms <= self.observed_at_ms + 30_000
            && self.observed_at_ms - self.device_watchdog.last_poll_ms <= watchdog_poll_window_ms;
        let expected_settings_restored = !self.power.stay_on
            && self.power.auto_sleep_disabled != Some(true)
            && !self
                .power
                .proximity_state
                .trim()
                .eq_ignore_ascii_case("CLOSE");
        let expected_settings_left_unchanged = matches!(
            self.action,
            QuestAwakeAction::Status | QuestAwakeAction::StopWatchdogs
        );
        if self.stay_on_effective != expected_stay_on
            || self.proximity_hold_effective != expected_proximity_hold
            || self.wake_effective != expected_wake
            || self.device_watchdog_effective != expected_device_watchdog
            || self.settings_restored != expected_settings_restored
            || self.settings_left_unchanged != expected_settings_left_unchanged
        {
            failures.push(ContractViolation::new(
                "readback_summary_mismatch",
                "receipt",
                "summary effects must be derived from the included independent readbacks",
            ));
        }
        let expected_effective = self.derived_effective();
        if self.effective != expected_effective {
            failures.push(ContractViolation::new(
                "receipt_effect_mismatch",
                "effective",
                "effective state must match the independent power, watchdog, and restore readbacks",
            ));
        }
        finish(failures)
    }
}

impl QuestAwakeOwnerReceipt {
    #[must_use]
    pub fn derived_effective(&self) -> bool {
        let watchdog_absent = !self.device_watchdog.reported_active;
        match self.action {
            QuestAwakeAction::Status => {
                self.power.captured_at_ms >= 0
                    && self.power.captured_at_ms <= self.observed_at_ms
                    && self.observed_at_ms - self.power.captured_at_ms <= 30_000
            }
            QuestAwakeAction::ApplyBounded => {
                self.stay_on_effective
                    && self.proximity_hold_effective
                    && self.wake_effective
                    && !self.windows_watchdog_effective
                    && watchdog_absent
            }
            QuestAwakeAction::StartWindowsWatchdog => {
                self.stay_on_effective
                    && self.proximity_hold_effective
                    && self.wake_effective
                    && self.windows_watchdog_effective
                    && watchdog_absent
            }
            QuestAwakeAction::StartDeviceWatchdog => {
                self.stay_on_effective
                    && self.proximity_hold_effective
                    && self.wake_effective
                    && !self.windows_watchdog_effective
                    && self.device_watchdog_effective
            }
            QuestAwakeAction::StopWatchdogs => {
                !self.windows_watchdog_effective && watchdog_absent && self.settings_left_unchanged
            }
            QuestAwakeAction::RestoreNormal => {
                !self.windows_watchdog_effective && watchdog_absent && self.settings_restored
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAwakeTargetLedger {
    pub device_id: String,
    pub identity_revision: u64,
    pub preflight: QuestAwakeTargetPreflight,
    pub lifecycle: CommandLifecycle,
    pub invocation: Option<QuestAwakeOwnerInvocation>,
    pub receipt: Option<QuestAwakeOwnerReceipt>,
    pub failure_code: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAwakeOperation {
    pub schema: String,
    pub operation_id: String,
    pub action_id: String,
    pub lifecycle: CommandLifecycle,
    pub preview: QuestAwakePreview,
    pub confirmed_at_ms: Option<i64>,
    pub targets: Vec<QuestAwakeTargetLedger>,
    pub updated_at_ms: i64,
}

impl ValidateContract for QuestAwakeOperation {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != QUEST_AWAKE_OPERATION_SCHEMA
            || self.operation_id != self.preview.operation_id
            || self.action_id != QUEST_AWAKE_ACTION_ID
            || self.updated_at_ms < self.preview.created_at_ms
        {
            failures.push(ContractViolation::new(
                "invalid_operation_header",
                "operation",
                "Quest awake operation identity, action, or time is invalid",
            ));
        }
        if let Err(mut nested) = self.preview.validate() {
            failures.append(&mut nested);
        }
        if self.targets.len() != self.preview.targets.len() {
            failures.push(ContractViolation::new(
                "target_count_mismatch",
                "targets",
                "operation ledger must retain every preview target",
            ));
        }
        for target in &self.targets {
            let Some(preflight) = self
                .preview
                .targets
                .iter()
                .find(|candidate| candidate.device_id == target.device_id)
            else {
                failures.push(ContractViolation::new(
                    "unknown_target",
                    "targets",
                    "operation ledger target is absent from the preview",
                ));
                continue;
            };
            if target.identity_revision != preflight.identity_revision
                || &target.preflight != preflight
                || target.updated_at_ms < self.preview.created_at_ms
            {
                failures.push(ContractViolation::new(
                    "target_binding_mismatch",
                    "targets",
                    "operation target differs from the frozen preview",
                ));
            }
            if let Some(invocation) = &target.invocation
                && (invocation.validate().is_err()
                    || invocation.operation_id != self.operation_id
                    || invocation.preview_id != self.preview.preview_id
                    || invocation.device_id != target.device_id
                    || invocation.identity_revision != target.identity_revision
                    || invocation.action != self.preview.action
                    || (!matches!(
                        self.preview.action,
                        QuestAwakeAction::StopWatchdogs | QuestAwakeAction::RestoreNormal
                    ) && invocation.watchdog_generation != self.preview.watchdog_generation)
                    || invocation.duration_ms != self.preview.duration_ms
                    || invocation.watchdog_interval_ms != self.preview.watchdog_interval_ms)
            {
                failures.push(ContractViolation::new(
                    "invocation_binding_mismatch",
                    "targets.invocation",
                    "owner invocation differs from the frozen operation",
                ));
            }
            if let Some(receipt) = &target.receipt
                && (receipt.validate().is_err()
                    || receipt.operation_id != self.operation_id
                    || receipt.preview_id != self.preview.preview_id
                    || receipt.device_id != target.device_id
                    || receipt.identity_revision != target.identity_revision
                    || receipt.action != self.preview.action
                    || target.invocation.as_ref().is_none_or(|invocation| {
                        receipt.watchdog_generation != invocation.watchdog_generation
                    })
                    || receipt.requested_duration_ms != self.preview.duration_ms
                    || receipt.requested_watchdog_interval_ms != self.preview.watchdog_interval_ms
                    || target
                        .invocation
                        .as_ref()
                        .is_none_or(|invocation| invocation.request_id != receipt.request_id))
            {
                failures.push(ContractViolation::new(
                    "receipt_binding_mismatch",
                    "targets.receipt",
                    "owner receipt differs from the exact invocation",
                ));
            }
            if target.lifecycle == CommandLifecycle::Applied
                && target
                    .receipt
                    .as_ref()
                    .is_none_or(|receipt| !receipt.effective || receipt.validate().is_err())
            {
                failures.push(ContractViolation::new(
                    "applied_without_effective_receipt",
                    "targets.lifecycle",
                    "an applied Quest awake target requires its exact effective owner receipt",
                ));
            }
        }
        finish(failures)
    }
}

fn validate_policy(
    duration_ms: u32,
    watchdog_interval_ms: u32,
    failures: &mut Vec<ContractViolation>,
) {
    if !(QUEST_AWAKE_MIN_DURATION_MS..=QUEST_AWAKE_MAX_DURATION_MS).contains(&duration_ms) {
        failures.push(ContractViolation::new(
            "invalid_duration",
            "duration_ms",
            "Quest awake duration must be between one minute and eight hours",
        ));
    }
    if !(QUEST_AWAKE_MIN_WATCHDOG_INTERVAL_MS..=QUEST_AWAKE_MAX_WATCHDOG_INTERVAL_MS)
        .contains(&watchdog_interval_ms)
    {
        failures.push(ContractViolation::new(
            "invalid_watchdog_interval",
            "watchdog_interval_ms",
            "watchdog interval must be between one and sixty seconds",
        ));
    }
}

fn validate_targets(targets: &BTreeMap<String, u64>, failures: &mut Vec<ContractViolation>) {
    if targets.is_empty() || targets.len() > 10_000 {
        failures.push(ContractViolation::new(
            "invalid_target_count",
            "targets",
            "Quest awake request must contain 1 through 10,000 exact targets",
        ));
    }
    for (device_id, revision) in targets {
        if !is_portable_id(device_id, 256) || *revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_target",
                "targets",
                "target device IDs must be bounded portable IDs with positive revisions",
            ));
        }
    }
}

fn is_portable_id(value: &str, maximum: usize) -> bool {
    (1..=maximum).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
