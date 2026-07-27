// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{
    AuthorizationState, CommandLifecycle, ConsumedPackageUpdaterClaimIdentity, EnablementState,
    FreshnessState, MAX_CONSUMED_PACKAGE_OWNER_CLAIMS, PACKAGE_UPDATER_CAPABILITY_ID,
    PACKAGE_UPDATER_OWNER, PACKAGES_INSTALL_RELEASE_ACTION_ID, PackageInstallReleaseOperation,
    PackageInstallReleasePreview, PackageInstallReleasePreviewRequest, PackageInstallStage,
    PackageInstallTargetLedger, PackageInstallTargetPreflight, PackageUpdaterClaim,
    PackageUpdaterEffectiveReceipt, PackageUpdaterInvocation,
    PackageUpdaterInvocationAcknowledgement, PackageUpdaterOwnerContractBinding, ReachabilityState,
    SupportState, ValidateContract,
};
use serde::{Deserialize, Serialize};

use crate::{DeviceRecord, FleetHub, HubError, MAX_PACKAGE_OPERATIONS};

const PREVIEW_MAX_LIFETIME_MS: i64 = 15 * 60 * 1_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageInstallReleasePreviewPlan {
    pub operation_id: String,
    pub preview_id: String,
    pub request: PackageInstallReleasePreviewRequest,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub max_parallelism: u16,
}

impl FleetHub {
    #[must_use]
    pub fn select_package_updater_offer(&self, now_ms: i64) -> Option<PackageUpdaterInvocation> {
        self.package_operations.values().find_map(|operation| {
            let occupied = operation
                .targets
                .iter()
                .filter(|target| {
                    matches!(
                        target.lifecycle,
                        CommandLifecycle::Dispatched | CommandLifecycle::Running
                    ) || (target.lifecycle == CommandLifecycle::Accepted
                        && target
                            .owner_claim
                            .as_ref()
                            .is_some_and(|claim| claim.expires_at_ms > now_ms))
                })
                .count();
            if occupied >= usize::from(operation.max_parallelism) {
                return None;
            }
            operation.targets.iter().find_map(|target| {
                let invocation = target.invocation.as_ref()?;
                (target.lifecycle == CommandLifecycle::Accepted
                    && target.stage == PackageInstallStage::DispatchReady
                    && target
                        .owner_claim
                        .as_ref()
                        .is_none_or(|claim| claim.expires_at_ms <= now_ms)
                    && target.consumed_owner_claim_identities.len()
                        < MAX_CONSUMED_PACKAGE_OWNER_CLAIMS
                    && invocation.expires_at_ms > now_ms)
                    .then(|| invocation.clone())
            })
        })
    }

    pub fn preview_package_install_release(
        &mut self,
        plan: PackageInstallReleasePreviewPlan,
    ) -> Result<PackageInstallReleaseOperation, HubError> {
        self.validate_package_preview_plan(&plan)?;
        if let Some(existing) = self.package_operations.get(&plan.operation_id) {
            return if package_plan_matches(&plan, existing) {
                Ok(existing.clone())
            } else {
                Err(HubError::new(
                    "package_operation_id_conflict",
                    "operation ID already names a different immutable package preview",
                ))
            };
        }
        let mut preflights = Vec::with_capacity(plan.request.targets.len());
        for (device_id, identity_revision) in &plan.request.targets {
            let record = self.devices.get(device_id).ok_or_else(|| {
                HubError::new(
                    "package_target_not_found",
                    format!("unknown package operation target {device_id}"),
                )
            })?;
            if record.observation.identity.identity_revision != *identity_revision {
                return Err(HubError::new(
                    "package_target_identity_mismatch",
                    format!("target {device_id} identity revision differs from the request"),
                ));
            }
            preflights.push(self.package_updater_preflight(record, plan.created_at_ms));
        }
        let preview = PackageInstallReleasePreview {
            schema: "rusty.fleet.package_install_release_preview.v1".to_owned(),
            preview_id: plan.preview_id,
            operation_id: plan.operation_id.clone(),
            action_id: PACKAGES_INSTALL_RELEASE_ACTION_ID.to_owned(),
            created_at_ms: plan.created_at_ms,
            expires_at_ms: plan.expires_at_ms,
            fleet_revision: self.result_revision,
            release: plan.request.release,
            expected_package_name: plan.request.expected_package_name,
            expected_rollout_ring: plan.request.expected_rollout_ring,
            owner_contract: PackageUpdaterOwnerContractBinding::attended_v1(),
            targets: preflights,
        };
        let targets = preview
            .targets
            .iter()
            .cloned()
            .map(|preflight| {
                let eligible = preflight.eligible;
                PackageInstallTargetLedger {
                    device_id: preflight.device_id.clone(),
                    identity_revision: preflight.identity_revision,
                    preflight,
                    lifecycle: if eligible {
                        CommandLifecycle::Proposed
                    } else {
                        CommandLifecycle::Rejected
                    },
                    stage: if eligible {
                        PackageInstallStage::PreviewReady
                    } else {
                        PackageInstallStage::PreflightRejected
                    },
                    invocation: None,
                    owner_claim: None,
                    prior_owner_claims: Vec::new(),
                    consumed_owner_claim_identities: Vec::new(),
                    invocation_acknowledgement: None,
                    effective_receipt: None,
                    reason_code: if eligible {
                        "preview_ready".to_owned()
                    } else {
                        "preflight_excluded".to_owned()
                    },
                    message: if eligible {
                        "Target is ready for explicit confirmation".to_owned()
                    } else {
                        "Target was excluded by the frozen updater preflight".to_owned()
                    },
                    last_transition_ms: plan.created_at_ms,
                }
            })
            .collect();
        let mut operation = PackageInstallReleaseOperation {
            schema: "rusty.fleet.package_install_release_operation.v1".to_owned(),
            operation_id: plan.operation_id,
            action_id: PACKAGES_INSTALL_RELEASE_ACTION_ID.to_owned(),
            created_at_ms: plan.created_at_ms,
            preview,
            lifecycle: CommandLifecycle::Proposed,
            max_parallelism: plan.max_parallelism,
            cleanup_required: false,
            targets,
        };
        operation.lifecycle = operation.derived_lifecycle();
        validate_package_operation(&operation)?;
        if self.package_operations.len() >= MAX_PACKAGE_OPERATIONS {
            return Err(HubError::new(
                "package_operation_limit_reached",
                format!("Fleet Hub retains at most {MAX_PACKAGE_OPERATIONS} package operations"),
            ));
        }
        self.package_operations
            .insert(operation.operation_id.clone(), operation.clone());
        Ok(operation)
    }

