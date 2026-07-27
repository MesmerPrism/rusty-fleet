// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    AuthorizationState, CommandLifecycle, ContractViolation, EnablementState, FreshnessState,
    ReachabilityState, SupportState, ValidateContract, finish, require_nonempty,
};

pub const PACKAGES_INSTALL_RELEASE_ACTION_ID: &str = "packages.install-release";
pub const PACKAGE_UPDATER_CAPABILITY_ID: &str = "rusty-quest.package-updater";
pub const PACKAGE_UPDATER_OWNER: &str = "rusty-quest";
pub const PACKAGE_UPDATE_MANIFEST_ENVELOPE_SCHEMA: &str =
    "rusty.quest.package_update_manifest_envelope.v1";
pub const PACKAGE_UPDATE_RECEIPT_SCHEMA: &str = "rusty.quest.package_update_receipt.v1";
pub const PACKAGE_INSTALL_PREVIEW_REQUEST_SCHEMA: &str =
    "rusty.fleet.package_install_release_preview_request.v1";
pub const PACKAGE_INSTALL_EXECUTE_REQUEST_SCHEMA: &str =
    "rusty.fleet.package_install_release_execute_request.v1";
pub const PACKAGE_UPDATER_ACK_SCHEMA: &str =
    "rusty.fleet.package_updater_invocation_acknowledgement.v1";
pub const PACKAGE_UPDATER_RECEIPT_SUBMISSION_SCHEMA: &str =
    "rusty.fleet.package_updater_receipt_submission.v1";
pub const PACKAGE_UPDATER_CLAIM_REQUEST_SCHEMA: &str =
    "rusty.fleet.package_updater_claim_request.v1";
pub const PACKAGE_UPDATER_CLAIM_SCHEMA: &str = "rusty.fleet.package_updater_claim.v1";
pub const PACKAGE_UPDATER_OFFER_SCHEMA: &str = "rusty.fleet.package_updater_offer.v1";
pub const MAX_CONSUMED_PACKAGE_OWNER_CLAIMS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PackageReleaseReference {
    ManifestUrl { manifest_url: String },
    ReleaseId { release_id: String },
}

impl ValidateContract for PackageReleaseReference {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        match self {
            Self::ManifestUrl { manifest_url } => {
                if manifest_url.len() > 2_048
                    || !manifest_url.starts_with("https://")
                    || manifest_url.contains('#')
                    || manifest_url
                        .strip_prefix("https://")
                        .is_none_or(|authority| {
                            authority.is_empty()
                                || authority.starts_with('/')
                                || authority
                                    .split('/')
                                    .next()
                                    .is_some_and(|host| host.contains('@'))
                        })
                {
                    failures.push(ContractViolation::new(
                        "invalid_manifest_url",
                        "manifest_url",
                        "manifest URL must be a bounded HTTPS URL without credentials or a fragment",
                    ));
                }
            }
            Self::ReleaseId { release_id } => {
                if !(1..=256).contains(&release_id.len())
                    || !release_id.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/')
                    })
                {
                    failures.push(ContractViolation::new(
                        "invalid_release_id",
                        "release_id",
                        "release ID must contain 1 through 256 portable identifier bytes",
                    ));
                }
            }
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageInstallReleasePreviewRequest {
    pub schema: String,
    pub action_id: String,
    pub release: PackageReleaseReference,
    pub expected_package_name: String,
    pub expected_rollout_ring: String,
    pub targets: BTreeMap<String, u64>,
}

impl ValidateContract for PackageInstallReleasePreviewRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != PACKAGE_INSTALL_PREVIEW_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.package_install_release_preview_request.v1",
            ));
        }
        if self.action_id != PACKAGES_INSTALL_RELEASE_ACTION_ID {
            failures.push(ContractViolation::new(
                "wrong_action",
                "action_id",
                "expected packages.install-release",
            ));
        }
        if let Err(mut nested) = self.release.validate() {
            failures.append(&mut nested);
        }
        if !is_package_name(&self.expected_package_name) {
            failures.push(ContractViolation::new(
                "invalid_package_name",
                "expected_package_name",
                "expected package name must be a bounded dotted Android package identity",
            ));
        }
        if !is_portable_id(&self.expected_rollout_ring, 128) {
            failures.push(ContractViolation::new(
                "invalid_rollout_ring",
                "expected_rollout_ring",
                "expected rollout ring must be a bounded portable identifier",
            ));
        }
        validate_targets(&self.targets, &mut failures);
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageInstallReleaseExecuteRequest {
    pub schema: String,
    pub operation_id: String,
    pub preview_id: String,
}

