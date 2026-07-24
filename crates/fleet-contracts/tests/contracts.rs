// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use fleet_contracts::{
    AdmissionBudget, CadenceMode, CadencePolicy, CommandLifecycle, ComponentEpoch, ComponentEpochs,
    ConditionState, ContentProgressPolicy, EdgeQueuePolicy, EpochContinuity, ExperimentRun,
    FleetQuery, NativeDescriptor, NavigationRestoration, OperationLedger, OperationTargetResult,
    OverflowPolicy, ProgressApplicability, ProgressProfile, ProgressStage, ProgressStageEvidence,
    QueryExpression, QueueLimits, RecordingArtifact, RecordingArtifactState, SavedView,
    SavedViewCollection, SavedViewMutationReceipt, SavedViewMutationRequest, SelectionMethod,
    Sensitivity, SourceSelection, StreamDescriptor, StreamPlane, StreamSemantic, TargetEligibility,
    TargetSnapshot, TimingCorrelation, TimingDomain, TimingTransform, ValidateContract,
};
use serde_json::json;

const DIGEST_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn valid_stream() -> StreamDescriptor {
    let mut stages = BTreeMap::new();
    stages.insert(
        ProgressStage::SourceReceipt,
        ProgressStageEvidence {
            applicability: ProgressApplicability::Required,
            deadline_ms: Some(2_000),
            state: Some(ConditionState::Current),
            observed_revision: Some(7),
            last_progress_ms: Some(2_000_000_000_000),
            reason: "owner_receipt".to_owned(),
        },
    );
    stages.insert(
        ProgressStage::Recording,
        ProgressStageEvidence {
            applicability: ProgressApplicability::NotApplicable,
            deadline_ms: None,
            state: None,
            observed_revision: None,
            last_progress_ms: None,
            reason: "not_requested".to_owned(),
        },
    );
    StreamDescriptor {
        schema: "rusty.fleet.stream_descriptor.v1".to_owned(),
        logical_stream_id: "device-1/lsl/eeg".to_owned(),
        semantic: StreamSemantic {
            family: "eeg".to_owned(),
            class: "scalar_sample".to_owned(),
            schema_profile: "eeg.channels.v1".to_owned(),
            plane: StreamPlane::Observation,
            sensitivity: Sensitivity::Restricted,
        },
        native_descriptor: NativeDescriptor {
            kind: "lsl.stream_info+xml".to_owned(),
            adapter_version: "rusty-lsl-fixture/1".to_owned(),
            native_protocol_version: "1.16".to_owned(),
            digest_sha256: DIGEST_A.to_owned(),
            captured_at_ms: 2_000_000_000_000,
            document: Some(json!({"name": "Synthetic EEG", "type": "EEG"})),
            reference: None,
        },
        selection: SourceSelection {
            query_language: "lsl_property".to_owned(),
            query: "source_id=synthetic-eeg-1".to_owned(),
            expected_cardinality: 1,
            candidate_count: 1,
            candidate_descriptor_digests: vec![DIGEST_A.to_owned()],
            chosen_native_instance: Some("uid-1".to_owned()),
            method: SelectionMethod::SingleCandidate,
            valid_until_ms: 2_000_000_010_000,
            override_actor: None,
            override_reason: None,
        },
        epochs: ComponentEpochs {
            source: ComponentEpoch {
                id: "source-1".to_owned(),
                predecessor: None,
                reason: "initial_outlet".to_owned(),
                continuity: EpochContinuity::Unknown,
                native_instance: Some("uid-1".to_owned()),
            },
            route: Some(ComponentEpoch {
                id: "route-1".to_owned(),
                predecessor: None,
                reason: "initial_connection".to_owned(),
                continuity: EpochContinuity::SourceContinuityProven,
                native_instance: None,
            }),
            processing: None,
            sink: None,
            path_generation: "path-1".to_owned(),
        },
        accepted_authority_revision: 7,
        timing: TimingCorrelation {
            domains: vec![
                TimingDomain {
                    domain_id: "lsl_local_clock".to_owned(),
                    domain_kind: "source_monotonic".to_owned(),
                    units: "seconds".to_owned(),
                    time_base_numerator: 1,
                    time_base_denominator: 1_000_000_000,
                    raw_timestamp_preserved: true,
                },
                TimingDomain {
                    domain_id: "hub_monotonic".to_owned(),
                    domain_kind: "receive_monotonic".to_owned(),
                    units: "milliseconds".to_owned(),
                    time_base_numerator: 1,
                    time_base_denominator: 1_000,
                    raw_timestamp_preserved: true,
                },
            ],
            transforms: vec![TimingTransform {
                transform_id: "lsl-offset-7".to_owned(),
                from_domain: "lsl_local_clock".to_owned(),
                to_domain: "hub_monotonic".to_owned(),
                method: "lsl_offset_observation".to_owned(),
                offset_ms: -2.4,
                uncertainty_ms: 0.4,
                valid_from_ms: 2_000_000_000_000,
                valid_to_ms: None,
                postprocessing: vec!["clocksync".to_owned()],
            }],
            clock_reset_count: 0,
            fixed_latency_ms: Some(18.2),
            fixed_latency_uncertainty_ms: Some(1.1),
            calibration_reference: Some("synthetic-calibration-v1".to_owned()),
        },
        cadence: CadencePolicy {
            mode: CadenceMode::Regular,
            nominal_rate_hz: Some(1_000.0),
            accepted_rate_min_hz: Some(950.0),
            accepted_rate_max_hz: Some(1_050.0),
            measurement_window_ms: 10_000,
            gap_tolerance_ms: Some(50),
            no_data_deadline_ms: Some(500),
            heartbeat: None,
            sequence_semantics: "timestamp_only".to_owned(),
        },
        progress: ProgressProfile {
            profile_id: "lsl_regular_sample".to_owned(),
            content_progress: ContentProgressPolicy::Timestamp,
            stages,
        },
        queues: vec![EdgeQueuePolicy {
            edge_id: "inlet_to_observer".to_owned(),
            producer: "rusty_lsl".to_owned(),
            consumer: "fleet_hub".to_owned(),
            limits: QueueLimits {
                items: 2_000,
                bytes: 8_388_608,
                duration_ms: 2_000,
            },
            overflow: OverflowPolicy::Reject,
            producer_may_block: false,
            retain_codec_configuration: false,
            restart_requires_keyframe: false,
            slow_consumer_action: "degrade_observation".to_owned(),
        }],
        budget: AdmissionBudget {
            bandwidth_bytes_per_second: 1_000_000,
            samples_or_frames_per_second: 1_000.0,
            decode_slots: 0,
            queue_bytes: 8_388_608,
            queue_duration_ms: 2_000,
            disk_bytes: 0,
            recorder_slots: 0,
            maximum_clock_uncertainty_ms: Some(2.0),
            priority_class: "scientific_observation".to_owned(),
        },
        experiment_run: None,
        recording: None,
        extensions: BTreeMap::new(),
    }
}

