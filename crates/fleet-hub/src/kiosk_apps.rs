// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use fleet_contracts::{
    KIOSK_APP_CANDIDATES_RESULT_SCHEMA, KIOSK_APP_SEARCH_RESULT_SCHEMA, KioskAppCandidate,
    KioskAppCandidatesResult, KioskAppMatch, KioskAppSearchRequest, KioskAppSearchResult,
    KioskCatalogCoverage, KioskCatalogSnapshot, KioskV2CatalogAdmissionBinding,
    ProductionKioskCatalogAdmission, ProjectionFreshness, ValidateContract, catalog_entry_matches,
    normalize_catalog_text,
};
use serde::{Deserialize, Serialize};

use crate::{FleetHub, HubError};

pub(crate) const MAX_KIOSK_CATALOGS: usize = 1_000;
pub(crate) const MAX_KIOSK_CATALOG_REPLAY_IDENTITIES: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KioskCatalogRecord {
    pub(crate) snapshot: KioskCatalogSnapshot,
    pub(crate) received_at_ms: i64,
    pub(crate) source_epoch: String,
    #[serde(default)]
    pub(crate) production_admission: Option<KioskV2CatalogAdmissionBinding>,
}

impl FleetHub {
    fn accept_kiosk_catalog_internal(
        &mut self,
        snapshot: KioskCatalogSnapshot,
        received_at_ms: i64,
    ) -> Result<(), HubError> {
        snapshot.validate().map_err(|failures| {
            HubError::new(
                "kiosk_catalog_invalid",
                failures
                    .into_iter()
                    .map(|failure| format!("{}:{}", failure.path, failure.code))
                    .collect::<Vec<_>>()
                    .join(","),
            )
        })?;
        if received_at_ms < snapshot.observed_at_ms {
            return Err(HubError::new(
                "kiosk_catalog_receive_time_invalid",
                "catalog receive time cannot precede its owner observation",
            ));
        }
        let device = self.devices.get(&snapshot.device_id).ok_or_else(|| {
            HubError::new(
                "kiosk_catalog_device_unknown",
                "catalog device is not enrolled in Fleet",
            )
        })?;
        if device.observation.identity.identity_revision != snapshot.identity_revision {
            return Err(HubError::new(
                "kiosk_catalog_identity_mismatch",
                "catalog does not bind the current Fleet identity revision",
            ));
        }
        if let Some(existing) = self.kiosk_catalogs.get(&snapshot.device_id) {
            if received_at_ms < existing.received_at_ms {
                return Err(HubError::new(
                    "kiosk_catalog_receive_time_regression",
                    "catalog receive time cannot regress",
                ));
            }
            if snapshot.observed_at_ms <= existing.snapshot.observed_at_ms {
                return Err(HubError::new(
                    "kiosk_catalog_observation_regression",
                    "catalog owner observation time must increase",
                ));
            }
        }
        if !self.kiosk_catalogs.contains_key(&snapshot.device_id)
            && self.kiosk_catalogs.len() >= MAX_KIOSK_CATALOGS
        {
            return Err(HubError::new(
                "kiosk_catalog_full",
                "catalogue observation capacity reached",
            ));
        }
        self.kiosk_catalogs.insert(
            snapshot.device_id.clone(),
            KioskCatalogRecord {
                snapshot,
                received_at_ms,
                source_epoch: device.observation.source_epoch.clone(),
                production_admission: None,
            },
        );
        self.result_revision = self.result_revision.saturating_add(1);
        Ok(())
    }

    pub fn kiosk_catalog_association(&self, device_id: &str) -> Option<&(u64, String, String)> {
        self.kiosk_catalog_associations.get(device_id)
    }

