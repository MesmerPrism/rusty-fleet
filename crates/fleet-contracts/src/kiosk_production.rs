// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fleet-owned bindings around opaque Kiosk encrypted-operator v2 evidence.
//!
//! These contracts deliberately do not define Kiosk encryption, keys,
//! decrypted-envelope schemas, or owner receipt semantics. A trusted
//! Kiosk-owned verifier supplies those facts; Fleet only retains exact byte
//! digests and rechecks their cross-owner bindings.

use serde::{Deserialize, Serialize};

use crate::{
    ContractViolation, KIOSK_CATALOG_SUMMARY_SCOPE, KioskCatalogSnapshot, ValidateContract, finish,
    require_nonempty,
};

pub const KIOSK_V2_CATALOG_ADMISSION_SCHEMA: &str = "rusty.fleet.kiosk_v2_catalog_admission.v1";
pub const KIOSK_V2_CATALOG_REFRESH_REQUEST_SCHEMA: &str =
    "rusty.fleet.kiosk_v2_catalog_refresh_request.v1";
pub const KIOSK_V2_CATALOG_REFRESH_PROJECTION_SCHEMA: &str =
    "rusty.fleet.kiosk_v2_catalog_refresh_projection.v1";
pub const KIOSK_V2_ENCRYPTED_ROUTE_ID: &str = "kiosk.encrypted.v2";
const MAX_EVIDENCE_WINDOW_MS: i64 = 30_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskV2CatalogRefreshRequest {
    pub schema: String,
    pub device_id: String,
    pub identity_revision: u64,
}

impl ValidateContract for KioskV2CatalogRefreshRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != KIOSK_V2_CATALOG_REFRESH_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.kiosk_v2_catalog_refresh_request.v1",
            ));
        }
        if self.device_id.is_empty() || self.device_id.len() > 256 {
            failures.push(ContractViolation::new(
                "invalid_device_id",
                "device_id",
                "catalog refresh requires one bounded exact Fleet device ID",
            ));
        }
        if self.identity_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_identity_revision",
                "identity_revision",
                "catalog refresh requires the exact positive Fleet identity revision",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskV2CatalogRefreshProjection {
    pub schema: String,
    pub admitted: bool,
    pub device_id: String,
    pub identity_revision: u64,
    pub kiosk_owner_epoch: String,
    pub catalog_revision: String,
    pub snapshot_digest: String,
    pub replay_identity_sha256: String,
    pub grant_revision: u64,
    pub entry_count: usize,
    pub observed_at_ms: i64,
    pub fresh_until_ms: i64,
    pub scopes: Vec<String>,
    pub fleet_revision: u64,
}

