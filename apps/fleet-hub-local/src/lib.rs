// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bounded local-network ingress over the deterministic Fleet Hub.
//!
//! The HTTP surface is an adapter. It does not own enrollment, signed
//! check-in, Manifold admission, or projection semantics.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path as FilePath, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::to_bytes;
use axum::extract::{Path as AxumPath, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use fleet_contracts::{FleetQuery, SavedViewMutationRequest, SignedFleetCheckIn};
use fleet_hub::{FleetApi, FleetHub, FleetHubSnapshot, HubPolicy};
use fleet_manifold_adapter::{FleetManifoldAdapter, FleetManifoldAdapterSnapshot};
use rusty_manifold_model::{DottedId, SchemaId};
use rusty_manifold_peer::{
    ManifoldPeerCredentialRecord, ManifoldPeerCredentialStatus, ManifoldPeerEnrollmentAction,
    ManifoldPeerEnrollmentRequest,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tower::limit::GlobalConcurrencyLimitLayer;

const CONFIG_SCHEMA: &str = "rusty.fleet.local_hub_config.v1";
const HEALTH_SCHEMA: &str = "rusty.fleet.local_hub_health.v1";
const ERROR_SCHEMA: &str = "rusty.fleet.local_api_error.v1";
const STATE_SCHEMA: &str = "rusty.fleet.local_hub_durable_state.v1";
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_STATE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CHECKIN_BYTES: usize = 256 * 1024;
const MAX_QUERY_BYTES: usize = 64 * 1024;
const MAX_SAVED_VIEW_BYTES: usize = 128 * 1024;
const MAX_CONCURRENT_REQUESTS: usize = 64;
const RATE_WINDOW_MS: i64 = 10_000;
const MAX_GLOBAL_CHECKINS_PER_WINDOW: usize = 4_096;
const MAX_CHECKINS_PER_CREDENTIAL_PER_WINDOW: usize = 8;
const BODY_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalHubPolicy {
    pub stale_after_ms: i64,
    pub offline_after_ms: i64,
    pub history_limit_per_device: usize,
    pub source_epoch_limit_per_device: usize,
    pub event_limit: usize,
}

impl Default for LocalHubPolicy {
    fn default() -> Self {
        let policy = HubPolicy::default();
        Self {
            stale_after_ms: policy.stale_after_ms,
            offline_after_ms: policy.offline_after_ms,
            history_limit_per_device: policy.history_limit_per_device,
            source_epoch_limit_per_device: policy.source_epoch_limit_per_device,
            event_limit: policy.event_limit,
        }
    }
}

impl From<LocalHubPolicy> for HubPolicy {
    fn from(value: LocalHubPolicy) -> Self {
        Self {
            stale_after_ms: value.stale_after_ms,
            offline_after_ms: value.offline_after_ms,
            history_limit_per_device: value.history_limit_per_device,
            source_epoch_limit_per_device: value.source_epoch_limit_per_device,
            event_limit: value.event_limit,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfiguredEnrollment {
    pub request_id: DottedId,
    pub operator_id: DottedId,
    pub credential: ManifoldPeerCredentialRecord,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalHubConfig {
    pub schema: String,
    pub bind: String,
    #[serde(default)]
    pub allow_non_loopback: bool,
    pub state_directory: PathBuf,
    pub trusted_operator_ids: Vec<DottedId>,
    #[serde(default)]
    pub enrollments: Vec<ConfiguredEnrollment>,
    #[serde(default)]
    pub hub_policy: LocalHubPolicy,
}

impl LocalHubConfig {
    pub fn validate(&self) -> Result<SocketAddr, String> {
        if self.schema != CONFIG_SCHEMA {
            return Err(format!("config schema must be {CONFIG_SCHEMA}"));
        }
        let bind = self
            .bind
            .parse::<SocketAddr>()
            .map_err(|error| format!("bind must be an IP socket address: {error}"))?;
        if !bind.ip().is_loopback() && !self.allow_non_loopback {
            return Err(
                "non-loopback binding requires explicit allow_non_loopback=true".to_owned(),
            );
        }
        if !self.state_directory.is_absolute() {
            return Err("state_directory must be an absolute private path".to_owned());
        }
        if self.state_directory.exists() && !self.state_directory.is_dir() {
            return Err("state_directory must name a directory".to_owned());
        }
        if self.trusted_operator_ids.is_empty() {
            return Err("at least one trusted operator is required".to_owned());
        }
        let operators: BTreeSet<_> = self.trusted_operator_ids.iter().cloned().collect();
        if operators.len() != self.trusted_operator_ids.len() {
            return Err("trusted operator identifiers must be unique".to_owned());
        }
        if self
            .enrollments
            .iter()
            .any(|enrollment| !operators.contains(&enrollment.operator_id))
        {
            return Err("every enrollment operator must be trusted by this config".to_owned());
        }
        if self.hub_policy.stale_after_ms <= 0
            || self.hub_policy.offline_after_ms <= self.hub_policy.stale_after_ms
            || self.hub_policy.history_limit_per_device == 0
            || self.hub_policy.source_epoch_limit_per_device == 0
            || self.hub_policy.event_limit == 0
        {
            return Err("hub policy limits must be positive and freshness ordered".to_owned());
        }
        Ok(bind)
    }
}

struct RuntimeState {
    hub: FleetHub,
    adapter: FleetManifoldAdapter,
    rate_limiter: IngressRateLimiter,
    state_store: DurableStateStore,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LocalHubDurableState {
    schema: String,
    generation: u64,
    written_at_ms: i64,
    hub: FleetHubSnapshot,
    adapter: FleetManifoldAdapterSnapshot,
}

struct DurableStateStore {
    directory: PathBuf,
    generation: u64,
    restored: bool,
}

#[derive(Default)]
struct CounterWindow {
    started_at_ms: i64,
    count: usize,
}

#[derive(Default)]
struct IngressRateLimiter {
    global: CounterWindow,
    by_credential: BTreeMap<String, CounterWindow>,
}

impl IngressRateLimiter {
    fn admit(&mut self, enrolled_key_id: Option<&str>, now_ms: i64) -> bool {
        roll_window(&mut self.global, now_ms);
        self.by_credential
            .retain(|_, window| window_age_ms(window, now_ms) < RATE_WINDOW_MS);
        if self.global.count >= MAX_GLOBAL_CHECKINS_PER_WINDOW {
            return false;
        }
        self.global.count = self.global.count.saturating_add(1);
        let Some(key_id) = enrolled_key_id else {
            return true;
        };
        let window = self.by_credential.entry(key_id.to_owned()).or_default();
        roll_window(window, now_ms);
        if window.count >= MAX_CHECKINS_PER_CREDENTIAL_PER_WINDOW {
            return false;
        }
        window.count = window.count.saturating_add(1);
        true
    }
}

impl DurableStateStore {
    fn open(
        directory: &FilePath,
        hub: &mut FleetHub,
        adapter: &mut FleetManifoldAdapter,
        now_ms: i64,
    ) -> Result<Self, String> {
        fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create state directory: {error}"))?;
        let Some(state) = load_latest_state(directory)? else {
            return Ok(Self {
                directory: directory.to_path_buf(),
                generation: 0,
                restored: false,
            });
        };
        let restored_hub = FleetHub::restore(hub.policy(), state.hub)
            .map_err(|error| format!("cannot restore Fleet Hub state: {error}"))?;
        adapter
            .restore_session(state.adapter, now_ms)
            .map_err(|error| format!("cannot restore Manifold adapter state: {error}"))?;
        let hub_ids: BTreeSet<_> = restored_hub.device_ids().into_iter().collect();
        let accepted_ids: BTreeSet<_> = adapter.accepted_peer_ids().into_iter().collect();
        if hub_ids != accepted_ids {
            return Err(
                "durable Fleet Hub devices do not match accepted Manifold peers".to_owned(),
            );
        }
        *hub = restored_hub;
        Ok(Self {
            directory: directory.to_path_buf(),
            generation: state.generation,
            restored: true,
        })
    }

    fn persist(
        &mut self,
        hub: &FleetHub,
        adapter: &FleetManifoldAdapter,
        now_ms: i64,
    ) -> Result<(), String> {
        let hub_ids: BTreeSet<_> = hub.device_ids().into_iter().collect();
        let accepted_ids: BTreeSet<_> = adapter.accepted_peer_ids().into_iter().collect();
        if hub_ids != accepted_ids {
            return Err(
                "refusing to persist mismatched Fleet Hub and Manifold authority".to_owned(),
            );
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| "durable state generation is exhausted".to_owned())?;
        let state = LocalHubDurableState {
            schema: STATE_SCHEMA.to_owned(),
            generation,
            written_at_ms: now_ms,
            hub: hub.snapshot(),
            adapter: adapter.snapshot(),
        };
        let bytes = serde_json::to_vec(&state)
            .map_err(|error| format!("cannot serialize durable state: {error}"))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_STATE_BYTES {
            return Err("durable state exceeds the 16 MiB limit".to_owned());
        }
        fs::create_dir_all(&self.directory)
            .map_err(|error| format!("cannot create state directory: {error}"))?;
        let slot = generation % 2;
        let target = state_slot_path(&self.directory, slot);
        let temporary = self.directory.join(format!("fleet-hub-state.{slot}.tmp"));
        if temporary.exists() {
            fs::remove_file(&temporary)
                .map_err(|error| format!("cannot remove stale state temporary: {error}"))?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("cannot create state temporary: {error}"))?;
        file.write_all(&bytes)
            .map_err(|error| format!("cannot write state temporary: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("cannot sync state temporary: {error}"))?;
        drop(file);
        if target.exists() {
            fs::remove_file(&target)
                .map_err(|error| format!("cannot replace prior state slot: {error}"))?;
        }
        fs::rename(&temporary, &target)
            .map_err(|error| format!("cannot publish durable state slot: {error}"))?;
        self.generation = generation;
        self.restored = true;
        Ok(())
    }
}

fn load_latest_state(directory: &FilePath) -> Result<Option<LocalHubDurableState>, String> {
    let mut states = Vec::new();
    let mut found_slot = false;
    let mut failures = Vec::new();
    for slot in 0..=1 {
        let path = state_slot_path(directory, slot);
        if !path.exists() {
            continue;
        }
        found_slot = true;
        match read_state_slot(&path) {
            Ok(state) => states.push(state),
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    states.sort_by_key(|state| state.generation);
    if let Some(state) = states.pop() {
        return Ok(Some(state));
    }
    if found_slot {
        return Err(format!(
            "no valid durable state slot remains: {}",
            failures.join("; ")
        ));
    }
    Ok(None)
}

fn read_state_slot(path: &FilePath) -> Result<LocalHubDurableState, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("cannot inspect state slot: {error}"))?;
    if metadata.len() > MAX_STATE_BYTES {
        return Err("state slot exceeds the 16 MiB limit".to_owned());
    }
    let bytes = fs::read(path).map_err(|error| format!("cannot read state slot: {error}"))?;
    let state: LocalHubDurableState =
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid state JSON: {error}"))?;
    if state.schema != STATE_SCHEMA || state.generation == 0 || state.written_at_ms < 0 {
        return Err("state slot header is invalid".to_owned());
    }
    Ok(state)
}

fn state_slot_path(directory: &FilePath, slot: u64) -> PathBuf {
    directory.join(format!("fleet-hub-state.{slot}.json"))
}

#[derive(Clone)]
pub struct LocalHubState {
    runtime: Arc<Mutex<RuntimeState>>,
}

impl LocalHubState {
    pub fn from_config(config: &LocalHubConfig, now_ms: i64) -> Result<Self, String> {
        config.validate()?;
        let now_unsigned =
            u64::try_from(now_ms).map_err(|_| "current time must be nonnegative".to_owned())?;
        let mut adapter = FleetManifoldAdapter::new(config.trusted_operator_ids.clone());
        for enrollment in &config.enrollments {
            let request = ManifoldPeerEnrollmentRequest {
                schema_id: schema_id("rusty.manifold.peer.enrollment_request.v1")?,
                request_id: enrollment.request_id.clone(),
                expected_authority_revision: adapter.enrollment().authority_revision,
                operator_id: enrollment.operator_id.clone(),
                issued_at_ms: now_unsigned,
                action: ManifoldPeerEnrollmentAction::Enroll {
                    credential: enrollment.credential.clone(),
                },
            };
            let receipt = adapter.apply_enrollment(&request, now_unsigned);
            if !receipt.applied {
                return Err(format!(
                    "enrollment {} was rejected: {:?}",
                    enrollment.request_id, receipt.rejection_reason
                ));
            }
        }
        let mut hub = FleetHub::new(config.hub_policy.clone().into());
        let state_store =
            DurableStateStore::open(&config.state_directory, &mut hub, &mut adapter, now_ms)?;
        Ok(Self {
            runtime: Arc::new(Mutex::new(RuntimeState {
                hub,
                adapter,
                rate_limiter: IngressRateLimiter::default(),
                state_store,
            })),
        })
    }
}

#[derive(Debug, Serialize)]
struct ApiError {
    schema: &'static str,
    code: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct HealthProjection {
    schema: &'static str,
    status: &'static str,
    now_ms: i64,
    enrolled_credentials: usize,
    accepted_devices: usize,
    durable_generation: u64,
    durable_state: &'static str,
}

#[derive(Debug, Deserialize)]
struct WatchQuery {
    #[serde(default)]
    after_sequence: u64,
    #[serde(default = "default_watch_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
struct SavedViewRevisionQuery {
    expected_revision: u64,
}

fn default_watch_limit() -> usize {
    100
}

pub fn router(state: LocalHubState) -> Router {
    Router::new()
        .route("/fleet/v1/health", get(health))
        .route("/fleet/v1/checkins", post(checkin))
        .route("/fleet/v1/query", post(query_devices))
        .route("/fleet/v1/summary", get(summary))
        .route("/fleet/v1/saved-views", get(saved_views))
        .route(
            "/fleet/v1/saved-views/{view_id}",
            get(saved_view)
                .put(upsert_saved_view)
                .delete(delete_saved_view),
        )
        .route("/fleet/v1/devices/{device_id}", get(device_detail))
        .route("/fleet/v1/devices/{device_id}/inspect", get(device_inspect))
        .route("/fleet/v1/watch", get(watch))
        .with_state(state)
        .layer(GlobalConcurrencyLimitLayer::new(MAX_CONCURRENT_REQUESTS))
}

pub async fn serve(config: LocalHubConfig) -> Result<(), String> {
    let bind = config.validate()?;
    let state = LocalHubState::from_config(&config, unix_time_ms()?)?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|error| format!("failed to bind {bind}: {error}"))?;
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| format!("local Hub server failed: {error}"))
}

pub fn load_config(path: &std::path::Path) -> Result<LocalHubConfig, String> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("cannot inspect config: {error}"))?;
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err("local Hub config exceeds the 1 MiB limit".to_owned());
    }
    let bytes = std::fs::read(path).map_err(|error| format!("cannot read config: {error}"))?;
    let config: LocalHubConfig =
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid config JSON: {error}"))?;
    config.validate()?;
    Ok(config)
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn health(State(state): State<LocalHubState>) -> Response {
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let runtime = state.runtime.lock().await;
    Json(HealthProjection {
        schema: HEALTH_SCHEMA,
        status: "ready",
        now_ms,
        enrolled_credentials: runtime.adapter.enrollment().credentials.len(),
        accepted_devices: runtime.hub.device_count(),
        durable_generation: runtime.state_store.generation,
        durable_state: if runtime.state_store.restored {
            "restored_or_persisted"
        } else {
            "new"
        },
    })
    .into_response()
}

async fn checkin(State(state): State<LocalHubState>, request: Request) -> Response {
    if !is_json(request.headers()) {
        return api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content_type_required",
            "check-ins require Content-Type: application/json",
        );
    }
    let bytes = match bounded_body(request, MAX_CHECKIN_BYTES).await {
        Ok(bytes) => bytes,
        Err(response) => return response,
    };
    let signed = match serde_json::from_slice::<SignedFleetCheckIn>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_checkin_json",
                format!("check-in is not a valid signed envelope: {error}"),
            );
        }
    };
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let mut runtime = state.runtime.lock().await;
    let enrolled_key_id = runtime
        .adapter
        .enrollment()
        .credentials
        .iter()
        .find(|credential| {
            credential.key_id.as_str() == signed.key_id
                && credential.status == ManifoldPeerCredentialStatus::Active
        })
        .map(|credential| credential.key_id.to_string());
    if !runtime
        .rate_limiter
        .admit(enrolled_key_id.as_deref(), now_ms)
    {
        return api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "checkin_rate_exceeded",
            "the bounded local check-in rate was exceeded",
        );
    }
    let RuntimeState {
        hub,
        adapter,
        state_store,
        ..
    } = &mut *runtime;
    let mut candidate_hub = hub.clone();
    let mut candidate_adapter = adapter.clone();
    let receipt = candidate_adapter.accept(&mut candidate_hub, signed, now_ms);
    if receipt.accepted {
        if let Err(error) = state_store.persist(&candidate_hub, &candidate_adapter, now_ms) {
            return api_error(
                StatusCode::INSUFFICIENT_STORAGE,
                "durable_state_failed",
                error,
            );
        }
        *hub = candidate_hub;
        *adapter = candidate_adapter;
    }
    let status = if receipt.accepted {
        StatusCode::OK
    } else {
        StatusCode::CONFLICT
    };
    (status, Json(receipt)).into_response()
}

async fn query_devices(State(state): State<LocalHubState>, request: Request) -> Response {
    if !is_json(request.headers()) {
        return api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content_type_required",
            "queries require Content-Type: application/json",
        );
    }
    let bytes = match bounded_body(request, MAX_QUERY_BYTES).await {
        Ok(bytes) => bytes,
        Err(response) => return response,
    };
    let query = match serde_json::from_slice::<FleetQuery>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_query_json",
                format!("query is not valid JSON: {error}"),
            );
        }
    };
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let runtime = state.runtime.lock().await;
    match runtime.hub.list(&query, now_ms) {
        Ok(result) => Json(result).into_response(),
        Err(error) => api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_query",
            error.to_string(),
        ),
    }
}

