// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{
    AuthorizationState, CommandLifecycle, EnablementState, FreshnessState, QUEST_AWAKE_ACTION_ID,
    QUEST_AWAKE_CAPABILITY_ID, QUEST_AWAKE_OPERATION_SCHEMA, QUEST_AWAKE_OWNER,
    QuestAwakeOperation, QuestAwakeOwnerBinding, QuestAwakeOwnerInvocation, QuestAwakeOwnerReceipt,
    QuestAwakePreview, QuestAwakePreviewRequest, QuestAwakeTargetLedger, QuestAwakeTargetPreflight,
    ReachabilityState, SupportState, ValidateContract,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::{DeviceRecord, FleetHub, HubError, MAX_AWAKE_OPERATIONS};

const PREVIEW_MAX_LIFETIME_MS: i64 = 15 * 60 * 1_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestAwakePreviewPlan {
    pub operation_id: String,
    pub preview_id: String,
    pub watchdog_generation: String,
    pub request: QuestAwakePreviewRequest,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub provider_ready_devices: BTreeSet<String>,
}

impl FleetHub {
    pub fn preview_quest_awake(
        &mut self,
        plan: QuestAwakePreviewPlan,
    ) -> Result<QuestAwakeOperation, HubError> {
        if plan.request.validate().is_err()
            || plan.created_at_ms < 0
            || plan.expires_at_ms <= plan.created_at_ms
            || plan.expires_at_ms - plan.created_at_ms > PREVIEW_MAX_LIFETIME_MS
            || plan.operation_id.is_empty()
            || plan.preview_id.is_empty()
            || plan.watchdog_generation.is_empty()
        {
            return Err(HubError::new(
                "awake_preview_plan_invalid",
                "Quest awake preview plan is invalid or exceeds its bounded lifetime",
            ));
        }
        if let Some(existing) = self.awake_operations.get(&plan.operation_id) {
            return if existing.preview.preview_id == plan.preview_id
                && existing.preview.action == plan.request.action
                && existing.preview.duration_ms == plan.request.duration_ms
                && existing.preview.watchdog_interval_ms == plan.request.watchdog_interval_ms
            {
                Ok(existing.clone())
            } else {
                Err(HubError::new(
                    "awake_operation_id_conflict",
                    "operation ID already names a different immutable Quest awake preview",
                ))
            };
        }
        let mut preflights = Vec::with_capacity(plan.request.targets.len());
        for (device_id, identity_revision) in &plan.request.targets {
            let record = self.devices.get(device_id).ok_or_else(|| {
                HubError::new(
                    "awake_target_not_found",
                    format!("unknown Quest awake target {device_id}"),
                )
            })?;
            if record.observation.identity.identity_revision != *identity_revision {
                return Err(HubError::new(
                    "awake_target_identity_mismatch",
                    format!("target {device_id} identity revision differs from the request"),
                ));
            }
            preflights.push(awake_preflight(
                record,
                plan.created_at_ms,
                plan.provider_ready_devices.contains(device_id),
            ));
        }
        let preview = QuestAwakePreview {
            schema: "rusty.fleet.quest_awake_preview.v1".to_owned(),
            preview_id: plan.preview_id,
            operation_id: plan.operation_id.clone(),
            action_id: QUEST_AWAKE_ACTION_ID.to_owned(),
            action: plan.request.action,
            created_at_ms: plan.created_at_ms,
            expires_at_ms: plan.expires_at_ms,
            fleet_revision: self.result_revision,
            duration_ms: plan.request.duration_ms,
            watchdog_interval_ms: plan.request.watchdog_interval_ms,
            watchdog_generation: plan.watchdog_generation,
            owner: QuestAwakeOwnerBinding::file_manager_v1(),
            targets: preflights,
        };
        let targets = preview
            .targets
            .iter()
            .cloned()
            .map(|preflight| QuestAwakeTargetLedger {
                device_id: preflight.device_id.clone(),
                identity_revision: preflight.identity_revision,
                lifecycle: if preflight.eligible {
                    CommandLifecycle::Proposed
                } else {
                    CommandLifecycle::Rejected
                },
                preflight,
                invocation: None,
                receipt: None,
                failure_code: None,
                updated_at_ms: plan.created_at_ms,
            })
            .collect();
        let operation = QuestAwakeOperation {
            schema: QUEST_AWAKE_OPERATION_SCHEMA.to_owned(),
            operation_id: plan.operation_id.clone(),
            action_id: QUEST_AWAKE_ACTION_ID.to_owned(),
            lifecycle: CommandLifecycle::Proposed,
            preview,
            confirmed_at_ms: None,
            targets,
            updated_at_ms: plan.created_at_ms,
        };
        validate_operation(&operation)?;
        if self.awake_operations.len() >= MAX_AWAKE_OPERATIONS {
            return Err(HubError::new(
                "awake_operation_limit_reached",
                format!("Fleet Hub retains at most {MAX_AWAKE_OPERATIONS} Quest awake operations"),
            ));
        }
        self.awake_operations
            .insert(plan.operation_id, operation.clone());
        Ok(operation)
    }