    pub fn confirm_package_install_release(
        &mut self,
        operation_id: &str,
        preview_id: &str,
        now_ms: i64,
    ) -> Result<PackageInstallReleaseOperation, HubError> {
        let existing = self.package_operation(operation_id)?;
        require_package_preview(&existing, preview_id)?;
        if existing.lifecycle != CommandLifecycle::Proposed {
            return Ok(existing);
        }
        if now_ms > existing.preview.expires_at_ms {
            return Err(HubError::new(
                "package_preview_expired",
                "immutable package preview expired before confirmation",
            ));
        }
        self.require_package_targets_current(&existing, now_ms)?;
        let operation = self
            .package_operations
            .get_mut(operation_id)
            .expect("package operation was just read");
        for target in &mut operation.targets {
            if target.preflight.eligible {
                target.lifecycle = CommandLifecycle::Accepted;
                target.stage = PackageInstallStage::Approved;
                target.reason_code = "operator_confirmed".to_owned();
                target.message = "Exact package preview confirmed for dispatch".to_owned();
                target.last_transition_ms = now_ms;
            }
        }
        operation.lifecycle = operation.derived_lifecycle();
        validate_package_operation(operation)?;
        Ok(operation.clone())
    }

    pub fn prepare_package_install_release_invocation(
        &mut self,
        operation_id: &str,
        device_id: &str,
        owner_action_request_id: String,
        now_ms: i64,
    ) -> Result<PackageInstallReleaseOperation, HubError> {
        let existing = self.package_operation(operation_id)?;
        if now_ms > existing.preview.expires_at_ms {
            return Err(HubError::new(
                "package_preview_expired",
                "package updater dispatch cannot begin after preview expiry",
            ));
        }
        self.require_package_target_current(&existing, device_id, now_ms)?;
        let operation = self
            .package_operations
            .get_mut(operation_id)
            .expect("package operation was just read");
        let target = operation
            .targets
            .iter_mut()
            .find(|target| target.device_id == device_id)
            .ok_or_else(|| {
                HubError::new("package_target_not_found", "target is not in operation")
            })?;
        if target.lifecycle != CommandLifecycle::Accepted
            || target.stage != PackageInstallStage::Approved
        {
            return Err(HubError::new(
                "package_dispatch_state_invalid",
                "only a confirmed package target can prepare an owner invocation",
            ));
        }
        target.invocation = Some(PackageUpdaterInvocation {
            schema: "rusty.fleet.package_updater_invocation.v1".to_owned(),
            operation_id: operation.operation_id.clone(),
            preview_id: operation.preview.preview_id.clone(),
            device_id: target.device_id.clone(),
            identity_revision: target.identity_revision,
            owner_action_request_id,
            release: operation.preview.release.clone(),
            expected_package_name: operation.preview.expected_package_name.clone(),
            expected_rollout_ring: operation.preview.expected_rollout_ring.clone(),
            expires_at_ms: operation.preview.expires_at_ms,
        });
        target.lifecycle = CommandLifecycle::Accepted;
        target.stage = PackageInstallStage::DispatchReady;
        target.reason_code = "owner_dispatch_ready".to_owned();
        target.message =
            "Exact updater invocation is ready for delivery; application remains unproven"
                .to_owned();
        target.last_transition_ms = now_ms;
        operation.lifecycle = operation.derived_lifecycle();
        validate_package_operation(operation)?;
        Ok(operation.clone())
    }

