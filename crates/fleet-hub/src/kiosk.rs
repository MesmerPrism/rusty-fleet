// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{
    AuthorizationState, CommandLifecycle, EnablementState, FreshnessState,
    KIOSK_DIRECT_OPERATOR_CAPABILITY_ID, KIOSK_DIRECT_OPERATOR_OWNER,
    KIOSK_SHOW_CONTROLS_ACTION_ID, KioskCancelDisposition, KioskEffectiveReceipt,
    KioskOwnerContractBinding, KioskRetryDisposition, KioskShowControlsOperation,
    KioskShowControlsPreview, KioskShowControlsTargetLedger, KioskShowControlsTargetPreflight,
    OperationExecuteRequest, OperationPreviewRequest, ReachabilityState, SupportState,
    ValidateContract,
};
use serde::{Deserialize, Serialize};

use crate::{DeviceRecord, FleetHub, HubError, MAX_KIOSK_OPERATIONS};

const PREVIEW_MAX_LIFETIME_MS: i64 = 90_000;

/// A Hub-owned request to freeze one exact Kiosk show-controls operation.
///
/// This is an in-process planning input rather than an owner wire contract.
/// The caller supplies stable identifiers so durable replay can be idempotent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KioskShowControlsPreviewPlan {
    pub operation_id: String,
    pub preview_id: String,
    pub request: OperationPreviewRequest,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub max_parallelism: u16,
    pub max_attempts_per_target: u8,
}

impl FleetHub {
    /// Freezes the exact target identities and current Kiosk capability facts.
    ///
    /// Replaying the same operation ID is idempotent only when the complete
    /// resulting operation is identical.
    pub fn preview_kiosk_show_controls(
        &mut self,
        plan: KioskShowControlsPreviewPlan,
    ) -> Result<KioskShowControlsOperation, HubError> {
        self.validate_preview_plan(&plan)?;
        if let Some(existing) = self.kiosk_operations.get(&plan.operation_id) {
            return if plan_matches_operation(&plan, existing) {
                Ok(existing.clone())
            } else {
                Err(HubError::new(
                    "kiosk_operation_id_conflict",
                    "operation ID already names a different immutable preview",
                ))
            };
        }

        let mut preflights = Vec::with_capacity(plan.request.targets.len());
        for (device_id, identity_revision) in &plan.request.targets {
            let record = self.devices.get(device_id).ok_or_else(|| {
                HubError::new(
                    "kiosk_target_not_found",
                    format!("unknown Kiosk operation target {device_id}"),
                )
            })?;
            if record.observation.identity.identity_revision != *identity_revision {
                return Err(HubError::new(
                    "kiosk_target_identity_mismatch",
                    format!(
                        "target {device_id} identity revision changed from requested {identity_revision} to {}",
                        record.observation.identity.identity_revision
                    ),
                ));
            }
            preflights.push(self.kiosk_preflight(record, plan.created_at_ms));
        }

        let preview = KioskShowControlsPreview {
            schema: "rusty.fleet.kiosk_show_controls_preview.v1".to_owned(),
            preview_id: plan.preview_id,
            operation_id: plan.operation_id.clone(),
            action_id: KIOSK_SHOW_CONTROLS_ACTION_ID.to_owned(),
            created_at_ms: plan.created_at_ms,
            expires_at_ms: plan.expires_at_ms,
            fleet_revision: self.result_revision,
            owner_contract: KioskOwnerContractBinding::show_controls_v1(),
            targets: preflights,
        };
        let targets = preview
            .targets
            .iter()
            .cloned()
            .map(|preflight| {
                let eligible = preflight.eligible;
                KioskShowControlsTargetLedger {
                    device_id: preflight.device_id.clone(),
                    identity_revision: preflight.identity_revision,
                    preflight,
                    lifecycle: if eligible {
                        CommandLifecycle::Proposed
                    } else {
                        CommandLifecycle::Rejected
                    },
                    dispatched_at_ms: None,
                    owner_deadline_at_ms: None,
                    attempt_count: 0,
                    owner_request_ids: Vec::new(),
                    owner_request_id: None,
                    effective_receipt: None,
                    retry_disposition: KioskRetryDisposition::NotEligible,
                    cancel_disposition: if eligible {
                        KioskCancelDisposition::CancelableBeforeDispatch
                    } else {
                        KioskCancelDisposition::Terminal
                    },
                    reason_code: if eligible {
                        "preview_ready".to_owned()
                    } else {
                        "preflight_excluded".to_owned()
                    },
                    message: if eligible {
                        "Target is ready for explicit confirmation".to_owned()
                    } else {
                        "Target was excluded by the frozen preflight".to_owned()
                    },
                    last_transition_ms: plan.created_at_ms,
                }
            })
            .collect();
        let mut operation = KioskShowControlsOperation {
            schema: "rusty.fleet.kiosk_show_controls_operation.v1".to_owned(),
            operation_id: plan.operation_id,
            action_id: KIOSK_SHOW_CONTROLS_ACTION_ID.to_owned(),
            created_at_ms: plan.created_at_ms,
            preview,
            lifecycle: CommandLifecycle::Proposed,
            max_parallelism: plan.max_parallelism,
            max_attempts_per_target: plan.max_attempts_per_target,
            cleanup_required: false,
            targets,
        };
        operation.lifecycle = derive_operation_lifecycle(&operation);
        validate_operation(&operation)?;

        if self.kiosk_operations.len() >= MAX_KIOSK_OPERATIONS {
            return Err(HubError::new(
                "kiosk_operation_limit_reached",
                format!("Fleet Hub retains at most {MAX_KIOSK_OPERATIONS} Kiosk operations"),
            ));
        }
        self.kiosk_operations
            .insert(operation.operation_id.clone(), operation.clone());
        Ok(operation)
    }

