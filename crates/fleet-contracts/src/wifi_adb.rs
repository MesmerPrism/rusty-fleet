// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    AuthorizationState, CommandLifecycle, ContractViolation, EnablementState, FreshnessState,
    ReachabilityState, SupportState, ValidateContract, finish, require_nonempty,
};

pub const QUEST_WIFI_ADB_ACTION_ID: &str = "quest.wifi-adb-control";
pub const QUEST_WIFI_ADB_CAPABILITY_ID: &str = "questionable-file-manager.quest-wifi-adb-provider";
pub const QUEST_WIFI_ADB_OWNER: &str = "questionable-file-manager";
pub const QUEST_WIFI_ADB_PROVIDER_CONTRACT: &str =
    "questionable.file_manager.fleet_connectivity_provider.v1";
pub const QUEST_WIFI_ADB_RECEIPT_SCHEMA: &str =
    "questionable.file_manager.quest_wifi_adb_receipt.v1";
pub const QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA: &str =
    "rusty.fleet.quest_wifi_adb_preview_request.v1";
pub const QUEST_WIFI_ADB_EXECUTE_REQUEST_SCHEMA: &str =
    "rusty.fleet.quest_wifi_adb_execute_request.v1";
pub const QUEST_WIFI_ADB_OPERATION_SCHEMA: &str = "rusty.fleet.quest_wifi_adb_operation.v1";
pub const QUEST_WIFI_ADB_TERMUX_PROOF_SCHEMA: &str = "rusty.fleet.quest_wifi_adb_termux_proof.v1";
pub const QUEST_WIFI_ADB_TERMUX_PROOF_OWNER: &str = "quest-termux-lab";
pub const QUEST_WIFI_ADB_TERMUX_CAPABILITY_ID: &str = "capability.quest-termux-loopback-adb-shell";
pub const TERMUX_ADB_SHELL_IDENTITY: &str = "uid=2000(shell)";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestWifiAdbAction {
    Status,
    RequestWirelessAdb,
    EnableRequestAfterBoot,
    DisableRequestAfterBoot,
    DisableWirelessAdb,
    EnableClassicTcpipFromUsb,
}

impl QuestWifiAdbAction {
    #[must_use]
    pub const fn provider_action(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::RequestWirelessAdb => "request_wireless_adb",
            Self::EnableRequestAfterBoot => "enable_request_after_boot",
            Self::DisableRequestAfterBoot => "disable_request_after_boot",
            Self::DisableWirelessAdb => "disable_wireless_adb",
            Self::EnableClassicTcpipFromUsb => "enable_classic_tcpip_from_usb",
        }
    }

    #[must_use]
    pub const fn is_classic(self) -> bool {
        matches!(self, Self::EnableClassicTcpipFromUsb)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestWifiAdbRouteMode {
    None,
    ModernTls,
    ClassicTcpip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestWifiAdbWearerApproval {
    NotApplicable,
    Pending,
    Accepted,
    Rejected,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestWifiAdbPreviewRequest {
    pub schema: String,
    pub action_id: String,
    pub action: QuestWifiAdbAction,
    pub targets: BTreeMap<String, u64>,
}

impl ValidateContract for QuestWifiAdbPreviewRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.quest_wifi_adb_preview_request.v1",
            ));
        }
        if self.action_id != QUEST_WIFI_ADB_ACTION_ID {
            failures.push(ContractViolation::new(
                "wrong_action",
                "action_id",
                "expected quest.wifi-adb-control",
            ));
        }
        validate_targets(&self.targets, &mut failures);
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestWifiAdbExecuteRequest {
    pub schema: String,
    pub operation_id: String,
    pub preview_id: String,
}

impl ValidateContract for QuestWifiAdbExecuteRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != QUEST_WIFI_ADB_EXECUTE_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.quest_wifi_adb_execute_request.v1",
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
pub struct QuestWifiAdbOwnerBinding {
    pub owner_repo_id: String,
    pub capability_id: String,
    pub provider_contract: String,
    pub receipt_schema: String,
    pub transport: String,
    pub private_target_resolution: String,
}

impl QuestWifiAdbOwnerBinding {
    #[must_use]
    pub fn file_manager_v1() -> Self {
        Self {
            owner_repo_id: QUEST_WIFI_ADB_OWNER.to_owned(),
            capability_id: QUEST_WIFI_ADB_CAPABILITY_ID.to_owned(),
            provider_contract: QUEST_WIFI_ADB_PROVIDER_CONTRACT.to_owned(),
            receipt_schema: QUEST_WIFI_ADB_RECEIPT_SCHEMA.to_owned(),
            transport: "pinned_local_subprocess".to_owned(),
            private_target_resolution: "provider_owned_credential_profile".to_owned(),
        }
    }
}

