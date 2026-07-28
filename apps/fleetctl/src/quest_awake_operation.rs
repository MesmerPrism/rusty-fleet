// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use fleet_contracts::{
    QUEST_AWAKE_ACTION_ID, QUEST_AWAKE_EXECUTE_REQUEST_SCHEMA, QUEST_AWAKE_PREVIEW_REQUEST_SCHEMA,
    QuestAwakeAction, QuestAwakeExecuteRequest, QuestAwakeOperation, QuestAwakePreviewRequest,
    ValidateContract,
};

use crate::CliFailure;
use crate::operation::{
    FleetOperationClient, parse_target, validate_identifier, validate_request, violation_summary,
};

pub(crate) fn is_quest_awake_operation_command(command: &str) -> bool {
    matches!(command, "awake-preview" | "awake-execute" | "awake-get")
}

pub(crate) fn execute_quest_awake_operation_command<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    match arguments.first().map(String::as_str) {
        Some("awake-preview") => preview(arguments, client),
        Some("awake-execute") => execute(arguments, client),
        Some("awake-get") => get(arguments, client),
        _ => Err(CliFailure::new(
            "unknown_command",
            "the requested Quest awake command is unknown",
        )),
    }
}

fn preview<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    if arguments.len() < 5 {
        return Err(CliFailure::new(
            "missing_awake_preview_arguments",
            "awake-preview requires ACTION DURATION_MS WATCHDOG_INTERVAL_MS and at least one DEVICE@IDENTITY_REVISION target",
        ));
    }
    if arguments.len() - 4 > 10_000 {
        return Err(CliFailure::new(
            "too_many_targets",
            "awake-preview accepts at most 10,000 targets",
        ));
    }
    let action = parse_action(&arguments[1])?;
    let duration_ms = parse_u32(&arguments[2], "duration_ms")?;
    let watchdog_interval_ms = parse_u32(&arguments[3], "watchdog_interval_ms")?;
    let mut targets = BTreeMap::new();
    for target in &arguments[4..] {
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
    let request = QuestAwakePreviewRequest {
        schema: QUEST_AWAKE_PREVIEW_REQUEST_SCHEMA.to_owned(),
        action_id: QUEST_AWAKE_ACTION_ID.to_owned(),
        action,
        duration_ms,
        watchdog_interval_ms,
        targets,
    };
    validate_request(&request, "invalid_awake_preview_request")?;
    let raw = client.preview_quest_awake(&request)?;
    let operation = validate_operation(&raw)?;
    if operation.action_id != request.action_id
        || operation.preview.action != request.action
        || operation.preview.duration_ms != request.duration_ms
        || operation.preview.watchdog_interval_ms != request.watchdog_interval_ms
        || preview_targets(&operation) != request.targets
    {
        return Err(CliFailure::new(
            "awake_operation_response_mismatch",
            "Fleet Hub preview did not bind the exact awake action, policy, and target identities",
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
            "awake-execute requires exactly OPERATION_ID PREVIEW_ID",
        ));
    }
    let request = QuestAwakeExecuteRequest {
        schema: QUEST_AWAKE_EXECUTE_REQUEST_SCHEMA.to_owned(),
        operation_id: arguments[1].clone(),
        preview_id: arguments[2].clone(),
    };
    validate_request(&request, "invalid_awake_execute_request")?;
    let raw = client.execute_quest_awake(&request)?;
    let operation = validate_operation(&raw)?;
    if operation.operation_id != request.operation_id
        || operation.preview.preview_id != request.preview_id
    {
        return Err(CliFailure::new(
            "awake_operation_response_mismatch",
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
            "awake-get requires exactly OPERATION_ID",
        ));
    }
    validate_identifier(&arguments[1], "operation_id")?;
    let raw = client.get_quest_awake(&arguments[1])?;
    let operation = validate_operation(&raw)?;
    if operation.operation_id != arguments[1] {
        return Err(CliFailure::new(
            "awake_operation_response_mismatch",
            "Fleet Hub lookup returned a different Quest awake operation ID",
        ));
    }
    Ok(raw)
}

fn parse_action(value: &str) -> Result<QuestAwakeAction, CliFailure> {
    match value {
        "status" => Ok(QuestAwakeAction::Status),
        "apply-bounded" => Ok(QuestAwakeAction::ApplyBounded),
        "start-windows-watchdog" => Ok(QuestAwakeAction::StartWindowsWatchdog),
        "start-device-watchdog" => Ok(QuestAwakeAction::StartDeviceWatchdog),
        "stop-watchdogs" => Ok(QuestAwakeAction::StopWatchdogs),
        "restore-normal" => Ok(QuestAwakeAction::RestoreNormal),
        _ => Err(CliFailure::new(
            "unsupported_awake_action",
            format!("awake-preview does not support action {value}"),
        )),
    }
}

fn parse_u32(value: &str, name: &str) -> Result<u32, CliFailure> {
    value.parse::<u32>().map_err(|_| {
        CliFailure::new(
            "invalid_awake_policy",
            format!("{name} must be an unsigned integer"),
        )
    })
}

fn validate_operation(raw: &serde_json::Value) -> Result<QuestAwakeOperation, CliFailure> {
    let operation =
        serde_json::from_value::<QuestAwakeOperation>(raw.clone()).map_err(|error| {
            CliFailure::new("malformed_awake_operation_response", error.to_string())
        })?;
    operation.validate().map_err(|failures| {
        CliFailure::new(
            "invalid_awake_operation_response",
            format!(
                "Fleet Hub Quest awake contract rejected: {}",
                violation_summary(&failures)
            ),
        )
    })?;
    Ok(operation)
}

fn preview_targets(operation: &QuestAwakeOperation) -> BTreeMap<String, u64> {
    operation
        .preview
        .targets
        .iter()
        .map(|target| (target.device_id.clone(), target.identity_revision))
        .collect()
}