    /// Confirms the exact preview without recomputing target membership.
    ///
    /// Every selected target's identity and capability facts are checked
    /// against the frozen preflight. Any drift rejects the whole confirmation.
    pub fn confirm_kiosk_show_controls(
        &mut self,
        operation_id: &str,
        preview_id: &str,
        now_ms: i64,
    ) -> Result<KioskShowControlsOperation, HubError> {
        let operation = self.kiosk_operation(operation_id)?;
        require_preview(&operation, preview_id)?;
        require_monotonic_time(&operation, now_ms)?;
        if operation.lifecycle != CommandLifecycle::Proposed {
            return Ok(operation);
        }
        if now_ms > operation.preview.expires_at_ms {
            return Err(HubError::new(
                "kiosk_preview_expired",
                "the immutable Kiosk preview expired before confirmation",
            ));
        }
        self.require_frozen_targets_current(&operation, now_ms)?;

        let mut updated = operation;
        for target in &mut updated.targets {
            if target.preflight.eligible {
                target.lifecycle = CommandLifecycle::Accepted;
                target.reason_code = "operator_confirmed".to_owned();
                target.message = "Exact preview confirmed for dispatch".to_owned();
                target.last_transition_ms = now_ms;
            }
        }
        updated.lifecycle = derive_operation_lifecycle(&updated);
        validate_operation(&updated)?;
        self.kiosk_operations
            .insert(operation_id.to_owned(), updated.clone());
        Ok(updated)
    }

    pub fn confirm_kiosk_show_controls_request(
        &mut self,
        request: &OperationExecuteRequest,
        now_ms: i64,
    ) -> Result<KioskShowControlsOperation, HubError> {
        request.validate().map_err(|failures| {
            contract_error(
                "kiosk_execute_request_invalid",
                "Kiosk operation execute request",
                failures,
            )
        })?;
        self.confirm_kiosk_show_controls(&request.operation_id, &request.preview_id, now_ms)
    }

    pub fn kiosk_operation(
        &self,
        operation_id: &str,
    ) -> Result<KioskShowControlsOperation, HubError> {
        self.kiosk_operations
            .get(operation_id)
            .cloned()
            .ok_or_else(|| {
                HubError::new(
                    "kiosk_operation_not_found",
                    format!("unknown Kiosk operation {operation_id}"),
                )
            })
    }

