// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Opaque encrypted-operator v2 boundary.
//!
//! Fleet does not construct, decrypt, authenticate, or interpret a Kiosk v2
//! envelope. A trusted Kiosk-owned provider returns exact envelope bytes and a
//! verification result. This adapter independently rechecks every Fleet-owned
//! binding before projecting an admission.

use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};

use fleet_contracts::{
    KIOSK_CATALOG_DETAIL_SCOPE, KIOSK_CATALOG_SUMMARY_SCOPE, KIOSK_DIRECT_OPERATOR_CAPABILITY_ID,
    KIOSK_V2_CATALOG_ADMISSION_SCHEMA, KIOSK_V2_ENCRYPTED_ROUTE_ID, KioskCatalogAuthorityBinding,
    KioskCatalogGrants, KioskV2CatalogAdmissionBinding, ProductionKioskCatalogAdmission,
    ValidateContract, bind_kiosk_owner_catalog,
};

use crate::{catalog::admit_catalog_with_limit, sha256_hex};

pub const MAX_KIOSK_V2_REQUEST_ENVELOPE_BYTES: usize = 1024 * 1024;
pub const MAX_KIOSK_V2_RESPONSE_ENVELOPE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_KIOSK_V2_OWNER_CATALOG_BYTES: usize = 768 * 1024;
pub const MAX_KIOSK_V2_OWNER_GRANT_RECEIPT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KioskV2CatalogExpectation {
    pub request_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub capability_id: String,
    pub capability_evidence_revision: u64,
    pub route_id: String,
    pub expected_owner_epoch: Option<String>,
    pub scopes: Vec<String>,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedKioskV2CatalogExchange {
    pub request_envelope: Vec<u8>,
    pub response_envelope: Vec<u8>,
    pub owner_catalog_json: Vec<u8>,
    pub owner_grant_receipt: Vec<u8>,
    pub response_request_id: String,
    pub replay_identity: String,
    pub capability_id: String,
    pub capability_evidence_revision: u64,
    pub grant_revision: u64,
    pub route_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub kiosk_owner_epoch: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub scopes: Vec<String>,
}

pub trait KioskEncryptedOperatorV2Provider: Send {
    /// Produces and verifies an exact owner v2 catalog exchange.
    ///
    /// Implementations own all Kiosk crypto, key, nonce, envelope, endpoint,
    /// and authentication behavior. Fleet must not supply or log those values.
    fn verified_catalog_exchange(
        &mut self,
        expectation: &KioskV2CatalogExpectation,
    ) -> Result<VerifiedKioskV2CatalogExchange, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KioskV2AdapterError {
    InvalidExpectation,
    Provider(String),
    EnvelopeEmpty,
    EnvelopeTooLarge,
    OwnerReceiptEmpty,
    OwnerReceiptTooLarge,
    RequestMismatch,
    CapabilityMismatch,
    IdentityMismatch,
    OwnerEpochMismatch,
    ScopeMismatch,
    FreshnessMismatch,
    CatalogMismatch,
    ReplayIdentityInvalid,
}

impl Display for KioskV2AdapterError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidExpectation => formatter.write_str("Kiosk v2 expectation is invalid"),
            Self::Provider(reason) => write!(formatter, "Kiosk v2 provider rejected: {reason}"),
            Self::EnvelopeEmpty => formatter.write_str("Kiosk v2 envelope is empty"),
            Self::EnvelopeTooLarge => {
                formatter.write_str("Kiosk v2 envelope exceeds its role-specific bound")
            }
            Self::OwnerReceiptEmpty => formatter.write_str("Kiosk v2 owner receipt is empty"),
            Self::OwnerReceiptTooLarge => {
                formatter.write_str("Kiosk v2 owner receipt exceeds its role-specific bound")
            }
            Self::RequestMismatch => {
                formatter.write_str("Kiosk v2 response does not bind the exact request")
            }
            Self::CapabilityMismatch => {
                formatter.write_str("Kiosk v2 capability evidence does not match")
            }
            Self::IdentityMismatch => formatter.write_str("Kiosk v2 Fleet identity does not match"),
            Self::OwnerEpochMismatch => formatter.write_str("Kiosk v2 owner epoch does not match"),
            Self::ScopeMismatch => formatter.write_str("Kiosk v2 scopes do not match"),
            Self::FreshnessMismatch => {
                formatter.write_str("Kiosk v2 freshness window does not match")
            }
            Self::CatalogMismatch => {
                formatter.write_str("Kiosk v2 owner catalog does not match the verified exchange")
            }
            Self::ReplayIdentityInvalid => {
                formatter.write_str("Kiosk v2 replay identity is invalid")
            }
        }
    }
}

