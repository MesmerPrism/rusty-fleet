// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{
    AuthorizationState, CommandLifecycle, EnablementState, FreshnessState, OperationExecuteRequest,
    OperationPreviewRequest, PACKAGE_INSTALL_EXECUTE_REQUEST_SCHEMA,
    PACKAGE_INSTALL_PREVIEW_REQUEST_SCHEMA, PACKAGES_INSTALL_RELEASE_ACTION_ID,
    PackageInstallReleaseExecuteRequest, PackageInstallReleaseOperation,
    PackageInstallReleasePreview, PackageInstallReleasePreviewRequest, PackageInstallStage,
    PackageInstallTargetLedger, PackageInstallTargetPreflight, PackageReleaseReference,
    PackageUpdaterInvocation, PackageUpdaterOwnerContractBinding, ReachabilityState, SupportState,
};
use fleetctl::{CliFailure, FleetOperationClient, execute, execute_with_operation_client};

#[derive(Default)]
struct MockPackageClient {
    preview_request: Option<PackageInstallReleasePreviewRequest>,
    execute_request: Option<PackageInstallReleaseExecuteRequest>,
    operation: Option<PackageInstallReleaseOperation>,
    damage_next_response: bool,
}

impl FleetOperationClient for MockPackageClient {
    fn preview_operation(
        &mut self,
        _request: &OperationPreviewRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        Err(CliFailure::new(
            "unexpected_route",
            "Kiosk route was not expected",
        ))
    }

    fn execute_operation(
        &mut self,
        _request: &OperationExecuteRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        Err(CliFailure::new(
            "unexpected_route",
            "Kiosk route was not expected",
        ))
    }

    fn get_operation(&mut self, _operation_id: &str) -> Result<serde_json::Value, CliFailure> {
        Err(CliFailure::new(
            "unexpected_route",
            "Kiosk route was not expected",
        ))
    }

    fn preview_package_install_release(
        &mut self,
        request: &PackageInstallReleasePreviewRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.preview_request = Some(request.clone());
        self.operation = Some(operation(request, false));
        self.response()
    }

    fn execute_package_install_release(
        &mut self,
        request: &PackageInstallReleaseExecuteRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        let prior = self.operation.as_ref().ok_or_else(|| {
            CliFailure::new("operation_missing", "package preview was not created")
        })?;
        if prior.operation_id != request.operation_id
            || prior.preview.preview_id != request.preview_id
        {
            return Err(CliFailure::new(
                "operation_mismatch",
                "package execute request did not bind the preview",
            ));
        }
        self.execute_request = Some(request.clone());
        let preview = self.preview_request.as_ref().expect("preview request");
        self.operation = Some(operation(preview, true));
        self.response()
    }

    fn get_package_install_release(
        &mut self,
        operation_id: &str,
    ) -> Result<serde_json::Value, CliFailure> {
        if self
            .operation
            .as_ref()
            .is_none_or(|operation| operation.operation_id != operation_id)
        {
            return Err(CliFailure::new(
                "operation_missing",
                "package operation was not found",
            ));
        }
        self.response()
    }
}

impl MockPackageClient {
    fn response(&mut self) -> Result<serde_json::Value, CliFailure> {
        let mut value = serde_json::to_value(
            self.operation
                .as_ref()
                .expect("mock package operation exists"),
        )
        .expect("serialize package operation");
        if self.damage_next_response {
            self.damage_next_response = false;
            value["targets"][0]["identity_revision"] = serde_json::json!(999);
        }
        Ok(value)
    }
}

fn operation(
    request: &PackageInstallReleasePreviewRequest,
    prepared: bool,
) -> PackageInstallReleaseOperation {
    let created_at_ms = 1_000;
    let expires_at_ms = 61_000;
    let operation_id = "package-operation-1";
    let preview_id = "package-preview-1";
    let preflights = request
        .targets
        .iter()
        .enumerate()
        .map(
            |(index, (device_id, identity_revision))| PackageInstallTargetPreflight {
                device_id: device_id.clone(),
                identity_revision: *identity_revision,
                capability_id: "rusty-quest.package-updater".to_owned(),
                capability_evidence_revision: 10 + index as u64,
                capability_owner: "rusty-quest".to_owned(),
                support: SupportState::Supported,
                enablement: EnablementState::Enabled,
                authorization: AuthorizationState::Authorized,
                reachability: ReachabilityState::Reachable,
                freshness: FreshnessState::Current,
                observed_at_ms: 900,
                fresh_until_ms: 30_000,
                evaluated_at_ms: 1_001,
                eligible: true,
                reason_code: "ready".to_owned(),
                message: "Attended package updater is current and ready.".to_owned(),
            },
        )
        .collect::<Vec<_>>();
    let targets = preflights
        .iter()
        .enumerate()
        .map(|(index, preflight)| PackageInstallTargetLedger {
            device_id: preflight.device_id.clone(),
            identity_revision: preflight.identity_revision,
            preflight: preflight.clone(),
            lifecycle: if prepared {
                CommandLifecycle::Accepted
            } else {
                CommandLifecycle::Proposed
            },
            stage: if prepared {
                PackageInstallStage::DispatchReady
            } else {
                PackageInstallStage::PreviewReady
            },
            invocation: prepared.then(|| PackageUpdaterInvocation {
                schema: "rusty.fleet.package_updater_invocation.v1".to_owned(),
                operation_id: operation_id.to_owned(),
                preview_id: preview_id.to_owned(),
                device_id: preflight.device_id.clone(),
                identity_revision: preflight.identity_revision,
                owner_action_request_id: format!("package-owner-{}", index + 1),
                release: request.release.clone(),
                expected_package_name: request.expected_package_name.clone(),
                expected_rollout_ring: request.expected_rollout_ring.clone(),
                expires_at_ms,
            }),
            owner_claim: None,
            prior_owner_claims: Vec::new(),
            consumed_owner_claim_identities: Vec::new(),
            invocation_acknowledgement: None,
            effective_receipt: None,
            reason_code: if prepared {
                "owner_dispatch_ready".to_owned()
            } else {
                "preview_ready".to_owned()
            },
            message: if prepared {
                "Exact updater invocation is ready for delivery; application remains unproven"
                    .to_owned()
            } else {
                "Target is ready for explicit confirmation".to_owned()
            },
            last_transition_ms: if prepared { 1_002 } else { 1_000 },
        })
        .collect::<Vec<_>>();
    PackageInstallReleaseOperation {
        schema: "rusty.fleet.package_install_release_operation.v1".to_owned(),
        operation_id: operation_id.to_owned(),
        action_id: PACKAGES_INSTALL_RELEASE_ACTION_ID.to_owned(),
        created_at_ms,
        preview: PackageInstallReleasePreview {
            schema: "rusty.fleet.package_install_release_preview.v1".to_owned(),
            preview_id: preview_id.to_owned(),
            operation_id: operation_id.to_owned(),
            action_id: PACKAGES_INSTALL_RELEASE_ACTION_ID.to_owned(),
            created_at_ms,
            expires_at_ms,
            fleet_revision: 7,
            release: request.release.clone(),
            expected_package_name: request.expected_package_name.clone(),
            expected_rollout_ring: request.expected_rollout_ring.clone(),
            owner_contract: PackageUpdaterOwnerContractBinding::attended_v1(),
            targets: preflights,
        },
        lifecycle: if prepared {
            CommandLifecycle::Accepted
        } else {
            CommandLifecycle::Proposed
        },
        max_parallelism: 1,
        cleanup_required: false,
        targets,
    }
}