    #[must_use]
    pub fn kiosk_operations(&self) -> Vec<KioskShowControlsOperation> {
        let mut operations = self.kiosk_operations.values().cloned().collect::<Vec<_>>();
        operations.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.operation_id.cmp(&right.operation_id))
        });
        operations
    }

    /// Starts one owner attempt while enforcing the operation concurrency and
    /// per-target attempt limits.
    pub fn dispatch_kiosk_show_controls(
        &mut self,
        operation_id: &str,
        device_id: &str,
        owner_request_id: String,
        now_ms: i64,
    ) -> Result<KioskShowControlsOperation, HubError> {
        let operation = self.kiosk_operation(operation_id)?;
        require_monotonic_time(&operation, now_ms)?;
        let target = target(&operation, device_id)?;
        if matches!(
            target.lifecycle,
            CommandLifecycle::Dispatched | CommandLifecycle::Running | CommandLifecycle::Applied
        ) && target.owner_request_id.as_deref() == Some(owner_request_id.as_str())
        {
            return Ok(operation);
        }
        if now_ms > operation.preview.expires_at_ms {
            return Err(HubError::new(
                "kiosk_preview_expired",
                "Kiosk owner dispatch cannot begin after preview expiry",
            ));
        }
        if !matches!(
            target.lifecycle,
            CommandLifecycle::Accepted | CommandLifecycle::Failed | CommandLifecycle::Expired
        ) {
            return Err(HubError::new(
                "kiosk_dispatch_state_invalid",
                format!("target {device_id} is not eligible for a new owner attempt"),
            ));
        }
        if target.attempt_count >= operation.max_attempts_per_target {
            return Err(HubError::new(
                "kiosk_attempt_limit_reached",
                format!("target {device_id} reached its bounded owner attempt limit"),
            ));
        }
        if target.owner_request_ids.contains(&owner_request_id) {
            return Err(HubError::new(
                "kiosk_owner_request_reused",
                "a retry must use a new Kiosk owner action request ID",
            ));
        }
        if !is_owner_request_id(&owner_request_id) {
            return Err(HubError::new(
                "kiosk_owner_request_invalid",
                "Kiosk owner action request IDs contain 8 through 64 ASCII letters, digits, underscores, or hyphens",
            ));
        }
        let inflight = operation
            .targets
            .iter()
            .filter(|candidate| {
                matches!(
                    candidate.lifecycle,
                    CommandLifecycle::Dispatched | CommandLifecycle::Running
                )
            })
            .count();
        if inflight >= usize::from(operation.max_parallelism) {
            return Err(HubError::new(
                "kiosk_parallelism_limit_reached",
                "the operation already has its maximum number of owner attempts in flight",
            ));
        }
        self.require_target_current(target, now_ms)?;

        let mut updated = operation;
        let max_attempts = updated.max_attempts_per_target;
        let owner_deadline_at_ms = updated.preview.expires_at_ms;
        let target = target_mut(&mut updated, device_id)?;
        target.attempt_count = target.attempt_count.saturating_add(1);
        target.owner_request_ids.push(owner_request_id.clone());
        target.owner_request_id = Some(owner_request_id);
        target.dispatched_at_ms = Some(now_ms);
        target.owner_deadline_at_ms = Some(owner_deadline_at_ms);
        target.effective_receipt = None;
        target.lifecycle = CommandLifecycle::Dispatched;
        target.retry_disposition = KioskRetryDisposition::NotEligible;
        target.cancel_disposition = KioskCancelDisposition::NotCancelableAfterDispatch;
        target.reason_code = "owner_dispatch_started".to_owned();
        target.message = format!(
            "Kiosk owner attempt {} of {max_attempts} was dispatched",
            target.attempt_count
        );
        target.last_transition_ms = now_ms;
        updated.lifecycle = derive_operation_lifecycle(&updated);
        validate_operation(&updated)?;
        self.kiosk_operations
            .insert(operation_id.to_owned(), updated.clone());
        Ok(updated)
    }

    pub fn mark_kiosk_show_controls_running(
        &mut self,
        operation_id: &str,
        device_id: &str,
        now_ms: i64,
    ) -> Result<KioskShowControlsOperation, HubError> {
        self.transition_inflight(
            operation_id,
            device_id,
            now_ms,
            CommandLifecycle::Running,
            "owner_result_pending",
            "Kiosk accepted the request; signed effective readback is pending",
        )
    }

    /// Applies a target only from an exact, owner-authenticated effective
    /// receipt. An invoke acknowledgement is never sufficient.
    pub fn apply_kiosk_show_controls_receipt(
        &mut self,
        receipt: KioskEffectiveReceipt,
        now_ms: i64,
    ) -> Result<KioskShowControlsOperation, HubError> {
        let operation = self.kiosk_operation(&receipt.operation_id)?;
        require_monotonic_time(&operation, now_ms)?;
        let target = target(&operation, &receipt.device_id)?;
        if target.lifecycle == CommandLifecycle::Applied
            && target.effective_receipt.as_ref() == Some(&receipt)
        {
            return Ok(operation);
        }
        if !matches!(
            target.lifecycle,
            CommandLifecycle::Dispatched | CommandLifecycle::Running
        ) {
            return Err(HubError::new(
                "kiosk_receipt_state_invalid",
                "effective Kiosk receipt arrived for a target without an owner attempt in flight",
            ));
        }
        receipt.validate().map_err(|failures| {
            contract_error("kiosk_receipt_invalid", "effective Kiosk receipt", failures)
        })?;
        if receipt.device_id != target.device_id
            || receipt.identity_revision != target.identity_revision
            || receipt.owner_action_request_id != target.owner_request_id.as_deref().unwrap_or("")
        {
            return Err(HubError::new(
                "kiosk_receipt_binding_mismatch",
                "effective Kiosk receipt does not bind the exact frozen target and owner attempt",
            ));
        }

        let operation_id = receipt.operation_id.clone();
        let device_id = receipt.device_id.clone();
        let mut updated = operation;
        let target = target_mut(&mut updated, &device_id)?;
        target.effective_receipt = Some(receipt);
        target.lifecycle = CommandLifecycle::Applied;
        target.retry_disposition = KioskRetryDisposition::NotEligible;
        target.cancel_disposition = KioskCancelDisposition::Terminal;
        target.reason_code = "owner_effective_receipt".to_owned();
        target.message =
            "Kiosk signed result confirms accepted, completed, controls-open state".to_owned();
        target.last_transition_ms = now_ms;
        updated.lifecycle = derive_operation_lifecycle(&updated);
        validate_operation(&updated)?;
        self.kiosk_operations.insert(operation_id, updated.clone());
        Ok(updated)
    }

    pub fn fail_kiosk_show_controls(
        &mut self,
        operation_id: &str,
        device_id: &str,
        reason_code: &str,
        message: &str,
        now_ms: i64,
    ) -> Result<KioskShowControlsOperation, HubError> {
        self.finish_owner_attempt(
            operation_id,
            device_id,
            reason_code,
            message,
            now_ms,
            CommandLifecycle::Failed,
        )
    }

    pub fn expire_kiosk_show_controls(
        &mut self,
        operation_id: &str,
        device_id: &str,
        reason_code: &str,
        message: &str,
        now_ms: i64,
    ) -> Result<KioskShowControlsOperation, HubError> {
        self.finish_owner_attempt(
            operation_id,
            device_id,
            reason_code,
            message,
            now_ms,
            CommandLifecycle::Expired,
        )
    }

    /// Records a cancellation request only while no owner request exists.
    pub fn request_kiosk_show_controls_cancellation(
        &mut self,
        operation_id: &str,
        device_id: &str,
        now_ms: i64,
    ) -> Result<KioskShowControlsOperation, HubError> {
        let operation = self.kiosk_operation(operation_id)?;
        require_monotonic_time(&operation, now_ms)?;
        let target = target(&operation, device_id)?;
        if matches!(
            target.lifecycle,
            CommandLifecycle::CancellationRequested | CommandLifecycle::Cancelled
        ) {
            return Ok(operation);
        }
        if target.lifecycle != CommandLifecycle::Accepted {
            return Err(HubError::new(
                "kiosk_target_not_cancelable",
                "confirmed show-controls can be cancelled only before owner dispatch",
            ));
        }
        let mut updated = operation;
        let target = target_mut(&mut updated, device_id)?;
        target.lifecycle = CommandLifecycle::CancellationRequested;
        target.cancel_disposition = KioskCancelDisposition::CancellationRequestedBeforeDispatch;
        target.reason_code = "cancellation_requested".to_owned();
        target.message = "Cancellation requested before owner dispatch".to_owned();
        target.last_transition_ms = now_ms;
        updated.lifecycle = derive_operation_lifecycle(&updated);
        validate_operation(&updated)?;
        self.kiosk_operations
            .insert(operation_id.to_owned(), updated.clone());
        Ok(updated)
    }

    pub fn complete_kiosk_show_controls_cancellation(
        &mut self,
        operation_id: &str,
        device_id: &str,
        now_ms: i64,
    ) -> Result<KioskShowControlsOperation, HubError> {
        let operation = self.kiosk_operation(operation_id)?;
        require_monotonic_time(&operation, now_ms)?;
        let target = target(&operation, device_id)?;
        if target.lifecycle == CommandLifecycle::Cancelled {
            return Ok(operation);
        }
        if target.lifecycle != CommandLifecycle::CancellationRequested {
            return Err(HubError::new(
                "kiosk_cancellation_state_invalid",
                "target has no pre-dispatch cancellation request to complete",
            ));
        }
        let mut updated = operation;
        let target = target_mut(&mut updated, device_id)?;
        target.lifecycle = CommandLifecycle::Cancelled;
        target.cancel_disposition = KioskCancelDisposition::CancelledBeforeDispatch;
        target.reason_code = "cancelled_before_dispatch".to_owned();
        target.message = "Cancelled before any owner request was sent".to_owned();
        target.last_transition_ms = now_ms;
        updated.lifecycle = derive_operation_lifecycle(&updated);
        validate_operation(&updated)?;
        self.kiosk_operations
            .insert(operation_id.to_owned(), updated.clone());
        Ok(updated)
    }

    #[must_use]
    pub fn kiosk_operations_for_device(&self, device_id: &str) -> Vec<KioskShowControlsOperation> {
        let mut operations = self
            .kiosk_operations
            .values()
            .filter(|operation| {
                operation
                    .targets
                    .iter()
                    .any(|target| target.device_id == device_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        operations.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.operation_id.cmp(&right.operation_id))
        });
        operations
    }

    pub(crate) fn active_kiosk_operation_count(&self, device_id: &str) -> usize {
        self.kiosk_operations
            .values()
            .filter(|operation| {
                !is_kiosk_terminal(operation.lifecycle)
                    && operation.targets.iter().any(|target| {
                        target.device_id == device_id && !is_kiosk_terminal(target.lifecycle)
                    })
            })
            .count()
    }

    fn validate_preview_plan(&self, plan: &KioskShowControlsPreviewPlan) -> Result<(), HubError> {
        if plan.operation_id.trim().is_empty()
            || plan.preview_id.trim().is_empty()
            || plan.operation_id.len() > 256
            || plan.preview_id.len() > 256
        {
            return Err(HubError::new(
                "kiosk_preview_id_invalid",
                "operation and preview IDs contain 1 through 256 bytes",
            ));
        }
        plan.request.validate().map_err(|failures| {
            contract_error(
                "kiosk_preview_request_invalid",
                "Kiosk operation preview request",
                failures,
            )
        })?;
        if plan.created_at_ms < 0
            || plan.expires_at_ms <= plan.created_at_ms
            || plan.expires_at_ms.saturating_sub(plan.created_at_ms) > PREVIEW_MAX_LIFETIME_MS
        {
            return Err(HubError::new(
                "kiosk_preview_window_invalid",
                "Kiosk preview lifetime must be positive and no longer than 90 seconds",
            ));
        }
        if !(1..=64).contains(&plan.max_parallelism) {
            return Err(HubError::new(
                "kiosk_parallelism_invalid",
                "Kiosk operation parallelism must be between 1 and 64",
            ));
        }
        if !(1..=8).contains(&plan.max_attempts_per_target) {
            return Err(HubError::new(
                "kiosk_attempt_limit_invalid",
                "Kiosk operation attempts per target must be between 1 and 8",
            ));
        }
        Ok(())
    }

    fn kiosk_preflight(
        &self,
        record: &DeviceRecord,
        evaluated_at_ms: i64,
    ) -> KioskShowControlsTargetPreflight {
        let capability = record
            .observation
            .capabilities
            .get(KIOSK_DIRECT_OPERATOR_CAPABILITY_ID);
        let (
            capability_evidence_revision,
            support,
            enablement,
            authorization,
            mut reachability,
            mut freshness,
            observed_at_ms,
            fresh_until_ms,
            message,
        ) = match capability {
            Some(capability) if capability.owner == KIOSK_DIRECT_OPERATOR_OWNER => (
                capability.evidence_revision,
                capability.support,
                capability.enablement,
                capability.authorization,
                capability.reachability,
                capability.freshness,
                capability.observed_at_ms,
                capability.fresh_until_ms,
                capability.reason.clone(),
            ),
            Some(capability) => (
                capability.evidence_revision,
                SupportState::Unknown,
                EnablementState::Unknown,
                AuthorizationState::Unknown,
                ReachabilityState::Unknown,
                FreshnessState::Unknown,
                capability.observed_at_ms,
                capability.fresh_until_ms,
                "Kiosk capability evidence has the wrong owner".to_owned(),
            ),
            None => (
                record.observation.source_revision,
                SupportState::Unknown,
                EnablementState::Unknown,
                AuthorizationState::Unknown,
                ReachabilityState::Unknown,
                FreshnessState::Unknown,
                record.observation.received_time_ms,
                record.observation.received_time_ms,
                "No Kiosk direct-operator capability evidence is present".to_owned(),
            ),
        };
        let observation_age_ms = evaluated_at_ms.saturating_sub(record.accepted_at_ms).max(0);
        if observation_age_ms > self.policy.offline_after_ms {
            reachability = ReachabilityState::Disconnected;
        }
        if observation_age_ms > self.policy.stale_after_ms {
            freshness = FreshnessState::Stale;
        }
        let mut preflight = KioskShowControlsTargetPreflight {
            device_id: record.observation.identity.device_id.clone(),
            identity_revision: record.observation.identity.identity_revision,
            capability_id: KIOSK_DIRECT_OPERATOR_CAPABILITY_ID.to_owned(),
            capability_evidence_revision,
            capability_owner: KIOSK_DIRECT_OPERATOR_OWNER.to_owned(),
            support,
            enablement,
            authorization,
            reachability,
            freshness,
            observed_at_ms,
            fresh_until_ms,
            evaluated_at_ms,
            eligible: false,
            reason_code: String::new(),
            message,
        };
        preflight.eligible = preflight.expected_eligibility();
        preflight.reason_code = preflight.expected_reason_code().to_owned();
        preflight
    }

    fn require_frozen_targets_current(
        &self,
        operation: &KioskShowControlsOperation,
        now_ms: i64,
    ) -> Result<(), HubError> {
        for target in &operation.targets {
            self.require_target_current(target, now_ms)?;
        }
        Ok(())
    }

    fn require_target_current(
        &self,
        target: &KioskShowControlsTargetLedger,
        now_ms: i64,
    ) -> Result<(), HubError> {
        let record = self.devices.get(&target.device_id).ok_or_else(|| {
            HubError::new(
                "kiosk_target_not_found",
                format!(
                    "Kiosk target {} is no longer in the Fleet directory",
                    target.device_id
                ),
            )
        })?;
        if record.observation.identity.identity_revision != target.identity_revision {
            return Err(HubError::new(
                "kiosk_target_identity_changed",
                format!(
                    "Kiosk target {} changed identity revision",
                    target.device_id
                ),
            ));
        }
        let current = self.kiosk_preflight(record, now_ms);
        if !same_frozen_facts(&target.preflight, &current)
            || target.preflight.eligible != current.expected_eligibility()
            || target.preflight.reason_code != current.expected_reason_code()
        {
            return Err(HubError::new(
                "kiosk_target_changed_since_preview",
                format!(
                    "Kiosk target {} capability or freshness facts changed since preview",
                    target.device_id
                ),
            ));
        }
        Ok(())
    }

    fn transition_inflight(
        &mut self,
        operation_id: &str,
        device_id: &str,
        now_ms: i64,
        lifecycle: CommandLifecycle,
        reason_code: &str,
        message: &str,
    ) -> Result<KioskShowControlsOperation, HubError> {
        let operation = self.kiosk_operation(operation_id)?;
        require_monotonic_time(&operation, now_ms)?;
        let target = target(&operation, device_id)?;
        if target.lifecycle == lifecycle {
            return Ok(operation);
        }
        if target.lifecycle != CommandLifecycle::Dispatched {
            return Err(HubError::new(
                "kiosk_running_state_invalid",
                "only a dispatched Kiosk target can become running",
            ));
        }
        let mut updated = operation;
        let target = target_mut(&mut updated, device_id)?;
        target.lifecycle = lifecycle;
        target.reason_code = reason_code.to_owned();
        target.message = message.to_owned();
        target.last_transition_ms = now_ms;
        updated.lifecycle = derive_operation_lifecycle(&updated);
        validate_operation(&updated)?;
        self.kiosk_operations
            .insert(operation_id.to_owned(), updated.clone());
        Ok(updated)
    }

    fn finish_owner_attempt(
        &mut self,
        operation_id: &str,
        device_id: &str,
        reason_code: &str,
        message: &str,
        now_ms: i64,
        lifecycle: CommandLifecycle,
    ) -> Result<KioskShowControlsOperation, HubError> {
        if reason_code.trim().is_empty() || message.trim().is_empty() {
            return Err(HubError::new(
                "kiosk_terminal_reason_invalid",
                "failed or expired Kiosk attempts require a reason and message",
            ));
        }
        let operation = self.kiosk_operation(operation_id)?;
        require_monotonic_time(&operation, now_ms)?;
        let target = target(&operation, device_id)?;
        if target.lifecycle == lifecycle
            && target.reason_code == reason_code
            && target.message == message
        {
            return Ok(operation);
        }
        if !matches!(
            target.lifecycle,
            CommandLifecycle::Dispatched | CommandLifecycle::Running
        ) {
            return Err(HubError::new(
                "kiosk_terminal_state_invalid",
                "only an in-flight Kiosk owner attempt can fail or expire",
            ));
        }
        let mut updated = operation;
        let max_attempts = updated.max_attempts_per_target;
        let target = target_mut(&mut updated, device_id)?;
        target.lifecycle = lifecycle;
        target.retry_disposition = if target.attempt_count < max_attempts {
            KioskRetryDisposition::NewOwnerRequestRequired
        } else {
            KioskRetryDisposition::NotEligible
        };
        target.cancel_disposition = KioskCancelDisposition::Terminal;
        target.reason_code = reason_code.to_owned();
        target.message = message.to_owned();
        target.last_transition_ms = now_ms;
        updated.lifecycle = derive_operation_lifecycle(&updated);
        validate_operation(&updated)?;
        self.kiosk_operations
            .insert(operation_id.to_owned(), updated.clone());
        Ok(updated)
    }
}