async fn summary(State(state): State<LocalHubState>) -> Response {
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let runtime = state.runtime.lock().await;
    Json(runtime.hub.summary(now_ms)).into_response()
}

async fn saved_views(State(state): State<LocalHubState>) -> Response {
    let runtime = state.runtime.lock().await;
    Json(runtime.hub.saved_views()).into_response()
}

async fn saved_view(
    State(state): State<LocalHubState>,
    AxumPath(view_id): AxumPath<String>,
) -> Response {
    let runtime = state.runtime.lock().await;
    match runtime.hub.saved_view(&view_id) {
        Ok(view) => Json(view).into_response(),
        Err(error) => api_error(
            StatusCode::NOT_FOUND,
            "saved_view_not_found",
            error.to_string(),
        ),
    }
}

async fn upsert_saved_view(
    State(state): State<LocalHubState>,
    AxumPath(view_id): AxumPath<String>,
    request: Request,
) -> Response {
    if !is_json(request.headers()) {
        return api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content_type_required",
            "saved-view mutations require Content-Type: application/json",
        );
    }
    let bytes = match bounded_body(request, MAX_SAVED_VIEW_BYTES).await {
        Ok(bytes) => bytes,
        Err(response) => return response,
    };
    let mutation = match serde_json::from_slice::<SavedViewMutationRequest>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_saved_view_json",
                format!("saved-view mutation is not valid JSON: {error}"),
            );
        }
    };
    if mutation.view.view_id != view_id {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "saved_view_identity_mismatch",
            "saved-view path and payload identities must match",
        );
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let mut runtime = state.runtime.lock().await;
    let RuntimeState {
        hub,
        adapter,
        state_store,
        ..
    } = &mut *runtime;
    let mut candidate_hub = hub.clone();
    let receipt = match candidate_hub.upsert_saved_view(mutation) {
        Ok(receipt) => receipt,
        Err(error) => return saved_view_error(error),
    };
    if receipt.changed {
        if let Err(error) = state_store.persist(&candidate_hub, adapter, now_ms) {
            return api_error(
                StatusCode::INSUFFICIENT_STORAGE,
                "durable_state_failed",
                error,
            );
        }
        *hub = candidate_hub;
    }
    Json(receipt).into_response()
}