fn preview_arguments() -> Vec<String> {
    [
        "package-preview",
        "manifest-url",
        "https://updates.example.invalid/labs/envelope.json",
        "org.example.kiosk",
        "labs",
        "device.quest.1@7",
        "device.quest.2@8",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[test]
fn package_cli_preserves_preview_execute_status_and_owner_boundary() {
    let mut client = MockPackageClient::default();
    let preview =
        execute_with_operation_client(preview_arguments(), &mut client).expect("package preview");
    let request = client.preview_request.as_ref().expect("preview request");
    assert_eq!(request.schema, PACKAGE_INSTALL_PREVIEW_REQUEST_SCHEMA);
    assert_eq!(request.targets.len(), 2);
    assert_eq!(preview["lifecycle"], "proposed");

    let executed = execute_with_operation_client(
        vec![
            "package-execute".to_owned(),
            "package-operation-1".to_owned(),
            "package-preview-1".to_owned(),
        ],
        &mut client,
    )
    .expect("package execute");
    assert_eq!(
        client
            .execute_request
            .as_ref()
            .expect("execute request")
            .schema,
        PACKAGE_INSTALL_EXECUTE_REQUEST_SCHEMA
    );
    assert_eq!(executed["lifecycle"], "accepted");
    assert_eq!(executed["max_parallelism"], 1);
    assert_eq!(executed["targets"].as_array().expect("targets").len(), 2);
    for target in executed["targets"].as_array().expect("targets") {
        assert_eq!(target["stage"], "dispatch_ready");
        assert_eq!(target["lifecycle"], "accepted");
        assert!(target["invocation"].is_object());
        assert!(target["invocation_acknowledgement"].is_null());
        assert!(target["effective_receipt"].is_null());
    }

    let status = execute_with_operation_client(
        vec!["package-get".to_owned(), "package-operation-1".to_owned()],
        &mut client,
    )
    .expect("package get");
    assert_eq!(status, executed);
}

#[test]
fn package_cli_rejects_ambiguous_input_and_damaged_hub_evidence() {
    assert_eq!(
        execute(preview_arguments())
            .expect_err("live client required")
            .code,
        "operation_client_required"
    );

    let mut client = MockPackageClient::default();
    let mut unknown = preview_arguments();
    unknown[1] = "ambient".to_owned();
    assert_eq!(
        execute_with_operation_client(unknown, &mut client)
            .expect_err("unknown release kind")
            .code,
        "unsupported_release_kind"
    );

    let mut duplicate = preview_arguments();
    duplicate[6] = duplicate[5].clone();
    assert_eq!(
        execute_with_operation_client(duplicate, &mut client)
            .expect_err("duplicate target")
            .code,
        "duplicate_target"
    );

    client.damage_next_response = true;
    assert_eq!(
        execute_with_operation_client(preview_arguments(), &mut client)
            .expect_err("damaged Hub response")
            .code,
        "invalid_package_operation_response"
    );
}

#[test]
fn package_release_id_is_explicit_and_not_inferred_from_a_url() {
    let mut client = MockPackageClient::default();
    let arguments = [
        "package-preview",
        "release-id",
        "alpha/release-15",
        "org.example.kiosk",
        "labs",
        "device.quest.1@7",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    execute_with_operation_client(arguments, &mut client).expect("release ID preview");
    assert_eq!(
        client
            .preview_request
            .as_ref()
            .expect("preview request")
            .release,
        PackageReleaseReference::ReleaseId {
            release_id: "alpha/release-15".to_owned()
        }
    );
}
