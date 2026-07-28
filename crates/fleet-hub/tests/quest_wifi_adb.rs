// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet};

use fleet_contracts::{
    CommandLifecycle, QUEST_WIFI_ADB_ACTION_ID, QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA,
    QUEST_WIFI_ADB_RECEIPT_SCHEMA, QuestWifiAdbAction, QuestWifiAdbOwnerReceipt,
    QuestWifiAdbPreviewRequest, QuestWifiAdbRouteMode, QuestWifiAdbWearerApproval,
};
use fleet_hub::{FleetHub, HubPolicy, ObservationDecision, QuestWifiAdbPreviewPlan};
use fleet_simulator::{BASE_TIME_MS, ScenarioBuilder};

fn setup() -> (FleetHub, String, u64) {
    let observation = ScenarioBuilder::new(1).build().initial.remove(0);
    let device_id = observation.identity.device_id.clone();
    let identity_revision = observation.identity.identity_revision;
    let mut hub = FleetHub::new(HubPolicy::default());
    assert!(matches!(
        hub.accept_observation(observation, BASE_TIME_MS),
        ObservationDecision::Accepted { .. }
    ));
    (hub, device_id, identity_revision)
}

fn plan(
    device_id: &str,
    identity_revision: u64,
    action: QuestWifiAdbAction,
) -> QuestWifiAdbPreviewPlan {
    QuestWifiAdbPreviewPlan {
        operation_id: "wifi-adb-operation-1".to_owned(),
        preview_id: "wifi-adb-preview-1".to_owned(),
        request: QuestWifiAdbPreviewRequest {
            schema: QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA.to_owned(),
            action_id: QUEST_WIFI_ADB_ACTION_ID.to_owned(),
            action,
            targets: BTreeMap::from([(device_id.to_owned(), identity_revision)]),
        },
        created_at_ms: BASE_TIME_MS,
        expires_at_ms: BASE_TIME_MS + 60_000,
        provider_ready_devices: BTreeSet::from([device_id.to_owned()]),
    }
}

#[test]
fn exact_preview_confirmation_invocation_and_receipt_form_a_per_target_ledger() {
    let (mut hub, device_id, identity_revision) = setup();
    let operation = hub
        .preview_quest_wifi_adb(plan(
            &device_id,
            identity_revision,
            QuestWifiAdbAction::RequestWirelessAdb,
        ))
        .expect("preview");
    hub.confirm_quest_wifi_adb(
        &operation.operation_id,
        &operation.preview.preview_id,
        BASE_TIME_MS + 1,
    )
    .expect("confirm");
    hub.prepare_quest_wifi_adb_invocation(
        &operation.operation_id,
        &device_id,
        "request.wifi.1".to_owned(),
        BASE_TIME_MS + 2,
    )
    .expect("prepare");
    hub.mark_quest_wifi_adb_dispatched(&operation.operation_id, &device_id, BASE_TIME_MS + 3)
        .expect("dispatch");
    let applied = hub
        .apply_quest_wifi_adb_receipt(
            QuestWifiAdbOwnerReceipt {
                schema: QUEST_WIFI_ADB_RECEIPT_SCHEMA.to_owned(),
                request_id: "request.wifi.1".to_owned(),
                operation_id: operation.operation_id.clone(),
                preview_id: operation.preview.preview_id.clone(),
                device_id: device_id.clone(),
                identity_revision,
                action: QuestWifiAdbAction::RequestWirelessAdb,
                route_mode: QuestWifiAdbRouteMode::ModernTls,
                request_delivered: true,
                kiosk_setting_applied: true,
                request_after_boot_enabled: None,
                wearer_approval: QuestWifiAdbWearerApproval::Pending,
                listener_discovered: false,
                effect_applied: true,
                outcome: "wireless_adb_request_applied".to_owned(),
                evidence_sha256: "11".repeat(32),
                observed_at_ms: BASE_TIME_MS + 4,
            },
            BASE_TIME_MS + 4,
        )
        .expect("receipt");
    assert_eq!(applied.lifecycle, CommandLifecycle::Applied);
    assert_eq!(applied.targets[0].lifecycle, CommandLifecycle::Applied);
    assert!(!applied.targets[0].termux_usable);
    assert!(applied.targets[0].termux_proof.is_none());
}

#[test]
fn absent_provider_is_inert_and_classic_action_remains_distinct() {
    let (mut hub, device_id, identity_revision) = setup();
    let mut unavailable = plan(&device_id, identity_revision, QuestWifiAdbAction::Status);
    unavailable.provider_ready_devices.clear();
    assert!(hub.preview_quest_wifi_adb(unavailable).is_err());

    let operation = hub
        .preview_quest_wifi_adb(QuestWifiAdbPreviewPlan {
            operation_id: "wifi-adb-operation-classic".to_owned(),
            preview_id: "wifi-adb-preview-classic".to_owned(),
            ..plan(
                &device_id,
                identity_revision,
                QuestWifiAdbAction::EnableClassicTcpipFromUsb,
            )
        })
        .expect("classic preview");
    assert_eq!(
        operation.preview.action,
        QuestWifiAdbAction::EnableClassicTcpipFromUsb
    );
}
