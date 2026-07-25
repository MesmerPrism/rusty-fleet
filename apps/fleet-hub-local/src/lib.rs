// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bounded local-network ingress over the deterministic Fleet Hub.
//!
//! The HTTP surface is an adapter. It does not own enrollment, signed
//! check-in, Manifold admission, or projection semantics.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path as FilePath, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::to_bytes;
use axum::extract::{Path as AxumPath, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use fleet_contracts::{
    CommandLifecycle, FleetQuery, OperationExecuteRequest, OperationPreviewRequest,
    SavedViewMutationRequest, SignedFleetCheckIn, ValidateContract,
};
use fleet_hub::{FleetApi, FleetHub, FleetHubSnapshot, HubPolicy, KioskShowControlsPreviewPlan};
use fleet_kiosk_adapter::{
    AdapterError, FleetKioskAdapter, HttpMethod, KioskAdapterLimits, KioskHttpRequest,
    KioskHttpResponse, KioskShowControlsRequest, KioskTransport, RawOwnerReceiptEvidence,
    TransportRequestIdSource, sha256_hex, validate_kiosk_endpoint,
};
use fleet_manifold_adapter::{
    FleetManifoldAdapter, FleetManifoldAdapterSnapshot, KioskShowControlsCommandAuthorization,
    kiosk_manifold_request_id,
};
use rusty_manifold_model::{DottedId, SchemaId};
use rusty_manifold_peer::{
    ManifoldPeerCredentialRecord, ManifoldPeerCredentialStatus, ManifoldPeerEnrollmentAction,
    ManifoldPeerEnrollmentRequest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
const MAX_OPERATION_BYTES: usize = 128 * 1024;
const MAX_CONCURRENT_REQUESTS: usize = 64;
const RATE_WINDOW_MS: i64 = 10_000;
const MAX_GLOBAL_CHECKINS_PER_WINDOW: usize = 4_096;
const MAX_CHECKINS_PER_CREDENTIAL_PER_WINDOW: usize = 8;
const BODY_DEADLINE: Duration = Duration::from_secs(5);
const OPERATION_PREVIEW_LIFETIME_MS: i64 = 60_000;
const DEFAULT_OPERATION_PARALLELISM: u16 = 8;
const DEFAULT_OPERATION_ATTEMPTS: u8 = 3;

static OPERATION_ID_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static TRANSPORT_ID_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static TRANSPORT_BOOT_NAMESPACE: OnceLock<String> = OnceLock::new();

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

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredKioskDirectOperator {
    pub device_id: String,
    pub endpoint: String,
    pub pairing_key: String,
}

impl std::fmt::Debug for ConfiguredKioskDirectOperator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfiguredKioskDirectOperator")
            .field("device_id", &self.device_id)
            .field("endpoint", &self.endpoint)
            .field("pairing_key", &"[redacted]")
            .finish()
    }
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
    pub kiosk_direct_operators: Vec<ConfiguredKioskDirectOperator>,
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
        let mut kiosk_devices = BTreeSet::new();
        for direct_operator in &self.kiosk_direct_operators {
            if direct_operator.device_id.is_empty()
                || direct_operator.device_id.len() > 256
                || direct_operator.pairing_key.is_empty()
                || !kiosk_devices.insert(direct_operator.device_id.clone())
            {
                return Err(
                    "Kiosk direct-operator entries require unique bounded device IDs and nonempty private keys"
                        .to_owned(),
                );
            }
            validate_kiosk_endpoint(&direct_operator.endpoint)
                .map_err(|error| format!("invalid Kiosk endpoint: {error}"))?;
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
    kiosk_direct_operators: BTreeMap<String, ConfiguredKioskDirectOperator>,
    owner_receipts: BTreeMap<String, RawOwnerReceiptEvidence>,
    inflight_kiosk_targets: BTreeSet<(String, String)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LocalHubDurableState {
    schema: String,
    generation: u64,
    written_at_ms: i64,
    hub: FleetHubSnapshot,
    adapter: FleetManifoldAdapterSnapshot,
    #[serde(default)]
    owner_receipts: BTreeMap<String, RawOwnerReceiptEvidence>,
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
        owner_receipts: &mut BTreeMap<String, RawOwnerReceiptEvidence>,
        now_ms: i64,
    ) -> Result<Self, String> {
        fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create state directory: {error}"))?;
        let candidates = read_state_candidates(directory)?;
        if !candidates.found_slot {
            return Ok(Self {
                directory: directory.to_path_buf(),
                generation: 0,
                restored: false,
            });
        }
        let mut failures = candidates.failures;
        for (slot, state) in candidates.states {
            match restore_state_candidate(hub.policy(), adapter, state, now_ms) {
                Ok((restored_hub, restored_adapter, restored_receipts, generation)) => {
                    *hub = restored_hub;
                    *adapter = restored_adapter;
                    *owner_receipts = restored_receipts;
                    return Ok(Self {
                        directory: directory.to_path_buf(),
                        generation,
                        restored: true,
                    });
                }
                Err(error) => failures.push(format!("slot {slot}: {error}")),
            }
        }
        Err(format!(
            "no fully valid durable state slot remains: {}",
            failures.join("; ")
        ))
    }

    fn persist(
        &mut self,
        hub: &FleetHub,
        adapter: &FleetManifoldAdapter,
        owner_receipts: &BTreeMap<String, RawOwnerReceiptEvidence>,
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
            owner_receipts: owner_receipts.clone(),
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
        sync_published_state(&self.directory, &target)?;
        self.generation = generation;
        self.restored = true;
        Ok(())
    }
}

#[cfg(unix)]
fn sync_published_state(directory: &FilePath, _target: &FilePath) -> Result<(), String> {
    fs::File::open(directory)
        .and_then(|directory_file| directory_file.sync_all())
        .map_err(|error| format!("cannot sync durable state directory metadata: {error}"))
}

#[cfg(windows)]
fn sync_published_state(_directory: &FilePath, target: &FilePath) -> Result<(), String> {
    OpenOptions::new()
        .write(true)
        .open(target)
        .and_then(|published_file| published_file.sync_all())
        .map_err(|error| format!("cannot flush published durable state metadata: {error}"))
}

#[cfg(not(any(unix, windows)))]
fn sync_published_state(_directory: &FilePath, target: &FilePath) -> Result<(), String> {
    fs::File::open(target)
        .and_then(|published_file| published_file.sync_all())
        .map_err(|error| format!("cannot flush published durable state: {error}"))
}

struct StateCandidates {
    found_slot: bool,
    states: Vec<(u64, LocalHubDurableState)>,
    failures: Vec<String>,
}

fn read_state_candidates(directory: &FilePath) -> Result<StateCandidates, String> {
    let mut states = Vec::new();
    let mut found_slot = false;
    let mut failures = Vec::new();
    for slot in 0..=1 {
        let path = state_slot_path(directory, slot);
        if !path.exists() {
            continue;
        }
        found_slot = true;
        match read_state_slot(&path, slot) {
            Ok(state) => states.push((slot, state)),
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    states.sort_by_key(|state| std::cmp::Reverse(state.1.generation));
    if states
        .windows(2)
        .any(|pair| pair[0].1.generation == pair[1].1.generation)
    {
        return Err("durable state slots contain an ambiguous duplicate generation".to_owned());
    }
    Ok(StateCandidates {
        found_slot,
        states,
        failures,
    })
}

fn restore_state_candidate(
    policy: HubPolicy,
    configured_adapter: &FleetManifoldAdapter,
    state: LocalHubDurableState,
    now_ms: i64,
) -> Result<
    (
        FleetHub,
        FleetManifoldAdapter,
        BTreeMap<String, RawOwnerReceiptEvidence>,
        u64,
    ),
    String,
> {
    let restored_hub = FleetHub::restore(policy, state.hub)
        .map_err(|error| format!("cannot restore Fleet Hub state: {error}"))?;
    let mut restored_adapter = configured_adapter.clone();
    restored_adapter
        .restore_session(state.adapter, now_ms)
        .map_err(|error| format!("cannot restore Manifold adapter state: {error}"))?;
    let hub_ids: BTreeSet<_> = restored_hub.device_ids().into_iter().collect();
    let accepted_ids: BTreeSet<_> = restored_adapter.accepted_peer_ids().into_iter().collect();
    if hub_ids != accepted_ids {
        return Err("Fleet Hub devices do not match accepted Manifold peers".to_owned());
    }
    for operation in restored_hub.kiosk_operations() {
        for target in operation.targets {
            for owner_request_id in target.owner_request_ids {
                if !restored_adapter.has_applied_kiosk_authorization(
                    &operation.operation_id,
                    &target.device_id,
                    &owner_request_id,
                ) {
                    return Err(format!(
                        "Kiosk operation {} target {} lacks applied Manifold authority for owner request {}",
                        operation.operation_id, target.device_id, owner_request_id
                    ));
                }
            }
        }
    }
    for operation in restored_hub.kiosk_operations() {
        for target in operation.targets {
            if let Some(receipt) = target.effective_receipt {
                let Some(raw) = state.owner_receipts.get(&receipt.receipt_id) else {
                    return Err(format!(
                        "Kiosk effective receipt {} lacks durable raw owner evidence",
                        receipt.receipt_id
                    ));
                };
                if raw.raw_receipt.is_empty()
                    || raw.raw_receipt.len() > 512 * 1024
                    || sha256_hex(&raw.raw_receipt) != raw.raw_receipt_sha256
                    || raw.raw_receipt_sha256 != receipt.response_content_sha256
                    || raw.host_received_at_ms != receipt.wrapped_at_ms
                    || raw.result_transport_request_id != receipt.owner_result_transport_request_id
                {
                    return Err(format!(
                        "Kiosk effective receipt {} does not match raw owner evidence",
                        receipt.receipt_id
                    ));
                }
            }
        }
    }
    Ok((
        restored_hub,
        restored_adapter,
        state.owner_receipts,
        state.generation,
    ))
}

fn read_state_slot(path: &FilePath, expected_slot: u64) -> Result<LocalHubDurableState, String> {
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
    if state.generation % 2 != expected_slot {
        return Err("state generation does not match its durable slot".to_owned());
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
        let mut owner_receipts = BTreeMap::new();
        let state_store = DurableStateStore::open(
            &config.state_directory,
            &mut hub,
            &mut adapter,
            &mut owner_receipts,
            now_ms,
        )?;
        let kiosk_direct_operators = config
            .kiosk_direct_operators
            .iter()
            .cloned()
            .map(|direct_operator| (direct_operator.device_id.clone(), direct_operator))
            .collect();
        Ok(Self {
            runtime: Arc::new(Mutex::new(RuntimeState {
                hub,
                adapter,
                rate_limiter: IngressRateLimiter::default(),
                state_store,
                kiosk_direct_operators,
                owner_receipts,
                inflight_kiosk_targets: BTreeSet::new(),
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

#[derive(Debug, Deserialize)]
struct StrictOperationPreviewRequest {
    schema: String,
    action_id: String,
    #[serde(deserialize_with = "deserialize_unique_targets")]
    targets: BTreeMap<String, u64>,
}

impl From<StrictOperationPreviewRequest> for OperationPreviewRequest {
    fn from(value: StrictOperationPreviewRequest) -> Self {
        Self {
            schema: value.schema,
            action_id: value.action_id,
            targets: value.targets,
        }
    }
}

#[derive(Clone)]
struct OwnerWork {
    request: KioskShowControlsRequest,
    pairing_key: String,
    deadline_at_ms: i64,
    recovery_only: bool,
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
        .route("/fleet/v1/operations/preview", post(preview_operation))
        .route(
            "/fleet/v1/operations/{operation_id}/execute",
            post(execute_operation),
        )
        .route("/fleet/v1/operations/{operation_id}", get(operation_status))
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
    schedule_recovered_owner_work(state.clone()).await?;
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
        owner_receipts,
        ..
    } = &mut *runtime;
    let mut candidate_hub = hub.clone();
    let mut candidate_adapter = adapter.clone();
    let receipt = candidate_adapter.accept(&mut candidate_hub, signed, now_ms);
    if receipt.accepted {
        if let Err(error) =
            state_store.persist(&candidate_hub, &candidate_adapter, owner_receipts, now_ms)
        {
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

async fn preview_operation(State(state): State<LocalHubState>, request: Request) -> Response {
    let bytes = match strict_json_body(request, MAX_OPERATION_BYTES, "operation previews").await {
        Ok(bytes) => bytes,
        Err(response) => return response,
    };
    let request = match serde_json::from_slice::<StrictOperationPreviewRequest>(&bytes) {
        Ok(value) => OperationPreviewRequest::from(value),
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_operation_preview_json",
                format!("operation preview is not valid strict JSON: {error}"),
            );
        }
    };
    if let Err(failures) = request.validate() {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_operation_preview",
            format_contract_failures(&failures),
        );
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let mut runtime = state.runtime.lock().await;
    if let Some(existing) = runtime
        .hub
        .kiosk_operations()
        .into_iter()
        .find(|operation| {
            operation.action_id == request.action_id
                && operation.preview.fleet_revision == runtime.hub.result_revision()
                && operation.preview.expires_at_ms >= now_ms
                && operation
                    .preview
                    .targets
                    .iter()
                    .map(|target| (target.device_id.clone(), target.identity_revision))
                    .collect::<BTreeMap<_, _>>()
                    == request.targets
        })
    {
        return Json(existing).into_response();
    }
    let expires_at_ms = match now_ms.checked_add(OPERATION_PREVIEW_LIFETIME_MS) {
        Some(value) => value,
        None => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "clock_error",
                "operation preview expiry overflowed",
            );
        }
    };
    let (operation_id, preview_id) = operation_ids(&request, now_ms);
    let mut candidate_hub = runtime.hub.clone();
    let operation = match candidate_hub.preview_kiosk_show_controls(KioskShowControlsPreviewPlan {
        operation_id,
        preview_id,
        request,
        created_at_ms: now_ms,
        expires_at_ms,
        max_parallelism: DEFAULT_OPERATION_PARALLELISM,
        max_attempts_per_target: DEFAULT_OPERATION_ATTEMPTS,
    }) {
        Ok(operation) => operation,
        Err(error) => return hub_operation_error(error),
    };
    let RuntimeState {
        hub,
        adapter,
        state_store,
        owner_receipts,
        ..
    } = &mut *runtime;
    if let Err(error) = state_store.persist(&candidate_hub, adapter, owner_receipts, now_ms) {
        return api_error(
            StatusCode::INSUFFICIENT_STORAGE,
            "durable_state_failed",
            error,
        );
    }
    *hub = candidate_hub;
    Json(operation).into_response()
}

async fn execute_operation(
    State(state): State<LocalHubState>,
    AxumPath(operation_id): AxumPath<String>,
    request: Request,
) -> Response {
    let bytes = match strict_json_body(request, MAX_OPERATION_BYTES, "operation execution").await {
        Ok(bytes) => bytes,
        Err(response) => return response,
    };
    let execute = match serde_json::from_slice::<OperationExecuteRequest>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_operation_execute_json",
                format!("operation execution is not valid JSON: {error}"),
            );
        }
    };
    if operation_id != execute.operation_id {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "operation_identity_mismatch",
            "operation path and payload identities must match",
        );
    }
    if let Err(failures) = execute.validate() {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_operation_execute",
            format_contract_failures(&failures),
        );
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let now_unsigned = match u64::try_from(now_ms) {
        Ok(value) => value,
        Err(_) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "clock_error",
                "current time must be nonnegative",
            );
        }
    };
    let mut runtime = state.runtime.lock().await;
    let existing = match runtime.hub.kiosk_operation(&operation_id) {
        Ok(operation) => operation,
        Err(error) => return hub_operation_error(error),
    };
    if existing.preview.preview_id != execute.preview_id {
        return api_error(
            StatusCode::CONFLICT,
            "operation_preview_conflict",
            "execute request does not bind the stored immutable preview",
        );
    }
    let eligible_devices = existing
        .targets
        .iter()
        .filter(|target| target.preflight.eligible)
        .map(|target| target.device_id.clone())
        .collect::<Vec<_>>();
    if eligible_devices.is_empty() {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "operation_has_no_eligible_targets",
            "the immutable preview contains no eligible Kiosk targets",
        );
    }
    if existing.lifecycle != CommandLifecycle::Proposed {
        return Json(existing).into_response();
    }
    let mut direct_operators = BTreeMap::new();
    for device_id in &eligible_devices {
        let Some(direct_operator) = runtime.kiosk_direct_operators.get(device_id).cloned() else {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "kiosk_direct_operator_unconfigured",
                format!("Kiosk direct operator is not privately configured for {device_id}"),
            );
        };
        direct_operators.insert(device_id.clone(), direct_operator);
    }

    let mut candidate_hub = runtime.hub.clone();
    let mut candidate_adapter = runtime.adapter.clone();
    let mut operation = match candidate_hub.confirm_kiosk_show_controls_request(&execute, now_ms) {
        Ok(operation) => operation,
        Err(error) => return hub_operation_error(error),
    };
    let expires_unsigned = match u64::try_from(operation.preview.expires_at_ms) {
        Ok(value) => value,
        Err(_) => {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "operation_expiry_invalid",
                "operation expiry is not representable",
            );
        }
    };
    let mut work = Vec::new();
    let dispatch_devices = eligible_devices
        .into_iter()
        .take(usize::from(operation.max_parallelism))
        .collect::<Vec<_>>();
    for device_id in dispatch_devices {
        let Some(target) = operation
            .targets
            .iter()
            .find(|target| target.device_id == device_id)
        else {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "operation_target_missing",
                "confirmed operation omitted a frozen target",
            );
        };
        let identity_revision = target.identity_revision;
        let owner_action_request_id =
            owner_action_request_id(&operation.operation_id, &device_id, 1);
        let manifold_request_id = kiosk_manifold_request_id(
            &operation.operation_id,
            &device_id,
            &owner_action_request_id,
        );
        if let Err(error) = candidate_adapter.authorize_kiosk_show_controls(
            &KioskShowControlsCommandAuthorization {
                manifold_request_id,
                owner_action_request_id: owner_action_request_id.clone(),
                requester_id: "operator.fleet.local".to_owned(),
                operation_id: operation.operation_id.clone(),
                preview_id: operation.preview.preview_id.clone(),
                device_id: device_id.clone(),
                identity_revision,
                issued_at_ms: now_unsigned,
                expires_at_ms: expires_unsigned,
            },
            now_unsigned,
        ) {
            return api_error(StatusCode::CONFLICT, "manifold_command_rejected", error);
        }
        operation = match candidate_hub.dispatch_kiosk_show_controls(
            &operation.operation_id,
            &device_id,
            owner_action_request_id.clone(),
            now_ms,
        ) {
            Ok(operation) => operation,
            Err(error) => return hub_operation_error(error),
        };
        let direct_operator = direct_operators
            .remove(&device_id)
            .expect("validated Kiosk direct-operator configuration");
        work.push(OwnerWork {
            request: KioskShowControlsRequest {
                receipt_id: receipt_id(&operation.operation_id, &device_id, 1),
                operation_id: operation.operation_id.clone(),
                device_id: device_id.clone(),
                identity_revision,
                endpoint: direct_operator.endpoint,
                owner_action_request_id,
            },
            pairing_key: direct_operator.pairing_key,
            deadline_at_ms: operation.preview.expires_at_ms,
            recovery_only: false,
        });
    }
    let RuntimeState {
        hub,
        adapter,
        state_store,
        owner_receipts,
        inflight_kiosk_targets,
        ..
    } = &mut *runtime;
    if let Err(error) =
        state_store.persist(&candidate_hub, &candidate_adapter, owner_receipts, now_ms)
    {
        return api_error(
            StatusCode::INSUFFICIENT_STORAGE,
            "durable_state_failed",
            error,
        );
    }
    *hub = candidate_hub;
    *adapter = candidate_adapter;
    for owner_work in &work {
        inflight_kiosk_targets.insert((
            owner_work.request.operation_id.clone(),
            owner_work.request.device_id.clone(),
        ));
    }
    drop(runtime);
    for owner_work in work {
        spawn_owner_work(state.clone(), owner_work);
    }
    Json(operation).into_response()
}

