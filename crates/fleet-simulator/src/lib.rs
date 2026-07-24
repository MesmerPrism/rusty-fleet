// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deterministic synthetic fleet datasets and damaged-message variants.

use std::collections::BTreeMap;

use fleet_contracts::{
    ApplicationLifecycle, ApplicationObservation, AuthorizationState, CapabilitySnapshot,
    CapabilityState, ConditionFamily, ConditionState, ContentProgressPolicy, DeviceIdentity,
    DeviceObservation, EnablementState, EpochContinuity, ExperimentRun, FactProvenance,
    ForegroundAuthority, ForegroundState, FreshnessState, KioskState, PowerObservation,
    ProgressApplicability, ProgressStage, ProgressStageEvidence, ReachabilityState,
    RecordingArtifact, RecordingArtifactState, SelectionMethod, Sensitivity, StatusCondition,
    StatusSource, StreamDescriptor, SupportState,
};
use serde::{Deserialize, Serialize};

pub const BASE_TIME_MS: i64 = 2_000_000_000_000;
pub const MIXED_FRESHNESS_TIME_MS: i64 = BASE_TIME_MS + 600_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioMutationKind {
    Reconnect,
    AgentRestart,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum M1LifecycleStepKind {
    SleepCheckIn,
    KeepAlive,
    StaleWhileSleeping,
    WakeCheckIn,
    RouteLoss,
    DuplicateCheckIn,
    StaleRevision,
    AgentUpgrade,
    OldEpochReplay,
    RouteRecovery,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct M1LifecycleStep {
    pub at_ms: i64,
    pub kind: M1LifecycleStepKind,
    pub device_id: String,
    pub observation: Option<DeviceObservation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct M1LifecycleScenario {
    pub schema: String,
    pub seed: u64,
    pub initial: Vec<DeviceObservation>,
    pub steps: Vec<M1LifecycleStep>,
    pub final_time_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixedFreshnessFixture {
    pub schema: String,
    pub now_ms: i64,
    pub observations: Vec<DeviceObservation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatastreamScenarioKind {
    AmbiguousSelection,
    NativeDescriptorDrift,
    ComponentDiscontinuity,
    ClockReset,
    ClockDegraded,
    ValidSilence,
    NoData,
    Stalled,
    ByteOnlyActivity,
    ChangingContent,
    StaticContent,
    DecodeOrSchemaFailure,
    SinkFailure,
    RecordingFailure,
    BudgetRejected,
    Recovering,
    ReplayValidated,
    CleanupFailure,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DatastreamScenario {
    pub schema: String,
    pub kind: DatastreamScenarioKind,
    pub expected_stage: ProgressStage,
    pub expected_condition: ConditionState,
    pub descriptor: StreamDescriptor,
}

#[must_use]
pub fn datastream_scenarios() -> Vec<DatastreamScenario> {
    use DatastreamScenarioKind::{
        AmbiguousSelection, BudgetRejected, ByteOnlyActivity, ChangingContent, CleanupFailure,
        ClockDegraded, ClockReset, ComponentDiscontinuity, DecodeOrSchemaFailure,
        NativeDescriptorDrift, NoData, RecordingFailure, Recovering, ReplayValidated, SinkFailure,
        Stalled, StaticContent, ValidSilence,
    };

    let kinds = [
        AmbiguousSelection,
        NativeDescriptorDrift,
        ComponentDiscontinuity,
        ClockReset,
        ClockDegraded,
        ValidSilence,
        NoData,
        Stalled,
        ByteOnlyActivity,
        ChangingContent,
        StaticContent,
        DecodeOrSchemaFailure,
        SinkFailure,
        RecordingFailure,
        BudgetRejected,
        Recovering,
        ReplayValidated,
        CleanupFailure,
    ];
    kinds
        .into_iter()
        .map(|kind| {
            let mut descriptor = base_stream_descriptor();
            let (expected_stage, expected_condition) =
                configure_stream_scenario(kind, &mut descriptor);
            DatastreamScenario {
                schema: "rusty.fleet.datastream_scenario.v1".to_owned(),
                kind,
                expected_stage,
                expected_condition,
                descriptor,
            }
        })
        .collect()
}

fn base_stream_descriptor() -> StreamDescriptor {
    serde_json::from_str(include_str!(
        "../../../fixtures/contracts/stream-descriptor.valid.json"
    ))
    .expect("repository stream fixture must deserialize")
}

fn configure_stream_scenario(
    kind: DatastreamScenarioKind,
    descriptor: &mut StreamDescriptor,
) -> (ProgressStage, ConditionState) {
    use DatastreamScenarioKind::{
        AmbiguousSelection, BudgetRejected, ByteOnlyActivity, ChangingContent, CleanupFailure,
        ClockDegraded, ClockReset, ComponentDiscontinuity, DecodeOrSchemaFailure,
        NativeDescriptorDrift, NoData, RecordingFailure, Recovering, ReplayValidated, SinkFailure,
        Stalled, StaticContent, ValidSilence,
    };

    match kind {
        AmbiguousSelection => {
            descriptor.selection.method = SelectionMethod::UnresolvedAmbiguous;
            descriptor.selection.candidate_count = 2;
            descriptor.selection.candidate_descriptor_digests.push(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
            );
            descriptor.selection.chosen_native_instance = None;
            insert_stage(
                descriptor,
                ProgressStage::SourceReceipt,
                ConditionState::Unavailable,
                "ambiguous_selection",
            );
            (ProgressStage::SourceReceipt, ConditionState::Unavailable)
        }
        NativeDescriptorDrift => {
            descriptor.native_descriptor.digest_sha256 =
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned();
            descriptor.epochs.source.predecessor = Some(descriptor.epochs.source.id.clone());
            descriptor.epochs.source.id = "source-2".to_owned();
            descriptor.epochs.source.reason = "native_descriptor_drift".to_owned();
            descriptor.epochs.source.continuity = EpochContinuity::Discontinuous;
            insert_stage(
                descriptor,
                ProgressStage::SourceReceipt,
                ConditionState::Degraded,
                "native_descriptor_drift",
            );
            (ProgressStage::SourceReceipt, ConditionState::Degraded)
        }
        ComponentDiscontinuity => {
            if let Some(route) = &mut descriptor.epochs.route {
                route.predecessor = Some(route.id.clone());
                route.id = "route-2".to_owned();
                route.reason = "route_replaced".to_owned();
                route.continuity = EpochContinuity::Discontinuous;
            }
            insert_stage(
                descriptor,
                ProgressStage::Route,
                ConditionState::Degraded,
                "component_discontinuity",
            );
            (ProgressStage::Route, ConditionState::Degraded)
        }
        ClockReset => {
            descriptor.timing.clock_reset_count = 1;
            insert_stage(
                descriptor,
                ProgressStage::SourceReceipt,
                ConditionState::Degraded,
                "clock_reset",
            );
            (ProgressStage::SourceReceipt, ConditionState::Degraded)
        }
        ClockDegraded => {
            descriptor.timing.transforms[0].uncertainty_ms = 8.0;
            insert_stage(
                descriptor,
                ProgressStage::SourceReceipt,
                ConditionState::Degraded,
                "clock_uncertainty_exceeds_budget",
            );
            (ProgressStage::SourceReceipt, ConditionState::Degraded)
        }
        ValidSilence => {
            descriptor.cadence.mode = fleet_contracts::CadenceMode::EventDriven;
            descriptor.cadence.nominal_rate_hz = None;
            descriptor.cadence.accepted_rate_min_hz = None;
            descriptor.cadence.accepted_rate_max_hz = None;
            descriptor.cadence.no_data_deadline_ms = None;
            descriptor.cadence.heartbeat = None;
            descriptor.progress.content_progress = ContentProgressPolicy::NotApplicable;
            insert_stage(
                descriptor,
                ProgressStage::Route,
                ConditionState::Current,
                "valid_event_silence",
            );
            (ProgressStage::Route, ConditionState::Current)
        }
        NoData => {
            insert_stage(
                descriptor,
                ProgressStage::CompletePayload,
                ConditionState::Failed,
                "no_data_deadline_exceeded",
            );
            (ProgressStage::CompletePayload, ConditionState::Failed)
        }
        Stalled => {
            insert_stage(
                descriptor,
                ProgressStage::CompletePayload,
                ConditionState::Degraded,
                "payload_stalled",
            );
            (ProgressStage::CompletePayload, ConditionState::Degraded)
        }
        ByteOnlyActivity => {
            insert_stage(
                descriptor,
                ProgressStage::Bytes,
                ConditionState::Current,
                "bytes_advancing",
            );
            insert_stage(
                descriptor,
                ProgressStage::CompletePayload,
                ConditionState::Failed,
                "byte_only_activity",
            );
            (ProgressStage::CompletePayload, ConditionState::Failed)
        }
        ChangingContent => {
            descriptor.progress.content_progress = ContentProgressPolicy::ChangingIdentity;
            insert_stage(
                descriptor,
                ProgressStage::Sink,
                ConditionState::Current,
                "content_identity_advancing",
            );
            (ProgressStage::Sink, ConditionState::Current)
        }
        StaticContent => {
            descriptor.progress.content_progress = ContentProgressPolicy::ExpectedStatic;
            insert_stage(
                descriptor,
                ProgressStage::Sink,
                ConditionState::Current,
                "expected_static_content",
            );
            (ProgressStage::Sink, ConditionState::Current)
        }
        DecodeOrSchemaFailure => {
            insert_stage(
                descriptor,
                ProgressStage::DecodeOrSchema,
                ConditionState::Failed,
                "schema_validation_failed",
            );
            (ProgressStage::DecodeOrSchema, ConditionState::Failed)
        }
        SinkFailure => {
            insert_stage(
                descriptor,
                ProgressStage::Sink,
                ConditionState::Failed,
                "sink_apply_failed",
            );
            (ProgressStage::Sink, ConditionState::Failed)
        }
        RecordingFailure => {
            add_recording(descriptor, RecordingArtifactState::Failed, "write_failed");
            insert_stage(
                descriptor,
                ProgressStage::Recording,
                ConditionState::Failed,
                "recording_write_failed",
            );
            (ProgressStage::Recording, ConditionState::Failed)
        }
        BudgetRejected => {
            insert_stage(
                descriptor,
                ProgressStage::Admission,
                ConditionState::Failed,
                "budget_rejected",
            );
            (ProgressStage::Admission, ConditionState::Failed)
        }
        Recovering => {
            insert_stage(
                descriptor,
                ProgressStage::Route,
                ConditionState::InProgress,
                "bounded_recovery_attempt_1",
            );
            (ProgressStage::Route, ConditionState::InProgress)
        }
        ReplayValidated => {
            add_recording(
                descriptor,
                RecordingArtifactState::Complete,
                "xdf_round_trip_passed",
            );
            insert_stage(
                descriptor,
                ProgressStage::Recording,
                ConditionState::Current,
                "replay_validated",
            );
            (ProgressStage::Recording, ConditionState::Current)
        }
        CleanupFailure => {
            insert_stage(
                descriptor,
                ProgressStage::Cleanup,
                ConditionState::Failed,
                "cleanup_residue",
            );
            (ProgressStage::Cleanup, ConditionState::Failed)
        }
    }
}

fn insert_stage(
    descriptor: &mut StreamDescriptor,
    stage: ProgressStage,
    state: ConditionState,
    reason: &str,
) {
    descriptor.progress.stages.insert(
        stage,
        ProgressStageEvidence {
            applicability: ProgressApplicability::Required,
            deadline_ms: Some(2_000),
            state: Some(state),
            observed_revision: Some(descriptor.accepted_authority_revision),
            last_progress_ms: Some(BASE_TIME_MS),
            reason: reason.to_owned(),
        },
    );
}

fn add_recording(
    descriptor: &mut StreamDescriptor,
    state: RecordingArtifactState,
    replay_validation: &str,
) {
    descriptor.experiment_run = Some(ExperimentRun {
        run_id: "synthetic-run-1".to_owned(),
        protocol_id: "synthetic-protocol".to_owned(),
        protocol_version: "1".to_owned(),
        participant_reference: Some("synthetic-participant".to_owned()),
        required_stream_rules: vec!["EEG".to_owned()],
        optional_stream_rules: Vec::new(),
        marker_schema: None,
        selection_snapshot_id: "synthetic-selection-1".to_owned(),
        started_at_ms: BASE_TIME_MS,
        recording_policy_revision: 1,
        approved_deviations: Vec::new(),
    });
    descriptor.recording = Some(RecordingArtifact {
        artifact_id: "synthetic-artifact-1".to_owned(),
        format: "xdf".to_owned(),
        state,
        bytes_written: 4_096,
        last_write_ms: Some(BASE_TIME_MS + 1_000),
        native_metadata_digests: vec![descriptor.native_descriptor.digest_sha256.clone()],
        clock_history_present: true,
        checksum_sha256: (state == RecordingArtifactState::Complete)
            .then(|| "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_owned()),
        encrypted_at_rest: true,
        retention_until_ms: Some(BASE_TIME_MS + 86_400_000),
        cleanup_receipt_id: None,
        replay_validation: Some(replay_validation.to_owned()),
    });
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
            let mut restart = first.clone();
            restart.source_epoch = "agent-epoch-2".to_owned();
            restart.source_revision = 1;
            mutations.push(ScenarioMutation {
                at_ms: BASE_TIME_MS + 8_500,
                kind: ScenarioMutationKind::AgentRestart,
                observation: restart,
            });
            let mut old_epoch_replay = first.clone();
            old_epoch_replay.source_revision = 4;
            mutations.push(ScenarioMutation {
                at_ms: BASE_TIME_MS + 8_501,
                kind: ScenarioMutationKind::Replay,
                observation: old_epoch_replay,
            });
            let mut reenrollment = first.clone();
            reenrollment.identity.identity_revision = 2;
            reenrollment.source_epoch = "agent-epoch-3".to_owned();
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

#[must_use]
pub fn mixed_freshness_fixture(device_count: usize) -> MixedFreshnessFixture {
    let mut observations = ScenarioBuilder::new(device_count).build().initial;
    for (index, observation) in observations.iter_mut().enumerate() {
        let age_ms = match index % 4 {
            0 | 3 => 30_000,
            1 => 120_000,
            _ => 420_000,
        };
        let target_received_time_ms = MIXED_FRESHNESS_TIME_MS - age_ms;
        let offset_ms = target_received_time_ms - observation.received_time_ms;
        shift_observation_time(observation, offset_ms);

        if index % 8 == 3
            && let Some(control) = observation
                .capabilities
                .capabilities
                .get_mut("participating_app_control")
        {
            control.authorization = AuthorizationState::Unauthorized;
            control.reason = "synthetic_grant_expired".to_owned();
        }
    }

    MixedFreshnessFixture {
        schema: "rusty.fleet.mixed_freshness_fixture.v1".to_owned(),
        now_ms: MIXED_FRESHNESS_TIME_MS,
        observations,
    }
}

#[must_use]
pub fn m1_lifecycle_scenario() -> M1LifecycleScenario {
    let seed = 0x4d31_4c49_4645_4359;
    let initial = ScenarioBuilder::new(4).with_seed(seed).build().initial;
    let mut steps = Vec::new();

    let mut sleeping = advance_observation(&initial[0], "agent-epoch-1", 2, BASE_TIME_MS + 60_000);
    if let Some(agent) = sleeping.agent.as_mut() {
        agent.lifecycle = ApplicationLifecycle::Stopped;
        agent.foreground_state = ForegroundState::Unknown;
    }
    set_freshness(
        &mut sleeping,
        ConditionState::Disconnected,
        "device_sleeping",
    );
    steps.push(lifecycle_step(
        M1LifecycleStepKind::SleepCheckIn,
        BASE_TIME_MS + 60_000,
        Some(sleeping.clone()),
    ));

    let mut keepalives = Vec::new();
    for observation in initial.iter().skip(1) {
        let keepalive =
            advance_observation(observation, "agent-epoch-1", 2, BASE_TIME_MS + 170_000);
        steps.push(lifecycle_step(
            M1LifecycleStepKind::KeepAlive,
            BASE_TIME_MS + 170_000,
            Some(keepalive.clone()),
        ));
        keepalives.push(keepalive);
    }

    steps.push(M1LifecycleStep {
        at_ms: BASE_TIME_MS + 180_000,
        kind: M1LifecycleStepKind::StaleWhileSleeping,
        device_id: initial[0].identity.device_id.clone(),
        observation: None,
    });

    let mut wake = advance_observation(&sleeping, "agent-epoch-1", 3, BASE_TIME_MS + 180_001);
    if let Some(agent) = wake.agent.as_mut() {
        agent.lifecycle = ApplicationLifecycle::Background;
        agent.foreground_state = ForegroundState::Background;
    }
    set_freshness(&mut wake, ConditionState::Current, "local");
    steps.push(lifecycle_step(
        M1LifecycleStepKind::WakeCheckIn,
        BASE_TIME_MS + 180_001,
        Some(wake.clone()),
    ));

    let mut route_loss =
        advance_observation(&keepalives[0], "agent-epoch-1", 3, BASE_TIME_MS + 190_000);
    set_freshness(&mut route_loss, ConditionState::Disconnected, "route_lost");
    steps.push(lifecycle_step(
        M1LifecycleStepKind::RouteLoss,
        BASE_TIME_MS + 190_000,
        Some(route_loss.clone()),
    ));

    steps.push(lifecycle_step(
        M1LifecycleStepKind::DuplicateCheckIn,
        BASE_TIME_MS + 190_001,
        Some(keepalives[1].clone()),
    ));

    let stale_revision =
        advance_observation(&keepalives[2], "agent-epoch-1", 1, BASE_TIME_MS + 190_002);
    steps.push(lifecycle_step(
        M1LifecycleStepKind::StaleRevision,
        BASE_TIME_MS + 190_002,
        Some(stale_revision),
    ));

    let mut upgrade = advance_observation(&wake, "agent-epoch-2", 1, BASE_TIME_MS + 200_000);
    upgrade
        .identity
        .tags
        .insert("agent_build".to_owned(), "synthetic-2".to_owned());
    steps.push(lifecycle_step(
        M1LifecycleStepKind::AgentUpgrade,
        BASE_TIME_MS + 200_000,
        Some(upgrade),
    ));

    let old_epoch_replay = advance_observation(&wake, "agent-epoch-1", 4, BASE_TIME_MS + 200_001);
    steps.push(lifecycle_step(
        M1LifecycleStepKind::OldEpochReplay,
        BASE_TIME_MS + 200_001,
        Some(old_epoch_replay),
    ));

    let mut route_recovery =
        advance_observation(&route_loss, "agent-epoch-1", 4, BASE_TIME_MS + 210_000);
    set_freshness(&mut route_recovery, ConditionState::Current, "local");
    steps.push(lifecycle_step(
        M1LifecycleStepKind::RouteRecovery,
        BASE_TIME_MS + 210_000,
        Some(route_recovery),
    ));

    M1LifecycleScenario {
        schema: "rusty.fleet.m1_lifecycle_scenario.v1".to_owned(),
        seed,
        initial,
        steps,
        final_time_ms: BASE_TIME_MS + 210_001,
    }
}

fn lifecycle_step(
    kind: M1LifecycleStepKind,
    at_ms: i64,
    observation: Option<DeviceObservation>,
) -> M1LifecycleStep {
    let device_id = observation
        .as_ref()
        .map_or_else(String::new, |value| value.identity.device_id.clone());
    M1LifecycleStep {
        at_ms,
        kind,
        device_id,
        observation,
    }
}

fn advance_observation(
    observation: &DeviceObservation,
    source_epoch: &str,
    source_revision: u64,
    at_ms: i64,
) -> DeviceObservation {
    let mut next = observation.clone();
    let offset_ms = at_ms - next.received_time_ms;
    shift_observation_time(&mut next, offset_ms);
    next.source_epoch = source_epoch.to_owned();
    next.source_revision = source_revision;
    for condition in &mut next.conditions {
        condition.accepted_revision = source_revision;
        condition.source.authority_revision = source_revision;
    }
    for capability in next.capabilities.capabilities.values_mut() {
        capability.evidence_revision = source_revision;
    }
    next
}

fn set_freshness(observation: &mut DeviceObservation, state: ConditionState, reason: &str) {
    if let Some(condition) = observation
        .conditions
        .iter_mut()
        .find(|condition| condition.family == ConditionFamily::Freshness)
    {
        condition.state = state;
        condition.reason = reason.to_owned();
        condition.message = reason.replace('_', " ");
    }
}

fn shift_observation_time(observation: &mut DeviceObservation, offset_ms: i64) {
    observation.source_time_ms += offset_ms;
    observation.received_time_ms += offset_ms;

    for condition in &mut observation.conditions {
        condition.source_time_ms += offset_ms;
        condition.received_time_ms += offset_ms;
        condition.fresh_until_ms += offset_ms;
    }
    for capability in observation.capabilities.capabilities.values_mut() {
        capability.observed_at_ms += offset_ms;
        capability.fresh_until_ms += offset_ms;
    }
    for application in [observation.agent.as_mut(), observation.application.as_mut()]
        .into_iter()
        .flatten()
    {
        application.provenance.observed_at_ms += offset_ms;
        application.provenance.fresh_until_ms += offset_ms;
    }
    if let Some(power) = observation.power.as_mut() {
        power.provenance.observed_at_ms += offset_ms;
        power.provenance.fresh_until_ms += offset_ms;
    }
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
        source_epoch: "agent-epoch-1".to_owned(),
        source_revision,
        source_time_ms: observed_at,
        received_time_ms: observed_at,
        battery_percent: Some(battery_percent),
        charging: Some(index.is_multiple_of(5)),
        foreground_app: Some("org.example.synthetic.kiosk".to_owned()),
        agent: Some(ApplicationObservation {
            package_name: Some("io.github.mesmerprism.rustyquest.fleetagent".to_owned()),
            lifecycle: ApplicationLifecycle::Background,
            foreground_state: ForegroundState::Background,
            foreground_authority: ForegroundAuthority::SelfReport,
            provenance: FactProvenance {
                owner: "rusty-quest".to_owned(),
                adapter_id: "synthetic-quest-agent".to_owned(),
                observed_at_ms: observed_at,
                fresh_until_ms: observed_at + 60_000,
            },
        }),
        power: Some(PowerObservation {
            battery_percent,
            charging: index.is_multiple_of(5),
            provenance: FactProvenance {
                owner: "rusty-quest".to_owned(),
                adapter_id: "synthetic-quest-agent".to_owned(),
                observed_at_ms: observed_at,
                fresh_until_ms: observed_at + 60_000,
            },
        }),
        application: Some(ApplicationObservation {
            package_name: Some("org.example.synthetic.kiosk".to_owned()),
            lifecycle: ApplicationLifecycle::Foreground,
            foreground_state: ForegroundState::Foreground,
            foreground_authority: ForegroundAuthority::ParticipatingApp,
            provenance: FactProvenance {
                owner: "org.example.synthetic.kiosk".to_owned(),
                adapter_id: "synthetic-participating-app".to_owned(),
                observed_at_ms: observed_at,
                fresh_until_ms: observed_at + 60_000,
            },
        }),
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
    use std::collections::BTreeSet;

    use fleet_contracts::{AuthorizationState, ValidateContract};

    use super::{
        M1LifecycleStepKind, MIXED_FRESHNESS_TIME_MS, ScenarioBuilder, ScenarioMutationKind,
        datastream_scenarios, m1_lifecycle_scenario, mixed_freshness_fixture,
        supported_scale_fixtures,
    };

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
    fn mixed_freshness_fixture_is_deterministic_valid_and_bounded() {
        let first = mixed_freshness_fixture(1_000);
        let second = mixed_freshness_fixture(1_000);
        assert_eq!(first, second);
        assert_eq!(first.now_ms, MIXED_FRESHNESS_TIME_MS);
        assert_eq!(first.observations.len(), 1_000);
        assert!(
            first
                .observations
                .iter()
                .all(|observation| observation.validate().is_ok())
        );

        let ages = first
            .observations
            .iter()
            .map(|observation| first.now_ms - observation.received_time_ms)
            .collect::<Vec<_>>();
        assert_eq!(ages.iter().filter(|age| **age == 30_000).count(), 500);
        assert_eq!(ages.iter().filter(|age| **age == 120_000).count(), 250);
        assert_eq!(ages.iter().filter(|age| **age == 420_000).count(), 250);
        assert_eq!(
            first
                .observations
                .iter()
                .filter(|observation| {
                    observation
                        .capabilities
                        .get("participating_app_control")
                        .is_some_and(|capability| {
                            capability.authorization == AuthorizationState::Unauthorized
                        })
                })
                .count(),
            125
        );
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

    #[test]
    fn m1_lifecycle_fixture_is_deterministic_valid_and_complete() {
        let first = m1_lifecycle_scenario();
        let second = m1_lifecycle_scenario();
        assert_eq!(first, second);
        assert_eq!(first.initial.len(), 4);
        assert!(
            first
                .initial
                .iter()
                .all(|observation| observation.validate().is_ok())
        );

        let kinds = first
            .steps
            .iter()
            .map(|step| step.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(kinds.len(), 10);
        assert_eq!(
            first
                .steps
                .iter()
                .filter(|step| step.kind == M1LifecycleStepKind::KeepAlive)
                .count(),
            3
        );
        for step in first.steps {
            if step.kind == M1LifecycleStepKind::StaleWhileSleeping {
                assert!(step.observation.is_none());
            } else {
                assert!(
                    step.observation
                        .is_some_and(|observation| observation.validate().is_ok()),
                    "{:?} observation must validate",
                    step.kind
                );
            }
        }
    }

    #[test]
    fn datastream_matrix_is_complete_valid_and_truthful() {
        let scenarios = datastream_scenarios();
        assert_eq!(scenarios.len(), 18);
        let kinds = scenarios
            .iter()
            .map(|scenario| scenario.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(kinds.len(), scenarios.len());
        for scenario in scenarios {
            assert!(
                scenario.descriptor.validate().is_ok(),
                "{:?} descriptor must validate",
                scenario.kind
            );
            assert_eq!(
                scenario.descriptor.progress.strongest_required_condition(),
                Some((scenario.expected_stage, scenario.expected_condition)),
                "{:?} strongest condition must match the fixture expectation",
                scenario.kind
            );
        }
    }
}
