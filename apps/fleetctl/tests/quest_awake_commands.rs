// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{
    AuthorizationState, CommandLifecycle, EnablementState, FreshnessState, OperationExecuteRequest,
    OperationPreviewRequest, QUEST_AWAKE_ACTION_ID, QUEST_AWAKE_EXECUTE_REQUEST_SCHEMA,
    QUEST_AWAKE_PREVIEW_REQUEST_SCHEMA, QuestAwakeExecuteRequest, QuestAwakeOperation,
    QuestAwakeOwnerBinding, QuestAwakePreview, QuestAwakePreviewRequest, QuestAwakeTargetLedger,
    QuestAwakeTargetPreflight, ReachabilityState, SupportState,
};
use fleetctl::{CliFailure, FleetOperationClient, execute_with_operation_client};

#[derive(Default)]
struct MockAwakeClient {
    preview_request: Option<QuestAwakePreviewRequest>,
    operation: Option<QuestAwakeOperation>,
}

impl FleetOperationClient for MockAwakeClient {
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

    fn preview_quest_awake(
        &mut self,
        request: &QuestAwakePreviewRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.preview_request = Some(request.clone());
        let operation = operation(request);
        self.operation = Some(operation.clone());
        Ok(serde_json::to_value(operation).expect("serialize operation"))
    }

    fn execute_quest_awake(
        &mut self,
        request: &QuestAwakeExecuteRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        assert_eq!(request.schema, QUEST_AWAKE_EXECUTE_REQUEST_SCHEMA);
        let operation = self.operation.as_mut().expect("preview first");
        assert_eq!(operation.operation_id, request.operation_id);
        assert_eq!(operation.preview.preview_id, request.preview_id);
        operation.lifecycle = CommandLifecycle::Accepted;
        operation.confirmed_at_ms = Some(1_100);
        Ok(serde_json::to_value(operation).expect("serialize operation"))
    }

    fn get_quest_awake(&mut self, operation_id: &str) -> Result<serde_json::Value, CliFailure> {
        let operation = self.operation.as_ref().expect("preview first");
        assert_eq!(operation.operation_id, operation_id);
        Ok(serde_json::to_value(operation).expect("serialize operation"))
    }
}

fn operation(request: &QuestAwakePreviewRequest) -> QuestAwakeOperation {
    let preflight = QuestAwakeTargetPreflight {
        device_id: "device.quest.1".to_owned(),
        identity_revision: 7,
        capability_id: "questionable-file-manager.quest-awake-provider".to_owned(),
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
        message: "Pinned Quest awake provider is ready.".to_owned(),
    };
    QuestAwakeOperation {
        schema: "rusty.fleet.quest_awake_operation.v1".to_owned(),
        operation_id: "awake-operation-1".to_owned(),
        action_id: QUEST_AWAKE_ACTION_ID.to_owned(),
        lifecycle: CommandLifecycle::Proposed,
        preview: QuestAwakePreview {
            schema: "rusty.fleet.quest_awake_preview.v1".to_owned(),
            preview_id: "awake-preview-1".to_owned(),
            operation_id: "awake-operation-1".to_owned(),
            action_id: QUEST_AWAKE_ACTION_ID.to_owned(),
            action: request.action,
            created_at_ms: 1_000,
            expires_at_ms: 61_000,
            fleet_revision: 7,
            duration_ms: request.duration_ms,
            watchdog_interval_ms: request.watchdog_interval_ms,
            watchdog_generation: "awake-generation-1".to_owned(),
            owner: QuestAwakeOwnerBinding::file_manager_v1(),
            targets: vec![preflight.clone()],
        },
        confirmed_at_ms: None,
        targets: vec![QuestAwakeTargetLedger {
            device_id: preflight.device_id.clone(),
            identity_revision: preflight.identity_revision,
            preflight,
            lifecycle: CommandLifecycle::Proposed,
            invocation: None,
            receipt: None,
            failure_code: None,
            updated_at_ms: 1_000,
        }],
        updated_at_ms: 1_000,
    }
}

fn preview(action: &str) -> Vec<String> {
    [
        "awake-preview",
        action,
        "28800000",
        "5000",
        "device.quest.1@7",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[test]
fn awake_cli_binds_action_policy_targets_and_execute_get() {
    for action in [
        "status",
        "apply-bounded",
        "start-windows-watchdog",
        "start-device-watchdog",
        "stop-watchdogs",
        "restore-normal",
    ] {
        let mut client = MockAwakeClient::default();
        let previewed =
            execute_with_operation_client(preview(action), &mut client).expect("awake preview");
        let request = client.preview_request.as_ref().expect("preview request");
        assert_eq!(request.schema, QUEST_AWAKE_PREVIEW_REQUEST_SCHEMA);
        assert_eq!(request.duration_ms, 28_800_000);
        assert_eq!(request.watchdog_interval_ms, 5_000);
        assert_eq!(request.targets.get("device.quest.1"), Some(&7));
        assert_eq!(previewed["preview"]["duration_ms"], 28_800_000);

        let executed = execute_with_operation_client(
            vec![
                "awake-execute".to_owned(),
                "awake-operation-1".to_owned(),
                "awake-preview-1".to_owned(),
            ],
            &mut client,
        )
        .expect("awake execute");
        assert_eq!(executed["lifecycle"], "accepted");
        let fetched = execute_with_operation_client(
            vec!["awake-get".to_owned(), "awake-operation-1".to_owned()],
            &mut client,
        )
        .expect("awake get");
        assert_eq!(fetched, executed);
    }
}

#[test]
fn awake_cli_rejects_unknown_action_over_eight_hours_and_duplicates() {
    let mut client = MockAwakeClient::default();
    let error =
        execute_with_operation_client(preview("forever"), &mut client).expect_err("unknown action");
    assert_eq!(error.code, "unsupported_awake_action");

    let mut too_long = preview("apply-bounded");
    too_long[2] = "28800001".to_owned();
    let error =
        execute_with_operation_client(too_long, &mut client).expect_err("duration over cap");
    assert_eq!(error.code, "invalid_awake_preview_request");

    let mut duplicate = preview("status");
    duplicate.push("device.quest.1@7".to_owned());
    let error =
        execute_with_operation_client(duplicate, &mut client).expect_err("duplicate target");
    assert_eq!(error.code, "duplicate_target");
}