fn all_query() -> FleetQuery {
    FleetQuery {
        schema: "rusty.fleet.query.v1".to_owned(),
        query_id: "all".to_owned(),
        expression: None,
        sort: Vec::new(),
        offset: 0,
        limit: 250,
    }
}

#[test]
fn valid_stream_preserves_native_and_normalized_contracts() {
    let stream = valid_stream();
    assert!(stream.validate().is_ok());
    let round_trip: StreamDescriptor =
        serde_json::from_value(serde_json::to_value(&stream).expect("serialize"))
            .expect("deserialize");
    assert_eq!(round_trip, stream);
}

#[test]
fn ambiguous_selection_cannot_silently_choose_a_candidate() {
    let mut stream = valid_stream();
    stream.selection.method = SelectionMethod::UnresolvedAmbiguous;
    stream.selection.candidate_count = 2;
    stream.selection.candidate_descriptor_digests = vec![DIGEST_A.to_owned(), DIGEST_B.to_owned()];
    stream.selection.chosen_native_instance = None;
    assert!(stream.validate().is_ok());

    stream.selection.chosen_native_instance = Some("first-returned".to_owned());
    let failures = stream.validate().expect_err("silent selection must fail");
    assert!(
        failures
            .iter()
            .any(|failure| failure.code == "invalid_selection_result")
    );
}