    pub fn offer_package_updater_claim(
        &mut self,
        claim: PackageUpdaterClaim,
        now_ms: i64,
    ) -> Result<PackageInstallReleaseOperation, HubError> {
        if claim.validate().is_err()
            || now_ms < claim.claimed_at_ms
            || now_ms >= claim.expires_at_ms
        {
            return Err(HubError::new(
                "package_owner_claim_invalid",
                "owner claim is invalid or outside its bounded lifetime",
            ));
        }
        if self.package_operations.values().any(|operation| {
            operation.targets.iter().any(|target| {
                target
                    .consumed_owner_claim_identities
                    .iter()
                    .any(|existing| {
                        existing.claim_id == claim.claim_id
                            || existing.request_id == claim.request_id
                    })
            })
        }) {
            return Err(HubError::new(
                "package_owner_claim_replay",
                "owner claim or request identifier was already consumed",
            ));
        }
        let operation = self
            .package_operations
            .get_mut(&claim.invocation.operation_id)
            .ok_or_else(|| HubError::new("package_operation_not_found", "operation not found"))?;
        let active = operation
            .targets
            .iter()
            .filter(|target| {
                target.owner_claim.as_ref().is_some_and(|existing| {
                    existing.expires_at_ms > now_ms
                        && !matches!(
                            target.lifecycle,
                            CommandLifecycle::Applied
                                | CommandLifecycle::Failed
                                | CommandLifecycle::Cancelled
                                | CommandLifecycle::Expired
                        )
                })
            })
            .count();
        if active >= usize::from(operation.max_parallelism) {
            return Err(HubError::new(
                "package_owner_scheduler_saturated",
                "bounded package delivery scheduler has no free claim slot",
            ));
        }
        let target = operation
            .targets
            .iter_mut()
            .find(|target| target.device_id == claim.invocation.device_id)
            .ok_or_else(|| HubError::new("package_target_not_found", "target not found"))?;
        if target.lifecycle != CommandLifecycle::Accepted
            || target.stage != PackageInstallStage::DispatchReady
            || target.invocation.as_ref() != Some(&claim.invocation)
            || target.owner_claim.is_some()
        {
            return Err(HubError::new(
                "package_owner_claim_state_invalid",
                "only one unclaimed dispatch-ready invocation can be offered",
            ));
        }
        if target.consumed_owner_claim_identities.len() >= MAX_CONSUMED_PACKAGE_OWNER_CLAIMS {
            return Err(HubError::new(
                "package_owner_replay_authority_exhausted",
                "bounded durable replay authority is exhausted for this target",
            ));
        }
        target
            .consumed_owner_claim_identities
            .push(ConsumedPackageUpdaterClaimIdentity {
                claim_id: claim.claim_id.clone(),
                request_id: claim.request_id.clone(),
            });
        target.owner_claim = Some(claim);
        target.reason_code = "owner_delivery_offered".to_owned();
        target.message =
            "Authenticated updater owner claimed the exact invocation; transport remains unacknowledged"
                .to_owned();
        target.last_transition_ms = now_ms;
        validate_package_operation(operation)?;
        Ok(operation.clone())
    }

    pub fn expire_package_updater_claims(&mut self, now_ms: i64) -> Result<usize, HubError> {
        let mut expired = 0;
        for operation in self.package_operations.values_mut() {
            for target in &mut operation.targets {
                if target.lifecycle == CommandLifecycle::Accepted
                    && target.stage == PackageInstallStage::DispatchReady
                    && target
                        .owner_claim
                        .as_ref()
                        .is_some_and(|claim| claim.expires_at_ms <= now_ms)
                {
                    let prior = target.owner_claim.take().expect("expired claim exists");
                    if target.prior_owner_claims.len() >= 16 {
                        target.prior_owner_claims.remove(0);
                    }
                    target.prior_owner_claims.push(prior);
                    if target.consumed_owner_claim_identities.len()
                        >= MAX_CONSUMED_PACKAGE_OWNER_CLAIMS
                    {
                        target.lifecycle = CommandLifecycle::Failed;
                        target.stage = PackageInstallStage::Failed;
                        target.reason_code = "owner_replay_authority_exhausted".to_owned();
                        target.message =
                            "Bounded durable replay authority is exhausted; target failed closed"
                                .to_owned();
                    } else {
                        target.lifecycle = CommandLifecycle::Accepted;
                        target.stage = PackageInstallStage::DispatchReady;
                        target.reason_code = "owner_claim_expired".to_owned();
                        target.message =
                            "Prior owner claim expired unacknowledged; a fresh exact request may retry"
                                .to_owned();
                    }
                    target.last_transition_ms = now_ms;
                    expired += 1;
                }
            }
            operation.lifecycle = operation.derived_lifecycle();
            validate_package_operation(operation)?;
        }
        Ok(expired)
    }

    pub fn admit_authenticated_package_updater_acknowledgement(
        &mut self,
        acknowledgement: PackageUpdaterInvocationAcknowledgement,
        now_ms: i64,
    ) -> Result<PackageInstallReleaseOperation, HubError> {
        let operation = self
            .package_operations
            .get_mut(&acknowledgement.operation_id)
            .ok_or_else(|| {
                HubError::new(
                    "package_operation_not_found",
                    "package operation was not found",
                )
            })?;
        let target = operation
            .targets
            .iter_mut()
            .find(|target| target.device_id == acknowledgement.device_id)
            .ok_or_else(|| {
                HubError::new("package_target_not_found", "target is not in operation")
            })?;
        let invocation = target.invocation.as_ref().ok_or_else(|| {
            HubError::new(
                "package_invocation_missing",
                "target has no dispatched updater invocation",
            )
        })?;
        if acknowledgement.validate().is_err()
            || acknowledgement.operation_id != invocation.operation_id
            || acknowledgement.device_id != invocation.device_id
            || acknowledgement.owner_action_request_id != invocation.owner_action_request_id
            || acknowledgement.acknowledged_at_ms > now_ms
            || now_ms > invocation.expires_at_ms
        {
            return Err(HubError::new(
                "package_acknowledgement_invalid",
                "updater acknowledgement is invalid, stale, or not bound to the invocation",
            ));
        }
        if target.lifecycle != CommandLifecycle::Accepted
            || target.stage != PackageInstallStage::DispatchReady
        {
            return Err(HubError::new(
                "package_acknowledgement_state_invalid",
                "only a dispatched target accepts one updater acknowledgement",
            ));
        }
        let accepted = acknowledgement.accepted;
        target.invocation_acknowledgement = Some(acknowledgement);
        target.lifecycle = if accepted {
            CommandLifecycle::Dispatched
        } else {
            CommandLifecycle::Failed
        };
        target.stage = if accepted {
            PackageInstallStage::OwnerAcknowledged
        } else {
            PackageInstallStage::Failed
        };
        target.reason_code = if accepted {
            "owner_invocation_acknowledged".to_owned()
        } else {
            "owner_invocation_rejected".to_owned()
        };
        target.message = if accepted {
            "Updater acknowledged invocation; installed-version proof is still pending".to_owned()
        } else {
            "Updater rejected the invocation".to_owned()
        };
        target.last_transition_ms = now_ms;
        operation.lifecycle = operation.derived_lifecycle();
        validate_package_operation(operation)?;
        Ok(operation.clone())
    }

