// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ContractViolation, DeviceRowProjection, ProjectionFreshness, ValidateContract, finish,
    require_nonempty,
};

pub const KIOSK_OWNER_CATALOG_SNAPSHOT_SCHEMA: &str = "rusty.kiosk.catalog_snapshot.v1";
pub const KIOSK_CATALOG_SNAPSHOT_SCHEMA: &str = "rusty.fleet.kiosk_catalog_snapshot.v1";
pub const KIOSK_APP_SEARCH_REQUEST_SCHEMA: &str = "rusty.fleet.kiosk_app_search_request.v1";
pub const KIOSK_APP_SEARCH_RESULT_SCHEMA: &str = "rusty.fleet.kiosk_app_search_result.v1";
pub const KIOSK_APP_CANDIDATES_RESULT_SCHEMA: &str = "rusty.fleet.kiosk_app_candidates_result.v1";
pub const KIOSK_CATALOG_SEARCH_SEMANTICS: &str = "rusty.kiosk.catalog_search.v1";
pub const KIOSK_CATALOG_SUMMARY_SCOPE: &str = "kiosk.catalog-summary";
pub const KIOSK_CATALOG_DETAIL_SCOPE: &str = "kiosk.catalog-detail";

const MAX_CATALOG_ENTRIES: usize = 500;
const MAX_TAGS_PER_ENTRY: usize = 64;
const MAX_SEARCH_MATCHES: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskOwnerSigningEvidence {
    #[serde(default)]
    pub current_signer_sha256: Vec<String>,
    #[serde(default)]
    pub signing_lineage_sha256: Vec<String>,
    pub lineage_verified_by_android: bool,
    pub lineage_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskOwnerCatalogEntry {
    pub catalog_entry_id: String,
    pub entry_digest: String,
    pub app_identity: String,
    pub disambiguator: String,
    pub label: String,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub installed: bool,
    pub launchable: bool,
    pub source: String,
    #[serde(default)]
    pub signing_evidence: Option<KioskOwnerSigningEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskOwnerCatalogSnapshot {
    pub schema: String,
    pub owner_epoch: String,
    pub catalog_revision: String,
    pub snapshot_digest: String,
    pub observed_at_ms: i64,
    pub fresh_until_ms: i64,
    pub tag_revision: String,
    pub installed_observation_revision: String,
    pub search_semantics: String,
    pub complete: bool,
    pub truncated: bool,
    pub total_candidate_count: usize,
    pub permission_limited_count: usize,
    #[serde(default)]
    pub entries: Vec<KioskOwnerCatalogEntry>,
}

impl ValidateContract for KioskOwnerCatalogSnapshot {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != KIOSK_OWNER_CATALOG_SNAPSHOT_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.kiosk.catalog_snapshot.v1",
            ));
        }
        if self.owner_epoch.len() < 16
            || self.owner_epoch.len() > 96
            || !self
                .owner_epoch
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            failures.push(ContractViolation::new(
                "invalid_owner_epoch",
                "owner_epoch",
                "Kiosk owner epoch must use the bounded owner token grammar",
            ));
        }
        for (path, digest) in [
            ("catalog_revision", &self.catalog_revision),
            ("snapshot_digest", &self.snapshot_digest),
            ("tag_revision", &self.tag_revision),
            (
                "installed_observation_revision",
                &self.installed_observation_revision,
            ),
        ] {
            if !is_sha256(digest) {
                failures.push(ContractViolation::new(
                    "invalid_catalog_digest",
                    path,
                    "Kiosk catalog lineage fields must be SHA-256 digests",
                ));
            }
        }
        if self.observed_at_ms <= 0
            || self.fresh_until_ms < self.observed_at_ms
            || self.fresh_until_ms.saturating_sub(self.observed_at_ms) > 3_600_000
        {
            failures.push(ContractViolation::new(
                "invalid_catalog_freshness",
                "fresh_until_ms",
                "owner catalog freshness is invalid or exceeds one hour",
            ));
        }
        if self.search_semantics != KIOSK_CATALOG_SEARCH_SEMANTICS {
            failures.push(ContractViolation::new(
                "unsupported_search_semantics",
                "search_semantics",
                "owner catalog search semantics do not match Fleet",
            ));
        }
        if self.entries.len() > MAX_CATALOG_ENTRIES
            || self.total_candidate_count < self.entries.len()
            || self.permission_limited_count > self.total_candidate_count
            || self.complete
                != (!self.truncated
                    && self.permission_limited_count == 0
                    && self.total_candidate_count == self.entries.len())
        {
            failures.push(ContractViolation::new(
                "invalid_catalog_completeness",
                "complete",
                "owner catalog completeness, truncation, and counts are inconsistent",
            ));
        }
        let mut entry_ids = BTreeSet::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if !is_sha256(&entry.catalog_entry_id)
                || !is_sha256(&entry.entry_digest)
                || !is_sha256(&entry.app_identity)
                || entry.label.trim().is_empty()
                || entry.label.len() > 160
                || entry.disambiguator.is_empty()
                || entry.disambiguator.len() > 96
                || entry.source.trim().is_empty()
                || entry.source.len() > 64
                || !entry.installed
                || !entry.launchable
                || entry.tags.len() > MAX_TAGS_PER_ENTRY
                || entry.tags.iter().any(|tag| {
                    tag.is_empty() || tag.len() > 96 || tag != &normalize_catalog_text(tag)
                })
                || entry.tags.iter().collect::<BTreeSet<_>>().len() != entry.tags.len()
            {
                failures.push(ContractViolation::new(
                    "invalid_owner_catalog_entry",
                    &format!("entries[{index}]"),
                    "owner catalog entry identity, display, source, or candidate state is invalid",
                ));
            }
            if !entry_ids.insert(&entry.catalog_entry_id) {
                failures.push(ContractViolation::new(
                    "duplicate_catalog_entry",
                    &format!("entries[{index}].catalog_entry_id"),
                    "owner catalog entry identifiers must be unique",
                ));
            }
            if entry.package.is_some() != entry.signing_evidence.is_some() {
                failures.push(ContractViolation::new(
                    "catalog_detail_partial",
                    &format!("entries[{index}]"),
                    "package and signing evidence must be disclosed together",
                ));
            }
            if let Some(signing) = &entry.signing_evidence
                && (!is_sha256(&signing.lineage_digest)
                    || signing.current_signer_sha256.is_empty()
                    || signing
                        .current_signer_sha256
                        .iter()
                        .chain(&signing.signing_lineage_sha256)
                        .any(|digest| !is_sha256(digest))
                    || signing.lineage_verified_by_android
                        == signing.signing_lineage_sha256.is_empty())
            {
                failures.push(ContractViolation::new(
                    "invalid_signing_evidence",
                    &format!("entries[{index}].signing_evidence"),
                    "Kiosk signing evidence or Android-verified lineage is invalid",
                ));
            }
        }
        if !self.truncated {
            let expected_revision = owner_catalog_revision(self);
            if self.catalog_revision != expected_revision {
                failures.push(ContractViolation::new(
                    "catalog_revision_digest_mismatch",
                    "catalog_revision",
                    "Kiosk catalog revision does not match its source lineage and entries",
                ));
            }
        }
        let expected_snapshot = owner_snapshot_digest(self);
        if self.snapshot_digest != expected_snapshot {
            failures.push(ContractViolation::new(
                "snapshot_digest_mismatch",
                "snapshot_digest",
                "Kiosk snapshot digest does not match the exact bounded owner snapshot",
            ));
        }
        finish(failures)
    }
}