async fn operation_status(
    State(state): State<LocalHubState>,
    AxumPath(operation_id): AxumPath<String>,
) -> Response {
    let runtime = state.runtime.lock().await;
    match runtime.hub.kiosk_operation(&operation_id) {
        Ok(operation) => Json(operation).into_response(),
        Err(error) => hub_operation_error(error),
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
        owner_receipts,
        ..
    } = &mut *runtime;
    let mut candidate_hub = hub.clone();
    let receipt = match candidate_hub.upsert_saved_view(mutation) {
        Ok(receipt) => receipt,
        Err(error) => return saved_view_error(error),
    };
    if receipt.changed {
        if let Err(error) = state_store.persist(&candidate_hub, adapter, owner_receipts, now_ms) {
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
        owner_receipts,
        ..
    } = &mut *runtime;
    let mut candidate_hub = hub.clone();
    let receipt = match candidate_hub.delete_saved_view(&view_id, query.expected_revision) {
        Ok(receipt) => receipt,
        Err(error) => return saved_view_error(error),
    };
    if let Err(error) = state_store.persist(&candidate_hub, adapter, owner_receipts, now_ms) {
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

async fn strict_json_body(
    request: Request,
    limit: usize,
    purpose: &'static str,
) -> Result<axum::body::Bytes, Response> {
    if !is_json(request.headers()) {
        return Err(api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content_type_required",
            format!("{purpose} require Content-Type: application/json"),
        ));
    }
    let declared_length = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| {
            api_error(
                StatusCode::LENGTH_REQUIRED,
                "content_length_required",
                format!("{purpose} require one valid Content-Length"),
            )
        })?;
    if declared_length == 0 || declared_length > limit {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "body_limit_exceeded",
            format!("{purpose} require 1 through {limit} bytes"),
        ));
    }
    let bytes = bounded_body(request, limit).await?;
    if bytes.len() != declared_length {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "content_length_mismatch",
            format!("{purpose} body length differs from Content-Length"),
        ));
    }
    Ok(bytes)
}