#[test]
fn timing_queue_and_recording_damage_fail_closed() {
    let mut stream = valid_stream();
    stream.timing.transforms[0].to_domain = "missing-domain".to_owned();
    stream.queues[0].overflow = OverflowPolicy::BlockProducer;
    stream.queues[0].producer_may_block = false;
    stream.recording = Some(RecordingArtifact {
        artifact_id: "artifact-1".to_owned(),
        format: "xdf".to_owned(),
        state: RecordingArtifactState::Writing,
        bytes_written: 1_024,
        last_write_ms: Some(2_000_000_000_100),
        native_metadata_digests: vec![DIGEST_A.to_owned()],
        clock_history_present: true,
        checksum_sha256: None,
        encrypted_at_rest: true,
        retention_until_ms: Some(2_000_086_400_000),
        cleanup_receipt_id: None,
        replay_validation: None,
    });
    let codes = stream
        .validate()
        .expect_err("damaged stream must fail")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"invalid_time_transform".to_owned()));
    assert!(codes.contains(&"unsafe_block_policy".to_owned()));
    assert!(codes.contains(&"recording_without_run".to_owned()));
}

#[test]
fn recording_with_provenance_is_valid() {
    let mut stream = valid_stream();
    stream.experiment_run = Some(ExperimentRun {
        run_id: "run-1".to_owned(),
        protocol_id: "balance-vr".to_owned(),
        protocol_version: "3.2".to_owned(),
        participant_reference: Some("participant-pseudonym".to_owned()),
        required_stream_rules: vec!["EEG".to_owned()],
        optional_stream_rules: vec!["head_pose".to_owned()],
        marker_schema: Some("experiment-markers.v2".to_owned()),
        selection_snapshot_id: "stream-selection-1".to_owned(),
        started_at_ms: 2_000_000_000_000,
        recording_policy_revision: 3,
        approved_deviations: Vec::new(),
    });
    stream.recording = Some(RecordingArtifact {
        artifact_id: "artifact-1".to_owned(),
        format: "xdf".to_owned(),
        state: RecordingArtifactState::Complete,
        bytes_written: 1_024,
        last_write_ms: Some(2_000_000_001_000),
        native_metadata_digests: vec![DIGEST_A.to_owned()],
        clock_history_present: true,
        checksum_sha256: Some(DIGEST_B.to_owned()),
        encrypted_at_rest: true,
        retention_until_ms: Some(2_000_086_400_000),
        cleanup_receipt_id: None,
        replay_validation: Some("xdf_round_trip_passed".to_owned()),
    });
    assert!(stream.validate().is_ok());
}

#[test]
fn command_lifecycle_does_not_confuse_dispatch_with_application() {
    assert!(CommandLifecycle::Accepted.can_transition_to(CommandLifecycle::Dispatched));
    assert!(CommandLifecycle::Dispatched.can_transition_to(CommandLifecycle::Applied));
    assert!(!CommandLifecycle::Accepted.can_transition_to(CommandLifecycle::Applied));
    assert!(!CommandLifecycle::Applied.is_terminal());
    assert!(CommandLifecycle::Cleaned.is_terminal());
}