impl ValidateContract for PackageInstallReleaseExecuteRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != PACKAGE_INSTALL_EXECUTE_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.package_install_release_execute_request.v1",
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
pub struct PackageUpdaterOwnerContractBinding {
    pub owner_repo_id: String,
    pub capability_id: String,
    pub manifest_envelope_schema: String,
    pub receipt_schema: String,
    pub install_mode: String,
    pub application_proof: String,
}

impl PackageUpdaterOwnerContractBinding {
    #[must_use]
    pub fn attended_v1() -> Self {
        Self {
            owner_repo_id: PACKAGE_UPDATER_OWNER.to_owned(),
            capability_id: PACKAGE_UPDATER_CAPABILITY_ID.to_owned(),
            manifest_envelope_schema: PACKAGE_UPDATE_MANIFEST_ENVELOPE_SCHEMA.to_owned(),
            receipt_schema: PACKAGE_UPDATE_RECEIPT_SCHEMA.to_owned(),
            install_mode: "attended_package_installer".to_owned(),
            application_proof: "effective_installed_version_receipt".to_owned(),
        }
    }
}

impl ValidateContract for PackageUpdaterOwnerContractBinding {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let expected = Self::attended_v1();
        if self == &expected {
            Ok(())
        } else {
            Err(vec![ContractViolation::new(
                "owner_contract_mismatch",
                "owner_contract",
                "package updater owner binding differs from the pinned attended updater contract",
            )])
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageInstallTargetPreflight {
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

impl PackageInstallTargetPreflight {
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

impl ValidateContract for PackageInstallTargetPreflight {
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
        if self.capability_id != PACKAGE_UPDATER_CAPABILITY_ID
            || self.capability_owner != PACKAGE_UPDATER_OWNER
        {
            failures.push(ContractViolation::new(
                "wrong_capability_owner",
                "capability_id",
                "package install preflight must use the Rusty Quest updater capability",
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
pub struct PackageInstallReleasePreview {
    pub schema: String,
    pub preview_id: String,
    pub operation_id: String,
    pub action_id: String,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub fleet_revision: u64,
    pub release: PackageReleaseReference,
    pub expected_package_name: String,
    pub expected_rollout_ring: String,
    pub owner_contract: PackageUpdaterOwnerContractBinding,
    pub targets: Vec<PackageInstallTargetPreflight>,
}

impl ValidateContract for PackageInstallReleasePreview {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.package_install_release_preview.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.package_install_release_preview.v1",
            ));
        }
        if !is_portable_id(&self.preview_id, 256)
            || !is_portable_id(&self.operation_id, 256)
            || self.action_id != PACKAGES_INSTALL_RELEASE_ACTION_ID
            || self.created_at_ms < 0
            || self.expires_at_ms <= self.created_at_ms
            || self.fleet_revision == 0
            || !is_package_name(&self.expected_package_name)
            || !is_portable_id(&self.expected_rollout_ring, 128)
        {
            failures.push(ContractViolation::new(
                "invalid_preview_header",
                "preview",
                "package preview identity, action, time, package, ring, or revision is invalid",
            ));
        }
        if let Err(mut nested) = self.release.validate() {
            failures.append(&mut nested);
        }
        if let Err(mut nested) = self.owner_contract.validate() {
            failures.append(&mut nested);
        }
        validate_preflights(
            &self.targets,
            self.created_at_ms,
            self.expires_at_ms,
            &mut failures,
        );
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdaterInvocation {
    pub schema: String,
    pub operation_id: String,
    pub preview_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub owner_action_request_id: String,
    pub release: PackageReleaseReference,
    pub expected_package_name: String,
    pub expected_rollout_ring: String,
    pub expires_at_ms: i64,
}

impl ValidateContract for PackageUpdaterInvocation {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.package_updater_invocation.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.package_updater_invocation.v1",
            ));
        }
        for (path, value) in [
            ("operation_id", self.operation_id.as_str()),
            ("preview_id", self.preview_id.as_str()),
            ("device_id", self.device_id.as_str()),
            (
                "owner_action_request_id",
                self.owner_action_request_id.as_str(),
            ),
        ] {
            if !is_portable_id(value, 256) {
                failures.push(ContractViolation::new(
                    "invalid_identifier",
                    path,
                    "invocation identifiers must be bounded portable identifiers",
                ));
            }
        }
        if self.identity_revision == 0
            || self.expires_at_ms < 0
            || !is_package_name(&self.expected_package_name)
            || !is_portable_id(&self.expected_rollout_ring, 128)
        {
            failures.push(ContractViolation::new(
                "invalid_invocation",
                "invocation",
                "invocation revision, expiry, package, or ring is invalid",
            ));
        }
        if let Err(mut nested) = self.release.validate() {
            failures.append(&mut nested);
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdaterInvocationAcknowledgement {
    pub schema: String,
    pub operation_id: String,
    pub device_id: String,
    pub owner_action_request_id: String,
    pub accepted: bool,
    pub code: String,
    pub acknowledged_at_ms: i64,
}

impl ValidateContract for PackageUpdaterInvocationAcknowledgement {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != PACKAGE_UPDATER_ACK_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.package_updater_invocation_acknowledgement.v1",
            ));
        }
        for (path, value) in [
            ("operation_id", self.operation_id.as_str()),
            ("device_id", self.device_id.as_str()),
            (
                "owner_action_request_id",
                self.owner_action_request_id.as_str(),
            ),
            ("code", self.code.as_str()),
        ] {
            if !is_portable_id(value, 256) {
                failures.push(ContractViolation::new(
                    "invalid_identifier",
                    path,
                    "acknowledgement identifiers and code must be bounded portable identifiers",
                ));
            }
        }
        if self.acknowledged_at_ms < 0 {
            failures.push(ContractViolation::new(
                "invalid_timestamp",
                "acknowledged_at_ms",
                "acknowledgement time must be nonnegative",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageUpdateReceiptStage {
    ManifestAdmission,
    InstallCommit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageUpdateReceiptDecision {
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdateCheckpoint {
    pub package_name: String,
    pub rollout_ring: String,
    pub sequence: u64,
    pub version_code: u64,
    pub signed_manifest_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdateReceipt {
    pub schema: String,
    pub stage: PackageUpdateReceiptStage,
    pub decision: PackageUpdateReceiptDecision,
    pub code: String,
    pub observed_at_ms: i64,
    pub envelope_sha256: Option<String>,
    pub signed_manifest_sha256: Option<String>,
    pub key_id: Option<String>,
    pub manifest_id: Option<String>,
    pub package_name: Option<String>,
    pub rollout_ring: Option<String>,
    pub sequence: Option<u64>,
    pub version_code: Option<u64>,
    pub prior_checkpoint: Option<PackageUpdateCheckpoint>,
    pub accepted_checkpoint: Option<PackageUpdateCheckpoint>,
    pub state_changed: bool,
}

impl PackageUpdateReceipt {
    #[must_use]
    pub fn proves_installed_version(&self, package_name: &str, rollout_ring: &str) -> bool {
        let Some(checkpoint) = &self.accepted_checkpoint else {
            return false;
        };
        self.schema == PACKAGE_UPDATE_RECEIPT_SCHEMA
            && self.stage == PackageUpdateReceiptStage::InstallCommit
            && self.decision == PackageUpdateReceiptDecision::Accepted
            && self.package_name.as_deref() == Some(package_name)
            && self.rollout_ring.as_deref() == Some(rollout_ring)
            && self.sequence == Some(checkpoint.sequence)
            && self.version_code == Some(checkpoint.version_code)
            && self.signed_manifest_sha256.as_deref()
                == Some(checkpoint.signed_manifest_sha256.as_str())
            && checkpoint.package_name == package_name
            && checkpoint.rollout_ring == rollout_ring
            && checkpoint.sequence > 0
            && checkpoint.version_code > 0
            && is_prefixed_sha256(&checkpoint.signed_manifest_sha256)
    }
}

impl ValidateContract for PackageUpdateReceipt {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != PACKAGE_UPDATE_RECEIPT_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.quest.package_update_receipt.v1",
            ));
        }
        if !is_portable_id(&self.code, 256) || self.observed_at_ms < 0 {
            failures.push(ContractViolation::new(
                "invalid_receipt_header",
                "receipt",
                "receipt code or observation time is invalid",
            ));
        }
        for (path, digest) in [
            ("envelope_sha256", self.envelope_sha256.as_deref()),
            (
                "signed_manifest_sha256",
                self.signed_manifest_sha256.as_deref(),
            ),
        ] {
            if digest.is_some_and(|value| !is_prefixed_sha256(value)) {
                failures.push(ContractViolation::new(
                    "invalid_sha256",
                    path,
                    "digest must use sha256: followed by lowercase hex",
                ));
            }
        }
        if let Some(checkpoint) = &self.accepted_checkpoint
            && (!is_package_name(&checkpoint.package_name)
                || !is_portable_id(&checkpoint.rollout_ring, 128)
                || checkpoint.sequence == 0
                || checkpoint.version_code == 0
                || !is_prefixed_sha256(&checkpoint.signed_manifest_sha256))
        {
            failures.push(ContractViolation::new(
                "invalid_checkpoint",
                "accepted_checkpoint",
                "accepted checkpoint is invalid",
            ));
        }
        if self.decision == PackageUpdateReceiptDecision::Rejected
            && (self.accepted_checkpoint.is_some() || self.state_changed)
        {
            failures.push(ContractViolation::new(
                "rejected_state_change",
                "state_changed",
                "a rejected owner receipt cannot advance updater state",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdaterEffectiveReceipt {
    pub schema: String,
    pub operation_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub owner_action_request_id: String,
    pub updater_receipt: PackageUpdateReceipt,
    pub wrapped_at_ms: i64,
}

impl PackageUpdaterEffectiveReceipt {
    pub fn validate_for(
        &self,
        invocation: &PackageUpdaterInvocation,
    ) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.package_updater_effective_receipt.v1"
            || self.operation_id != invocation.operation_id
            || self.device_id != invocation.device_id
            || self.identity_revision != invocation.identity_revision
            || self.owner_action_request_id != invocation.owner_action_request_id
            || self.wrapped_at_ms < 0
            || self.wrapped_at_ms > invocation.expires_at_ms
        {
            failures.push(ContractViolation::new(
                "effective_receipt_binding_mismatch",
                "effective_receipt",
                "effective receipt must bind the exact invocation and its deadline",
            ));
        }
        if let Err(mut nested) = self.updater_receipt.validate() {
            failures.append(&mut nested);
        }
        if !self.updater_receipt.proves_installed_version(
            &invocation.expected_package_name,
            &invocation.expected_rollout_ring,
        ) {
            failures.push(ContractViolation::new(
                "installed_version_not_proven",
                "updater_receipt",
                "only an accepted install_commit checkpoint for the expected package and ring proves application",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdaterReceiptSubmission {
    pub schema: String,
    pub effective_receipt: PackageUpdaterEffectiveReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdaterClaimRequest {
    pub schema: String,
    pub owner_id: String,
    pub request_id: String,
    pub operation_id: String,
    pub device_id: String,
    pub expected_invocation_sha256: String,
}

impl ValidateContract for PackageUpdaterClaimRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != PACKAGE_UPDATER_CLAIM_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.package_updater_claim_request.v1",
            ));
        }
        for (path, value) in [
            ("owner_id", self.owner_id.as_str()),
            ("request_id", self.request_id.as_str()),
            ("operation_id", self.operation_id.as_str()),
            ("device_id", self.device_id.as_str()),
        ] {
            if !is_portable_id(value, 256) {
                failures.push(ContractViolation::new(
                    "invalid_identifier",
                    path,
                    "owner claim identifiers must be bounded portable identifiers",
                ));
            }
        }
        if !is_prefixed_sha256(&self.expected_invocation_sha256) {
            failures.push(ContractViolation::new(
                "invalid_sha256",
                "expected_invocation_sha256",
                "expected invocation digest must use sha256: followed by lowercase hex",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdaterOffer {
    pub schema: String,
    pub owner_id: String,
    pub operation_id: String,
    pub device_id: String,
    pub invocation_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdaterClaim {
    pub schema: String,
    pub claim_id: String,
    pub owner_id: String,
    pub request_id: String,
    pub claimed_at_ms: i64,
    pub expires_at_ms: i64,
    pub invocation_sha256: String,
    pub release_sha256: String,
    pub target_sha256: String,
    pub invocation: PackageUpdaterInvocation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumedPackageUpdaterClaimIdentity {
    pub claim_id: String,
    pub request_id: String,
}

impl ValidateContract for PackageUpdaterClaim {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != PACKAGE_UPDATER_CLAIM_SCHEMA
            || self.claimed_at_ms < 0
            || self.expires_at_ms <= self.claimed_at_ms
            || self.expires_at_ms > self.invocation.expires_at_ms
        {
            failures.push(ContractViolation::new(
                "invalid_claim",
                "claim",
                "owner claim schema or bounded lifetime is invalid",
            ));
        }
        for (path, value) in [
            ("claim_id", self.claim_id.as_str()),
            ("owner_id", self.owner_id.as_str()),
            ("request_id", self.request_id.as_str()),
        ] {
            if !is_portable_id(value, 256) {
                failures.push(ContractViolation::new(
                    "invalid_identifier",
                    path,
                    "claim identifiers must be bounded portable identifiers",
                ));
            }
        }
        for (path, digest) in [
            ("invocation_sha256", self.invocation_sha256.as_str()),
            ("release_sha256", self.release_sha256.as_str()),
            ("target_sha256", self.target_sha256.as_str()),
        ] {
            if !is_prefixed_sha256(digest) {
                failures.push(ContractViolation::new(
                    "invalid_sha256",
                    path,
                    "claim digests must use sha256: followed by lowercase hex",
                ));
            }
        }
        if let Err(mut nested) = self.invocation.validate() {
            failures.append(&mut nested);
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedPackageUpdaterAcknowledgement {
    pub schema: String,
    pub owner_id: String,
    pub claim_id: String,
    pub invocation_sha256: String,
    pub acknowledgement: PackageUpdaterInvocationAcknowledgement,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedPackageUpdaterReceipt {
    pub schema: String,
    pub owner_id: String,
    pub claim_id: String,
    pub invocation_sha256: String,
    pub effective_receipt: PackageUpdaterEffectiveReceipt,
}

impl ValidateContract for PackageUpdaterReceiptSubmission {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        if self.schema == PACKAGE_UPDATER_RECEIPT_SUBMISSION_SCHEMA {
            Ok(())
        } else {
            Err(vec![ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.package_updater_receipt_submission.v1",
            )])
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageInstallStage {
    PreflightRejected,
    PreviewReady,
    Approved,
    DispatchReady,
    OwnerAcknowledged,
    Staged,
    AwaitingWearer,
    CancellationRequested,
    Cancelled,
    Applied,
    Failed,
    Expired,
    RecoveryRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageInstallTargetLedger {
    pub device_id: String,
    pub identity_revision: u64,
    pub preflight: PackageInstallTargetPreflight,
    pub lifecycle: CommandLifecycle,
    pub stage: PackageInstallStage,
    pub invocation: Option<PackageUpdaterInvocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_claim: Option<PackageUpdaterClaim>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prior_owner_claims: Vec<PackageUpdaterClaim>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumed_owner_claim_identities: Vec<ConsumedPackageUpdaterClaimIdentity>,
    pub invocation_acknowledgement: Option<PackageUpdaterInvocationAcknowledgement>,
    pub effective_receipt: Option<PackageUpdaterEffectiveReceipt>,
    pub reason_code: String,
    pub message: String,
    pub last_transition_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageInstallReleaseOperation {
    pub schema: String,
    pub operation_id: String,
    pub action_id: String,
    pub created_at_ms: i64,
    pub preview: PackageInstallReleasePreview,
    pub lifecycle: CommandLifecycle,
    pub max_parallelism: u16,
    pub cleanup_required: bool,
    pub targets: Vec<PackageInstallTargetLedger>,
}

impl PackageInstallReleaseOperation {
    #[must_use]
    pub fn derived_lifecycle(&self) -> CommandLifecycle {
        let eligible: Vec<_> = self
            .targets
            .iter()
            .filter(|target| target.preflight.eligible)
            .collect();
        if eligible.is_empty() {
            return CommandLifecycle::Rejected;
        }
        if eligible
            .iter()
            .any(|target| target.lifecycle == CommandLifecycle::Running)
        {
            return CommandLifecycle::Running;
        }
        if eligible
            .iter()
            .any(|target| target.lifecycle == CommandLifecycle::Dispatched)
        {
            return CommandLifecycle::Dispatched;
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
            .all(|target| target.lifecycle == CommandLifecycle::Expired)
        {
            return CommandLifecycle::Expired;
        }
        CommandLifecycle::Failed
    }
}

impl ValidateContract for PackageInstallReleaseOperation {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.package_install_release_operation.v1"
            || !is_portable_id(&self.operation_id, 256)
            || self.action_id != PACKAGES_INSTALL_RELEASE_ACTION_ID
            || self.created_at_ms < 0
            || !(1..=64).contains(&self.max_parallelism)
            || self.cleanup_required
        {
            failures.push(ContractViolation::new(
                "invalid_operation_header",
                "operation",
                "package operation identity, action, time, parallelism, or cleanup flag is invalid",
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
            || self.targets.len() != self.preview.targets.len()
        {
            failures.push(ContractViolation::new(
                "preview_binding_mismatch",
                "preview",
                "operation must retain its exact preview and target set",
            ));
        }
        let previews: BTreeMap<_, _> = self
            .preview
            .targets
            .iter()
            .map(|target| (target.device_id.as_str(), target))
            .collect();
        let mut seen = BTreeSet::new();
        for (index, target) in self.targets.iter().enumerate() {
            if !seen.insert(target.device_id.as_str())
                || previews
                    .get(target.device_id.as_str())
                    .is_none_or(|preview| **preview != target.preflight)
                || target.identity_revision != target.preflight.identity_revision
                || target.last_transition_ms < self.created_at_ms
                || !is_portable_id(&target.reason_code, 256)
                || target.message.is_empty()
            {
                failures.push(ContractViolation::new(
                    "invalid_target_ledger",
                    &format!("targets[{index}]"),
                    "target ledger must preserve one frozen preview target and bounded state",
                ));
            }
            if !target.preflight.eligible {
                if target.lifecycle != CommandLifecycle::Rejected
                    || target.stage != PackageInstallStage::PreflightRejected
                    || target.invocation.is_some()
                    || target.owner_claim.is_some()
                    || target.invocation_acknowledgement.is_some()
                    || target.effective_receipt.is_some()
                {
                    failures.push(ContractViolation::new(
                        "ineligible_target_dispatched",
                        &format!("targets[{index}]"),
                        "ineligible target must remain rejected without owner evidence",
                    ));
                }
                continue;
            }
            if target.prior_owner_claims.len() > 16
                || target.prior_owner_claims.iter().any(|claim| {
                    claim.validate().is_err()
                        || target.invocation.as_ref() != Some(&claim.invocation)
                })
            {
                failures.push(ContractViolation::new(
                    "invalid_prior_owner_claims",
                    &format!("targets[{index}].prior_owner_claims"),
                    "prior claim evidence must be bounded and bind the exact immutable invocation",
                ));
            }
            let consumed_identities_valid = target.consumed_owner_claim_identities.len()
                <= MAX_CONSUMED_PACKAGE_OWNER_CLAIMS
                && target
                    .consumed_owner_claim_identities
                    .iter()
                    .all(|identity| {
                        is_portable_id(&identity.claim_id, 256)
                            && is_portable_id(&identity.request_id, 256)
                    })
                && target
                    .consumed_owner_claim_identities
                    .iter()
                    .enumerate()
                    .all(|(identity_index, identity)| {
                        target.consumed_owner_claim_identities[..identity_index]
                            .iter()
                            .all(|prior| {
                                prior.claim_id != identity.claim_id
                                    && prior.request_id != identity.request_id
                            })
                    });
            if !consumed_identities_valid
                || target.prior_owner_claims.iter().any(|claim| {
                    !target
                        .consumed_owner_claim_identities
                        .iter()
                        .any(|identity| {
                            identity.claim_id == claim.claim_id
                                && identity.request_id == claim.request_id
                        })
                })
                || target.owner_claim.as_ref().is_some_and(|claim| {
                    !target
                        .consumed_owner_claim_identities
                        .iter()
                        .any(|identity| {
                            identity.claim_id == claim.claim_id
                                && identity.request_id == claim.request_id
                        })
                })
            {
                failures.push(ContractViolation::new(
                    "invalid_consumed_owner_claim_identities",
                    &format!("targets[{index}].consumed_owner_claim_identities"),
                    "durable replay authority must retain every unique consumed claim and request identity",
                ));
            }
            match target.lifecycle {
                CommandLifecycle::Proposed => {
                    if target.stage != PackageInstallStage::PreviewReady
                        || target.invocation.is_some()
                        || target.owner_claim.is_some()
                        || target.invocation_acknowledgement.is_some()
                        || target.effective_receipt.is_some()
                    {
                        failures.push(ContractViolation::new(
                            "invalid_pre_dispatch_state",
                            &format!("targets[{index}]"),
                            "pre-dispatch target cannot contain owner evidence",
                        ));
                    }
                }
                CommandLifecycle::Accepted => {
                    let valid_stage = match target.stage {
                        PackageInstallStage::Approved => target.invocation.is_none(),
                        PackageInstallStage::DispatchReady => target.invocation.is_some(),
                        _ => false,
                    };
                    if !valid_stage
                        || target.invocation_acknowledgement.is_some()
                        || target.effective_receipt.is_some()
                    {
                        failures.push(ContractViolation::new(
                            "invalid_accepted_state",
                            &format!("targets[{index}]"),
                            "accepted target must be approved or hold one not-yet-delivered invocation",
                        ));
                    }
                    if target.owner_claim.as_ref().is_some_and(|claim| {
                        claim.validate().is_err()
                            || target.invocation.as_ref() != Some(&claim.invocation)
                    }) {
                        failures.push(ContractViolation::new(
                            "owner_claim_binding_mismatch",
                            &format!("targets[{index}].owner_claim"),
                            "owner claim must bind the exact immutable invocation",
                        ));
                    }
                    if let Some(invocation) = &target.invocation
                        && (invocation.validate().is_err()
                            || invocation.operation_id != self.operation_id
                            || invocation.preview_id != self.preview.preview_id
                            || invocation.device_id != target.device_id
                            || invocation.identity_revision != target.identity_revision
                            || invocation.release != self.preview.release
                            || invocation.expected_package_name
                                != self.preview.expected_package_name
                            || invocation.expected_rollout_ring
                                != self.preview.expected_rollout_ring)
                    {
                        failures.push(ContractViolation::new(
                            "invocation_binding_mismatch",
                            &format!("targets[{index}].invocation"),
                            "prepared owner invocation must bind the exact frozen operation",
                        ));
                    }
                }
                CommandLifecycle::Dispatched => {
                    if target.stage != PackageInstallStage::OwnerAcknowledged {
                        failures.push(ContractViolation::new(
                            "invalid_inflight_stage",
                            &format!("targets[{index}].stage"),
                            "dispatched package work requires an authenticated owner acknowledgement",
                        ));
                    }
                    let Some(invocation) = &target.invocation else {
                        failures.push(ContractViolation::new(
                            "missing_invocation",
                            &format!("targets[{index}].invocation"),
                            "dispatched target requires the exact owner invocation",
                        ));
                        continue;
                    };
                    if invocation.validate().is_err()
                        || invocation.operation_id != self.operation_id
                        || invocation.preview_id != self.preview.preview_id
                        || invocation.device_id != target.device_id
                        || invocation.identity_revision != target.identity_revision
                        || invocation.release != self.preview.release
                        || invocation.expected_package_name != self.preview.expected_package_name
                        || invocation.expected_rollout_ring != self.preview.expected_rollout_ring
                        || target.effective_receipt.is_some()
                    {
                        failures.push(ContractViolation::new(
                            "invocation_binding_mismatch",
                            &format!("targets[{index}].invocation"),
                            "owner invocation must bind the exact frozen operation",
                        ));
                    }
                    match &target.invocation_acknowledgement {
                        Some(acknowledgement)
                            if acknowledgement.validate().is_ok()
                                && acknowledgement.accepted
                                && acknowledgement.operation_id == invocation.operation_id
                                && acknowledgement.device_id == invocation.device_id
                                && acknowledgement.owner_action_request_id
                                    == invocation.owner_action_request_id => {}
                        _ => failures.push(ContractViolation::new(
                            "dispatched_without_owner_acknowledgement",
                            &format!("targets[{index}].invocation_acknowledgement"),
                            "dispatched lifecycle requires a bound accepted owner acknowledgement",
                        )),
                    }
                }
                CommandLifecycle::Running => {
                    if !matches!(
                        target.stage,
                        PackageInstallStage::Staged
                            | PackageInstallStage::AwaitingWearer
                            | PackageInstallStage::CancellationRequested
                            | PackageInstallStage::RecoveryRequired
                    ) {
                        failures.push(ContractViolation::new(
                            "invalid_running_stage",
                            &format!("targets[{index}].stage"),
                            "running package work requires a later authenticated owner stage",
                        ));
                    }
                    let Some(invocation) = &target.invocation else {
                        failures.push(ContractViolation::new(
                            "missing_invocation",
                            &format!("targets[{index}].invocation"),
                            "running target requires the exact owner invocation",
                        ));
                        continue;
                    };
                    if invocation.validate().is_err()
                        || invocation.operation_id != self.operation_id
                        || invocation.preview_id != self.preview.preview_id
                        || invocation.device_id != target.device_id
                        || invocation.identity_revision != target.identity_revision
                        || invocation.release != self.preview.release
                        || invocation.expected_package_name != self.preview.expected_package_name
                        || invocation.expected_rollout_ring != self.preview.expected_rollout_ring
                        || target.effective_receipt.is_some()
                        || target.invocation_acknowledgement.as_ref().is_none_or(
                            |acknowledgement| {
                                acknowledgement.validate().is_err()
                                    || !acknowledgement.accepted
                                    || acknowledgement.operation_id != invocation.operation_id
                                    || acknowledgement.device_id != invocation.device_id
                                    || acknowledgement.owner_action_request_id
                                        != invocation.owner_action_request_id
                            },
                        )
                    {
                        failures.push(ContractViolation::new(
                            "running_owner_evidence_mismatch",
                            &format!("targets[{index}]"),
                            "running package work requires exact invocation and authenticated owner acknowledgement evidence",
                        ));
                    }
                }
                CommandLifecycle::Applied => {
                    if target.stage != PackageInstallStage::Applied {
                        failures.push(ContractViolation::new(
                            "invalid_applied_stage",
                            &format!("targets[{index}].stage"),
                            "applied lifecycle requires the applied package stage",
                        ));
                    }
                    match (&target.invocation, &target.effective_receipt) {
                        (Some(invocation), Some(receipt)) => {
                            if receipt.validate_for(invocation).is_err() {
                                failures.push(ContractViolation::new(
                                    "applied_without_effective_receipt",
                                    &format!("targets[{index}].effective_receipt"),
                                    "applied target requires an effective installed-version receipt",
                                ));
                            }
                        }
                        _ => failures.push(ContractViolation::new(
                            "applied_without_effective_receipt",
                            &format!("targets[{index}].effective_receipt"),
                            "applied target requires invocation and installed-version receipt",
                        )),
                    }
                }
                CommandLifecycle::Failed | CommandLifecycle::Expired => {
                    let expected_stage = if target.lifecycle == CommandLifecycle::Expired {
                        PackageInstallStage::Expired
                    } else {
                        PackageInstallStage::Failed
                    };
                    if target.stage != expected_stage
                        || target.invocation.is_none()
                        || target.effective_receipt.is_some()
                    {
                        failures.push(ContractViolation::new(
                            "invalid_terminal_failure",
                            &format!("targets[{index}]"),
                            "failed or expired target retains an invocation and no effective receipt",
                        ));
                    }
                }
                CommandLifecycle::Rejected => failures.push(ContractViolation::new(
                    "eligible_target_rejected",
                    &format!("targets[{index}].lifecycle"),
                    "eligible target cannot use preflight rejection",
                )),
                CommandLifecycle::CancellationRequested => {
                    if target.stage != PackageInstallStage::CancellationRequested
                        || target.invocation.is_none()
                        || target.effective_receipt.is_some()
                    {
                        failures.push(ContractViolation::new(
                            "invalid_cancellation_state",
                            &format!("targets[{index}]"),
                            "cancellation requires a prepared invocation and no application proof",
                        ));
                    }
                }
                CommandLifecycle::Cancelled => {
                    if target.stage != PackageInstallStage::Cancelled
                        || target.invocation.is_none()
                        || target.effective_receipt.is_some()
                    {
                        failures.push(ContractViolation::new(
                            "invalid_cancelled_state",
                            &format!("targets[{index}]"),
                            "cancelled package work retains its invocation and no application proof",
                        ));
                    }
                }
                CommandLifecycle::CleanupPending | CommandLifecycle::Cleaned => {
                    failures.push(ContractViolation::new(
                        "unsupported_lifecycle",
                        &format!("targets[{index}].lifecycle"),
                        "package install v1 has no separate cleanup lifecycle",
                    ));
                }
            }
        }
        if self.lifecycle != self.derived_lifecycle() {
            failures.push(ContractViolation::new(
                "operation_lifecycle_mismatch",
                "lifecycle",
                "operation lifecycle must be derived from target ledgers",
            ));
        }
        let occupied_delivery_slots = self
            .targets
            .iter()
            .filter(|target| {
                matches!(
                    target.lifecycle,
                    CommandLifecycle::Dispatched | CommandLifecycle::Running
                ) || (target.lifecycle == CommandLifecycle::Accepted
                    && target.owner_claim.is_some())
            })
            .count();
        if occupied_delivery_slots > usize::from(self.max_parallelism) {
            failures.push(ContractViolation::new(
                "operation_parallelism_exceeded",
                "targets",
                "active owner claims and in-flight targets exceed the frozen parallelism",
            ));
        }
        finish(failures)
    }
}

fn validate_targets(targets: &BTreeMap<String, u64>, failures: &mut Vec<ContractViolation>) {
    if targets.is_empty() || targets.len() > 10_000 {
        failures.push(ContractViolation::new(
            "invalid_target_count",
            "targets",
            "operation previews require 1 through 10000 targets",
        ));
    }
    for (device_id, identity_revision) in targets {
        if !is_portable_id(device_id, 256) || *identity_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_target",
                "targets",
                "target device IDs and identity revisions must be valid",
            ));
        }
    }
}

fn validate_preflights(
    targets: &[PackageInstallTargetPreflight],
    created_at_ms: i64,
    expires_at_ms: i64,
    failures: &mut Vec<ContractViolation>,
) {
    if targets.is_empty() || targets.len() > 10_000 {
        failures.push(ContractViolation::new(
            "invalid_target_count",
            "targets",
            "package previews require 1 through 10000 targets",
        ));
    }
    let mut last: Option<&str> = None;
    for (index, target) in targets.iter().enumerate() {
        if let Err(nested) = target.validate() {
            failures.extend(nested.into_iter().map(|failure| ContractViolation {
                path: format!("targets[{index}].{}", failure.path),
                ..failure
            }));
        }
        if target.evaluated_at_ms < created_at_ms
            || target.evaluated_at_ms > expires_at_ms
            || last.is_some_and(|previous| previous >= target.device_id.as_str())
        {
            failures.push(ContractViolation::new(
                "noncanonical_preflight",
                &format!("targets[{index}]"),
                "preflights must be sorted, unique, and evaluated inside the preview window",
            ));
        }
        last = Some(target.device_id.as_str());
    }
}

fn is_package_name(value: &str) -> bool {
    (3..=255).contains(&value.len())
        && value.contains('.')
        && value.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

fn is_portable_id(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

fn is_prefixed_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}