fn deserialize_unique_targets<'de, D>(deserializer: D) -> Result<BTreeMap<String, u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct UniqueTargetsVisitor;

    impl<'de> serde::de::Visitor<'de> for UniqueTargetsVisitor {
        type Value = BTreeMap<String, u64>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a JSON object with unique target device IDs")
        }

        fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut targets = BTreeMap::new();
            while let Some((device_id, identity_revision)) = access.next_entry::<String, u64>()? {
                if targets
                    .insert(device_id.clone(), identity_revision)
                    .is_some()
                {
                    return Err(serde::de::Error::custom(format!(
                        "duplicate target device ID {device_id}"
                    )));
                }
            }
            Ok(targets)
        }
    }

    deserializer.deserialize_map(UniqueTargetsVisitor)
}

fn operation_ids(request: &OperationPreviewRequest, now_ms: i64) -> (String, String) {
    let sequence = OPERATION_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.kiosk.operation.v1\0");
    digest.update(serde_json::to_vec(request).unwrap_or_default());
    digest.update(now_ms.to_le_bytes());
    digest.update(sequence.to_le_bytes());
    let suffix = hex::encode(digest.finalize());
    (
        format!("kiosk-operation-{}", &suffix[..32]),
        format!("kiosk-preview-{}", &suffix[32..64]),
    )
}

