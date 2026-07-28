// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet};

use fleet_contracts::{
    QUEST_AWAKE_ACTION_ID, QUEST_AWAKE_PREVIEW_REQUEST_SCHEMA, QuestAwakeAction,
    QuestAwakePreviewRequest,
};
use fleet_hub::{FleetHub, HubPolicy, ObservationDecision, QuestAwakePreviewPlan};
use fleet_simulator::{BASE_TIME_MS, ScenarioBuilder};

fn hub_and_target() -> (FleetHub, String, u64) {
    let scenario = ScenarioBuilder::new(4).build();
    let target = scenario.initial[0].identity.clone();
    let mut hub = FleetHub::new(HubPolicy::default());
    for observation in scenario.initial {
        assert!(matches!(
            hub.accept_observation(observation, BASE_TIME_MS),
            ObservationDecision::Accepted { .. }
        ));
    }
    (hub, target.device_id, target.identity_revision)
}

fn plan(
    device_id: &str,
    identity_revision: u64,
    action: QuestAwakeAction,
    provider_ready: bool,
) -> QuestAwakePreviewPlan {
    QuestAwakePreviewPlan {
        operation_id: format!("awake-operation-{action:?}").to_ascii_lowercase(),
        preview_id: format!("awake-preview-{action:?}").to_ascii_lowercase(),
        watchdog_generation: format!("awake-generation-{action:?}").to_ascii_lowercase(),
        request: QuestAwakePreviewRequest {
            schema: QUEST_AWAKE_PREVIEW_REQUEST_SCHEMA.to_owned(),
            action_id: QUEST_AWAKE_ACTION_ID.to_owned(),
            action,
            duration_ms: 28_800_000,
            watchdog_interval_ms: 5_000,
            targets: BTreeMap::from([(device_id.to_owned(), identity_revision)]),
        },
        created_at_ms: BASE_TIME_MS,
        expires_at_ms: BASE_TIME_MS + 60_000,
        provider_ready_devices: if provider_ready {
            BTreeSet::from([device_id.to_owned()])
        } else {
            BTreeSet::new()
        },
    }
}

#[test]
fn host_provider_is_absent_and_inert_until_explicitly_configured() {
    let (mut hub, device_id, identity_revision) = hub_and_target();
    let error = hub
        .preview_quest_awake(plan(
            &device_id,
            identity_revision,
            QuestAwakeAction::Status,
            false,
        ))
        .expect_err("absent provider must not produce an actionable operation");
    assert_eq!(error.code, "awake_operation_contract_invalid");
    assert!(hub.quest_awake_operations().is_empty());
}

#[test]
fn confirmation_and_invocation_preserve_exact_identity_action_and_policy() {
    let (mut hub, device_id, identity_revision) = hub_and_target();
    let preview = hub
        .preview_quest_awake(plan(
            &device_id,
            identity_revision,
            QuestAwakeAction::StopWatchdogs,
            true,
        ))
        .expect("preview");
    assert!(preview.preview.targets[0].eligible);
    hub.confirm_quest_awake(
        &preview.operation_id,
        &preview.preview.preview_id,
        BASE_TIME_MS + 1,
    )
    .expect("confirm");
    let prepared = hub
        .prepare_quest_awake_invocation(
            &preview.operation_id,
            &device_id,
            "awake-request-1".to_owned(),
            BASE_TIME_MS + 2,
        )
        .expect("prepare invocation");
    let invocation = prepared.targets[0].invocation.as_ref().expect("invocation");
    assert_eq!(invocation.device_id, device_id);
    assert_eq!(invocation.identity_revision, identity_revision);
    assert_eq!(invocation.action, QuestAwakeAction::StopWatchdogs);
    assert_eq!(invocation.duration_ms, 28_800_000);
    assert_eq!(invocation.watchdog_interval_ms, 5_000);
    assert_eq!(invocation.watchdog_generation, "no-known-device-watchdog");
}

#[test]
fn stop_binds_the_observed_device_watchdog_generation_per_target() {
    let (mut hub, device_id, identity_revision) = hub_and_target();
    let preview = hub
        .preview_quest_awake(plan(
            &device_id,
            identity_revision,
            QuestAwakeAction::StopWatchdogs,
            true,
        ))
        .expect("preview");
    hub.confirm_quest_awake(
        &preview.operation_id,
        &preview.preview.preview_id,
        BASE_TIME_MS + 1,
    )
    .expect("confirm");
    let prepared = hub
        .prepare_quest_awake_invocation_with_watchdog_generation(
            &preview.operation_id,
            &device_id,
            "awake-request-stop-observed".to_owned(),
            Some("observed-device-watchdog-generation".to_owned()),
            BASE_TIME_MS + 2,
        )
        .expect("prepare exact stop invocation");
    assert_eq!(
        prepared.targets[0]
            .invocation
            .as_ref()
            .expect("invocation")
            .watchdog_generation,
        "observed-device-watchdog-generation"
    );
}

#[test]
fn windows_watchdog_uses_a_durable_generation_without_turning_stop_into_restore() {
    let (mut hub, device_id, identity_revision) = hub_and_target();
    let preview = hub
        .preview_quest_awake(plan(
            &device_id,
            identity_revision,
            QuestAwakeAction::StartWindowsWatchdog,
            true,
        ))
        .expect("preview");
    hub.confirm_quest_awake(
        &preview.operation_id,
        &preview.preview.preview_id,
        BASE_TIME_MS + 1,
    )
    .expect("confirm");
    let prepared = hub
        .prepare_quest_awake_invocation(
            &preview.operation_id,
            &device_id,
            "awake-request-windows-1".to_owned(),
            BASE_TIME_MS + 2,
        )
        .expect("prepare invocation");
    let invocation = prepared.targets[0].invocation.as_ref().expect("invocation");
    assert_eq!(invocation.action, QuestAwakeAction::StartWindowsWatchdog);
    assert_eq!(
        invocation.watchdog_generation,
        preview.preview.watchdog_generation
    );
    assert_eq!(invocation.expires_at_ms, i64::MAX);
}