    pub fn confirm_quest_awake(
        &mut self,
        operation_id: &str,
        preview_id: &str,
        now_ms: i64,
    ) -> Result<QuestAwakeOperation, HubError> {
        let existing = self.quest_awake_operation(operation_id)?;
        if existing.preview.preview_id != preview_id {
            return Err(HubError::new(
                "awake_preview_conflict",
                "execute request does not bind the immutable Quest awake preview",
            ));
        }
        if existing.lifecycle != CommandLifecycle::Proposed {
            return Ok(existing);
        }
        if now_ms > existing.preview.expires_at_ms {
            return Err(HubError::new(
                "awake_preview_expired",
                "Quest awake preview expired before confirmation",
            ));
        }
        self.require_awake_targets_current(&existing, now_ms)?;
        let operation = self
            .awake_operations
            .get_mut(operation_id)
            .expect("Quest awake operation was just read");
        operation.confirmed_at_ms = Some(now_ms);
        operation.updated_at_ms = now_ms;
        for target in &mut operation.targets {
            if target.preflight.eligible {
                target.lifecycle = CommandLifecycle::Accepted;
                target.updated_at_ms = now_ms;
            }
        }
        operation.lifecycle = derive_lifecycle(operation);
        validate_operation(operation)?;
        Ok(operation.clone())
    }

    pub fn prepare_quest_awake_invocation(
        &mut self,
        operation_id: &str,
        device_id: &str,
        request_id: String,
        now_ms: i64,
    ) -> Result<QuestAwakeOperation, HubError> {
        self.prepare_quest_awake_invocation_with_watchdog_generation(
            operation_id,
            device_id,
            request_id,
            None,
            now_ms,
        )
    }

