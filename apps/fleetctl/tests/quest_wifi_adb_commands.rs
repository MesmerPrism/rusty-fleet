// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{
    AuthorizationState, CommandLifecycle, EnablementState, FreshnessState, OperationExecuteRequest,
    OperationPreviewRequest, QUEST_WIFI_ADB_ACTION_ID, QUEST_WIFI_ADB_EXECUTE_REQUEST_SCHEMA,
    QuestWifiAdbExecuteRequest, QuestWifiAdbOperation, QuestWifiAdbOwnerBinding,
    QuestWifiAdbPreview, QuestWifiAdbPreviewRequest, QuestWifiAdbTargetLedger,
    QuestWifiAdbTargetPreflight, ReachabilityState, SupportState,
};
use fleetctl::{CliFailure, FleetOperationClient, execute_with_operation_client};

#[derive(Default)]
struct MockClient {
    operation: Option<QuestWifiAdbOperation>,
}

impl FleetOperationClient for MockClient {
    fn preview_operation(
        &mut self,
        _: &OperationPreviewRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        unreachable!()
    }

    fn execute_operation(
        &mut self,
        _: &OperationExecuteRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        unreachable!()
    }

    fn get_operation(&mut self, _: &str) -> Result<serde_json::Value, CliFailure> {
        unreachable!()
    }

    fn preview_quest_wifi_adb(
        &mut self,
        request: &QuestWifiAdbPreviewRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        let operation = operation(request);
        self.operation = Some(operation.clone());
        serde_json::to_value(operation)
            .map_err(|error| CliFailure::new("serialization_failed", error.to_string()))
    }

    fn execute_quest_wifi_adb(
        &mut self,
        request: &QuestWifiAdbExecuteRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        assert_eq!(request.schema, QUEST_WIFI_ADB_EXECUTE_REQUEST_SCHEMA);
        let operation = self.operation.as_mut().expect("preview first");
        operation.lifecycle = CommandLifecycle::Accepted;
        operation.confirmed_at_ms = Some(1_100);
        serde_json::to_value(operation.clone())
            .map_err(|error| CliFailure::new("serialization_failed", error.to_string()))
    }

    fn get_quest_wifi_adb(&mut self, _: &str) -> Result<serde_json::Value, CliFailure> {
        serde_json::to_value(self.operation.clone().expect("preview first"))
            .map_err(|error| CliFailure::new("serialization_failed", error.to_string()))
    }

    fn list_quest_wifi_adb(&mut self) -> Result<serde_json::Value, CliFailure> {
        serde_json::to_value(
            self.operation
                .clone()
                .into_iter()
                .collect::<Vec<QuestWifiAdbOperation>>(),
        )
        .map_err(|error| CliFailure::new("serialization_failed", error.to_string()))
    }
}

fn operation(request: &QuestWifiAdbPreviewRequest) -> QuestWifiAdbOperation {
    let preflight = QuestWifiAdbTargetPreflight {
        device_id: "device.quest.1".to_owned(),
        identity_revision: 7,
        capability_id: "questionable-file-manager.quest-wifi-adb-provider".to_owned(),
        capability_evidence_revision: 11,
        capability_owner: "questionable-file-manager".to_owned(),
        support: SupportState::Supported,
        enablement: EnablementState::Enabled,
        authorization: AuthorizationState::Authorized,
        reachability: ReachabilityState::Reachable,
        freshness: FreshnessState::Current,
        observed_at_ms: 900,
        fresh_until_ms: 30_000,
        evaluated_at_ms: 1_000,
        eligible: true,
        reason_code: "ready".to_owned(),
        message: "Pinned provider is ready".to_owned(),
    };
    QuestWifiAdbOperation {
        schema: "rusty.fleet.quest_wifi_adb_operation.v1".to_owned(),
        operation_id: "wifi-adb-operation-1".to_owned(),
        action_id: QUEST_WIFI_ADB_ACTION_ID.to_owned(),
        lifecycle: CommandLifecycle::Proposed,
        preview: QuestWifiAdbPreview {
            schema: "rusty.fleet.quest_wifi_adb_preview.v1".to_owned(),
            preview_id: "wifi-adb-preview-1".to_owned(),
            operation_id: "wifi-adb-operation-1".to_owned(),
            action_id: QUEST_WIFI_ADB_ACTION_ID.to_owned(),
            action: request.action,
            created_at_ms: 1_000,
            expires_at_ms: 61_000,
            fleet_revision: 7,
            owner: QuestWifiAdbOwnerBinding::file_manager_v1(),
            targets: vec![preflight.clone()],
        },
        confirmed_at_ms: None,
        targets: vec![QuestWifiAdbTargetLedger {
            device_id: preflight.device_id.clone(),
            identity_revision: preflight.identity_revision,
            preflight,
            lifecycle: CommandLifecycle::Proposed,
            invocation: None,
            receipt: None,
            termux_proof: None,
            termux_admission: None,
            termux_usable: false,
            failure_code: None,
            updated_at_ms: 1_000,
        }],
        updated_at_ms: 1_000,
    }
}

#[test]
fn cli_exposes_every_typed_action_and_execute_get_parity() {
    for action in [
        "status",
        "request-wireless-adb",
        "enable-request-after-boot",
        "disable-request-after-boot",
        "disable-wireless-adb",
        "enable-classic-tcpip-from-usb",
    ] {
        let mut client = MockClient::default();
        let preview = execute_with_operation_client(
            vec![
                "wifi-adb-preview".to_owned(),
                action.to_owned(),
                "device.quest.1@7".to_owned(),
            ],
            &mut client,
        )
        .expect("preview");
        assert_eq!(
            preview["preview"]["targets"][0]["device_id"],
            "device.quest.1"
        );
        let executed = execute_with_operation_client(
            vec![
                "wifi-adb-execute".to_owned(),
                "wifi-adb-operation-1".to_owned(),
                "wifi-adb-preview-1".to_owned(),
            ],
            &mut client,
        )
        .expect("execute");
        assert_eq!(executed["lifecycle"], "accepted");
        let fetched = execute_with_operation_client(
            vec!["wifi-adb-get".to_owned(), "wifi-adb-operation-1".to_owned()],
            &mut client,
        )
        .expect("get");
        assert_eq!(fetched, executed);
        let listed = execute_with_operation_client(vec!["wifi-adb-list".to_owned()], &mut client)
            .expect("list");
        assert_eq!(listed.as_array().expect("array").len(), 1);
        assert_eq!(listed[0], executed);
    }
}

#[test]
fn cli_rejects_unknown_action_and_duplicate_target() {
    let mut client = MockClient::default();
    let error = execute_with_operation_client(
        vec![
            "wifi-adb-preview".to_owned(),
            "magic".to_owned(),
            "device.quest.1@7".to_owned(),
        ],
        &mut client,
    )
    .expect_err("unknown action");
    assert_eq!(error.code, "unsupported_wifi_adb_action");

    let error = execute_with_operation_client(
        vec![
            "wifi-adb-preview".to_owned(),
            "status".to_owned(),
            "device.quest.1@7".to_owned(),
            "device.quest.1@7".to_owned(),
        ],
        &mut client,
    )
    .expect_err("duplicate");
    assert_eq!(error.code, "duplicate_target");
}
