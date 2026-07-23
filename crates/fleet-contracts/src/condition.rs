// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ContractViolation, ValidateContract, finish, require_nonempty};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionFamily {
    Identity,
    Freshness,
    Power,
    Application,
    Control,
    Privileged,
    Media,
    Work,
    Alert,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionState {
    Current,
    InProgress,
    Busy,
    Stale,
    Unknown,
    Unsupported,
    Disabled,
    Unauthorized,
    Disconnected,
    Unavailable,
    Degraded,
    Failed,
    Critical,
    Restricted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Public,
    Operator,
    Restricted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSource {
    pub adapter_id: String,
    pub owner: String,
    pub authority_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusCondition {
    pub family: ConditionFamily,
    pub state: ConditionState,
    pub reason: String,
    pub message: String,
    pub source_time_ms: i64,
    pub received_time_ms: i64,
    pub accepted_revision: u64,
    pub fresh_until_ms: i64,
    pub source: StatusSource,
    pub sensitivity: Sensitivity,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl StatusCondition {
    #[must_use]
    pub fn age_ms(&self, now_ms: i64) -> i64 {
        now_ms.saturating_sub(self.received_time_ms).max(0)
    }

    #[must_use]
    pub fn is_stale_at(&self, now_ms: i64) -> bool {
        now_ms > self.fresh_until_ms || self.state == ConditionState::Stale
    }
}

impl ValidateContract for StatusCondition {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        require_nonempty(&mut failures, &self.reason, "reason");
        require_nonempty(&mut failures, &self.message, "message");
        require_nonempty(&mut failures, &self.source.adapter_id, "source.adapter_id");
        require_nonempty(&mut failures, &self.source.owner, "source.owner");
        if self.accepted_revision == 0 || self.source.authority_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_revision",
                "accepted_revision",
                "accepted and authority revisions must be greater than zero",
            ));
        }
        if self.fresh_until_ms < self.received_time_ms {
            failures.push(ContractViolation::new(
                "invalid_freshness",
                "fresh_until_ms",
                "freshness deadline must not precede receive time",
            ));
        }
        finish(failures)
    }
}
