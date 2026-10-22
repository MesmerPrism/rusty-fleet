// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::fmt::{Display, Formatter};

use fleet_contracts::{KioskOwnerCatalogSnapshot, ValidateContract};

use crate::sha256_hex;

pub const MAX_PASSIVE_CATALOG_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedKioskCatalog {
    pub owner_snapshot: KioskOwnerCatalogSnapshot,
    pub raw_owner_sha256: String,
    pub received_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogAdmissionError {
    UnsupportedContentType,
    ResponseTooLarge,
    InvalidJson,
    InvalidOwnerContract(String),
    InvalidReceiveTime,
}

impl Display for CatalogAdmissionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedContentType => {
                formatter.write_str("Kiosk passive catalog requires application/json")
            }
            Self::ResponseTooLarge => formatter.write_str("Kiosk passive catalog exceeds 512 KiB"),
            Self::InvalidJson => formatter.write_str("Kiosk passive catalog is not strict JSON"),
            Self::InvalidOwnerContract(reason) => {
                write!(
                    formatter,
                    "Kiosk passive catalog contract is invalid: {reason}"
                )
            }
            Self::InvalidReceiveTime => {
                formatter.write_str("Kiosk passive catalog receive time is invalid")
            }
        }
    }
}

impl std::error::Error for CatalogAdmissionError {}

pub fn admit_passive_catalog(
    content_type: &str,
    body: &[u8],
    received_at_ms: i64,
) -> Result<AdmittedKioskCatalog, CatalogAdmissionError> {
    admit_catalog_with_limit(
        content_type,
        body,
        received_at_ms,
        MAX_PASSIVE_CATALOG_BYTES,
    )
}

pub(crate) fn admit_catalog_with_limit(
    content_type: &str,
    body: &[u8],
    received_at_ms: i64,
    maximum_bytes: usize,
) -> Result<AdmittedKioskCatalog, CatalogAdmissionError> {
    if content_type != "application/json" {
        return Err(CatalogAdmissionError::UnsupportedContentType);
    }
    if body.len() > maximum_bytes {
        return Err(CatalogAdmissionError::ResponseTooLarge);
    }
    let owner: KioskOwnerCatalogSnapshot =
        serde_json::from_slice(body).map_err(|_| CatalogAdmissionError::InvalidJson)?;
    owner.validate().map_err(|failures| {
        CatalogAdmissionError::InvalidOwnerContract(
            failures
                .into_iter()
                .map(|failure| format!("{}:{}", failure.path, failure.code))
                .collect::<Vec<_>>()
                .join(","),
        )
    })?;
    if received_at_ms < owner.observed_at_ms {
        return Err(CatalogAdmissionError::InvalidReceiveTime);
    }
    let raw_owner_sha256 = sha256_hex(body);
    Ok(AdmittedKioskCatalog {
        owner_snapshot: owner,
        raw_owner_sha256,
        received_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary_json() -> Vec<u8> {
        let raw = format!(
            r#"{{
              "schema":"rusty.kiosk.catalog_snapshot.v1",
              "owner_epoch":"KioskOwnerEpoch_001",
              "catalog_revision":"{}",
              "snapshot_digest":"{}",
              "observed_at_ms":1000,
              "fresh_until_ms":301000,
              "tag_revision":"{}",
              "installed_observation_revision":"{}",
              "search_semantics":"rusty.kiosk.catalog_search.v1",
              "complete":true,
              "truncated":false,
              "total_candidate_count":1,
              "permission_limited_count":0,
              "entries":[{{
                "catalog_entry_id":"{}",
                "entry_digest":"{}",
                "app_identity":"{}",
                "disambiguator":"0123456789ab",
                "label":"Spatial Study",
                "tags":["research"],
                "installed":true,
                "launchable":true,
                "source":"installed"
              }}]
            }}"#,
            "1".repeat(64),
            "2".repeat(64),
            "3".repeat(64),
            "4".repeat(64),
            "5".repeat(64),
            "6".repeat(64),
            "7".repeat(64),
        );
        let mut owner: KioskOwnerCatalogSnapshot =
            serde_json::from_str(&raw).expect("owner fixture");
        owner.catalog_revision = fleet_contracts::owner_catalog_revision(&owner);
        owner.snapshot_digest = fleet_contracts::owner_snapshot_digest(&owner);
        serde_json::to_vec(&owner).expect("owner JSON")
    }

    #[test]
    fn passive_catalog_admission_retains_exact_owner_bytes_without_device_grant() {
        let bytes = summary_json();
        let admitted =
            admit_passive_catalog("application/json", &bytes, 1_001).expect("admitted catalog");
        assert_eq!(admitted.owner_snapshot.owner_epoch, "KioskOwnerEpoch_001");
        assert_eq!(admitted.raw_owner_sha256, sha256_hex(&bytes));
    }

    #[test]
    fn passive_catalog_rejects_unknown_fields_and_oversize() {
        let mut damaged: serde_json::Value = serde_json::from_slice(&summary_json()).expect("JSON");
        damaged["free"] = serde_json::Value::Bool(true);
        let bytes = serde_json::to_vec(&damaged).expect("damaged JSON");
        assert!(matches!(
            admit_passive_catalog("application/json", &bytes, 1_001),
            Err(CatalogAdmissionError::InvalidJson)
        ));
        assert!(admit_passive_catalog("application/json", &summary_json(), 1_001).is_ok());
        let mut digest_damaged: serde_json::Value =
            serde_json::from_slice(&summary_json()).expect("owner JSON");
        digest_damaged["snapshot_digest"] = serde_json::Value::String("0".repeat(64));
        assert!(matches!(
            admit_passive_catalog(
                "application/json",
                &serde_json::to_vec(&digest_damaged).expect("damaged digest"),
                1_001
            ),
            Err(CatalogAdmissionError::InvalidOwnerContract(_))
        ));
        assert!(matches!(
            admit_passive_catalog(
                "application/json",
                &vec![b' '; MAX_PASSIVE_CATALOG_BYTES + 1],
                1_001
            ),
            Err(CatalogAdmissionError::ResponseTooLarge)
        ));
    }
}
