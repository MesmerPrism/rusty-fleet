// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use fleet_contracts::{
    Q2Q_STEREO_EVIDENCE_SCHEMA, Q2Q_STEREO_PLAN_SCHEMA, Q2qStereoAdapter, Q2qStereoDirection,
    Q2qStereoEvidenceProjection, Q2qStereoHealth, Q2qStereoPhase, Q2qStereoPlan, Q2qStereoPlanStep,
    Q2qStereoTarget, Q2qStereoTargetSelection, ValidateContract,
};

const DIGEST_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn valid_plan() -> Q2qStereoPlan {
    let directions = vec![
        Q2qStereoDirection {
            direction_id: "a-to-b".to_owned(),
            source_target_id: "target.quest-a".to_owned(),
            sink_target_id: "target.quest-b".to_owned(),
            conservative_duration_seconds: 60,
        },
        Q2qStereoDirection {
            direction_id: "b-to-a".to_owned(),
            source_target_id: "target.quest-b".to_owned(),
            sink_target_id: "target.quest-a".to_owned(),
            conservative_duration_seconds: 60,
        },
    ];
    let adapters = vec![
        Q2qStereoAdapter::InfrastructureLan,
        Q2qStereoAdapter::WifiDirect,
        Q2qStereoAdapter::AuthenticatedTlsRelay,
    ];
    let phases = [
        Q2qStereoPhase::ReceiverStart,
        Q2qStereoPhase::ReceiverReady,
        Q2qStereoPhase::SenderStart,
        Q2qStereoPhase::Streaming,
        Q2qStereoPhase::StopRequested,
        Q2qStereoPhase::Stopped,
        Q2qStereoPhase::CleanupVerified,
    ];
    let mut ordinal = 0;
    let mut steps = Vec::new();
    for adapter in &adapters {
        for direction in &directions {
            for phase in phases {
                ordinal += 1;
                steps.push(Q2qStereoPlanStep {
                    ordinal,
                    adapter: *adapter,
                    direction_id: direction.direction_id.clone(),
                    phase,
                });
            }
        }
    }
    Q2qStereoPlan {
        schema: Q2Q_STEREO_PLAN_SCHEMA.to_owned(),
        run_id: "night-run".to_owned(),
        selection: Q2qStereoTargetSelection {
            selection_id: "selection-night-run".to_owned(),
            immutable: true,
            targets: vec![
                Q2qStereoTarget {
                    target_id: "target.quest-a".to_owned(),
                    identity_revision: 7,
                    selection_digest_sha256: DIGEST_A.to_owned(),
                },
                Q2qStereoTarget {
                    target_id: "target.quest-b".to_owned(),
                    identity_revision: 9,
                    selection_digest_sha256: DIGEST_B.to_owned(),
                },
            ],
        },
        adapter_order: adapters,
        directions,
        steps,
        evidence_root_reference: "evidence.q2q.night-run".to_owned(),
    }
}

#[test]
fn exact_two_target_receiver_first_ladder_passes() {
    let plan = valid_plan();
    assert!(plan.validate().is_ok());
    let encoded = serde_json::to_string(&plan).expect("serialize plan");
    assert!(!encoded.contains("serial"));
    assert!(!encoded.contains("endpoint"));
    assert!(!encoded.contains("token"));
    assert!(!encoded.contains("media_bytes"));
}

#[test]
fn sender_before_receiver_ready_and_target_substitution_fail_closed() {
    let mut plan = valid_plan();
    plan.steps[1].phase = Q2qStereoPhase::SenderStart;
    assert!(
        plan.validate()
            .expect_err("sender-first schedule must fail")
            .iter()
            .any(|failure| failure.code == "invalid_receiver_first_schedule")
    );

    let mut substituted = valid_plan();
    substituted.directions[0].sink_target_id = "target.quest-c".to_owned();
    assert!(
        substituted
            .validate()
            .expect_err("target substitution must fail")
            .iter()
            .any(|failure| failure.code == "invalid_direction")
    );
}

#[test]
fn evidence_projection_contains_references_not_media_or_secrets() {
    let projection = Q2qStereoEvidenceProjection {
        schema: Q2Q_STEREO_EVIDENCE_SCHEMA.to_owned(),
        run_id: "night-run".to_owned(),
        selection_id: "selection-night-run".to_owned(),
        adapter: Q2qStereoAdapter::InfrastructureLan,
        direction_id: "a-to-b".to_owned(),
        phase: Q2qStereoPhase::Streaming,
        health: Q2qStereoHealth::Current,
        stop_requested: false,
        observed_at_ms: 2_000_000_000_000,
        evidence_references: BTreeMap::from([
            ("source".to_owned(), "evidence.source.window-3".to_owned()),
            ("sink".to_owned(), "evidence.sink.window-3".to_owned()),
        ]),
    };
    assert!(projection.validate().is_ok());
    let encoded = serde_json::to_string(&projection).expect("serialize evidence");
    assert!(!encoded.contains("serial"));
    assert!(!encoded.contains("endpoint"));
    assert!(!encoded.contains("bearer"));

    let mut leaked = projection;
    leaked.evidence_references.insert(
        "transport".to_owned(),
        "http://private-endpoint:9443".to_owned(),
    );
    assert!(
        leaked
            .validate()
            .expect_err("endpoint-shaped evidence reference must fail")
            .iter()
            .any(|failure| failure.code == "invalid_public_reference")
    );
}