fn owner_action_request_id(operation_id: &str, device_id: &str, attempt: u8) -> String {
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.kiosk.owner-action.v1\0");
    digest.update(operation_id.as_bytes());
    digest.update([0]);
    digest.update(device_id.as_bytes());
    digest.update([0, attempt]);
    format!("fleetact-{}", &hex::encode(digest.finalize())[..32])
}

fn receipt_id(operation_id: &str, device_id: &str, attempt: u8) -> String {
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.kiosk.receipt.v1\0");
    digest.update(operation_id.as_bytes());
    digest.update([0]);
    digest.update(device_id.as_bytes());
    digest.update([0, attempt]);
    format!("kiosk-receipt-{}", &hex::encode(digest.finalize())[..32])
}

fn format_contract_failures(failures: &[fleet_contracts::ContractViolation]) -> String {
    failures
        .iter()
        .map(|failure| format!("{}:{}:{}", failure.code, failure.path, failure.message))
        .collect::<Vec<_>>()
        .join("; ")
}

fn hub_operation_error(error: fleet_hub::HubError) -> Response {
    let (status, code) = match error.code.as_str() {
        "kiosk_operation_not_found" | "kiosk_target_not_found" => {
            (StatusCode::NOT_FOUND, "operation_not_found")
        }
        "kiosk_preview_mismatch"
        | "kiosk_operation_id_conflict"
        | "kiosk_preview_expired"
        | "kiosk_target_changed_since_preview"
        | "kiosk_target_identity_changed" => (StatusCode::CONFLICT, "operation_conflict"),
        _ => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_operation"),
    };
    api_error(status, code, error.to_string())
}

fn spawn_owner_work(state: LocalHubState, work: OwnerWork) {
    tokio::spawn(async move {
        let operation_id = work.request.operation_id.clone();
        let device_id = work.request.device_id.clone();
        let result = tokio::task::spawn_blocking(move || run_owner_work(work))
            .await
            .map_err(|error| format!("Kiosk worker could not join: {error}"))
            .and_then(|result| result.map_err(|error| error.to_string()));
        let mut persistence_retry = Duration::from_millis(100);
        loop {
            let now_ms = match unix_time_ms() {
                Ok(value) => value,
                Err(_) => {
                    tokio::time::sleep(persistence_retry).await;
                    persistence_retry = (persistence_retry * 2).min(Duration::from_secs(5));
                    continue;
                }
            };
            let mut runtime = state.runtime.lock().await;
            let current = match runtime.hub.kiosk_operation(&operation_id) {
                Ok(operation) => operation,
                Err(_) => {
                    runtime
                        .inflight_kiosk_targets
                        .remove(&(operation_id.clone(), device_id.clone()));
                    return;
                }
            };
            let Some(target) = current
                .targets
                .iter()
                .find(|target| target.device_id == device_id)
            else {
                runtime
                    .inflight_kiosk_targets
                    .remove(&(operation_id.clone(), device_id.clone()));
                return;
            };
            if matches!(
                target.lifecycle,
                CommandLifecycle::Applied
                    | CommandLifecycle::Failed
                    | CommandLifecycle::Expired
                    | CommandLifecycle::Cancelled
                    | CommandLifecycle::Rejected
            ) {
                runtime
                    .inflight_kiosk_targets
                    .remove(&(operation_id.clone(), device_id.clone()));
                return;
            }
            let mut candidate_hub = runtime.hub.clone();
            let mut candidate_receipts = runtime.owner_receipts.clone();
            let transition = match &result {
                Ok(outcome) => {
                    let receipt_id = outcome.effective_receipt.receipt_id.clone();
                    candidate_receipts.insert(receipt_id, outcome.owner_receipt.clone());
                    candidate_hub.apply_kiosk_show_controls_receipt(
                        outcome.effective_receipt.clone(),
                        now_ms,
                    )
                }
                Err(error) => candidate_hub.expire_kiosk_show_controls(
                    &operation_id,
                    &device_id,
                    "owner_outcome_unknown",
                    &format!(
                        "Kiosk owner outcome remained unknown at the durable deadline: {error}"
                    ),
                    now_ms,
                ),
            };
            if transition.is_err() {
                drop(runtime);
                tokio::time::sleep(persistence_retry).await;
                persistence_retry = (persistence_retry * 2).min(Duration::from_secs(5));
                continue;
            }
            let RuntimeState {
                hub,
                adapter,
                state_store,
                owner_receipts,
                inflight_kiosk_targets,
                ..
            } = &mut *runtime;
            if state_store
                .persist(&candidate_hub, adapter, &candidate_receipts, now_ms)
                .is_ok()
            {
                *hub = candidate_hub;
                *owner_receipts = candidate_receipts;
                inflight_kiosk_targets.remove(&(operation_id.clone(), device_id.clone()));
                drop(runtime);
                schedule_pending_owner_work(state.clone(), &operation_id).await;
                return;
            }
            drop(runtime);
            tokio::time::sleep(persistence_retry).await;
            persistence_retry = (persistence_retry * 2).min(Duration::from_secs(5));
        }
    });
}

fn run_owner_work(
    work: OwnerWork,
) -> Result<fleet_kiosk_adapter::KioskShowControlsPollOutcome, AdapterError> {
    let now_ms = unix_time_ms().map_err(AdapterError::Transport)?;
    let remaining_ms = work
        .deadline_at_ms
        .checked_sub(now_ms)
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0 && *value <= 90_000)
        .ok_or(AdapterError::DeadlineExceeded)?;
    let adapter = FleetKioskAdapter::new(KioskAdapterLimits {
        maximum_polls: 64,
        poll_interval_ms: 1_000,
        operation_deadline_ms: remaining_ms,
        request_timeout_ms: remaining_ms.min(5_000),
        maximum_response_bytes: 512 * 1024,
    })?;
    let mut transport = StdKioskTransport;
    let mut request_ids = AtomicTransportRequestIds;
    if !work.recovery_only {
        let _ambiguous_or_acknowledged = adapter.invoke_show_controls(
            &work.request,
            &work.pairing_key,
            &mut transport,
            &mut request_ids,
        );
    }
    adapter.resume_show_controls_result(
        &work.request,
        &work.pairing_key,
        work.deadline_at_ms,
        &mut transport,
        &mut request_ids,
    )
}

