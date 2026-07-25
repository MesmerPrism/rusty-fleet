// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{OperationExecuteRequest, OperationPreviewRequest};
use fleetctl::{CliFailure, FleetOperationClient, execute, execute_with_operation_client};

fn valid_operation() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../fixtures/contracts/kiosk-show-controls-operation.valid.json"
    ))
    .expect("valid operation fixture")
}

fn damaged_operation() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../fixtures/contracts/kiosk-show-controls-operation.damaged.json"
    ))
    .expect("damaged operation fixture remains JSON")
}

#[derive(Default)]
struct MockOperationClient {
    response: serde_json::Value,
    previews: Vec<OperationPreviewRequest>,
    executions: Vec<OperationExecuteRequest>,
    lookups: Vec<String>,
}

impl MockOperationClient {
    fn returning(response: serde_json::Value) -> Self {
        Self {
            response,
            ..Self::default()
        }
    }
}

impl FleetOperationClient for MockOperationClient {
    fn preview_operation(
        &mut self,
        request: &OperationPreviewRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.previews.push(request.clone());
        Ok(self.response.clone())
    }

    fn execute_operation(
        &mut self,
        request: &OperationExecuteRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.executions.push(request.clone());
        Ok(self.response.clone())
    }

    fn get_operation(&mut self, operation_id: &str) -> Result<serde_json::Value, CliFailure> {
        self.lookups.push(operation_id.to_owned());
        Ok(self.response.clone())
    }
}