async fn delete_saved_view(
    State(state): State<LocalHubState>,
    AxumPath(view_id): AxumPath<String>,
    Query(query): Query<SavedViewRevisionQuery>,
) -> Response {
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let mut runtime = state.runtime.lock().await;
    let RuntimeState {
        hub,
        adapter,
        state_store,
        ..
    } = &mut *runtime;
    let mut candidate_hub = hub.clone();
    let receipt = match candidate_hub.delete_saved_view(&view_id, query.expected_revision) {
        Ok(receipt) => receipt,
        Err(error) => return saved_view_error(error),
    };
    if let Err(error) = state_store.persist(&candidate_hub, adapter, now_ms) {
        return api_error(
            StatusCode::INSUFFICIENT_STORAGE,
            "durable_state_failed",
            error,
        );
    }
    *hub = candidate_hub;
    Json(receipt).into_response()
}

fn saved_view_error(error: fleet_hub::HubError) -> Response {
    let (status, code) = match error.code.as_str() {
        "saved_view_not_found" => (StatusCode::NOT_FOUND, "saved_view_not_found"),
        "saved_view_revision_conflict" => (StatusCode::CONFLICT, "saved_view_revision_conflict"),
        "saved_view_limit_exceeded" => (StatusCode::CONFLICT, "saved_view_limit_exceeded"),
        _ => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_saved_view"),
    };
    api_error(status, code, error.to_string())
}