#[test]
fn saved_view_and_operation_ledger_preserve_scope_and_per_target_results() {
    let view = SavedView {
        schema: "rusty.fleet.saved_view.v1".to_owned(),
        view_id: "needs-attention".to_owned(),
        name: "Needs attention".to_owned(),
        query: all_query(),
        columns: vec![
            "device".to_owned(),
            "age".to_owned(),
            "attention".to_owned(),
        ],
        density: "standard".to_owned(),
        grouping: None,
        restoration: NavigationRestoration {
            selected_device_id: Some("sim-00001".to_owned()),
            inspector_tab: Some("overview".to_owned()),
            scroll_anchor_device_id: Some("sim-00001".to_owned()),
            focused_region: Some("grid".to_owned()),
            collapsed_groups: Vec::new(),
        },
        schema_version: 1,
    };
    assert!(view.validate().is_ok());
    let mut invalid_id = view.clone();
    invalid_id.view_id = "View/unsafe".to_owned();
    assert!(
        invalid_id
            .validate()
            .expect_err("unsafe saved-view ID")
            .iter()
            .any(|failure| failure.code == "invalid_saved_view_id")
    );
    let collection = SavedViewCollection {
        schema: "rusty.fleet.saved_view_collection.v1".to_owned(),
        revision: 1,
        views: vec![view.clone()],
    };
    assert!(collection.validate().is_ok());
    let request = SavedViewMutationRequest {
        schema: "rusty.fleet.saved_view_mutation_request.v1".to_owned(),
        expected_revision: collection.revision,
        view: view.clone(),
    };
    assert!(request.validate().is_ok());
    let receipt = SavedViewMutationReceipt {
        schema: "rusty.fleet.saved_view_mutation_receipt.v1".to_owned(),
        view_id: view.view_id.clone(),
        previous_revision: 1,
        current_revision: 2,
        changed: true,
        deleted: false,
        view: Some(view),
    };
    assert!(receipt.validate().is_ok());

    let snapshot = TargetSnapshot {
        snapshot_id: "snapshot-1".to_owned(),
        created_at_ms: 2_000_000_000_000,
        expires_at_ms: 2_000_000_060_000,
        query: all_query(),
        result_revision: 44,
        identity_revisions: BTreeMap::from([
            ("sim-00001".to_owned(), 1),
            ("sim-00002".to_owned(), 1),
        ]),
        extensions: BTreeMap::new(),
    };
    let ledger = OperationLedger {
        schema: "rusty.fleet.operation_ledger.v1".to_owned(),
        operation_id: "operation-1".to_owned(),
        action_id: "participating_app.launch".to_owned(),
        created_at_ms: 2_000_000_000_000,
        target_snapshot: snapshot,
        lifecycle: CommandLifecycle::Dispatched,
        cleanup_required: true,
        targets: vec![
            OperationTargetResult {
                device_id: "sim-00001".to_owned(),
                identity_revision: 1,
                eligibility: TargetEligibility::Eligible,
                lifecycle: CommandLifecycle::Applied,
                reason_code: "owner_receipt".to_owned(),
                message: "applied by participating app".to_owned(),
                last_transition_ms: 2_000_000_001_000,
                receipt_id: Some("receipt-1".to_owned()),
                retry_eligible: false,
                cancel_eligible: false,
                extensions: BTreeMap::new(),
            },
            OperationTargetResult {
                device_id: "sim-00002".to_owned(),
                identity_revision: 1,
                eligibility: TargetEligibility::RefreshRequired,
                lifecycle: CommandLifecycle::Rejected,
                reason_code: "stale_capability".to_owned(),
                message: "refresh required before dispatch".to_owned(),
                last_transition_ms: 2_000_000_000_500,
                receipt_id: None,
                retry_eligible: true,
                cancel_eligible: false,
                extensions: BTreeMap::new(),
            },
        ],
        extensions: BTreeMap::new(),
    };
    assert!(ledger.validate().is_ok());
    assert_ne!(ledger.targets[0].lifecycle, ledger.targets[1].lifecycle);
}

#[test]
fn contract_limits_reject_amplification_shapes() {
    let mut query = all_query();
    let mut expression = QueryExpression::Predicate {
        field: fleet_contracts::QueryField::DisplayName,
        comparison: fleet_contracts::Comparison::Equals,
        value: Some(fleet_contracts::QueryValue::Text("device".to_owned())),
        qualifier: None,
    };
    for _ in 0..17 {
        expression = QueryExpression::Not {
            expression: Box::new(expression),
        };
    }
    query.expression = Some(expression);
    assert!(
        query
            .validate()
            .expect_err("deep query must fail")
            .iter()
            .any(|failure| failure.code == "query_shape_exceeded")
    );

    let mut stream = valid_stream();
    stream.native_descriptor.document = Some(json!({
        "oversized": "x".repeat(1024 * 1024 + 1)
    }));
    assert!(
        stream
            .validate()
            .expect_err("oversized native descriptor must fail")
            .iter()
            .any(|failure| failure.code == "native_descriptor_too_large")
    );
}
