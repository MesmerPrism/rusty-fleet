// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ContractViolation, ValidateContract, finish, require_nonempty};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportState {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnablementState {
    Enabled,
    Disabled,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationState {
    Authorized,
    Unauthorized,
    Restricted,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityState {
    Reachable,
    Disconnected,
    Unavailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessState {
    Current,
    Stale,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityState {
    pub capability_id: String,
    pub support: SupportState,
    pub enablement: EnablementState,
    pub authorization: AuthorizationState,
    pub reachability: ReachabilityState,
    pub freshness: FreshnessState,
    pub evidence_revision: u64,
    pub observed_at_ms: i64,
    pub fresh_until_ms: i64,
    pub owner: String,
    pub reason: String,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl CapabilityState {
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.support == SupportState::Supported
            && self.enablement == EnablementState::Enabled
            && self.authorization == AuthorizationState::Authorized
            && self.reachability == ReachabilityState::Reachable
            && self.freshness == FreshnessState::Current
    }
}

impl ValidateContract for CapabilityState {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        require_nonempty(&mut failures, &self.capability_id, "capability_id");
        require_nonempty(&mut failures, &self.owner, "owner");
        require_nonempty(&mut failures, &self.reason, "reason");
        if self.evidence_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_revision",
                "evidence_revision",
                "evidence revision must be greater than zero",
            ));
        }
        if self.fresh_until_ms < self.observed_at_ms {
            failures.push(ContractViolation::new(
                "invalid_freshness",
                "fresh_until_ms",
                "freshness deadline must not precede observation time",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CapabilitySnapshot {
    #[serde(default)]
    pub capabilities: BTreeMap<String, CapabilityState>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl CapabilitySnapshot {
    #[must_use]
    pub fn get(&self, capability_id: &str) -> Option<&CapabilityState> {
        self.capabilities.get(capability_id)
    }
}

impl ValidateContract for CapabilitySnapshot {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        for (key, capability) in &self.capabilities {
            if key != &capability.capability_id {
                failures.push(ContractViolation::new(
                    "capability_key_mismatch",
                    &format!("capabilities.{key}"),
                    "map key must equal capability_id",
                ));
            }
            if let Err(nested) = capability.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("capabilities.{key}.{}", failure.path),
                    ..failure
                }));
            }
        }
        finish(failures)
    }
}