async fn device_inspect(
    State(state): State<LocalHubState>,
    AxumPath(device_id): AxumPath<String>,
) -> Response {
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let runtime = state.runtime.lock().await;
    match runtime.hub.inspect(&device_id, now_ms) {
        Ok(result) => Json(result).into_response(),
        Err(error) => api_error(StatusCode::NOT_FOUND, "device_not_found", error.to_string()),
    }
}

async fn device_detail(
    State(state): State<LocalHubState>,
    AxumPath(device_id): AxumPath<String>,
) -> Response {
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let runtime = state.runtime.lock().await;
    match runtime.hub.detail(&device_id, now_ms) {
        Ok(result) => Json(result).into_response(),
        Err(error) => api_error(StatusCode::NOT_FOUND, "device_not_found", error.to_string()),
    }
}

async fn watch(State(state): State<LocalHubState>, Query(query): Query<WatchQuery>) -> Response {
    if query.limit == 0 || query.limit > 10_000 {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_watch_limit",
            "watch limit must be between 1 and 10000",
        );
    }
    let runtime = state.runtime.lock().await;
    Json(runtime.hub.watch(query.after_sequence, query.limit)).into_response()
}

async fn bounded_body(request: Request, limit: usize) -> Result<axum::body::Bytes, Response> {
    if request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > limit)
    {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "body_limit_exceeded",
            format!("request body exceeds the {limit}-byte limit"),
        ));
    }
    match timeout(BODY_DEADLINE, to_bytes(request.into_body(), limit)).await {
        Err(_) => Err(api_error(
            StatusCode::REQUEST_TIMEOUT,
            "body_deadline_exceeded",
            "request body did not complete within five seconds",
        )),
        Ok(Err(error)) => Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "body_unreadable_or_too_large",
            format!("request body could not be read within its bound: {error}"),
        )),
        Ok(Ok(bytes)) => Ok(bytes),
    }
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(';').next().is_some_and(|media_type| {
                media_type.trim().eq_ignore_ascii_case("application/json")
            })
        })
}