    pub fn admit_authenticated_package_updater_receipt(
        &mut self,
        receipt: PackageUpdaterEffectiveReceipt,
        now_ms: i64,
    ) -> Result<PackageInstallReleaseOperation, HubError> {
        let operation = self
            .package_operations
            .get_mut(&receipt.operation_id)
            .ok_or_else(|| {
                HubError::new(
                    "package_operation_not_found",
                    "package operation was not found",
                )
            })?;
        let target = operation
            .targets
            .iter_mut()
            .find(|target| target.device_id == receipt.device_id)
            .ok_or_else(|| {
                HubError::new("package_target_not_found", "target is not in operation")
            })?;
        let invocation = target.invocation.as_ref().ok_or_else(|| {
            HubError::new(
                "package_invocation_missing",
                "target has no dispatched updater invocation",
            )
        })?;
        if !matches!(
            target.lifecycle,
            CommandLifecycle::Dispatched | CommandLifecycle::Running
        ) {
            return Err(HubError::new(
                "package_receipt_state_invalid",
                "only an in-flight package target accepts an effective receipt",
            ));
        }
        if now_ms > invocation.expires_at_ms || receipt.validate_for(invocation).is_err() {
            return Err(HubError::new(
                "package_effective_receipt_invalid",
                "owner receipt does not prove the exact installed package version",
            ));
        }
        target.effective_receipt = Some(receipt);
        target.lifecycle = CommandLifecycle::Applied;
        target.stage = PackageInstallStage::Applied;
        target.reason_code = "owner_effective_installed_version".to_owned();
        target.message =
            "Updater install_commit receipt proves the effective installed package version"
                .to_owned();
        target.last_transition_ms = now_ms;
        operation.lifecycle = operation.derived_lifecycle();
        validate_package_operation(operation)?;
        Ok(operation.clone())
    }

    pub fn package_operation(
        &self,
        operation_id: &str,
    ) -> Result<PackageInstallReleaseOperation, HubError> {
        self.package_operations
            .get(operation_id)
            .cloned()
            .ok_or_else(|| {
                HubError::new(
                    "package_operation_not_found",
                    "package operation was not found",
                )
            })
    }

    pub fn package_operations(&self) -> Vec<PackageInstallReleaseOperation> {
        let mut operations = self
            .package_operations
            .values()
            .cloned()
            .collect::<Vec<_>>();
        operations.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.operation_id.cmp(&right.operation_id))
        });
        operations
    }

    fn validate_package_preview_plan(
        &self,
        plan: &PackageInstallReleasePreviewPlan,
    ) -> Result<(), HubError> {
        if plan.operation_id.is_empty()
            || plan.operation_id.len() > 256
            || plan.preview_id.is_empty()
            || plan.preview_id.len() > 256
            || plan.request.validate().is_err()
            || plan.created_at_ms < 0
            || plan.expires_at_ms <= plan.created_at_ms
            || plan.expires_at_ms.saturating_sub(plan.created_at_ms) > PREVIEW_MAX_LIFETIME_MS
            || !(1..=64).contains(&plan.max_parallelism)
        {
            return Err(HubError::new(
                "package_preview_plan_invalid",
                "package preview plan is invalid or exceeds its bounded lifetime",
            ));
        }
        Ok(())
    }

    fn package_updater_preflight(
        &self,
        record: &DeviceRecord,
        evaluated_at_ms: i64,
    ) -> PackageInstallTargetPreflight {
        let capability = record
            .observation
            .capabilities
            .get(PACKAGE_UPDATER_CAPABILITY_ID);
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
            Some(capability) if capability.owner == PACKAGE_UPDATER_OWNER => (
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
                "Package updater capability evidence has the wrong owner".to_owned(),
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
                "No Rusty Quest package-updater capability evidence is present".to_owned(),
            ),
        };
        let age_ms = evaluated_at_ms.saturating_sub(record.accepted_at_ms).max(0);
        if age_ms > self.policy.offline_after_ms {
            reachability = ReachabilityState::Disconnected;
        }
        if age_ms > self.policy.stale_after_ms {
            freshness = FreshnessState::Stale;
        }
        let mut preflight = PackageInstallTargetPreflight {
            device_id: record.observation.identity.device_id.clone(),
            identity_revision: record.observation.identity.identity_revision,
            capability_id: PACKAGE_UPDATER_CAPABILITY_ID.to_owned(),
            capability_evidence_revision,
            capability_owner: PACKAGE_UPDATER_OWNER.to_owned(),
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

    fn require_package_targets_current(
        &self,
        operation: &PackageInstallReleaseOperation,
        now_ms: i64,
    ) -> Result<(), HubError> {
        for target in &operation.targets {
            if target.preflight.eligible {
                self.require_package_target_current(operation, &target.device_id, now_ms)?;
            }
        }
        Ok(())
    }

    fn require_package_target_current(
        &self,
        operation: &PackageInstallReleaseOperation,
        device_id: &str,
        now_ms: i64,
    ) -> Result<(), HubError> {
        let target = operation
            .targets
            .iter()
            .find(|target| target.device_id == device_id)
            .ok_or_else(|| {
                HubError::new("package_target_not_found", "target is not in operation")
            })?;
        let record = self.devices.get(device_id).ok_or_else(|| {
            HubError::new(
                "package_target_not_found",
                "package target is no longer in the Fleet directory",
            )
        })?;
        if record.observation.identity.identity_revision != target.identity_revision {
            return Err(HubError::new(
                "package_target_identity_changed",
                "package target identity changed after preview",
            ));
        }
        let current = self.package_updater_preflight(record, now_ms);
        if !same_package_frozen_facts(&target.preflight, &current) {
            return Err(HubError::new(
                "package_target_changed_since_preview",
                "package updater capability or freshness changed after preview",
            ));
        }
        Ok(())
    }
}

