// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use fleet_contracts::{
    PACKAGE_INSTALL_EXECUTE_REQUEST_SCHEMA, PACKAGE_INSTALL_PREVIEW_REQUEST_SCHEMA,
    PACKAGES_INSTALL_RELEASE_ACTION_ID, PackageInstallReleaseExecuteRequest,
    PackageInstallReleaseOperation, PackageInstallReleasePreviewRequest, PackageReleaseReference,
    ValidateContract,
};

use crate::CliFailure;
use crate::operation::{
    FleetOperationClient, parse_target, validate_identifier, validate_request, violation_summary,
};

pub(crate) fn is_package_operation_command(command: &str) -> bool {
    matches!(
        command,
        "package-preview" | "package-execute" | "package-get"
    )
}

pub(crate) fn execute_package_operation_command<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    match arguments.first().map(String::as_str) {
        Some("package-preview") => preview(arguments, client),
        Some("package-execute") => execute(arguments, client),
        Some("package-get") => get(arguments, client),
        _ => Err(CliFailure::new(
            "unknown_command",
            "the requested package operation command is unknown",
        )),
    }
}

fn preview<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    if arguments.len() < 6 {
        return Err(CliFailure::new(
            "missing_package_preview_arguments",
            "package-preview requires RELEASE_KIND RELEASE PACKAGE RING and at least one DEVICE@IDENTITY_REVISION target",
        ));
    }
    if arguments.len() - 5 > 10_000 {
        return Err(CliFailure::new(
            "too_many_targets",
            "package-preview accepts at most 10,000 targets",
        ));
    }
    let release = match arguments[1].as_str() {
        "manifest-url" => PackageReleaseReference::ManifestUrl {
            manifest_url: arguments[2].clone(),
        },
        "release-id" => PackageReleaseReference::ReleaseId {
            release_id: arguments[2].clone(),
        },
        kind => {
            return Err(CliFailure::new(
                "unsupported_release_kind",
                format!("package-preview does not support release kind {kind}"),
            ));
        }
    };
    let mut targets = BTreeMap::new();
    for target in &arguments[5..] {
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
    let request = PackageInstallReleasePreviewRequest {
        schema: PACKAGE_INSTALL_PREVIEW_REQUEST_SCHEMA.to_owned(),
        action_id: PACKAGES_INSTALL_RELEASE_ACTION_ID.to_owned(),
        release,
        expected_package_name: arguments[3].clone(),
        expected_rollout_ring: arguments[4].clone(),
        targets,
    };
    validate_request(&request, "invalid_package_preview_request")?;
    let raw = client.preview_package_install_release(&request)?;
    let operation = validate_operation(&raw)?;
    if operation.action_id != request.action_id
        || operation.preview.release != request.release
        || operation.preview.expected_package_name != request.expected_package_name
        || operation.preview.expected_rollout_ring != request.expected_rollout_ring
        || preview_targets(&operation) != request.targets
    {
        return Err(CliFailure::new(
            "package_operation_response_mismatch",
            "Fleet Hub preview did not bind the exact release, package, ring, and target identities",
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
            "package-execute requires exactly OPERATION_ID PREVIEW_ID",
        ));
    }
    let request = PackageInstallReleaseExecuteRequest {
        schema: PACKAGE_INSTALL_EXECUTE_REQUEST_SCHEMA.to_owned(),
        operation_id: arguments[1].clone(),
        preview_id: arguments[2].clone(),
    };
    validate_request(&request, "invalid_package_execute_request")?;
    let raw = client.execute_package_install_release(&request)?;
    let operation = validate_operation(&raw)?;
    if operation.operation_id != request.operation_id
        || operation.preview.preview_id != request.preview_id
    {
        return Err(CliFailure::new(
            "package_operation_response_mismatch",
            "Fleet Hub execution did not bind the requested package operation and immutable preview",
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
            "package-get requires exactly OPERATION_ID",
        ));
    }
    validate_identifier(&arguments[1], "operation_id")?;
    let raw = client.get_package_install_release(&arguments[1])?;
    let operation = validate_operation(&raw)?;
    if operation.operation_id != arguments[1] {
        return Err(CliFailure::new(
            "package_operation_response_mismatch",
            "Fleet Hub lookup returned a different package operation ID",
        ));
    }
    Ok(raw)
}

fn validate_operation(
    raw: &serde_json::Value,
) -> Result<PackageInstallReleaseOperation, CliFailure> {
    let operation =
        serde_json::from_value::<PackageInstallReleaseOperation>(raw.clone()).map_err(|error| {
            CliFailure::new("malformed_package_operation_response", error.to_string())
        })?;
    operation.validate().map_err(|failures| {
        CliFailure::new(
            "invalid_package_operation_response",
            format!(
                "Fleet Hub package operation contract rejected: {}",
                violation_summary(&failures)
            ),
        )
    })?;
    Ok(operation)
}

fn preview_targets(operation: &PackageInstallReleaseOperation) -> BTreeMap<String, u64> {
    operation
        .preview
        .targets
        .iter()
        .map(|target| (target.device_id.clone(), target.identity_revision))
        .collect()
}
