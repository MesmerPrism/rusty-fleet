// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ContractViolation, DeviceObservation, ValidateContract, finish, require_nonempty};

const MAX_CHECKIN_TTL_MS: i64 = 300_000;
const MAX_CHECKIN_BYTES: usize = 256 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 128;

/// Domain separator used before the RFC 8785/JCS-encoded claims bytes.
pub const CHECKIN_SIGNATURE_DOMAIN: &[u8] = b"rusty.fleet.signed_checkin.v1\0";

/// Signature algorithm and serialization profile supported by the v1 envelope.
pub const CHECKIN_SIGNATURE_ALGORITHM: &str = "ed25519-jcs";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FleetCheckInClaims {
    pub schema: String,
    pub checkin_id: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub manifold_peer_status_proposal: Value,
    pub observation: DeviceObservation,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl ValidateContract for FleetCheckInClaims {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.checkin_claims.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.checkin_claims.v1",
            ));
        }
        require_dotted_id(&mut failures, &self.checkin_id, "checkin_id");
        if self.issued_at_ms < 0
            || self.expires_at_ms <= self.issued_at_ms
            || self.expires_at_ms.saturating_sub(self.issued_at_ms) > MAX_CHECKIN_TTL_MS
        {
            failures.push(ContractViolation::new(
                "invalid_checkin_window",
                "expires_at_ms",
                "check-in window must be positive and no longer than five minutes",
            ));
        }
        if !self.manifold_peer_status_proposal.is_object() {
            failures.push(ContractViolation::new(
                "invalid_manifold_proposal",
                "manifold_peer_status_proposal",
                "Manifold peer-status proposal must be a JSON object",
            ));
        }
        if self.observation.source_time_ms != self.issued_at_ms {
            failures.push(ContractViolation::new(
                "source_issue_mismatch",
                "observation.source_time_ms",
                "M1 check-in source time must equal its signed issue time",
            ));
        }
        if self.observation.received_time_ms != 0 {
            failures.push(ContractViolation::new(
                "untrusted_receive_time",
                "observation.received_time_ms",
                "signed check-ins must leave received time zero for the Hub ingress adapter",
            ));
        }
        for (path, provenance) in [
            (
                "observation.agent.provenance",
                self.observation
                    .agent
                    .as_ref()
                    .map(|value| &value.provenance),
            ),
            (
                "observation.power.provenance",
                self.observation
                    .power
                    .as_ref()
                    .map(|value| &value.provenance),
            ),
            (
                "observation.application.provenance",
                self.observation
                    .application
                    .as_ref()
                    .map(|value| &value.provenance),
            ),
        ] {
            if provenance.is_some_and(|value| {
                value.observed_at_ms != self.observation.source_time_ms
                    || value.fresh_until_ms != self.expires_at_ms
            }) {
                failures.push(ContractViolation::new(
                    "fact_window_mismatch",
                    path,
                    "M1 fact time must match source time and check-in expiry",
                ));
            }
        }
        if let Err(nested) = self.observation.validate() {
            failures.extend(nested.into_iter().map(|failure| ContractViolation {
                path: format!("observation.{}", failure.path),
                ..failure
            }));
        }
        if self.extensions.len() > 16 {
            failures.push(ContractViolation::new(
                "checkin_extensions_exceeded",
                "extensions",
                "check-in supports at most 16 extension fields",
            ));
        }
        if serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() > MAX_CHECKIN_BYTES) {
            failures.push(ContractViolation::new(
                "checkin_bytes_exceeded",
                "checkin",
                "serialized check-in exceeds the 256 KiB low-rate limit",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignedFleetCheckIn {
    pub schema: String,
    pub key_id: String,
    pub algorithm: String,
    pub signature_hex: String,
    pub claims: FleetCheckInClaims,
}

impl ValidateContract for SignedFleetCheckIn {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.signed_checkin.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.signed_checkin.v1",
            ));
        }
        require_dotted_id(&mut failures, &self.key_id, "key_id");
        if self.algorithm != CHECKIN_SIGNATURE_ALGORITHM {
            failures.push(ContractViolation::new(
                "unsupported_signature",
                "algorithm",
                "M1 supports only Ed25519 signatures over RFC 8785/JCS claims",
            ));
        }
        if self.signature_hex.len() != 128
            || !self
                .signature_hex
                .bytes()
                .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
        {
            failures.push(ContractViolation::new(
                "invalid_signature_encoding",
                "signature_hex",
                "signature must be 64 bytes encoded as lowercase hexadecimal",
            ));
        }
        if let Err(mut nested) = self.claims.validate() {
            failures.append(&mut nested);
        }
        finish(failures)
    }
}

impl FleetCheckInClaims {
    /// Returns the exact domain-separated bytes covered by a v1 signature.
    ///
    /// JCS removes struct-field and map-insertion ordering as an implicit
    /// cross-repository signing requirement.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let canonical_claims = serde_jcs::to_vec(self)?;
        let mut message =
            Vec::with_capacity(CHECKIN_SIGNATURE_DOMAIN.len() + canonical_claims.len());
        message.extend_from_slice(CHECKIN_SIGNATURE_DOMAIN);
        message.extend_from_slice(&canonical_claims);
        Ok(message)
    }
}

fn require_dotted_id(failures: &mut Vec<ContractViolation>, value: &str, path: &str) {
    require_nonempty(failures, value, path);
    if value.len() > MAX_IDENTIFIER_BYTES
        || value.split('.').any(|segment| {
            segment.is_empty()
                || !segment.chars().next().is_some_and(is_identifier_edge)
                || !segment.chars().last().is_some_and(is_identifier_edge)
                || !segment.chars().all(is_identifier_body)
        })
    {
        failures.push(ContractViolation::new(
            "invalid_dotted_id",
            path,
            "identifier must be at most 128 bytes and use lowercase dotted-id grammar",
        ));
    }
}

fn is_identifier_edge(value: char) -> bool {
    value.is_ascii_lowercase() || value.is_ascii_digit()
}

fn is_identifier_body(value: char) -> bool {
    is_identifier_edge(value) || value == '_' || value == '-'
}
