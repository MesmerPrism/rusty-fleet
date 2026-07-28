// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use fleet_contracts::{
    QUEST_WIFI_ADB_ACTION_ID, QUEST_WIFI_ADB_EXECUTE_REQUEST_SCHEMA,
    QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA, QuestWifiAdbAction, QuestWifiAdbExecuteRequest,
    QuestWifiAdbOperation, QuestWifiAdbPreviewRequest, ValidateContract,
};

use crate::CliFailure;
use crate::operation::{
    FleetOperationClient, parse_target, validate_identifier, validate_request, violation_summary,
};

pub(crate) fn is_quest_wifi_adb_operation_command(command: &str) -> bool {
    matches!(
        command,
        "wifi-adb-preview" | "wifi-adb-execute" | "wifi-adb-get" | "wifi-adb-list"
    )
}

pub(crate) fn execute_quest_wifi_adb_operation_command<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    match arguments.first().map(String::as_str) {
        Some("wifi-adb-preview") => preview(arguments, client),
        Some("wifi-adb-execute") => execute(arguments, client),
        Some("wifi-adb-get") => get(arguments, client),
        Some("wifi-adb-list") => list(arguments, client),
        _ => Err(CliFailure::new(
            "unknown_command",
            "the requested Quest Wi-Fi ADB command is unknown",
        )),
    }
}

fn list<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    if arguments.len() != 1 {
        return Err(CliFailure::new(
            "unexpected_arguments",
            "wifi-adb-list accepts no arguments",
        ));
    }
    let raw = client.list_quest_wifi_adb()?;
    let values = raw.as_array().ok_or_else(|| {
        CliFailure::new(
            "malformed_wifi_adb_operation_set_response",
            "Fleet Hub operation set must be a JSON array",
        )
    })?;
    let mut prior_id: Option<String> = None;
    for value in values {
        let operation = validate_operation(value)?;
        if prior_id
            .as_deref()
            .is_some_and(|prior| prior >= operation.operation_id.as_str())
        {
            return Err(CliFailure::new(
                "invalid_wifi_adb_operation_set_response",
                "Fleet Hub operation set must be uniquely sorted by operation ID",
            ));
        }
        prior_id = Some(operation.operation_id);
    }
    Ok(raw)
}

fn preview<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    if arguments.len() < 3 {
        return Err(CliFailure::new(
            "missing_wifi_adb_preview_arguments",
            "wifi-adb-preview requires ACTION and at least one DEVICE@IDENTITY_REVISION target",
        ));
    }
    if arguments.len() - 2 > 10_000 {
        return Err(CliFailure::new(
            "too_many_targets",
            "wifi-adb-preview accepts at most 10,000 targets",
        ));
    }
    let action = parse_action(&arguments[1])?;
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
    let request = QuestWifiAdbPreviewRequest {
        schema: QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA.to_owned(),
        action_id: QUEST_WIFI_ADB_ACTION_ID.to_owned(),
        action,
        targets,
    };
    validate_request(&request, "invalid_wifi_adb_preview_request")?;
    let raw = client.preview_quest_wifi_adb(&request)?;
    let operation = validate_operation(&raw)?;
    if operation.action_id != request.action_id
        || operation.preview.action != request.action
        || preview_targets(&operation) != request.targets
    {
        return Err(CliFailure::new(
            "wifi_adb_operation_response_mismatch",
            "Fleet Hub preview did not bind the exact Wi-Fi ADB action and target identities",
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
            "wifi-adb-execute requires exactly OPERATION_ID PREVIEW_ID",
        ));
    }
    let request = QuestWifiAdbExecuteRequest {
        schema: QUEST_WIFI_ADB_EXECUTE_REQUEST_SCHEMA.to_owned(),
        operation_id: arguments[1].clone(),
        preview_id: arguments[2].clone(),
    };
    validate_request(&request, "invalid_wifi_adb_execute_request")?;
    let raw = client.execute_quest_wifi_adb(&request)?;
    let operation = validate_operation(&raw)?;
    if operation.operation_id != request.operation_id
        || operation.preview.preview_id != request.preview_id
    {
        return Err(CliFailure::new(
            "wifi_adb_operation_response_mismatch",
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
            "wifi-adb-get requires exactly OPERATION_ID",
        ));
    }
    validate_identifier(&arguments[1], "operation_id")?;
    let raw = client.get_quest_wifi_adb(&arguments[1])?;
    let operation = validate_operation(&raw)?;
    if operation.operation_id != arguments[1] {
        return Err(CliFailure::new(
            "wifi_adb_operation_response_mismatch",
            "Fleet Hub lookup returned a different Quest Wi-Fi ADB operation ID",
        ));
    }
    Ok(raw)
}

fn parse_action(value: &str) -> Result<QuestWifiAdbAction, CliFailure> {
    match value {
        "status" => Ok(QuestWifiAdbAction::Status),
        "request-wireless-adb" => Ok(QuestWifiAdbAction::RequestWirelessAdb),
        "enable-request-after-boot" => Ok(QuestWifiAdbAction::EnableRequestAfterBoot),
        "disable-request-after-boot" => Ok(QuestWifiAdbAction::DisableRequestAfterBoot),
        "disable-wireless-adb" => Ok(QuestWifiAdbAction::DisableWirelessAdb),
        "enable-classic-tcpip-from-usb" => Ok(QuestWifiAdbAction::EnableClassicTcpipFromUsb),
        _ => Err(CliFailure::new(
            "unsupported_wifi_adb_action",
            format!("wifi-adb-preview does not support action {value}"),
        )),
    }
}

fn validate_operation(raw: &serde_json::Value) -> Result<QuestWifiAdbOperation, CliFailure> {
    let operation =
        serde_json::from_value::<QuestWifiAdbOperation>(raw.clone()).map_err(|error| {
            CliFailure::new("malformed_wifi_adb_operation_response", error.to_string())
        })?;
    operation.validate().map_err(|failures| {
        CliFailure::new(
            "invalid_wifi_adb_operation_response",
            format!(
                "Fleet Hub Quest Wi-Fi ADB contract rejected: {}",
                violation_summary(&failures)
            ),
        )
    })?;
    Ok(operation)
}

fn preview_targets(operation: &QuestWifiAdbOperation) -> BTreeMap<String, u64> {
    operation
        .preview
        .targets
        .iter()
        .map(|target| (target.device_id.clone(), target.identity_revision))
        .collect()
}