impl std::error::Error for KioskV2AdapterError {}

pub fn ingest_production_catalog<P: KioskEncryptedOperatorV2Provider + ?Sized>(
    provider: &mut P,
    expectation: &KioskV2CatalogExpectation,
    received_at_ms: i64,
) -> Result<ProductionKioskCatalogAdmission, KioskV2AdapterError> {
    validate_catalog_expectation(expectation, received_at_ms)?;
    let exchange = provider
        .verified_catalog_exchange(expectation)
        .map_err(KioskV2AdapterError::Provider)?;
    validate_envelope(
        &exchange.request_envelope,
        MAX_KIOSK_V2_REQUEST_ENVELOPE_BYTES,
    )?;
    validate_envelope(
        &exchange.response_envelope,
        MAX_KIOSK_V2_RESPONSE_ENVELOPE_BYTES,
    )?;
    validate_owner_receipt(
        &exchange.owner_grant_receipt,
        MAX_KIOSK_V2_OWNER_GRANT_RECEIPT_BYTES,
    )?;
    if exchange.owner_catalog_json.is_empty()
        || exchange.owner_catalog_json.len() > MAX_KIOSK_V2_OWNER_CATALOG_BYTES
    {
        return Err(KioskV2AdapterError::CatalogMismatch);
    }
    if exchange.response_request_id != expectation.request_id
        || !valid_sha256(&exchange.replay_identity)
    {
        return Err(if exchange.response_request_id != expectation.request_id {
            KioskV2AdapterError::RequestMismatch
        } else {
            KioskV2AdapterError::ReplayIdentityInvalid
        });
    }
    if exchange.capability_id != expectation.capability_id
        || exchange.capability_evidence_revision != expectation.capability_evidence_revision
        || exchange.route_id != expectation.route_id
        || exchange.grant_revision == 0
    {
        return Err(KioskV2AdapterError::CapabilityMismatch);
    }
    if exchange.device_id != expectation.device_id
        || exchange.identity_revision != expectation.identity_revision
    {
        return Err(KioskV2AdapterError::IdentityMismatch);
    }
    if expectation
        .expected_owner_epoch
        .as_ref()
        .is_some_and(|expected| exchange.kiosk_owner_epoch != *expected)
    {
        return Err(KioskV2AdapterError::OwnerEpochMismatch);
    }
    if exchange.scopes != expectation.scopes {
        return Err(KioskV2AdapterError::ScopeMismatch);
    }
    if exchange.issued_at_ms != expectation.issued_at_ms
        || exchange.expires_at_ms != expectation.expires_at_ms
        || received_at_ms < exchange.issued_at_ms
        || received_at_ms > exchange.expires_at_ms
    {
        return Err(KioskV2AdapterError::FreshnessMismatch);
    }

    let admitted_owner = admit_catalog_with_limit(
        "application/json",
        &exchange.owner_catalog_json,
        received_at_ms,
        MAX_KIOSK_V2_OWNER_CATALOG_BYTES,
    )
    .map_err(|_| KioskV2AdapterError::CatalogMismatch)?;
    if admitted_owner.owner_snapshot.owner_epoch != exchange.kiosk_owner_epoch
        || admitted_owner.owner_snapshot.observed_at_ms < exchange.issued_at_ms
        || admitted_owner.owner_snapshot.observed_at_ms > exchange.expires_at_ms
        || admitted_owner.owner_snapshot.fresh_until_ms < received_at_ms
        || (exchange.scopes.as_slice() == [KIOSK_CATALOG_SUMMARY_SCOPE]
            && admitted_owner
                .owner_snapshot
                .entries
                .iter()
                .any(|entry| entry.package.is_some() || entry.signing_evidence.is_some()))
    {
        return Err(KioskV2AdapterError::CatalogMismatch);
    }
    let scopes = exchange.scopes.iter().cloned().collect::<BTreeSet<_>>();
    let grants = KioskCatalogGrants {
        catalog_summary: scopes.contains(KIOSK_CATALOG_SUMMARY_SCOPE),
        catalog_detail: scopes.contains(KIOSK_CATALOG_DETAIL_SCOPE),
        app_launch: false,
    };
    let owner_grant_receipt_sha256 = sha256_hex(&exchange.owner_grant_receipt);
    let snapshot = bind_kiosk_owner_catalog(
        admitted_owner.owner_snapshot,
        &expectation.device_id,
        expectation.identity_revision,
        grants,
        &admitted_owner.raw_owner_sha256,
        Some(KioskCatalogAuthorityBinding {
            receipt_sha256: owner_grant_receipt_sha256.clone(),
            grant_revision: exchange.grant_revision,
            route_id: exchange.route_id.clone(),
            device_id: expectation.device_id.clone(),
            identity_revision: expectation.identity_revision,
            kiosk_owner_epoch: exchange.kiosk_owner_epoch.clone(),
            issued_at_ms: exchange.issued_at_ms,
            expires_at_ms: exchange.expires_at_ms,
            scopes: exchange.scopes.clone(),
        }),
    )
    .map_err(|_| KioskV2AdapterError::CatalogMismatch)?;
    let binding = KioskV2CatalogAdmissionBinding {
        schema: KIOSK_V2_CATALOG_ADMISSION_SCHEMA.to_owned(),
        request_id: expectation.request_id.clone(),
        response_request_id: exchange.response_request_id,
        replay_identity: exchange.replay_identity,
        request_envelope_sha256: sha256_hex(&exchange.request_envelope),
        response_envelope_sha256: sha256_hex(&exchange.response_envelope),
        raw_owner_catalog_sha256: admitted_owner.raw_owner_sha256,
        owner_grant_receipt_sha256,
        capability_id: exchange.capability_id,
        capability_evidence_revision: exchange.capability_evidence_revision,
        grant_revision: exchange.grant_revision,
        route_id: exchange.route_id,
        device_id: exchange.device_id,
        identity_revision: exchange.identity_revision,
        kiosk_owner_epoch: exchange.kiosk_owner_epoch,
        catalog_revision: snapshot.catalog_revision.clone(),
        snapshot_digest: snapshot.snapshot_digest.clone(),
        issued_at_ms: exchange.issued_at_ms,
        expires_at_ms: exchange.expires_at_ms,
        scopes: exchange.scopes,
    };
    let admission = ProductionKioskCatalogAdmission { binding, snapshot };
    admission
        .validate()
        .map_err(|_| KioskV2AdapterError::CatalogMismatch)?;
    Ok(admission)
}