async fn schedule_pending_owner_work(state: LocalHubState, operation_id: &str) {
    let mut persistence_retry = Duration::from_millis(100);
    loop {
        let now_ms = match unix_time_ms() {
            Ok(value) => value,
            Err(_) => return,
        };
        let now_unsigned = match u64::try_from(now_ms) {
            Ok(value) => value,
            Err(_) => return,
        };
        let mut runtime = state.runtime.lock().await;
        let operation = match runtime.hub.kiosk_operation(operation_id) {
            Ok(operation) => operation,
            Err(_) => return,
        };
        let pending = operation
            .targets
            .iter()
            .filter(|target| target.lifecycle == CommandLifecycle::Accepted)
            .map(|target| (target.device_id.clone(), target.identity_revision))
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return;
        }
        let mut candidate_hub = runtime.hub.clone();
        let mut candidate_adapter = runtime.adapter.clone();
        if now_ms >= operation.preview.expires_at_ms {
            for (device_id, _) in pending {
                if candidate_hub
                    .request_kiosk_show_controls_cancellation(operation_id, &device_id, now_ms)
                    .and_then(|_| {
                        candidate_hub.complete_kiosk_show_controls_cancellation(
                            operation_id,
                            &device_id,
                            now_ms,
                        )
                    })
                    .is_err()
                {
                    return;
                }
            }
            let RuntimeState {
                hub,
                adapter,
                state_store,
                owner_receipts,
                ..
            } = &mut *runtime;
            if state_store
                .persist(&candidate_hub, adapter, owner_receipts, now_ms)
                .is_ok()
            {
                *hub = candidate_hub;
                return;
            }
            drop(runtime);
            tokio::time::sleep(persistence_retry).await;
            persistence_retry = (persistence_retry * 2).min(Duration::from_secs(5));
            continue;
        }
        let inflight = operation
            .targets
            .iter()
            .filter(|target| {
                matches!(
                    target.lifecycle,
                    CommandLifecycle::Dispatched | CommandLifecycle::Running
                )
            })
            .count();
        let slots = usize::from(operation.max_parallelism).saturating_sub(inflight);
        if slots == 0 {
            return;
        }
        let expires_unsigned = match u64::try_from(operation.preview.expires_at_ms) {
            Ok(value) => value,
            Err(_) => return,
        };
        let mut work = Vec::new();
        for (device_id, identity_revision) in pending.into_iter().take(slots) {
            let Some(direct_operator) = runtime.kiosk_direct_operators.get(&device_id).cloned()
            else {
                return;
            };
            let owner_action_request_id = owner_action_request_id(operation_id, &device_id, 1);
            let manifold_request_id =
                kiosk_manifold_request_id(operation_id, &device_id, &owner_action_request_id);
            if candidate_adapter
                .authorize_kiosk_show_controls(
                    &KioskShowControlsCommandAuthorization {
                        manifold_request_id,
                        owner_action_request_id: owner_action_request_id.clone(),
                        requester_id: "operator.fleet.local".to_owned(),
                        operation_id: operation_id.to_owned(),
                        preview_id: operation.preview.preview_id.clone(),
                        device_id: device_id.clone(),
                        identity_revision,
                        issued_at_ms: now_unsigned,
                        expires_at_ms: expires_unsigned,
                    },
                    now_unsigned,
                )
                .is_err()
            {
                return;
            }
            let updated = match candidate_hub.dispatch_kiosk_show_controls(
                operation_id,
                &device_id,
                owner_action_request_id.clone(),
                now_ms,
            ) {
                Ok(updated) => updated,
                Err(_) => return,
            };
            work.push(OwnerWork {
                request: KioskShowControlsRequest {
                    receipt_id: receipt_id(operation_id, &device_id, 1),
                    operation_id: operation_id.to_owned(),
                    device_id: device_id.clone(),
                    identity_revision,
                    endpoint: direct_operator.endpoint,
                    owner_action_request_id,
                },
                pairing_key: direct_operator.pairing_key,
                deadline_at_ms: updated.preview.expires_at_ms,
                recovery_only: false,
            });
        }
        let RuntimeState {
            hub,
            adapter,
            state_store,
            owner_receipts,
            inflight_kiosk_targets,
            ..
        } = &mut *runtime;
        if state_store
            .persist(&candidate_hub, &candidate_adapter, owner_receipts, now_ms)
            .is_err()
        {
            drop(runtime);
            tokio::time::sleep(persistence_retry).await;
            persistence_retry = (persistence_retry * 2).min(Duration::from_secs(5));
            continue;
        }
        *hub = candidate_hub;
        *adapter = candidate_adapter;
        for owner_work in &work {
            inflight_kiosk_targets.insert((
                owner_work.request.operation_id.clone(),
                owner_work.request.device_id.clone(),
            ));
        }
        drop(runtime);
        for owner_work in work {
            spawn_owner_work(state.clone(), owner_work);
        }
        return;
    }
}

async fn schedule_recovered_owner_work(state: LocalHubState) -> Result<(), String> {
    let mut runtime = state.runtime.lock().await;
    let mut work = Vec::new();
    let operations = runtime.hub.kiosk_operations();
    for operation in &operations {
        for target in operation
            .targets
            .iter()
            .filter(|target| target.lifecycle == CommandLifecycle::Accepted)
        {
            if !runtime
                .kiosk_direct_operators
                .contains_key(&target.device_id)
            {
                return Err(format!(
                    "recovered accepted Kiosk target {} lacks private direct-operator configuration",
                    target.device_id
                ));
            }
        }
    }
    let pending_operation_ids = operations
        .iter()
        .filter(|operation| {
            operation
                .targets
                .iter()
                .any(|target| target.lifecycle == CommandLifecycle::Accepted)
        })
        .map(|operation| operation.operation_id.clone())
        .collect::<BTreeSet<_>>();
    for operation in operations {
        for target in operation.targets.iter().filter(|target| {
            matches!(
                target.lifecycle,
                CommandLifecycle::Dispatched | CommandLifecycle::Running
            )
        }) {
            let direct_operator = runtime
                .kiosk_direct_operators
                .get(&target.device_id)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "recovered Kiosk target {} lacks private direct-operator configuration",
                        target.device_id
                    )
                })?;
            let owner_action_request_id = target.owner_request_id.clone().ok_or_else(|| {
                format!(
                    "recovered Kiosk target {} lacks its stable owner action request ID",
                    target.device_id
                )
            })?;
            let deadline_at_ms = target.owner_deadline_at_ms.ok_or_else(|| {
                format!(
                    "recovered Kiosk target {} lacks its durable owner deadline",
                    target.device_id
                )
            })?;
            let key = (operation.operation_id.clone(), target.device_id.clone());
            if !runtime.inflight_kiosk_targets.insert(key) {
                continue;
            }
            work.push(OwnerWork {
                request: KioskShowControlsRequest {
                    receipt_id: receipt_id(
                        &operation.operation_id,
                        &target.device_id,
                        target.attempt_count,
                    ),
                    operation_id: operation.operation_id.clone(),
                    device_id: target.device_id.clone(),
                    identity_revision: target.identity_revision,
                    endpoint: direct_operator.endpoint,
                    owner_action_request_id,
                },
                pairing_key: direct_operator.pairing_key,
                deadline_at_ms,
                recovery_only: true,
            });
        }
    }
    drop(runtime);
    for owner_work in work {
        spawn_owner_work(state.clone(), owner_work);
    }
    for operation_id in pending_operation_ids {
        schedule_pending_owner_work(state.clone(), &operation_id).await;
    }
    Ok(())
}