fn target<'a>(
    operation: &'a KioskShowControlsOperation,
    device_id: &str,
) -> Result<&'a KioskShowControlsTargetLedger, HubError> {
    operation
        .targets
        .iter()
        .find(|target| target.device_id == device_id)
        .ok_or_else(|| {
            HubError::new(
                "kiosk_operation_target_not_found",
                format!(
                    "device {device_id} is not in operation {}",
                    operation.operation_id
                ),
            )
        })
}

fn plan_matches_operation(
    plan: &KioskShowControlsPreviewPlan,
    operation: &KioskShowControlsOperation,
) -> bool {
    plan.operation_id == operation.operation_id
        && plan.preview_id == operation.preview.preview_id
        && plan.created_at_ms == operation.created_at_ms
        && plan.expires_at_ms == operation.preview.expires_at_ms
        && plan.max_parallelism == operation.max_parallelism
        && plan.max_attempts_per_target == operation.max_attempts_per_target
        && plan.request.action_id == operation.action_id
        && plan.request.targets
            == operation
                .preview
                .targets
                .iter()
                .map(|target| (target.device_id.clone(), target.identity_revision))
                .collect()
}

fn target_mut<'a>(
    operation: &'a mut KioskShowControlsOperation,
    device_id: &str,
) -> Result<&'a mut KioskShowControlsTargetLedger, HubError> {
    let operation_id = operation.operation_id.clone();
    operation
        .targets
        .iter_mut()
        .find(|target| target.device_id == device_id)
        .ok_or_else(|| {
            HubError::new(
                "kiosk_operation_target_not_found",
                format!("device {device_id} is not in operation {operation_id}"),
            )
        })
}