impl ValidateContract for QuestWifiAdbOwnerBinding {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        if self == &Self::file_manager_v1() {
            Ok(())
        } else {
            Err(vec![ContractViolation::new(
                "owner_contract_mismatch",
                "owner",
                "Quest Wi-Fi ADB owner differs from the pinned File Manager provider",
            )])
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestWifiAdbTargetPreflight {
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

impl QuestWifiAdbTargetPreflight {
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

impl ValidateContract for QuestWifiAdbTargetPreflight {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        require_nonempty(&mut failures, &self.device_id, "device_id");
        require_nonempty(&mut failures, &self.message, "message");
        if self.identity_revision == 0 || self.capability_evidence_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_revision",
                "identity_revision",
                "identity and capability revisions must be positive",
            ));
        }
        if self.capability_id != QUEST_WIFI_ADB_CAPABILITY_ID
            || self.capability_owner != QUEST_WIFI_ADB_OWNER
        {
            failures.push(ContractViolation::new(
                "wrong_capability_owner",
                "capability_id",
                "Quest Wi-Fi ADB preflight must use File Manager capability evidence",
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
pub struct QuestWifiAdbPreview {
    pub schema: String,
    pub preview_id: String,
    pub operation_id: String,
    pub action_id: String,
    pub action: QuestWifiAdbAction,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub fleet_revision: u64,
    pub owner: QuestWifiAdbOwnerBinding,
    pub targets: Vec<QuestWifiAdbTargetPreflight>,
}

impl ValidateContract for QuestWifiAdbPreview {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.quest_wifi_adb_preview.v1"
            || !is_portable_id(&self.preview_id, 256)
            || !is_portable_id(&self.operation_id, 256)
            || self.action_id != QUEST_WIFI_ADB_ACTION_ID
            || self.created_at_ms < 0
            || self.expires_at_ms <= self.created_at_ms
            || self.fleet_revision == 0
        {
            failures.push(ContractViolation::new(
                "invalid_preview_header",
                "preview",
                "Quest Wi-Fi ADB preview identity, time, or revision is invalid",
            ));
        }
        if let Err(mut nested) = self.owner.validate() {
            failures.append(&mut nested);
        }
        let mut devices = BTreeSet::new();
        if self.targets.is_empty() || self.targets.len() > 10_000 {
            failures.push(ContractViolation::new(
                "invalid_target_count",
                "targets",
                "preview must contain 1 through 10,000 targets",
            ));
        }
        for target in &self.targets {
            if !devices.insert(target.device_id.clone()) {
                failures.push(ContractViolation::new(
                    "duplicate_target",
                    "targets",
                    "preview targets must be unique",
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
pub struct QuestWifiAdbOwnerInvocation {
    pub schema: String,
    pub request_id: String,
    pub operation_id: String,
    pub preview_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub action: QuestWifiAdbAction,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

impl ValidateContract for QuestWifiAdbOwnerInvocation {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.quest_wifi_adb_owner_invocation.v1"
            || self.identity_revision == 0
            || self.issued_at_ms < 0
            || self.expires_at_ms <= self.issued_at_ms
        {
            failures.push(ContractViolation::new(
                "invalid_invocation_header",
                "invocation",
                "Quest Wi-Fi ADB invocation header is invalid",
            ));
        }
        for (path, value) in [
            ("request_id", self.request_id.as_str()),
            ("operation_id", self.operation_id.as_str()),
            ("preview_id", self.preview_id.as_str()),
            ("device_id", self.device_id.as_str()),
        ] {
            if !is_portable_id(value, 256) {
                failures.push(ContractViolation::new(
                    "invalid_identifier",
                    path,
                    "invocation identifiers must be bounded and portable",
                ));
            }
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestWifiAdbOwnerReceipt {
    pub schema: String,
    pub request_id: String,
    pub operation_id: String,
    pub preview_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub action: QuestWifiAdbAction,
    pub route_mode: QuestWifiAdbRouteMode,
    pub request_delivered: bool,
    pub kiosk_setting_applied: bool,
    pub request_after_boot_enabled: Option<bool>,
    pub wearer_approval: QuestWifiAdbWearerApproval,
    pub listener_discovered: bool,
    pub effect_applied: bool,
    pub outcome: String,
    pub evidence_sha256: String,
    pub observed_at_ms: i64,
}

impl QuestWifiAdbOwnerReceipt {
    #[must_use]
    pub fn derived_effect_applied(&self) -> bool {
        match self.action {
            QuestWifiAdbAction::Status => self.request_delivered,
            QuestWifiAdbAction::RequestWirelessAdb => {
                self.request_delivered
                    && self.kiosk_setting_applied
                    && matches!(
                        self.wearer_approval,
                        QuestWifiAdbWearerApproval::Pending | QuestWifiAdbWearerApproval::Accepted
                    )
            }
            QuestWifiAdbAction::EnableRequestAfterBoot => {
                self.request_delivered
                    && self.kiosk_setting_applied
                    && self.request_after_boot_enabled == Some(true)
            }
            QuestWifiAdbAction::DisableRequestAfterBoot => {
                self.request_delivered
                    && self.kiosk_setting_applied
                    && self.request_after_boot_enabled == Some(false)
            }
            QuestWifiAdbAction::DisableWirelessAdb => {
                self.request_delivered && self.kiosk_setting_applied
            }
            QuestWifiAdbAction::EnableClassicTcpipFromUsb => {
                self.request_delivered
                    && !self.kiosk_setting_applied
                    && self.route_mode == QuestWifiAdbRouteMode::ClassicTcpip
                    && self.listener_discovered
            }
        }
    }
}

impl ValidateContract for QuestWifiAdbOwnerReceipt {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != QUEST_WIFI_ADB_RECEIPT_SCHEMA
            || self.identity_revision == 0
            || self.observed_at_ms < 0
        {
            failures.push(ContractViolation::new(
                "invalid_receipt_header",
                "receipt",
                "Quest Wi-Fi ADB receipt header is invalid",
            ));
        }
        for (path, value) in [
            ("request_id", self.request_id.as_str()),
            ("operation_id", self.operation_id.as_str()),
            ("preview_id", self.preview_id.as_str()),
            ("device_id", self.device_id.as_str()),
        ] {
            if !is_portable_id(value, 256) {
                failures.push(ContractViolation::new(
                    "invalid_identifier",
                    path,
                    "receipt identifiers must be bounded and portable",
                ));
            }
        }
        if !is_lower_sha256(&self.evidence_sha256)
            || self.outcome.is_empty()
            || self.outcome.len() > 256
        {
            failures.push(ContractViolation::new(
                "invalid_receipt_evidence",
                "receipt",
                "receipt outcome and evidence digest must be bounded and valid",
            ));
        }
        if self.action.is_classic() {
            if self.route_mode != QuestWifiAdbRouteMode::ClassicTcpip
                || self.kiosk_setting_applied
                || self.request_after_boot_enabled.is_some()
                || self.wearer_approval != QuestWifiAdbWearerApproval::NotApplicable
            {
                failures.push(ContractViolation::new(
                    "classic_route_conflated",
                    "route_mode",
                    "classic USB tcpip must remain independent of Kiosk and modern TLS facts",
                ));
            }
        } else if self.route_mode == QuestWifiAdbRouteMode::ClassicTcpip {
            failures.push(ContractViolation::new(
                "modern_route_conflated",
                "route_mode",
                "modern Kiosk request actions cannot claim the classic USB tcpip route",
            ));
        }
        if self.action == QuestWifiAdbAction::RequestWirelessAdb
            && self.route_mode != QuestWifiAdbRouteMode::ModernTls
        {
            failures.push(ContractViolation::new(
                "request_route_invalid",
                "route_mode",
                "wireless debugging requests use the modern TLS route",
            ));
        }
        if self.wearer_approval == QuestWifiAdbWearerApproval::Accepted {
            failures.push(ContractViolation::new(
                "wearer_approval_not_observable",
                "wearer_approval",
                "File Manager and Kiosk setting readback cannot assert wearer acceptance",
            ));
        }
        if self.effect_applied != self.derived_effect_applied() {
            failures.push(ContractViolation::new(
                "effect_summary_mismatch",
                "effect_applied",
                "effect summary contradicts the independent action facts",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestWifiAdbTermuxProof {
    pub schema: String,
    pub proof_id: String,
    pub owner_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub source_epoch: String,
    pub source_revision: u64,
    pub evidence_revision: u64,
    pub route_mode: QuestWifiAdbRouteMode,
    pub discovery_mode: String,
    pub listener_discovered: bool,
    pub shell_identity: Option<String>,
    pub available: bool,
    pub evidence_sha256: String,
    pub observed_at_ms: i64,
    pub fresh_until_ms: i64,
}

impl ValidateContract for QuestWifiAdbTermuxProof {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != QUEST_WIFI_ADB_TERMUX_PROOF_SCHEMA
            || self.owner_id != QUEST_WIFI_ADB_TERMUX_PROOF_OWNER
            || self.identity_revision == 0
            || self.source_revision == 0
            || self.evidence_revision == 0
            || self.observed_at_ms < 0
            || self.fresh_until_ms <= self.observed_at_ms
            || self.fresh_until_ms - self.observed_at_ms > 60_000
            || !is_lower_sha256(&self.evidence_sha256)
            || !is_portable_id(&self.source_epoch, 256)
            || self.route_mode != QuestWifiAdbRouteMode::ModernTls
            || !matches!(self.discovery_mode.as_str(), "tls_nsd" | "tls_mdns")
            || self.available
                != (self.listener_discovered
                    && self.shell_identity.as_deref() == Some(TERMUX_ADB_SHELL_IDENTITY))
            || !self.available && (self.listener_discovered || self.shell_identity.is_some())
        {
            failures.push(ContractViolation::new(
                "invalid_termux_proof",
                "proof",
                "proof must bind a discovered route and exact uid=2000(shell) identity",
            ));
        }
        for (path, value) in [
            ("proof_id", self.proof_id.as_str()),
            ("device_id", self.device_id.as_str()),
        ] {
            if !is_portable_id(value, 256) {
                failures.push(ContractViolation::new(
                    "invalid_identifier",
                    path,
                    "proof identifiers must be bounded and portable",
                ));
            }
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestWifiAdbTargetLedger {
    pub device_id: String,
    pub identity_revision: u64,
    pub preflight: QuestWifiAdbTargetPreflight,
    pub lifecycle: CommandLifecycle,
    pub invocation: Option<QuestWifiAdbOwnerInvocation>,
    pub receipt: Option<QuestWifiAdbOwnerReceipt>,
    pub termux_proof: Option<QuestWifiAdbTermuxProof>,
    pub termux_usable: bool,
    pub failure_code: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestWifiAdbOperation {
    pub schema: String,
    pub operation_id: String,
    pub action_id: String,
    pub lifecycle: CommandLifecycle,
    pub preview: QuestWifiAdbPreview,
    pub confirmed_at_ms: Option<i64>,
    pub targets: Vec<QuestWifiAdbTargetLedger>,
    pub updated_at_ms: i64,
}

impl ValidateContract for QuestWifiAdbOperation {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != QUEST_WIFI_ADB_OPERATION_SCHEMA
            || self.operation_id != self.preview.operation_id
            || self.action_id != QUEST_WIFI_ADB_ACTION_ID
            || self.updated_at_ms < self.preview.created_at_ms
        {
            failures.push(ContractViolation::new(
                "invalid_operation_header",
                "operation",
                "Quest Wi-Fi ADB operation identity, action, or time is invalid",
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
                    "ledger target is absent from the immutable preview",
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
                    || invocation.action != self.preview.action)
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
                    .is_none_or(|receipt| !receipt.effect_applied)
            {
                failures.push(ContractViolation::new(
                    "applied_without_effective_receipt",
                    "targets.lifecycle",
                    "an applied target requires its exact effective owner receipt",
                ));
            }
            if let Some(proof) = &target.termux_proof
                && (proof.validate().is_err()
                    || proof.device_id != target.device_id
                    || proof.identity_revision != target.identity_revision
                    || target
                        .receipt
                        .as_ref()
                        .is_none_or(|receipt| receipt.route_mode != proof.route_mode))
            {
                failures.push(ContractViolation::new(
                    "termux_proof_binding_mismatch",
                    "targets.termux_proof",
                    "Termux proof differs from the exact operation and provider route",
                ));
            }
            if target.termux_usable
                && !target
                    .termux_proof
                    .as_ref()
                    .is_some_and(|proof| proof.available)
            {
                failures.push(ContractViolation::new(
                    "termux_usable_without_proof",
                    "targets.termux_usable",
                    "only an admitted exact uid=2000(shell) proof makes Termux usable",
                ));
            }
        }
        finish(failures)
    }
}

fn validate_targets(targets: &BTreeMap<String, u64>, failures: &mut Vec<ContractViolation>) {
    if targets.is_empty() || targets.len() > 10_000 {
        failures.push(ContractViolation::new(
            "invalid_target_count",
            "targets",
            "request must contain 1 through 10,000 exact targets",
        ));
    }
    for (device_id, revision) in targets {
        if !is_portable_id(device_id, 256) || *revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_target",
                "targets",
                "target IDs must be bounded portable identifiers with positive revisions",
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

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