struct AtomicTransportRequestIds;

impl TransportRequestIdSource for AtomicTransportRequestIds {
    fn next_transport_request_id(&mut self) -> Result<String, String> {
        let sequence = TRANSPORT_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Ok(transport_request_id(transport_boot_namespace(), sequence))
    }
}

fn transport_boot_namespace() -> &'static str {
    TRANSPORT_BOOT_NAMESPACE
        .get_or_init(|| {
            let elapsed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let mut digest = Sha256::new();
            digest.update(b"rusty.fleet.kiosk.transport-boot.v1\0");
            digest.update(std::process::id().to_le_bytes());
            digest.update(elapsed.as_nanos().to_le_bytes());
            hex::encode(digest.finalize())[..16].to_owned()
        })
        .as_str()
}

fn transport_request_id(namespace: &str, sequence: u64) -> String {
    format!("fleettrans-{namespace}-{sequence:016x}")
}

struct StdKioskTransport;

impl KioskTransport for StdKioskTransport {
    fn now_ms(&self) -> i64 {
        unix_time_ms().unwrap_or(i64::MAX)
    }

    fn wait_until_ms(&mut self, not_before_ms: i64, deadline_at_ms: i64) -> Result<(), String> {
        let now_ms = unix_time_ms()?;
        if now_ms >= deadline_at_ms {
            return Err("Kiosk owner deadline was reached".to_owned());
        }
        let wait_ms = not_before_ms
            .saturating_sub(now_ms)
            .min(deadline_at_ms.saturating_sub(now_ms));
        if wait_ms > 0 {
            std::thread::sleep(Duration::from_millis(
                u64::try_from(wait_ms).map_err(|_| "invalid wait duration".to_owned())?,
            ));
        }
        Ok(())
    }

    fn send(&mut self, request: KioskHttpRequest) -> Result<KioskHttpResponse, String> {
        send_kiosk_http(request)
    }
}

fn send_kiosk_http(request: KioskHttpRequest) -> Result<KioskHttpResponse, String> {
    let endpoint = request
        .url
        .strip_suffix(&request.request_target)
        .ok_or_else(|| "Kiosk request URL and target do not match".to_owned())?;
    validate_kiosk_endpoint(endpoint).map_err(|error| error.to_string())?;
    let authority = endpoint
        .strip_prefix("http://")
        .ok_or_else(|| "Kiosk endpoint scheme is invalid".to_owned())?;
    let socket = authority
        .parse::<SocketAddr>()
        .map_err(|error| format!("Kiosk endpoint must use a literal IP address: {error}"))?;
    let now_ms = unix_time_ms()?;
    let remaining_ms = request.deadline_at_ms.saturating_sub(now_ms);
    if remaining_ms <= 0 {
        return Err("Kiosk request deadline was reached".to_owned());
    }
    let timeout_ms = request
        .timeout_ms
        .min(u64::try_from(remaining_ms).map_err(|_| "invalid deadline".to_owned())?);
    let timeout_duration = Duration::from_millis(timeout_ms);
    let mut stream = TcpStream::connect_timeout(&socket, timeout_duration)
        .map_err(|error| format!("cannot connect to Kiosk owner: {error}"))?;
    stream
        .set_read_timeout(Some(timeout_duration))
        .map_err(|error| format!("cannot bound Kiosk read: {error}"))?;
    stream
        .set_write_timeout(Some(timeout_duration))
        .map_err(|error| format!("cannot bound Kiosk write: {error}"))?;
    let mut wire = format!(
        "{} {} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nContent-Length: {}\r\n",
        request.method.as_str(),
        request.request_target,
        request.body.len()
    )
    .into_bytes();
    if request.method == HttpMethod::Post {
        wire.extend_from_slice(b"Content-Type: application/json\r\n");
    }
    for (name, value) in &request.headers {
        if name.contains(['\r', '\n', ':']) || value.contains(['\r', '\n']) {
            return Err("Kiosk request header is invalid".to_owned());
        }
        wire.extend_from_slice(name.as_bytes());
        wire.extend_from_slice(b": ");
        wire.extend_from_slice(value.as_bytes());
        wire.extend_from_slice(b"\r\n");
    }
    wire.extend_from_slice(b"\r\n");
    wire.extend_from_slice(&request.body);
    let mut written = 0;
    while written < wire.len() {
        let remaining = remaining_transport_timeout(request.deadline_at_ms, request.timeout_ms)?;
        stream
            .set_write_timeout(Some(remaining))
            .map_err(|error| format!("cannot refresh Kiosk write bound: {error}"))?;
        let count = stream
            .write(&wire[written..])
            .map_err(|error| format!("cannot write Kiosk request: {error}"))?;
        if count == 0 {
            return Err("Kiosk connection closed while writing the request".to_owned());
        }
        written = written.saturating_add(count);
    }
    stream
        .set_write_timeout(Some(remaining_transport_timeout(
            request.deadline_at_ms,
            request.timeout_ms,
        )?))
        .map_err(|error| format!("cannot refresh Kiosk flush bound: {error}"))?;
    stream
        .flush()
        .map_err(|error| format!("cannot flush Kiosk request: {error}"))?;

    let maximum_wire_bytes = request.maximum_response_bytes.saturating_add(32 * 1024);
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        stream
            .set_read_timeout(Some(remaining_transport_timeout(
                request.deadline_at_ms,
                request.timeout_ms,
            )?))
            .map_err(|error| format!("cannot refresh Kiosk read bound: {error}"))?;
        let count = stream
            .read(&mut chunk)
            .map_err(|error| format!("cannot read Kiosk response: {error}"))?;
        if count == 0 {
            break;
        }
        if response.len().saturating_add(count) > maximum_wire_bytes {
            return Err("Kiosk response exceeded its bounded wire size".to_owned());
        }
        response.extend_from_slice(&chunk[..count]);
        if let Some(expected_length) =
            expected_http_response_length(&response, request.maximum_response_bytes)?
        {
            if response.len() > expected_length {
                return Err("Kiosk response exceeded its declared Content-Length".to_owned());
            }
            if response.len() == expected_length {
                break;
            }
        }
    }
    parse_kiosk_http_response(&response, request.maximum_response_bytes)
}

fn remaining_transport_timeout(
    deadline_at_ms: i64,
    per_io_timeout_ms: u64,
) -> Result<Duration, String> {
    let remaining_ms = deadline_at_ms.saturating_sub(unix_time_ms()?);
    if remaining_ms <= 0 {
        return Err("Kiosk absolute request deadline was reached".to_owned());
    }
    Ok(Duration::from_millis(
        u64::try_from(remaining_ms)
            .map_err(|_| "Kiosk deadline is not representable".to_owned())?
            .min(per_io_timeout_ms),
    ))
}