fn package_plan_matches(
    plan: &PackageInstallReleasePreviewPlan,
    operation: &PackageInstallReleaseOperation,
) -> bool {
    plan.operation_id == operation.operation_id
        && plan.preview_id == operation.preview.preview_id
        && plan.created_at_ms == operation.created_at_ms
        && plan.expires_at_ms == operation.preview.expires_at_ms
        && plan.max_parallelism == operation.max_parallelism
        && plan.request.release == operation.preview.release
        && plan.request.expected_package_name == operation.preview.expected_package_name
        && plan.request.expected_rollout_ring == operation.preview.expected_rollout_ring
        && plan.request.targets
            == operation
                .preview
                .targets
                .iter()
                .map(|target| (target.device_id.clone(), target.identity_revision))
                .collect()
}

fn require_package_preview(
    operation: &PackageInstallReleaseOperation,
    preview_id: &str,
) -> Result<(), HubError> {
    if operation.preview.preview_id == preview_id {
        Ok(())
    } else {
        Err(HubError::new(
            "package_preview_mismatch",
            "execute request does not bind the immutable package preview",
        ))
    }
}

fn same_package_frozen_facts(
    frozen: &PackageInstallTargetPreflight,
    current: &PackageInstallTargetPreflight,
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
        && frozen.eligible == current.expected_eligibility()
        && frozen.reason_code == current.expected_reason_code()
}

