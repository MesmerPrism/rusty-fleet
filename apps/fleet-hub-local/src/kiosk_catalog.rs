// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use fleet_contracts::{
    KIOSK_CATALOG_SUMMARY_SCOPE, KIOSK_DIRECT_OPERATOR_CAPABILITY_ID,
    KIOSK_V2_CATALOG_REFRESH_PROJECTION_SCHEMA, KIOSK_V2_ENCRYPTED_ROUTE_ID, KioskAppSearchRequest,
    KioskV2CatalogRefreshProjection, KioskV2CatalogRefreshRequest, ProjectionFreshness,
};
use fleet_kiosk_adapter::{KioskV2CatalogExpectation, ingest_production_catalog};

// Target-specific continuity. Ordinary telemetry and unrelated peers may advance.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Continuity {
    source_epoch: String,
    key_id: String,
    key_generation: u64,
    fingerprint: String,
}
fn continuity(
    runtime: &RuntimeState,
    request: &KioskV2CatalogRefreshRequest,
    now_ms: i64,
) -> Result<(Continuity, u64), String> {
    let row = runtime
        .hub
        .inspect(&request.device_id, now_ms)
        .map_err(|error| error.to_string())?
        .row;
    if row.identity.identity_revision != request.identity_revision
        || row.freshness != ProjectionFreshness::Fresh
    {
        return Err("current Fleet identity or freshness changed".to_owned());
    }
    let capability = row
        .capabilities
        .get(KIOSK_DIRECT_OPERATOR_CAPABILITY_ID)
        .ok_or("current Kiosk capability absent")?;
    if capability.owner != "rusty-kiosk"
        || !capability.is_ready()
        || now_ms < capability.observed_at_ms
        || now_ms > capability.fresh_until_ms
    {
        return Err("current Kiosk capability is not ready".to_owned());
    }
    let now = u64::try_from(now_ms).map_err(|_| "clock invalid")?;
    let peer = runtime
        .adapter
        .accepted_peers()
        .peers
        .iter()
        .find(|peer| peer.identity.peer_id.as_str() == request.device_id)
        .filter(|peer| peer.status.observed_at_ms <= now && now < peer.status.expires_at_ms)
        .ok_or("current signed peer evidence absent")?;
    let fingerprint = peer
        .identity
        .key_fingerprint
        .as_str()
        .strip_prefix("fingerprint.")
        .ok_or("peer key fingerprint invalid")?;
    let mut credentials = runtime
        .adapter
        .enrollment()
        .credentials
        .iter()
        .filter(|credential| {
            credential.peer_id == peer.identity.peer_id
                && credential.status == ManifoldPeerCredentialStatus::Active
                && credential.public_key_sha256 == format!("sha256:{fingerprint}")
                && credential.valid_from_ms <= now
                && now < credential.expires_at_ms
        });
    let credential = credentials
        .next()
        .ok_or("current signed peer credential absent or expired")?;
    if credentials.next().is_some() {
        return Err("current signed peer credential ambiguous".to_owned());
    }
    Ok((
        Continuity {
            source_epoch: row.source_epoch,
            key_id: credential.key_id.to_string(),
            key_generation: credential.key_generation,
            fingerprint: fingerprint.to_owned(),
        },
        capability.evidence_revision,
    ))
}
impl LocalHubState {
    pub async fn synchronize_production_kiosk_catalog(
        &self,
        request: KioskV2CatalogRefreshRequest,
        now_ms: i64,
    ) -> Result<KioskV2CatalogRefreshProjection, String> {
        request
            .validate()
            .map_err(|_| "invalid closed catalogue refresh request")?;
        let (expectation, provider, profile_id, captured, permit) = {
            let mut runtime = self.runtime.lock().await;
            let provider = runtime
                .kiosk_v2_catalog_provider
                .clone()
                .ok_or("kiosk_catalog_provider_absent")?;
            let permit = runtime
                .kiosk_catalog_slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| "kiosk_catalog_provider_busy")?;
            let (captured, capability_evidence_revision) = continuity(&runtime, &request, now_ms)?;
            let profile_id = provider
                .lock()
                .map_err(|_| "provider lock failed")?
                .profile_id_for(&request.device_id, request.identity_revision)?;
            let association = runtime.hub.kiosk_catalog_association(&request.device_id);
            if association.is_some_and(|(revision, profile, _)| {
                *revision != request.identity_revision || profile != &profile_id
            }) {
                return Err(
                    "kiosk_catalog_setup_required: stored profile or identity changed".to_owned(),
                );
            }
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).map_err(|_| "request identity unavailable")?;
            let expectation = KioskV2CatalogExpectation {
                request_id: format!("fleet-kiosk-catalog-{}", hex::encode(random)),
                device_id: request.device_id.clone(),
                identity_revision: request.identity_revision,
                capability_id: KIOSK_DIRECT_OPERATOR_CAPABILITY_ID.to_owned(),
                capability_evidence_revision,
                route_id: KIOSK_V2_ENCRYPTED_ROUTE_ID.to_owned(),
                expected_owner_epoch: association.map(|(_, _, owner)| owner.clone()),
                scopes: vec![KIOSK_CATALOG_SUMMARY_SCOPE.to_owned()],
                issued_at_ms: now_ms,
                expires_at_ms: now_ms.checked_add(30_000).ok_or("expiry overflow")?,
            };
            if !runtime
                .inflight_kiosk_catalog_refreshes
                .insert(request.device_id.clone())
            {
                return Err("catalogue refresh already running".to_owned());
            }
            (expectation, provider, profile_id, captured, permit)
        };
        let state = self.clone();
        // Detached owner task retains the permit and clears its marker after every
        // result, including caller cancellation and blocking-provider panic.
        tokio::spawn(async move {
            let _permit = permit;
            let provider_expectation = expectation.clone();
            let provider_for_read = provider.clone();
            let result = tokio::task::spawn_blocking(move || {
                let mut provider = provider_for_read
                    .lock()
                    .map_err(|_| "provider lock failed".to_owned())?;
                let exchange = provider.verified_catalog_exchange(&provider_expectation)?;
                let completed_at_ms = unix_time_ms()?;
                let mut verified = ReturnedExchange(Some(exchange));
                let admission = ingest_production_catalog(
                    &mut verified,
                    &provider_expectation,
                    completed_at_ms,
                )
                .map_err(|error| error.to_string())?;
                Ok::<_, String>((admission, completed_at_ms))
            })
            .await;
            let mut runtime = state.runtime.lock().await;
            runtime
                .inflight_kiosk_catalog_refreshes
                .remove(&request.device_id);
            let (admission, completed_at_ms) = result.map_err(|_| "provider task failed")??;
            let admission_at_ms = unix_time_ms()?;
            if admission_at_ms < completed_at_ms {
                return Err("clock regressed after provider read".to_owned());
            }
            let (current, revision) = continuity(&runtime, &request, admission_at_ms)?;
            if current != captured
                || revision != expectation.capability_evidence_revision
                || completed_at_ms < now_ms
                || admission_at_ms > expectation.expires_at_ms
            {
                return Err("target changed or read expired during catalogue refresh".to_owned());
            }
            if provider
                .lock()
                .map_err(|_| "provider lock failed")?
                .profile_id_for(&request.device_id, request.identity_revision)?
                != profile_id
            {
                return Err("kiosk_catalog_setup_required: provider profile changed".to_owned());
            }
            let mut candidate = runtime.hub.clone();
            candidate
                .accept_profiled_kiosk_catalog(admission.clone(), &profile_id, admission_at_ms)
                .map_err(|error| error.to_string())?;
            let projection = KioskV2CatalogRefreshProjection {
                schema: KIOSK_V2_CATALOG_REFRESH_PROJECTION_SCHEMA.to_owned(),
                admitted: true,
                device_id: request.device_id,
                identity_revision: request.identity_revision,
                kiosk_owner_epoch: admission.binding.kiosk_owner_epoch,
                catalog_revision: admission.binding.catalog_revision,
                snapshot_digest: admission.binding.snapshot_digest,
                replay_identity_sha256: sha256_hex(admission.binding.replay_identity.as_bytes()),
                grant_revision: admission.binding.grant_revision,
                entry_count: admission.snapshot.entries.len(),
                observed_at_ms: admission.snapshot.observed_at_ms,
                fresh_until_ms: admission.snapshot.fresh_until_ms,
                scopes: admission.binding.scopes,
                fleet_revision: candidate.result_revision(),
            };
            projection
                .validate()
                .map_err(|_| "refresh projection invalid")?;
            let RuntimeState {
                hub,
                adapter,
                state_store,
                owner_receipts,
                ..
            } = &mut *runtime;
            state_store.persist(&candidate, adapter, owner_receipts, admission_at_ms)?;
            *hub = candidate;
            Ok(projection)
        })
        .await
        .map_err(|_| "refresh task failed".to_owned())?
    }
}
struct ReturnedExchange(Option<fleet_kiosk_adapter::VerifiedKioskV2CatalogExchange>);
impl fleet_kiosk_adapter::KioskEncryptedOperatorV2Provider for ReturnedExchange {
    fn verified_catalog_exchange(
        &mut self,
        _expectation: &KioskV2CatalogExpectation,
    ) -> Result<fleet_kiosk_adapter::VerifiedKioskV2CatalogExchange, String> {
        self.0
            .take()
            .ok_or_else(|| "verified exchange already consumed".to_owned())
    }
}
pub(super) async fn refresh(State(state): State<LocalHubState>, request: Request) -> Response {
    if !state.catalogue_listener_loopback {
        return api_error(
            StatusCode::FORBIDDEN,
            "catalogue_loopback_required",
            "Catalogue reads require the loopback operator listener",
        );
    }
    if !is_json(request.headers()) {
        return api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content_type_required",
            "JSON required",
        );
    }
    let bytes = match bounded_body(request, MAX_QUERY_BYTES).await {
        Ok(bytes) => bytes,
        Err(response) => return response,
    };
    let request = match serde_json::from_slice::<KioskV2CatalogRefreshRequest>(&bytes) {
        Ok(request) => request,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_catalog_request",
                "closed request required",
            );
        }
    };
    let now = match unix_time_ms() {
        Ok(now) => now,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    match state
        .synchronize_production_kiosk_catalog(request, now)
        .await
    {
        Ok(result) => Json(result).into_response(),
        Err(error) => api_error(
            if error == "kiosk_catalog_provider_absent" {
                StatusCode::NOT_IMPLEMENTED
            } else {
                StatusCode::CONFLICT
            },
            "kiosk_catalog_refresh_refused",
            error,
        ),
    }
}
pub(super) async fn search(State(state): State<LocalHubState>, request: Request) -> Response {
    if !state.catalogue_listener_loopback {
        return api_error(
            StatusCode::FORBIDDEN,
            "catalogue_loopback_required",
            "Catalogue reads require the loopback operator listener",
        );
    }
    if !is_json(request.headers()) {
        return api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content_type_required",
            "JSON required",
        );
    }
    let bytes = match bounded_body(request, MAX_QUERY_BYTES).await {
        Ok(bytes) => bytes,
        Err(response) => return response,
    };
    let request = match serde_json::from_slice::<KioskAppSearchRequest>(&bytes) {
        Ok(request) => request,
        Err(_) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_app_search",
                "closed query required",
            );
        }
    };
    let now = match unix_time_ms() {
        Ok(now) => now,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    match state
        .runtime
        .lock()
        .await
        .hub
        .kiosk_app_search(&request, now)
    {
        Ok(result) => Json(result).into_response(),
        Err(error) => api_error(
            StatusCode::BAD_REQUEST,
            "invalid_app_search",
            error.to_string(),
        ),
    }
}
pub(super) async fn candidates(
    State(state): State<LocalHubState>,
    AxumPath(identity): AxumPath<String>,
) -> Response {
    if !state.catalogue_listener_loopback {
        return api_error(
            StatusCode::FORBIDDEN,
            "catalogue_loopback_required",
            "Catalogue reads require the loopback operator listener",
        );
    }
    let now = match unix_time_ms() {
        Ok(now) => now,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    match state
        .runtime
        .lock()
        .await
        .hub
        .kiosk_app_candidates(&identity, now)
    {
        Ok(result) => Json(result).into_response(),
        Err(error) => api_error(
            StatusCode::BAD_REQUEST,
            "invalid_app_identity",
            error.to_string(),
        ),
    }
}
