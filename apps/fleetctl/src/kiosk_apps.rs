// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::{CliFailure, FleetOperationClient};
use fleet_contracts::{
    KIOSK_APP_CANDIDATES_RESULT_SCHEMA, KIOSK_APP_SEARCH_REQUEST_SCHEMA,
    KIOSK_APP_SEARCH_RESULT_SCHEMA, KIOSK_V2_CATALOG_REFRESH_PROJECTION_SCHEMA,
    KIOSK_V2_CATALOG_REFRESH_REQUEST_SCHEMA, KioskAppCandidatesResult, KioskAppSearchRequest,
    KioskAppSearchResult, KioskV2CatalogRefreshProjection, KioskV2CatalogRefreshRequest,
    ValidateContract,
};

pub(crate) fn is_kiosk_app_command(command: &str) -> bool {
    matches!(
        command,
        "kiosk-apps-search" | "kiosk-apps-candidates" | "kiosk-catalog-refresh"
    )
}

pub(crate) fn execute_kiosk_app_command<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    match arguments.first().map(String::as_str) {
        Some("kiosk-apps-search") => search(arguments, client),
        Some("kiosk-apps-candidates") => candidates(arguments, client),
        Some("kiosk-catalog-refresh") => refresh_catalog(arguments, client),
        _ => Err(CliFailure::new(
            "unknown_command",
            "the requested Kiosk app-finder command is unknown",
        )),
    }
}

fn refresh_catalog<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    if arguments.len() != 2 {
        return Err(CliFailure::new(
            "unexpected_arguments",
            "kiosk-catalog-refresh requires DEVICE@IDENTITY_REVISION",
        ));
    }
    let (device_id, identity_revision) = crate::operation::parse_target(&arguments[1])?;
    let request = KioskV2CatalogRefreshRequest {
        schema: KIOSK_V2_CATALOG_REFRESH_REQUEST_SCHEMA.to_owned(),
        device_id,
        identity_revision,
    };
    request.validate().map_err(|failures| {
        CliFailure::new(
            "invalid_kiosk_catalog_refresh",
            failures
                .into_iter()
                .map(|failure| format!("{}@{}", failure.code, failure.path))
                .collect::<Vec<_>>()
                .join(","),
        )
    })?;
    let raw = client.refresh_kiosk_catalog(&request)?;
    let projection: KioskV2CatalogRefreshProjection =
        serde_json::from_value(raw.clone()).map_err(|_| {
            CliFailure::new(
                "invalid_kiosk_catalog_refresh_response",
                "Fleet Hub returned a malformed catalog refresh projection",
            )
        })?;
    projection.validate().map_err(|failures| {
        CliFailure::new(
            "invalid_kiosk_catalog_refresh_response",
            failures
                .into_iter()
                .map(|failure| format!("{}@{}", failure.code, failure.path))
                .collect::<Vec<_>>()
                .join(","),
        )
    })?;
    if projection.schema != KIOSK_V2_CATALOG_REFRESH_PROJECTION_SCHEMA
        || projection.device_id != request.device_id
        || projection.identity_revision != request.identity_revision
    {
        return Err(CliFailure::new(
            "kiosk_catalog_refresh_response_mismatch",
            "Fleet Hub catalog refresh did not bind the exact requested device identity",
        ));
    }
    Ok(raw)
}

fn search<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    if arguments.len() != 2 {
        return Err(CliFailure::new(
            "unexpected_arguments",
            "kiosk-apps-search requires exactly one app name, package text, or tag query",
        ));
    }
    let request = KioskAppSearchRequest {
        schema: KIOSK_APP_SEARCH_REQUEST_SCHEMA.to_owned(),
        query: arguments[1].clone(),
        limit: 128,
    };
    request.validate().map_err(|failures| {
        CliFailure::new(
            "invalid_kiosk_app_search",
            failures
                .into_iter()
                .map(|failure| failure.code)
                .collect::<Vec<_>>()
                .join(","),
        )
    })?;
    let raw = client.search_kiosk_apps(&request)?;
    let result: KioskAppSearchResult = serde_json::from_value(raw.clone()).map_err(|error| {
        CliFailure::new(
            "invalid_kiosk_app_search_response",
            format!("Fleet Hub returned an invalid app-search projection: {error}"),
        )
    })?;
    if result.schema != KIOSK_APP_SEARCH_RESULT_SCHEMA
        || result.query != request.query
        || result.matches.len() > request.limit
    {
        return Err(CliFailure::new(
            "kiosk_app_search_response_mismatch",
            "Fleet Hub app-search response did not bind the exact query and bounds",
        ));
    }
    Ok(raw)
}

fn candidates<C: FleetOperationClient + ?Sized>(
    arguments: &[String],
    client: &mut C,
) -> Result<serde_json::Value, CliFailure> {
    if arguments.len() != 2 || arguments[1].is_empty() || arguments[1].len() > 160 {
        return Err(CliFailure::new(
            "unexpected_arguments",
            "kiosk-apps-candidates requires one exact bounded Kiosk app identity",
        ));
    }
    let raw = client.kiosk_app_candidates(&arguments[1])?;
    let result: KioskAppCandidatesResult =
        serde_json::from_value(raw.clone()).map_err(|error| {
            CliFailure::new(
                "invalid_kiosk_app_candidates_response",
                format!("Fleet Hub returned invalid app candidates: {error}"),
            )
        })?;
    if result.schema != KIOSK_APP_CANDIDATES_RESULT_SCHEMA || result.app_identity != arguments[1] {
        return Err(CliFailure::new(
            "kiosk_app_candidates_response_mismatch",
            "Fleet Hub candidates did not bind the requested exact app identity",
        ));
    }
    Ok(raw)
}