fn api_error(status: StatusCode, code: &'static str, message: impl Into<String>) -> Response {
    (
        status,
        Json(ApiError {
            schema: ERROR_SCHEMA,
            code,
            message: message.into(),
        }),
    )
        .into_response()
}

fn unix_time_ms() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?;
    i64::try_from(duration.as_millis()).map_err(|_| "system time exceeds i64 millis".to_owned())
}

fn roll_window(window: &mut CounterWindow, now_ms: i64) {
    if window.started_at_ms == 0
        || now_ms < window.started_at_ms
        || window_age_ms(window, now_ms) >= RATE_WINDOW_MS
    {
        window.started_at_ms = now_ms;
        window.count = 0;
    }
}

fn window_age_ms(window: &CounterWindow, now_ms: i64) -> i64 {
    now_ms.saturating_sub(window.started_at_ms).max(0)
}

fn schema_id(value: &str) -> Result<SchemaId, String> {
    SchemaId::new(value.to_owned()).map_err(|error| format!("invalid static schema id: {error}"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use ed25519_dalek::{Signer, SigningKey};
    use fleet_contracts::{
        CHECKIN_SIGNATURE_ALGORITHM, FleetCheckInClaims, FleetQuery, NavigationRestoration,
        SavedView, SavedViewMutationRequest, SignedFleetCheckIn,
    };
    use fleet_simulator::ScenarioBuilder;
    use rusty_manifold_model::{DottedId, Revision, SchemaId};
    use rusty_manifold_peer::{
        ManifoldPeerAvailability, ManifoldPeerCredentialAlgorithm, ManifoldPeerCredentialRecord,
        ManifoldPeerCredentialStatus, ManifoldPeerIdentity, ManifoldPeerPayloadClass,
        ManifoldPeerRole, ManifoldPeerStatus, ManifoldPeerStatusProposal,
    };
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;

    use super::{
        ConfiguredEnrollment, IngressRateLimiter, LocalHubConfig, LocalHubState,
        MAX_CHECKINS_PER_CREDENTIAL_PER_WINDOW, router, state_slot_path, unix_time_ms,
    };

    static STATE_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[tokio::test]
    async fn signed_checkin_query_and_replay_share_one_authority() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let (config, key_id) = config(&signing_key, now_ms);
        let state_directory = config.state_directory.clone();
        let state = LocalHubState::from_config(&config, now_ms).expect("valid config");
        let app = router(state);
        let signed = signed_checkin(&signing_key, key_id.as_str(), now_ms, 1);

        let accepted = app
            .clone()
            .oneshot(json_request(
                "/fleet/v1/checkins",
                serde_json::to_vec(&signed).expect("signed JSON"),
            ))
            .await
            .expect("check-in response");
        let accepted_status = accepted.status();
        let accepted_body = to_bytes(accepted.into_body(), 64 * 1024)
            .await
            .expect("accepted body");
        let accepted_json: Value = serde_json::from_slice(&accepted_body).expect("accepted JSON");
        assert_eq!(
            accepted_status,
            StatusCode::OK,
            "unexpected receipt: {accepted_json}"
        );

        let query = FleetQuery {
            schema: "rusty.fleet.query.v1".to_owned(),
            query_id: "test.all".to_owned(),
            expression: None,
            sort: Vec::new(),
            offset: 0,
            limit: 10,
        };
        let listed = app
            .clone()
            .oneshot(json_request(
                "/fleet/v1/query",
                serde_json::to_vec(&query).expect("query JSON"),
            ))
            .await
            .expect("query response");
        assert_eq!(listed.status(), StatusCode::OK);
        let listed_body = to_bytes(listed.into_body(), 64 * 1024)
            .await
            .expect("query body");
        let listed_json: Value = serde_json::from_slice(&listed_body).expect("query JSON");
        assert_eq!(listed_json["total_count"], 1);
        assert_eq!(
            listed_json["rows"][0]["identity"]["device_id"],
            "device.quest.1"
        );

        let replay = app
            .oneshot(json_request(
                "/fleet/v1/checkins",
                serde_json::to_vec(&signed).expect("signed JSON"),
            ))
            .await
            .expect("replay response");
        assert_eq!(replay.status(), StatusCode::CONFLICT);
        let replay_body = to_bytes(replay.into_body(), 64 * 1024)
            .await
            .expect("replay body");
        let replay_json: Value = serde_json::from_slice(&replay_body).expect("replay JSON");
        assert_eq!(replay_json["accepted"], false);
        assert_eq!(replay_json["rejection_reason"], "replay");

        let restored = LocalHubState::from_config(&config, now_ms + 3).expect("restored config");
        let restored_app = router(restored);
        let restored_query = restored_app
            .clone()
            .oneshot(json_request(
                "/fleet/v1/query",
                serde_json::to_vec(&query).expect("query JSON"),
            ))
            .await
            .expect("restored query response");
        assert_eq!(restored_query.status(), StatusCode::OK);
        let restored_body = to_bytes(restored_query.into_body(), 64 * 1024)
            .await
            .expect("restored query body");
        let restored_json: Value =
            serde_json::from_slice(&restored_body).expect("restored query JSON");
        assert_eq!(restored_json["total_count"], 1);

        let restored_replay = restored_app
            .oneshot(json_request(
                "/fleet/v1/checkins",
                serde_json::to_vec(&signed).expect("signed JSON"),
            ))
            .await
            .expect("restored replay response");
        assert_eq!(restored_replay.status(), StatusCode::CONFLICT);
        let restored_replay_body = to_bytes(restored_replay.into_body(), 64 * 1024)
            .await
            .expect("restored replay body");
        let restored_replay_json: Value =
            serde_json::from_slice(&restored_replay_body).expect("restored replay JSON");
        assert_eq!(restored_replay_json["rejection_reason"], "replay");
        fs::remove_dir_all(state_directory).expect("remove test state directory");
    }

    #[tokio::test]
    async fn ingress_rejects_wrong_content_type_and_oversize_body() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let (config, _) = config(&signing_key, now_ms);
        let state_directory = config.state_directory.clone();
        let state = LocalHubState::from_config(&config, now_ms).expect("valid config");
        let app = router(state);

        let wrong_type = Request::builder()
            .method("POST")
            .uri("/fleet/v1/checkins")
            .body(Body::from("{}"))
            .expect("request");
        let response = app
            .clone()
            .oneshot(wrong_type)
            .await
            .expect("wrong-type response");
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let oversize = Request::builder()
            .method("POST")
            .uri("/fleet/v1/checkins")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, (256 * 1024 + 1).to_string())
            .body(Body::empty())
            .expect("request");
        let response = app.oneshot(oversize).await.expect("oversize response");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        fs::remove_dir_all(state_directory).expect("remove test state directory");
    }

    #[tokio::test]
    async fn saved_view_routes_preserve_revision_and_durable_restoration() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let (config, _) = config(&signing_key, now_ms);
        let state_directory = config.state_directory.clone();
        let app = router(LocalHubState::from_config(&config, now_ms).expect("valid config"));
        let mutation = SavedViewMutationRequest {
            schema: "rusty.fleet.saved_view_mutation_request.v1".to_owned(),
            expected_revision: 1,
            view: saved_view(),
        };

        let saved = app
            .clone()
            .oneshot(json_method_request(
                "PUT",
                "/fleet/v1/saved-views/view.needs_attention",
                serde_json::to_vec(&mutation).expect("mutation JSON"),
            ))
            .await
            .expect("saved-view response");
        assert_eq!(saved.status(), StatusCode::OK);
        let saved_body = to_bytes(saved.into_body(), 128 * 1024)
            .await
            .expect("saved-view body");
        let saved_json: Value = serde_json::from_slice(&saved_body).expect("saved-view JSON");
        assert_eq!(saved_json["previous_revision"], 1);
        assert_eq!(saved_json["current_revision"], 2);
        assert_eq!(saved_json["changed"], true);

        let stale = app
            .clone()
            .oneshot(json_method_request(
                "PUT",
                "/fleet/v1/saved-views/view.needs_attention",
                serde_json::to_vec(&mutation).expect("mutation JSON"),
            ))
            .await
            .expect("stale mutation response");
        assert_eq!(stale.status(), StatusCode::CONFLICT);

        drop(app);
        let restored =
            router(LocalHubState::from_config(&config, now_ms + 10).expect("restored config"));
        let listed = restored
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/fleet/v1/saved-views")
                    .body(Body::empty())
                    .expect("list request"),
            )
            .await
            .expect("list response");
        assert_eq!(listed.status(), StatusCode::OK);
        let listed_body = to_bytes(listed.into_body(), 128 * 1024)
            .await
            .expect("list body");
        let listed_json: Value = serde_json::from_slice(&listed_body).expect("list JSON");
        assert_eq!(listed_json["revision"], 2);
        assert_eq!(listed_json["views"][0]["view_id"], "view.needs_attention");
        assert_eq!(
            listed_json["views"][0]["restoration"]["focused_region"],
            "grid"
        );

        let deleted = restored
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/fleet/v1/saved-views/view.needs_attention?expected_revision=2")
                    .body(Body::empty())
                    .expect("delete request"),
            )
            .await
            .expect("delete response");
        assert_eq!(deleted.status(), StatusCode::OK);
        let deleted_body = to_bytes(deleted.into_body(), 128 * 1024)
            .await
            .expect("delete body");
        let deleted_json: Value = serde_json::from_slice(&deleted_body).expect("delete JSON");
        assert_eq!(deleted_json["current_revision"], 3);
        assert_eq!(deleted_json["deleted"], true);

        let missing = restored
            .oneshot(
                Request::builder()
                    .uri("/fleet/v1/saved-views/view.needs_attention")
                    .body(Body::empty())
                    .expect("get request"),
            )
            .await
            .expect("get response");
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        fs::remove_dir_all(state_directory).expect("remove test state directory");
    }

    #[tokio::test]
    async fn damaged_newest_state_slot_falls_back_and_can_be_replayed_forward() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let (config, key_id) = config(&signing_key, now_ms);
        let state_directory = config.state_directory.clone();
        let app = router(LocalHubState::from_config(&config, now_ms).expect("valid config"));
        let first = signed_checkin(&signing_key, key_id.as_str(), now_ms, 1);
        let second = signed_checkin(&signing_key, key_id.as_str(), now_ms + 10, 2);

        for signed in [&first, &second] {
            let response = app
                .clone()
                .oneshot(json_request(
                    "/fleet/v1/checkins",
                    serde_json::to_vec(signed).expect("signed JSON"),
                ))
                .await
                .expect("check-in response");
            assert_eq!(response.status(), StatusCode::OK);
        }
        drop(app);

        fs::write(state_slot_path(&state_directory, 0), b"{damaged")
            .expect("damage newest state slot");
        let restored =
            LocalHubState::from_config(&config, now_ms + 20).expect("fallback state restored");
        {
            let runtime = restored.runtime.lock().await;
            assert_eq!(runtime.state_store.generation, 1);
            assert_eq!(runtime.hub.device_count(), 1);
        }
        let replay_forward = router(restored)
            .oneshot(json_request(
                "/fleet/v1/checkins",
                serde_json::to_vec(&second).expect("second signed JSON"),
            ))
            .await
            .expect("replay-forward response");
        assert_eq!(replay_forward.status(), StatusCode::OK);
        fs::remove_dir_all(state_directory).expect("remove test state directory");
    }

    #[test]
    fn non_loopback_binding_requires_explicit_activation() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let (mut config, _) = config(&signing_key, now_ms);
        config.bind = "0.0.0.0:8741".to_owned();
        assert!(config.validate().is_err());
        config.allow_non_loopback = true;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn per_credential_rate_is_finite_without_unbounded_identifiers() {
        let mut limiter = IngressRateLimiter::default();
        for _ in 0..MAX_CHECKINS_PER_CREDENTIAL_PER_WINDOW {
            assert!(limiter.admit(Some("key.device.quest.1"), 10_000));
        }
        assert!(!limiter.admit(Some("key.device.quest.1"), 10_000));
        assert!(limiter.admit(Some("key.device.quest.1"), 20_000));
        for index in 0..100 {
            assert!(limiter.admit(None, 20_001 + index));
        }
        assert_eq!(limiter.by_credential.len(), 1);
    }

    fn config(signing_key: &SigningKey, now_ms: i64) -> (LocalHubConfig, DottedId) {
        let public_key = signing_key.verifying_key().to_bytes();
        let digest = hex::encode(Sha256::digest(public_key));
        let peer_id = dotted("device.quest.1");
        let key_id = dotted("key.device.quest.1");
        let operator_id = dotted("operator.local");
        (
            LocalHubConfig {
                schema: "rusty.fleet.local_hub_config.v1".to_owned(),
                bind: "127.0.0.1:8741".to_owned(),
                allow_non_loopback: false,
                state_directory: test_state_directory(),
                trusted_operator_ids: vec![operator_id.clone()],
                enrollments: vec![ConfiguredEnrollment {
                    request_id: dotted("request.enroll.quest.1"),
                    operator_id,
                    credential: ManifoldPeerCredentialRecord {
                        schema_id: schema("rusty.manifold.peer.credential_record.v1"),
                        credential_id: dotted("credential.device.quest.1"),
                        peer_id,
                        trust_domain: dotted("trust.local"),
                        key_id: key_id.clone(),
                        key_generation: 1,
                        algorithm: ManifoldPeerCredentialAlgorithm::Ed25519,
                        public_key_hex: hex::encode(public_key),
                        public_key_sha256: format!("sha256:{digest}"),
                        valid_from_ms: u64::try_from(now_ms - 60_000).expect("positive"),
                        expires_at_ms: u64::try_from(now_ms + 600_000).expect("positive"),
                        status: ManifoldPeerCredentialStatus::Active,
                        replaced_by_key_id: None,
                    },
                }],
                hub_policy: Default::default(),
            },
            key_id,
        )
    }

    fn signed_checkin(
        signing_key: &SigningKey,
        key_id: &str,
        now_ms: i64,
        revision: u64,
    ) -> SignedFleetCheckIn {
        let peer_id = dotted("device.quest.1");
        let fingerprint = {
            let digest = hex::encode(Sha256::digest(signing_key.verifying_key().to_bytes()));
            dotted(&format!("fingerprint.{digest}"))
        };
        let mut observation = ScenarioBuilder::new(1).build().initial.remove(0);
        observation.identity.device_id = peer_id.to_string();
        observation.source_revision = revision;
        observation.source_time_ms = now_ms;
        observation.received_time_ms = 0;
        for provenance in [
            observation
                .agent
                .as_mut()
                .map(|value| &mut value.provenance),
            observation
                .power
                .as_mut()
                .map(|value| &mut value.provenance),
            observation
                .application
                .as_mut()
                .map(|value| &mut value.provenance),
        ]
        .into_iter()
        .flatten()
        {
            provenance.observed_at_ms = now_ms;
            provenance.fresh_until_ms = now_ms + 60_000;
        }
        for condition in &mut observation.conditions {
            condition.source_time_ms = now_ms;
            condition.received_time_ms = 0;
            condition.fresh_until_ms = now_ms + 60_000;
        }
        for capability in observation.capabilities.capabilities.values_mut() {
            capability.observed_at_ms = now_ms;
            capability.fresh_until_ms = now_ms + 60_000;
        }
        let proposal = ManifoldPeerStatusProposal {
            schema_id: schema("rusty.manifold.peer.status_proposal.v1"),
            proposal_id: dotted(&format!("proposal.status.quest.{revision}")),
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
                status_revision: Revision::new(revision).expect("positive status revision"),
                observed_at_ms: u64::try_from(now_ms).expect("positive"),
                expires_at_ms: u64::try_from(now_ms + 60_000).expect("positive"),
                availability: ManifoldPeerAvailability::Ready,
                capability_ids: vec![dotted("capability.monitoring")],
            },
            payload_class: ManifoldPeerPayloadClass::LowRateDescriptor,
        };
        let claims = FleetCheckInClaims {
            schema: "rusty.fleet.checkin_claims.v1".to_owned(),
            checkin_id: format!("checkin.quest.{revision}"),
            issued_at_ms: now_ms,
            expires_at_ms: now_ms + 60_000,
            manifold_peer_status_proposal: serde_json::to_value(proposal).expect("proposal JSON"),
            observation,
            extensions: Default::default(),
        };
        let message = claims.signing_bytes().expect("signing bytes");
        SignedFleetCheckIn {
            schema: "rusty.fleet.signed_checkin.v1".to_owned(),
            key_id: key_id.to_owned(),
            algorithm: CHECKIN_SIGNATURE_ALGORITHM.to_owned(),
            signature_hex: hex::encode(signing_key.sign(&message).to_bytes()),
            claims,
        }
    }

    fn saved_view() -> SavedView {
        SavedView {
            schema: "rusty.fleet.saved_view.v1".to_owned(),
            view_id: "view.needs_attention".to_owned(),
            name: "Needs attention".to_owned(),
            query: FleetQuery {
                schema: "rusty.fleet.query.v1".to_owned(),
                query_id: "query.needs_attention".to_owned(),
                expression: None,
                sort: Vec::new(),
                offset: 0,
                limit: 250,
            },
            columns: vec![
                "device".to_owned(),
                "age".to_owned(),
                "attention".to_owned(),
            ],
            density: "standard".to_owned(),
            grouping: None,
            restoration: NavigationRestoration {
                selected_device_id: Some("device.quest.1".to_owned()),
                inspector_tab: Some("overview".to_owned()),
                scroll_anchor_device_id: Some("device.quest.1".to_owned()),
                focused_region: Some("grid".to_owned()),
                collapsed_groups: Vec::new(),
            },
            schema_version: 1,
        }
    }

    fn json_request(uri: &str, body: Vec<u8>) -> Request<Body> {
        json_method_request("POST", uri, body)
    }

    fn json_method_request(method: &str, uri: &str, body: Vec<u8>) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .expect("request")
    }

    fn dotted(value: &str) -> DottedId {
        DottedId::new(value.to_owned()).expect("dotted id")
    }

    fn schema(value: &str) -> SchemaId {
        SchemaId::new(value.to_owned()).expect("schema id")
    }

    fn test_state_directory() -> PathBuf {
        std::env::temp_dir().join(format!(
            "rusty-fleet-local-hub-test-{}-{}",
            std::process::id(),
            STATE_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