fn expected_http_response_length(
    response: &[u8],
    maximum_body_bytes: usize,
) -> Result<Option<usize>, String> {
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        if response.len() > 32 * 1024 {
            return Err("Kiosk response header exceeded 32 KiB".to_owned());
        }
        return Ok(None);
    };
    let header_text = std::str::from_utf8(&response[..header_end])
        .map_err(|_| "Kiosk response header is not UTF-8".to_owned())?;
    let mut content_length = None;
    for line in header_text.split("\r\n").skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            return Err("Kiosk response header line is malformed".to_owned());
        };
        if name.trim().eq_ignore_ascii_case("transfer-encoding") {
            return Err("Kiosk chunked responses are not accepted".to_owned());
        }
        if name.trim().eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err("Kiosk response repeats Content-Length".to_owned());
            }
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| "Kiosk response Content-Length is invalid".to_owned())?;
            if parsed > maximum_body_bytes {
                return Err("Kiosk response body exceeds its limit".to_owned());
            }
            content_length = Some(parsed);
        }
    }
    let content_length =
        content_length.ok_or_else(|| "Kiosk response omitted Content-Length".to_owned())?;
    Ok(Some(
        header_end
            .checked_add(4)
            .and_then(|length| length.checked_add(content_length))
            .ok_or_else(|| "Kiosk response length overflowed".to_owned())?,
    ))
}