    pub fn prepare_quest_awake_invocation_with_watchdog_generation(
        &mut self,
        operation_id: &str,
        device_id: &str,
        request_id: String,
        expected_device_watchdog_generation: Option<String>,
        now_ms: i64,
    ) -> Result<QuestAwakeOperation, HubError> {
        let existing = self.quest_awake_operation(operation_id)?;
        if now_ms > existing.preview.expires_at_ms {
            return Err(HubError::new(
                "awake_preview_expired",
                "Quest awake dispatch cannot begin after preview expiry",
            ));
        }
        self.require_awake_target_current(&existing, device_id, now_ms)?;
        let operation = self
            .awake_operations
            .get_mut(operation_id)
            .expect("Quest awake operation was just read");
        let target = operation
            .targets
            .iter_mut()
            .find(|target| target.device_id == device_id)
            .ok_or_else(|| HubError::new("awake_target_not_found", "target is not in operation"))?;
        if target.lifecycle != CommandLifecycle::Accepted {
            return Err(HubError::new(
                "awake_dispatch_state_invalid",
                "only a confirmed Quest awake target can prepare an owner invocation",
            ));
        }
        let watchdog_generation = if matches!(
            operation.preview.action,
            fleet_contracts::QuestAwakeAction::StopWatchdogs
                | fleet_contracts::QuestAwakeAction::RestoreNormal
        ) {
            expected_device_watchdog_generation
                .unwrap_or_else(|| "no-known-device-watchdog".to_owned())
        } else if expected_device_watchdog_generation.is_some() {
            return Err(HubError::new(
                "awake_watchdog_generation_override_invalid",
                "only stop or restore may bind an observed device-watchdog generation",
            ));
        } else {
            operation.preview.watchdog_generation.clone()
        };
        target.invocation = Some(QuestAwakeOwnerInvocation {
            schema: "rusty.fleet.quest_awake_owner_invocation.v1".to_owned(),
            request_id,
            operation_id: operation.operation_id.clone(),
            preview_id: operation.preview.preview_id.clone(),
            device_id: target.device_id.clone(),
            identity_revision: target.identity_revision,
            action: operation.preview.action,
            duration_ms: operation.preview.duration_ms,
            watchdog_interval_ms: operation.preview.watchdog_interval_ms,
            watchdog_generation,
            issued_at_ms: now_ms,
            expires_at_ms: if operation.preview.action
                == fleet_contracts::QuestAwakeAction::StartWindowsWatchdog
            {
                i64::MAX
            } else {
                operation.preview.expires_at_ms
            },
        });
        target.updated_at_ms = now_ms;
        operation.updated_at_ms = now_ms;
        validate_operation(operation)?;
        Ok(operation.clone())
    }

    pub fn mark_quest_awake_dispatched(
        &mut self,
        operation_id: &str,
        device_id: &str,
        now_ms: i64,
    ) -> Result<QuestAwakeOperation, HubError> {
        let operation = self
            .awake_operations
            .get_mut(operation_id)
            .ok_or_else(|| HubError::new("awake_operation_not_found", "operation not found"))?;
        let target = operation
            .targets
            .iter_mut()
            .find(|target| target.device_id == device_id)
            .ok_or_else(|| HubError::new("awake_target_not_found", "target is not in operation"))?;
        if target.lifecycle != CommandLifecycle::Accepted || target.invocation.is_none() {
            return Err(HubError::new(
                "awake_dispatch_state_invalid",
                "Quest awake target has no accepted exact invocation",
            ));
        }
        target.lifecycle = CommandLifecycle::Dispatched;
        target.updated_at_ms = now_ms;
        operation.updated_at_ms = now_ms;
        operation.lifecycle = derive_lifecycle(operation);
        validate_operation(operation)?;
        Ok(operation.clone())
    }