    pub fn accept_profiled_kiosk_catalog(
        &mut self,
        admission: ProductionKioskCatalogAdmission,
        profile_id: &str,
        received_at_ms: i64,
    ) -> Result<(), HubError> {
        let device_id = admission.snapshot.device_id.clone();
        let association = (
            admission.snapshot.identity_revision,
            profile_id.to_owned(),
            admission.snapshot.kiosk_owner_epoch.clone(),
        );
        if self
            .kiosk_catalog_associations
            .get(&device_id)
            .is_some_and(|old| old != &association)
        {
            return Err(HubError::new(
                "kiosk_catalog_setup_required",
                "stored identity, profile or owner association differs; explicit setup required",
            ));
        }
        if !(8..=128).contains(&profile_id.len())
            || !profile_id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':')
            })
            || (!self.kiosk_catalog_associations.contains_key(&device_id)
                && self.kiosk_catalog_associations.len() >= MAX_KIOSK_CATALOGS)
        {
            return Err(HubError::new(
                "kiosk_catalog_setup_invalid",
                "profile association is invalid or full",
            ));
        }
        let mut candidate = self.clone();
        candidate.accept_production_kiosk_catalog(admission, received_at_ms)?;
        candidate
            .kiosk_catalog_associations
            .insert(device_id, association);
        *self = candidate;
        Ok(())
    }

    fn accept_production_kiosk_catalog(
        &mut self,
        admission: ProductionKioskCatalogAdmission,
        received_at_ms: i64,
    ) -> Result<(), HubError> {
        admission.validate().map_err(|failures| {
            HubError::new(
                "kiosk_production_catalog_invalid",
                failures
                    .into_iter()
                    .map(|failure| format!("{}:{}", failure.path, failure.code))
                    .collect::<Vec<_>>()
                    .join(","),
            )
        })?;
        if received_at_ms < admission.binding.issued_at_ms
            || received_at_ms > admission.binding.expires_at_ms
            || received_at_ms < admission.snapshot.observed_at_ms
            || received_at_ms > admission.snapshot.fresh_until_ms
        {
            return Err(HubError::new(
                "kiosk_production_catalog_expired",
                "production catalog admission is outside its exact verified freshness window",
            ));
        }
        if self
            .seen_kiosk_catalog_replay_identities
            .contains(&admission.binding.replay_identity)
        {
            return Err(HubError::new(
                "kiosk_production_catalog_replay",
                "production Kiosk catalog replay identity was already consumed",
            ));
        }
        if self.seen_kiosk_catalog_replay_identities.len() >= MAX_KIOSK_CATALOG_REPLAY_IDENTITIES {
            return Err(HubError::new(
                "kiosk_production_catalog_replay_evidence_full",
                "production Kiosk catalog replay evidence reached its durable bound",
            ));
        }
        let device = self
            .devices
            .get(&admission.snapshot.device_id)
            .ok_or_else(|| {
                HubError::new(
                    "kiosk_catalog_device_unknown",
                    "production catalog device is not enrolled in Fleet",
                )
            })?;
        let capability = device
            .observation
            .capabilities
            .get(&admission.binding.capability_id)
            .ok_or_else(|| {
                HubError::new(
                    "kiosk_production_capability_absent",
                    "production catalog lacks current Kiosk capability evidence",
                )
            })?;
        if capability.owner != "rusty-kiosk"
            || !capability.is_ready()
            || capability.evidence_revision != admission.binding.capability_evidence_revision
            || received_at_ms > capability.fresh_until_ms
        {
            return Err(HubError::new(
                "kiosk_production_capability_mismatch",
                "production catalog does not bind current ready Kiosk capability evidence",
            ));
        }

        let replay_identity = admission.binding.replay_identity.clone();
        let production_binding = admission.binding.clone();
        let device_id = admission.snapshot.device_id.clone();
        let mut candidate = self.clone();
        candidate.accept_kiosk_catalog_internal(admission.snapshot, received_at_ms)?;
        let record = candidate
            .kiosk_catalogs
            .get_mut(&device_id)
            .ok_or_else(|| {
                HubError::new(
                    "kiosk_production_catalog_internal",
                    "production catalog was not retained after validation",
                )
            })?;
        record.production_admission = Some(production_binding);
        candidate
            .seen_kiosk_catalog_replay_identities
            .insert(replay_identity);
        *self = candidate;
        Ok(())
    }

    pub fn kiosk_app_search(
        &self,
        request: &KioskAppSearchRequest,
        now_ms: i64,
    ) -> Result<KioskAppSearchResult, HubError> {
        request.validate().map_err(|failures| {
            HubError::new(
                "kiosk_app_query_invalid",
                failures
                    .into_iter()
                    .map(|failure| failure.code)
                    .collect::<Vec<_>>()
                    .join(","),
            )
        })?;
        let normalized_query = normalize_catalog_text(&request.query);
        let mut coverage = KioskCatalogCoverage {
            current: 0,
            stale: 0,
            absent: 0,
            incomplete: 0,
            permission_limited: 0,
        };
        let mut matches: BTreeMap<String, KioskAppMatch> = BTreeMap::new();
        for device_id in self.devices.keys() {
            let device = self
                .devices
                .get(device_id)
                .expect("device key originates from the same map");
            let Some(record) = self.kiosk_catalogs.get(device_id) else {
                coverage.absent += 1;
                continue;
            };
            let snapshot = &record.snapshot;
            if snapshot.identity_revision != device.observation.identity.identity_revision
                || record.source_epoch != device.observation.source_epoch
            {
                coverage.absent += 1;
                continue;
            }
            if !snapshot.grants.catalog_summary || snapshot.permission_limited_count > 0 {
                coverage.permission_limited += 1;
                continue;
            }
            let current = now_ms <= snapshot.fresh_until_ms;
            if !snapshot.complete {
                coverage.incomplete += 1;
            } else if current {
                coverage.current += 1;
            } else {
                coverage.stale += 1;
            }
            if !current || !snapshot.complete {
                continue;
            }
            for entry in snapshot
                .entries
                .iter()
                .filter(|entry| catalog_entry_matches(entry, &normalized_query))
            {
                let item = matches
                    .entry(entry.app_identity.clone())
                    .or_insert_with(|| KioskAppMatch {
                        app_identity: entry.app_identity.clone(),
                        display_label: entry.display_label.clone(),
                        disambiguator: entry.disambiguator.clone(),
                        package_name: entry.package_name.clone(),
                        matched_tags: entry
                            .tags
                            .iter()
                            .filter(|tag| tag.contains(&normalized_query))
                            .cloned()
                            .collect(),
                        current_device_count: 0,
                        refresh_required_device_count: 0,
                    });
                if item.package_name.is_none() {
                    item.package_name.clone_from(&entry.package_name);
                }
                for tag in entry
                    .tags
                    .iter()
                    .filter(|tag| tag.contains(&normalized_query))
                {
                    if !item.matched_tags.contains(tag) {
                        item.matched_tags.push(tag.clone());
                    }
                }
                item.matched_tags.sort();
                item.current_device_count += 1;
            }
        }
        let mut matches: Vec<_> = matches.into_values().collect();
        matches.sort_by(|left, right| {
            left.display_label
                .to_lowercase()
                .cmp(&right.display_label.to_lowercase())
                .then_with(|| left.app_identity.cmp(&right.app_identity))
        });
        matches.truncate(request.limit);
        Ok(KioskAppSearchResult {
            schema: KIOSK_APP_SEARCH_RESULT_SCHEMA.to_owned(),
            query: request.query.clone(),
            normalized_query,
            as_of_ms: now_ms,
            coverage,
            matches,
        })
    }

    pub fn kiosk_app_candidates(
        &self,
        app_identity: &str,
        now_ms: i64,
    ) -> Result<KioskAppCandidatesResult, HubError> {
        if app_identity.is_empty() || app_identity.len() > 160 {
            return Err(HubError::new(
                "kiosk_app_identity_invalid",
                "exact Kiosk app identity is required",
            ));
        }
        let mut candidates = Vec::new();
        for (device_id, record) in &self.kiosk_catalogs {
            let Some(entry) = record
                .snapshot
                .entries
                .iter()
                .find(|entry| entry.app_identity == app_identity)
            else {
                continue;
            };
            let Some(device) = self.devices.get(device_id) else {
                continue;
            };
            if record.snapshot.identity_revision != device.observation.identity.identity_revision
                || record.source_epoch != device.observation.source_epoch
            {
                continue;
            }
            let row = self.row(device, now_ms);
            let catalog_current = now_ms <= record.snapshot.fresh_until_ms;
            candidates.push(KioskAppCandidate {
                app_identity: entry.app_identity.clone(),
                display_label: entry.display_label.clone(),
                disambiguator: entry.disambiguator.clone(),
                package_name: entry.package_name.clone(),
                catalog_snapshot_id: record.snapshot.snapshot_id.clone(),
                kiosk_owner_epoch: record.snapshot.kiosk_owner_epoch.clone(),
                catalog_revision: record.snapshot.catalog_revision.clone(),
                snapshot_digest: record.snapshot.snapshot_digest.clone(),
                catalog_entry_id: entry.entry_id.clone(),
                entry_digest: entry.entry_digest.clone(),
                catalog_age_ms: now_ms.saturating_sub(record.snapshot.observed_at_ms).max(0),
                catalog_freshness: if catalog_current {
                    ProjectionFreshness::Fresh
                } else {
                    ProjectionFreshness::Stale
                },
                catalog_complete: record.snapshot.complete,
                grant_expires_at_ms: record
                    .production_admission
                    .as_ref()
                    .map(|binding| binding.expires_at_ms),
                row,
            });
        }
        candidates.sort_by(|left, right| {
            left.row
                .identity
                .display_name
                .cmp(&right.row.identity.display_name)
                .then_with(|| {
                    left.row
                        .identity
                        .device_id
                        .cmp(&right.row.identity.device_id)
                })
        });
        Ok(KioskAppCandidatesResult {
            schema: KIOSK_APP_CANDIDATES_RESULT_SCHEMA.to_owned(),
            app_identity: app_identity.to_owned(),
            as_of_ms: now_ms,
            candidates,
        })
    }
}
