// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Versioned source-only contracts shared by the Hub, simulator, CLI, and
//! future operator projections.

mod capability;
mod checkin;
mod command;
mod condition;
mod identity;
mod projection;
mod query;
mod stream;

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