fn validate_catalog_expectation(
    expectation: &KioskV2CatalogExpectation,
    now_ms: i64,
) -> Result<(), KioskV2AdapterError> {
    if !valid_token(&expectation.request_id, 8, 256)
        || !valid_token(&expectation.device_id, 1, 256)
        || expectation.identity_revision == 0
        || expectation.capability_id != KIOSK_DIRECT_OPERATOR_CAPABILITY_ID
        || expectation.capability_evidence_revision == 0
        || expectation.route_id != KIOSK_V2_ENCRYPTED_ROUTE_ID
        || expectation
            .expected_owner_epoch
            .as_ref()
            .is_some_and(|epoch| !valid_token(epoch, 16, 96))
        || expectation.scopes.as_slice() != [KIOSK_CATALOG_SUMMARY_SCOPE]
        || expectation.issued_at_ms <= 0
        || expectation.expires_at_ms <= expectation.issued_at_ms
        || expectation
            .expires_at_ms
            .saturating_sub(expectation.issued_at_ms)
            > 30_000
        || now_ms < expectation.issued_at_ms
        || now_ms > expectation.expires_at_ms
    {
        return Err(KioskV2AdapterError::InvalidExpectation);
    }
    Ok(())
}

fn validate_envelope(bytes: &[u8], maximum: usize) -> Result<(), KioskV2AdapterError> {
    if bytes.is_empty() {
        return Err(KioskV2AdapterError::EnvelopeEmpty);
    }
    if bytes.len() > maximum {
        return Err(KioskV2AdapterError::EnvelopeTooLarge);
    }
    Ok(())
}

fn validate_owner_receipt(bytes: &[u8], maximum: usize) -> Result<(), KioskV2AdapterError> {
    if bytes.is_empty() {
        return Err(KioskV2AdapterError::OwnerReceiptEmpty);
    }
    if bytes.len() > maximum {
        return Err(KioskV2AdapterError::OwnerReceiptTooLarge);
    }
    Ok(())
}

fn valid_token(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
