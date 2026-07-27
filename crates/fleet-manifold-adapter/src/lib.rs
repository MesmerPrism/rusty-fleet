// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Authenticated low-rate check-in admission through exact Manifold authority.

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signature, VerifyingKey};
use fleet_contracts::{
    AuthorizationState, ConditionFamily, ConditionState, EnablementState, FreshnessState,
    PACKAGES_INSTALL_RELEASE_ACTION_ID, PackageReleaseReference, QUEST_AWAKE_ACTION_ID,
    QUEST_WIFI_ADB_ACTION_ID, QUEST_WIFI_ADB_TERMUX_CAPABILITY_ID,
    QUEST_WIFI_ADB_TERMUX_PROOF_OWNER, QUEST_WIFI_ADB_TERMUX_PROOF_SCHEMA, QuestAwakeAction,
    QuestWifiAdbAction, QuestWifiAdbOperation, QuestWifiAdbRouteMode, QuestWifiAdbTermuxProof,
    ReachabilityState, Sensitivity, SignedFleetCheckIn, StatusCondition, StatusSource,
    SupportState, TERMUX_ADB_SHELL_IDENTITY, ValidateContract,
};
use fleet_hub::{FleetApi, FleetHub, ObservationDecision};
use rusty_manifold_model::{DottedId, Revision};
use rusty_manifold_peer::{
    ManifoldAcceptedPeerState, ManifoldPeerApplicationReceipt, ManifoldPeerCredentialStatus,
    ManifoldPeerDecision, ManifoldPeerDecisionOutcome, ManifoldPeerEnrollmentReceipt,
    ManifoldPeerEnrollmentRequest, ManifoldPeerEnrollmentState, ManifoldPeerReviewCase,
    ManifoldPeerStatusProposal, review_and_apply_peer_enrollment, review_and_apply_peer_proposal,
};
use rusty_manifold_runtime_host::{
    HOST_COMMAND_REQUEST_SCHEMA, HOST_SNAPSHOT_SCHEMA, HOST_TYPED_PARAMS_DIGEST_SCHEMA,
    ManifoldRuntimeApplicationReceipt, ManifoldRuntimeCommandDescriptor,
    ManifoldRuntimeCommandRequest, ManifoldRuntimeDispatchOutcome, ManifoldRuntimeDispatchReceipt,
    ManifoldRuntimeHost, ManifoldRuntimeHostSnapshot, ManifoldRuntimeTypedParamsDigest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_SEEN_CHECKINS: usize = 10_000;
const FLEET_RUNTIME_HOST_ID: &str = "runtime.fleet.hub";
const KIOSK_SHOW_CONTROLS_COMMAND_ID: &str = "kiosk.show-controls";
const KIOSK_SHOW_CONTROLS_PARAMS_TYPE_ID: &str = "rusty.fleet.kiosk.show-controls.params.v1";
const PACKAGES_INSTALL_RELEASE_COMMAND_ID: &str = PACKAGES_INSTALL_RELEASE_ACTION_ID;
const PACKAGES_INSTALL_RELEASE_PARAMS_TYPE_ID: &str =
    "rusty.fleet.packages.install-release.params.v1";
const QUEST_AWAKE_COMMAND_ID: &str = QUEST_AWAKE_ACTION_ID;
const QUEST_AWAKE_PARAMS_TYPE_ID: &str = "rusty.fleet.quest.awake-control.params.v1";
const QUEST_WIFI_ADB_COMMAND_ID: &str = QUEST_WIFI_ADB_ACTION_ID;
const QUEST_WIFI_ADB_PARAMS_TYPE_ID: &str = "rusty.fleet.quest.wifi-adb-control.params.v1";
const FLEET_MANIFOLD_SNAPSHOT_V1_SCHEMA: &str = "rusty.fleet.manifold_adapter_snapshot.v1";
const FLEET_MANIFOLD_SNAPSHOT_V2_SCHEMA: &str = "rusty.fleet.manifold_adapter_snapshot.v2";
const FLEET_MANIFOLD_SNAPSHOT_SCHEMA: &str = "rusty.fleet.manifold_adapter_snapshot.v3";
const MAX_KIOSK_COMMAND_LIFETIME_MS: u64 = 90_000;
const MAX_PACKAGE_COMMAND_LIFETIME_MS: u64 = 15 * 60_000;
const MAX_QUEST_AWAKE_COMMAND_LIFETIME_MS: u64 = 15 * 60_000;
const MAX_QUEST_WIFI_ADB_COMMAND_LIFETIME_MS: u64 = 15 * 60_000;

#[must_use]
pub fn kiosk_manifold_request_id(
    operation_id: &str,
    device_id: &str,
    owner_action_request_id: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.kiosk.manifold-request.v1\0");
    for value in [operation_id, device_id, owner_action_request_id] {
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    format!("request.fleet.kiosk.{}", hex::encode(digest.finalize()))
}

#[must_use]
pub fn package_manifold_request_id(
    operation_id: &str,
    device_id: &str,
    owner_action_request_id: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.packages.manifold-request.v1\0");
    for value in [operation_id, device_id, owner_action_request_id] {
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    format!("request.fleet.packages.{}", hex::encode(digest.finalize()))
}

#[must_use]
pub fn quest_awake_manifold_request_id(
    operation_id: &str,
    device_id: &str,
    owner_action_request_id: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.quest-awake.manifold-request.v1\0");
    for value in [operation_id, device_id, owner_action_request_id] {
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    format!(
        "request.fleet.quest-awake.{}",
        hex::encode(digest.finalize())
    )
}

#[must_use]
pub fn quest_wifi_adb_manifold_request_id(
    operation_id: &str,
    device_id: &str,
    owner_action_request_id: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.quest-wifi-adb.manifold-request.v1\0");
    for value in [operation_id, device_id, owner_action_request_id] {
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    format!(
        "request.fleet.quest-wifi-adb.{}",
        hex::encode(digest.finalize())
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckInRejectionReason {
    ContractInvalid,
    StaleOrFuture,
    Replay,
    UnknownOrInactiveKey,
    KeyOutsideValidity,
    IdentityMismatch,
    AuthorityEvidenceMismatch,
    SignatureInvalid,
    ManifoldRejected,
    FleetRejected,
    EvidenceLimitExceeded,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckInReceipt {
    pub schema: String,
    pub checkin_id: String,
    pub accepted: bool,
    pub rejection_reason: Option<CheckInRejectionReason>,
    pub manifold_decision: Option<ManifoldPeerDecision>,
    pub manifold_application: Option<ManifoldPeerApplicationReceipt>,
    pub fleet_decision: Option<ObservationDecision>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FleetManifoldAdapterSnapshot {
    schema: String,
    accepted_peers: ManifoldAcceptedPeerState,
    seen_checkins: BTreeMap<String, i64>,
    #[serde(default)]
    runtime_host: Option<ManifoldRuntimeHostSnapshot>,
    #[serde(default)]
    termux_proofs: BTreeMap<String, AdmittedQuestWifiAdbTermuxProof>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct AdmittedQuestWifiAdbTermuxProof {
    proof: QuestWifiAdbTermuxProof,
    checkin_id: String,
    checkin_expires_at_ms: i64,
    accepted_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KioskShowControlsCommandAuthorization {
    pub manifold_request_id: String,
    pub owner_action_request_id: String,
    pub requester_id: String,
    pub operation_id: String,
    pub preview_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KioskShowControlsAuthorityReceipt {
    pub request: ManifoldRuntimeCommandRequest,
    pub dispatch: ManifoldRuntimeDispatchReceipt,
    pub application: ManifoldRuntimeApplicationReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageInstallReleaseCommandAuthorization {
    pub manifold_request_id: String,
    pub owner_action_request_id: String,
    pub requester_id: String,
    pub operation_id: String,
    pub preview_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub release: PackageReleaseReference,
    pub expected_package_name: String,
    pub expected_rollout_ring: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageInstallReleaseAuthorityReceipt {
    pub request: ManifoldRuntimeCommandRequest,
    pub dispatch: ManifoldRuntimeDispatchReceipt,
    pub application: ManifoldRuntimeApplicationReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestAwakeCommandAuthorization {
    pub manifold_request_id: String,
    pub owner_action_request_id: String,
    pub requester_id: String,
    pub operation_id: String,
    pub preview_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub action: QuestAwakeAction,
    pub duration_ms: u32,
    pub watchdog_interval_ms: u32,
    pub watchdog_generation: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestAwakeAuthorityReceipt {
    pub request: ManifoldRuntimeCommandRequest,
    pub dispatch: ManifoldRuntimeDispatchReceipt,
    pub application: ManifoldRuntimeApplicationReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestWifiAdbCommandAuthorization {
    pub manifold_request_id: String,
    pub owner_action_request_id: String,
    pub requester_id: String,
    pub operation_id: String,
    pub preview_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub action: QuestWifiAdbAction,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestWifiAdbAuthorityReceipt {
    pub request: ManifoldRuntimeCommandRequest,
    pub dispatch: ManifoldRuntimeDispatchReceipt,
    pub application: ManifoldRuntimeApplicationReceipt,
}

#[derive(Serialize)]
struct KioskShowControlsTypedParams<'a> {
    schema: &'static str,
    operation_id: &'a str,
    preview_id: &'a str,
    action_id: &'static str,
    device_id: &'a str,
    identity_revision: u64,
}

#[derive(Serialize)]
struct PackageInstallReleaseTypedParams<'a> {
    schema: &'static str,
    operation_id: &'a str,
    preview_id: &'a str,
    action_id: &'static str,
    device_id: &'a str,
    identity_revision: u64,
    release: &'a PackageReleaseReference,
    expected_package_name: &'a str,
    expected_rollout_ring: &'a str,
}

#[derive(Serialize)]
struct QuestAwakeTypedParams<'a> {
    schema: &'static str,
    operation_id: &'a str,
    preview_id: &'a str,
    action_id: &'static str,
    device_id: &'a str,
    identity_revision: u64,
    action: QuestAwakeAction,
    duration_ms: u32,
    watchdog_interval_ms: u32,
    watchdog_generation: &'a str,
}

#[derive(Serialize)]
struct QuestWifiAdbTypedParams<'a> {
    schema: &'static str,
    operation_id: &'a str,
    preview_id: &'a str,
    action_id: &'static str,
    device_id: &'a str,
    identity_revision: u64,
    action: QuestWifiAdbAction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoopbackAdbCapabilityExtensions {
    proof_schema: String,
    state: String,
    route_mode: String,
    discovery_mode: String,
    listener_discovered: bool,
    shell_uid: String,
    owner_evidence_sha256: String,
}

#[derive(Clone, Debug)]
pub struct FleetManifoldAdapter {
    enrollment: ManifoldPeerEnrollmentState,
    accepted_peers: ManifoldAcceptedPeerState,
    trusted_operator_ids: Vec<DottedId>,
    seen_checkins: BTreeMap<String, i64>,
    runtime_host: ManifoldRuntimeHost,
    termux_proofs: BTreeMap<String, AdmittedQuestWifiAdbTermuxProof>,
}

impl FleetManifoldAdapter {
    #[must_use]
    pub fn new(trusted_operator_ids: Vec<DottedId>) -> Self {
        let runtime_host = ManifoldRuntimeHost::from_snapshot(ManifoldRuntimeHostSnapshot {
            schema_id: schema_id(HOST_SNAPSHOT_SCHEMA),
            host_id: DottedId::new(FLEET_RUNTIME_HOST_ID).expect("static runtime host id"),
            authority_revision: Revision::INITIAL,
            commands: vec![
                ManifoldRuntimeCommandDescriptor {
                    command_id: DottedId::new(KIOSK_SHOW_CONTROLS_COMMAND_ID)
                        .expect("static command id"),
                    required_lease_scope: None,
                },
                ManifoldRuntimeCommandDescriptor {
                    command_id: DottedId::new(QUEST_AWAKE_COMMAND_ID).expect("static command id"),
                    required_lease_scope: None,
                },
                ManifoldRuntimeCommandDescriptor {
                    command_id: DottedId::new(QUEST_WIFI_ADB_COMMAND_ID)
                        .expect("static command id"),
                    required_lease_scope: None,
                },
                ManifoldRuntimeCommandDescriptor {
                    command_id: DottedId::new(PACKAGES_INSTALL_RELEASE_COMMAND_ID)
                        .expect("static command id"),
                    required_lease_scope: None,
                },
            ],
            leases: Vec::new(),
            applied_request_ids: Vec::new(),
            reviewed_sweep_ids: Vec::new(),
            audit_events: Vec::new(),
        })
        .expect("static Fleet runtime host snapshot");
        Self {
            enrollment: ManifoldPeerEnrollmentState::empty(),
            accepted_peers: ManifoldAcceptedPeerState {
                schema_id: schema_id("rusty.manifold.peer.accepted_state.v1"),
                authority_revision: rusty_manifold_model::Revision::INITIAL,
                peers: Vec::new(),
                applied_proposal_ids: Vec::new(),
            },
            trusted_operator_ids,
            seen_checkins: BTreeMap::new(),
            runtime_host,
            termux_proofs: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn enrollment(&self) -> &ManifoldPeerEnrollmentState {
        &self.enrollment
    }

    #[must_use]
    pub const fn accepted_peers(&self) -> &ManifoldAcceptedPeerState {
        &self.accepted_peers
    }

    #[must_use]
    pub fn accepted_peer_ids(&self) -> Vec<String> {
        self.accepted_peers
            .peers
            .iter()
            .map(|peer| peer.identity.peer_id.to_string())
            .collect()
    }

    #[must_use]
    pub fn snapshot(&self) -> FleetManifoldAdapterSnapshot {
        FleetManifoldAdapterSnapshot {
            schema: FLEET_MANIFOLD_SNAPSHOT_SCHEMA.to_owned(),
            accepted_peers: self.accepted_peers.clone(),
            seen_checkins: self.seen_checkins.clone(),
            runtime_host: Some(self.runtime_host.snapshot().clone()),
            termux_proofs: self.termux_proofs.clone(),
        }
    }

    pub fn restore_session(
        &mut self,
        mut snapshot: FleetManifoldAdapterSnapshot,
        now_ms: i64,
    ) -> Result<(), String> {
        if snapshot.schema != FLEET_MANIFOLD_SNAPSHOT_V1_SCHEMA
            && snapshot.schema != FLEET_MANIFOLD_SNAPSHOT_V2_SCHEMA
            && snapshot.schema != FLEET_MANIFOLD_SNAPSHOT_SCHEMA
        {
            return Err("Fleet Manifold adapter snapshot schema is not supported".to_owned());
        }
        if snapshot.schema != FLEET_MANIFOLD_SNAPSHOT_V1_SCHEMA && snapshot.runtime_host.is_none() {
            return Err("Fleet Manifold adapter v2 snapshot omitted runtime authority".to_owned());
        }
        snapshot.termux_proofs.retain(|_, admitted| {
            admitted.checkin_expires_at_ms > now_ms && admitted.proof.fresh_until_ms > now_ms
        });
        if snapshot.seen_checkins.len() > MAX_SEEN_CHECKINS
            || snapshot.accepted_peers.applied_proposal_ids.len() > MAX_SEEN_CHECKINS
            || snapshot.termux_proofs.len() > MAX_SEEN_CHECKINS
        {
            return Err("Fleet Manifold adapter snapshot exceeds evidence limits".to_owned());
        }
        if snapshot.termux_proofs.iter().any(|(proof_id, admitted)| {
            proof_id != &admitted.proof.proof_id
                || admitted.proof.validate().is_err()
                || admitted.checkin_id.is_empty()
                || admitted.checkin_expires_at_ms <= admitted.proof.observed_at_ms
                || admitted.accepted_at_ms < 0
        }) {
            return Err(
                "Fleet Manifold adapter snapshot contains invalid Termux proofs".to_owned(),
            );
        }
        let peer_ids: BTreeSet<_> = snapshot
            .accepted_peers
            .peers
            .iter()
            .map(|peer| peer.identity.peer_id.clone())
            .collect();
        if peer_ids.len() != snapshot.accepted_peers.peers.len()
            || snapshot
                .accepted_peers
                .applied_proposal_ids
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != snapshot.accepted_peers.applied_proposal_ids.len()
        {
            return Err(
                "Fleet Manifold adapter snapshot contains duplicate authority evidence".to_owned(),
            );
        }
        for peer in &snapshot.accepted_peers.peers {
            let matching_credential = self.enrollment.credentials.iter().any(|credential| {
                let fingerprint = credential
                    .public_key_sha256
                    .strip_prefix("sha256:")
                    .map(|digest| format!("fingerprint.{digest}"));
                credential.peer_id == peer.identity.peer_id
                    && credential.status == ManifoldPeerCredentialStatus::Active
                    && fingerprint.as_deref() == Some(peer.identity.key_fingerprint.as_str())
            });
            if !matching_credential {
                return Err(format!(
                    "accepted peer {} is not bound to a current active enrollment",
                    peer.identity.peer_id
                ));
            }
        }
        snapshot
            .seen_checkins
            .retain(|_, expires_at_ms| *expires_at_ms > now_ms);
        let restored_runtime_host = match snapshot.runtime_host {
            Some(runtime_snapshot) => {
                if runtime_snapshot.authority_revision.get() == u64::MAX {
                    return Err(
                        "Fleet runtime authority cannot recover at the terminal revision"
                            .to_owned(),
                    );
                }
                ManifoldRuntimeHost::from_snapshot(runtime_snapshot).map_err(|error| {
                    format!("Fleet runtime authority snapshot is invalid: {error}")
                })?
            }
            None => self.runtime_host.clone(),
        };
        self.accepted_peers = snapshot.accepted_peers;
        self.seen_checkins = snapshot.seen_checkins;
        self.runtime_host = restored_runtime_host;
        self.termux_proofs = snapshot.termux_proofs;
        Ok(())
    }

    /// Projects only Termux proof already admitted inside an enrolled,
    /// Ed25519-signed check-in. The Hub itself exposes no shape-only proof
    /// mutation route.
    pub fn quest_wifi_adb_operation(
        &self,
        hub: &FleetHub,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<QuestWifiAdbOperation, String> {
        let mut operation = hub
            .quest_wifi_adb_operation(operation_id)
            .map_err(|error| error.to_string())?;
        for target in &mut operation.targets {
            let current = hub
                .inspect(&target.device_id, now_ms)
                .map_err(|error| error.to_string())?;
            let proof = self
                .termux_proofs
                .values()
                .filter(|admitted| {
                    admitted.proof.device_id == target.device_id
                        && admitted.proof.identity_revision == target.identity_revision
                        && admitted.proof.source_epoch == current.row.source_epoch
                        && hub.device_source_lineage(&target.device_id).is_some_and(
                            |(_, source_revision, identity_revision)| {
                                admitted.proof.source_revision == source_revision
                                    && admitted.proof.identity_revision == identity_revision
                            },
                        )
                })
                .max_by_key(|admitted| {
                    (
                        admitted.proof.source_revision,
                        admitted.proof.observed_at_ms,
                    )
                });
            if let Some(admitted) = proof {
                let proof = &admitted.proof;
                target.termux_proof = Some(proof.clone());
                let receipt_matches = target.receipt.as_ref().is_some_and(|receipt| {
                    receipt.effect_applied
                        && receipt.route_mode == QuestWifiAdbRouteMode::ModernTls
                        // The device proof and File Manager receipt have
                        // different clock authorities. Order them only by
                        // their trusted Hub admission times and fail closed
                        // on a same-millisecond tie.
                        && admitted.accepted_at_ms > target.updated_at_ms
                });
                let disabled_after_proof =
                    hub.quest_wifi_adb_operations().iter().any(|candidate| {
                        candidate.preview.action == QuestWifiAdbAction::DisableWirelessAdb
                            && candidate.targets.iter().any(|candidate_target| {
                                candidate_target.device_id == target.device_id
                                    && candidate_target.receipt.as_ref().is_some_and(|receipt| {
                                        receipt.effect_applied
                                            && candidate_target.updated_at_ms
                                                >= admitted.accepted_at_ms
                                    })
                            })
                    });
                target.termux_usable = proof.available
                    && proof.fresh_until_ms > now_ms
                    && receipt_matches
                    && operation.preview.action != QuestWifiAdbAction::DisableWirelessAdb
                    && !disabled_after_proof;
                target.updated_at_ms = target.updated_at_ms.max(admitted.accepted_at_ms);
                operation.updated_at_ms = operation.updated_at_ms.max(admitted.accepted_at_ms);
            }
        }
        operation.validate().map_err(|failures| {
            format!(
                "admitted Termux proof projection is invalid: {}",
                failures
                    .iter()
                    .map(|failure| format!("{}:{}", failure.path, failure.code))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })?;
        Ok(operation)
    }

    #[must_use]
    pub const fn runtime_host_snapshot(&self) -> &ManifoldRuntimeHostSnapshot {
        self.runtime_host.snapshot()
    }

    #[must_use]
    pub fn has_applied_kiosk_authorization(
        &self,
        operation_id: &str,
        device_id: &str,
        owner_action_request_id: &str,
    ) -> bool {
        let expected = kiosk_manifold_request_id(operation_id, device_id, owner_action_request_id);
        self.runtime_host
            .snapshot()
            .applied_request_ids
            .iter()
            .any(|request_id| request_id.as_str() == expected)
    }

    #[must_use]
    pub fn has_applied_package_authorization(
        &self,
        operation_id: &str,
        device_id: &str,
        owner_action_request_id: &str,
    ) -> bool {
        let expected =
            package_manifold_request_id(operation_id, device_id, owner_action_request_id);
        self.runtime_host
            .snapshot()
            .applied_request_ids
            .iter()
            .any(|request_id| request_id.as_str() == expected)
    }

    pub fn authorize_kiosk_show_controls(
        &mut self,
        authorization: &KioskShowControlsCommandAuthorization,
        now_ms: u64,
    ) -> Result<KioskShowControlsAuthorityReceipt, String> {
        if authorization.operation_id.is_empty()
            || authorization.operation_id.len() > 256
            || authorization.preview_id.is_empty()
            || authorization.preview_id.len() > 256
            || authorization.device_id.is_empty()
            || authorization.device_id.len() > 256
            || authorization.identity_revision == 0
            || authorization.issued_at_ms > now_ms
            || now_ms >= authorization.expires_at_ms
            || authorization.expires_at_ms <= authorization.issued_at_ms
            || authorization
                .expires_at_ms
                .checked_sub(authorization.issued_at_ms)
                .is_none_or(|lifetime| lifetime > MAX_KIOSK_COMMAND_LIFETIME_MS)
        {
            return Err("Fleet Kiosk command authorization is invalid or expired".to_owned());
        }
        if authorization.owner_action_request_id.is_empty()
            || authorization.manifold_request_id
                != kiosk_manifold_request_id(
                    &authorization.operation_id,
                    &authorization.device_id,
                    &authorization.owner_action_request_id,
                )
        {
            return Err(
                "Fleet Kiosk Manifold request identity does not bind the owner action".to_owned(),
            );
        }
        if self.runtime_host.snapshot().authority_revision.get() == u64::MAX {
            return Err("Fleet runtime authority reached its terminal revision".to_owned());
        }
        let params = KioskShowControlsTypedParams {
            schema: "rusty.fleet.kiosk_show_controls_params.v1",
            operation_id: &authorization.operation_id,
            preview_id: &authorization.preview_id,
            action_id: KIOSK_SHOW_CONTROLS_COMMAND_ID,
            device_id: &authorization.device_id,
            identity_revision: authorization.identity_revision,
        };
        let canonical_params = serde_jcs::to_vec(&params)
            .map_err(|error| format!("Fleet Kiosk parameters are not canonicalizable: {error}"))?;
        if canonical_params.is_empty() || canonical_params.len() > 4_096 {
            return Err("Fleet Kiosk parameters exceed the Manifold command bound".to_owned());
        }
        let canonical_size_bytes = u32::try_from(canonical_params.len())
            .map_err(|_| "Fleet Kiosk parameter size is not representable".to_owned())?;
        let request = ManifoldRuntimeCommandRequest {
            schema_id: schema_id(HOST_COMMAND_REQUEST_SCHEMA),
            request_id: DottedId::new(authorization.manifold_request_id.clone())
                .map_err(|error| format!("invalid Manifold request ID: {error}"))?,
            expected_authority_revision: self.runtime_host.snapshot().authority_revision,
            requester_id: DottedId::new(authorization.requester_id.clone())
                .map_err(|error| format!("invalid Manifold requester ID: {error}"))?,
            command_id: DottedId::new(KIOSK_SHOW_CONTROLS_COMMAND_ID).expect("static command id"),
            lease_id: None,
            params_digest: Some(ManifoldRuntimeTypedParamsDigest {
                schema_id: schema_id(HOST_TYPED_PARAMS_DIGEST_SCHEMA),
                params_type_id: DottedId::new(KIOSK_SHOW_CONTROLS_PARAMS_TYPE_ID)
                    .expect("static params type id"),
                canonical_sha256: format!(
                    "sha256:{}",
                    hex::encode(Sha256::digest(&canonical_params))
                ),
                canonical_size_bytes,
            }),
            issued_at_ms: authorization.issued_at_ms,
            expires_at_ms: authorization.expires_at_ms,
        };
        let dispatch = self.runtime_host.review_command(&request, now_ms);
        if dispatch.outcome != ManifoldRuntimeDispatchOutcome::Ready {
            return Err(format!(
                "Manifold rejected Fleet Kiosk dispatch review: {:?}",
                dispatch.rejection_reason
            ));
        }
        let application = self
            .runtime_host
            .apply_dispatch(&request, &dispatch, now_ms);
        if !application.applied {
            return Err(format!(
                "Manifold rejected Fleet Kiosk dispatch application: {:?}",
                application.rejection_reason
            ));
        }
        Ok(KioskShowControlsAuthorityReceipt {
            request,
            dispatch,
            application,
        })
    }

    pub fn authorize_package_install_release(
        &mut self,
        authorization: &PackageInstallReleaseCommandAuthorization,
        now_ms: u64,
    ) -> Result<PackageInstallReleaseAuthorityReceipt, String> {
        if authorization.operation_id.is_empty()
            || authorization.operation_id.len() > 256
            || authorization.preview_id.is_empty()
            || authorization.preview_id.len() > 256
            || authorization.device_id.is_empty()
            || authorization.device_id.len() > 256
            || authorization.identity_revision == 0
            || authorization.expected_package_name.is_empty()
            || authorization.expected_package_name.len() > 255
            || authorization.expected_rollout_ring.is_empty()
            || authorization.expected_rollout_ring.len() > 128
            || authorization.release.validate().is_err()
            || authorization.issued_at_ms > now_ms
            || now_ms >= authorization.expires_at_ms
            || authorization.expires_at_ms <= authorization.issued_at_ms
            || authorization
                .expires_at_ms
                .checked_sub(authorization.issued_at_ms)
                .is_none_or(|lifetime| lifetime > MAX_PACKAGE_COMMAND_LIFETIME_MS)
        {
            return Err("Fleet package command authorization is invalid or expired".to_owned());
        }
        if authorization.owner_action_request_id.is_empty()
            || authorization.manifold_request_id
                != package_manifold_request_id(
                    &authorization.operation_id,
                    &authorization.device_id,
                    &authorization.owner_action_request_id,
                )
        {
            return Err(
                "Fleet package Manifold request identity does not bind the owner action".to_owned(),
            );
        }
        if self.runtime_host.snapshot().authority_revision.get() == u64::MAX {
            return Err("Fleet runtime authority reached its terminal revision".to_owned());
        }
        let params = PackageInstallReleaseTypedParams {
            schema: "rusty.fleet.package_install_release_params.v1",
            operation_id: &authorization.operation_id,
            preview_id: &authorization.preview_id,
            action_id: PACKAGES_INSTALL_RELEASE_COMMAND_ID,
            device_id: &authorization.device_id,
            identity_revision: authorization.identity_revision,
            release: &authorization.release,
            expected_package_name: &authorization.expected_package_name,
            expected_rollout_ring: &authorization.expected_rollout_ring,
        };
        let canonical_params = serde_jcs::to_vec(&params).map_err(|error| {
            format!("Fleet package parameters are not canonicalizable: {error}")
        })?;
        if canonical_params.is_empty() || canonical_params.len() > 8_192 {
            return Err("Fleet package parameters exceed the Manifold command bound".to_owned());
        }
        let canonical_size_bytes = u32::try_from(canonical_params.len())
            .map_err(|_| "Fleet package parameter size is not representable".to_owned())?;
        let request = ManifoldRuntimeCommandRequest {
            schema_id: schema_id(HOST_COMMAND_REQUEST_SCHEMA),
            request_id: DottedId::new(authorization.manifold_request_id.clone())
                .map_err(|error| format!("invalid Manifold request ID: {error}"))?,
            expected_authority_revision: self.runtime_host.snapshot().authority_revision,
            requester_id: DottedId::new(authorization.requester_id.clone())
                .map_err(|error| format!("invalid Manifold requester ID: {error}"))?,
            command_id: DottedId::new(PACKAGES_INSTALL_RELEASE_COMMAND_ID)
                .expect("static command id"),
            lease_id: None,
            params_digest: Some(ManifoldRuntimeTypedParamsDigest {
                schema_id: schema_id(HOST_TYPED_PARAMS_DIGEST_SCHEMA),
                params_type_id: DottedId::new(PACKAGES_INSTALL_RELEASE_PARAMS_TYPE_ID)
                    .expect("static params type id"),
                canonical_sha256: format!(
                    "sha256:{}",
                    hex::encode(Sha256::digest(&canonical_params))
                ),
                canonical_size_bytes,
            }),
            issued_at_ms: authorization.issued_at_ms,
            expires_at_ms: authorization.expires_at_ms,
        };
        let dispatch = self.runtime_host.review_command(&request, now_ms);
        if dispatch.outcome != ManifoldRuntimeDispatchOutcome::Ready {
            return Err(format!(
                "Manifold rejected Fleet package dispatch review: {:?}",
                dispatch.rejection_reason
            ));
        }
        let application = self
            .runtime_host
            .apply_dispatch(&request, &dispatch, now_ms);
        if !application.applied {
            return Err(format!(
                "Manifold rejected Fleet package dispatch application: {:?}",
                application.rejection_reason
            ));
        }
        Ok(PackageInstallReleaseAuthorityReceipt {
            request,
            dispatch,
            application,
        })
    }

    pub fn authorize_quest_awake(
        &mut self,
        authorization: &QuestAwakeCommandAuthorization,
        now_ms: u64,
    ) -> Result<QuestAwakeAuthorityReceipt, String> {
        if authorization.operation_id.is_empty()
            || authorization.operation_id.len() > 256
            || authorization.preview_id.is_empty()
            || authorization.preview_id.len() > 256
            || authorization.device_id.is_empty()
            || authorization.device_id.len() > 256
            || authorization.watchdog_generation.is_empty()
            || authorization.watchdog_generation.len() > 256
            || authorization.identity_revision == 0
            || !(60_000..=28_800_000).contains(&authorization.duration_ms)
            || !(1_000..=60_000).contains(&authorization.watchdog_interval_ms)
            || authorization.issued_at_ms > now_ms
            || now_ms >= authorization.expires_at_ms
            || authorization.expires_at_ms <= authorization.issued_at_ms
            || authorization
                .expires_at_ms
                .checked_sub(authorization.issued_at_ms)
                .is_none_or(|lifetime| lifetime > MAX_QUEST_AWAKE_COMMAND_LIFETIME_MS)
        {
            return Err("Fleet Quest awake authorization is invalid or expired".to_owned());
        }
        if authorization.owner_action_request_id.is_empty()
            || authorization.manifold_request_id
                != quest_awake_manifold_request_id(
                    &authorization.operation_id,
                    &authorization.device_id,
                    &authorization.owner_action_request_id,
                )
        {
            return Err(
                "Fleet Quest awake Manifold request identity does not bind the owner action"
                    .to_owned(),
            );
        }
        if self.runtime_host.snapshot().authority_revision.get() == u64::MAX {
            return Err("Fleet runtime authority reached its terminal revision".to_owned());
        }
        let params = QuestAwakeTypedParams {
            schema: "rusty.fleet.quest_awake_params.v1",
            operation_id: &authorization.operation_id,
            preview_id: &authorization.preview_id,
            action_id: QUEST_AWAKE_COMMAND_ID,
            device_id: &authorization.device_id,
            identity_revision: authorization.identity_revision,
            action: authorization.action,
            duration_ms: authorization.duration_ms,
            watchdog_interval_ms: authorization.watchdog_interval_ms,
            watchdog_generation: &authorization.watchdog_generation,
        };
        let canonical_params = serde_jcs::to_vec(&params).map_err(|error| {
            format!("Fleet Quest awake parameters are not canonicalizable: {error}")
        })?;
        if canonical_params.is_empty() || canonical_params.len() > 8_192 {
            return Err(
                "Fleet Quest awake parameters exceed the Manifold command bound".to_owned(),
            );
        }
        let canonical_size_bytes = u32::try_from(canonical_params.len())
            .map_err(|_| "Fleet Quest awake parameter size is not representable".to_owned())?;
        let request = ManifoldRuntimeCommandRequest {
            schema_id: schema_id(HOST_COMMAND_REQUEST_SCHEMA),
            request_id: DottedId::new(authorization.manifold_request_id.clone())
                .map_err(|error| format!("invalid Manifold request ID: {error}"))?,
            expected_authority_revision: self.runtime_host.snapshot().authority_revision,
            requester_id: DottedId::new(authorization.requester_id.clone())
                .map_err(|error| format!("invalid Manifold requester ID: {error}"))?,
            command_id: DottedId::new(QUEST_AWAKE_COMMAND_ID).expect("static command id"),
            lease_id: None,
            params_digest: Some(ManifoldRuntimeTypedParamsDigest {
                schema_id: schema_id(HOST_TYPED_PARAMS_DIGEST_SCHEMA),
                params_type_id: DottedId::new(QUEST_AWAKE_PARAMS_TYPE_ID)
                    .expect("static params type id"),
                canonical_sha256: format!(
                    "sha256:{}",
                    hex::encode(Sha256::digest(&canonical_params))
                ),
                canonical_size_bytes,
            }),
            issued_at_ms: authorization.issued_at_ms,
            expires_at_ms: authorization.expires_at_ms,
        };
        let dispatch = self.runtime_host.review_command(&request, now_ms);
        if dispatch.outcome != ManifoldRuntimeDispatchOutcome::Ready {
            return Err(format!(
                "Manifold rejected Fleet Quest awake dispatch review: {:?}",
                dispatch.rejection_reason
            ));
        }
        let application = self
            .runtime_host
            .apply_dispatch(&request, &dispatch, now_ms);
        if !application.applied {
            return Err(format!(
                "Manifold rejected Fleet Quest awake dispatch application: {:?}",
                application.rejection_reason
            ));
        }
        Ok(QuestAwakeAuthorityReceipt {
            request,
            dispatch,
            application,
        })
    }

    pub fn authorize_quest_wifi_adb(
        &mut self,
        authorization: &QuestWifiAdbCommandAuthorization,
        now_ms: u64,
    ) -> Result<QuestWifiAdbAuthorityReceipt, String> {
        if authorization.operation_id.is_empty()
            || authorization.operation_id.len() > 256
            || authorization.preview_id.is_empty()
            || authorization.preview_id.len() > 256
            || authorization.device_id.is_empty()
            || authorization.device_id.len() > 256
            || authorization.identity_revision == 0
            || authorization.issued_at_ms > now_ms
            || now_ms >= authorization.expires_at_ms
            || authorization.expires_at_ms <= authorization.issued_at_ms
            || authorization
                .expires_at_ms
                .checked_sub(authorization.issued_at_ms)
                .is_none_or(|lifetime| lifetime > MAX_QUEST_WIFI_ADB_COMMAND_LIFETIME_MS)
        {
            return Err("Fleet Quest Wi-Fi ADB authorization is invalid or expired".to_owned());
        }
        if authorization.owner_action_request_id.is_empty()
            || authorization.manifold_request_id
                != quest_wifi_adb_manifold_request_id(
                    &authorization.operation_id,
                    &authorization.device_id,
                    &authorization.owner_action_request_id,
                )
        {
            return Err(
                "Fleet Quest Wi-Fi ADB Manifold request identity does not bind the owner action"
                    .to_owned(),
            );
        }
        if self.runtime_host.snapshot().authority_revision.get() == u64::MAX {
            return Err("Fleet runtime authority reached its terminal revision".to_owned());
        }
        let params = QuestWifiAdbTypedParams {
            schema: "rusty.fleet.quest_wifi_adb_params.v1",
            operation_id: &authorization.operation_id,
            preview_id: &authorization.preview_id,
            action_id: QUEST_WIFI_ADB_COMMAND_ID,
            device_id: &authorization.device_id,
            identity_revision: authorization.identity_revision,
            action: authorization.action,
        };
        let canonical_params = serde_jcs::to_vec(&params).map_err(|error| {
            format!("Fleet Quest Wi-Fi ADB parameters are not canonicalizable: {error}")
        })?;
        if canonical_params.is_empty() || canonical_params.len() > 8_192 {
            return Err(
                "Fleet Quest Wi-Fi ADB parameters exceed the Manifold command bound".to_owned(),
            );
        }
        let canonical_size_bytes = u32::try_from(canonical_params.len())
            .map_err(|_| "Fleet Quest Wi-Fi ADB parameter size is not representable".to_owned())?;
        let request = ManifoldRuntimeCommandRequest {
            schema_id: schema_id(HOST_COMMAND_REQUEST_SCHEMA),
            request_id: DottedId::new(authorization.manifold_request_id.clone())
                .map_err(|error| format!("invalid Manifold request ID: {error}"))?,
            expected_authority_revision: self.runtime_host.snapshot().authority_revision,
            requester_id: DottedId::new(authorization.requester_id.clone())
                .map_err(|error| format!("invalid Manifold requester ID: {error}"))?,
            command_id: DottedId::new(QUEST_WIFI_ADB_COMMAND_ID).expect("static command id"),
            lease_id: None,
            params_digest: Some(ManifoldRuntimeTypedParamsDigest {
                schema_id: schema_id(HOST_TYPED_PARAMS_DIGEST_SCHEMA),
                params_type_id: DottedId::new(QUEST_WIFI_ADB_PARAMS_TYPE_ID)
                    .expect("static params type id"),
                canonical_sha256: format!(
                    "sha256:{}",
                    hex::encode(Sha256::digest(&canonical_params))
                ),
                canonical_size_bytes,
            }),
            issued_at_ms: authorization.issued_at_ms,
            expires_at_ms: authorization.expires_at_ms,
        };
        let dispatch = self.runtime_host.review_command(&request, now_ms);
        if dispatch.outcome != ManifoldRuntimeDispatchOutcome::Ready {
            return Err(format!(
                "Manifold rejected Fleet Quest Wi-Fi ADB dispatch review: {:?}",
                dispatch.rejection_reason
            ));
        }
        let application = self
            .runtime_host
            .apply_dispatch(&request, &dispatch, now_ms);
        if !application.applied {
            return Err(format!(
                "Manifold rejected Fleet Quest Wi-Fi ADB dispatch application: {:?}",
                application.rejection_reason
            ));
        }
        Ok(QuestWifiAdbAuthorityReceipt {
            request,
            dispatch,
            application,
        })
    }

    pub fn apply_enrollment(
        &mut self,
        request: &ManifoldPeerEnrollmentRequest,
        now_ms: u64,
    ) -> ManifoldPeerEnrollmentReceipt {
        let (next, receipt) = review_and_apply_peer_enrollment(
            &self.enrollment,
            request,
            &self.trusted_operator_ids,
            now_ms,
        );
        self.enrollment = next;
        receipt
    }

    pub fn accept(
        &mut self,
        hub: &mut FleetHub,
        signed: SignedFleetCheckIn,
        now_ms: i64,
    ) -> CheckInReceipt {
        let checkin_id = signed.claims.checkin_id.clone();
        if signed.validate().is_err() {
            return rejected(checkin_id, CheckInRejectionReason::ContractInvalid);
        }
        if now_ms < signed.claims.issued_at_ms || now_ms >= signed.claims.expires_at_ms {
            return rejected(checkin_id, CheckInRejectionReason::StaleOrFuture);
        }
        self.seen_checkins
            .retain(|_, expires_at_ms| *expires_at_ms > now_ms);
        if self.seen_checkins.contains_key(&checkin_id) {
            return rejected(checkin_id, CheckInRejectionReason::Replay);
        }
        if self.seen_checkins.len() >= MAX_SEEN_CHECKINS {
            return rejected(checkin_id, CheckInRejectionReason::EvidenceLimitExceeded);
        }

        let Ok(mut proposal) = serde_json::from_value::<ManifoldPeerStatusProposal>(
            signed.claims.manifold_peer_status_proposal.clone(),
        ) else {
            return rejected(checkin_id, CheckInRejectionReason::ContractInvalid);
        };
        if proposal.identity.peer_id.as_str() != signed.claims.observation.identity.device_id
            || proposal.status.peer_id != proposal.identity.peer_id
        {
            return rejected(checkin_id, CheckInRejectionReason::IdentityMismatch);
        }
        let Ok(source_time_ms) = u64::try_from(signed.claims.observation.source_time_ms) else {
            return rejected(
                checkin_id,
                CheckInRejectionReason::AuthorityEvidenceMismatch,
            );
        };
        let Ok(expires_at_ms) = u64::try_from(signed.claims.expires_at_ms) else {
            return rejected(
                checkin_id,
                CheckInRejectionReason::AuthorityEvidenceMismatch,
            );
        };
        if proposal.status.observed_at_ms != source_time_ms
            || proposal.status.expires_at_ms != expires_at_ms
        {
            return rejected(
                checkin_id,
                CheckInRejectionReason::AuthorityEvidenceMismatch,
            );
        }

        let Some(credential) = self.enrollment.credentials.iter().find(|candidate| {
            candidate.key_id.as_str() == signed.key_id
                && candidate.peer_id == proposal.identity.peer_id
                && candidate.status == ManifoldPeerCredentialStatus::Active
        }) else {
            return rejected(checkin_id, CheckInRejectionReason::UnknownOrInactiveKey);
        };
        let Ok(now_unsigned) = u64::try_from(now_ms) else {
            return rejected(checkin_id, CheckInRejectionReason::StaleOrFuture);
        };
        if credential.valid_from_ms > now_unsigned || credential.expires_at_ms <= now_unsigned {
            return rejected(checkin_id, CheckInRejectionReason::KeyOutsideValidity);
        }
        if !verify_signature(credential.public_key_hex.as_str(), &signed) {
            return rejected(checkin_id, CheckInRejectionReason::SignatureInvalid);
        }
        let candidate_termux_proof = match signed_termux_proof_from_capability(&signed, &proposal) {
            Ok(proof) => proof,
            Err(()) => {
                return rejected(
                    checkin_id,
                    CheckInRejectionReason::AuthorityEvidenceMismatch,
                );
            }
        };
        let Some(public_key_sha256) = credential.public_key_sha256.strip_prefix("sha256:") else {
            return rejected(checkin_id, CheckInRejectionReason::ContractInvalid);
        };
        let Ok(trusted_key_fingerprint) = DottedId::new(format!("fingerprint.{public_key_sha256}"))
        else {
            return rejected(checkin_id, CheckInRejectionReason::ContractInvalid);
        };

        let Ok(case_id) = DottedId::new(format!("case.{checkin_id}")) else {
            return rejected(checkin_id, CheckInRejectionReason::ContractInvalid);
        };

        // Fleet admission is previewed against a clone so neither authority
        // advances when the other side rejects this envelope.
        let mut candidate_hub = hub.clone();
        let mut candidate_termux_proofs = self.termux_proofs.clone();
        candidate_termux_proofs.retain(|_, admitted| {
            admitted.checkin_expires_at_ms > now_ms && admitted.proof.fresh_until_ms > now_ms
        });
        let mut accepted_observation = signed.claims.observation.clone();
        accepted_observation.received_time_ms = now_ms;
        accepted_observation.conditions.push(StatusCondition {
            family: ConditionFamily::Freshness,
            state: ConditionState::Current,
            reason: "local_authenticated_checkin".to_owned(),
            message: "The local Hub admitted a current signed device check-in.".to_owned(),
            source_time_ms: accepted_observation.source_time_ms,
            received_time_ms: now_ms,
            accepted_revision: hub.result_revision().saturating_add(1),
            fresh_until_ms: signed.claims.expires_at_ms,
            source: StatusSource {
                owner: "rusty-manifold".to_owned(),
                adapter_id: "fleet.local-ingress".to_owned(),
                authority_revision: self
                    .accepted_peers
                    .authority_revision
                    .get()
                    .saturating_add(1),
            },
            sensitivity: Sensitivity::Operator,
            extensions: BTreeMap::new(),
        });
        let fleet_decision = candidate_hub.accept_observation(accepted_observation, now_ms);
        if !matches!(fleet_decision, ObservationDecision::Accepted { .. }) {
            return CheckInReceipt {
                schema: "rusty.fleet.checkin_receipt.v1".to_owned(),
                checkin_id,
                accepted: false,
                rejection_reason: Some(CheckInRejectionReason::FleetRejected),
                manifold_decision: None,
                manifold_application: None,
                fleet_decision: Some(fleet_decision),
            };
        }
        if let Some(proof) = candidate_termux_proof {
            if validate_signed_termux_proof(
                &candidate_hub,
                &signed,
                &proof,
                now_ms,
                &candidate_termux_proofs,
            )
            .is_err()
            {
                return rejected(
                    checkin_id,
                    CheckInRejectionReason::AuthorityEvidenceMismatch,
                );
            }
            candidate_termux_proofs
                .retain(|_, admitted| admitted.proof.device_id != proof.device_id);
            if candidate_termux_proofs.len() >= MAX_SEEN_CHECKINS {
                return rejected(checkin_id, CheckInRejectionReason::EvidenceLimitExceeded);
            }
            candidate_termux_proofs.insert(
                proof.proof_id.clone(),
                AdmittedQuestWifiAdbTermuxProof {
                    proof,
                    checkin_id: checkin_id.clone(),
                    checkin_expires_at_ms: signed.claims.expires_at_ms,
                    accepted_at_ms: now_ms,
                },
            );
        } else {
            // A newer accepted source snapshot without the capability
            // supersedes all older proofs for that device.
            candidate_termux_proofs.retain(|_, admitted| {
                admitted.proof.device_id != signed.claims.observation.identity.device_id
            });
        }

        // The source signs its complete peer status and proposal identity, but
        // the trusted ingress owns the optimistic lock against fleet-global
        // Manifold state. Independent devices cannot predict or serialize that
        // global revision. All source-owned evidence remains byte-for-byte
        // covered by the signature; only this authority-owned field is bound
        // to the current state immediately before review.
        proposal.expected_authority_revision = self.accepted_peers.authority_revision;
        let case = ManifoldPeerReviewCase {
            schema_id: schema_id("rusty.manifold.peer.review_case.v1"),
            case_id,
            current_state: self.accepted_peers.clone(),
            proposal,
            trusted_key_fingerprints: vec![trusted_key_fingerprint],
            now_ms: now_unsigned,
            expected_outcome: ManifoldPeerDecisionOutcome::Accepted,
        };
        let (decision, application) = review_and_apply_peer_proposal(&case);
        if !application.applied {
            return CheckInReceipt {
                schema: "rusty.fleet.checkin_receipt.v1".to_owned(),
                checkin_id,
                accepted: false,
                rejection_reason: Some(CheckInRejectionReason::ManifoldRejected),
                manifold_decision: Some(decision),
                manifold_application: Some(application),
                fleet_decision: None,
            };
        }
        if let Some(state) = decision.accepted_state.clone() {
            self.accepted_peers = state;
            if self.accepted_peers.applied_proposal_ids.len() > MAX_SEEN_CHECKINS {
                let remove = self.accepted_peers.applied_proposal_ids.len() - MAX_SEEN_CHECKINS;
                self.accepted_peers.applied_proposal_ids.drain(..remove);
            }
        }
        *hub = candidate_hub;
        self.termux_proofs = candidate_termux_proofs;
        self.seen_checkins
            .insert(checkin_id.clone(), signed.claims.expires_at_ms);
        CheckInReceipt {
            schema: "rusty.fleet.checkin_receipt.v1".to_owned(),
            checkin_id,
            accepted: true,
            rejection_reason: None,
            manifold_decision: Some(decision),
            manifold_application: Some(application),
            fleet_decision: Some(fleet_decision),
        }
    }
}

fn signed_termux_proof_from_capability(
    signed: &SignedFleetCheckIn,
    proposal: &ManifoldPeerStatusProposal,
) -> Result<Option<QuestWifiAdbTermuxProof>, ()> {
    let capability = signed
        .claims
        .observation
        .capabilities
        .get(QUEST_WIFI_ADB_TERMUX_CAPABILITY_ID);
    let proposal_has_capability = proposal
        .status
        .capability_ids
        .iter()
        .any(|value| value.as_str() == QUEST_WIFI_ADB_TERMUX_CAPABILITY_ID);
    if capability.is_some() != proposal_has_capability {
        return Err(());
    }
    let Some(capability) = capability else {
        return Ok(None);
    };
    if capability.capability_id != QUEST_WIFI_ADB_TERMUX_CAPABILITY_ID
        || capability.owner != QUEST_WIFI_ADB_TERMUX_PROOF_OWNER
        || capability.reason != "loopback_tls_shell_uid_2000"
        || capability.support != SupportState::Supported
        || capability.enablement != EnablementState::Enabled
        || capability.authorization != AuthorizationState::Authorized
        || capability.reachability != ReachabilityState::Reachable
        || capability.freshness != FreshnessState::Current
        || capability.evidence_revision == 0
        || capability.observed_at_ms < 0
        || capability.observed_at_ms > signed.claims.issued_at_ms
        || capability.observed_at_ms > signed.claims.observation.source_time_ms
        || capability.fresh_until_ms <= signed.claims.issued_at_ms
        || capability.fresh_until_ms <= signed.claims.observation.source_time_ms
        || capability.fresh_until_ms > signed.claims.expires_at_ms
        || capability
            .fresh_until_ms
            .checked_sub(capability.observed_at_ms)
            .is_none_or(|lifetime| lifetime == 0 || lifetime > 60_000)
    {
        return Err(());
    }
    let extensions = serde_json::from_value::<LoopbackAdbCapabilityExtensions>(
        serde_json::to_value(&capability.extensions).map_err(|_| ())?,
    )
    .map_err(|_| ())?;
    if extensions.proof_schema != "rusty.quest.loopback_adb_proof.v1"
        || extensions.state != "available"
        || extensions.route_mode != "modern_tls"
        || !matches!(extensions.discovery_mode.as_str(), "tls_nsd" | "tls_mdns")
        || !extensions.listener_discovered
        || extensions.shell_uid != "2000"
        || !is_lower_sha256(&extensions.owner_evidence_sha256)
    {
        return Err(());
    }
    let signing_bytes = signed.claims.signing_bytes().map_err(|_| ())?;
    let claims_digest = hex::encode(Sha256::digest(signing_bytes));
    Ok(Some(QuestWifiAdbTermuxProof {
        schema: QUEST_WIFI_ADB_TERMUX_PROOF_SCHEMA.to_owned(),
        proof_id: format!("termux-proof-{}", &claims_digest[..32]),
        owner_id: QUEST_WIFI_ADB_TERMUX_PROOF_OWNER.to_owned(),
        device_id: signed.claims.observation.identity.device_id.clone(),
        identity_revision: signed.claims.observation.identity.identity_revision,
        source_epoch: signed.claims.observation.source_epoch.clone(),
        source_revision: signed.claims.observation.source_revision,
        evidence_revision: capability.evidence_revision,
        route_mode: QuestWifiAdbRouteMode::ModernTls,
        discovery_mode: extensions.discovery_mode,
        listener_discovered: true,
        shell_identity: Some(TERMUX_ADB_SHELL_IDENTITY.to_owned()),
        available: true,
        evidence_sha256: extensions.owner_evidence_sha256,
        observed_at_ms: capability.observed_at_ms,
        fresh_until_ms: capability.fresh_until_ms,
    }))
}

fn validate_signed_termux_proof(
    _hub: &FleetHub,
    signed: &SignedFleetCheckIn,
    proof: &QuestWifiAdbTermuxProof,
    now_ms: i64,
    existing: &BTreeMap<String, AdmittedQuestWifiAdbTermuxProof>,
) -> Result<(), String> {
    if proof.schema != QUEST_WIFI_ADB_TERMUX_PROOF_SCHEMA
        || proof.validate().is_err()
        || proof.device_id != signed.claims.observation.identity.device_id
        || proof.identity_revision != signed.claims.observation.identity.identity_revision
        || proof.source_epoch != signed.claims.observation.source_epoch
        || proof.observed_at_ms > signed.claims.issued_at_ms
        || proof.observed_at_ms > signed.claims.observation.source_time_ms
        || signed.claims.issued_at_ms >= proof.fresh_until_ms
        || signed.claims.observation.source_time_ms >= proof.fresh_until_ms
        || proof.fresh_until_ms > signed.claims.expires_at_ms
        || proof.observed_at_ms > now_ms
        || existing.contains_key(&proof.proof_id)
        || proof.source_revision != signed.claims.observation.source_revision
        || existing.values().any(|candidate| {
            candidate.proof.device_id == proof.device_id
                && candidate.proof.source_epoch == proof.source_epoch
                && candidate.proof.source_revision >= proof.source_revision
        })
    {
        return Err("signed Termux proof identity, freshness, or replay binding failed".to_owned());
    }
    Ok(())
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn verify_signature(public_key_hex: &str, signed: &SignedFleetCheckIn) -> bool {
    let Ok(public_key_bytes) = hex::decode(public_key_hex) else {
        return false;
    };
    let Ok(public_key_array) = <[u8; 32]>::try_from(public_key_bytes) else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&public_key_array) else {
        return false;
    };
    let Ok(signature_bytes) = hex::decode(&signed.signature_hex) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(&signature_bytes) else {
        return false;
    };
    let Ok(message) = signed.claims.signing_bytes() else {
        return false;
    };
    verifying_key.verify_strict(&message, &signature).is_ok()
}

fn rejected(checkin_id: String, reason: CheckInRejectionReason) -> CheckInReceipt {
    CheckInReceipt {
        schema: "rusty.fleet.checkin_receipt.v1".to_owned(),
        checkin_id,
        accepted: false,
        rejection_reason: Some(reason),
        manifold_decision: None,
        manifold_application: None,
        fleet_decision: None,
    }
}

fn schema_id(value: &str) -> rusty_manifold_model::SchemaId {
    rusty_manifold_model::SchemaId::new(value.to_owned()).expect("static schema id")
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};
    use fleet_contracts::{
        AuthorizationState, CHECKIN_SIGNATURE_ALGORITHM, CHECKIN_SIGNATURE_DOMAIN, CapabilityState,
        EnablementState, FleetCheckInClaims, FreshnessState, QUEST_WIFI_ADB_ACTION_ID,
        QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA, QUEST_WIFI_ADB_RECEIPT_SCHEMA,
        QUEST_WIFI_ADB_TERMUX_CAPABILITY_ID, QUEST_WIFI_ADB_TERMUX_PROOF_OWNER,
        QUEST_WIFI_ADB_TERMUX_PROOF_SCHEMA, QuestWifiAdbAction, QuestWifiAdbOwnerReceipt,
        QuestWifiAdbPreviewRequest, QuestWifiAdbRouteMode, QuestWifiAdbTermuxProof,
        QuestWifiAdbWearerApproval, ReachabilityState, SignedFleetCheckIn, SupportState,
        TERMUX_ADB_SHELL_IDENTITY,
    };
    use fleet_hub::{FleetApi, FleetHub, HubPolicy, QuestWifiAdbPreviewPlan};
    use fleet_simulator::{BASE_TIME_MS, ScenarioBuilder};
    use rusty_manifold_model::{DottedId, Revision, SchemaId};
    use rusty_manifold_peer::{
        ManifoldPeerAvailability, ManifoldPeerCredentialAlgorithm, ManifoldPeerCredentialRecord,
        ManifoldPeerCredentialStatus, ManifoldPeerEnrollmentAction, ManifoldPeerEnrollmentRequest,
        ManifoldPeerIdentity, ManifoldPeerPayloadClass, ManifoldPeerRole, ManifoldPeerStatus,
        ManifoldPeerStatusProposal,
    };
    use serde_json::json;
    use sha2::{Digest, Sha256};

    use super::{
        CheckInRejectionReason, FleetManifoldAdapter, FleetManifoldAdapterSnapshot,
        KioskShowControlsCommandAuthorization, QuestWifiAdbCommandAuthorization,
        kiosk_manifold_request_id, quest_wifi_adb_manifold_request_id,
        signed_termux_proof_from_capability,
    };
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn kiosk_command_authority_binds_typed_params_and_survives_restart() {
        let mut adapter = FleetManifoldAdapter::new(vec![dotted("operator.local")]);
        let owner_action_request_id = "owner-action-0001";
        let authorization = KioskShowControlsCommandAuthorization {
            manifold_request_id: kiosk_manifold_request_id(
                "operation-1",
                "device-1",
                owner_action_request_id,
            ),
            owner_action_request_id: owner_action_request_id.to_owned(),
            requester_id: "operator.local".to_owned(),
            operation_id: "operation-1".to_owned(),
            preview_id: "preview-1".to_owned(),
            device_id: "device-1".to_owned(),
            identity_revision: 7,
            issued_at_ms: 2_000,
            expires_at_ms: 92_000,
        };
        let receipt = adapter
            .authorize_kiosk_show_controls(&authorization, 2_000)
            .expect("Manifold authority applies exact Kiosk command");
        assert!(receipt.application.applied);
        let digest = receipt
            .request
            .params_digest
            .as_ref()
            .expect("typed parameters are mandatory");
        assert!(digest.canonical_sha256.starts_with("sha256:"));
        assert!(digest.canonical_size_bytes > 0);
        assert_eq!(adapter.runtime_host_snapshot().authority_revision.get(), 2);
        assert!(adapter.has_applied_kiosk_authorization(
            "operation-1",
            "device-1",
            owner_action_request_id
        ));

        let snapshot_json =
            serde_json::to_string(&adapter.snapshot()).expect("snapshot serializes");
        let snapshot: FleetManifoldAdapterSnapshot =
            serde_json::from_str(&snapshot_json).expect("snapshot deserializes");
        let mut restarted = FleetManifoldAdapter::new(vec![dotted("operator.local")]);
        restarted
            .restore_session(snapshot, 2_001)
            .expect("runtime authority restarts");
        assert_eq!(
            restarted.runtime_host_snapshot().authority_revision.get(),
            2
        );
        assert!(
            restarted
                .authorize_kiosk_show_controls(&authorization, 2_001)
                .expect_err("the same authority request cannot replay")
                .contains("ReplayedRequest")
        );
    }

    #[test]
    fn wifi_adb_command_authority_binds_typed_params_and_rejects_replay() {
        let mut adapter = FleetManifoldAdapter::new(vec![dotted("operator.local")]);
        let owner_action_request_id = "owner-action-wifi-adb-0001";
        let authorization = QuestWifiAdbCommandAuthorization {
            manifold_request_id: quest_wifi_adb_manifold_request_id(
                "operation-wifi-adb-1",
                "device.quest.1",
                owner_action_request_id,
            ),
            owner_action_request_id: owner_action_request_id.to_owned(),
            requester_id: "operator.local".to_owned(),
            operation_id: "operation-wifi-adb-1".to_owned(),
            preview_id: "preview-wifi-adb-1".to_owned(),
            device_id: "device.quest.1".to_owned(),
            identity_revision: 7,
            action: QuestWifiAdbAction::RequestWirelessAdb,
            issued_at_ms: 2_000,
            expires_at_ms: 92_000,
        };
        let receipt = adapter
            .authorize_quest_wifi_adb(&authorization, 2_000)
            .expect("Manifold authority applies exact Quest Wi-Fi ADB command");
        assert!(receipt.application.applied);
        let digest = receipt
            .request
            .params_digest
            .as_ref()
            .expect("typed parameters are mandatory");
        assert_eq!(
            digest.params_type_id.as_str(),
            "rusty.fleet.quest.wifi-adb-control.params.v1"
        );
        assert!(digest.canonical_sha256.starts_with("sha256:"));
        assert!(
            adapter
                .authorize_quest_wifi_adb(&authorization, 2_001)
                .expect_err("the same authority request cannot replay")
                .contains("ReplayedRequest")
        );
    }

    #[test]
    fn kiosk_command_authority_rejects_overlong_expiry_and_terminal_revision() {
        let owner_action_request_id = "owner-action-0001";
        let authorization = KioskShowControlsCommandAuthorization {
            manifold_request_id: kiosk_manifold_request_id(
                "operation-1",
                "device-1",
                owner_action_request_id,
            ),
            owner_action_request_id: owner_action_request_id.to_owned(),
            requester_id: "operator.local".to_owned(),
            operation_id: "operation-1".to_owned(),
            preview_id: "preview-1".to_owned(),
            device_id: "device-1".to_owned(),
            identity_revision: 7,
            issued_at_ms: 2_000,
            expires_at_ms: 92_001,
        };
        let mut adapter = FleetManifoldAdapter::new(vec![dotted("operator.local")]);
        assert!(
            adapter
                .authorize_kiosk_show_controls(&authorization, 2_000)
                .is_err()
        );

        let mut snapshot_value =
            serde_json::to_value(adapter.snapshot()).expect("snapshot serializes");
        snapshot_value["runtime_host"]["authority_revision"] = serde_json::json!(u64::MAX);
        let snapshot: FleetManifoldAdapterSnapshot =
            serde_json::from_value(snapshot_value).expect("terminal snapshot deserializes");
        assert!(
            FleetManifoldAdapter::new(vec![dotted("operator.local")])
                .restore_session(snapshot, 2_000)
                .expect_err("terminal revision recovery must fail")
                .contains("terminal revision")
        );
    }

    #[test]
    fn authenticated_checkin_is_admitted_once_by_manifold_and_fleet() {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let public_key = signing_key.verifying_key().to_bytes();
        let digest = hex::encode(Sha256::digest(public_key));
        let peer_id = dotted("device.quest.1");
        let key_id = dotted("key.device.quest.1");
        let fingerprint = dotted(format!("fingerprint.{digest}").as_str());
        let operator = dotted("operator.local");

        let mut adapter = FleetManifoldAdapter::new(vec![operator.clone()]);
        let enrollment = ManifoldPeerEnrollmentRequest {
            schema_id: schema("rusty.manifold.peer.enrollment_request.v1"),
            request_id: dotted("request.enroll.quest.1"),
            expected_authority_revision: Revision::INITIAL,
            operator_id: operator,
            issued_at_ms: u64::try_from(BASE_TIME_MS).expect("positive time"),
            action: ManifoldPeerEnrollmentAction::Enroll {
                credential: ManifoldPeerCredentialRecord {
                    schema_id: schema("rusty.manifold.peer.credential_record.v1"),
                    credential_id: dotted("credential.device.quest.1"),
                    peer_id: peer_id.clone(),
                    trust_domain: dotted("trust.local"),
                    key_id: key_id.clone(),
                    key_generation: 1,
                    algorithm: ManifoldPeerCredentialAlgorithm::Ed25519,
                    public_key_hex: hex::encode(public_key),
                    public_key_sha256: format!("sha256:{digest}"),
                    valid_from_ms: u64::try_from(BASE_TIME_MS - 1_000).expect("positive time"),
                    expires_at_ms: u64::try_from(BASE_TIME_MS + 600_000).expect("positive time"),
                    status: ManifoldPeerCredentialStatus::Active,
                    replaced_by_key_id: None,
                },
            },
        };
        assert!(
            adapter
                .apply_enrollment(
                    &enrollment,
                    u64::try_from(BASE_TIME_MS).expect("positive time")
                )
                .applied
        );

        let mut observation = ScenarioBuilder::new(4).build().initial.remove(0);
        observation.identity.device_id = peer_id.to_string();
        observation.received_time_ms = 0;
        observation.source_time_ms = BASE_TIME_MS;
        let proposal = ManifoldPeerStatusProposal {
            schema_id: schema("rusty.manifold.peer.status_proposal.v1"),
            proposal_id: dotted("proposal.status.quest.1"),
            expected_authority_revision: Revision::INITIAL,
            proposer_id: dotted("adapter.quest.fleet-agent"),
            identity: ManifoldPeerIdentity {
                schema_id: schema("rusty.manifold.peer.identity.v1"),
                peer_id: peer_id.clone(),
                key_fingerprint: fingerprint,
                trust_domain: dotted("trust.local"),
                roles: vec![ManifoldPeerRole::Observer],
            },
            status: ManifoldPeerStatus {
                schema_id: schema("rusty.manifold.peer.status.v1"),
                peer_id,
                status_revision: Revision::INITIAL,
                observed_at_ms: u64::try_from(BASE_TIME_MS).expect("positive time"),
                expires_at_ms: u64::try_from(BASE_TIME_MS + 60_000).expect("positive time"),
                availability: ManifoldPeerAvailability::Ready,
                capability_ids: vec![dotted("capability.monitoring")],
            },
            payload_class: ManifoldPeerPayloadClass::LowRateDescriptor,
        };
        let claims = FleetCheckInClaims {
            schema: "rusty.fleet.checkin_claims.v1".to_owned(),
            checkin_id: "checkin.quest.1".to_owned(),
            issued_at_ms: BASE_TIME_MS,
            expires_at_ms: BASE_TIME_MS + 60_000,
            manifold_peer_status_proposal: serde_json::to_value(proposal)
                .expect("proposal serialization"),
            observation,
            extensions: Default::default(),
        };
        let signed = sign_checkin(&signing_key, key_id.as_str(), claims);

        let mut hub = FleetHub::new(HubPolicy::default());
        let receipt = adapter.accept(&mut hub, signed.clone(), BASE_TIME_MS + 1);
        assert!(receipt.accepted);
        assert_eq!(hub.device_count(), 1);
        assert!(
            receipt
                .manifold_application
                .is_some_and(|value| value.applied)
        );

        let replay = adapter.accept(&mut hub, signed.clone(), BASE_TIME_MS + 2);
        assert!(!replay.accepted);
        assert_eq!(
            replay.rejection_reason,
            Some(CheckInRejectionReason::Replay)
        );
        assert_eq!(hub.device_count(), 1);

        let first_authority_revision = adapter.accepted_peers().authority_revision;
        let mut next_claims = signed.claims.clone();
        next_claims.checkin_id = "checkin.quest.2".to_owned();
        let mut next_proposal: ManifoldPeerStatusProposal =
            serde_json::from_value(next_claims.manifold_peer_status_proposal.clone())
                .expect("proposal");
        next_proposal.proposal_id = dotted("proposal.status.quest.2");
        // Devices do not serialize against the fleet-global Manifold
        // authority revision. Ingress binds the signed status to current
        // authority immediately before review.
        next_proposal.expected_authority_revision = Revision::INITIAL;
        next_proposal.status.status_revision = Revision::new(2).expect("second status revision");
        next_claims.manifold_peer_status_proposal =
            serde_json::to_value(next_proposal).expect("proposal serialization");
        next_claims.observation.source_revision = 2;
        let next = sign_checkin(&signing_key, key_id.as_str(), next_claims);
        let next_receipt = adapter.accept(&mut hub, next, BASE_TIME_MS + 3);
        assert!(next_receipt.accepted);
        assert_eq!(
            adapter.accepted_peers().authority_revision.get(),
            first_authority_revision.get().saturating_add(1)
        );

        let prior_authority_revision = adapter.accepted_peers().authority_revision;
        let prior_proposal_count = adapter.accepted_peers().applied_proposal_ids.len();
        let mut fleet_rejected_claims = signed.claims;
        fleet_rejected_claims.checkin_id = "checkin.quest.3".to_owned();
        let mut rejected_proposal: ManifoldPeerStatusProposal =
            serde_json::from_value(fleet_rejected_claims.manifold_peer_status_proposal.clone())
                .expect("proposal");
        rejected_proposal.proposal_id = dotted("proposal.status.quest.3");
        rejected_proposal.expected_authority_revision = Revision::INITIAL;
        rejected_proposal.status.status_revision = Revision::new(3).expect("third status revision");
        fleet_rejected_claims.manifold_peer_status_proposal =
            serde_json::to_value(rejected_proposal).expect("proposal serialization");
        fleet_rejected_claims.observation.source_revision = 2;
        let fleet_rejected = sign_checkin(&signing_key, key_id.as_str(), fleet_rejected_claims);
        let fleet_rejection = adapter.accept(&mut hub, fleet_rejected, BASE_TIME_MS + 4);
        assert!(!fleet_rejection.accepted);
        assert_eq!(
            fleet_rejection.rejection_reason,
            Some(CheckInRejectionReason::FleetRejected)
        );
        assert!(fleet_rejection.manifold_decision.is_none());
        assert_eq!(
            adapter.accepted_peers().authority_revision,
            prior_authority_revision
        );
        assert_eq!(
            adapter.accepted_peers().applied_proposal_ids.len(),
            prior_proposal_count
        );
    }

    #[test]
    fn key_rotation_rejects_the_old_signer_and_accepts_a_fresh_source_epoch() {
        let original_key = SigningKey::from_bytes(&[17_u8; 32]);
        let replacement_key = SigningKey::from_bytes(&[23_u8; 32]);
        let peer_id = dotted("device.quest.rotation");
        let original_key_id = dotted("key.device.quest.rotation.1");
        let replacement_key_id = dotted("key.device.quest.rotation.2");
        let operator = dotted("operator.local");
        let mut adapter = FleetManifoldAdapter::new(vec![operator.clone()]);
        let mut hub = FleetHub::new(HubPolicy::default());

        let enrolled = adapter.apply_enrollment(
            &ManifoldPeerEnrollmentRequest {
                schema_id: schema("rusty.manifold.peer.enrollment_request.v1"),
                request_id: dotted("request.enroll.quest.rotation"),
                expected_authority_revision: Revision::INITIAL,
                operator_id: operator.clone(),
                issued_at_ms: u64::try_from(BASE_TIME_MS).expect("positive time"),
                action: ManifoldPeerEnrollmentAction::Enroll {
                    credential: credential(&peer_id, &original_key_id, 1, &original_key),
                },
            },
            u64::try_from(BASE_TIME_MS).expect("positive time"),
        );
        assert!(enrolled.applied);

        let first = synthetic_checkin(
            &original_key,
            &original_key_id,
            &peer_id,
            "checkin.quest.rotation.1",
            "proposal.status.quest.rotation.1",
            1,
            "agent-epoch-1",
            1,
        );
        assert!(adapter.accept(&mut hub, first, BASE_TIME_MS + 1).accepted);
        let revision_before_rotation = hub.result_revision();

        let rotated = adapter.apply_enrollment(
            &ManifoldPeerEnrollmentRequest {
                schema_id: schema("rusty.manifold.peer.enrollment_request.v1"),
                request_id: dotted("request.rotate.quest.rotation"),
                expected_authority_revision: adapter.enrollment().authority_revision,
                operator_id: operator,
                issued_at_ms: u64::try_from(BASE_TIME_MS + 2).expect("positive time"),
                action: ManifoldPeerEnrollmentAction::Rotate {
                    prior_key_id: original_key_id.clone(),
                    credential: credential(&peer_id, &replacement_key_id, 2, &replacement_key),
                },
            },
            u64::try_from(BASE_TIME_MS + 2).expect("positive time"),
        );
        assert!(rotated.applied);
        assert_eq!(
            adapter
                .enrollment()
                .credentials
                .iter()
                .find(|credential| credential.key_id == original_key_id)
                .map(|credential| credential.status.clone()),
            Some(ManifoldPeerCredentialStatus::Rotated)
        );

        let old_signer = synthetic_checkin(
            &original_key,
            &original_key_id,
            &peer_id,
            "checkin.quest.rotation.old",
            "proposal.status.quest.rotation.old",
            2,
            "agent-epoch-1",
            2,
        );
        let old_rejection = adapter.accept(&mut hub, old_signer, BASE_TIME_MS + 3);
        assert!(!old_rejection.accepted);
        assert_eq!(
            old_rejection.rejection_reason,
            Some(CheckInRejectionReason::UnknownOrInactiveKey)
        );
        assert_eq!(hub.result_revision(), revision_before_rotation);

        let replacement = synthetic_checkin(
            &replacement_key,
            &replacement_key_id,
            &peer_id,
            "checkin.quest.rotation.2",
            "proposal.status.quest.rotation.2",
            2,
            "agent-epoch-2",
            1,
        );
        let replacement_receipt = adapter.accept(&mut hub, replacement, BASE_TIME_MS + 4);
        assert!(replacement_receipt.accepted);
        assert_eq!(hub.device_count(), 1);
        assert_eq!(
            hub.inspect(peer_id.as_str(), BASE_TIME_MS + 4)
                .expect("rotated device")
                .row
                .source_epoch,
            "agent-epoch-2"
        );
        assert_eq!(
            adapter.accepted_peers().peers[0]
                .status
                .status_revision
                .get(),
            2
        );
    }

    #[test]
    fn malformed_checkin_rejects_before_authority_mutation() {
        let mut adapter = FleetManifoldAdapter::new(vec![dotted("operator.local")]);
        let mut observation = ScenarioBuilder::new(4).build().initial.remove(0);
        observation.identity.device_id = "device.quest.2".to_owned();
        let checkin = SignedFleetCheckIn {
            schema: "rusty.fleet.signed_checkin.v1".to_owned(),
            key_id: "key.device.quest.2".to_owned(),
            algorithm: CHECKIN_SIGNATURE_ALGORITHM.to_owned(),
            signature_hex: "00".repeat(64),
            claims: FleetCheckInClaims {
                schema: "rusty.fleet.checkin_claims.v1".to_owned(),
                checkin_id: "checkin quest 2".to_owned(),
                issued_at_ms: BASE_TIME_MS,
                expires_at_ms: BASE_TIME_MS + 60_000,
                manifold_peer_status_proposal: json!({"not": "a proposal"}),
                observation,
                extensions: Default::default(),
            },
        };
        let mut hub = FleetHub::new(HubPolicy::default());
        let receipt = adapter.accept(&mut hub, checkin, BASE_TIME_MS + 1);
        assert!(!receipt.accepted);
        assert_eq!(
            receipt.rejection_reason,
            Some(CheckInRejectionReason::ContractInvalid)
        );
        assert_eq!(hub.device_count(), 0);
        assert!(adapter.accepted_peers().peers.is_empty());
    }

    #[test]
    fn canonical_signing_vector_is_stable() {
        let claim_fixture = include_bytes!("../../../fixtures/contracts/checkin-claims.valid.json");
        let claims: FleetCheckInClaims =
            serde_json::from_slice(claim_fixture).expect("valid committed check-in claims");
        let vector: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/contracts/checkin-signing-vector.valid.json"
        ))
        .expect("valid committed signing vector");
        let seed = hex::decode(
            vector["private_seed_hex"]
                .as_str()
                .expect("test seed string"),
        )
        .expect("test seed hex");
        let signing_key =
            SigningKey::from_bytes(&<[u8; 32]>::try_from(seed).expect("32-byte test seed"));
        let signed = sign_checkin(&signing_key, "key.fixture.synthetic.1", claims);
        assert_eq!(
            signed.signature_hex,
            vector["signature_hex"].as_str().expect("signature string")
        );
        assert_eq!(
            hex::encode(signing_key.verifying_key().to_bytes()),
            vector["public_key_hex"]
                .as_str()
                .expect("public-key string")
        );
        assert_eq!(
            hex::encode(Sha256::digest(
                signed
                    .claims
                    .signing_bytes()
                    .expect("canonical signing bytes")
            )),
            vector["signing_message_sha256"]
                .as_str()
                .expect("message digest string")
        );
        let signing_bytes = signed
            .claims
            .signing_bytes()
            .expect("canonical signing bytes");
        assert_eq!(
            hex::encode(Sha256::digest(
                &signing_bytes[CHECKIN_SIGNATURE_DOMAIN.len()..]
            )),
            vector["claims_jcs_sha256"]
                .as_str()
                .expect("canonical claims digest string")
        );
        assert_eq!(
            std::str::from_utf8(CHECKIN_SIGNATURE_DOMAIN).expect("UTF-8 domain"),
            vector["signature_domain_utf8"]
                .as_str()
                .expect("signature domain string")
        );
    }

    #[test]
    fn signed_termux_capability_is_admitted_replayed_and_superseded_fail_closed() {
        let signing_key = SigningKey::from_bytes(&[41_u8; 32]);
        let peer_id = dotted("device.quest.proof");
        let key_id = dotted("key.device.quest.proof.1");
        let operator = dotted("operator.local");
        let mut adapter = FleetManifoldAdapter::new(vec![operator.clone()]);
        let enrollment = adapter.apply_enrollment(
            &ManifoldPeerEnrollmentRequest {
                schema_id: schema("rusty.manifold.peer.enrollment_request.v1"),
                request_id: dotted("request.enroll.quest.proof"),
                expected_authority_revision: Revision::INITIAL,
                operator_id: operator,
                issued_at_ms: u64::try_from(BASE_TIME_MS).expect("positive"),
                action: ManifoldPeerEnrollmentAction::Enroll {
                    credential: credential(&peer_id, &key_id, 1, &signing_key),
                },
            },
            u64::try_from(BASE_TIME_MS).expect("positive"),
        );
        assert!(enrollment.applied);
        let mut hub = FleetHub::new(HubPolicy::default());
        let first = synthetic_checkin(
            &signing_key,
            &key_id,
            &peer_id,
            "checkin.quest.proof.1",
            "proposal.status.quest.proof.1",
            1,
            "agent-epoch-1",
            1,
        );
        assert!(adapter.accept(&mut hub, first, BASE_TIME_MS).accepted);
        let identity_revision = hub
            .inspect(peer_id.as_str(), BASE_TIME_MS)
            .expect("device")
            .row
            .identity
            .identity_revision;
        let operation = install_wifi_request_receipt(
            &mut hub,
            peer_id.as_str(),
            identity_revision,
            BASE_TIME_MS,
        );

        let available = termux_proof(
            peer_id.as_str(),
            identity_revision,
            "termux-proof-available-1",
            2,
            true,
        );
        let signed_available = proof_checkin(
            &signing_key,
            &key_id,
            &peer_id,
            "checkin.quest.proof.2",
            "proposal.status.quest.proof.2",
            2,
            2,
            available.clone(),
        );
        assert!(
            adapter
                .accept(&mut hub, signed_available.clone(), BASE_TIME_MS + 1)
                .accepted
        );
        assert_eq!(
            adapter.termux_proofs.len(),
            1,
            "only the current per-device proof is retained"
        );
        let projected = adapter
            .quest_wifi_adb_operation(&hub, &operation.operation_id, BASE_TIME_MS + 1)
            .expect("authenticated proof projection");
        assert!(projected.targets[0].termux_usable);

        let replay_receipt = adapter.accept(&mut hub, signed_available, BASE_TIME_MS + 2);
        assert!(!replay_receipt.accepted);
        assert_eq!(
            replay_receipt.rejection_reason,
            Some(CheckInRejectionReason::Replay)
        );

        let other_device = termux_proof(
            peer_id.as_str(),
            identity_revision,
            "termux-proof-other-device",
            3,
            true,
        );
        let mut other = proof_checkin(
            &signing_key,
            &key_id,
            &peer_id,
            "checkin.quest.proof.other",
            "proposal.status.quest.proof.other",
            3,
            3,
            other_device,
        );
        other.claims.observation.identity.device_id = "device.quest.other".to_owned();
        let other_receipt = adapter.accept(&mut hub, other, BASE_TIME_MS + 2);
        assert_eq!(
            other_receipt.rejection_reason,
            Some(CheckInRejectionReason::IdentityMismatch)
        );

        let unavailable = termux_proof(
            peer_id.as_str(),
            identity_revision,
            "termux-proof-unavailable-3",
            3,
            false,
        );
        let signed_unavailable = proof_checkin(
            &signing_key,
            &key_id,
            &peer_id,
            "checkin.quest.proof.3",
            "proposal.status.quest.proof.3",
            3,
            3,
            unavailable,
        );
        assert!(
            adapter
                .accept(&mut hub, signed_unavailable, BASE_TIME_MS + 2)
                .accepted
        );
        assert!(
            adapter.termux_proofs.is_empty(),
            "a newer signed snapshot without the capability clears the old proof"
        );
        let projected = adapter
            .quest_wifi_adb_operation(&hub, &operation.operation_id, BASE_TIME_MS + 2)
            .expect("unavailable projection");
        assert!(!projected.targets[0].termux_usable);

        let renewed = termux_proof(
            peer_id.as_str(),
            identity_revision,
            "termux-proof-renewed-4",
            4,
            true,
        );
        let signed_renewed = proof_checkin(
            &signing_key,
            &key_id,
            &peer_id,
            "checkin.quest.proof.4",
            "proposal.status.quest.proof.4",
            4,
            4,
            renewed,
        );
        assert!(
            adapter
                .accept(&mut hub, signed_renewed, BASE_TIME_MS + 3)
                .accepted
        );
        assert_eq!(
            adapter.termux_proofs.len(),
            1,
            "renewal replaces rather than accumulates proof evidence"
        );
        assert!(
            adapter
                .quest_wifi_adb_operation(&hub, &operation.operation_id, BASE_TIME_MS + 3)
                .expect("renewed projection")
                .targets[0]
                .termux_usable
        );
        install_wifi_disable_receipt(
            &mut hub,
            peer_id.as_str(),
            identity_revision,
            BASE_TIME_MS + 4,
        );
        assert!(
            !adapter
                .quest_wifi_adb_operation(&hub, &operation.operation_id, BASE_TIME_MS + 4)
                .expect("disable supersession")
                .targets[0]
                .termux_usable
        );

        let reboot = synthetic_checkin(
            &signing_key,
            &key_id,
            &peer_id,
            "checkin.quest.proof.reboot",
            "proposal.status.quest.proof.reboot",
            5,
            "agent-epoch-2",
            1,
        );
        assert!(adapter.accept(&mut hub, reboot, BASE_TIME_MS + 5).accepted);
        let reboot_projection = adapter
            .quest_wifi_adb_operation(&hub, &operation.operation_id, BASE_TIME_MS + 5)
            .expect("reboot projection");
        assert!(!reboot_projection.targets[0].termux_usable);
        assert!(reboot_projection.targets[0].termux_proof.is_none());
        assert!(
            !adapter
                .quest_wifi_adb_operation(&hub, &operation.operation_id, BASE_TIME_MS + 60_000)
                .expect("expired projection")
                .targets[0]
                .termux_usable
        );
    }

    #[test]
    fn exact_rusty_quest_capability_fixture_derives_the_internal_proof() {
        let signing_key = SigningKey::from_bytes(&[47_u8; 32]);
        let key_id = dotted("key.device.quest.fixture.1");
        let peer_id = dotted("device.quest.fixture");
        let base = synthetic_checkin(
            &signing_key,
            &key_id,
            &peer_id,
            "checkin.quest.fixture.2",
            "proposal.status.quest.fixture.2",
            2,
            "agent-epoch-1",
            2,
        );
        let mut claims = base.claims;
        let capability: CapabilityState = serde_json::from_str(include_str!(
            "../../../fixtures/contracts/quest-loopback-adb-capability.valid.json"
        ))
        .expect("exact Rusty Quest capability fixture");
        claims
            .observation
            .capabilities
            .capabilities
            .insert(QUEST_WIFI_ADB_TERMUX_CAPABILITY_ID.to_owned(), capability);
        let mut proposal: ManifoldPeerStatusProposal =
            serde_json::from_value(claims.manifold_peer_status_proposal.clone()).expect("proposal");
        proposal
            .status
            .capability_ids
            .push(dotted(QUEST_WIFI_ADB_TERMUX_CAPABILITY_ID));
        claims.manifold_peer_status_proposal =
            serde_json::to_value(&proposal).expect("proposal JSON");
        let signed = sign_checkin(&signing_key, key_id.as_str(), claims);
        let proof = signed_termux_proof_from_capability(&signed, &proposal)
            .expect("strict capability")
            .expect("derived proof");
        assert_eq!(proof.owner_id, "quest-termux-lab");
        assert_eq!(proof.evidence_revision, 17);
        assert_eq!(proof.discovery_mode, "tls_nsd");
        assert_eq!(
            proof.shell_identity.as_deref(),
            Some(TERMUX_ADB_SHELL_IDENTITY)
        );
        assert_eq!(
            proof.evidence_sha256,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn signed_termux_capability_rejects_signature_mutation_and_stale_freshness() {
        let signing_key = SigningKey::from_bytes(&[43_u8; 32]);
        let peer_id = dotted("device.quest.proof-mutation");
        let key_id = dotted("key.device.quest.proof-mutation.1");
        let operator = dotted("operator.local");
        let mut adapter = FleetManifoldAdapter::new(vec![operator.clone()]);
        assert!(
            adapter
                .apply_enrollment(
                    &ManifoldPeerEnrollmentRequest {
                        schema_id: schema("rusty.manifold.peer.enrollment_request.v1"),
                        request_id: dotted("request.enroll.quest.proof-mutation"),
                        expected_authority_revision: Revision::INITIAL,
                        operator_id: operator,
                        issued_at_ms: u64::try_from(BASE_TIME_MS).expect("positive"),
                        action: ManifoldPeerEnrollmentAction::Enroll {
                            credential: credential(&peer_id, &key_id, 1, &signing_key),
                        },
                    },
                    u64::try_from(BASE_TIME_MS).expect("positive"),
                )
                .applied
        );
        let mut hub = FleetHub::new(HubPolicy::default());
        assert!(
            adapter
                .accept(
                    &mut hub,
                    synthetic_checkin(
                        &signing_key,
                        &key_id,
                        &peer_id,
                        "checkin.quest.proof-mutation.1",
                        "proposal.status.quest.proof-mutation.1",
                        1,
                        "agent-epoch-1",
                        1,
                    ),
                    BASE_TIME_MS,
                )
                .accepted
        );
        let identity_revision = hub
            .inspect(peer_id.as_str(), BASE_TIME_MS)
            .expect("device")
            .row
            .identity
            .identity_revision;
        let proof = termux_proof(
            peer_id.as_str(),
            identity_revision,
            "termux-proof-mutation-2",
            2,
            true,
        );
        let mut mutated = proof_checkin(
            &signing_key,
            &key_id,
            &peer_id,
            "checkin.quest.proof-mutation.2",
            "proposal.status.quest.proof-mutation.2",
            2,
            2,
            proof.clone(),
        );
        mutated
            .claims
            .observation
            .capabilities
            .capabilities
            .get_mut(QUEST_WIFI_ADB_TERMUX_CAPABILITY_ID)
            .expect("signed capability")
            .extensions
            .insert(
                "owner_evidence_sha256".to_owned(),
                serde_json::Value::String("ff".repeat(32)),
            );
        let mutated_receipt = adapter.accept(&mut hub, mutated, BASE_TIME_MS + 1);
        assert_eq!(
            mutated_receipt.rejection_reason,
            Some(CheckInRejectionReason::SignatureInvalid)
        );

        let mut stale = termux_proof(
            peer_id.as_str(),
            identity_revision,
            "termux-proof-stale-2",
            2,
            true,
        );
        stale.observed_at_ms = BASE_TIME_MS - 60_000;
        stale.fresh_until_ms = BASE_TIME_MS - 1;
        let stale_checkin = proof_checkin(
            &signing_key,
            &key_id,
            &peer_id,
            "checkin.quest.proof-stale.2",
            "proposal.status.quest.proof-stale.2",
            2,
            2,
            stale,
        );
        let stale_receipt = adapter.accept(&mut hub, stale_checkin, BASE_TIME_MS + 1);
        assert_eq!(
            stale_receipt.rejection_reason,
            Some(CheckInRejectionReason::AuthorityEvidenceMismatch)
        );
    }

    fn install_wifi_request_receipt(
        hub: &mut FleetHub,
        device_id: &str,
        identity_revision: u64,
        now_ms: i64,
    ) -> fleet_contracts::QuestWifiAdbOperation {
        let operation = hub
            .preview_quest_wifi_adb(QuestWifiAdbPreviewPlan {
                operation_id: "wifi-adb-operation-proof".to_owned(),
                preview_id: "wifi-adb-preview-proof".to_owned(),
                request: QuestWifiAdbPreviewRequest {
                    schema: QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA.to_owned(),
                    action_id: QUEST_WIFI_ADB_ACTION_ID.to_owned(),
                    action: QuestWifiAdbAction::RequestWirelessAdb,
                    targets: BTreeMap::from([(device_id.to_owned(), identity_revision)]),
                },
                created_at_ms: now_ms,
                expires_at_ms: now_ms + 60_000,
                provider_ready_devices: BTreeSet::from([device_id.to_owned()]),
            })
            .expect("Wi-Fi ADB preview");
        hub.confirm_quest_wifi_adb(
            &operation.operation_id,
            &operation.preview.preview_id,
            now_ms,
        )
        .expect("confirm");
        hub.prepare_quest_wifi_adb_invocation(
            &operation.operation_id,
            device_id,
            "request.wifi.proof".to_owned(),
            now_ms,
        )
        .expect("prepare");
        hub.mark_quest_wifi_adb_dispatched(&operation.operation_id, device_id, now_ms)
            .expect("dispatch");
        hub.apply_quest_wifi_adb_receipt(
            QuestWifiAdbOwnerReceipt {
                schema: QUEST_WIFI_ADB_RECEIPT_SCHEMA.to_owned(),
                request_id: "request.wifi.proof".to_owned(),
                operation_id: operation.operation_id.clone(),
                preview_id: operation.preview.preview_id.clone(),
                device_id: device_id.to_owned(),
                identity_revision,
                action: QuestWifiAdbAction::RequestWirelessAdb,
                route_mode: QuestWifiAdbRouteMode::ModernTls,
                request_delivered: true,
                kiosk_setting_applied: true,
                request_after_boot_enabled: None,
                wearer_approval: QuestWifiAdbWearerApproval::Pending,
                listener_discovered: false,
                effect_applied: true,
                outcome: "wireless_adb_request_applied".to_owned(),
                evidence_sha256: "11".repeat(32),
                // Simulate the Windows provider clock running ahead of the
                // device source clock. Projection must use trusted Hub
                // admission order, not compare these unrelated timestamps.
                observed_at_ms: now_ms + 20_000,
            },
            now_ms,
        )
        .expect("apply receipt")
    }

    fn install_wifi_disable_receipt(
        hub: &mut FleetHub,
        device_id: &str,
        identity_revision: u64,
        now_ms: i64,
    ) {
        let operation = hub
            .preview_quest_wifi_adb(QuestWifiAdbPreviewPlan {
                operation_id: "wifi-adb-operation-disable-proof".to_owned(),
                preview_id: "wifi-adb-preview-disable-proof".to_owned(),
                request: QuestWifiAdbPreviewRequest {
                    schema: QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA.to_owned(),
                    action_id: QUEST_WIFI_ADB_ACTION_ID.to_owned(),
                    action: QuestWifiAdbAction::DisableWirelessAdb,
                    targets: BTreeMap::from([(device_id.to_owned(), identity_revision)]),
                },
                created_at_ms: now_ms,
                expires_at_ms: now_ms + 60_000,
                provider_ready_devices: BTreeSet::from([device_id.to_owned()]),
            })
            .expect("disable preview");
        hub.confirm_quest_wifi_adb(
            &operation.operation_id,
            &operation.preview.preview_id,
            now_ms,
        )
        .expect("confirm disable");
        hub.prepare_quest_wifi_adb_invocation(
            &operation.operation_id,
            device_id,
            "request.wifi.disable-proof".to_owned(),
            now_ms,
        )
        .expect("prepare disable");
        hub.mark_quest_wifi_adb_dispatched(&operation.operation_id, device_id, now_ms)
            .expect("dispatch disable");
        hub.apply_quest_wifi_adb_receipt(
            QuestWifiAdbOwnerReceipt {
                schema: QUEST_WIFI_ADB_RECEIPT_SCHEMA.to_owned(),
                request_id: "request.wifi.disable-proof".to_owned(),
                operation_id: operation.operation_id,
                preview_id: operation.preview.preview_id,
                device_id: device_id.to_owned(),
                identity_revision,
                action: QuestWifiAdbAction::DisableWirelessAdb,
                route_mode: QuestWifiAdbRouteMode::ModernTls,
                request_delivered: true,
                kiosk_setting_applied: true,
                request_after_boot_enabled: None,
                wearer_approval: QuestWifiAdbWearerApproval::NotApplicable,
                listener_discovered: false,
                effect_applied: true,
                outcome: "wireless_adb_disabled".to_owned(),
                evidence_sha256: "55".repeat(32),
                observed_at_ms: now_ms,
            },
            now_ms,
        )
        .expect("apply disable receipt");
    }

    fn termux_proof(
        device_id: &str,
        identity_revision: u64,
        proof_id: &str,
        source_revision: u64,
        available: bool,
    ) -> QuestWifiAdbTermuxProof {
        QuestWifiAdbTermuxProof {
            schema: QUEST_WIFI_ADB_TERMUX_PROOF_SCHEMA.to_owned(),
            proof_id: proof_id.to_owned(),
            owner_id: QUEST_WIFI_ADB_TERMUX_PROOF_OWNER.to_owned(),
            device_id: device_id.to_owned(),
            identity_revision,
            source_epoch: "agent-epoch-1".to_owned(),
            source_revision,
            evidence_revision: source_revision + 10,
            route_mode: QuestWifiAdbRouteMode::ModernTls,
            discovery_mode: "tls_nsd".to_owned(),
            listener_discovered: available,
            shell_identity: available.then(|| TERMUX_ADB_SHELL_IDENTITY.to_owned()),
            available,
            evidence_sha256: "33".repeat(32),
            observed_at_ms: BASE_TIME_MS,
            fresh_until_ms: BASE_TIME_MS + 60_000,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn proof_checkin(
        signing_key: &SigningKey,
        key_id: &DottedId,
        peer_id: &DottedId,
        checkin_id: &str,
        proposal_id: &str,
        status_revision: u64,
        source_revision: u64,
        proof: QuestWifiAdbTermuxProof,
    ) -> SignedFleetCheckIn {
        let base = synthetic_checkin(
            signing_key,
            key_id,
            peer_id,
            checkin_id,
            proposal_id,
            status_revision,
            "agent-epoch-1",
            source_revision,
        );
        let mut claims = base.claims;
        if proof.available {
            claims.observation.capabilities.capabilities.insert(
                QUEST_WIFI_ADB_TERMUX_CAPABILITY_ID.to_owned(),
                CapabilityState {
                    capability_id: QUEST_WIFI_ADB_TERMUX_CAPABILITY_ID.to_owned(),
                    support: SupportState::Supported,
                    enablement: EnablementState::Enabled,
                    authorization: AuthorizationState::Authorized,
                    reachability: ReachabilityState::Reachable,
                    freshness: FreshnessState::Current,
                    evidence_revision: proof.evidence_revision,
                    observed_at_ms: proof.observed_at_ms,
                    fresh_until_ms: proof.fresh_until_ms,
                    owner: QUEST_WIFI_ADB_TERMUX_PROOF_OWNER.to_owned(),
                    reason: "loopback_tls_shell_uid_2000".to_owned(),
                    extensions: BTreeMap::from([
                        (
                            "proof_schema".to_owned(),
                            serde_json::Value::String(
                                "rusty.quest.loopback_adb_proof.v1".to_owned(),
                            ),
                        ),
                        (
                            "state".to_owned(),
                            serde_json::Value::String("available".to_owned()),
                        ),
                        (
                            "route_mode".to_owned(),
                            serde_json::Value::String("modern_tls".to_owned()),
                        ),
                        (
                            "discovery_mode".to_owned(),
                            serde_json::Value::String(proof.discovery_mode.clone()),
                        ),
                        (
                            "listener_discovered".to_owned(),
                            serde_json::Value::Bool(true),
                        ),
                        (
                            "shell_uid".to_owned(),
                            serde_json::Value::String("2000".to_owned()),
                        ),
                        (
                            "owner_evidence_sha256".to_owned(),
                            serde_json::Value::String(proof.evidence_sha256.clone()),
                        ),
                    ]),
                },
            );
            let mut proposal: ManifoldPeerStatusProposal =
                serde_json::from_value(claims.manifold_peer_status_proposal.clone())
                    .expect("proposal");
            proposal
                .status
                .capability_ids
                .push(dotted(QUEST_WIFI_ADB_TERMUX_CAPABILITY_ID));
            claims.manifold_peer_status_proposal =
                serde_json::to_value(proposal).expect("proposal JSON");
        }
        sign_checkin(signing_key, key_id.as_str(), claims)
    }

    fn dotted(value: &str) -> DottedId {
        DottedId::new(value.to_owned()).expect("dotted id")
    }

    fn schema(value: &str) -> SchemaId {
        SchemaId::new(value.to_owned()).expect("schema id")
    }

    fn credential(
        peer_id: &DottedId,
        key_id: &DottedId,
        key_generation: u64,
        signing_key: &SigningKey,
    ) -> ManifoldPeerCredentialRecord {
        let public_key = signing_key.verifying_key().to_bytes();
        let digest = hex::encode(Sha256::digest(public_key));
        ManifoldPeerCredentialRecord {
            schema_id: schema("rusty.manifold.peer.credential_record.v1"),
            credential_id: dotted(format!("credential.{}", key_id.as_str()).as_str()),
            peer_id: peer_id.clone(),
            trust_domain: dotted("trust.local"),
            key_id: key_id.clone(),
            key_generation,
            algorithm: ManifoldPeerCredentialAlgorithm::Ed25519,
            public_key_hex: hex::encode(public_key),
            public_key_sha256: format!("sha256:{digest}"),
            valid_from_ms: u64::try_from(BASE_TIME_MS - 1_000).expect("positive time"),
            expires_at_ms: u64::try_from(BASE_TIME_MS + 600_000).expect("positive time"),
            status: ManifoldPeerCredentialStatus::Active,
            replaced_by_key_id: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn synthetic_checkin(
        signing_key: &SigningKey,
        key_id: &DottedId,
        peer_id: &DottedId,
        checkin_id: &str,
        proposal_id: &str,
        status_revision: u64,
        source_epoch: &str,
        source_revision: u64,
    ) -> SignedFleetCheckIn {
        let digest = hex::encode(Sha256::digest(signing_key.verifying_key().to_bytes()));
        let mut observation = ScenarioBuilder::new(1).build().initial.remove(0);
        observation.identity.device_id = peer_id.to_string();
        observation.source_epoch = source_epoch.to_owned();
        observation.source_revision = source_revision;
        observation.received_time_ms = 0;
        let proposal = ManifoldPeerStatusProposal {
            schema_id: schema("rusty.manifold.peer.status_proposal.v1"),
            proposal_id: dotted(proposal_id),
            expected_authority_revision: Revision::INITIAL,
            proposer_id: dotted("adapter.quest.fleet-agent"),
            identity: ManifoldPeerIdentity {
                schema_id: schema("rusty.manifold.peer.identity.v1"),
                peer_id: peer_id.clone(),
                key_fingerprint: dotted(format!("fingerprint.{digest}").as_str()),
                trust_domain: dotted("trust.local"),
                roles: vec![ManifoldPeerRole::Observer],
            },
            status: ManifoldPeerStatus {
                schema_id: schema("rusty.manifold.peer.status.v1"),
                peer_id: peer_id.clone(),
                status_revision: Revision::new(status_revision).expect("status revision"),
                observed_at_ms: u64::try_from(BASE_TIME_MS).expect("positive time"),
                expires_at_ms: u64::try_from(BASE_TIME_MS + 60_000).expect("positive time"),
                availability: ManifoldPeerAvailability::Ready,
                capability_ids: vec![dotted("capability.monitoring")],
            },
            payload_class: ManifoldPeerPayloadClass::LowRateDescriptor,
        };
        sign_checkin(
            signing_key,
            key_id.as_str(),
            FleetCheckInClaims {
                schema: "rusty.fleet.checkin_claims.v1".to_owned(),
                checkin_id: checkin_id.to_owned(),
                issued_at_ms: BASE_TIME_MS,
                expires_at_ms: BASE_TIME_MS + 60_000,
                manifold_peer_status_proposal: serde_json::to_value(proposal)
                    .expect("proposal serialization"),
                observation,
                extensions: Default::default(),
            },
        )
    }

    fn sign_checkin(
        signing_key: &SigningKey,
        key_id: &str,
        claims: FleetCheckInClaims,
    ) -> SignedFleetCheckIn {
        let message = claims.signing_bytes().expect("canonical signing bytes");
        SignedFleetCheckIn {
            schema: "rusty.fleet.signed_checkin.v1".to_owned(),
            key_id: key_id.to_owned(),
            algorithm: CHECKIN_SIGNATURE_ALGORITHM.to_owned(),
            signature_hex: hex::encode(signing_key.sign(&message).to_bytes()),
            claims,
        }
    }
}
