// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{ContractViolation, ValidateContract, finish, require_nonempty};

pub const Q2Q_STEREO_PLAN_SCHEMA: &str = "rusty.fleet.q2q_stereo_plan.v1";
pub const Q2Q_STEREO_EVIDENCE_SCHEMA: &str = "rusty.fleet.q2q_stereo_evidence.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Q2qStereoAdapter {
    InfrastructureLan,
    WifiDirect,
    AuthenticatedTlsRelay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Q2qStereoPhase {
    ReceiverStart,
    ReceiverReady,
    SenderStart,
    Streaming,
    StopRequested,
    Stopped,
    CleanupVerified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Q2qStereoHealth {
    Pending,
    Current,
    Degraded,
    Failed,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Q2qStereoTarget {
    /// Fleet-local identity only. Device serials remain private run input.
    pub target_id: String,
    pub identity_revision: u64,
    pub selection_digest_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Q2qStereoTargetSelection {
    pub selection_id: String,
    pub immutable: bool,
    pub targets: Vec<Q2qStereoTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Q2qStereoDirection {
    pub direction_id: String,
    pub source_target_id: String,
    pub sink_target_id: String,
    pub conservative_duration_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Q2qStereoPlanStep {
    pub ordinal: u32,
    pub adapter: Q2qStereoAdapter,
    pub direction_id: String,
    pub phase: Q2qStereoPhase,
}

/// Fleet's deliberately bounded projection of a product-owned media run.
///
/// Endpoints, media bytes, pairing secrets, relay credentials, and Manifold
/// session bodies are intentionally absent. Fleet selects exactly two targets,
/// enforces receiver-first scheduling, and projects only health/progress/stop
/// plus immutable evidence references.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Q2qStereoPlan {
    pub schema: String,
    pub run_id: String,
    pub selection: Q2qStereoTargetSelection,
    pub adapter_order: Vec<Q2qStereoAdapter>,
    pub directions: Vec<Q2qStereoDirection>,
    pub steps: Vec<Q2qStereoPlanStep>,
    pub evidence_root_reference: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Q2qStereoEvidenceProjection {
    pub schema: String,
    pub run_id: String,
    pub selection_id: String,
    pub adapter: Q2qStereoAdapter,
    pub direction_id: String,
    pub phase: Q2qStereoPhase,
    pub health: Q2qStereoHealth,
    pub stop_requested: bool,
    pub observed_at_ms: i64,
    #[serde(default)]
    pub evidence_references: BTreeMap<String, String>,
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_bounded_public_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

impl ValidateContract for Q2qStereoPlan {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != Q2Q_STEREO_PLAN_SCHEMA {
            failures.push(ContractViolation::new(
                "unsupported_schema",
                "schema",
                "Q2Q stereo plan schema is not supported",
            ));
        }
        require_nonempty(&mut failures, &self.run_id, "run_id");
        require_nonempty(
            &mut failures,
            &self.selection.selection_id,
            "selection.selection_id",
        );
        if !is_bounded_public_token(&self.run_id)
            || !is_bounded_public_token(&self.selection.selection_id)
            || !is_bounded_public_token(&self.evidence_root_reference)
        {
            failures.push(ContractViolation::new(
                "invalid_public_reference",
                "run_id/selection.selection_id/evidence_root_reference",
                "Q2Q Fleet identifiers and evidence references must be bounded lowercase tokens",
            ));
        }
        require_nonempty(
            &mut failures,
            &self.evidence_root_reference,
            "evidence_root_reference",
        );
        if !self.selection.immutable || self.selection.targets.len() != 2 {
            failures.push(ContractViolation::new(
                "invalid_target_selection",
                "selection",
                "Q2Q stereo requires one immutable selection containing exactly two targets",
            ));
        }
        let target_ids = self
            .selection
            .targets
            .iter()
            .map(|target| target.target_id.as_str())
            .collect::<BTreeSet<_>>();
        if target_ids.len() != self.selection.targets.len() {
            failures.push(ContractViolation::new(
                "duplicate_target",
                "selection.targets",
                "selected target identities must be distinct",
            ));
        }
        for (index, target) in self.selection.targets.iter().enumerate() {
            require_nonempty(
                &mut failures,
                &target.target_id,
                &format!("selection.targets[{index}].target_id"),
            );
            if !target.target_id.starts_with("target.")
                || !is_bounded_public_token(&target.target_id)
                || target.identity_revision == 0
                || !is_sha256(&target.selection_digest_sha256)
            {
                failures.push(ContractViolation::new(
                    "invalid_target_pin",
                    &format!("selection.targets[{index}]"),
                    "target identity revision and lowercase SHA-256 selection digest are required",
                ));
            }
        }
        if self.adapter_order
            != [
                Q2qStereoAdapter::InfrastructureLan,
                Q2qStereoAdapter::WifiDirect,
                Q2qStereoAdapter::AuthenticatedTlsRelay,
            ]
        {
            failures.push(ContractViolation::new(
                "invalid_adapter_order",
                "adapter_order",
                "adapter ladder must be infrastructure LAN, Wi-Fi Direct, then authenticated TLS relay",
            ));
        }
        let mut directions = BTreeMap::new();
        for (index, direction) in self.directions.iter().enumerate() {
            require_nonempty(
                &mut failures,
                &direction.direction_id,
                &format!("directions[{index}].direction_id"),
            );
            if !is_bounded_public_token(&direction.direction_id)
                || direction.source_target_id == direction.sink_target_id
                || !target_ids.contains(direction.source_target_id.as_str())
                || !target_ids.contains(direction.sink_target_id.as_str())
                || !(1..=3_600).contains(&direction.conservative_duration_seconds)
                || directions
                    .insert(direction.direction_id.as_str(), direction)
                    .is_some()
            {
                failures.push(ContractViolation::new(
                    "invalid_direction",
                    &format!("directions[{index}]"),
                    "direction must be unique, bounded, and connect the two selected targets",
                ));
            }
        }
        let mut prior_ordinal = 0;
        let mut phase_rank = BTreeMap::<(Q2qStereoAdapter, &str), u8>::new();
        for (index, step) in self.steps.iter().enumerate() {
            let rank = match step.phase {
                Q2qStereoPhase::ReceiverStart => 1,
                Q2qStereoPhase::ReceiverReady => 2,
                Q2qStereoPhase::SenderStart => 3,
                Q2qStereoPhase::Streaming => 4,
                Q2qStereoPhase::StopRequested => 5,
                Q2qStereoPhase::Stopped => 6,
                Q2qStereoPhase::CleanupVerified => 7,
            };
            let key = (step.adapter, step.direction_id.as_str());
            let previous = phase_rank.get(&key).copied().unwrap_or(0);
            if step.ordinal <= prior_ordinal
                || !directions.contains_key(step.direction_id.as_str())
                || rank != previous + 1
            {
                failures.push(ContractViolation::new(
                    "invalid_receiver_first_schedule",
                    &format!("steps[{index}]"),
                    "steps must be globally ordered and complete receiver-ready before sender start",
                ));
            }
            prior_ordinal = step.ordinal;
            phase_rank.insert(key, rank);
        }
        for adapter in &self.adapter_order {
            for direction in directions.keys() {
                if phase_rank.get(&(*adapter, *direction)) != Some(&7) {
                    failures.push(ContractViolation::new(
                        "incomplete_schedule",
                        "steps",
                        "every adapter and direction requires receiver-first start through verified cleanup",
                    ));
                }
            }
        }
        finish(failures)
    }
}

impl ValidateContract for Q2qStereoEvidenceProjection {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != Q2Q_STEREO_EVIDENCE_SCHEMA {
            failures.push(ContractViolation::new(
                "unsupported_schema",
                "schema",
                "Q2Q stereo evidence schema is not supported",
            ));
        }
        require_nonempty(&mut failures, &self.run_id, "run_id");
        require_nonempty(&mut failures, &self.selection_id, "selection_id");
        require_nonempty(&mut failures, &self.direction_id, "direction_id");
        if !is_bounded_public_token(&self.run_id)
            || !is_bounded_public_token(&self.selection_id)
            || !is_bounded_public_token(&self.direction_id)
            || self.evidence_references.len() > 16
        {
            failures.push(ContractViolation::new(
                "invalid_public_reference",
                "run_id/selection_id/direction_id/evidence_references",
                "Q2Q evidence identifiers must be bounded lowercase tokens",
            ));
        }
        if self.observed_at_ms <= 0 {
            failures.push(ContractViolation::new(
                "invalid_observation_time",
                "observed_at_ms",
                "observation time must be positive",
            ));
        }
        for (family, reference) in &self.evidence_references {
            require_nonempty(&mut failures, family, "evidence_references.key");
            require_nonempty(&mut failures, reference, "evidence_references.value");
            if !is_bounded_public_token(family) || !is_bounded_public_token(reference) {
                failures.push(ContractViolation::new(
                    "invalid_public_reference",
                    "evidence_references",
                    "Q2Q evidence references cannot contain device, endpoint, path, or credential syntax",
                ));
            }
        }
        finish(failures)
    }
}