fn require_preview(
    operation: &KioskShowControlsOperation,
    preview_id: &str,
) -> Result<(), HubError> {
    if operation.preview.preview_id != preview_id {
        return Err(HubError::new(
            "kiosk_preview_mismatch",
            "execute request does not bind the operation's immutable preview",
        ));
    }
    Ok(())
}

fn require_monotonic_time(
    operation: &KioskShowControlsOperation,
    now_ms: i64,
) -> Result<(), HubError> {
    if now_ms < operation.created_at_ms
        || operation
            .targets
            .iter()
            .any(|target| now_ms < target.last_transition_ms)
    {
        return Err(HubError::new(
            "kiosk_operation_time_regression",
            "Kiosk operation transition time must not regress",
        ));
    }
    Ok(())
}

fn same_frozen_facts(
    frozen: &KioskShowControlsTargetPreflight,
    current: &KioskShowControlsTargetPreflight,
) -> bool {
    frozen.device_id == current.device_id
        && frozen.identity_revision == current.identity_revision
        && frozen.capability_id == current.capability_id
        && frozen.capability_evidence_revision == current.capability_evidence_revision
        && frozen.capability_owner == current.capability_owner
        && frozen.support == current.support
        && frozen.enablement == current.enablement
        && frozen.authorization == current.authorization
        && frozen.reachability == current.reachability
        && frozen.freshness == current.freshness
        && frozen.observed_at_ms == current.observed_at_ms
        && frozen.fresh_until_ms == current.fresh_until_ms
        && frozen.message == current.message
}

