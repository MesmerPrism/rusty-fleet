// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use fleet_contracts::{
    KIOSK_SHOW_CONTROLS_ACTION_ID, KioskShowControlsOperation, OPERATION_EXECUTE_REQUEST_SCHEMA,
    OPERATION_PREVIEW_REQUEST_SCHEMA, OperationExecuteRequest, OperationPreviewRequest,
    ValidateContract,
};

use crate::CliFailure;

/// Transport-neutral projection of the Fleet Hub operation routes.
///
/// Responses remain raw JSON until this crate has decoded and validated the
/// complete Kiosk show-controls operation contract.
pub trait FleetOperationClient {
    fn preview_operation(
        &mut self,
        request: &OperationPreviewRequest,
    ) -> Result<serde_json::Value, CliFailure>;

    fn execute_operation(
        &mut self,
        request: &OperationExecuteRequest,
    ) -> Result<serde_json::Value, CliFailure>;

    fn get_operation(&mut self, operation_id: &str) -> Result<serde_json::Value, CliFailure>;
}

pub fn is_operation_command(command: &str) -> bool {
    matches!(
        command,
        "operation-preview" | "operation-execute" | "operation-get"
    )
}

pub(crate) fn execute_operation_command<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    match arguments.first().map(String::as_str) {
        Some("operation-preview") => preview(arguments, client),
        Some("operation-execute") => execute(arguments, client),
        Some("operation-get") => get(arguments, client),
        _ => Err(CliFailure::new(
            "unknown_command",
            "the requested operation command is unknown",
        )),
    }
}

fn preview<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    let action_id = arguments.get(1).ok_or_else(|| {
        CliFailure::new(
            "missing_action_id",
            "operation-preview requires kiosk.show-controls",
        )
    })?;
    if action_id != KIOSK_SHOW_CONTROLS_ACTION_ID {
        return Err(CliFailure::new(
            "unsupported_action",
            format!("operation-preview does not support action {action_id}"),
        ));
    }
    if arguments.len() < 3 {
        return Err(CliFailure::new(
            "missing_targets",
            "operation-preview requires at least one DEVICE@IDENTITY_REVISION target",
        ));
    }
    if arguments.len() - 2 > 10_000 {
        return Err(CliFailure::new(
            "too_many_targets",
            "operation-preview accepts at most 10,000 targets",
        ));
    }

    let mut targets = BTreeMap::new();
    for target in &arguments[2..] {
        let (device_id, identity_revision) = parse_target(target)?;
        if targets
            .insert(device_id.clone(), identity_revision)
            .is_some()
        {
            return Err(CliFailure::new(
                "duplicate_target",
                format!("target {device_id} was specified more than once"),
            ));
        }
    }
    let request = OperationPreviewRequest {
        schema: OPERATION_PREVIEW_REQUEST_SCHEMA.to_owned(),
        action_id: action_id.clone(),
        targets,
    };
    validate_request(&request, "invalid_preview_request")?;
    let raw = client.preview_operation(&request)?;
    let operation = validate_operation(&raw)?;
    if operation.action_id != request.action_id || preview_targets(&operation) != request.targets {
        return Err(CliFailure::new(
            "operation_response_mismatch",
            "Fleet Hub preview did not bind the exact requested action and target identities",
        ));
    }
    Ok(raw)
}

fn execute<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    if arguments.len() != 3 {
        return Err(CliFailure::new(
            "unexpected_arguments",
            "operation-execute requires exactly OPERATION_ID PREVIEW_ID",
        ));
    }
    let request = OperationExecuteRequest {
        schema: OPERATION_EXECUTE_REQUEST_SCHEMA.to_owned(),
        operation_id: arguments[1].clone(),
        preview_id: arguments[2].clone(),
    };
    validate_request(&request, "invalid_execute_request")?;
    let raw = client.execute_operation(&request)?;
    let operation = validate_operation(&raw)?;
    if operation.operation_id != request.operation_id
        || operation.preview.preview_id != request.preview_id
    {
        return Err(CliFailure::new(
            "operation_response_mismatch",
            "Fleet Hub execution did not bind the requested operation and immutable preview",
        ));
    }
    Ok(raw)
}

fn get<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    if arguments.len() != 2 {
        return Err(CliFailure::new(
            "unexpected_arguments",
            "operation-get requires exactly OPERATION_ID",
        ));
    }
    validate_identifier(&arguments[1], "operation_id")?;
    let raw = client.get_operation(&arguments[1])?;
    let operation = validate_operation(&raw)?;
    if operation.operation_id != arguments[1] {
        return Err(CliFailure::new(
            "operation_response_mismatch",
            "Fleet Hub lookup returned a different operation ID",
        ));
    }
    Ok(raw)
}

fn parse_target(target: &str) -> Result<(String, u64), CliFailure> {
    let (device_id, revision) = target.rsplit_once('@').ok_or_else(|| {
        CliFailure::new(
            "invalid_target",
            format!("target {target} must use DEVICE@IDENTITY_REVISION"),
        )
    })?;
    if device_id.is_empty() || device_id.len() > 256 || revision.is_empty() {
        return Err(CliFailure::new(
            "invalid_target",
            format!("target {target} must contain a bounded device ID and revision"),
        ));
    }
    let identity_revision = revision.parse::<u64>().map_err(|_| {
        CliFailure::new(
            "invalid_identity_revision",
            format!("target {target} has an invalid identity revision"),
        )
    })?;
    if identity_revision == 0 {
        return Err(CliFailure::new(
            "invalid_identity_revision",
            format!("target {target} identity revision must be greater than zero"),
        ));
    }
    Ok((device_id.to_owned(), identity_revision))
}

fn validate_identifier(value: &str, name: &str) -> Result<(), CliFailure> {
    if value.is_empty() || value.len() > 256 {
        return Err(CliFailure::new(
            "invalid_identifier",
            format!("{name} must contain 1 through 256 bytes"),
        ));
    }
    Ok(())
}

fn validate_request<T: ValidateContract>(request: &T, code: &str) -> Result<(), CliFailure> {
    request.validate().map_err(|failures| {
        CliFailure::new(
            code,
            format!(
                "request contract rejected: {}",
                violation_summary(&failures)
            ),
        )
    })
}

fn validate_operation(raw: &serde_json::Value) -> Result<KioskShowControlsOperation, CliFailure> {
    let operation = serde_json::from_value::<KioskShowControlsOperation>(raw.clone())
        .map_err(|error| CliFailure::new("malformed_operation_response", error.to_string()))?;
    operation.validate().map_err(|failures| {
        CliFailure::new(
            "invalid_operation_response",
            format!(
                "Fleet Hub operation contract rejected: {}",
                violation_summary(&failures)
            ),
        )
    })?;
    Ok(operation)
}

fn preview_targets(operation: &KioskShowControlsOperation) -> BTreeMap<String, u64> {
    operation
        .preview
        .targets
        .iter()
        .map(|target| (target.device_id.clone(), target.identity_revision))
        .collect()
}

fn violation_summary(failures: &[fleet_contracts::ContractViolation]) -> String {
    failures
        .iter()
        .map(|failure| format!("{}@{}", failure.code, failure.path))
        .collect::<Vec<_>>()
        .join(", ")
}