fn parse_kiosk_http_response(
    response: &[u8],
    maximum_body_bytes: usize,
) -> Result<KioskHttpResponse, String> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "Kiosk response omitted a complete HTTP header".to_owned())?;
    if header_end > 32 * 1024 {
        return Err("Kiosk response header exceeded 32 KiB".to_owned());
    }
    let header_text = std::str::from_utf8(&response[..header_end])
        .map_err(|_| "Kiosk response header is not UTF-8".to_owned())?;
    let mut lines = header_text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| "Kiosk response omitted its status line".to_owned())?;
    let mut status_parts = status_line.split_whitespace();
    let protocol = status_parts.next().unwrap_or_default();
    let status = status_parts
        .next()
        .ok_or_else(|| "Kiosk response omitted its status code".to_owned())?
        .parse::<u16>()
        .map_err(|_| "Kiosk response status code is invalid".to_owned())?;
    if protocol != "HTTP/1.1" && protocol != "HTTP/1.0" {
        return Err("Kiosk response protocol is unsupported".to_owned());
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| "Kiosk response header line is malformed".to_owned())?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();
        if name.is_empty() || headers.insert(name, value).is_some() {
            return Err("Kiosk response contains an empty or duplicate header".to_owned());
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err("Kiosk chunked responses are not accepted".to_owned());
    }
    let content_length = headers
        .get("content-length")
        .ok_or_else(|| "Kiosk response omitted Content-Length".to_owned())?
        .parse::<usize>()
        .map_err(|_| "Kiosk response Content-Length is invalid".to_owned())?;
    if content_length > maximum_body_bytes {
        return Err("Kiosk response body exceeds its limit".to_owned());
    }
    let body_start = header_end + 4;
    if response.len() != body_start.saturating_add(content_length) {
        return Err("Kiosk response body differs from Content-Length".to_owned());
    }
    Ok(KioskHttpResponse {
        status,
        headers,
        body: response[body_start..].to_vec(),
        redirected: false,
        received_at_ms: unix_time_ms()?,
    })
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
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use ed25519_dalek::{Signer, SigningKey};
    use fleet_contracts::{
        AuthorizationState, CHECKIN_SIGNATURE_ALGORITHM, CapabilityState, CommandLifecycle,
        EnablementState, FleetCheckInClaims, FleetQuery, FreshnessState, NavigationRestoration,
        ReachabilityState, SavedView, SavedViewMutationRequest, SignedFleetCheckIn, SupportState,
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
        ConfiguredEnrollment, ConfiguredKioskDirectOperator, IngressRateLimiter, LocalHubConfig,
        LocalHubState, MAX_CHECKINS_PER_CREDENTIAL_PER_WINDOW, RuntimeState, router,
        schedule_recovered_owner_work, state_slot_path, transport_request_id, unix_time_ms,
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

    #[tokio::test]
    async fn nested_invalid_newest_state_falls_back_to_fully_valid_slot() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[8_u8; 32]);
        let (config, key_id) = config(&signing_key, now_ms);
        let state_directory = config.state_directory.clone();
        let app = router(LocalHubState::from_config(&config, now_ms).expect("valid config"));
        for revision in 1..=2 {
            let signed = signed_checkin(
                &signing_key,
                key_id.as_str(),
                now_ms + i64::try_from(revision).expect("small revision"),
                revision,
            );
            let response = app
                .clone()
                .oneshot(json_request(
                    "/fleet/v1/checkins",
                    serde_json::to_vec(&signed).expect("signed JSON"),
                ))
                .await
                .expect("check-in response");
            assert_eq!(response.status(), StatusCode::OK);
        }
        drop(app);

        let newest_path = state_slot_path(&state_directory, 0);
        let mut newest: Value =
            serde_json::from_slice(&fs::read(&newest_path).expect("newest state"))
                .expect("newest state JSON");
        newest["hub"]["result_revision"] = serde_json::json!(0);
        fs::write(
            &newest_path,
            serde_json::to_vec(&newest).expect("damaged nested state JSON"),
        )
        .expect("write wrapper-valid nested-invalid state");

        let restored =
            LocalHubState::from_config(&config, now_ms + 20).expect("older full state restored");
        let runtime = restored.runtime.lock().await;
        assert_eq!(runtime.state_store.generation, 1);
        assert_eq!(runtime.hub.device_count(), 1);
        drop(runtime);
        fs::remove_dir_all(state_directory).expect("remove test state directory");
    }

    #[tokio::test]
    async fn operation_routes_are_strict_idempotent_and_persist_before_dispatch() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
        let (mut config, key_id) = config(&signing_key, now_ms);
        config.kiosk_direct_operators = vec![ConfiguredKioskDirectOperator {
            device_id: "device.quest.1".to_owned(),
            endpoint: "http://127.0.0.1:39873".to_owned(),
            pairing_key: "test-pairing-key".to_owned(),
        }];
        let state_directory = config.state_directory.clone();
        let state = LocalHubState::from_config(&config, now_ms).expect("valid config");
        let app = router(state.clone());
        let signed = signed_checkin(&signing_key, key_id.as_str(), now_ms, 1);
        let identity_revision = signed.claims.observation.identity.identity_revision;
        let accepted = app
            .clone()
            .oneshot(json_request(
                "/fleet/v1/checkins",
                serde_json::to_vec(&signed).expect("signed JSON"),
            ))
            .await
            .expect("check-in response");
        assert_eq!(accepted.status(), StatusCode::OK);

        let preview_request = serde_json::json!({
            "schema": "rusty.fleet.operation_preview_request.v1",
            "action_id": "kiosk.show-controls",
            "targets": {"device.quest.1": identity_revision}
        });
        let preview_bytes = serde_json::to_vec(&preview_request).expect("preview JSON");
        let first = app
            .clone()
            .oneshot(json_request(
                "/fleet/v1/operations/preview",
                preview_bytes.clone(),
            ))
            .await
            .expect("preview response");
        assert_eq!(first.status(), StatusCode::OK);
        let first_json: Value = serde_json::from_slice(
            &to_bytes(first.into_body(), 256 * 1024)
                .await
                .expect("preview body"),
        )
        .expect("preview response JSON");
        assert_eq!(first_json["lifecycle"], "proposed");
        assert_eq!(first_json["targets"][0]["preflight"]["eligible"], true);

        let repeated = app
            .clone()
            .oneshot(json_request("/fleet/v1/operations/preview", preview_bytes))
            .await
            .expect("repeated preview response");
        let repeated_json: Value = serde_json::from_slice(
            &to_bytes(repeated.into_body(), 256 * 1024)
                .await
                .expect("repeated preview body"),
        )
        .expect("repeated preview JSON");
        assert_eq!(repeated_json, first_json);

        let duplicate = br#"{"schema":"rusty.fleet.operation_preview_request.v1","action_id":"kiosk.show-controls","targets":{"device.quest.1":1,"device.quest.1":1}}"#.to_vec();
        let duplicate_response = app
            .clone()
            .oneshot(json_request("/fleet/v1/operations/preview", duplicate))
            .await
            .expect("duplicate preview response");
        assert_eq!(duplicate_response.status(), StatusCode::BAD_REQUEST);

        let operation_id = first_json["operation_id"].as_str().expect("operation id");
        let preview_id = first_json["preview"]["preview_id"]
            .as_str()
            .expect("preview id");
        let execute_request = serde_json::json!({
            "schema": "rusty.fleet.operation_execute_request.v1",
            "operation_id": operation_id,
            "preview_id": preview_id
        });
        let execute = app
            .clone()
            .oneshot(json_request(
                &format!("/fleet/v1/operations/{operation_id}/execute"),
                serde_json::to_vec(&execute_request).expect("execute JSON"),
            ))
            .await
            .expect("execute response");
        assert_eq!(execute.status(), StatusCode::OK);
        let execute_json: Value = serde_json::from_slice(
            &to_bytes(execute.into_body(), 256 * 1024)
                .await
                .expect("execute body"),
        )
        .expect("execute response JSON");
        assert_eq!(execute_json["lifecycle"], "running");
        assert_eq!(
            execute_json["targets"][0]["owner_deadline_at_ms"],
            execute_json["preview"]["expires_at_ms"]
        );
        {
            let runtime = state.runtime.lock().await;
            assert!(runtime.state_store.generation >= 3);
            assert_eq!(
                runtime
                    .adapter
                    .runtime_host_snapshot()
                    .authority_revision
                    .get(),
                2
            );
        }
        for _ in 0..700 {
            if state.runtime.lock().await.inflight_kiosk_targets.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(state.runtime.lock().await.inflight_kiosk_targets.is_empty());
        fs::remove_dir_all(state_directory).expect("remove test state directory");
    }

    #[tokio::test]
    async fn startup_resumes_durable_accepted_targets_without_reconfirmation() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[10_u8; 32]);
        let (mut config, key_id) = config(&signing_key, now_ms);
        config.kiosk_direct_operators = vec![ConfiguredKioskDirectOperator {
            device_id: "device.quest.1".to_owned(),
            endpoint: "http://127.0.0.1:39873".to_owned(),
            pairing_key: "test-pairing-key".to_owned(),
        }];
        let state_directory = config.state_directory.clone();
        let state = LocalHubState::from_config(&config, now_ms).expect("valid config");
        let app = router(state.clone());
        let signed = signed_checkin(&signing_key, key_id.as_str(), now_ms, 1);
        let identity_revision = signed.claims.observation.identity.identity_revision;
        let accepted = app
            .clone()
            .oneshot(json_request(
                "/fleet/v1/checkins",
                serde_json::to_vec(&signed).expect("signed JSON"),
            ))
            .await
            .expect("check-in response");
        assert_eq!(accepted.status(), StatusCode::OK);

        let preview_request = serde_json::json!({
            "schema": "rusty.fleet.operation_preview_request.v1",
            "action_id": "kiosk.show-controls",
            "targets": {"device.quest.1": identity_revision}
        });
        let preview = app
            .clone()
            .oneshot(json_request(
                "/fleet/v1/operations/preview",
                serde_json::to_vec(&preview_request).expect("preview JSON"),
            ))
            .await
            .expect("preview response");
        assert_eq!(preview.status(), StatusCode::OK);
        let preview_json: Value = serde_json::from_slice(
            &to_bytes(preview.into_body(), 256 * 1024)
                .await
                .expect("preview body"),
        )
        .expect("preview response JSON");
        let operation_id = preview_json["operation_id"]
            .as_str()
            .expect("operation ID")
            .to_owned();
        let preview_id = preview_json["preview"]["preview_id"]
            .as_str()
            .expect("preview ID")
            .to_owned();

        let accepted_at_ms = unix_time_ms().expect("confirmation time");
        let accepted_generation = {
            let mut runtime = state.runtime.lock().await;
            let mut candidate_hub = runtime.hub.clone();
            let confirmed = candidate_hub
                .confirm_kiosk_show_controls(&operation_id, &preview_id, accepted_at_ms)
                .expect("confirm exact preview");
            assert_eq!(confirmed.targets[0].lifecycle, CommandLifecycle::Accepted);
            let RuntimeState {
                hub,
                adapter,
                state_store,
                owner_receipts,
                ..
            } = &mut *runtime;
            state_store
                .persist(&candidate_hub, adapter, owner_receipts, accepted_at_ms)
                .expect("persist accepted target");
            *hub = candidate_hub;
            state_store.generation
        };
        drop(app);
        drop(state);

        let mut missing_private_config = config.clone();
        missing_private_config.kiosk_direct_operators.clear();
        let unrestorable = LocalHubState::from_config(&missing_private_config, accepted_at_ms + 1)
            .expect("restore durable accepted target before worker scheduling");
        let error = schedule_recovered_owner_work(unrestorable)
            .await
            .expect_err("startup must fail closed without private owner configuration");
        assert!(error.contains("lacks private direct-operator configuration"));

        let restored = LocalHubState::from_config(&config, accepted_at_ms + 1)
            .expect("restore accepted target");
        {
            let runtime = restored.runtime.lock().await;
            assert_eq!(
                runtime
                    .hub
                    .kiosk_operation(&operation_id)
                    .expect("restored operation")
                    .targets[0]
                    .lifecycle,
                CommandLifecycle::Accepted
            );
        }
        schedule_recovered_owner_work(restored.clone())
            .await
            .expect("schedule recovered accepted target");
        {
            let runtime = restored.runtime.lock().await;
            let target = &runtime
                .hub
                .kiosk_operation(&operation_id)
                .expect("scheduled operation")
                .targets[0];
            assert_ne!(target.lifecycle, CommandLifecycle::Accepted);
            assert!(runtime.state_store.generation > accepted_generation);
            assert_eq!(
                runtime
                    .adapter
                    .runtime_host_snapshot()
                    .authority_revision
                    .get(),
                2
            );
        }
        for _ in 0..700 {
            if restored
                .runtime
                .lock()
                .await
                .inflight_kiosk_targets
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            restored
                .runtime
                .lock()
                .await
                .inflight_kiosk_targets
                .is_empty()
        );
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

    #[test]
    fn transport_request_ids_bind_boot_namespace_and_sequence() {
        let first = transport_request_id("0123456789abcdef", 1);
        assert_ne!(first, transport_request_id("fedcba9876543210", 1));
        assert_ne!(first, transport_request_id("0123456789abcdef", 2));
        assert!(first.len() <= 64);
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
                kiosk_direct_operators: Vec::new(),
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
        observation.capabilities.capabilities.insert(
            "rusty-kiosk.direct-operator".to_owned(),
            CapabilityState {
                capability_id: "rusty-kiosk.direct-operator".to_owned(),
                support: SupportState::Supported,
                enablement: EnablementState::Enabled,
                authorization: AuthorizationState::Authorized,
                reachability: ReachabilityState::Reachable,
                freshness: FreshnessState::Current,
                evidence_revision: revision,
                observed_at_ms: now_ms,
                fresh_until_ms: now_ms + 60_000,
                owner: "rusty-kiosk".to_owned(),
                reason: "owner_ready".to_owned(),
                extensions: BTreeMap::new(),
            },
        );
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
        let content_length = body.len();
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, content_length)
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