fn derive_operation_lifecycle(operation: &KioskShowControlsOperation) -> CommandLifecycle {
    operation.derived_lifecycle()
}

fn is_kiosk_terminal(lifecycle: CommandLifecycle) -> bool {
    matches!(
        lifecycle,
        CommandLifecycle::Rejected
            | CommandLifecycle::Applied
            | CommandLifecycle::Failed
            | CommandLifecycle::Expired
            | CommandLifecycle::Cancelled
    )
}

fn is_owner_request_id(value: &str) -> bool {
    (8..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn validate_operation(operation: &KioskShowControlsOperation) -> Result<(), HubError> {
    operation.validate().map_err(|failures| {
        contract_error(
            "kiosk_operation_invalid",
            "Kiosk show-controls operation",
            failures,
        )
    })
}

fn contract_error(
    code: &str,
    subject: &str,
    failures: Vec<fleet_contracts::ContractViolation>,
) -> HubError {
    HubError::new(
        code,
        format!(
            "{subject} failed contract validation: {}",
            failures
                .into_iter()
                .map(|failure| format!("{}:{}", failure.code, failure.path))
                .collect::<Vec<_>>()
                .join(",")
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fleet_contracts::{CapabilityState, KIOSK_SHOW_CONTROLS_COMMAND, KioskEffectiveReceipt};
    use fleet_simulator::{BASE_TIME_MS, ScenarioBuilder};

    use super::*;
    use crate::{FleetApi, HubPolicy, ObservationDecision};

    fn hub_with_kiosk_targets(ready: usize, excluded: usize) -> FleetHub {
        let total = ready + excluded;
        let mut hub = FleetHub::new(HubPolicy::default());
        for (index, mut observation) in ScenarioBuilder::new(total)
            .build()
            .initial
            .into_iter()
            .enumerate()
        {
            let is_ready = index < ready;
            observation.capabilities.capabilities.insert(
                KIOSK_DIRECT_OPERATOR_CAPABILITY_ID.to_owned(),
                CapabilityState {
                    capability_id: KIOSK_DIRECT_OPERATOR_CAPABILITY_ID.to_owned(),
                    support: SupportState::Supported,
                    enablement: if is_ready {
                        EnablementState::Enabled
                    } else {
                        EnablementState::Disabled
                    },
                    authorization: AuthorizationState::Authorized,
                    reachability: ReachabilityState::Reachable,
                    freshness: FreshnessState::Current,
                    evidence_revision: observation.source_revision,
                    observed_at_ms: BASE_TIME_MS,
                    fresh_until_ms: BASE_TIME_MS + 60_000,
                    owner: KIOSK_DIRECT_OPERATOR_OWNER.to_owned(),
                    reason: if is_ready {
                        "owner_ready".to_owned()
                    } else {
                        "owner_disabled".to_owned()
                    },
                    extensions: BTreeMap::new(),
                },
            );
            assert!(matches!(
                hub.accept_observation(observation, BASE_TIME_MS),
                ObservationDecision::Accepted { .. }
            ));
        }
        hub
    }

    fn plan(
        hub: &FleetHub,
        targets: &[&str],
        max_parallelism: u16,
    ) -> KioskShowControlsPreviewPlan {
        let target_identity_revisions = targets
            .iter()
            .map(|device_id| {
                (
                    (*device_id).to_owned(),
                    hub.inspect(device_id, BASE_TIME_MS)
                        .expect("target")
                        .row
                        .identity
                        .identity_revision,
                )
            })
            .collect();
        KioskShowControlsPreviewPlan {
            operation_id: "operation-kiosk-0001".to_owned(),
            preview_id: "preview-kiosk-0001".to_owned(),
            request: OperationPreviewRequest {
                schema: "rusty.fleet.operation_preview_request.v1".to_owned(),
                action_id: KIOSK_SHOW_CONTROLS_ACTION_ID.to_owned(),
                targets: target_identity_revisions,
            },
            created_at_ms: BASE_TIME_MS + 1,
            expires_at_ms: BASE_TIME_MS + 60_000,
            max_parallelism,
            max_attempts_per_target: 2,
        }
    }

    fn effective_receipt() -> KioskEffectiveReceipt {
        KioskEffectiveReceipt {
            schema: "rusty.fleet.kiosk_effective_receipt.v1".to_owned(),
            receipt_id: "receipt-kiosk-0001".to_owned(),
            operation_id: "operation-kiosk-0001".to_owned(),
            device_id: "sim-00001".to_owned(),
            identity_revision: 1,
            owner_contract: KioskOwnerContractBinding::show_controls_v1(),
            owner_action_request_id: "owner-action-0001".to_owned(),
            owner_result_transport_request_id: "owner-result-0001".to_owned(),
            owner_command: KIOSK_SHOW_CONTROLS_COMMAND.to_owned(),
            response_status: 200,
            response_content_sha256: "a".repeat(64),
            response_signature: "b".repeat(64),
            response_auth_verified: true,
            owner_result_schema: "rusty.kiosk.cli_result.v1".to_owned(),
            owner_accepted: true,
            owner_completed: true,
            owner_recorded_at_ms: BASE_TIME_MS + 4,
            controls_open: true,
            wrapped_at_ms: BASE_TIME_MS + 5,
        }
    }

    #[test]
    fn preview_freezes_mixed_target_facts_and_confirmation_is_exact() {
        let mut hub = hub_with_kiosk_targets(1, 1);
        let plan = plan(&hub, &["sim-00001", "sim-00002"], 2);
        let operation = hub
            .preview_kiosk_show_controls(plan.clone())
            .expect("preview");
        assert_eq!(operation.lifecycle, CommandLifecycle::Proposed);
        assert!(operation.targets[0].preflight.eligible);
        assert_eq!(operation.targets[1].lifecycle, CommandLifecycle::Rejected);
        assert_eq!(operation.targets[1].preflight.reason_code, "disabled");
        assert_eq!(
            hub.preview_kiosk_show_controls(plan).expect("idempotent"),
            operation
        );

        let confirmed = hub
            .confirm_kiosk_show_controls(
                &operation.operation_id,
                &operation.preview.preview_id,
                BASE_TIME_MS + 2,
            )
            .expect("confirm");
        assert_eq!(confirmed.lifecycle, CommandLifecycle::Accepted);
        assert_eq!(confirmed.targets[0].lifecycle, CommandLifecycle::Accepted);
        assert_eq!(confirmed.targets[1].lifecycle, CommandLifecycle::Rejected);

        let restored =
            FleetHub::restore(HubPolicy::default(), hub.snapshot()).expect("durable restore");
        assert_eq!(
            restored
                .kiosk_operation("operation-kiosk-0001")
                .expect("restored operation"),
            confirmed
        );
        assert_eq!(
            restored
                .inspect("sim-00001", BASE_TIME_MS + 2)
                .expect("inspector")
                .row
                .active_work_count,
            1
        );
    }

    #[test]
    fn confirmation_rejects_identity_or_capability_drift_without_mutation() {
        let mut hub = hub_with_kiosk_targets(1, 0);
        let operation = hub
            .preview_kiosk_show_controls(plan(&hub, &["sim-00001"], 1))
            .expect("preview");

        let mut changed = ScenarioBuilder::new(1).build().initial.remove(0);
        changed.source_revision = 2;
        changed.source_time_ms = BASE_TIME_MS + 2;
        changed.received_time_ms = BASE_TIME_MS + 2;
        changed.capabilities.capabilities.insert(
            KIOSK_DIRECT_OPERATOR_CAPABILITY_ID.to_owned(),
            CapabilityState {
                capability_id: KIOSK_DIRECT_OPERATOR_CAPABILITY_ID.to_owned(),
                support: SupportState::Supported,
                enablement: EnablementState::Disabled,
                authorization: AuthorizationState::Authorized,
                reachability: ReachabilityState::Reachable,
                freshness: FreshnessState::Current,
                evidence_revision: 2,
                observed_at_ms: BASE_TIME_MS + 2,
                fresh_until_ms: BASE_TIME_MS + 60_000,
                owner: KIOSK_DIRECT_OPERATOR_OWNER.to_owned(),
                reason: "owner_disabled".to_owned(),
                extensions: BTreeMap::new(),
            },
        );
        assert!(matches!(
            hub.accept_observation(changed, BASE_TIME_MS + 2),
            ObservationDecision::Accepted { .. }
        ));
        assert_eq!(
            hub.confirm_kiosk_show_controls(
                &operation.operation_id,
                &operation.preview.preview_id,
                BASE_TIME_MS + 3,
            )
            .expect_err("changed facts")
            .code,
            "kiosk_target_changed_since_preview"
        );
        assert_eq!(
            hub.kiosk_operation(&operation.operation_id)
                .expect("unchanged")
                .lifecycle,
            CommandLifecycle::Proposed
        );
    }

    #[test]
    fn dispatch_retry_parallelism_and_cancellation_are_bounded() {
        let mut hub = hub_with_kiosk_targets(2, 0);
        let operation = hub
            .preview_kiosk_show_controls(plan(&hub, &["sim-00001", "sim-00002"], 1))
            .expect("preview");
        hub.confirm_kiosk_show_controls(
            &operation.operation_id,
            &operation.preview.preview_id,
            BASE_TIME_MS + 2,
        )
        .expect("confirm");
        let before_invalid_dispatch = hub
            .kiosk_operation(&operation.operation_id)
            .expect("confirmed operation");
        assert_eq!(
            hub.dispatch_kiosk_show_controls(
                &operation.operation_id,
                "sim-00001",
                "bad".to_owned(),
                BASE_TIME_MS + 3,
            )
            .expect_err("invalid owner request")
            .code,
            "kiosk_owner_request_invalid"
        );
        assert_eq!(
            hub.kiosk_operation(&operation.operation_id)
                .expect("operation remains valid"),
            before_invalid_dispatch
        );
        hub.dispatch_kiosk_show_controls(
            &operation.operation_id,
            "sim-00001",
            "owner-action-0001".to_owned(),
            BASE_TIME_MS + 3,
        )
        .expect("dispatch");
        assert_eq!(
            hub.dispatch_kiosk_show_controls(
                &operation.operation_id,
                "sim-00002",
                "owner-action-0002".to_owned(),
                BASE_TIME_MS + 3,
            )
            .expect_err("bounded parallelism")
            .code,
            "kiosk_parallelism_limit_reached"
        );
        assert_eq!(
            hub.request_kiosk_show_controls_cancellation(
                &operation.operation_id,
                "sim-00001",
                BASE_TIME_MS + 4,
            )
            .expect_err("post-dispatch cancellation")
            .code,
            "kiosk_target_not_cancelable"
        );
        hub.fail_kiosk_show_controls(
            &operation.operation_id,
            "sim-00001",
            "owner_timeout",
            "Owner readback timed out",
            BASE_TIME_MS + 4,
        )
        .expect("fail");
        assert_eq!(
            hub.dispatch_kiosk_show_controls(
                &operation.operation_id,
                "sim-00001",
                "owner-action-0001".to_owned(),
                BASE_TIME_MS + 5,
            )
            .expect_err("request ID replay")
            .code,
            "kiosk_owner_request_reused"
        );
        let retried = hub
            .dispatch_kiosk_show_controls(
                &operation.operation_id,
                "sim-00001",
                "owner-action-0003".to_owned(),
                BASE_TIME_MS + 5,
            )
            .expect("bounded retry");
        assert_eq!(retried.targets[0].attempt_count, 2);
        hub.expire_kiosk_show_controls(
            &operation.operation_id,
            "sim-00001",
            "owner_result_expired",
            "Owner result did not arrive before its deadline",
            BASE_TIME_MS + 6,
        )
        .expect("expire");
        assert_eq!(
            hub.dispatch_kiosk_show_controls(
                &operation.operation_id,
                "sim-00001",
                "owner-action-0004".to_owned(),
                BASE_TIME_MS + 7,
            )
            .expect_err("attempt bound")
            .code,
            "kiosk_attempt_limit_reached"
        );
        hub.request_kiosk_show_controls_cancellation(
            &operation.operation_id,
            "sim-00002",
            BASE_TIME_MS + 7,
        )
        .expect("cancel request");
        let cancelled = hub
            .complete_kiosk_show_controls_cancellation(
                &operation.operation_id,
                "sim-00002",
                BASE_TIME_MS + 8,
            )
            .expect("cancel complete");
        assert_eq!(cancelled.lifecycle, CommandLifecycle::Failed);
        assert_eq!(cancelled.targets[1].lifecycle, CommandLifecycle::Cancelled);
    }

    #[test]
    fn only_an_exact_effective_owner_receipt_can_mark_applied() {
        let mut hub = hub_with_kiosk_targets(1, 0);
        let operation = hub
            .preview_kiosk_show_controls(plan(&hub, &["sim-00001"], 1))
            .expect("preview");
        hub.confirm_kiosk_show_controls(
            &operation.operation_id,
            &operation.preview.preview_id,
            BASE_TIME_MS + 2,
        )
        .expect("confirm");
        hub.dispatch_kiosk_show_controls(
            &operation.operation_id,
            "sim-00001",
            "owner-action-0001".to_owned(),
            BASE_TIME_MS + 3,
        )
        .expect("dispatch");
        let running = hub
            .mark_kiosk_show_controls_running(
                &operation.operation_id,
                "sim-00001",
                BASE_TIME_MS + 4,
            )
            .expect("running");
        assert_eq!(running.lifecycle, CommandLifecycle::Running);

        let mut mismatched = effective_receipt();
        mismatched.owner_action_request_id = "owner-action-wrong".to_owned();
        assert_eq!(
            hub.apply_kiosk_show_controls_receipt(mismatched, BASE_TIME_MS + 6)
                .expect_err("receipt binding")
                .code,
            "kiosk_receipt_binding_mismatch"
        );
        let applied = hub
            .apply_kiosk_show_controls_receipt(effective_receipt(), BASE_TIME_MS + 6)
            .expect("effective receipt");
        assert_eq!(applied.lifecycle, CommandLifecycle::Applied);
        assert_eq!(applied.targets[0].lifecycle, CommandLifecycle::Applied);
        assert_eq!(
            hub.inspect("sim-00001", BASE_TIME_MS + 6)
                .expect("inspector")
                .row
                .active_work_count,
            0
        );
    }

    #[test]
    fn operation_listing_is_deterministic_and_restore_rejects_target_identity_drift() {
        let mut hub = hub_with_kiosk_targets(1, 0);
        let mut later_plan = plan(&hub, &["sim-00001"], 1);
        later_plan.operation_id = "operation-kiosk-0002".to_owned();
        later_plan.preview_id = "preview-kiosk-0002".to_owned();
        later_plan.created_at_ms = BASE_TIME_MS + 2;
        let later = hub
            .preview_kiosk_show_controls(later_plan)
            .expect("later preview");
        let earlier = hub
            .preview_kiosk_show_controls(plan(&hub, &["sim-00001"], 1))
            .expect("earlier preview");
        assert_eq!(
            hub.kiosk_operations()
                .iter()
                .map(|operation| operation.operation_id.as_str())
                .collect::<Vec<_>>(),
            vec![earlier.operation_id.as_str(), later.operation_id.as_str()]
        );

        let mut changed = ScenarioBuilder::new(1).build().initial.remove(0);
        changed.identity.identity_revision = 2;
        changed.source_revision = 1;
        changed.source_time_ms = BASE_TIME_MS + 3;
        changed.received_time_ms = BASE_TIME_MS + 3;
        assert!(matches!(
            hub.accept_observation(changed, BASE_TIME_MS + 3),
            ObservationDecision::Accepted { .. }
        ));
        assert_eq!(
            FleetHub::restore(HubPolicy::default(), hub.snapshot())
                .expect_err("persisted operation target identity must match")
                .code,
            "snapshot_kiosk_operations_invalid"
        );

        let mut missing = hub_with_kiosk_targets(1, 0);
        missing
            .preview_kiosk_show_controls(plan(&missing, &["sim-00001"], 1))
            .expect("preview");
        let mut snapshot = missing.snapshot();
        snapshot.devices.remove("sim-00001");
        assert_eq!(
            FleetHub::restore(HubPolicy::default(), snapshot)
                .expect_err("persisted operation target must exist")
                .code,
            "snapshot_kiosk_operations_invalid"
        );
    }

    #[test]
    fn restore_preserves_terminal_historical_operation_across_identity_change() {
        let mut hub = hub_with_kiosk_targets(1, 0);
        let operation = hub
            .preview_kiosk_show_controls(plan(&hub, &["sim-00001"], 1))
            .expect("preview");
        hub.confirm_kiosk_show_controls(
            &operation.operation_id,
            &operation.preview.preview_id,
            BASE_TIME_MS + 2,
        )
        .expect("confirm");
        hub.dispatch_kiosk_show_controls(
            &operation.operation_id,
            "sim-00001",
            "owner-action-0001".to_owned(),
            BASE_TIME_MS + 3,
        )
        .expect("dispatch");
        hub.apply_kiosk_show_controls_receipt(effective_receipt(), BASE_TIME_MS + 6)
            .expect("terminal receipt");

        let mut changed = ScenarioBuilder::new(1).build().initial.remove(0);
        changed.identity.identity_revision = 2;
        changed.source_revision = 1;
        changed.source_time_ms = BASE_TIME_MS + 7;
        changed.received_time_ms = BASE_TIME_MS + 7;
        assert!(matches!(
            hub.accept_observation(changed, BASE_TIME_MS + 7),
            ObservationDecision::Accepted { .. }
        ));
        let restored =
            FleetHub::restore(HubPolicy::default(), hub.snapshot()).expect("historical restore");
        assert_eq!(
            restored
                .kiosk_operation(&operation.operation_id)
                .expect("historical operation")
                .lifecycle,
            CommandLifecycle::Applied
        );
    }
}
