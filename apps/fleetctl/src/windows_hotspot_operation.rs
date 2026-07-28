// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{
    ValidateContract, WINDOWS_HOTSPOT_ACTION_ID, WINDOWS_HOTSPOT_EXECUTE_REQUEST_SCHEMA,
    WINDOWS_HOTSPOT_PREVIEW_REQUEST_SCHEMA, WindowsHotspotAction, WindowsHotspotExecuteRequest,
    WindowsHotspotOperation, WindowsHotspotPreviewRequest,
};

use crate::CliFailure;
use crate::operation::{
    FleetOperationClient, validate_identifier, validate_request, violation_summary,
};

pub(crate) fn is_windows_hotspot_command(command: &str) -> bool {
    matches!(
        command,
        "hotspot-preview" | "hotspot-execute" | "hotspot-get"
    )
}

pub(crate) fn execute_windows_hotspot_command<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    match arguments.first().map(String::as_str) {
        Some("hotspot-preview") => preview(arguments, client),
        Some("hotspot-execute") => execute(arguments, client),
        Some("hotspot-get") => get(arguments, client),
        _ => Err(CliFailure::new(
            "unknown_command",
            "the requested Windows hotspot command is unknown",
        )),
    }
}

fn preview<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    if arguments.len() != 2 {
        return Err(CliFailure::new(
            "unexpected_arguments",
            "hotspot-preview requires exactly status, start, ensure, or stop",
        ));
    }
    let action = match arguments[1].as_str() {
        "status" => WindowsHotspotAction::Status,
        "start" => WindowsHotspotAction::Start,
        "ensure" => WindowsHotspotAction::Ensure,
        "stop" => WindowsHotspotAction::Stop,
        _ => {
            return Err(CliFailure::new(
                "unsupported_hotspot_action",
                "hotspot action must be status, start, ensure, or stop",
            ));
        }
    };
    let request = WindowsHotspotPreviewRequest {
        schema: WINDOWS_HOTSPOT_PREVIEW_REQUEST_SCHEMA.to_owned(),
        action_id: WINDOWS_HOTSPOT_ACTION_ID.to_owned(),
        action,
    };
    validate_request(&request, "invalid_hotspot_preview_request")?;
    let raw = client.preview_windows_hotspot(&request)?;
    let operation = validate_operation(&raw)?;
    if operation.preview.action != action {
        return Err(CliFailure::new(
            "hotspot_operation_response_mismatch",
            "Fleet Hub preview returned a different host action",
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
            "hotspot-execute requires exactly OPERATION_ID PREVIEW_ID",
        ));
    }
    let request = WindowsHotspotExecuteRequest {
        schema: WINDOWS_HOTSPOT_EXECUTE_REQUEST_SCHEMA.to_owned(),
        operation_id: arguments[1].clone(),
        preview_id: arguments[2].clone(),
    };
    validate_request(&request, "invalid_hotspot_execute_request")?;
    let raw = client.execute_windows_hotspot(&request)?;
    let operation = validate_operation(&raw)?;
    if operation.operation_id != request.operation_id
        || operation.preview.preview_id != request.preview_id
    {
        return Err(CliFailure::new(
            "hotspot_operation_response_mismatch",
            "Fleet Hub execution did not bind the immutable preview",
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
            "hotspot-get requires exactly OPERATION_ID",
        ));
    }
    validate_identifier(&arguments[1], "operation_id")?;
    let raw = client.get_windows_hotspot(&arguments[1])?;
    let operation = validate_operation(&raw)?;
    if operation.operation_id != arguments[1] {
        return Err(CliFailure::new(
            "hotspot_operation_response_mismatch",
            "Fleet Hub lookup returned another operation",
        ));
    }
    Ok(raw)
}

fn validate_operation(raw: &serde_json::Value) -> Result<WindowsHotspotOperation, CliFailure> {
    let operation =
        serde_json::from_value::<WindowsHotspotOperation>(raw.clone()).map_err(|error| {
            CliFailure::new("malformed_hotspot_operation_response", error.to_string())
        })?;
    operation.validate().map_err(|failures| {
        CliFailure::new(
            "invalid_hotspot_operation_response",
            format!(
                "Fleet Hub hotspot contract rejected: {}",
                violation_summary(&failures)
            ),
        )
    })?;
    Ok(operation)
}
