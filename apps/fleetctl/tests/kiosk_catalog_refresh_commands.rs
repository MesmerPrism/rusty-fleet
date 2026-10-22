// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{
    KioskV2CatalogRefreshRequest, OperationExecuteRequest, OperationPreviewRequest,
};
use fleetctl::{CliFailure, FleetOperationClient, execute_with_operation_client};

#[derive(Default)]
struct RecordingClient {
    request: Option<KioskV2CatalogRefreshRequest>,
}

impl FleetOperationClient for RecordingClient {
    fn search_kiosk_apps(
        &mut self,
        request: &fleet_contracts::KioskAppSearchRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        let hub = fleet_hub::FleetHub::new(fleet_hub::HubPolicy::default());
        Ok(serde_json::to_value(
            hub.kiosk_app_search(request, 1_900_000_000_000)
                .expect("canonical engine search"),
        )
        .expect("projection"))
    }
    fn kiosk_app_candidates(&mut self, identity: &str) -> Result<serde_json::Value, CliFailure> {
        let hub = fleet_hub::FleetHub::new(fleet_hub::HubPolicy::default());
        Ok(serde_json::to_value(
            hub.kiosk_app_candidates(identity, 1_900_000_000_000)
                .expect("canonical candidates"),
        )
        .expect("projection"))
    }

    fn preview_operation(
        &mut self,
        _request: &OperationPreviewRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        unreachable!()
    }

    fn execute_operation(
        &mut self,
        _request: &OperationExecuteRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        unreachable!()
    }

    fn get_operation(&mut self, _operation_id: &str) -> Result<serde_json::Value, CliFailure> {
        unreachable!()
    }

    fn refresh_kiosk_catalog(
        &mut self,
        request: &KioskV2CatalogRefreshRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.request = Some(request.clone());
        Ok(serde_json::json!({
            "schema": "rusty.fleet.kiosk_v2_catalog_refresh_projection.v1",
            "admitted": true,
            "device_id": request.device_id,
            "identity_revision": request.identity_revision,
            "kiosk_owner_epoch": "KioskOwnerEpoch_001",
            "catalog_revision": "a".repeat(64),
            "snapshot_digest": "b".repeat(64),
            "replay_identity_sha256": "c".repeat(64),
            "grant_revision": 1,
            "entry_count": 1,
            "observed_at_ms": 1_000,
            "fresh_until_ms": 301_000,
            "scopes": ["kiosk.catalog-summary"],
            "fleet_revision": 2
        }))
    }
}

#[test]
fn refresh_cli_sends_only_the_exact_selected_device_identity() {
    let mut client = RecordingClient::default();
    execute_with_operation_client(
        ["kiosk-catalog-refresh", "device.quest.1@7"]
            .map(str::to_owned)
            .to_vec(),
        &mut client,
    )
    .expect("refresh projection");
    let request = client.request.expect("recorded request");
    assert_eq!(request.device_id, "device.quest.1");
    assert_eq!(request.identity_revision, 7);
    assert_eq!(
        serde_json::to_value(request).expect("request JSON"),
        serde_json::json!({
            "schema": "rusty.fleet.kiosk_v2_catalog_refresh_request.v1",
            "device_id": "device.quest.1",
            "identity_revision": 7
        })
    );
}

#[test]
fn refresh_cli_rejects_caller_owner_or_lifetime_arguments() {
    let mut client = RecordingClient::default();
    let error = execute_with_operation_client(
        [
            "kiosk-catalog-refresh",
            "device.quest.1@7",
            "KioskOwnerEpoch_001",
        ]
        .map(str::to_owned)
        .to_vec(),
        &mut client,
    )
    .expect_err("caller owner input must be rejected");
    assert_eq!(error.code, "unexpected_arguments");
    assert!(client.request.is_none());
}

#[test]
fn search_and_candidates_cli_preserve_canonical_engine_json() {
    let mut client = RecordingClient::default();
    let hub = fleet_hub::FleetHub::new(fleet_hub::HubPolicy::default());
    let request = fleet_contracts::KioskAppSearchRequest {
        schema: fleet_contracts::KIOSK_APP_SEARCH_REQUEST_SCHEMA.to_owned(),
        query: " Research ".to_owned(),
        limit: 128,
    };
    let result = execute_with_operation_client(
        vec!["kiosk-apps-search".to_owned(), request.query.clone()],
        &mut client,
    )
    .expect("search");
    assert_eq!(
        result,
        serde_json::to_value(
            hub.kiosk_app_search(&request, 1_900_000_000_000)
                .expect("engine")
        )
        .expect("projection")
    );
    let identity = "a".repeat(64);
    let result = execute_with_operation_client(
        vec!["kiosk-apps-candidates".to_owned(), identity.clone()],
        &mut client,
    )
    .expect("candidates");
    assert_eq!(
        result,
        serde_json::to_value(
            hub.kiosk_app_candidates(&identity, 1_900_000_000_000)
                .expect("engine")
        )
        .expect("projection")
    );
    for launch in [
        "kiosk-app-launch-preview",
        "kiosk-app-launch-execute",
        "kiosk-app-launch-get",
    ] {
        assert!(!fleetctl::is_operation_command(launch));
    }
}