impl ValidateContract for KioskV2CatalogRefreshProjection {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != KIOSK_V2_CATALOG_REFRESH_PROJECTION_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.kiosk_v2_catalog_refresh_projection.v1",
            ));
        }
        if !self.admitted {
            failures.push(ContractViolation::new(
                "catalog_not_admitted",
                "admitted",
                "a successful refresh projection must report admitted=true",
            ));
        }
        if self.device_id.is_empty()
            || self.device_id.len() > 256
            || self.identity_revision == 0
            || !valid_token(&self.kiosk_owner_epoch, 16, 96)
            || self.grant_revision == 0
            || self.entry_count > 500
            || self.observed_at_ms <= 0
            || self.fresh_until_ms < self.observed_at_ms
            || self.fleet_revision == 0
        {
            failures.push(ContractViolation::new(
                "invalid_catalog_refresh_projection",
                "device_id",
                "catalog refresh projection must retain bounded current identity and freshness evidence",
            ));
        }
        for (path, digest) in [
            ("catalog_revision", &self.catalog_revision),
            ("snapshot_digest", &self.snapshot_digest),
            ("replay_identity_sha256", &self.replay_identity_sha256),
        ] {
            if !is_sha256(digest) {
                failures.push(ContractViolation::new(
                    "invalid_catalog_refresh_digest",
                    path,
                    "catalog refresh digests must be lowercase SHA-256",
                ));
            }
        }
        if self.scopes != [KIOSK_CATALOG_SUMMARY_SCOPE] {
            failures.push(ContractViolation::new(
                "invalid_catalog_refresh_scopes",
                "scopes",
                "Fleet's File Manager provider admits summary-only catalog evidence",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskV2CatalogAdmissionBinding {
    pub schema: String,
    pub request_id: String,
    pub response_request_id: String,
    pub replay_identity: String,
    pub request_envelope_sha256: String,
    pub response_envelope_sha256: String,
    pub raw_owner_catalog_sha256: String,
    pub owner_grant_receipt_sha256: String,
    pub capability_id: String,
    pub capability_evidence_revision: u64,
    pub grant_revision: u64,
    pub route_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub kiosk_owner_epoch: String,
    pub catalog_revision: String,
    pub snapshot_digest: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub scopes: Vec<String>,
}

impl ValidateContract for KioskV2CatalogAdmissionBinding {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != KIOSK_V2_CATALOG_ADMISSION_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.kiosk_v2_catalog_admission.v1",
            ));
        }
        for (path, value) in [
            ("request_id", self.request_id.as_str()),
            ("response_request_id", self.response_request_id.as_str()),
            ("replay_identity", self.replay_identity.as_str()),
            ("capability_id", self.capability_id.as_str()),
            ("route_id", self.route_id.as_str()),
            ("device_id", self.device_id.as_str()),
            ("kiosk_owner_epoch", self.kiosk_owner_epoch.as_str()),
        ] {
            require_nonempty(&mut failures, value, path);
        }
        if !valid_token(&self.request_id, 8, 256)
            || !valid_token(&self.device_id, 1, 256)
            || !valid_token(&self.kiosk_owner_epoch, 16, 96)
            || !self
                .kiosk_owner_epoch
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            failures.push(ContractViolation::new(
                "invalid_catalog_binding_identity",
                "request_id",
                "request, device and owner identity grammar must match the selected read",
            ));
        }
        if self.response_request_id != self.request_id {
            failures.push(ContractViolation::new(
                "catalog_response_request_mismatch",
                "response_request_id",
                "verified Kiosk response must bind the exact catalog request",
            ));
        }
        for (path, digest) in [
            ("request_envelope_sha256", &self.request_envelope_sha256),
            ("response_envelope_sha256", &self.response_envelope_sha256),
            ("raw_owner_catalog_sha256", &self.raw_owner_catalog_sha256),
            (
                "owner_grant_receipt_sha256",
                &self.owner_grant_receipt_sha256,
            ),
            ("catalog_revision", &self.catalog_revision),
            ("snapshot_digest", &self.snapshot_digest),
        ] {
            if !is_sha256(digest) {
                failures.push(ContractViolation::new(
                    "invalid_kiosk_v2_digest",
                    path,
                    "Kiosk v2 evidence digests must be lowercase SHA-256",
                ));
            }
        }
        if self.identity_revision == 0
            || self.capability_evidence_revision == 0
            || self.grant_revision == 0
        {
            failures.push(ContractViolation::new(
                "invalid_kiosk_v2_revision",
                "grant_revision",
                "identity, capability, and grant revisions must be positive",
            ));
        }
        if self.issued_at_ms <= 0
            || self.expires_at_ms <= self.issued_at_ms
            || self.expires_at_ms.saturating_sub(self.issued_at_ms) > MAX_EVIDENCE_WINDOW_MS
        {
            failures.push(ContractViolation::new(
                "invalid_kiosk_v2_freshness",
                "expires_at_ms",
                "Kiosk v2 catalog evidence must use a positive bounded freshness window",
            ));
        }
        if self.scopes != [KIOSK_CATALOG_SUMMARY_SCOPE] || !is_sha256(&self.replay_identity) {
            failures.push(ContractViolation::new(
                "invalid_summary_binding",
                "scopes",
                "summary scope and exact replay digest required",
            ));
        }
        if self.route_id != KIOSK_V2_ENCRYPTED_ROUTE_ID
            || self.capability_id != crate::KIOSK_DIRECT_OPERATOR_CAPABILITY_ID
        {
            failures.push(ContractViolation::new(
                "wrong_kiosk_v2_route",
                "route_id",
                "production catalog admission requires the exact encrypted Kiosk v2 route",
            ));
        }

        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionKioskCatalogAdmission {
    pub binding: KioskV2CatalogAdmissionBinding,
    pub snapshot: KioskCatalogSnapshot,
}

impl ValidateContract for ProductionKioskCatalogAdmission {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = self.binding.validate().err().unwrap_or_default();
        if let Err(mut nested) = self.snapshot.validate() {
            failures.append(&mut nested);
        }
        if self.binding.device_id != self.snapshot.device_id
            || self.binding.identity_revision != self.snapshot.identity_revision
            || self.binding.kiosk_owner_epoch != self.snapshot.kiosk_owner_epoch
            || self.binding.catalog_revision != self.snapshot.catalog_revision
            || self.binding.snapshot_digest != self.snapshot.snapshot_digest
            || self.binding.raw_owner_catalog_sha256 != self.snapshot.raw_owner_sha256
            || self.binding.owner_grant_receipt_sha256
                != self
                    .snapshot
                    .operator_authority
                    .as_ref()
                    .map_or("", |authority| authority.receipt_sha256.as_str())
            || self.binding.grant_revision
                != self
                    .snapshot
                    .operator_authority
                    .as_ref()
                    .map_or(0, |authority| authority.grant_revision)
            || self.binding.route_id
                != self
                    .snapshot
                    .operator_authority
                    .as_ref()
                    .map_or("", |authority| authority.route_id.as_str())
            || self.binding.scopes
                != self
                    .snapshot
                    .operator_authority
                    .as_ref()
                    .map_or(&[][..], |authority| authority.scopes.as_slice())
            || self.binding.issued_at_ms
                != self
                    .snapshot
                    .operator_authority
                    .as_ref()
                    .map_or(0, |authority| authority.issued_at_ms)
            || self.binding.expires_at_ms
                != self
                    .snapshot
                    .operator_authority
                    .as_ref()
                    .map_or(0, |authority| authority.expires_at_ms)
            || self.snapshot.observed_at_ms < self.binding.issued_at_ms
            || self.snapshot.observed_at_ms > self.binding.expires_at_ms
        {
            failures.push(ContractViolation::new(
                "production_catalog_binding_mismatch",
                "binding",
                "production admission must bind the exact Fleet catalog wrapper and owner grant",
            ));
        }
        finish(failures)
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_token(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}
