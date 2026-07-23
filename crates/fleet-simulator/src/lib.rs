// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deterministic synthetic fleet datasets and damaged-message variants.

use std::collections::BTreeMap;

use fleet_contracts::{
    AuthorizationState, CapabilitySnapshot, CapabilityState, ConditionFamily, ConditionState,
    DeviceIdentity, DeviceObservation, EnablementState, FreshnessState, KioskState,
    ReachabilityState, Sensitivity, StatusCondition, StatusSource, SupportState,
};
use serde::{Deserialize, Serialize};

pub const BASE_TIME_MS: i64 = 2_000_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioMutationKind {
    Reconnect,
    Replay,
    ReorderedRevision,
    DuplicateIdentity,
    Reenrollment,
    CapabilityDowngrade,
    PartialFamilies,
    DamagedMessage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioMutation {
    pub at_ms: i64,
    pub kind: ScenarioMutationKind,
    pub observation: DeviceObservation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FleetScenario {
    pub schema: String,
    pub seed: u64,
    pub initial: Vec<DeviceObservation>,
    pub mutations: Vec<ScenarioMutation>,
}

#[derive(Clone, Copy, Debug)]
pub struct ScenarioBuilder {
    device_count: usize,
    seed: u64,
}

impl ScenarioBuilder {
    #[must_use]
    pub fn new(device_count: usize) -> Self {
        Self {
            device_count,
            seed: 0x5255_5354_5946_4c54,
        }
    }

    #[must_use]
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    #[must_use]
    pub fn build(self) -> FleetScenario {
        let initial = (0..self.device_count)
            .map(|index| observation(index, 1, 1, self.seed))
            .collect::<Vec<_>>();
        let mut mutations = Vec::new();
        if let Some(first) = initial.first() {
            let mut reconnect = first.clone();
            reconnect.source_revision = 3;
            reconnect.received_time_ms += 5_000;
            reconnect.source_time_ms += 5_000;
            mutations.push(ScenarioMutation {
                at_ms: BASE_TIME_MS + 5_000,
                kind: ScenarioMutationKind::Reconnect,
                observation: reconnect,
            });
            mutations.push(ScenarioMutation {
                at_ms: BASE_TIME_MS + 5_001,
                kind: ScenarioMutationKind::Replay,
                observation: first.clone(),
            });
            let mut reordered = first.clone();
            reordered.source_revision = 2;
            mutations.push(ScenarioMutation {
                at_ms: BASE_TIME_MS + 5_002,
                kind: ScenarioMutationKind::ReorderedRevision,
                observation: reordered,
            });
            let mut reenrollment = first.clone();
            reenrollment.identity.identity_revision = 2;
            reenrollment.source_revision = 1;
            reenrollment
                .identity
                .tags
                .insert("enrollment".to_owned(), "synthetic-reenrollment".to_owned());
            mutations.push(ScenarioMutation {
                at_ms: BASE_TIME_MS + 9_000,
                kind: ScenarioMutationKind::Reenrollment,
                observation: reenrollment,
            });
        }
        if let Some(second) = initial.get(1) {
            let mut duplicate = second.clone();
            duplicate.source_revision = 2;
            duplicate.identity.display_name = "Conflicting synthetic identity".to_owned();
            mutations.push(ScenarioMutation {
                at_ms: BASE_TIME_MS + 5_500,
                kind: ScenarioMutationKind::DuplicateIdentity,
                observation: duplicate,
            });
            let mut downgrade = second.clone();
            downgrade.source_revision = 2;
            if let Some(control) = downgrade
                .capabilities
                .capabilities
                .get_mut("participating_app_control")
            {
                control.authorization = AuthorizationState::Unauthorized;
                control.reason = "grant_expired".to_owned();
                control.evidence_revision += 1;
            }
            mutations.push(ScenarioMutation {
                at_ms: BASE_TIME_MS + 6_000,
                kind: ScenarioMutationKind::CapabilityDowngrade,
                observation: downgrade,
            });
        }
        if let Some(third) = initial.get(2) {
            let mut partial = third.clone();
            partial.source_revision = 2;
            partial.battery_percent = None;
            partial.conditions.retain(|condition| {
                condition.family != ConditionFamily::Power
                    && condition.family != ConditionFamily::Media
            });
            mutations.push(ScenarioMutation {
                at_ms: BASE_TIME_MS + 7_000,
                kind: ScenarioMutationKind::PartialFamilies,
                observation: partial,
            });
        }
        if let Some(fourth) = initial.get(3) {
            let mut damaged = fourth.clone();
            damaged.source_revision = 0;
            damaged.battery_percent = Some(240);
            mutations.push(ScenarioMutation {
                at_ms: BASE_TIME_MS + 8_000,
                kind: ScenarioMutationKind::DamagedMessage,
                observation: damaged,
            });
        }
        FleetScenario {
            schema: "rusty.fleet.simulation_scenario.v1".to_owned(),
            seed: self.seed,
            initial,
            mutations,
        }
    }
}

#[must_use]
pub fn supported_scale_fixtures() -> [usize; 5] {
    [4, 50, 250, 1_000, 5_000]
}

fn observation(
    index: usize,
    identity_revision: u64,
    source_revision: u64,
    seed: u64,
) -> DeviceObservation {
    let device_number = index + 1;
    let device_id = format!("sim-{device_number:05}");
    let cohort = if index.is_multiple_of(2) {
        "lab-a"
    } else {
        "lab-b"
    };
    let battery_percent = ((seed.wrapping_add(index as u64 * 17) % 91) + 10) as u8;
    let observed_at = BASE_TIME_MS + index as i64;
    let mut tags = BTreeMap::new();
    tags.insert("cohort".to_owned(), cohort.to_owned());
    tags.insert("fixture".to_owned(), "synthetic".to_owned());

    let conditions = vec![
        condition(
            ConditionFamily::Freshness,
            ConditionState::Current,
            "local",
            observed_at,
            source_revision,
        ),
        condition(
            ConditionFamily::Power,
            if battery_percent < 20 {
                ConditionState::Degraded
            } else {
                ConditionState::Current
            },
            if battery_percent < 20 {
                "low_battery"
            } else {
                "battery_observed"
            },
            observed_at,
            source_revision,
        ),
        condition(
            ConditionFamily::Application,
            ConditionState::Current,
            "participating_app_receipt",
            observed_at,
            source_revision,
        ),
    ];

    let mut capabilities = BTreeMap::new();
    capabilities.insert(
        "monitoring".to_owned(),
        capability(
            "monitoring",
            "fleet_agent",
            observed_at,
            source_revision,
            true,
        ),
    );
    capabilities.insert(
        "participating_app_control".to_owned(),
        capability(
            "participating_app_control",
            "rusty_kiosk",
            observed_at,
            source_revision,
            true,
        ),
    );
    capabilities.insert(
        "adb".to_owned(),
        CapabilityState {
            capability_id: "adb".to_owned(),
            support: SupportState::Supported,
            enablement: EnablementState::Disabled,
            authorization: AuthorizationState::Unknown,
            reachability: ReachabilityState::Disconnected,
            freshness: FreshnessState::Current,
            evidence_revision: source_revision,
            observed_at_ms: observed_at,
            fresh_until_ms: observed_at + 60_000,
            owner: "rusty_quest".to_owned(),
            reason: "optional_capability_disabled".to_owned(),
            extensions: BTreeMap::new(),
        },
    );

    DeviceObservation {
        schema: "rusty.fleet.device_observation.v1".to_owned(),
        identity: DeviceIdentity {
            device_id,
            identity_revision,
            display_name: format!("Quest {device_number:04}"),
            model: if index.is_multiple_of(3) {
                "Quest 3S".to_owned()
            } else {
                "Quest 3".to_owned()
            },
            hardware_class: "standalone_xr".to_owned(),
            tags,
            extensions: BTreeMap::new(),
        },
        source_revision,
        source_time_ms: observed_at,
        received_time_ms: observed_at,
        battery_percent: Some(battery_percent),
        charging: Some(index.is_multiple_of(5)),
        foreground_app: Some("org.example.synthetic.kiosk".to_owned()),
        kiosk_state: KioskState::Active,
        conditions,
        capabilities: CapabilitySnapshot {
            capabilities,
            extensions: BTreeMap::new(),
        },
        streams: Vec::new(),
        extensions: BTreeMap::new(),
    }
}

fn condition(
    family: ConditionFamily,
    state: ConditionState,
    reason: &str,
    observed_at: i64,
    revision: u64,
) -> StatusCondition {
    StatusCondition {
        family,
        state,
        reason: reason.to_owned(),
        message: reason.replace('_', " "),
        source_time_ms: observed_at,
        received_time_ms: observed_at,
        accepted_revision: revision,
        fresh_until_ms: observed_at + 60_000,
        source: StatusSource {
            adapter_id: "synthetic-fixture".to_owned(),
            owner: "fleet-simulator".to_owned(),
            authority_revision: revision,
        },
        sensitivity: Sensitivity::Operator,
        extensions: BTreeMap::new(),
    }
}

fn capability(
    capability_id: &str,
    owner: &str,
    observed_at: i64,
    revision: u64,
    ready: bool,
) -> CapabilityState {
    CapabilityState {
        capability_id: capability_id.to_owned(),
        support: SupportState::Supported,
        enablement: if ready {
            EnablementState::Enabled
        } else {
            EnablementState::Disabled
        },
        authorization: if ready {
            AuthorizationState::Authorized
        } else {
            AuthorizationState::Unauthorized
        },
        reachability: if ready {
            ReachabilityState::Reachable
        } else {
            ReachabilityState::Unavailable
        },
        freshness: FreshnessState::Current,
        evidence_revision: revision,
        observed_at_ms: observed_at,
        fresh_until_ms: observed_at + 60_000,
        owner: owner.to_owned(),
        reason: if ready {
            "ready".to_owned()
        } else {
            "disabled".to_owned()
        },
        extensions: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use fleet_contracts::ValidateContract;

    use super::{ScenarioBuilder, ScenarioMutationKind, supported_scale_fixtures};

    #[test]
    fn every_declared_scale_is_deterministic_and_valid() {
        for count in supported_scale_fixtures() {
            let first = ScenarioBuilder::new(count).build();
            let second = ScenarioBuilder::new(count).build();
            assert_eq!(first, second);
            assert_eq!(first.initial.len(), count);
            assert!(
                first
                    .initial
                    .iter()
                    .all(|observation| observation.validate().is_ok())
            );
        }
    }

    #[test]
    fn damaged_and_downgrade_paths_are_present() {
        let scenario = ScenarioBuilder::new(4).build();
        assert!(
            scenario
                .mutations
                .iter()
                .any(|mutation| mutation.kind == ScenarioMutationKind::DamagedMessage)
        );
        assert!(
            scenario
                .mutations
                .iter()
                .any(|mutation| mutation.kind == ScenarioMutationKind::CapabilityDowngrade)
        );
    }
}
