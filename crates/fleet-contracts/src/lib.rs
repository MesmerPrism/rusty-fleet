// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Versioned source-only contracts shared by the Hub, simulator, CLI, and
//! future operator projections.

mod awake;
mod capability;
mod checkin;
mod command;
mod condition;
mod identity;
mod kiosk;
mod packages;
mod projection;
mod query;
mod stream;

pub use awake::{
    QUEST_AWAKE_ACTION_ID, QUEST_AWAKE_CAPABILITY_ID, QUEST_AWAKE_DEFAULT_WATCHDOG_INTERVAL_MS,
    QUEST_AWAKE_EXECUTE_REQUEST_SCHEMA, QUEST_AWAKE_MAX_DURATION_MS,
    QUEST_AWAKE_MAX_WATCHDOG_INTERVAL_MS, QUEST_AWAKE_MIN_DURATION_MS,
    QUEST_AWAKE_MIN_WATCHDOG_INTERVAL_MS, QUEST_AWAKE_OPERATION_SCHEMA, QUEST_AWAKE_OWNER,
    QUEST_AWAKE_PREVIEW_REQUEST_SCHEMA, QUEST_AWAKE_PROVIDER_CONTRACT, QUEST_AWAKE_RECEIPT_SCHEMA,
    QuestAwakeAction, QuestAwakeExecuteRequest, QuestAwakeOperation, QuestAwakeOwnerBinding,
    QuestAwakeOwnerInvocation, QuestAwakeOwnerReceipt, QuestAwakePowerReadback, QuestAwakePreview,
    QuestAwakePreviewRequest, QuestAwakeTargetLedger, QuestAwakeTargetPreflight,
    QuestAwakeWatchdogReadback,
};
pub use capability::{
    AuthorizationState, CapabilitySnapshot, CapabilityState, EnablementState, FreshnessState,
    ReachabilityState, SupportState,
};
pub use checkin::{
    CHECKIN_SIGNATURE_ALGORITHM, CHECKIN_SIGNATURE_DOMAIN, FleetCheckInClaims, SignedFleetCheckIn,
};
pub use command::{
    CommandLifecycle, OperationLedger, OperationTargetResult, TargetEligibility, TargetSnapshot,
};
pub use condition::{ConditionFamily, ConditionState, Sensitivity, StatusCondition, StatusSource};
pub use identity::{
    ApplicationLifecycle, ApplicationObservation, DeviceIdentity, DeviceObservation,
    FactProvenance, ForegroundAuthority, ForegroundState, KioskState, PowerObservation,
};
pub use kiosk::{
    KIOSK_CLI_RESULT_SCHEMA, KIOSK_DIRECT_OPERATOR_CAPABILITY_ID,
    KIOSK_DIRECT_OPERATOR_INVOKE_METHOD, KIOSK_DIRECT_OPERATOR_INVOKE_TARGET,
    KIOSK_DIRECT_OPERATOR_MAX_CLOCK_SKEW_SECONDS, KIOSK_DIRECT_OPERATOR_OWNER,
    KIOSK_DIRECT_OPERATOR_PORT, KIOSK_DIRECT_OPERATOR_REQUEST_AUTH,
    KIOSK_DIRECT_OPERATOR_RESPONSE_AUTH, KIOSK_DIRECT_OPERATOR_RESULT_METHOD,
    KIOSK_DIRECT_OPERATOR_RESULT_REQUEST_ID_PARAMETER, KIOSK_DIRECT_OPERATOR_RESULT_TARGET,
    KIOSK_DIRECT_OPERATOR_REVISION, KIOSK_DIRECT_OPERATOR_SCHEMA, KIOSK_SHOW_CONTROLS_ACTION_ID,
    KIOSK_SHOW_CONTROLS_COMMAND, KioskCancelDisposition, KioskEffectiveReceipt,
    KioskOwnerContractBinding, KioskRetryDisposition, KioskShowControlsOperation,
    KioskShowControlsPreview, KioskShowControlsTargetLedger, KioskShowControlsTargetPreflight,
    OPERATION_EXECUTE_REQUEST_SCHEMA, OPERATION_PREVIEW_REQUEST_SCHEMA, OperationExecuteRequest,
    OperationPreviewRequest,
};
pub use packages::{
    AuthenticatedPackageUpdaterAcknowledgement, AuthenticatedPackageUpdaterReceipt,
    ConsumedPackageUpdaterClaimIdentity, MAX_CONSUMED_PACKAGE_OWNER_CLAIMS,
    PACKAGE_INSTALL_EXECUTE_REQUEST_SCHEMA, PACKAGE_INSTALL_PREVIEW_REQUEST_SCHEMA,
    PACKAGE_UPDATE_MANIFEST_ENVELOPE_SCHEMA, PACKAGE_UPDATE_RECEIPT_SCHEMA,
    PACKAGE_UPDATER_ACK_SCHEMA, PACKAGE_UPDATER_CAPABILITY_ID,
    PACKAGE_UPDATER_CLAIM_REQUEST_SCHEMA, PACKAGE_UPDATER_CLAIM_SCHEMA,
    PACKAGE_UPDATER_OFFER_SCHEMA, PACKAGE_UPDATER_OWNER, PACKAGE_UPDATER_RECEIPT_SUBMISSION_SCHEMA,
    PACKAGES_INSTALL_RELEASE_ACTION_ID, PackageInstallReleaseExecuteRequest,
    PackageInstallReleaseOperation, PackageInstallReleasePreview,
    PackageInstallReleasePreviewRequest, PackageInstallStage, PackageInstallTargetLedger,
    PackageInstallTargetPreflight, PackageReleaseReference, PackageUpdateCheckpoint,
    PackageUpdateReceipt, PackageUpdateReceiptDecision, PackageUpdateReceiptStage,
    PackageUpdaterClaim, PackageUpdaterClaimRequest, PackageUpdaterEffectiveReceipt,
    PackageUpdaterInvocation, PackageUpdaterInvocationAcknowledgement, PackageUpdaterOffer,
    PackageUpdaterOwnerContractBinding, PackageUpdaterReceiptSubmission,
};
pub use projection::{
    DeviceDetailProjection, DeviceInspectorProjection, DeviceRowProjection, FleetQueryResult,
    FleetSummaryProjection, NavigationRestoration, ProjectionFreshness, SavedView,
    SavedViewCollection, SavedViewMutationReceipt, SavedViewMutationRequest,
    is_valid_saved_view_id,
};
pub use query::{
    Comparison, FleetQuery, QueryExpression, QueryField, QueryValue, SortDirection, SortKey,
};
pub use stream::{
    AdmissionBudget, CadenceMode, CadencePolicy, ComponentEpoch, ComponentEpochs,
    ContentProgressPolicy, EdgeQueuePolicy, EpochContinuity, ExperimentRun, NativeDescriptor,
    OverflowPolicy, ProgressApplicability, ProgressProfile, ProgressStage, ProgressStageEvidence,
    QueueLimits, RecordingArtifact, RecordingArtifactState, SelectionMethod, SourceSelection,
    StreamDescriptor, StreamPlane, StreamSemantic, TimingCorrelation, TimingDomain,
    TimingTransform,
};

use serde::{Deserialize, Serialize};

/// A stable machine-readable validation failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractViolation {
    pub code: String,
    pub path: String,
    pub message: String,
}

impl ContractViolation {
    #[must_use]
    pub fn new(code: &str, path: &str, message: &str) -> Self {
        Self {
            code: code.to_owned(),
            path: path.to_owned(),
            message: message.to_owned(),
        }
    }
}

/// Implemented by contracts that can reject invalid or unsafe states without
/// needing an adapter, device, clock, or network.
pub trait ValidateContract {
    fn validate(&self) -> Result<(), Vec<ContractViolation>>;
}

pub(crate) fn require_nonempty(failures: &mut Vec<ContractViolation>, value: &str, path: &str) {
    if value.trim().is_empty() {
        failures.push(ContractViolation::new(
            "required_text",
            path,
            "value must not be empty",
        ));
    }
}

pub(crate) fn finish(failures: Vec<ContractViolation>) -> Result<(), Vec<ContractViolation>> {
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}
