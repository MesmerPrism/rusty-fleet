// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use fleet_contracts::{
    CommandLifecycle, KIOSK_CLI_RESULT_SCHEMA, KIOSK_DIRECT_OPERATOR_INVOKE_TARGET,
    KIOSK_DIRECT_OPERATOR_REVISION, KIOSK_DIRECT_OPERATOR_SCHEMA, KIOSK_SHOW_CONTROLS_ACTION_ID,
    KIOSK_SHOW_CONTROLS_COMMAND, KioskCancelDisposition, KioskRetryDisposition,
    KioskShowControlsOperation, OPERATION_EXECUTE_REQUEST_SCHEMA, OPERATION_PREVIEW_REQUEST_SCHEMA,
    OperationExecuteRequest, OperationPreviewRequest, ValidateContract,
};

fn valid_operation() -> KioskShowControlsOperation {
    serde_json::from_str(include_str!(
        "../../../fixtures/contracts/kiosk-show-controls-operation.valid.json"
    ))
    .expect("valid Kiosk show-controls fixture")
}

#[test]
fn committed_show_controls_fixture_pins_owner_contract_and_effective_readback() {
    let operation = valid_operation();
    assert!(operation.validate().is_ok());
    assert_eq!(operation.action_id, KIOSK_SHOW_CONTROLS_ACTION_ID);
    assert_eq!(
        operation.preview.owner_contract.owner_contract_schema,
        KIOSK_DIRECT_OPERATOR_SCHEMA
    );
    assert_eq!(
        operation.preview.owner_contract.owner_contract_revision,
        KIOSK_DIRECT_OPERATOR_REVISION
    );
    assert_eq!(
        operation.preview.owner_contract.invoke_target,
        KIOSK_DIRECT_OPERATOR_INVOKE_TARGET
    );
    assert_eq!(
        operation.preview.owner_contract.command,
        KIOSK_SHOW_CONTROLS_COMMAND
    );
    assert!(operation.preview.owner_contract.command_value.is_none());

    let receipt = operation.targets[0]
        .effective_receipt
        .as_ref()
        .expect("applied target has effective receipt");
    assert_eq!(receipt.owner_result_schema, KIOSK_CLI_RESULT_SCHEMA);
    assert!(receipt.response_auth_verified);
    assert!(receipt.owner_accepted);
    assert!(receipt.owner_completed);
    assert!(receipt.controls_open);
}

#[test]
fn committed_damaged_show_controls_fixture_fails_closed() {
    let operation: KioskShowControlsOperation = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/kiosk-show-controls-operation.damaged.json"
    ))
    .expect("damaged fixture remains syntactically valid JSON");
    let codes = operation
        .validate()
        .expect_err("damaged show-controls operation must fail")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    for expected in [
        "owner_contract_mismatch",
        "preflight_result_mismatch",
        "cleanup_forbidden",
        "invalid_parallelism",
        "preview_target_mismatch",
        "attempt_history_mismatch",
        "owner_request_reused",
        "applied_without_effective_receipt",
        "unverified_owner_response",
        "ineffective_owner_receipt",
        "invalid_receipt_time",
        "effective_receipt_binding_mismatch",
    ] {
        assert!(
            codes.iter().any(|code| code == expected),
            "missing expected failure code {expected}: {codes:?}"
        );
    }
}

#[test]
fn retry_requires_a_new_owner_request_and_remains_bounded() {
    let mut operation = valid_operation();
    {
        let target = &mut operation.targets[0];
        target.lifecycle = CommandLifecycle::Failed;
        target.dispatched_at_ms = Some(2_000_000_000_200);
        target.owner_deadline_at_ms = Some(2_000_000_060_000);
        target.attempt_count = 2;
        target.owner_request_ids = vec!["fleetctl-0001".to_owned(), "fleetctl-0002".to_owned()];
        target.owner_request_id = Some("fleetctl-0002".to_owned());
        target.effective_receipt = None;
        target.retry_disposition = KioskRetryDisposition::NewOwnerRequestRequired;
        target.cancel_disposition = KioskCancelDisposition::Terminal;
        target.reason_code = "owner_response_unavailable".to_owned();
    }
    operation.lifecycle = CommandLifecycle::Failed;
    assert!(operation.validate().is_ok());

    operation.targets[0].owner_request_ids[1] = "fleetctl-0001".to_owned();
    let codes = operation
        .validate()
        .expect_err("request-id reuse must fail")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"owner_request_reused".to_owned()));
}