#[must_use]
pub fn owner_catalog_revision(snapshot: &KioskOwnerCatalogSnapshot) -> String {
    kiosk_owner_digest(&[
        "rusty.kiosk.catalog_revision.v1",
        &snapshot.owner_epoch,
        &snapshot.tag_revision,
        &snapshot.installed_observation_revision,
        &snapshot
            .entries
            .iter()
            .map(|entry| entry.entry_digest.as_str())
            .collect::<Vec<_>>()
            .join("|"),
    ])
}

#[must_use]
pub fn owner_snapshot_digest(snapshot: &KioskOwnerCatalogSnapshot) -> String {
    kiosk_owner_digest(&[
        "rusty.kiosk.catalog_snapshot_digest.v1",
        &snapshot.owner_epoch,
        &snapshot.catalog_revision,
        &snapshot.observed_at_ms.to_string(),
        &snapshot.fresh_until_ms.to_string(),
        &snapshot.complete.to_string(),
        &snapshot.total_candidate_count.to_string(),
        &snapshot.permission_limited_count.to_string(),
        &snapshot
            .entries
            .iter()
            .map(|entry| entry.entry_digest.as_str())
            .collect::<Vec<_>>()
            .join("|"),
    ])
}

fn kiosk_owner_digest(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        let bytes = part.as_bytes();
        digest.update(bytes.len().to_string().as_bytes());
        digest.update([0]);
        digest.update(bytes);
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

pub fn bind_kiosk_owner_catalog(
    owner: KioskOwnerCatalogSnapshot,
    device_id: &str,
    identity_revision: u64,
    grants: KioskCatalogGrants,
    raw_owner_sha256: &str,
    operator_authority: Option<KioskCatalogAuthorityBinding>,
) -> Result<KioskCatalogSnapshot, Vec<ContractViolation>> {
    owner.validate()?;
    let detailed = owner.entries.iter().any(|entry| entry.package.is_some());
    if detailed != grants.catalog_detail || !grants.catalog_summary {
        return Err(vec![ContractViolation::new(
            "catalog_grant_mismatch",
            "grants",
            "selected Kiosk catalog route does not match the admitted Fleet grant",
        )]);
    }
    let snapshot = KioskCatalogSnapshot {
        schema: KIOSK_CATALOG_SNAPSHOT_SCHEMA.to_owned(),
        owner_schema: owner.schema,
        raw_owner_sha256: raw_owner_sha256.to_owned(),
        operator_authority,
        device_id: device_id.to_owned(),
        identity_revision,
        kiosk_owner_epoch: owner.owner_epoch,
        snapshot_id: owner.snapshot_digest.clone(),
        catalog_revision: owner.catalog_revision,
        snapshot_digest: owner.snapshot_digest,
        observed_at_ms: owner.observed_at_ms,
        fresh_until_ms: owner.fresh_until_ms,
        search_semantics: owner.search_semantics,
        complete: owner.complete,
        truncated: owner.truncated,
        total_candidate_count: owner.total_candidate_count,
        permission_limited_count: owner.permission_limited_count,
        grants,
        entries: owner
            .entries
            .into_iter()
            .map(|entry| KioskCatalogEntry {
                app_identity: entry.app_identity,
                entry_id: entry.catalog_entry_id,
                entry_digest: entry.entry_digest,
                display_label: entry.label,
                disambiguator: entry.disambiguator,
                package_name: entry.package,
                installed: entry.installed,
                launchable: entry.launchable,
                tags: entry.tags,
            })
            .collect(),
    };
    snapshot.validate()?;
    Ok(snapshot)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskCatalogGrants {
    pub catalog_summary: bool,
    pub catalog_detail: bool,
    pub app_launch: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskCatalogAuthorityBinding {
    pub receipt_sha256: String,
    pub grant_revision: u64,
    pub route_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub kiosk_owner_epoch: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskCatalogEntry {
    pub app_identity: String,
    pub entry_id: String,
    pub entry_digest: String,
    pub display_label: String,
    pub disambiguator: String,
    #[serde(default)]
    pub package_name: Option<String>,
    pub installed: bool,
    pub launchable: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl ValidateContract for KioskCatalogEntry {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        require_nonempty(&mut failures, &self.app_identity, "app_identity");
        require_nonempty(&mut failures, &self.entry_id, "entry_id");
        require_nonempty(&mut failures, &self.display_label, "display_label");
        require_nonempty(&mut failures, &self.disambiguator, "disambiguator");
        if !is_sha256(&self.entry_digest) {
            failures.push(ContractViolation::new(
                "invalid_entry_digest",
                "entry_digest",
                "catalog entry digest must be 64 lowercase hexadecimal characters",
            ));
        }
        if !is_bounded_identifier(&self.app_identity, 160)
            || !is_bounded_identifier(&self.entry_id, 160)
        {
            failures.push(ContractViolation::new(
                "invalid_catalog_identifier",
                "app_identity",
                "catalog identities must be bounded portable identifiers",
            ));
        }
        if self.display_label.len() > 256 || self.disambiguator.len() > 96 {
            failures.push(ContractViolation::new(
                "catalog_text_too_large",
                "display_label",
                "catalog display text exceeds its bound",
            ));
        }
        if self
            .package_name
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 256)
        {
            failures.push(ContractViolation::new(
                "invalid_package_name",
                "package_name",
                "package detail must be absent or bounded",
            ));
        }
        if self.tags.len() > MAX_TAGS_PER_ENTRY
            || self.tags.iter().any(|tag| {
                tag.trim().is_empty() || tag.len() > 96 || tag != &normalize_catalog_text(tag)
            })
            || self.tags.iter().collect::<BTreeSet<_>>().len() != self.tags.len()
        {
            failures.push(ContractViolation::new(
                "invalid_catalog_tags",
                "tags",
                "catalog tags must be unique, normalized, nonempty, and bounded",
            ));
        }
        if !self.installed || !self.launchable {
            failures.push(ContractViolation::new(
                "non_candidate_catalog_entry",
                "installed",
                "Fleet snapshots contain only Kiosk-observed installed and launchable entries",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskCatalogSnapshot {
    pub schema: String,
    pub owner_schema: String,
    pub raw_owner_sha256: String,
    #[serde(default)]
    pub operator_authority: Option<KioskCatalogAuthorityBinding>,
    pub device_id: String,
    pub identity_revision: u64,
    pub kiosk_owner_epoch: String,
    pub snapshot_id: String,
    pub catalog_revision: String,
    pub snapshot_digest: String,
    pub observed_at_ms: i64,
    pub fresh_until_ms: i64,
    pub search_semantics: String,
    pub complete: bool,
    pub truncated: bool,
    pub total_candidate_count: usize,
    pub permission_limited_count: usize,
    pub grants: KioskCatalogGrants,
    #[serde(default)]
    pub entries: Vec<KioskCatalogEntry>,
}

impl ValidateContract for KioskCatalogSnapshot {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != KIOSK_CATALOG_SNAPSHOT_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.kiosk_catalog_snapshot.v1",
            ));
        }
        if self.owner_schema != KIOSK_OWNER_CATALOG_SNAPSHOT_SCHEMA
            || !is_sha256(&self.raw_owner_sha256)
        {
            failures.push(ContractViolation::new(
                "invalid_catalog_admission_lineage",
                "raw_owner_sha256",
                "Fleet catalog wrappers must retain the exact owner schema and raw/admission digests",
            ));
        }
        require_nonempty(&mut failures, &self.device_id, "device_id");
        require_nonempty(&mut failures, &self.kiosk_owner_epoch, "kiosk_owner_epoch");
        if self.identity_revision == 0 || !is_sha256(&self.catalog_revision) {
            failures.push(ContractViolation::new(
                "invalid_catalog_revision",
                "catalog_revision",
                "device identity must be positive and catalog revision must be a SHA-256 digest",
            ));
        }
        if !is_bounded_identifier(&self.snapshot_id, 160)
            || !is_bounded_identifier(&self.kiosk_owner_epoch, 160)
            || !is_sha256(&self.snapshot_digest)
        {
            failures.push(ContractViolation::new(
                "invalid_catalog_lineage",
                "snapshot_id",
                "snapshot identity, owner epoch, or digest is malformed",
            ));
        }
        if self.observed_at_ms < 0
            || self.fresh_until_ms < self.observed_at_ms
            || self.fresh_until_ms.saturating_sub(self.observed_at_ms) > 3_600_000
        {
            failures.push(ContractViolation::new(
                "invalid_catalog_freshness",
                "fresh_until_ms",
                "catalog freshness must be ordered and no longer than one hour",
            ));
        }
        if self.search_semantics != KIOSK_CATALOG_SEARCH_SEMANTICS {
            failures.push(ContractViolation::new(
                "unsupported_search_semantics",
                "search_semantics",
                "Fleet supports only the pinned Kiosk catalog search semantics",
            ));
        }
        if !self.grants.catalog_summary && !self.entries.is_empty() {
            failures.push(ContractViolation::new(
                "catalog_without_summary_grant",
                "grants.catalog_summary",
                "catalog bytes require the current catalog-summary grant",
            ));
        }
        if !self.grants.catalog_detail
            && self
                .entries
                .iter()
                .any(|entry| entry.package_name.is_some())
        {
            failures.push(ContractViolation::new(
                "package_without_detail_grant",
                "entries.package_name",
                "package detail requires the current catalog-detail grant",
            ));
        }
        if self.grants.catalog_detail || self.grants.app_launch {
            failures.push(ContractViolation::new(
                "summary_only",
                "grants",
                "Fleet only admits summary catalogue reads",
            ));
        }
        let expected_scopes = [
            self.grants
                .catalog_summary
                .then_some(KIOSK_CATALOG_SUMMARY_SCOPE),
            self.grants
                .catalog_detail
                .then_some(KIOSK_CATALOG_DETAIL_SCOPE),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if let Some(authority) = &self.operator_authority {
            if !is_sha256(&authority.receipt_sha256)
                || authority.grant_revision == 0
                || authority.route_id.trim().is_empty()
                || authority.route_id.len() > 160
                || authority.device_id != self.device_id
                || authority.identity_revision != self.identity_revision
                || authority.kiosk_owner_epoch != self.kiosk_owner_epoch
                || authority.issued_at_ms <= 0
                || authority.issued_at_ms > self.observed_at_ms
                || authority.expires_at_ms < self.observed_at_ms
                || authority.expires_at_ms <= authority.issued_at_ms
                || authority
                    .expires_at_ms
                    .saturating_sub(authority.issued_at_ms)
                    > 3_600_000
                || authority
                    .scopes
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    != expected_scopes
            {
                failures.push(ContractViolation::new(
                    "invalid_catalog_authority_binding",
                    "operator_authority",
                    "authenticated catalog authority must bind receipt, route, device, owner epoch, grant revision, scopes, and expiry",
                ));
            }
        } else if self.grants.catalog_detail || self.grants.app_launch {
            failures.push(ContractViolation::new(
                "catalog_grant_without_authority_receipt",
                "operator_authority",
                "detail and app-launch grants require an authenticated operator authority receipt",
            ));
        }
        if self.entries.len() > MAX_CATALOG_ENTRIES
            || self.total_candidate_count < self.entries.len()
            || self.permission_limited_count > self.total_candidate_count
            || self.complete
                != (!self.truncated
                    && self.permission_limited_count == 0
                    && self.total_candidate_count == self.entries.len())
        {
            failures.push(ContractViolation::new(
                "catalog_too_large",
                "entries",
                "catalog exceeds the bounded entry count",
            ));
        }
        let mut entry_ids = BTreeSet::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if let Err(nested) = entry.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("entries[{index}].{}", failure.path),
                    ..failure
                }));
            }
            if !entry_ids.insert(&entry.entry_id) {
                failures.push(ContractViolation::new(
                    "duplicate_catalog_entry",
                    &format!("entries[{index}].entry_id"),
                    "catalog entry identifiers must be unique within a snapshot",
                ));
            }
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskAppSearchRequest {
    pub schema: String,
    pub query: String,
    pub limit: usize,
}

impl ValidateContract for KioskAppSearchRequest {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != KIOSK_APP_SEARCH_REQUEST_SCHEMA {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.kiosk_app_search_request.v1",
            ));
        }
        if self.query.trim().is_empty() || self.query.len() > 256 {
            failures.push(ContractViolation::new(
                "invalid_app_query",
                "query",
                "app or tag query must be nonempty and at most 256 bytes",
            ));
        }
        if self.limit == 0 || self.limit > MAX_SEARCH_MATCHES {
            failures.push(ContractViolation::new(
                "invalid_search_limit",
                "limit",
                "app search limit must be between 1 and 128",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskAppMatch {
    pub app_identity: String,
    pub display_label: String,
    pub disambiguator: String,
    #[serde(default)]
    pub package_name: Option<String>,
    #[serde(default)]
    pub matched_tags: Vec<String>,
    pub current_device_count: usize,
    pub refresh_required_device_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskCatalogCoverage {
    pub current: usize,
    pub stale: usize,
    pub absent: usize,
    pub incomplete: usize,
    pub permission_limited: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskAppSearchResult {
    pub schema: String,
    pub query: String,
    pub normalized_query: String,
    pub as_of_ms: i64,
    pub coverage: KioskCatalogCoverage,
    #[serde(default)]
    pub matches: Vec<KioskAppMatch>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskAppCandidate {
    pub app_identity: String,
    pub display_label: String,
    pub disambiguator: String,
    #[serde(default)]
    pub package_name: Option<String>,
    pub catalog_snapshot_id: String,
    pub kiosk_owner_epoch: String,
    pub catalog_revision: String,
    pub snapshot_digest: String,
    pub catalog_entry_id: String,
    pub entry_digest: String,
    pub catalog_age_ms: i64,
    pub catalog_freshness: ProjectionFreshness,
    pub catalog_complete: bool,
    pub grant_expires_at_ms: Option<i64>,
    pub row: DeviceRowProjection,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KioskAppCandidatesResult {
    pub schema: String,
    pub app_identity: String,
    pub as_of_ms: i64,
    #[serde(default)]
    pub candidates: Vec<KioskAppCandidate>,
}

#[must_use]
pub fn normalize_catalog_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[must_use]
pub fn catalog_entry_matches(entry: &KioskCatalogEntry, normalized_query: &str) -> bool {
    entry.app_identity == normalized_query
        || normalize_catalog_text(&entry.display_label).contains(normalized_query)
        || entry
            .package_name
            .as_ref()
            .is_some_and(|package| normalize_catalog_text(package).contains(normalized_query))
        || entry.tags.iter().any(|tag| tag.contains(normalized_query))
}

fn is_bounded_identifier(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