fn preview_arguments() -> Vec<String> {
    [
        "operation-preview",
        "kiosk.show-controls",
        "sim-00001@7",
        "sim-00002@3",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn execute_arguments() -> Vec<String> {
    [
        "operation-execute",
        "show-controls-operation-0001",
        "show-controls-preview-0001",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn get_arguments() -> Vec<String> {
    ["operation-get", "show-controls-operation-0001"]
        .map(str::to_owned)
        .to_vec()
}

#[test]
fn preview_execute_and_get_have_exact_raw_json_parity() {
    let expected = valid_operation();
    let mut client = MockOperationClient::returning(expected.clone());

    assert_eq!(
        execute_with_operation_client(preview_arguments(), &mut client)
            .expect("preview projection"),
        expected
    );
    assert_eq!(
        execute_with_operation_client(execute_arguments(), &mut client)
            .expect("execute projection"),
        expected
    );
    assert_eq!(
        execute_with_operation_client(get_arguments(), &mut client).expect("get projection"),
        expected
    );

    assert_eq!(client.previews.len(), 1);
    assert_eq!(
        client.previews[0].targets,
        [("sim-00001".to_owned(), 7), ("sim-00002".to_owned(), 3)].into()
    );
    assert_eq!(client.executions.len(), 1);
    assert_eq!(
        client.executions[0].operation_id,
        "show-controls-operation-0001"
    );
    assert_eq!(
        client.executions[0].preview_id,
        "show-controls-preview-0001"
    );
    assert_eq!(client.lookups, ["show-controls-operation-0001"]);
}

#[test]
fn repeated_commands_preserve_idempotent_request_identity() {
    let response = valid_operation();
    let mut client = MockOperationClient::returning(response.clone());

    let first_preview =
        execute_with_operation_client(preview_arguments(), &mut client).expect("first preview");
    let second_preview =
        execute_with_operation_client(preview_arguments(), &mut client).expect("second preview");
    assert_eq!(first_preview, response);
    assert_eq!(second_preview, response);
    assert_eq!(client.previews[0], client.previews[1]);

    let first_execute =
        execute_with_operation_client(execute_arguments(), &mut client).expect("first execute");
    let second_execute =
        execute_with_operation_client(execute_arguments(), &mut client).expect("second execute");
    assert_eq!(first_execute, response);
    assert_eq!(second_execute, response);
    assert_eq!(client.executions[0], client.executions[1]);
}

#[test]
fn target_parser_uses_the_final_at_separator() {
    let mut response = valid_operation();
    response["preview"]["targets"][0]["device_id"] = "device@alpha".into();
    response["targets"][0]["device_id"] = "device@alpha".into();
    response["targets"][0]["preflight"]["device_id"] = "device@alpha".into();
    response["targets"][0]["effective_receipt"]["device_id"] = "device@alpha".into();
    let mut client = MockOperationClient::returning(response.clone());
    let arguments = [
        "operation-preview",
        "kiosk.show-controls",
        "device@alpha@7",
        "sim-00002@3",
    ]
    .map(str::to_owned)
    .to_vec();

    assert_eq!(
        execute_with_operation_client(arguments, &mut client).expect("final @ target"),
        response
    );
    assert_eq!(client.previews[0].targets["device@alpha"], 7);
}

#[test]
fn rejects_duplicate_zero_overflow_and_malformed_targets_before_transport() {
    let invalid_cases = [
        (
            vec![
                "operation-preview",
                "kiosk.show-controls",
                "sim-00001@7",
                "sim-00001@8",
            ],
            "duplicate_target",
        ),
        (
            vec!["operation-preview", "kiosk.show-controls", "sim-00001@0"],
            "invalid_identity_revision",
        ),
        (
            vec![
                "operation-preview",
                "kiosk.show-controls",
                "sim-00001@18446744073709551616",
            ],
            "invalid_identity_revision",
        ),
        (
            vec!["operation-preview", "kiosk.show-controls", "sim-00001"],
            "invalid_target",
        ),
        (
            vec!["operation-preview", "kiosk.show-controls", "@7"],
            "invalid_target",
        ),
        (
            vec!["operation-preview", "kiosk.show-controls", "sim-00001@"],
            "invalid_target",
        ),
    ];
    for (arguments, expected_code) in invalid_cases {
        let mut client = MockOperationClient::returning(valid_operation());
        let failure = execute_with_operation_client(
            arguments.into_iter().map(str::to_owned).collect(),
            &mut client,
        )
        .expect_err("invalid target");
        assert_eq!(failure.code, expected_code);
        assert!(client.previews.is_empty());
    }
}

#[test]
fn rejects_missing_wrong_and_extra_arguments_before_transport() {
    let invalid_cases = [
        (vec!["operation-preview"], "missing_action_id"),
        (
            vec!["operation-preview", "kiosk.show-controls"],
            "missing_targets",
        ),
        (
            vec!["operation-preview", "other.action", "sim-00001@7"],
            "unsupported_action",
        ),
        (
            vec![
                "operation-execute",
                "show-controls-operation-0001",
                "show-controls-preview-0001",
                "extra",
            ],
            "unexpected_arguments",
        ),
        (
            vec!["operation-get", "show-controls-operation-0001", "extra"],
            "unexpected_arguments",
        ),
    ];
    for (arguments, expected_code) in invalid_cases {
        let mut client = MockOperationClient::returning(valid_operation());
        let failure = execute_with_operation_client(
            arguments.into_iter().map(str::to_owned).collect(),
            &mut client,
        )
        .expect_err("invalid arguments");
        assert_eq!(failure.code, expected_code);
        assert!(client.previews.is_empty());
        assert!(client.executions.is_empty());
        assert!(client.lookups.is_empty());
    }
}

#[test]
fn rejects_malformed_invalid_and_mismatched_operation_responses() {
    let mut malformed = MockOperationClient::returning(serde_json::json!({
        "schema": "rusty.fleet.kiosk_show_controls_operation.v1"
    }));
    assert_eq!(
        execute_with_operation_client(get_arguments(), &mut malformed)
            .expect_err("malformed operation")
            .code,
        "malformed_operation_response"
    );

    let mut invalid = MockOperationClient::returning(damaged_operation());
    assert_eq!(
        execute_with_operation_client(get_arguments(), &mut invalid)
            .expect_err("invalid operation")
            .code,
        "invalid_operation_response"
    );

    let mut mismatched = MockOperationClient::returning(valid_operation());
    assert_eq!(
        execute_with_operation_client(
            ["operation-get", "different-operation"]
                .map(str::to_owned)
                .to_vec(),
            &mut mismatched,
        )
        .expect_err("mismatched operation")
        .code,
        "operation_response_mismatch"
    );
}

#[test]
fn legacy_entrypoint_preserves_old_commands_and_requires_explicit_operation_client() {
    assert!(execute(vec!["list".to_owned(), "4".to_owned()]).is_ok());
    assert_eq!(
        execute(get_arguments())
            .expect_err("operation client required")
            .code,
        "operation_client_required"
    );
}