#[test]
fn cancellation_is_only_valid_before_dispatch() {
    let mut operation = valid_operation();
    {
        let target = &mut operation.targets[0];
        target.lifecycle = CommandLifecycle::Cancelled;
        target.dispatched_at_ms = None;
        target.owner_deadline_at_ms = None;
        target.attempt_count = 0;
        target.owner_request_ids.clear();
        target.owner_request_id = None;
        target.effective_receipt = None;
        target.retry_disposition = KioskRetryDisposition::NotEligible;
        target.cancel_disposition = KioskCancelDisposition::CancelledBeforeDispatch;
        target.reason_code = "cancelled_before_dispatch".to_owned();
    }
    operation.lifecycle = CommandLifecycle::Cancelled;
    assert!(operation.validate().is_ok());

    let target = &mut operation.targets[0];
    target.attempt_count = 1;
    target.dispatched_at_ms = Some(2_000_000_000_200);
    target.owner_deadline_at_ms = Some(2_000_000_060_000);
    target.owner_request_ids = vec!["fleetctl-0001".to_owned()];
    target.owner_request_id = Some("fleetctl-0001".to_owned());
    let codes = operation
        .validate()
        .expect_err("post-dispatch cancellation must fail")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"unsafe_cancellation".to_owned()));
}

#[test]
fn pending_or_unsigned_owner_result_cannot_become_effective() {
    let mut operation = valid_operation();
    let receipt = operation.targets[0]
        .effective_receipt
        .as_mut()
        .expect("valid fixture has receipt");
    receipt.owner_completed = false;
    receipt.controls_open = false;
    receipt.response_auth_verified = false;
    let codes = operation
        .validate()
        .expect_err("pending unsigned result must fail")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"unverified_owner_response".to_owned()));
    assert!(codes.contains(&"ineffective_owner_receipt".to_owned()));
}

#[test]
fn show_controls_never_enters_cleanup() {
    let mut operation = valid_operation();
    operation.cleanup_required = true;
    operation.lifecycle = CommandLifecycle::CleanupPending;
    let codes = operation
        .validate()
        .expect_err("cleanup is forbidden for show-controls")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"cleanup_forbidden".to_owned()));
}

#[test]
fn operation_lifecycle_is_derived_from_eligible_targets() {
    let mut operation = valid_operation();
    operation.lifecycle = CommandLifecycle::Running;
    let codes = operation
        .validate()
        .expect_err("aggregate running cannot hide terminal target ledgers")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"operation_lifecycle_mismatch".to_owned()));
}

#[test]
fn effective_receipt_binds_command_and_distinct_transport_request() {
    let mut operation = valid_operation();
    let receipt = operation.targets[0]
        .effective_receipt
        .as_mut()
        .expect("valid fixture has receipt");
    receipt.owner_command = "status".to_owned();
    receipt.owner_result_transport_request_id = receipt.owner_action_request_id.clone();
    let codes = operation
        .validate()
        .expect_err("wrong command and reused transport request must fail")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"owner_request_id_reused".to_owned()));
    assert!(codes.contains(&"ineffective_owner_receipt".to_owned()));
}

#[test]
fn operation_wire_requests_validate_exact_identity_bindings() {
    let preview = OperationPreviewRequest {
        schema: OPERATION_PREVIEW_REQUEST_SCHEMA.to_owned(),
        action_id: KIOSK_SHOW_CONTROLS_ACTION_ID.to_owned(),
        targets: BTreeMap::from([("sim-00001".to_owned(), 7)]),
    };
    assert!(preview.validate().is_ok());
    let execute = OperationExecuteRequest {
        schema: OPERATION_EXECUTE_REQUEST_SCHEMA.to_owned(),
        operation_id: "show-controls-operation-0001".to_owned(),
        preview_id: "show-controls-preview-0001".to_owned(),
    };
    assert!(execute.validate().is_ok());
}