    pub fn apply_quest_awake_receipt(
        &mut self,
        receipt: QuestAwakeOwnerReceipt,
        now_ms: i64,
    ) -> Result<QuestAwakeOperation, HubError> {
        if receipt.validate().is_err() || receipt.observed_at_ms > now_ms + 30_000 {
            return Err(HubError::new(
                "awake_receipt_invalid",
                "Quest awake owner receipt is invalid or future-dated",
            ));
        }
        let operation = self
            .awake_operations
            .get_mut(&receipt.operation_id)
            .ok_or_else(|| HubError::new("awake_operation_not_found", "operation not found"))?;
        let target = operation
            .targets
            .iter_mut()
            .find(|target| target.device_id == receipt.device_id)
            .ok_or_else(|| HubError::new("awake_target_not_found", "target is not in operation"))?;
        let invocation = target.invocation.as_ref().ok_or_else(|| {
            HubError::new(
                "awake_invocation_missing",
                "owner receipt cannot be admitted before exact invocation",
            )
        })?;
        if receipt.request_id != invocation.request_id
            || receipt.preview_id != invocation.preview_id
            || receipt.identity_revision != invocation.identity_revision
            || receipt.action != invocation.action
            || receipt.watchdog_generation != invocation.watchdog_generation
            || receipt.requested_duration_ms != invocation.duration_ms
            || receipt.requested_watchdog_interval_ms != invocation.watchdog_interval_ms
            || receipt.observed_at_ms < invocation.issued_at_ms
            || receipt.observed_at_ms > invocation.expires_at_ms
        {
            return Err(HubError::new(
                "awake_receipt_binding_mismatch",
                "Quest awake receipt does not bind the exact owner invocation",
            ));
        }
        if !matches!(
            target.lifecycle,
            CommandLifecycle::Dispatched | CommandLifecycle::Running | CommandLifecycle::Applied
        ) {
            return Err(HubError::new(
                "awake_receipt_state_invalid",
                "Quest awake receipt is not admissible in the target lifecycle",
            ));
        }
        target.lifecycle = if receipt.effective {
            CommandLifecycle::Applied
        } else {
            CommandLifecycle::Running
        };
        target.receipt = Some(receipt);
        target.failure_code = None;
        target.updated_at_ms = now_ms;
        operation.updated_at_ms = now_ms;
        operation.lifecycle = derive_lifecycle(operation);
        validate_operation(operation)?;
        Ok(operation.clone())
    }

    pub fn fail_quest_awake_target(
        &mut self,
        operation_id: &str,
        device_id: &str,
        code: String,
        now_ms: i64,
    ) -> Result<QuestAwakeOperation, HubError> {
        let operation = self
            .awake_operations
            .get_mut(operation_id)
            .ok_or_else(|| HubError::new("awake_operation_not_found", "operation not found"))?;
        let target = operation
            .targets
            .iter_mut()
            .find(|target| target.device_id == device_id)
            .ok_or_else(|| HubError::new("awake_target_not_found", "target is not in operation"))?;
        if code.is_empty() || code.len() > 256 {
            return Err(HubError::new(
                "awake_failure_code_invalid",
                "Quest awake failure code must be bounded",
            ));
        }
        target.lifecycle = CommandLifecycle::Failed;
        target.failure_code = Some(code);
        target.updated_at_ms = now_ms;
        operation.updated_at_ms = now_ms;
        operation.lifecycle = derive_lifecycle(operation);
        validate_operation(operation)?;
        Ok(operation.clone())
    }

    pub fn quest_awake_operation(
        &self,
        operation_id: &str,
    ) -> Result<QuestAwakeOperation, HubError> {
        self.awake_operations
            .get(operation_id)
            .cloned()
            .ok_or_else(|| HubError::new("awake_operation_not_found", "operation not found"))
    }

    #[must_use]
    pub fn quest_awake_operations(&self) -> Vec<QuestAwakeOperation> {
        self.awake_operations.values().cloned().collect()
    }

    fn require_awake_targets_current(
        &self,
        operation: &QuestAwakeOperation,
        now_ms: i64,
    ) -> Result<(), HubError> {
        for target in operation
            .targets
            .iter()
            .filter(|target| target.preflight.eligible)
        {
            self.require_awake_target_current(operation, &target.device_id, now_ms)?;
        }
        Ok(())
    }

    fn require_awake_target_current(
        &self,
        operation: &QuestAwakeOperation,
        device_id: &str,
        now_ms: i64,
    ) -> Result<(), HubError> {
        let target = operation
            .targets
            .iter()
            .find(|target| target.device_id == device_id)
            .ok_or_else(|| HubError::new("awake_target_not_found", "target is not in operation"))?;
        let record = self.devices.get(device_id).ok_or_else(|| {
            HubError::new("awake_target_not_found", "target is no longer enrolled")
        })?;
        if record.observation.identity.identity_revision != target.identity_revision {
            return Err(HubError::new(
                "awake_target_identity_changed",
                "target identity changed after preview",
            ));
        }
        if record.observation.received_time_ms > now_ms
            || record.observation.source_time_ms > now_ms
        {
            return Err(HubError::new(
                "awake_capability_changed",
                "Quest awake target observation is not current at confirmation",
            ));
        }
        Ok(())
    }
}

