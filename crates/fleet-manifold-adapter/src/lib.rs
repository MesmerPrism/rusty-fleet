// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Authenticated low-rate check-in admission through exact Manifold authority.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, VerifyingKey};
use fleet_contracts::{SignedFleetCheckIn, ValidateContract};
use fleet_hub::{FleetHub, ObservationDecision};
use rusty_manifold_model::DottedId;
use rusty_manifold_peer::{
    ManifoldAcceptedPeerState, ManifoldPeerApplicationReceipt, ManifoldPeerCredentialStatus,
    ManifoldPeerDecision, ManifoldPeerDecisionOutcome, ManifoldPeerEnrollmentReceipt,
    ManifoldPeerEnrollmentRequest, ManifoldPeerEnrollmentState, ManifoldPeerReviewCase,
    ManifoldPeerStatusProposal, review_and_apply_peer_enrollment, review_and_apply_peer_proposal,
};
use serde::{Deserialize, Serialize};

const MAX_SEEN_CHECKINS: usize = 10_000;

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

pub struct FleetManifoldAdapter {
    enrollment: ManifoldPeerEnrollmentState,
    accepted_peers: ManifoldAcceptedPeerState,
    trusted_operator_ids: Vec<DottedId>,
    seen_checkins: BTreeMap<String, i64>,
}

impl FleetManifoldAdapter {
    #[must_use]
    pub fn new(trusted_operator_ids: Vec<DottedId>) -> Self {
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

        let Ok(proposal) = serde_json::from_value::<ManifoldPeerStatusProposal>(
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
        let mut accepted_observation = signed.claims.observation.clone();
        accepted_observation.received_time_ms = now_ms;
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
        }
        *hub = candidate_hub;
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
        CHECKIN_SIGNATURE_ALGORITHM, CHECKIN_SIGNATURE_DOMAIN, FleetCheckInClaims,
        SignedFleetCheckIn,
    };
    use fleet_hub::{FleetHub, HubPolicy};
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

    use super::{CheckInRejectionReason, FleetManifoldAdapter};

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

        let prior_authority_revision = adapter.accepted_peers().authority_revision;
        let prior_proposal_count = adapter.accepted_peers().applied_proposal_ids.len();
        let mut fleet_rejected_claims = signed.claims;
        fleet_rejected_claims.checkin_id = "checkin.quest.2".to_owned();
        let mut next_proposal: ManifoldPeerStatusProposal =
            serde_json::from_value(fleet_rejected_claims.manifold_peer_status_proposal.clone())
                .expect("proposal");
        next_proposal.proposal_id = dotted("proposal.status.quest.2");
        next_proposal.expected_authority_revision = prior_authority_revision;
        next_proposal.status.status_revision = Revision::new(2).expect("second status revision");
        fleet_rejected_claims.manifold_peer_status_proposal =
            serde_json::to_value(next_proposal).expect("proposal serialization");
        let fleet_rejected = sign_checkin(&signing_key, key_id.as_str(), fleet_rejected_claims);
        let fleet_rejection = adapter.accept(&mut hub, fleet_rejected, BASE_TIME_MS + 3);
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

    fn dotted(value: &str) -> DottedId {
        DottedId::new(value.to_owned()).expect("dotted id")
    }

    fn schema(value: &str) -> SchemaId {
        SchemaId::new(value.to_owned()).expect("schema id")
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