fn validate_package_operation(operation: &PackageInstallReleaseOperation) -> Result<(), HubError> {
    operation.validate().map_err(|failures| {
        HubError::new(
            "package_operation_invalid",
            format!(
                "package operation failed validation: {}",
                failures
                    .into_iter()
                    .map(|failure| format!("{}:{}", failure.code, failure.path))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fleet_contracts::{
        CapabilityState, PACKAGE_INSTALL_PREVIEW_REQUEST_SCHEMA, PACKAGE_UPDATE_RECEIPT_SCHEMA,
        PACKAGE_UPDATER_ACK_SCHEMA, PackageReleaseReference, PackageUpdateCheckpoint,
        PackageUpdateReceipt, PackageUpdateReceiptDecision, PackageUpdateReceiptStage,
        PackageUpdaterEffectiveReceipt, PackageUpdaterInvocationAcknowledgement,
    };
    use fleet_simulator::{BASE_TIME_MS, ScenarioBuilder};

    use super::*;
    use crate::{FleetApi, HubPolicy, ObservationDecision};

    fn ready_hub() -> FleetHub {
        ready_hub_with_count(1)
    }

    fn ready_hub_with_count(count: usize) -> FleetHub {
        let mut hub = FleetHub::new(HubPolicy::default());
        for mut observation in ScenarioBuilder::new(count).build().initial {
            observation.capabilities.capabilities.insert(
                PACKAGE_UPDATER_CAPABILITY_ID.to_owned(),
                CapabilityState {
                    capability_id: PACKAGE_UPDATER_CAPABILITY_ID.to_owned(),
                    support: SupportState::Supported,
                    enablement: EnablementState::Enabled,
                    authorization: AuthorizationState::Authorized,
                    reachability: ReachabilityState::Reachable,
                    freshness: FreshnessState::Current,
                    evidence_revision: observation.source_revision,
                    observed_at_ms: BASE_TIME_MS,
                    fresh_until_ms: BASE_TIME_MS + 60_000,
                    owner: PACKAGE_UPDATER_OWNER.to_owned(),
                    reason: "owner_ready".to_owned(),
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

    fn plan(hub: &FleetHub) -> PackageInstallReleasePreviewPlan {
        let device_id = "sim-00001";
        let identity_revision = hub
            .inspect(device_id, BASE_TIME_MS)
            .expect("device")
            .row
            .identity
            .identity_revision;
        PackageInstallReleasePreviewPlan {
            operation_id: "package-operation-1".to_owned(),
            preview_id: "package-preview-1".to_owned(),
            request: PackageInstallReleasePreviewRequest {
                schema: PACKAGE_INSTALL_PREVIEW_REQUEST_SCHEMA.to_owned(),
                action_id: PACKAGES_INSTALL_RELEASE_ACTION_ID.to_owned(),
                release: PackageReleaseReference::ManifestUrl {
                    manifest_url: "https://updates.example.invalid/stable/manifest.json".to_owned(),
                },
                expected_package_name: "org.example.kiosk".to_owned(),
                expected_rollout_ring: "stable".to_owned(),
                targets: BTreeMap::from([(device_id.to_owned(), identity_revision)]),
            },
            created_at_ms: BASE_TIME_MS,
            expires_at_ms: BASE_TIME_MS + 60_000,
            max_parallelism: 1,
        }
    }

    #[test]
    fn immutable_preview_ack_and_effective_receipt_remain_distinct() {
        let mut hub = ready_hub();
        let preview = hub
            .preview_package_install_release(plan(&hub))
            .expect("preview");
        let confirmed = hub
            .confirm_package_install_release(
                &preview.operation_id,
                &preview.preview.preview_id,
                BASE_TIME_MS + 1,
            )
            .expect("confirm");
        assert_eq!(confirmed.lifecycle, CommandLifecycle::Accepted);
        let dispatched = hub
            .prepare_package_install_release_invocation(
                &preview.operation_id,
                "sim-00001",
                "package-owner-request-1".to_owned(),
                BASE_TIME_MS + 2,
            )
            .expect("dispatch");
        assert_eq!(dispatched.targets[0].lifecycle, CommandLifecycle::Accepted);
        assert_eq!(
            dispatched.targets[0].stage,
            PackageInstallStage::DispatchReady
        );
        let dispatched = hub
            .admit_authenticated_package_updater_acknowledgement(
                PackageUpdaterInvocationAcknowledgement {
                    schema: PACKAGE_UPDATER_ACK_SCHEMA.to_owned(),
                    operation_id: preview.operation_id.clone(),
                    device_id: "sim-00001".to_owned(),
                    owner_action_request_id: "package-owner-request-1".to_owned(),
                    accepted: true,
                    code: "accepted".to_owned(),
                    acknowledged_at_ms: BASE_TIME_MS + 3,
                },
                BASE_TIME_MS + 3,
            )
            .expect("acknowledge");
        assert_eq!(dispatched.lifecycle, CommandLifecycle::Dispatched);
        assert_eq!(
            dispatched.targets[0].stage,
            PackageInstallStage::OwnerAcknowledged
        );
        assert!(dispatched.targets[0].effective_receipt.is_none());

        let digest = format!("sha256:{}", "a".repeat(64));
        let applied = hub
            .admit_authenticated_package_updater_receipt(
                PackageUpdaterEffectiveReceipt {
                    schema: "rusty.fleet.package_updater_effective_receipt.v1".to_owned(),
                    operation_id: preview.operation_id,
                    device_id: "sim-00001".to_owned(),
                    identity_revision: dispatched.targets[0].identity_revision,
                    owner_action_request_id: "package-owner-request-1".to_owned(),
                    updater_receipt: PackageUpdateReceipt {
                        schema: PACKAGE_UPDATE_RECEIPT_SCHEMA.to_owned(),
                        stage: PackageUpdateReceiptStage::InstallCommit,
                        decision: PackageUpdateReceiptDecision::Accepted,
                        code: "installed".to_owned(),
                        observed_at_ms: BASE_TIME_MS + 4,
                        envelope_sha256: Some(format!("sha256:{}", "b".repeat(64))),
                        signed_manifest_sha256: Some(digest.clone()),
                        key_id: Some("key-1".to_owned()),
                        manifest_id: Some("release-15".to_owned()),
                        package_name: Some("org.example.kiosk".to_owned()),
                        rollout_ring: Some("stable".to_owned()),
                        sequence: Some(15),
                        version_code: Some(15),
                        prior_checkpoint: None,
                        accepted_checkpoint: Some(PackageUpdateCheckpoint {
                            package_name: "org.example.kiosk".to_owned(),
                            rollout_ring: "stable".to_owned(),
                            sequence: 15,
                            version_code: 15,
                            signed_manifest_sha256: digest,
                        }),
                        state_changed: true,
                    },
                    wrapped_at_ms: BASE_TIME_MS + 4,
                },
                BASE_TIME_MS + 4,
            )
            .expect("effective receipt");
        assert_eq!(applied.lifecycle, CommandLifecycle::Applied);
    }

    #[test]
    fn capability_drift_rejects_confirmation_without_mutation() {
        let mut hub = ready_hub();
        let preview = hub
            .preview_package_install_release(plan(&hub))
            .expect("preview");
        let mut changed = ScenarioBuilder::new(1).build().initial.remove(0);
        changed.source_revision += 1;
        changed.capabilities.capabilities.insert(
            PACKAGE_UPDATER_CAPABILITY_ID.to_owned(),
            CapabilityState {
                capability_id: PACKAGE_UPDATER_CAPABILITY_ID.to_owned(),
                support: SupportState::Supported,
                enablement: EnablementState::Disabled,
                authorization: AuthorizationState::Authorized,
                reachability: ReachabilityState::Reachable,
                freshness: FreshnessState::Current,
                evidence_revision: changed.source_revision,
                observed_at_ms: BASE_TIME_MS + 1,
                fresh_until_ms: BASE_TIME_MS + 60_000,
                owner: PACKAGE_UPDATER_OWNER.to_owned(),
                reason: "owner_disabled".to_owned(),
                extensions: BTreeMap::new(),
            },
        );
        assert!(matches!(
            hub.accept_observation(changed, BASE_TIME_MS + 1),
            ObservationDecision::Accepted { .. }
        ));
        assert!(
            hub.confirm_package_install_release(
                &preview.operation_id,
                &preview.preview.preview_id,
                BASE_TIME_MS + 2
            )
            .is_err()
        );
        assert_eq!(
            hub.package_operation(&preview.operation_id)
                .expect("unchanged")
                .lifecycle,
            CommandLifecycle::Proposed
        );
    }

    #[test]
    fn preparation_does_not_consume_future_owner_delivery_parallelism() {
        let mut hub = ready_hub_with_count(2);
        let targets = ["sim-00001", "sim-00002"]
            .into_iter()
            .map(|device_id| {
                let revision = hub
                    .inspect(device_id, BASE_TIME_MS)
                    .expect("device")
                    .row
                    .identity
                    .identity_revision;
                (device_id.to_owned(), revision)
            })
            .collect();
        let mut package_plan = plan(&hub);
        package_plan.request.targets = targets;
        package_plan.max_parallelism = 1;
        let preview = hub
            .preview_package_install_release(package_plan)
            .expect("preview");
        let mut prepared = hub
            .confirm_package_install_release(
                &preview.operation_id,
                &preview.preview.preview_id,
                BASE_TIME_MS + 1,
            )
            .expect("confirm");
        for (index, device_id) in ["sim-00001", "sim-00002"].into_iter().enumerate() {
            prepared = hub
                .prepare_package_install_release_invocation(
                    &preview.operation_id,
                    device_id,
                    format!("package-owner-request-{}", index + 1),
                    BASE_TIME_MS + 2,
                )
                .expect("prepare invocation");
        }
        assert_eq!(prepared.max_parallelism, 1);
        assert_eq!(prepared.lifecycle, CommandLifecycle::Accepted);
        assert!(prepared.targets.iter().all(|target| {
            target.lifecycle == CommandLifecycle::Accepted
                && target.stage == PackageInstallStage::DispatchReady
                && target.invocation.is_some()
                && target.invocation_acknowledgement.is_none()
                && target.effective_receipt.is_none()
        }));
    }

    #[test]
    fn owner_claims_are_one_use_bounded_and_restart_safe() {
        let mut hub = ready_hub_with_count(2);
        let targets = ["sim-00001", "sim-00002"]
            .into_iter()
            .map(|device_id| {
                let revision = hub
                    .inspect(device_id, BASE_TIME_MS)
                    .expect("device")
                    .row
                    .identity
                    .identity_revision;
                (device_id.to_owned(), revision)
            })
            .collect();
        let mut package_plan = plan(&hub);
        package_plan.request.targets = targets;
        package_plan.max_parallelism = 1;
        let preview = hub
            .preview_package_install_release(package_plan)
            .expect("preview");
        hub.confirm_package_install_release(
            &preview.operation_id,
            &preview.preview.preview_id,
            BASE_TIME_MS + 1,
        )
        .expect("confirm");
        for (index, device_id) in ["sim-00001", "sim-00002"].into_iter().enumerate() {
            hub.prepare_package_install_release_invocation(
                &preview.operation_id,
                device_id,
                format!("package-owner-request-{}", index + 1),
                BASE_TIME_MS + 2,
            )
            .expect("prepare");
        }
        let invocation = hub
            .package_operation(&preview.operation_id)
            .expect("operation")
            .targets[0]
            .invocation
            .clone()
            .expect("invocation");
        let digest = format!("sha256:{}", "a".repeat(64));
        let claim = PackageUpdaterClaim {
            schema: fleet_contracts::PACKAGE_UPDATER_CLAIM_SCHEMA.to_owned(),
            claim_id: "claim-1".to_owned(),
            owner_id: "rusty-quest.package-updater".to_owned(),
            request_id: "claim-request-1".to_owned(),
            claimed_at_ms: BASE_TIME_MS + 3,
            expires_at_ms: BASE_TIME_MS + 30_000,
            invocation_sha256: digest.clone(),
            release_sha256: digest.clone(),
            target_sha256: digest,
            invocation,
        };
        hub.offer_package_updater_claim(claim.clone(), BASE_TIME_MS + 3)
            .expect("first bounded claim");
        let second_invocation = hub
            .package_operation(&preview.operation_id)
            .expect("operation")
            .targets[1]
            .invocation
            .clone()
            .expect("second invocation");
        let mut second = claim.clone();
        second.claim_id = "claim-2".to_owned();
        second.request_id = "claim-request-2".to_owned();
        second.invocation = second_invocation;
        let damaged_second = second.clone();
        assert_eq!(
            hub.offer_package_updater_claim(second, BASE_TIME_MS + 4)
                .expect_err("parallelism one"),
            HubError::new(
                "package_owner_scheduler_saturated",
                "bounded package delivery scheduler has no free claim slot"
            )
        );
        let mut damaged_snapshot = hub.snapshot();
        let damaged_target = &mut damaged_snapshot
            .package_operations
            .get_mut(&preview.operation_id)
            .expect("damaged operation")
            .targets[1];
        damaged_target
            .consumed_owner_claim_identities
            .push(ConsumedPackageUpdaterClaimIdentity {
                claim_id: damaged_second.claim_id.clone(),
                request_id: damaged_second.request_id.clone(),
            });
        damaged_target.owner_claim = Some(damaged_second);
        let damaged_operation = damaged_snapshot
            .package_operations
            .get(&preview.operation_id)
            .expect("damaged operation");
        let violations = damaged_operation
            .validate()
            .expect_err("parallelism damage must violate the operation contract");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].code, "operation_parallelism_exceeded");
        assert_eq!(
            FleetHub::restore(HubPolicy::default(), damaged_snapshot)
                .expect_err("snapshot cannot restore claims beyond frozen parallelism")
                .code,
            "snapshot_package_operations_invalid"
        );
        let mut restored =
            FleetHub::restore(HubPolicy::default(), hub.snapshot()).expect("claim restores");
        assert_eq!(
            restored
                .package_operation(&preview.operation_id)
                .expect("operation")
                .targets[0]
                .owner_claim,
            Some(claim.clone())
        );
        assert_eq!(
            restored
                .expire_package_updater_claims(BASE_TIME_MS + 30_000)
                .expect("expiry"),
            1
        );
        let retry_ready = restored
            .package_operation(&preview.operation_id)
            .expect("retry-ready operation");
        assert_eq!(
            retry_ready.targets[0].stage,
            PackageInstallStage::DispatchReady
        );
        assert!(retry_ready.targets[0].owner_claim.is_none());
        assert_eq!(
            retry_ready.targets[0].prior_owner_claims,
            vec![claim.clone()]
        );
        let mut restarted = FleetHub::restore(HubPolicy::default(), restored.snapshot())
            .expect("expired claim evidence restores");
        let oldest_claim = claim;
        for attempt in 2_i64..=18 {
            let claimed_at_ms = BASE_TIME_MS + 30_001 + (attempt - 2) * 1_500;
            let mut retry = oldest_claim.clone();
            retry.claim_id = format!("claim-{attempt}");
            retry.request_id = format!("claim-request-{attempt}");
            retry.claimed_at_ms = claimed_at_ms;
            retry.expires_at_ms = claimed_at_ms + 1_000;
            restarted
                .offer_package_updater_claim(retry, claimed_at_ms)
                .expect("fresh retry claim");
            restarted
                .expire_package_updater_claims(claimed_at_ms + 1_000)
                .expect("retry expiry");
        }
        let retained = restarted
            .package_operation(&preview.operation_id)
            .expect("retained replay authority");
        assert_eq!(retained.targets[0].prior_owner_claims.len(), 16);
        assert_eq!(
            retained.targets[0].consumed_owner_claim_identities.len(),
            18
        );
        let mut restarted = FleetHub::restore(HubPolicy::default(), restarted.snapshot())
            .expect("17+ consumed identities restore");
        let mut replay = oldest_claim;
        replay.claimed_at_ms = BASE_TIME_MS + 56_000;
        replay.expires_at_ms = BASE_TIME_MS + 57_000;
        assert_eq!(
            restarted
                .offer_package_updater_claim(replay, BASE_TIME_MS + 56_000)
                .expect_err("oldest request cannot replay after evidence truncation and restart")
                .code,
            "package_owner_claim_replay"
        );
        for attempt in 19_i64..=64 {
            let claimed_at_ms = BASE_TIME_MS + 56_001 + (attempt - 19) * 80;
            let mut retry = restarted
                .package_operation(&preview.operation_id)
                .expect("retry operation")
                .targets[0]
                .prior_owner_claims
                .last()
                .expect("prior claim template")
                .clone();
            retry.claim_id = format!("claim-{attempt}");
            retry.request_id = format!("claim-request-{attempt}");
            retry.claimed_at_ms = claimed_at_ms;
            retry.expires_at_ms = claimed_at_ms + 50;
            restarted
                .offer_package_updater_claim(retry, claimed_at_ms)
                .expect("bounded replay-authority claim");
            restarted
                .expire_package_updater_claims(claimed_at_ms + 50)
                .expect("bounded replay-authority expiry");
        }
        let exhausted = restarted
            .package_operation(&preview.operation_id)
            .expect("exhausted target");
        assert_eq!(exhausted.targets[0].lifecycle, CommandLifecycle::Failed);
        assert_eq!(exhausted.targets[0].stage, PackageInstallStage::Failed);
        assert_eq!(
            exhausted.targets[0].consumed_owner_claim_identities.len(),
            MAX_CONSUMED_PACKAGE_OWNER_CLAIMS
        );
        assert_eq!(
            restarted
                .select_package_updater_offer(BASE_TIME_MS + 59_700)
                .expect("other target remains independently offerable")
                .device_id,
            "sim-00002"
        );
        FleetHub::restore(HubPolicy::default(), restarted.snapshot())
            .expect("fail-closed replay authority restores");
    }

    #[test]
    fn offer_selection_uses_logical_expiry_and_exact_capacity_without_mutation() {
        let mut hub = ready_hub_with_count(2);
        let targets = ["sim-00001", "sim-00002"]
            .into_iter()
            .map(|device_id| {
                let revision = hub
                    .inspect(device_id, BASE_TIME_MS)
                    .expect("device")
                    .row
                    .identity
                    .identity_revision;
                (device_id.to_owned(), revision)
            })
            .collect();
        let mut package_plan = plan(&hub);
        package_plan.request.targets = targets;
        package_plan.max_parallelism = 1;
        let preview = hub
            .preview_package_install_release(package_plan)
            .expect("preview");
        hub.confirm_package_install_release(
            &preview.operation_id,
            &preview.preview.preview_id,
            BASE_TIME_MS + 1,
        )
        .expect("confirm");
        for (index, device_id) in ["sim-00001", "sim-00002"].into_iter().enumerate() {
            hub.prepare_package_install_release_invocation(
                &preview.operation_id,
                device_id,
                format!("package-owner-request-{}", index + 1),
                BASE_TIME_MS + 2,
            )
            .expect("prepare");
        }
        let invocation = hub
            .select_package_updater_offer(BASE_TIME_MS + 3)
            .expect("initial offer");
        let digest = format!("sha256:{}", "a".repeat(64));
        let claim = PackageUpdaterClaim {
            schema: fleet_contracts::PACKAGE_UPDATER_CLAIM_SCHEMA.to_owned(),
            claim_id: "capacity-claim-1".to_owned(),
            owner_id: PACKAGE_UPDATER_OWNER.to_owned(),
            request_id: "capacity-request-1".to_owned(),
            claimed_at_ms: BASE_TIME_MS + 3,
            expires_at_ms: BASE_TIME_MS + 10,
            invocation_sha256: digest.clone(),
            release_sha256: digest.clone(),
            target_sha256: digest,
            invocation,
        };
        hub.offer_package_updater_claim(claim.clone(), BASE_TIME_MS + 3)
            .expect("claim");
        assert!(hub.select_package_updater_offer(BASE_TIME_MS + 4).is_none());

        let before_logical_expiry = hub.snapshot();
        assert_eq!(
            hub.select_package_updater_offer(BASE_TIME_MS + 10)
                .expect("logically expired claim becomes offerable")
                .device_id,
            "sim-00001"
        );
        assert_eq!(hub.snapshot(), before_logical_expiry);

        hub.admit_authenticated_package_updater_acknowledgement(
            PackageUpdaterInvocationAcknowledgement {
                schema: PACKAGE_UPDATER_ACK_SCHEMA.to_owned(),
                operation_id: preview.operation_id,
                device_id: "sim-00001".to_owned(),
                owner_action_request_id: claim.invocation.owner_action_request_id,
                accepted: true,
                code: "accepted".to_owned(),
                acknowledged_at_ms: BASE_TIME_MS + 5,
            },
            BASE_TIME_MS + 5,
        )
        .expect("acknowledgement");
        assert!(
            hub.select_package_updater_offer(BASE_TIME_MS + 10)
                .is_none()
        );
        let restarted = FleetHub::restore(HubPolicy::default(), hub.snapshot()).expect("restart");
        assert!(
            restarted
                .select_package_updater_offer(BASE_TIME_MS + 10)
                .is_none()
        );
    }
}