fn awake_preflight(
    record: &DeviceRecord,
    evaluated_at_ms: i64,
    provider_ready: bool,
) -> QuestAwakeTargetPreflight {
    let capability = record
        .observation
        .capabilities
        .capabilities
        .get(QUEST_AWAKE_CAPABILITY_ID);
    let (
        support,
        enablement,
        authorization,
        reachability,
        freshness,
        evidence_revision,
        observed_at_ms,
        fresh_until_ms,
        owner,
    ) = capability.map_or_else(
        || {
            if provider_ready {
                (
                    SupportState::Supported,
                    EnablementState::Enabled,
                    AuthorizationState::Authorized,
                    ReachabilityState::Reachable,
                    FreshnessState::Current,
                    record.accepted_revision,
                    evaluated_at_ms,
                    evaluated_at_ms + 60_000,
                    QUEST_AWAKE_OWNER.to_owned(),
                )
            } else {
                (
                    SupportState::Unknown,
                    EnablementState::Unknown,
                    AuthorizationState::Unknown,
                    ReachabilityState::Unknown,
                    FreshnessState::Unknown,
                    0,
                    evaluated_at_ms,
                    evaluated_at_ms,
                    String::new(),
                )
            }
        },
        |value| {
            (
                value.support,
                value.enablement,
                value.authorization,
                value.reachability,
                value.freshness,
                value.evidence_revision,
                value.observed_at_ms,
                value.fresh_until_ms,
                value.owner.clone(),
            )
        },
    );
    let mut preflight = QuestAwakeTargetPreflight {
        device_id: record.observation.identity.device_id.clone(),
        identity_revision: record.observation.identity.identity_revision,
        capability_id: QUEST_AWAKE_CAPABILITY_ID.to_owned(),
        capability_evidence_revision: evidence_revision,
        capability_owner: owner,
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
        message: String::new(),
    };
    preflight.reason_code = preflight.expected_reason_code().to_owned();
    preflight.eligible = preflight.expected_eligibility();
    preflight.message = if preflight.eligible {
        "File Manager Quest awake provider is current and ready".to_owned()
    } else {
        format!(
            "Quest awake target is not eligible: {}",
            preflight.reason_code
        )
    };
    preflight
}

fn derive_lifecycle(operation: &QuestAwakeOperation) -> CommandLifecycle {
    let eligible = operation
        .targets
        .iter()
        .filter(|target| target.preflight.eligible)
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return CommandLifecycle::Rejected;
    }
    if eligible
        .iter()
        .any(|target| target.lifecycle == CommandLifecycle::Failed)
    {
        return CommandLifecycle::Failed;
    }
    if eligible
        .iter()
        .all(|target| target.lifecycle == CommandLifecycle::Applied)
    {
        return CommandLifecycle::Applied;
    }
    if eligible.iter().any(|target| {
        matches!(
            target.lifecycle,
            CommandLifecycle::Dispatched | CommandLifecycle::Running
        )
    }) {
        return CommandLifecycle::Running;
    }
    if operation.confirmed_at_ms.is_some() {
        CommandLifecycle::Accepted
    } else {
        CommandLifecycle::Proposed
    }
}

fn validate_operation(operation: &QuestAwakeOperation) -> Result<(), HubError> {
    operation.validate().map_err(|failures| {
        HubError::new(
            "awake_operation_contract_invalid",
            failures
                .iter()
                .map(|failure| format!("{}:{}", failure.path, failure.code))
                .collect::<Vec<_>>()
                .join("; "),
        )
    })
}
