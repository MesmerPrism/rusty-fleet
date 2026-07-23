// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ContractViolation, FleetQuery, ValidateContract, finish, require_nonempty};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandLifecycle {
    Proposed,
    Accepted,
    Rejected,
    Dispatched,
    Running,
    Applied,
    Failed,
    Expired,
    CancellationRequested,
    Cancelled,
    CleanupPending,
    Cleaned,
}

impl CommandLifecycle {
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Rejected | Self::Failed | Self::Expired | Self::Cancelled | Self::Cleaned
        )
    }

    #[must_use]
    pub fn can_transition_to(self, next: Self) -> bool {
        use CommandLifecycle::{
            Accepted, Applied, CancellationRequested, Cancelled, Cleaned, CleanupPending,
            Dispatched, Expired, Failed, Proposed, Rejected, Running,
        };
        matches!(
            (self, next),
            (Proposed, Accepted | Rejected | Expired)
                | (
                    Accepted,
                    Dispatched | CancellationRequested | Expired | Failed
                )
                | (
                    Dispatched,
                    Running | Applied | CancellationRequested | Failed | Expired
                )
                | (Running, Applied | CancellationRequested | Failed | Expired)
                | (
                    CancellationRequested,
                    Cancelled | Applied | Failed | CleanupPending
                )
                | (Applied, CleanupPending | Cleaned)
                | (Failed | Cancelled | Expired, CleanupPending | Cleaned)
                | (CleanupPending, Cleaned | Failed)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TargetSnapshot {
    pub snapshot_id: String,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub query: FleetQuery,
    pub result_revision: u64,
    pub identity_revisions: BTreeMap<String, u64>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl ValidateContract for TargetSnapshot {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        require_nonempty(&mut failures, &self.snapshot_id, "snapshot_id");
        if self.expires_at_ms <= self.created_at_ms {
            failures.push(ContractViolation::new(
                "invalid_expiry",
                "expires_at_ms",
                "target snapshot expiry must follow creation",
            ));
        }
        if self.result_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_revision",
                "result_revision",
                "result revision must be greater than zero",
            ));
        }
        if self
            .identity_revisions
            .values()
            .any(|revision| *revision == 0)
        {
            failures.push(ContractViolation::new(
                "invalid_identity_revision",
                "identity_revisions",
                "all target identity revisions must be greater than zero",
            ));
        }
        if self
            .identity_revisions
            .keys()
            .any(|device_id| device_id.trim().is_empty())
        {
            failures.push(ContractViolation::new(
                "invalid_device_id",
                "identity_revisions",
                "target device IDs must not be empty",
            ));
        }
        if self.identity_revisions.len() > 10_000 || self.extensions.len() > 64 {
            failures.push(ContractViolation::new(
                "target_snapshot_too_large",
                "identity_revisions",
                "target snapshots support at most 10000 targets and 64 extension fields",
            ));
        }
        if let Err(mut nested) = self.query.validate() {
            failures.append(&mut nested);
        }
        finish(failures)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetEligibility {
    Eligible,
    Warning,
    Excluded,
    RefreshRequired,
    ChangedSincePreview,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationTargetResult {
    pub device_id: String,
    pub identity_revision: u64,
    pub eligibility: TargetEligibility,
    pub lifecycle: CommandLifecycle,
    pub reason_code: String,
    pub message: String,
    pub last_transition_ms: i64,
    pub receipt_id: Option<String>,
    pub retry_eligible: bool,
    pub cancel_eligible: bool,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationLedger {
    pub schema: String,
    pub operation_id: String,
    pub action_id: String,
    pub created_at_ms: i64,
    pub target_snapshot: TargetSnapshot,
    pub lifecycle: CommandLifecycle,
    pub cleanup_required: bool,
    #[serde(default)]
    pub targets: Vec<OperationTargetResult>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl ValidateContract for OperationLedger {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.operation_ledger.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.operation_ledger.v1",
            ));
        }
        require_nonempty(&mut failures, &self.operation_id, "operation_id");
        require_nonempty(&mut failures, &self.action_id, "action_id");
        if self.targets.len() > 10_000 || self.extensions.len() > 64 {
            failures.push(ContractViolation::new(
                "operation_ledger_too_large",
                "targets",
                "operation ledgers support at most 10000 targets and 64 extension fields",
            ));
        }
        if let Err(mut nested) = self.target_snapshot.validate() {
            failures.append(&mut nested);
        }
        if self.targets.len() != self.target_snapshot.identity_revisions.len() {
            failures.push(ContractViolation::new(
                "target_ledger_mismatch",
                "targets",
                "operation ledger must retain one result for every snapshotted target",
            ));
        }
        let mut seen = BTreeMap::new();
        for (index, target) in self.targets.iter().enumerate() {
            require_nonempty(
                &mut failures,
                &target.device_id,
                &format!("targets[{index}].device_id"),
            );
            require_nonempty(
                &mut failures,
                &target.reason_code,
                &format!("targets[{index}].reason_code"),
            );
            require_nonempty(
                &mut failures,
                &target.message,
                &format!("targets[{index}].message"),
            );
            if target.identity_revision == 0 {
                failures.push(ContractViolation::new(
                    "invalid_identity_revision",
                    &format!("targets[{index}].identity_revision"),
                    "target identity revision must be greater than zero",
                ));
            }
            if seen.insert(&target.device_id, index).is_some() {
                failures.push(ContractViolation::new(
                    "duplicate_target",
                    &format!("targets[{index}].device_id"),
                    "operation target occurs more than once",
                ));
            }
            if self
                .target_snapshot
                .identity_revisions
                .get(&target.device_id)
                .is_none_or(|revision| *revision != target.identity_revision)
            {
                failures.push(ContractViolation::new(
                    "target_snapshot_binding_mismatch",
                    &format!("targets[{index}]"),
                    "target identity must match the frozen target snapshot",
                ));
            }
        }
        finish(failures)
    }
}
