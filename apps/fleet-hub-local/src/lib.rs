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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::to_bytes;
use axum::extract::{Path as AxumPath, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use fleet_contracts::{
    AuthenticatedPackageUpdaterAcknowledgement, AuthenticatedPackageUpdaterReceipt,
    CommandLifecycle, FleetQuery, OperationExecuteRequest, OperationPreviewRequest,
    PACKAGE_INSTALL_EXECUTE_REQUEST_SCHEMA, PACKAGE_UPDATER_CLAIM_SCHEMA,
    PackageInstallReleaseExecuteRequest, PackageInstallReleasePreviewRequest,
    PackageReleaseReference, PackageUpdaterClaim, PackageUpdaterClaimRequest, PackageUpdaterOffer,
    QUEST_AWAKE_EXECUTE_REQUEST_SCHEMA, QUEST_AWAKE_PREVIEW_REQUEST_SCHEMA,
    QUEST_WIFI_ADB_EXECUTE_REQUEST_SCHEMA, QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA, QuestAwakeAction,
    QuestAwakeExecuteRequest, QuestAwakeOwnerInvocation, QuestAwakePreviewRequest,
    QuestWifiAdbAction, QuestWifiAdbExecuteRequest, QuestWifiAdbOperation,
    QuestWifiAdbOwnerInvocation, QuestWifiAdbOwnerReceipt, QuestWifiAdbPreviewRequest,
    SavedViewMutationRequest, SignedFleetCheckIn, ValidateContract,
    WINDOWS_HOTSPOT_EXECUTE_REQUEST_SCHEMA, WINDOWS_HOTSPOT_PREVIEW_REQUEST_SCHEMA,
    WindowsHotspotExecuteRequest, WindowsHotspotPreviewRequest,
};
use fleet_hub::{
    FleetApi, FleetHub, FleetHubSnapshot, HubPolicy, KioskShowControlsPreviewPlan,
    PackageInstallReleasePreviewPlan, QuestAwakePreviewPlan, QuestWifiAdbPreviewPlan,
    WindowsHotspotPreviewPlan,
};
use fleet_kiosk_adapter::{
    AdapterError, FleetKioskAdapter, HttpMethod, KioskAdapterLimits, KioskHttpRequest,
    KioskHttpResponse, KioskShowControlsRequest, KioskTransport, RawOwnerReceiptEvidence,
    TransportRequestIdSource, sha256_hex, validate_kiosk_endpoint,
};
use fleet_manifold_adapter::QuestAwakeCommandAuthorization;
use fleet_manifold_adapter::{
    FleetManifoldAdapter, FleetManifoldAdapterSnapshot, KioskShowControlsCommandAuthorization,
    PackageInstallReleaseCommandAuthorization, QuestWifiAdbCommandAuthorization,
    WindowsHotspotCommandAuthorization, kiosk_manifold_request_id, package_manifold_request_id,
    quest_awake_manifold_request_id, quest_wifi_adb_manifold_request_id,
    windows_hotspot_manifold_request_id,
};
use fleet_package_updater_adapter::{PackageUpdaterAdapterLimits, PackageUpdaterOwnerAdapter};
use fleet_provider_catalog::{
    CATALOG_SCHEMA, CONTRACT_SOURCE_COMMIT, CatalogState, ProviderCatalog, ProviderCatalogConfig,
    ProviderCatalogEntry, ProviderCatalogProjection,
};
use fleet_quest_awake_adapter::{
    QuestAwakeOwnerAdapter, QuestAwakePinnedArtifact, QuestAwakeProviderConfig,
};
use fleet_quest_connectivity_adapter::{
    QuestConnectivityOwnerAdapter, QuestConnectivityProviderConfig,
};
use fleet_windows_hotspot_adapter::{WindowsHotspotOwnerAdapter, WindowsHotspotProviderConfig};
use rusty_manifold_model::{DottedId, SchemaId};
use rusty_manifold_peer::{
    ManifoldPeerCredentialRecord, ManifoldPeerCredentialStatus, ManifoldPeerEnrollmentAction,
    ManifoldPeerEnrollmentRequest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, oneshot, watch};
use tokio::time::timeout;
use tower::limit::GlobalConcurrencyLimitLayer;

const CONFIG_SCHEMA: &str = "rusty.fleet.local_hub_config.v1";
const HEALTH_SCHEMA: &str = "rusty.fleet.local_hub_health.v1";
const ERROR_SCHEMA: &str = "rusty.fleet.local_api_error.v1";
const STATE_SCHEMA: &str = "rusty.fleet.local_hub_durable_state.v1";
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_CONCURRENT_AWAKE_PROVIDER_CALLS: usize = 8;
const MAX_CONCURRENT_CONNECTIVITY_PROVIDER_CALLS: usize = 8;
const MAX_STATE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CHECKIN_BYTES: usize = 256 * 1024;
const MAX_QUERY_BYTES: usize = 64 * 1024;
const MAX_SAVED_VIEW_BYTES: usize = 128 * 1024;
const MAX_OPERATION_BYTES: usize = 128 * 1024;
const MAX_CONCURRENT_REQUESTS: usize = 64;
const MAX_CONCURRENT_CHECKIN_LISTENER_REQUESTS: usize = 16;
const RATE_WINDOW_MS: i64 = 10_000;
const MAX_GLOBAL_CHECKINS_PER_WINDOW: usize = 4_096;
const MAX_CHECKINS_PER_CREDENTIAL_PER_WINDOW: usize = 8;
const BODY_DEADLINE: Duration = Duration::from_secs(5);
const OPERATION_PREVIEW_LIFETIME_MS: i64 = 60_000;
const DEFAULT_OPERATION_PARALLELISM: u16 = 8;
const DEFAULT_OPERATION_ATTEMPTS: u8 = 3;
const PACKAGE_OPERATION_PREVIEW_LIFETIME_MS: i64 = 15 * 60_000;
const DEFAULT_PACKAGE_OPERATION_PARALLELISM: u16 = 8;
const PACKAGE_OWNER_CLAIM_LIFETIME_MS: i64 = 60_000;
const AWAKE_OPERATION_PREVIEW_LIFETIME_MS: i64 = 15 * 60_000;
const WIFI_ADB_OPERATION_PREVIEW_LIFETIME_MS: i64 = 15 * 60_000;
const WINDOWS_HOTSPOT_PREVIEW_LIFETIME_MS: i64 = 5 * 60_000;
const AWAKE_WATCHDOG_AUTHORIZATION_LIFETIME_MS: u64 = 15 * 60_000;
const AWAKE_WATCHDOG_AUTHORIZATION_RENEWAL_MARGIN_MS: u64 = 30_000;

static OPERATION_ID_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static TRANSPORT_ID_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static AWAKE_AUTHORIZATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
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

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredPackageUpdaterOwner {
    pub owner_id: String,
    pub bearer_token: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredQuestAwakeTarget {
    pub device_id: String,
    pub serial: String,
}

impl std::fmt::Debug for ConfiguredQuestAwakeTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfiguredQuestAwakeTarget")
            .field("device_id", &self.device_id)
            .field("serial", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredQuestAwakeProvider {
    pub executable_path: PathBuf,
    pub executable_sha256: String,
    pub adb_executable_path: PathBuf,
    pub adb_executable_sha256: String,
    pub adb_support_artifacts: Vec<ConfiguredQuestAwakePinnedArtifact>,
    pub private_stage_root: PathBuf,
    pub targets: Vec<ConfiguredQuestAwakeTarget>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredQuestAwakePinnedArtifact {
    pub source_path: PathBuf,
    pub sha256: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredQuestConnectivityProvider {
    pub executable_path: PathBuf,
    pub executable_sha256: String,
    pub private_stage_root: PathBuf,
    pub targets: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredWindowsHotspotProvider {
    pub executable_path: PathBuf,
    pub executable_sha256: String,
    pub private_stage_root: PathBuf,
}

impl std::fmt::Debug for ConfiguredWindowsHotspotProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfiguredWindowsHotspotProvider")
            .field("executable_path", &"[private]")
            .field("executable_sha256", &self.executable_sha256)
            .field("private_stage_root", &"[private]")
            .finish()
    }
}

impl std::fmt::Debug for ConfiguredQuestConnectivityProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfiguredQuestConnectivityProvider")
            .field("executable_path", &"[private]")
            .field("executable_sha256", &self.executable_sha256)
            .field("private_stage_root", &"[private]")
            .field("targets", &self.targets)
            .finish()
    }
}

impl std::fmt::Debug for ConfiguredQuestAwakePinnedArtifact {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfiguredQuestAwakePinnedArtifact")
            .field("source_path", &"[private]")
            .field("sha256", &self.sha256)
            .finish()
    }
}

impl std::fmt::Debug for ConfiguredQuestAwakeProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfiguredQuestAwakeProvider")
            .field("executable_path", &"[private]")
            .field("executable_sha256", &self.executable_sha256)
            .field("adb_executable_path", &"[private]")
            .field("adb_executable_sha256", &self.adb_executable_sha256)
            .field("adb_support_artifacts", &self.adb_support_artifacts)
            .field("private_stage_root", &"[private]")
            .field("targets", &self.targets)
            .finish()
    }
}

impl std::fmt::Debug for ConfiguredPackageUpdaterOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfiguredPackageUpdaterOwner")
            .field("owner_id", &self.owner_id)
            .field("bearer_token", &"[redacted]")
            .finish()
    }
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
    #[serde(default)]
    pub checkin_bind: Option<String>,
    #[serde(default)]
    pub allow_non_loopback_checkin: bool,
    pub state_directory: PathBuf,
    pub trusted_operator_ids: Vec<DottedId>,
    #[serde(default)]
    pub enrollments: Vec<ConfiguredEnrollment>,
    #[serde(default)]
    pub kiosk_direct_operators: Vec<ConfiguredKioskDirectOperator>,
    #[serde(default)]
    pub package_updater_owner: Option<ConfiguredPackageUpdaterOwner>,
    #[serde(default)]
    pub quest_awake_provider: Option<ConfiguredQuestAwakeProvider>,
    #[serde(default)]
    pub quest_connectivity_provider: Option<ConfiguredQuestConnectivityProvider>,
    #[serde(default)]
    pub windows_hotspot_provider: Option<ConfiguredWindowsHotspotProvider>,
    #[serde(default)]
    pub provider_catalog: Vec<ProviderCatalogConfig>,
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
        if self.package_updater_owner.is_some() && !bind.ip().is_loopback() {
            return Err("package updater owner ingress requires a loopback bind".to_owned());
        }
        if self.quest_awake_provider.is_some() && !bind.ip().is_loopback() {
            return Err("Quest awake provider requires a loopback Hub bind".to_owned());
        }
        if self.quest_connectivity_provider.is_some() && !bind.ip().is_loopback() {
            return Err("Quest connectivity provider requires a loopback Hub bind".to_owned());
        }
        if self.windows_hotspot_provider.is_some() && !bind.ip().is_loopback() {
            return Err("Windows hotspot provider requires a loopback Hub bind".to_owned());
        }
        if let Some(checkin_bind) = self.checkin_socket()? {
            if checkin_bind == bind {
                return Err("checkin_bind must not collide with the operator/API bind".to_owned());
            }
            if !checkin_bind.ip().is_loopback() && !self.allow_non_loopback_checkin {
                return Err(
                    "non-loopback checkin_bind requires explicit allow_non_loopback_checkin=true"
                        .to_owned(),
                );
            }
            if checkin_bind.ip().is_loopback()
                || checkin_bind.ip().is_unspecified()
                || checkin_bind.ip().is_multicast()
                || checkin_bind.ip().to_string() == "255.255.255.255"
            {
                return Err(
                    "checkin_bind requires one exact non-loopback unicast interface IP".to_owned(),
                );
            }
        } else if self.allow_non_loopback_checkin {
            return Err("allow_non_loopback_checkin requires an explicit checkin_bind".to_owned());
        }
        if !self.provider_catalog.is_empty() && !bind.ip().is_loopback() {
            return Err("provider catalog requires a loopback Hub bind".to_owned());
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
        if let Some(owner) = &self.package_updater_owner
            && (owner.owner_id != "rusty-quest.package-updater"
                || owner.bearer_token.len() < 32
                || owner.bearer_token.len() > 512)
        {
            return Err(
                "package updater owner requires the pinned owner id and a 32..512 byte private bearer token"
                    .to_owned(),
            );
        }
        if let Some(provider) = &self.quest_awake_provider {
            QuestAwakeProviderConfig {
                executable_path: provider.executable_path.clone(),
                executable_sha256: provider.executable_sha256.clone(),
                adb_executable_path: provider.adb_executable_path.clone(),
                adb_executable_sha256: provider.adb_executable_sha256.clone(),
                adb_support_artifacts: provider
                    .adb_support_artifacts
                    .iter()
                    .map(|artifact| QuestAwakePinnedArtifact {
                        source_path: artifact.source_path.clone(),
                        sha256: artifact.sha256.clone(),
                    })
                    .collect(),
                private_stage_root: provider.private_stage_root.clone(),
            }
            .verify_artifacts()
            .map_err(|error| format!("invalid Quest awake provider: {error}"))?;
            if provider.targets.is_empty() || provider.targets.len() > 10_000 {
                return Err(
                    "Quest awake provider requires 1 through 10,000 exact target bindings"
                        .to_owned(),
                );
            }
            let mut devices = BTreeSet::new();
            let mut serials = BTreeSet::new();
            for target in &provider.targets {
                if target.device_id.is_empty()
                    || target.device_id.len() > 256
                    || target.serial.is_empty()
                    || target.serial.len() > 256
                    || target
                        .serial
                        .bytes()
                        .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
                    || !devices.insert(target.device_id.clone())
                    || !serials.insert(target.serial.clone())
                {
                    return Err(
                        "Quest awake target bindings require unique bounded device IDs and exact serials"
                            .to_owned(),
                    );
                }
            }
        }
        if let Some(provider) = &self.quest_connectivity_provider {
            QuestConnectivityProviderConfig {
                executable_path: provider.executable_path.clone(),
                executable_sha256: provider.executable_sha256.clone(),
                private_stage_root: provider.private_stage_root.clone(),
            }
            .verify_artifact()
            .map_err(|error| format!("invalid Quest connectivity provider: {error}"))?;
            let unique = provider.targets.iter().collect::<BTreeSet<_>>();
            if provider.targets.is_empty()
                || provider.targets.len() > 10_000
                || unique.len() != provider.targets.len()
                || provider.targets.iter().any(|device_id| {
                    device_id.is_empty()
                        || device_id.len() > 256
                        || device_id.bytes().any(|byte| {
                            !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
                        })
                })
            {
                return Err(
                    "Quest connectivity provider requires 1 through 10,000 unique portable target IDs"
                        .to_owned(),
                );
            }
        }
        if let Some(provider) = &self.windows_hotspot_provider {
            WindowsHotspotProviderConfig {
                executable_path: provider.executable_path.clone(),
                executable_sha256: provider.executable_sha256.clone(),
                private_stage_root: provider.private_stage_root.clone(),
            }
            .verify_artifact()
            .map_err(|error| format!("invalid Windows hotspot provider: {error}"))?;
        }
        if self.provider_catalog.len() > 8 {
            return Err("provider catalog accepts at most 8 configured slots".to_owned());
        }
        let mut catalog_ids = BTreeSet::new();
        let mut provider_ids = BTreeSet::new();
        for provider in &self.provider_catalog {
            provider
                .validate()
                .map_err(|error| format!("invalid provider catalog entry: {}", error.code))?;
            if !catalog_ids.insert(&provider.catalog_id)
                || !provider_ids.insert(&provider.expected_provider_id)
            {
                return Err(
                    "provider catalog slot and expected provider identities must be unique"
                        .to_owned(),
                );
            }
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

    fn checkin_socket(&self) -> Result<Option<SocketAddr>, String> {
        self.checkin_bind
            .as_ref()
            .map(|value| {
                value
                    .parse::<SocketAddr>()
                    .map_err(|error| format!("checkin_bind must be an IP socket address: {error}"))
            })
            .transpose()
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
    package_updater_adapter: PackageUpdaterOwnerAdapter,
    package_updater_owner: Option<ConfiguredPackageUpdaterOwner>,
    quest_awake_provider: Option<ConfiguredQuestAwakeProvider>,
    quest_awake_adapter: QuestAwakeOwnerAdapter,
    inflight_awake_targets: BTreeSet<(String, String)>,
    windows_awake_watchdogs: BTreeMap<String, WindowsAwakeWatchdogControl>,
    awake_provider_slots: Arc<Semaphore>,
    awake_device_slots: BTreeMap<String, Arc<Semaphore>>,
    quest_connectivity_provider: Option<ConfiguredQuestConnectivityProvider>,
    quest_connectivity_adapter: QuestConnectivityOwnerAdapter,
    inflight_connectivity_targets: BTreeSet<(String, String)>,
    connectivity_provider_slots: Arc<Semaphore>,
    connectivity_device_slots: BTreeMap<String, Arc<Semaphore>>,
    windows_hotspot_provider: Option<ConfiguredWindowsHotspotProvider>,
    inflight_windows_hotspot_operations: BTreeSet<String>,
    provider_catalog_configs: Vec<ProviderCatalogConfig>,
    provider_catalog_snapshot: ProviderCatalogProjection,
    provider_catalog_refresh_inflight: bool,
    provider_catalog_last_refresh_ms: Option<i64>,
}

#[derive(Clone)]
struct WindowsAwakeWatchdogControl {
    generation: String,
    cancel: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
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
    for operation in restored_hub.package_operations() {
        for target in operation.targets {
            if let Some(invocation) = target.invocation
                && !restored_adapter.has_applied_package_authorization(
                    &operation.operation_id,
                    &target.device_id,
                    &invocation.owner_action_request_id,
                )
            {
                return Err(format!(
                    "package operation {} target {} lacks applied Manifold authority",
                    operation.operation_id, target.device_id
                ));
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
                package_updater_adapter: PackageUpdaterOwnerAdapter::new(
                    PackageUpdaterAdapterLimits::default(),
                )
                .map_err(|error| format!("invalid package updater adapter limits: {error}"))?,
                package_updater_owner: config.package_updater_owner.clone(),
                quest_awake_provider: config.quest_awake_provider.clone(),
                quest_awake_adapter: QuestAwakeOwnerAdapter::default(),
                inflight_awake_targets: BTreeSet::new(),
                windows_awake_watchdogs: BTreeMap::new(),
                awake_provider_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_AWAKE_PROVIDER_CALLS)),
                awake_device_slots: config
                    .quest_awake_provider
                    .as_ref()
                    .map(|provider| {
                        provider
                            .targets
                            .iter()
                            .map(|target| (target.device_id.clone(), Arc::new(Semaphore::new(1))))
                            .collect()
                    })
                    .unwrap_or_default(),
                quest_connectivity_provider: config.quest_connectivity_provider.clone(),
                quest_connectivity_adapter: QuestConnectivityOwnerAdapter::default(),
                inflight_connectivity_targets: BTreeSet::new(),
                connectivity_provider_slots: Arc::new(Semaphore::new(
                    MAX_CONCURRENT_CONNECTIVITY_PROVIDER_CALLS,
                )),
                connectivity_device_slots: config
                    .quest_connectivity_provider
                    .as_ref()
                    .map(|provider| {
                        provider
                            .targets
                            .iter()
                            .map(|device_id| (device_id.clone(), Arc::new(Semaphore::new(1))))
                            .collect()
                    })
                    .unwrap_or_default(),
                windows_hotspot_provider: config.windows_hotspot_provider.clone(),
                inflight_windows_hotspot_operations: BTreeSet::new(),
                provider_catalog_configs: config.provider_catalog.clone(),
                provider_catalog_snapshot: initial_provider_catalog_snapshot(
                    &config.provider_catalog,
                ),
                provider_catalog_refresh_inflight: false,
                provider_catalog_last_refresh_ms: None,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictPackageInstallReleasePreviewRequest {
    schema: String,
    action_id: String,
    release: PackageReleaseReference,
    expected_package_name: String,
    expected_rollout_ring: String,
    #[serde(deserialize_with = "deserialize_unique_targets")]
    targets: BTreeMap<String, u64>,
}

impl From<StrictPackageInstallReleasePreviewRequest> for PackageInstallReleasePreviewRequest {
    fn from(value: StrictPackageInstallReleasePreviewRequest) -> Self {
        Self {
            schema: value.schema,
            action_id: value.action_id,
            release: value.release,
            expected_package_name: value.expected_package_name,
            expected_rollout_ring: value.expected_rollout_ring,
            targets: value.targets,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictQuestAwakePreviewRequest {
    schema: String,
    action_id: String,
    action: QuestAwakeAction,
    duration_ms: u32,
    watchdog_interval_ms: u32,
    #[serde(deserialize_with = "deserialize_unique_targets")]
    targets: BTreeMap<String, u64>,
}

impl From<StrictQuestAwakePreviewRequest> for QuestAwakePreviewRequest {
    fn from(value: StrictQuestAwakePreviewRequest) -> Self {
        Self {
            schema: value.schema,
            action_id: value.action_id,
            action: value.action,
            duration_ms: value.duration_ms,
            watchdog_interval_ms: value.watchdog_interval_ms,
            targets: value.targets,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictQuestWifiAdbPreviewRequest {
    schema: String,
    action_id: String,
    action: QuestWifiAdbAction,
    #[serde(deserialize_with = "deserialize_unique_targets")]
    targets: BTreeMap<String, u64>,
}

impl From<StrictQuestWifiAdbPreviewRequest> for QuestWifiAdbPreviewRequest {
    fn from(value: StrictQuestWifiAdbPreviewRequest) -> Self {
        Self {
            schema: value.schema,
            action_id: value.action_id,
            action: value.action,
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
        .route("/fleet/v1/provider-catalog", get(provider_catalog))
        .route(
            "/fleet/v1/provider-catalog/refresh",
            post(refresh_provider_catalog),
        )
        .route("/fleet/v1/checkins", post(checkin))
        .route("/fleet/v1/query", post(query_devices))
        .route("/fleet/v1/summary", get(summary))
        .route("/fleet/v1/operations/preview", post(preview_operation))
        .route(
            "/fleet/v1/operations/{operation_id}/execute",
            post(execute_operation),
        )
        .route("/fleet/v1/operations/{operation_id}", get(operation_status))
        .route(
            "/fleet/v1/package-install-releases/preview",
            post(preview_package_install_release),
        )
        .route(
            "/fleet/v1/package-install-releases/{operation_id}/execute",
            post(execute_package_install_release),
        )
        .route(
            "/fleet/v1/package-updater/claims",
            post(claim_package_updater_work),
        )
        .route(
            "/fleet/v1/package-updater/offers",
            get(peek_package_updater_offer),
        )
        .route(
            "/fleet/v1/package-install-releases/{operation_id}/acknowledgements",
            post(acknowledge_package_install_release),
        )
        .route(
            "/fleet/v1/package-install-releases/{operation_id}/receipts",
            post(apply_package_install_release_receipt),
        )
        .route(
            "/fleet/v1/package-install-releases/{operation_id}",
            get(package_install_release_status),
        )
        .route("/fleet/v1/quest-awake/preview", post(preview_quest_awake))
        .route(
            "/fleet/v1/quest-awake/{operation_id}/execute",
            post(execute_quest_awake),
        )
        .route(
            "/fleet/v1/quest-awake/{operation_id}",
            get(quest_awake_status),
        )
        .route(
            "/fleet/v1/quest-wifi-adb/preview",
            post(preview_quest_wifi_adb),
        )
        .route(
            "/fleet/v1/quest-wifi-adb/{operation_id}/execute",
            post(execute_quest_wifi_adb),
        )
        .route(
            "/fleet/v1/quest-wifi-adb/{operation_id}",
            get(quest_wifi_adb_status),
        )
        .route(
            "/fleet/v1/windows-hotspot/preview",
            post(preview_windows_hotspot),
        )
        .route(
            "/fleet/v1/windows-hotspot/{operation_id}/execute",
            post(execute_windows_hotspot),
        )
        .route(
            "/fleet/v1/windows-hotspot/{operation_id}",
            get(windows_hotspot_status),
        )
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

/// Check-in-only ingress for enrolled devices on an explicitly opted-in
/// listener. No operator, owner, catalog, query, or operation route is mounted.
pub fn checkin_router(state: LocalHubState) -> Router {
    Router::new()
        .route("/fleet/v1/checkins", post(checkin))
        .with_state(state)
        .layer(GlobalConcurrencyLimitLayer::new(
            MAX_CONCURRENT_CHECKIN_LISTENER_REQUESTS,
        ))
}

pub async fn serve(config: LocalHubConfig) -> Result<(), String> {
    let bind = config.validate()?;
    let checkin_bind = config.checkin_socket()?;
    let state = LocalHubState::from_config(&config, unix_time_ms()?)?;
    schedule_recovered_owner_work(state.clone()).await?;
    schedule_recovered_awake_work(state.clone()).await?;
    schedule_recovered_connectivity_work(state.clone()).await?;
    schedule_recovered_windows_hotspot_work(state.clone()).await?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|error| format!("failed to bind {bind}: {error}"))?;
    let Some(checkin_bind) = checkin_bind else {
        return axum::serve(listener, router(state))
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(|error| format!("local Hub server failed: {error}"));
    };
    let checkin_listener = tokio::net::TcpListener::bind(checkin_bind)
        .await
        .map_err(|error| format!("failed to bind check-in listener {checkin_bind}: {error}"))?;
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = shutdown_sender.send(true);
    });
    let mut operator_shutdown = shutdown_receiver.clone();
    let mut checkin_shutdown = shutdown_receiver;
    let operator =
        axum::serve(listener, router(state.clone())).with_graceful_shutdown(async move {
            while !*operator_shutdown.borrow() {
                if operator_shutdown.changed().await.is_err() {
                    break;
                }
            }
        });
    let checkins =
        axum::serve(checkin_listener, checkin_router(state)).with_graceful_shutdown(async move {
            while !*checkin_shutdown.borrow() {
                if checkin_shutdown.changed().await.is_err() {
                    break;
                }
            }
        });
    tokio::try_join!(operator, checkins)
        .map(|_| ())
        .map_err(|error| format!("local Hub listener failed: {error}"))
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

async fn provider_catalog(State(state): State<LocalHubState>) -> Response {
    let runtime = state.runtime.lock().await;
    (
        StatusCode::OK,
        Json(runtime.provider_catalog_snapshot.clone()),
    )
        .into_response()
}

async fn refresh_provider_catalog(State(state): State<LocalHubState>) -> Response {
    let started_at_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let configs = {
        let mut runtime = state.runtime.lock().await;
        if runtime.provider_catalog_refresh_inflight {
            return api_error(
                StatusCode::TOO_MANY_REQUESTS,
                "provider_catalog_refresh_saturated",
                "one bounded provider catalog refresh is already running",
            );
        }
        if runtime
            .provider_catalog_last_refresh_ms
            .is_some_and(|last| started_at_ms <= last)
        {
            runtime.provider_catalog_snapshot = failed_provider_catalog_snapshot(
                &runtime.provider_catalog_configs,
                runtime.provider_catalog_snapshot.revision.saturating_add(1),
                "provider-catalog-clock-rollback",
            );
            return api_error(
                StatusCode::CONFLICT,
                "provider_catalog_clock_rollback",
                "provider catalog refresh rejected a non-advancing host clock",
            );
        }
        runtime.provider_catalog_refresh_inflight = true;
        runtime.provider_catalog_snapshot = failed_provider_catalog_snapshot(
            &runtime.provider_catalog_configs,
            runtime.provider_catalog_snapshot.revision.saturating_add(1),
            "provider-catalog-refresh-in-progress",
        );
        runtime.provider_catalog_configs.clone()
    };
    let result =
        tokio::task::spawn_blocking(move || ProviderCatalog::default().inspect_all(&configs)).await;
    let completed_at_ms = unix_time_ms().ok();
    let mut runtime = state.runtime.lock().await;
    runtime.provider_catalog_refresh_inflight = false;
    match result {
        Ok(mut projection) if completed_at_ms.is_some_and(|value| value >= started_at_ms) => {
            let completed_at_ms = completed_at_ms.unwrap_or(started_at_ms);
            projection.revision = runtime.provider_catalog_snapshot.revision.saturating_add(1);
            projection.refreshed_at_ms = Some(completed_at_ms);
            runtime.provider_catalog_last_refresh_ms = Some(completed_at_ms);
            runtime.provider_catalog_snapshot = projection.clone();
            (StatusCode::OK, Json(projection)).into_response()
        }
        _ => {
            runtime.provider_catalog_snapshot = failed_provider_catalog_snapshot(
                &runtime.provider_catalog_configs,
                runtime.provider_catalog_snapshot.revision.saturating_add(1),
                "provider-catalog-refresh-failed",
            );
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "provider_catalog_task_failed",
                "provider catalog refresh did not complete",
            )
        }
    }
}

fn initial_provider_catalog_snapshot(
    configs: &[ProviderCatalogConfig],
) -> ProviderCatalogProjection {
    failed_provider_catalog_snapshot(configs, 1, "provider-catalog-not-refreshed")
}

fn failed_provider_catalog_snapshot(
    configs: &[ProviderCatalogConfig],
    revision: u64,
    configured_reason: &str,
) -> ProviderCatalogProjection {
    ProviderCatalogProjection {
        schema: CATALOG_SCHEMA,
        contract_source_commit: CONTRACT_SOURCE_COMMIT,
        entries: configs
            .iter()
            .map(|config| ProviderCatalogEntry {
                catalog_id: config.catalog_id.clone(),
                state: if config.executable_path.is_none() && config.executable_sha256.is_none() {
                    CatalogState::Unconfigured
                } else {
                    CatalogState::Unavailable
                },
                reason: if config.executable_path.is_none() && config.executable_sha256.is_none() {
                    "provider-not-configured".to_owned()
                } else {
                    configured_reason.to_owned()
                },
                descriptor: None,
                metadata_only: true,
                authorizes_execution: false,
            })
            .collect(),
        metadata_only: true,
        authorizes_execution: false,
        revision,
        refreshed_at_ms: None,
    }
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

async fn preview_package_install_release(
    State(state): State<LocalHubState>,
    request: Request,
) -> Response {
    let bytes =
        match strict_json_body(request, MAX_OPERATION_BYTES, "package operation previews").await {
            Ok(bytes) => bytes,
            Err(response) => return response,
        };
    let request = match serde_json::from_slice::<StrictPackageInstallReleasePreviewRequest>(&bytes)
    {
        Ok(value) => PackageInstallReleasePreviewRequest::from(value),
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_package_operation_preview_json",
                format!("package operation preview is not valid strict JSON: {error}"),
            );
        }
    };
    if let Err(failures) = request.validate() {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_package_operation_preview",
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
        .package_operations()
        .into_iter()
        .find(|operation| {
            operation.preview.fleet_revision == runtime.hub.result_revision()
                && operation.preview.expires_at_ms >= now_ms
                && operation.preview.release == request.release
                && operation.preview.expected_package_name == request.expected_package_name
                && operation.preview.expected_rollout_ring == request.expected_rollout_ring
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
    let expires_at_ms = match now_ms.checked_add(PACKAGE_OPERATION_PREVIEW_LIFETIME_MS) {
        Some(value) => value,
        None => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "clock_error",
                "package operation preview expiry overflowed",
            );
        }
    };
    let (operation_id, preview_id) = package_operation_ids(&request, now_ms);
    let mut candidate_hub = runtime.hub.clone();
    let operation =
        match candidate_hub.preview_package_install_release(PackageInstallReleasePreviewPlan {
            operation_id,
            preview_id,
            request,
            created_at_ms: now_ms,
            expires_at_ms,
            max_parallelism: DEFAULT_PACKAGE_OPERATION_PARALLELISM,
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

async fn execute_package_install_release(
    State(state): State<LocalHubState>,
    AxumPath(operation_id): AxumPath<String>,
    request: Request,
) -> Response {
    let bytes =
        match strict_json_body(request, MAX_OPERATION_BYTES, "package operation execution").await {
            Ok(bytes) => bytes,
            Err(response) => return response,
        };
    let execute = match serde_json::from_slice::<PackageInstallReleaseExecuteRequest>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_package_operation_execute_json",
                format!("package operation execution is not valid JSON: {error}"),
            );
        }
    };
    if execute.schema != PACKAGE_INSTALL_EXECUTE_REQUEST_SCHEMA
        || operation_id != execute.operation_id
    {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "package_operation_identity_mismatch",
            "package operation path, schema, and payload identity must match",
        );
    }
    if let Err(failures) = execute.validate() {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_package_operation_execute",
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
    let existing = match runtime.hub.package_operation(&operation_id) {
        Ok(operation) => operation,
        Err(error) => return hub_operation_error(error),
    };
    if existing.preview.preview_id != execute.preview_id {
        return api_error(
            StatusCode::CONFLICT,
            "package_operation_preview_conflict",
            "execute request does not bind the stored immutable package preview",
        );
    }
    if existing.lifecycle != CommandLifecycle::Proposed {
        return Json(existing).into_response();
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
            "package_operation_has_no_eligible_targets",
            "the immutable preview contains no eligible package-updater targets",
        );
    }
    let mut candidate_hub = runtime.hub.clone();
    let mut candidate_adapter = runtime.adapter.clone();
    let mut operation = match candidate_hub.confirm_package_install_release(
        &operation_id,
        &execute.preview_id,
        now_ms,
    ) {
        Ok(operation) => operation,
        Err(error) => return hub_operation_error(error),
    };
    let expires_unsigned = match u64::try_from(operation.preview.expires_at_ms) {
        Ok(value) => value,
        Err(_) => {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "package_operation_expiry_invalid",
                "package operation expiry is not representable",
            );
        }
    };
    for device_id in eligible_devices {
        let Some(target) = operation
            .targets
            .iter()
            .find(|target| target.device_id == device_id)
        else {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "package_operation_target_missing",
                "confirmed package operation omitted a frozen target",
            );
        };
        let owner_action_request_id =
            package_owner_action_request_id(&operation.operation_id, &device_id);
        let manifold_request_id = package_manifold_request_id(
            &operation.operation_id,
            &device_id,
            &owner_action_request_id,
        );
        if let Err(error) = candidate_adapter.authorize_package_install_release(
            &PackageInstallReleaseCommandAuthorization {
                manifold_request_id,
                owner_action_request_id: owner_action_request_id.clone(),
                requester_id: "operator.fleet.local".to_owned(),
                operation_id: operation.operation_id.clone(),
                preview_id: operation.preview.preview_id.clone(),
                device_id: device_id.clone(),
                identity_revision: target.identity_revision,
                release: operation.preview.release.clone(),
                expected_package_name: operation.preview.expected_package_name.clone(),
                expected_rollout_ring: operation.preview.expected_rollout_ring.clone(),
                issued_at_ms: now_unsigned,
                expires_at_ms: expires_unsigned,
            },
            now_unsigned,
        ) {
            return api_error(StatusCode::CONFLICT, "manifold_command_rejected", error);
        }
        operation = match candidate_hub.prepare_package_install_release_invocation(
            &operation.operation_id,
            &device_id,
            owner_action_request_id,
            now_ms,
        ) {
            Ok(operation) => operation,
            Err(error) => return hub_operation_error(error),
        };
        let Some(invocation) = operation
            .targets
            .iter()
            .find(|target| target.device_id == device_id)
            .and_then(|target| target.invocation.clone())
        else {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "package_invocation_missing",
                "package dispatch preparation omitted its exact owner invocation",
            );
        };
        if let Err(error) = runtime
            .package_updater_adapter
            .prepare_invocation(invocation, now_ms)
        {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "package_invocation_invalid",
                error.to_string(),
            );
        }
    }
    let RuntimeState {
        hub,
        adapter,
        state_store,
        owner_receipts,
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
    Json(operation).into_response()
}

async fn claim_package_updater_work(
    State(state): State<LocalHubState>,
    request: Request,
) -> Response {
    let configured_owner = {
        let runtime = state.runtime.lock().await;
        runtime.package_updater_owner.clone()
    };
    if let Err(response) =
        authenticate_package_updater(request.headers(), configured_owner.as_ref())
    {
        return *response;
    }
    let bytes = match strict_json_body(request, MAX_OPERATION_BYTES, "package updater claim").await
    {
        Ok(bytes) => bytes,
        Err(response) => return response,
    };
    let claim_request = match serde_json::from_slice::<PackageUpdaterClaimRequest>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_package_claim_json",
                error.to_string(),
            );
        }
    };
    if let Err(failures) = claim_request.validate() {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_package_claim",
            format_contract_failures(&failures),
        );
    }
    if configured_owner
        .as_ref()
        .is_none_or(|owner| owner.owner_id != claim_request.owner_id)
    {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "package_owner_identity_mismatch",
            "authenticated package owner identity does not match the claim",
        );
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let mut runtime = state.runtime.lock().await;
    let mut candidate_hub = runtime.hub.clone();
    match candidate_hub.expire_package_updater_claims(now_ms) {
        Ok(_) => {}
        Err(error) => return hub_operation_error(error),
    }
    let selected = candidate_hub.select_package_updater_offer(now_ms);
    let Some(invocation) = selected else {
        return api_error(
            StatusCode::CONFLICT,
            "package_claim_offer_stale",
            "the claimed package offer is no longer selectable",
        );
    };
    let invocation_sha256 = json_sha256(&invocation);
    if claim_request.operation_id != invocation.operation_id
        || claim_request.device_id != invocation.device_id
        || !constant_time_equal(
            claim_request.expected_invocation_sha256.as_bytes(),
            invocation_sha256.as_bytes(),
        )
    {
        return api_error(
            StatusCode::CONFLICT,
            "package_claim_offer_mismatch",
            "claim request does not match the current package offer",
        );
    }
    if !runtime.adapter.has_applied_package_authorization(
        &invocation.operation_id,
        &invocation.device_id,
        &invocation.owner_action_request_id,
    ) {
        return api_error(
            StatusCode::CONFLICT,
            "package_manifold_authority_unavailable",
            "the exact prepared invocation no longer has retained Manifold command authority",
        );
    }
    let release_sha256 = json_sha256(&invocation.release);
    let target_sha256 = json_sha256(&(invocation.device_id.as_str(), invocation.identity_revision));
    let claim_id = format!(
        "package-claim-{}",
        &sha256_hex(
            format!(
                "{}:{}:{}",
                claim_request.owner_id, claim_request.request_id, invocation_sha256
            )
            .as_bytes()
        )[..32]
    );
    let claim = PackageUpdaterClaim {
        schema: PACKAGE_UPDATER_CLAIM_SCHEMA.to_owned(),
        claim_id,
        owner_id: claim_request.owner_id,
        request_id: claim_request.request_id,
        claimed_at_ms: now_ms,
        expires_at_ms: invocation
            .expires_at_ms
            .min(now_ms.saturating_add(PACKAGE_OWNER_CLAIM_LIFETIME_MS)),
        invocation_sha256,
        release_sha256,
        target_sha256,
        invocation,
    };
    if let Err(error) = candidate_hub.offer_package_updater_claim(claim.clone(), now_ms) {
        return hub_operation_error(error);
    }
    let RuntimeState {
        adapter,
        state_store,
        owner_receipts,
        hub,
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
    Json(claim).into_response()
}

async fn peek_package_updater_offer(
    State(state): State<LocalHubState>,
    request: Request,
) -> Response {
    let configured_owner = {
        let runtime = state.runtime.lock().await;
        runtime.package_updater_owner.clone()
    };
    if let Err(response) =
        authenticate_package_updater(request.headers(), configured_owner.as_ref())
    {
        return *response;
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let runtime = state.runtime.lock().await;
    let offer = runtime
        .hub
        .select_package_updater_offer(now_ms)
        .map(|invocation| PackageUpdaterOffer {
            schema: fleet_contracts::PACKAGE_UPDATER_OFFER_SCHEMA.to_owned(),
            owner_id: configured_owner
                .as_ref()
                .expect("authenticated owner")
                .owner_id
                .clone(),
            operation_id: invocation.operation_id.clone(),
            device_id: invocation.device_id.clone(),
            invocation_sha256: json_sha256(&invocation),
        });
    Json(serde_json::json!({
        "schema": "rusty.fleet.package_updater_offer_result.v1",
        "offer": offer
    }))
    .into_response()
}

fn authenticate_package_updater(
    headers: &HeaderMap,
    configured: Option<&ConfiguredPackageUpdaterOwner>,
) -> Result<(), Box<Response>> {
    let Some(configured) = configured else {
        return Err(Box::new(api_error(
            StatusCode::NOT_IMPLEMENTED,
            "package_owner_authenticated_ingress_unavailable",
            "package updater owner ingress is disabled without explicit private configuration",
        )));
    };
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if supplied.is_none_or(|token| {
        let supplied_digest = Sha256::digest(token.as_bytes());
        let configured_digest = Sha256::digest(configured.bearer_token.as_bytes());
        !constant_time_equal(supplied_digest.as_slice(), configured_digest.as_slice())
    }) {
        return Err(Box::new(api_error(
            StatusCode::UNAUTHORIZED,
            "package_owner_authentication_failed",
            "package updater owner authentication failed",
        )));
    }
    Ok(())
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn json_sha256<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("contract serialization is infallible");
    format!("sha256:{}", sha256_hex(&bytes))
}

async fn acknowledge_package_install_release(
    State(state): State<LocalHubState>,
    AxumPath(operation_id): AxumPath<String>,
    request: Request,
) -> Response {
    let configured_owner = {
        let runtime = state.runtime.lock().await;
        runtime.package_updater_owner.clone()
    };
    if let Err(response) =
        authenticate_package_updater(request.headers(), configured_owner.as_ref())
    {
        return *response;
    }
    let bytes = match strict_json_body(
        request,
        MAX_OPERATION_BYTES,
        "package updater acknowledgement",
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(response) => return response,
    };
    let submission =
        match serde_json::from_slice::<AuthenticatedPackageUpdaterAcknowledgement>(&bytes) {
            Ok(value) => value,
            Err(error) => {
                return api_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_package_acknowledgement_json",
                    format!("package updater acknowledgement is not valid JSON: {error}"),
                );
            }
        };
    if submission.schema != "rusty.fleet.authenticated_package_updater_acknowledgement.v1"
        || operation_id != submission.acknowledgement.operation_id
    {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "package_operation_identity_mismatch",
            "package operation path and acknowledgement identity must match",
        );
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let mut runtime = state.runtime.lock().await;
    let operation = match runtime.hub.package_operation(&operation_id) {
        Ok(operation) => operation,
        Err(error) => return hub_operation_error(error),
    };
    let binding_exists = operation.targets.iter().any(|target| {
        target.device_id == submission.acknowledgement.device_id
            && target.owner_claim.as_ref().is_some_and(|claim| {
                claim.claim_id == submission.claim_id
                    && claim.owner_id == submission.owner_id
                    && constant_time_equal(
                        claim.invocation_sha256.as_bytes(),
                        submission.invocation_sha256.as_bytes(),
                    )
                    && claim.expires_at_ms > now_ms
            })
    });
    if !binding_exists {
        return api_error(
            StatusCode::CONFLICT,
            "package_acknowledgement_binding_mismatch",
            "acknowledgement does not bind a prepared package invocation",
        );
    }
    let target = operation
        .targets
        .iter()
        .find(|target| target.device_id == submission.acknowledgement.device_id)
        .expect("binding check found target");
    let invocation = target.invocation.as_ref().expect("claim binds invocation");
    let acknowledgement = match runtime
        .package_updater_adapter
        .validate_untrusted_acknowledgement(invocation, submission.acknowledgement, now_ms)
    {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "package_acknowledgement_invalid",
                error.to_string(),
            );
        }
    };
    let mut candidate_hub = runtime.hub.clone();
    let updated = match candidate_hub
        .admit_authenticated_package_updater_acknowledgement(acknowledgement, now_ms)
    {
        Ok(value) => value,
        Err(error) => return hub_operation_error(error),
    };
    let RuntimeState {
        adapter,
        state_store,
        owner_receipts,
        hub,
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
    Json(updated).into_response()
}

async fn apply_package_install_release_receipt(
    State(state): State<LocalHubState>,
    AxumPath(operation_id): AxumPath<String>,
    request: Request,
) -> Response {
    let configured_owner = {
        let runtime = state.runtime.lock().await;
        runtime.package_updater_owner.clone()
    };
    if let Err(response) =
        authenticate_package_updater(request.headers(), configured_owner.as_ref())
    {
        return *response;
    }
    let bytes =
        match strict_json_body(request, MAX_OPERATION_BYTES, "package updater receipt").await {
            Ok(bytes) => bytes,
            Err(response) => return response,
        };
    let submission = match serde_json::from_slice::<AuthenticatedPackageUpdaterReceipt>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_package_receipt_json",
                format!("package updater receipt is not valid JSON: {error}"),
            );
        }
    };
    if submission.schema != "rusty.fleet.authenticated_package_updater_receipt.v1"
        || operation_id != submission.effective_receipt.operation_id
    {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "package_operation_identity_mismatch",
            "package operation path and receipt identity must match",
        );
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let mut runtime = state.runtime.lock().await;
    let operation = match runtime.hub.package_operation(&operation_id) {
        Ok(operation) => operation,
        Err(error) => return hub_operation_error(error),
    };
    let binding_exists = operation.targets.iter().any(|target| {
        target.device_id == submission.effective_receipt.device_id
            && target.owner_claim.as_ref().is_some_and(|claim| {
                claim.claim_id == submission.claim_id
                    && claim.owner_id == submission.owner_id
                    && constant_time_equal(
                        claim.invocation_sha256.as_bytes(),
                        submission.invocation_sha256.as_bytes(),
                    )
                    && target
                        .invocation_acknowledgement
                        .as_ref()
                        .is_some_and(|ack| {
                            ack.accepted
                                && ack.operation_id == submission.effective_receipt.operation_id
                                && ack.device_id == submission.effective_receipt.device_id
                                && ack.owner_action_request_id
                                    == submission.effective_receipt.owner_action_request_id
                        })
            })
    });
    if !binding_exists {
        return api_error(
            StatusCode::CONFLICT,
            "package_receipt_binding_mismatch",
            "receipt does not bind a prepared package invocation",
        );
    }
    let target = operation
        .targets
        .iter()
        .find(|target| target.device_id == submission.effective_receipt.device_id)
        .expect("binding check found target");
    let invocation = target.invocation.as_ref().expect("claim binds invocation");
    let receipt = match runtime
        .package_updater_adapter
        .validate_untrusted_effective_receipt(invocation, submission.effective_receipt, now_ms)
    {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "package_receipt_invalid",
                error.to_string(),
            );
        }
    };
    let mut candidate_hub = runtime.hub.clone();
    let updated = match candidate_hub.admit_authenticated_package_updater_receipt(receipt, now_ms) {
        Ok(value) => value,
        Err(error) => return hub_operation_error(error),
    };
    let RuntimeState {
        adapter,
        state_store,
        owner_receipts,
        hub,
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
    Json(updated).into_response()
}

async fn package_install_release_status(
    State(state): State<LocalHubState>,
    AxumPath(operation_id): AxumPath<String>,
) -> Response {
    let runtime = state.runtime.lock().await;
    match runtime.hub.package_operation(&operation_id) {
        Ok(operation) => Json(operation).into_response(),
        Err(error) => hub_operation_error(error),
    }
}

#[derive(Clone)]
struct AwakeOwnerWork {
    invocation: QuestAwakeOwnerInvocation,
    serial: String,
    provider: QuestAwakeProviderConfig,
    authorization_expires_at_ms: u64,
}

async fn preview_quest_awake(State(state): State<LocalHubState>, request: Request) -> Response {
    let bytes = match strict_json_body(request, MAX_OPERATION_BYTES, "Quest awake previews").await {
        Ok(bytes) => bytes,
        Err(response) => return response,
    };
    let request = match serde_json::from_slice::<StrictQuestAwakePreviewRequest>(&bytes) {
        Ok(value) => QuestAwakePreviewRequest::from(value),
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_awake_preview_json",
                format!("Quest awake preview is not valid strict JSON: {error}"),
            );
        }
    };
    if request.schema != QUEST_AWAKE_PREVIEW_REQUEST_SCHEMA {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_awake_preview",
            "Quest awake preview schema is not supported",
        );
    }
    if let Err(failures) = request.validate() {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_awake_preview",
            format_contract_failures(&failures),
        );
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let mut runtime = state.runtime.lock().await;
    let Some(provider) = runtime.quest_awake_provider.as_ref() else {
        return api_error(
            StatusCode::NOT_IMPLEMENTED,
            "awake_provider_unavailable",
            "Quest awake provider is disabled until private exact-target configuration is supplied",
        );
    };
    let provider_ready_devices = provider
        .targets
        .iter()
        .map(|target| target.device_id.clone())
        .collect::<BTreeSet<_>>();
    if let Some(existing) = runtime
        .hub
        .quest_awake_operations()
        .into_iter()
        .find(|operation| {
            operation.preview.fleet_revision == runtime.hub.result_revision()
                && operation.preview.expires_at_ms >= now_ms
                && operation.preview.action == request.action
                && operation.preview.duration_ms == request.duration_ms
                && operation.preview.watchdog_interval_ms == request.watchdog_interval_ms
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
    let expires_at_ms = match now_ms.checked_add(AWAKE_OPERATION_PREVIEW_LIFETIME_MS) {
        Some(value) => value,
        None => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "clock_error",
                "Quest awake preview expiry overflowed",
            );
        }
    };
    let (operation_id, preview_id, watchdog_generation) = awake_operation_ids(&request, now_ms);
    let mut candidate_hub = runtime.hub.clone();
    let operation = match candidate_hub.preview_quest_awake(QuestAwakePreviewPlan {
        operation_id,
        preview_id,
        watchdog_generation,
        request,
        created_at_ms: now_ms,
        expires_at_ms,
        provider_ready_devices,
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

async fn execute_quest_awake(
    State(state): State<LocalHubState>,
    AxumPath(operation_id): AxumPath<String>,
    request: Request,
) -> Response {
    let bytes = match strict_json_body(request, MAX_OPERATION_BYTES, "Quest awake execution").await
    {
        Ok(bytes) => bytes,
        Err(response) => return response,
    };
    let execute = match serde_json::from_slice::<QuestAwakeExecuteRequest>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_awake_execute_json",
                format!("Quest awake execution is not valid JSON: {error}"),
            );
        }
    };
    if execute.schema != QUEST_AWAKE_EXECUTE_REQUEST_SCHEMA || execute.operation_id != operation_id
    {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "awake_operation_identity_mismatch",
            "Quest awake operation path, schema, and payload identity must match",
        );
    }
    if let Err(failures) = execute.validate() {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_awake_execute",
            format_contract_failures(&failures),
        );
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let mut runtime = state.runtime.lock().await;
    if runtime.quest_awake_provider.is_none() {
        return api_error(
            StatusCode::NOT_IMPLEMENTED,
            "awake_provider_unavailable",
            "Quest awake provider is disabled until private exact-target configuration is supplied",
        );
    }
    let existing = match runtime.hub.quest_awake_operation(&operation_id) {
        Ok(operation) => operation,
        Err(error) => return hub_operation_error(error),
    };
    if existing.preview.preview_id != execute.preview_id {
        return api_error(
            StatusCode::CONFLICT,
            "awake_preview_conflict",
            "execute request does not bind the immutable Quest awake preview",
        );
    }
    if existing.lifecycle != CommandLifecycle::Proposed {
        return Json(existing).into_response();
    }
    let mut candidate_hub = runtime.hub.clone();
    let operation =
        match candidate_hub.confirm_quest_awake(&operation_id, &execute.preview_id, now_ms) {
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
    drop(runtime);
    schedule_pending_awake_work(state.clone(), &operation_id).await;
    let runtime = state.runtime.lock().await;
    match runtime.hub.quest_awake_operation(&operation_id) {
        Ok(updated) => Json(updated).into_response(),
        Err(_) => Json(operation).into_response(),
    }
}

async fn quest_awake_status(
    State(state): State<LocalHubState>,
    AxumPath(operation_id): AxumPath<String>,
) -> Response {
    let runtime = state.runtime.lock().await;
    match runtime.hub.quest_awake_operation(&operation_id) {
        Ok(operation) => Json(operation).into_response(),
        Err(error) => hub_operation_error(error),
    }
}

#[derive(Clone)]
struct ConnectivityOwnerWork {
    invocation: QuestWifiAdbOwnerInvocation,
    provider: QuestConnectivityProviderConfig,
}

enum ConnectivityOwnerWorkFailure {
    InvocationExpired,
    ProviderInvocation,
}

async fn preview_quest_wifi_adb(State(state): State<LocalHubState>, request: Request) -> Response {
    let bytes =
        match strict_json_body(request, MAX_OPERATION_BYTES, "Quest Wi-Fi ADB previews").await {
            Ok(bytes) => bytes,
            Err(response) => return response,
        };
    let request = match serde_json::from_slice::<StrictQuestWifiAdbPreviewRequest>(&bytes) {
        Ok(value) => QuestWifiAdbPreviewRequest::from(value),
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_wifi_adb_preview_json",
                format!("Quest Wi-Fi ADB preview is not valid strict JSON: {error}"),
            );
        }
    };
    if request.schema != QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_wifi_adb_preview",
            "Quest Wi-Fi ADB preview schema is not supported",
        );
    }
    if let Err(failures) = request.validate() {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_wifi_adb_preview",
            format_contract_failures(&failures),
        );
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let mut runtime = state.runtime.lock().await;
    let Some(provider) = runtime.quest_connectivity_provider.as_ref() else {
        return api_error(
            StatusCode::NOT_IMPLEMENTED,
            "wifi_adb_provider_unavailable",
            "Quest connectivity provider is disabled until private pinned configuration is supplied",
        );
    };
    let provider_ready_devices = provider.targets.iter().cloned().collect::<BTreeSet<_>>();
    if let Some(existing) = runtime
        .hub
        .quest_wifi_adb_operations()
        .into_iter()
        .find(|operation| {
            operation.preview.fleet_revision == runtime.hub.result_revision()
                && operation.preview.expires_at_ms >= now_ms
                && operation.preview.action == request.action
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
    let Some(expires_at_ms) = now_ms.checked_add(WIFI_ADB_OPERATION_PREVIEW_LIFETIME_MS) else {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "clock_error",
            "Quest Wi-Fi ADB preview expiry overflowed",
        );
    };
    let (operation_id, preview_id) = wifi_adb_operation_ids(&request, now_ms);
    let mut candidate_hub = runtime.hub.clone();
    let operation = match candidate_hub.preview_quest_wifi_adb(QuestWifiAdbPreviewPlan {
        operation_id,
        preview_id,
        request,
        created_at_ms: now_ms,
        expires_at_ms,
        provider_ready_devices,
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

async fn execute_quest_wifi_adb(
    State(state): State<LocalHubState>,
    AxumPath(operation_id): AxumPath<String>,
    request: Request,
) -> Response {
    let bytes =
        match strict_json_body(request, MAX_OPERATION_BYTES, "Quest Wi-Fi ADB execution").await {
            Ok(bytes) => bytes,
            Err(response) => return response,
        };
    let execute = match serde_json::from_slice::<QuestWifiAdbExecuteRequest>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_wifi_adb_execute_json",
                format!("Quest Wi-Fi ADB execution is not valid JSON: {error}"),
            );
        }
    };
    if execute.schema != QUEST_WIFI_ADB_EXECUTE_REQUEST_SCHEMA
        || execute.operation_id != operation_id
    {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "wifi_adb_operation_identity_mismatch",
            "Quest Wi-Fi ADB operation path, schema, and payload identity must match",
        );
    }
    if let Err(failures) = execute.validate() {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_wifi_adb_execute",
            format_contract_failures(&failures),
        );
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let mut runtime = state.runtime.lock().await;
    if runtime.quest_connectivity_provider.is_none() {
        return api_error(
            StatusCode::NOT_IMPLEMENTED,
            "wifi_adb_provider_unavailable",
            "Quest connectivity provider is disabled until private pinned configuration is supplied",
        );
    }
    let existing = match runtime.hub.quest_wifi_adb_operation(&operation_id) {
        Ok(operation) => operation,
        Err(error) => return hub_operation_error(error),
    };
    if existing.preview.preview_id != execute.preview_id {
        return api_error(
            StatusCode::CONFLICT,
            "wifi_adb_preview_conflict",
            "execute request does not bind the immutable Quest Wi-Fi ADB preview",
        );
    }
    if existing.lifecycle != CommandLifecycle::Proposed {
        return Json(existing).into_response();
    }
    let mut candidate_hub = runtime.hub.clone();
    let operation =
        match candidate_hub.confirm_quest_wifi_adb(&operation_id, &execute.preview_id, now_ms) {
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
    drop(runtime);
    schedule_pending_connectivity_work(state.clone(), &operation_id).await;
    let runtime = state.runtime.lock().await;
    match runtime
        .adapter
        .quest_wifi_adb_operation(&runtime.hub, &operation_id, now_ms)
    {
        Ok(updated) => Json(updated).into_response(),
        Err(_) => Json(operation).into_response(),
    }
}

async fn quest_wifi_adb_status(
    State(state): State<LocalHubState>,
    AxumPath(operation_id): AxumPath<String>,
) -> Response {
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let runtime = state.runtime.lock().await;
    match runtime
        .adapter
        .quest_wifi_adb_operation(&runtime.hub, &operation_id, now_ms)
    {
        Ok(operation) => Json(operation).into_response(),
        Err(error) if error.contains("not found") => {
            api_error(StatusCode::NOT_FOUND, "operation_not_found", error)
        }
        Err(error) => api_error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_operation", error),
    }
}

async fn preview_windows_hotspot(State(state): State<LocalHubState>, request: Request) -> Response {
    let bytes =
        match strict_json_body(request, MAX_OPERATION_BYTES, "Windows hotspot previews").await {
            Ok(bytes) => bytes,
            Err(response) => return response,
        };
    let request = match serde_json::from_slice::<WindowsHotspotPreviewRequest>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_windows_hotspot_preview_json",
                format!("Windows hotspot preview is not strict JSON: {error}"),
            );
        }
    };
    if request.schema != WINDOWS_HOTSPOT_PREVIEW_REQUEST_SCHEMA {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_windows_hotspot_preview",
            "Windows hotspot preview schema is not supported",
        );
    }
    if let Err(failures) = request.validate() {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_windows_hotspot_preview",
            format_contract_failures(&failures),
        );
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let mut runtime = state.runtime.lock().await;
    let provider_ready = runtime.windows_hotspot_provider.is_some();
    if let Some(existing) = runtime
        .hub
        .windows_hotspot_operations()
        .into_iter()
        .find(|operation| {
            operation.lifecycle == CommandLifecycle::Proposed
                && operation.preview.expires_at_ms >= now_ms
                && operation.preview.action == request.action
        })
    {
        return Json(existing).into_response();
    }
    let expires_at_ms = match now_ms.checked_add(WINDOWS_HOTSPOT_PREVIEW_LIFETIME_MS) {
        Some(value) => value,
        None => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "clock_error",
                "Windows hotspot preview expiry overflowed",
            );
        }
    };
    let (operation_id, preview_id, lease_id, generation) =
        windows_hotspot_operation_ids(&request, now_ms);
    let mut candidate_hub = runtime.hub.clone();
    let operation = match candidate_hub.preview_windows_hotspot(WindowsHotspotPreviewPlan {
        operation_id,
        preview_id,
        lease_id,
        generation,
        request,
        created_at_ms: now_ms,
        expires_at_ms,
        provider_ready,
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

async fn execute_windows_hotspot(
    State(state): State<LocalHubState>,
    AxumPath(operation_id): AxumPath<String>,
    request: Request,
) -> Response {
    let bytes =
        match strict_json_body(request, MAX_OPERATION_BYTES, "Windows hotspot execution").await {
            Ok(bytes) => bytes,
            Err(response) => return response,
        };
    let execute = match serde_json::from_slice::<WindowsHotspotExecuteRequest>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_windows_hotspot_execute_json",
                format!("Windows hotspot execution is not strict JSON: {error}"),
            );
        }
    };
    if execute.schema != WINDOWS_HOTSPOT_EXECUTE_REQUEST_SCHEMA
        || execute.operation_id != operation_id
        || execute.validate().is_err()
    {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "windows_hotspot_operation_identity_mismatch",
            "operation path, schema, and immutable preview identity must match",
        );
    }
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "clock_error", error),
    };
    let (provider, invocation) = {
        let mut runtime = state.runtime.lock().await;
        let Some(configured) = runtime.windows_hotspot_provider.clone() else {
            return api_error(
                StatusCode::NOT_IMPLEMENTED,
                "windows_hotspot_provider_unavailable",
                "Hostess hotspot provider is disabled until private pinned configuration is supplied",
            );
        };
        let mut candidate_hub = runtime.hub.clone();
        candidate_hub.recover_windows_hotspot(now_ms);
        let existing = match candidate_hub.windows_hotspot_operation(&operation_id) {
            Ok(value) => value,
            Err(error) => return hub_operation_error(error),
        };
        if existing.preview.preview_id != execute.preview_id {
            return api_error(
                StatusCode::CONFLICT,
                "windows_hotspot_preview_conflict",
                "execute request does not bind the immutable preview",
            );
        }
        if matches!(
            existing.lifecycle,
            CommandLifecycle::Applied
                | CommandLifecycle::Failed
                | CommandLifecycle::Expired
                | CommandLifecycle::Rejected
        ) {
            return Json(existing).into_response();
        }
        if runtime
            .inflight_windows_hotspot_operations
            .contains(&operation_id)
        {
            return api_error(
                StatusCode::CONFLICT,
                "windows_hotspot_operation_inflight",
                "this host-scoped operation is already invoking Hostess",
            );
        }
        let mut operation = if existing.lifecycle == CommandLifecycle::Proposed {
            match candidate_hub.confirm_windows_hotspot(&operation_id, &execute.preview_id, now_ms)
            {
                Ok(value) => value,
                Err(error) => return hub_operation_error(error),
            }
        } else {
            existing
        };
        let mut candidate_adapter = runtime.adapter.clone();
        if operation.lifecycle == CommandLifecycle::Accepted {
            let request_id = windows_hotspot_provider_request_id(&operation, now_ms);
            operation = match candidate_hub.prepare_windows_hotspot_invocation(
                &operation_id,
                request_id,
                now_ms,
            ) {
                Ok(value) => value,
                Err(error) => return hub_operation_error(error),
            };
            let invocation = operation
                .invocation
                .as_ref()
                .expect("prepared hotspot invocation");
            let lease = operation.lease.as_ref().expect("prepared hotspot lease");
            let manifold_request_id = windows_hotspot_manifold_request_id(
                &operation.operation_id,
                &lease.lease_id,
                &invocation.request_id,
            );
            if let Err(error) = candidate_adapter.authorize_windows_hotspot(
                &WindowsHotspotCommandAuthorization {
                    manifold_request_id,
                    owner_action_request_id: invocation.request_id.clone(),
                    requester_id: "operator.fleet.local".to_owned(),
                    operation_id: operation.operation_id.clone(),
                    preview_id: operation.preview.preview_id.clone(),
                    lease_id: lease.lease_id.clone(),
                    lease_generation: lease.generation.clone(),
                    action: invocation.action,
                    ownership_generation: invocation.ownership_generation.clone(),
                    issued_at_ms: u64::try_from(now_ms).unwrap_or(0),
                    expires_at_ms: u64::try_from(operation.preview.expires_at_ms).unwrap_or(0),
                },
                u64::try_from(now_ms).unwrap_or(0),
            ) {
                return api_error(
                    StatusCode::FORBIDDEN,
                    "windows_hotspot_authorization_rejected",
                    error,
                );
            }
        }
        let invocation = match operation.invocation.clone() {
            Some(value) => value,
            None => {
                return api_error(
                    StatusCode::CONFLICT,
                    "windows_hotspot_invocation_missing",
                    "confirmed operation has no exact provider invocation",
                );
            }
        };
        let RuntimeState {
            hub,
            adapter,
            state_store,
            owner_receipts,
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
        runtime
            .inflight_windows_hotspot_operations
            .insert(operation_id.clone());
        (
            WindowsHotspotProviderConfig {
                executable_path: configured.executable_path,
                executable_sha256: configured.executable_sha256,
                private_stage_root: configured.private_stage_root,
            },
            invocation,
        )
    };
    match spawn_windows_hotspot_owner_work(state, operation_id, provider, invocation, now_ms).await
    {
        Ok(Ok(operation)) => Json(operation).into_response(),
        Ok(Err(error)) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "windows_hotspot_settlement_failed",
            error,
        ),
        Err(_) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "windows_hotspot_settlement_lost",
            "detached hotspot settlement task ended without a durable result",
        ),
    }
}

async fn windows_hotspot_status(
    State(state): State<LocalHubState>,
    AxumPath(operation_id): AxumPath<String>,
) -> Response {
    let runtime = state.runtime.lock().await;
    match runtime.hub.windows_hotspot_operation(&operation_id) {
        Ok(operation) => Json(operation).into_response(),
        Err(error) => api_error(
            StatusCode::NOT_FOUND,
            "operation_not_found",
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

fn package_operation_ids(
    request: &PackageInstallReleasePreviewRequest,
    now_ms: i64,
) -> (String, String) {
    let sequence = OPERATION_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.package.operation.v1\0");
    digest.update(serde_json::to_vec(request).unwrap_or_default());
    digest.update(now_ms.to_le_bytes());
    digest.update(sequence.to_le_bytes());
    let suffix = hex::encode(digest.finalize());
    (
        format!("package-operation-{}", &suffix[..32]),
        format!("package-preview-{}", &suffix[32..64]),
    )
}

fn awake_operation_ids(
    request: &QuestAwakePreviewRequest,
    now_ms: i64,
) -> (String, String, String) {
    let sequence = OPERATION_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.quest-awake.operation.v1\0");
    digest.update(serde_json::to_vec(request).unwrap_or_default());
    digest.update(now_ms.to_le_bytes());
    digest.update(sequence.to_le_bytes());
    let suffix = hex::encode(digest.finalize());
    (
        format!("awake-operation-{}", &suffix[..24]),
        format!("awake-preview-{}", &suffix[24..48]),
        format!("awake-generation-{}", &suffix[48..64]),
    )
}

fn wifi_adb_operation_ids(request: &QuestWifiAdbPreviewRequest, now_ms: i64) -> (String, String) {
    let sequence = OPERATION_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.quest-wifi-adb.operation.v1\0");
    digest.update(serde_json::to_vec(request).unwrap_or_default());
    digest.update(now_ms.to_le_bytes());
    digest.update(sequence.to_le_bytes());
    let suffix = hex::encode(digest.finalize());
    (
        format!("wifi-adb-operation-{}", &suffix[..32]),
        format!("wifi-adb-preview-{}", &suffix[32..64]),
    )
}

fn windows_hotspot_operation_ids(
    request: &WindowsHotspotPreviewRequest,
    now_ms: i64,
) -> (String, String, String, String) {
    let sequence = OPERATION_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.windows-hotspot.operation.v1\0");
    digest.update(serde_json::to_vec(request).unwrap_or_default());
    digest.update(now_ms.to_le_bytes());
    digest.update(sequence.to_le_bytes());
    let suffix = hex::encode(digest.finalize());
    (
        format!("hotspot-operation-{}", &suffix[..24]),
        format!("hotspot-preview-{}", &suffix[16..40]),
        format!("hotspot-lease-{}", &suffix[24..48]),
        format!("hotspot-generation-{}", &suffix[40..64]),
    )
}

fn windows_hotspot_provider_request_id(
    operation: &fleet_contracts::WindowsHotspotOperation,
    now_ms: i64,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.windows-hotspot.provider-request.v1\0");
    digest.update(operation.operation_id.as_bytes());
    digest.update([0]);
    digest.update(operation.preview.preview_id.as_bytes());
    digest.update([0]);
    digest.update(now_ms.to_le_bytes());
    format!("hotspot-request-{}", &hex::encode(digest.finalize())[..32])
}

fn package_owner_action_request_id(operation_id: &str, device_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.package.owner-action.v1\0");
    digest.update(operation_id.as_bytes());
    digest.update([0]);
    digest.update(device_id.as_bytes());
    format!("fleetpkg-{}", &hex::encode(digest.finalize())[..32])
}

fn awake_owner_action_request_id(operation_id: &str, device_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.quest-awake.owner-action.v1\0");
    digest.update(operation_id.as_bytes());
    digest.update([0]);
    digest.update(device_id.as_bytes());
    format!("fleetawake-{}", &hex::encode(digest.finalize())[..32])
}

fn wifi_adb_owner_action_request_id(operation_id: &str, device_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.quest-wifi-adb.owner-action.v1\0");
    digest.update(operation_id.as_bytes());
    digest.update([0]);
    digest.update(device_id.as_bytes());
    format!("fleetwifi-{}", &hex::encode(digest.finalize())[..32])
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
        "package_operation_not_found" | "package_target_not_found" => {
            (StatusCode::NOT_FOUND, "operation_not_found")
        }
        "awake_operation_not_found" | "awake_target_not_found" => {
            (StatusCode::NOT_FOUND, "operation_not_found")
        }
        "wifi_adb_operation_not_found" | "wifi_adb_target_not_found" => {
            (StatusCode::NOT_FOUND, "operation_not_found")
        }
        "windows_hotspot_operation_not_found" => (StatusCode::NOT_FOUND, "operation_not_found"),
        "kiosk_preview_mismatch"
        | "kiosk_operation_id_conflict"
        | "kiosk_preview_expired"
        | "kiosk_target_changed_since_preview"
        | "kiosk_target_identity_changed"
        | "package_preview_mismatch"
        | "package_operation_id_conflict"
        | "package_preview_expired"
        | "package_target_changed_since_preview"
        | "package_target_identity_changed"
        | "awake_preview_conflict"
        | "awake_operation_id_conflict"
        | "awake_preview_expired"
        | "awake_target_identity_changed"
        | "awake_capability_changed"
        | "wifi_adb_preview_conflict"
        | "wifi_adb_operation_id_conflict"
        | "wifi_adb_preview_expired"
        | "wifi_adb_target_identity_changed"
        | "wifi_adb_capability_changed" => (StatusCode::CONFLICT, "operation_conflict"),
        _ => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_operation"),
    };
    api_error(status, code, error.to_string())
}

async fn schedule_pending_connectivity_work(state: LocalHubState, operation_id: &str) {
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(_) => return,
    };
    let now_unsigned = match u64::try_from(now_ms) {
        Ok(value) => value,
        Err(_) => return,
    };
    let mut runtime = state.runtime.lock().await;
    let operation = match runtime.hub.quest_wifi_adb_operation(operation_id) {
        Ok(operation) => operation,
        Err(_) => return,
    };
    let Some(configured) = runtime.quest_connectivity_provider.clone() else {
        return;
    };
    let provider = QuestConnectivityProviderConfig {
        executable_path: configured.executable_path,
        executable_sha256: configured.executable_sha256,
        private_stage_root: configured.private_stage_root,
    };
    let configured_targets = configured.targets.into_iter().collect::<BTreeSet<_>>();
    let slots = connectivity_dispatch_slots(runtime.inflight_connectivity_targets.len());
    let expires_unsigned = match u64::try_from(operation.preview.expires_at_ms) {
        Ok(value) => value,
        Err(_) => return,
    };
    let pending = operation
        .targets
        .iter()
        .filter(|target| target.lifecycle == CommandLifecycle::Accepted)
        .map(|target| (target.device_id.clone(), target.identity_revision))
        .collect::<Vec<_>>();
    let mut candidate_hub = runtime.hub.clone();
    if now_ms >= operation.preview.expires_at_ms {
        for (device_id, _) in pending {
            let _ = candidate_hub.fail_quest_wifi_adb_target(
                operation_id,
                &device_id,
                "provider_invocation_expired".to_owned(),
                now_ms,
            );
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
        }
        return;
    }
    if slots == 0 {
        return;
    }
    let (prepared_hub, candidate_adapter, work) = prepare_pending_connectivity_batch(
        &runtime.hub,
        &runtime.adapter,
        &operation,
        &configured_targets,
        &runtime.inflight_connectivity_targets,
        &provider,
        now_ms,
        now_unsigned,
        expires_unsigned,
    );
    candidate_hub = prepared_hub;
    let RuntimeState {
        hub,
        adapter,
        state_store,
        owner_receipts,
        inflight_connectivity_targets,
        ..
    } = &mut *runtime;
    if state_store
        .persist(&candidate_hub, &candidate_adapter, owner_receipts, now_ms)
        .is_err()
    {
        return;
    }
    *hub = candidate_hub;
    *adapter = candidate_adapter;
    for item in &work {
        inflight_connectivity_targets.insert((
            item.invocation.operation_id.clone(),
            item.invocation.device_id.clone(),
        ));
    }
    drop(runtime);
    for item in work {
        spawn_connectivity_owner_work(state.clone(), item);
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_pending_connectivity_batch(
    hub: &FleetHub,
    adapter: &FleetManifoldAdapter,
    operation: &QuestWifiAdbOperation,
    configured_targets: &BTreeSet<String>,
    inflight_targets: &BTreeSet<(String, String)>,
    provider: &QuestConnectivityProviderConfig,
    now_ms: i64,
    now_unsigned: u64,
    expires_unsigned: u64,
) -> (FleetHub, FleetManifoldAdapter, Vec<ConnectivityOwnerWork>) {
    let mut candidate_hub = hub.clone();
    let mut candidate_adapter = adapter.clone();
    let slots = connectivity_dispatch_slots(inflight_targets.len());
    let pending = operation
        .targets
        .iter()
        .filter(|target| target.lifecycle == CommandLifecycle::Accepted)
        .map(|target| (target.device_id.clone(), target.identity_revision));
    let mut work = Vec::new();
    for (device_id, identity_revision) in pending {
        if work.len() >= slots {
            break;
        }
        if !configured_targets.contains(&device_id) {
            let _ = candidate_hub.fail_quest_wifi_adb_target(
                &operation.operation_id,
                &device_id,
                "provider_target_unconfigured".to_owned(),
                now_ms,
            );
            continue;
        }
        if inflight_targets
            .iter()
            .any(|(_, candidate_device)| candidate_device == &device_id)
        {
            let _ = candidate_hub.fail_quest_wifi_adb_target(
                &operation.operation_id,
                &device_id,
                "connectivity_control_conflict".to_owned(),
                now_ms,
            );
            continue;
        }
        let owner_action_request_id =
            wifi_adb_owner_action_request_id(&operation.operation_id, &device_id);
        let manifold_request_id = quest_wifi_adb_manifold_request_id(
            &operation.operation_id,
            &device_id,
            &owner_action_request_id,
        );
        if candidate_adapter
            .authorize_quest_wifi_adb(
                &QuestWifiAdbCommandAuthorization {
                    manifold_request_id,
                    owner_action_request_id: owner_action_request_id.clone(),
                    requester_id: "operator.fleet.local".to_owned(),
                    operation_id: operation.operation_id.clone(),
                    preview_id: operation.preview.preview_id.clone(),
                    device_id: device_id.clone(),
                    identity_revision,
                    action: operation.preview.action,
                    issued_at_ms: now_unsigned,
                    expires_at_ms: expires_unsigned,
                },
                now_unsigned,
            )
            .is_err()
        {
            let _ = candidate_hub.fail_quest_wifi_adb_target(
                &operation.operation_id,
                &device_id,
                "manifold_command_rejected".to_owned(),
                now_ms,
            );
            continue;
        }
        let updated = match candidate_hub.prepare_quest_wifi_adb_invocation(
            &operation.operation_id,
            &device_id,
            owner_action_request_id,
            now_ms,
        ) {
            Ok(updated) => updated,
            Err(_) => continue,
        };
        let Some(invocation) = updated
            .targets
            .iter()
            .find(|target| target.device_id == device_id)
            .and_then(|target| target.invocation.clone())
        else {
            continue;
        };
        if candidate_hub
            .mark_quest_wifi_adb_dispatched(&operation.operation_id, &device_id, now_ms)
            .is_err()
        {
            continue;
        }
        work.push(ConnectivityOwnerWork {
            invocation,
            provider: provider.clone(),
        });
    }
    (candidate_hub, candidate_adapter, work)
}

fn spawn_connectivity_owner_work(state: LocalHubState, work: ConnectivityOwnerWork) {
    tokio::spawn(async move {
        let operation_id = work.invocation.operation_id.clone();
        let device_id = work.invocation.device_id.clone();
        let (device_slot, provider_slots, adapter) = {
            let runtime = state.runtime.lock().await;
            (
                runtime.connectivity_device_slots.get(&device_id).cloned(),
                Arc::clone(&runtime.connectivity_provider_slots),
                runtime.quest_connectivity_adapter.clone(),
            )
        };
        let result = match device_slot {
            Some(device_slot) => match device_slot.try_acquire_owned() {
                Ok(_device_permit) => match provider_slots.try_acquire_owned() {
                    Ok(_provider_permit) => {
                        let provider = work.provider.clone();
                        let invocation = work.invocation.clone();
                        tokio::task::spawn_blocking(move || {
                            let invocation_started_at_ms = unix_time_ms()
                                .map_err(|_| ConnectivityOwnerWorkFailure::ProviderInvocation)?;
                            if connectivity_invocation_expired(
                                &invocation,
                                invocation_started_at_ms,
                            ) {
                                return Err(ConnectivityOwnerWorkFailure::InvocationExpired);
                            }
                            adapter
                                .invoke(&provider, &invocation)
                                .map_err(|_| ConnectivityOwnerWorkFailure::ProviderInvocation)
                        })
                        .await
                        .map_err(|_| ConnectivityOwnerWorkFailure::ProviderInvocation)
                        .and_then(|result| result)
                    }
                    Err(_) => Err(ConnectivityOwnerWorkFailure::ProviderInvocation),
                },
                Err(_) => Err(ConnectivityOwnerWorkFailure::ProviderInvocation),
            },
            None => Err(ConnectivityOwnerWorkFailure::ProviderInvocation),
        };
        let now_ms = unix_time_ms().unwrap_or(work.invocation.issued_at_ms);
        let mut runtime = state.runtime.lock().await;
        let mut candidate_hub = runtime.hub.clone();
        let settled = settle_connectivity_owner_result(
            &mut candidate_hub,
            &operation_id,
            &device_id,
            result,
            now_ms,
        )
        .is_ok();
        let RuntimeState {
            hub,
            adapter,
            state_store,
            owner_receipts,
            inflight_connectivity_targets,
            ..
        } = &mut *runtime;
        let persisted = settled
            && state_store
                .persist(&candidate_hub, adapter, owner_receipts, now_ms)
                .is_ok();
        if persisted {
            *hub = candidate_hub;
        }
        inflight_connectivity_targets.remove(&(operation_id, device_id));
        drop(runtime);
        if persisted {
            let _ = schedule_recovered_connectivity_work(state).await;
        }
    });
}

fn settle_connectivity_owner_result(
    hub: &mut FleetHub,
    operation_id: &str,
    device_id: &str,
    result: Result<QuestWifiAdbOwnerReceipt, ConnectivityOwnerWorkFailure>,
    now_ms: i64,
) -> Result<(), String> {
    let failure_code = match result {
        Ok(receipt) => match hub.apply_quest_wifi_adb_receipt(receipt, now_ms) {
            Ok(_) => return Ok(()),
            Err(_) => "provider_receipt_rejected",
        },
        Err(ConnectivityOwnerWorkFailure::InvocationExpired) => "provider_invocation_expired",
        Err(ConnectivityOwnerWorkFailure::ProviderInvocation) => "provider_invocation_failed",
    };
    hub.fail_quest_wifi_adb_target(operation_id, device_id, failure_code.to_owned(), now_ms)
        .map(|_| ())
        .map_err(|error| {
            format!(
                "Quest connectivity terminal settlement failed with code {}",
                error.code
            )
        })
}

fn connectivity_dispatch_slots(inflight: usize) -> usize {
    MAX_CONCURRENT_CONNECTIVITY_PROVIDER_CALLS.saturating_sub(inflight)
}

fn connectivity_invocation_expired(invocation: &QuestWifiAdbOwnerInvocation, now_ms: i64) -> bool {
    now_ms >= invocation.expires_at_ms
}

async fn schedule_recovered_connectivity_work(state: LocalHubState) -> Result<(), String> {
    let now_ms = unix_time_ms()?;
    let mut recovered = Vec::new();
    {
        let mut runtime = state.runtime.lock().await;
        let Some(configured) = runtime.quest_connectivity_provider.clone() else {
            return Ok(());
        };
        let provider = QuestConnectivityProviderConfig {
            executable_path: configured.executable_path,
            executable_sha256: configured.executable_sha256,
            private_stage_root: configured.private_stage_root,
        };
        let configured_targets = configured.targets.into_iter().collect::<BTreeSet<_>>();
        let operations = runtime.hub.quest_wifi_adb_operations();
        let mut candidate_hub = runtime.hub.clone();
        let mut changed = false;
        let mut remaining_slots =
            connectivity_dispatch_slots(runtime.inflight_connectivity_targets.len());
        let mut reserved = runtime.inflight_connectivity_targets.clone();
        for operation in operations {
            for target in operation
                .targets
                .into_iter()
                .filter(|target| target.lifecycle == CommandLifecycle::Dispatched)
            {
                let Some(invocation) = target.invocation else {
                    continue;
                };
                if connectivity_invocation_expired(&invocation, now_ms) {
                    candidate_hub
                        .fail_quest_wifi_adb_target(
                            &operation.operation_id,
                            &target.device_id,
                            "provider_invocation_expired".to_owned(),
                            now_ms,
                        )
                        .map_err(|error| error.to_string())?;
                    changed = true;
                    continue;
                }
                if !configured_targets.contains(&target.device_id) {
                    candidate_hub
                        .fail_quest_wifi_adb_target(
                            &operation.operation_id,
                            &target.device_id,
                            "provider_target_unconfigured".to_owned(),
                            now_ms,
                        )
                        .map_err(|error| error.to_string())?;
                    changed = true;
                    continue;
                }
                if remaining_slots == 0
                    || reserved
                        .iter()
                        .any(|(_, candidate_device)| candidate_device == &target.device_id)
                {
                    continue;
                }
                let key = (operation.operation_id.clone(), target.device_id.clone());
                reserved.insert(key.clone());
                recovered.push((
                    key,
                    ConnectivityOwnerWork {
                        invocation,
                        provider: provider.clone(),
                    },
                ));
                remaining_slots -= 1;
            }
        }
        if changed {
            let RuntimeState {
                hub,
                adapter,
                state_store,
                owner_receipts,
                ..
            } = &mut *runtime;
            state_store.persist(&candidate_hub, adapter, owner_receipts, now_ms)?;
            *hub = candidate_hub;
        }
        for (key, _) in &recovered {
            runtime.inflight_connectivity_targets.insert(key.clone());
        }
    }
    for (_, work) in recovered {
        spawn_connectivity_owner_work(state.clone(), work);
    }
    let pending_operation_ids = {
        let runtime = state.runtime.lock().await;
        runtime
            .hub
            .quest_wifi_adb_operations()
            .into_iter()
            .filter(|operation| {
                operation
                    .targets
                    .iter()
                    .any(|target| target.lifecycle == CommandLifecycle::Accepted)
            })
            .map(|operation| operation.operation_id)
            .collect::<Vec<_>>()
    };
    for operation_id in pending_operation_ids {
        schedule_pending_connectivity_work(state.clone(), &operation_id).await;
    }
    Ok(())
}

async fn schedule_recovered_windows_hotspot_work(state: LocalHubState) -> Result<(), String> {
    let now_ms = unix_time_ms()?;
    let work = {
        let mut runtime = state.runtime.lock().await;
        let mut candidate_hub = runtime.hub.clone();
        candidate_hub.recover_windows_hotspot(now_ms);
        let configured = runtime.windows_hotspot_provider.clone();
        let work = configured.and_then(|configured| {
            candidate_hub
                .windows_hotspot_operations()
                .into_iter()
                .find(|operation| {
                    operation.lifecycle == CommandLifecycle::Dispatched
                        && operation.preview.expires_at_ms >= now_ms
                        && operation.invocation.is_some()
                })
                .and_then(|operation| {
                    operation.invocation.map(|invocation| {
                        (
                            operation.operation_id,
                            WindowsHotspotProviderConfig {
                                executable_path: configured.executable_path,
                                executable_sha256: configured.executable_sha256,
                                private_stage_root: configured.private_stage_root,
                            },
                            invocation,
                        )
                    })
                })
        });
        let RuntimeState {
            hub,
            adapter,
            state_store,
            owner_receipts,
            ..
        } = &mut *runtime;
        state_store.persist(&candidate_hub, adapter, owner_receipts, now_ms)?;
        *hub = candidate_hub;
        if let Some((operation_id, _, _)) = &work {
            runtime
                .inflight_windows_hotspot_operations
                .insert(operation_id.clone());
        }
        work
    };
    if let Some((operation_id, provider, invocation)) = work {
        drop(spawn_windows_hotspot_owner_work(
            state,
            operation_id,
            provider,
            invocation,
            now_ms,
        ));
    }
    Ok(())
}

fn spawn_windows_hotspot_owner_work(
    state: LocalHubState,
    operation_id: String,
    provider: WindowsHotspotProviderConfig,
    invocation: fleet_contracts::WindowsHotspotProviderRequest,
    fallback_now_ms: i64,
) -> oneshot::Receiver<Result<fleet_contracts::WindowsHotspotOperation, String>> {
    spawn_windows_hotspot_owner_work_with_clock(
        state,
        operation_id,
        provider,
        invocation,
        fallback_now_ms,
        unix_time_ms,
    )
}

fn spawn_windows_hotspot_owner_work_with_clock<C>(
    state: LocalHubState,
    operation_id: String,
    provider: WindowsHotspotProviderConfig,
    invocation: fleet_contracts::WindowsHotspotProviderRequest,
    fallback_now_ms: i64,
    settlement_clock: C,
) -> oneshot::Receiver<Result<fleet_contracts::WindowsHotspotOperation, String>>
where
    C: FnOnce() -> Result<i64, String> + Send + 'static,
{
    let (sender, receiver) = oneshot::channel();
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            WindowsHotspotOwnerAdapter::default().invoke(&provider, &invocation)
        })
        .await;
        let now_ms = settlement_clock().unwrap_or(fallback_now_ms);
        let mut runtime = state.runtime.lock().await;
        runtime
            .inflight_windows_hotspot_operations
            .remove(&operation_id);
        let mut candidate_hub = runtime.hub.clone();
        let transition = match result {
            Ok(Ok(receipt)) => match candidate_hub.apply_windows_hotspot_receipt(receipt, now_ms) {
                Ok(operation) => Ok(operation),
                Err(_) => candidate_hub.fail_windows_hotspot_operation(
                    &operation_id,
                    "provider_receipt_rejected".to_owned(),
                    now_ms,
                ),
            },
            Ok(Err(error)) => candidate_hub.fail_windows_hotspot_operation(
                &operation_id,
                format!("provider_{}", error.code),
                now_ms,
            ),
            Err(_) => candidate_hub.fail_windows_hotspot_operation(
                &operation_id,
                "provider_worker_failed".to_owned(),
                now_ms,
            ),
        };
        let operation = match transition {
            Ok(operation) => operation,
            Err(error) => {
                let _ = sender.send(Err(format!(
                    "hotspot settlement transition failed with code {}",
                    error.code
                )));
                return;
            }
        };
        let RuntimeState {
            hub,
            adapter,
            state_store,
            owner_receipts,
            ..
        } = &mut *runtime;
        if let Err(error) = state_store.persist(&candidate_hub, adapter, owner_receipts, now_ms) {
            let _ = sender.send(Err(error));
            return;
        }
        *hub = candidate_hub;
        let _ = sender.send(Ok(operation));
    });
    receiver
}

async fn schedule_pending_awake_work(state: LocalHubState, operation_id: &str) {
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(_) => return,
    };
    let now_unsigned = match u64::try_from(now_ms) {
        Ok(value) => value,
        Err(_) => return,
    };
    let mut runtime = state.runtime.lock().await;
    let operation = match runtime.hub.quest_awake_operation(operation_id) {
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
    let slots = 8usize.saturating_sub(
        runtime
            .inflight_awake_targets
            .iter()
            .filter(|(candidate_operation, _)| candidate_operation == operation_id)
            .count(),
    );
    if slots == 0 {
        return;
    }
    let Some(configured) = runtime.quest_awake_provider.clone() else {
        return;
    };
    let provider = QuestAwakeProviderConfig {
        executable_path: configured.executable_path.clone(),
        executable_sha256: configured.executable_sha256.clone(),
        adb_executable_path: configured.adb_executable_path.clone(),
        adb_executable_sha256: configured.adb_executable_sha256.clone(),
        adb_support_artifacts: configured
            .adb_support_artifacts
            .iter()
            .map(|artifact| QuestAwakePinnedArtifact {
                source_path: artifact.source_path.clone(),
                sha256: artifact.sha256.clone(),
            })
            .collect(),
        private_stage_root: configured.private_stage_root.clone(),
    };
    let serials = configured
        .targets
        .into_iter()
        .map(|target| (target.device_id, target.serial))
        .collect::<BTreeMap<_, _>>();
    let expires_unsigned = match u64::try_from(operation.preview.expires_at_ms) {
        Ok(value) => value,
        Err(_) => return,
    };
    let mut candidate_hub = runtime.hub.clone();
    let mut candidate_adapter = runtime.adapter.clone();
    let mut work = Vec::new();
    let mut controls = Vec::new();
    for (device_id, identity_revision) in pending.into_iter().take(slots) {
        let Some(serial) = serials.get(&device_id).cloned() else {
            let _ = candidate_hub.fail_quest_awake_target(
                operation_id,
                &device_id,
                "provider_target_unconfigured".to_owned(),
                now_ms,
            );
            continue;
        };
        let active_windows_watchdog = runtime
            .windows_awake_watchdogs
            .get(&device_id)
            .is_some_and(|control| !control.cancel.load(Ordering::Acquire));
        let device_has_inflight_awake_work = runtime
            .inflight_awake_targets
            .iter()
            .any(|(_, candidate_device)| candidate_device == &device_id);
        let may_supersede_windows_watchdog = matches!(
            operation.preview.action,
            QuestAwakeAction::StopWatchdogs | QuestAwakeAction::RestoreNormal
        ) && active_windows_watchdog;
        if (active_windows_watchdog
            && !matches!(
                operation.preview.action,
                QuestAwakeAction::StopWatchdogs | QuestAwakeAction::RestoreNormal
            ))
            || (device_has_inflight_awake_work && !may_supersede_windows_watchdog)
        {
            let _ = candidate_hub.fail_quest_awake_target(
                operation_id,
                &device_id,
                "awake_control_conflict".to_owned(),
                now_ms,
            );
            continue;
        }
        let owner_action_request_id = awake_owner_action_request_id(operation_id, &device_id);
        let (watchdog_generation, watchdog_generation_override) = if matches!(
            operation.preview.action,
            QuestAwakeAction::StopWatchdogs | QuestAwakeAction::RestoreNormal
        ) {
            let generation = latest_reported_device_watchdog_generation(&runtime.hub, &device_id)
                .unwrap_or_else(|| "no-known-device-watchdog".to_owned());
            (generation.clone(), Some(generation))
        } else {
            (operation.preview.watchdog_generation.clone(), None)
        };
        let manifold_request_id =
            quest_awake_manifold_request_id(operation_id, &device_id, &owner_action_request_id);
        if candidate_adapter
            .authorize_quest_awake(
                &QuestAwakeCommandAuthorization {
                    manifold_request_id,
                    owner_action_request_id: owner_action_request_id.clone(),
                    requester_id: "operator.fleet.local".to_owned(),
                    operation_id: operation_id.to_owned(),
                    preview_id: operation.preview.preview_id.clone(),
                    device_id: device_id.clone(),
                    identity_revision,
                    action: operation.preview.action,
                    duration_ms: operation.preview.duration_ms,
                    watchdog_interval_ms: operation.preview.watchdog_interval_ms,
                    watchdog_generation,
                    issued_at_ms: now_unsigned,
                    expires_at_ms: expires_unsigned,
                },
                now_unsigned,
            )
            .is_err()
        {
            let _ = candidate_hub.fail_quest_awake_target(
                operation_id,
                &device_id,
                "manifold_command_rejected".to_owned(),
                now_ms,
            );
            continue;
        }
        let updated = match candidate_hub.prepare_quest_awake_invocation_with_watchdog_generation(
            operation_id,
            &device_id,
            owner_action_request_id,
            watchdog_generation_override,
            now_ms,
        ) {
            Ok(updated) => updated,
            Err(_) => continue,
        };
        let invocation = match updated
            .targets
            .iter()
            .find(|target| target.device_id == device_id)
            .and_then(|target| target.invocation.clone())
        {
            Some(invocation) => invocation,
            None => continue,
        };
        if candidate_hub
            .mark_quest_awake_dispatched(operation_id, &device_id, now_ms)
            .is_err()
        {
            continue;
        }
        if operation.preview.action == QuestAwakeAction::StartWindowsWatchdog {
            let control = WindowsAwakeWatchdogControl {
                generation: operation.preview.watchdog_generation.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                running: Arc::new(AtomicBool::new(true)),
            };
            controls.push((device_id.clone(), control));
        }
        work.push(AwakeOwnerWork {
            invocation,
            serial,
            provider: provider.clone(),
            authorization_expires_at_ms: expires_unsigned,
        });
    }
    let RuntimeState {
        hub,
        adapter,
        state_store,
        owner_receipts,
        inflight_awake_targets,
        windows_awake_watchdogs,
        ..
    } = &mut *runtime;
    if state_store
        .persist(&candidate_hub, &candidate_adapter, owner_receipts, now_ms)
        .is_err()
    {
        return;
    }
    *hub = candidate_hub;
    *adapter = candidate_adapter;
    for item in &work {
        inflight_awake_targets.insert((
            item.invocation.operation_id.clone(),
            item.invocation.device_id.clone(),
        ));
    }
    for (device_id, control) in &controls {
        windows_awake_watchdogs.insert(device_id.clone(), control.clone());
    }
    drop(runtime);
    for item in work {
        if item.invocation.action == QuestAwakeAction::StartWindowsWatchdog {
            spawn_windows_awake_watchdog(state.clone(), item);
        } else {
            spawn_awake_owner_work(state.clone(), item);
        }
    }
}

fn latest_reported_device_watchdog_generation(hub: &FleetHub, device_id: &str) -> Option<String> {
    hub.quest_awake_operations()
        .into_iter()
        .flat_map(|operation| operation.targets)
        .filter(|target| target.device_id == device_id)
        .filter_map(|target| target.receipt)
        .filter(|receipt| {
            receipt.device_watchdog.reported_active
                && !receipt.device_watchdog.generation.is_empty()
        })
        .max_by_key(|receipt| receipt.observed_at_ms)
        .map(|receipt| receipt.device_watchdog.generation)
}

fn spawn_awake_owner_work(state: LocalHubState, work: AwakeOwnerWork) {
    tokio::spawn(async move {
        let operation_id = work.invocation.operation_id.clone();
        let device_id = work.invocation.device_id.clone();
        if matches!(
            work.invocation.action,
            QuestAwakeAction::StopWatchdogs | QuestAwakeAction::RestoreNormal
        ) && !stop_windows_awake_watchdog(&state, &device_id).await
        {
            commit_awake_owner_result(
                &state,
                &work,
                Err("Windows awake watchdog did not stop before the safety deadline".to_owned()),
            )
            .await;
            let mut runtime = state.runtime.lock().await;
            runtime
                .inflight_awake_targets
                .remove(&(operation_id.clone(), device_id));
            drop(runtime);
            schedule_pending_awake_work(state.clone(), &operation_id).await;
            return;
        }
        let adapter = {
            let runtime = state.runtime.lock().await;
            runtime.quest_awake_adapter.clone()
        };
        let result = match acquire_awake_provider_slots(&state, &work.invocation.device_id).await {
            Ok(_permits) => {
                let invocation = work.invocation.clone();
                let serial = work.serial.clone();
                let provider = work.provider.clone();
                tokio::task::spawn_blocking(move || {
                    adapter.invoke(&provider, &invocation, &serial, false)
                })
                .await
                .map_err(|error| format!("Quest awake provider worker could not join: {error}"))
                .and_then(|result| result.map_err(|error| error.to_string()))
            }
            Err(error) => Err(error),
        };
        commit_awake_owner_result(&state, &work, result).await;
        let mut runtime = state.runtime.lock().await;
        runtime
            .inflight_awake_targets
            .remove(&(operation_id.clone(), device_id));
        drop(runtime);
        schedule_pending_awake_work(state.clone(), &operation_id).await;
    });
}

fn spawn_windows_awake_watchdog(state: LocalHubState, work: AwakeOwnerWork) {
    tokio::spawn(async move {
        let operation_id = work.invocation.operation_id.clone();
        let device_id = work.invocation.device_id.clone();
        let generation = work.invocation.watchdog_generation.clone();
        let control = {
            let runtime = state.runtime.lock().await;
            runtime.windows_awake_watchdogs.get(&device_id).cloned()
        };
        let Some(control) = control else {
            return;
        };
        let mut consecutive_failures = 0u8;
        let mut authorization_expires_at_ms = work.authorization_expires_at_ms;
        loop {
            if control.cancel.load(Ordering::Acquire) {
                break;
            }
            let now_ms = match unix_time_ms().and_then(|value| {
                u64::try_from(value).map_err(|_| "current time is negative".to_owned())
            }) {
                Ok(value) => value,
                Err(error) => {
                    commit_awake_owner_result(&state, &work, Err(error)).await;
                    break;
                }
            };
            if authorization_expires_at_ms
                <= now_ms.saturating_add(AWAKE_WATCHDOG_AUTHORIZATION_RENEWAL_MARGIN_MS)
            {
                match renew_windows_awake_authorization(&state, &work, now_ms).await {
                    Ok(expires_at_ms) => authorization_expires_at_ms = expires_at_ms,
                    Err(error) => {
                        commit_awake_owner_result(&state, &work, Err(error)).await;
                        break;
                    }
                }
            }
            let adapter = {
                let runtime = state.runtime.lock().await;
                runtime.quest_awake_adapter.clone()
            };
            let Some(_permits) = acquire_awake_provider_slots_until_cancelled(
                &state,
                &work.invocation.device_id,
                &control.cancel,
            )
            .await
            else {
                break;
            };
            let invocation = work.invocation.clone();
            let serial = work.serial.clone();
            let provider = work.provider.clone();
            let result = tokio::task::spawn_blocking(move || {
                adapter.invoke(&provider, &invocation, &serial, true)
            })
            .await
            .map_err(|error| format!("Windows awake watchdog worker could not join: {error}"))
            .and_then(|result| result.map_err(|error| error.to_string()));
            drop(_permits);
            if result.is_ok() {
                consecutive_failures = 0;
                commit_awake_owner_result(&state, &work, result).await;
            } else {
                consecutive_failures = consecutive_failures.saturating_add(1);
                if consecutive_failures >= 3 {
                    commit_awake_owner_result(&state, &work, result).await;
                }
            }
            if consecutive_failures >= 3 {
                break;
            }
            let interval = Duration::from_millis(u64::from(work.invocation.watchdog_interval_ms));
            let mut elapsed = Duration::ZERO;
            while elapsed < interval && !control.cancel.load(Ordering::Acquire) {
                let step = (interval - elapsed).min(Duration::from_millis(100));
                tokio::time::sleep(step).await;
                elapsed += step;
            }
        }
        control.running.store(false, Ordering::Release);
        let mut runtime = state.runtime.lock().await;
        if runtime
            .windows_awake_watchdogs
            .get(&device_id)
            .is_some_and(|current| current.generation == generation)
        {
            runtime.windows_awake_watchdogs.remove(&device_id);
        }
        runtime
            .inflight_awake_targets
            .remove(&(operation_id, device_id));
    });
}

async fn commit_awake_owner_result(
    state: &LocalHubState,
    work: &AwakeOwnerWork,
    result: Result<fleet_contracts::QuestAwakeOwnerReceipt, String>,
) {
    let now_ms = match unix_time_ms() {
        Ok(value) => value,
        Err(_) => return,
    };
    let mut runtime = state.runtime.lock().await;
    let mut candidate_hub = runtime.hub.clone();
    let transition = match result {
        Ok(receipt) => candidate_hub.apply_quest_awake_receipt(receipt, now_ms),
        Err(error) => candidate_hub.fail_quest_awake_target(
            &work.invocation.operation_id,
            &work.invocation.device_id,
            format!("provider_failed_{}", short_error_digest(&error)),
            now_ms,
        ),
    };
    if transition.is_err() {
        return;
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
    }
}

async fn renew_windows_awake_authorization(
    state: &LocalHubState,
    work: &AwakeOwnerWork,
    now_ms: u64,
) -> Result<u64, String> {
    let expires_at_ms = now_ms
        .checked_add(AWAKE_WATCHDOG_AUTHORIZATION_LIFETIME_MS)
        .ok_or_else(|| "Windows awake authorization expiry overflowed".to_owned())?;
    let sequence = AWAKE_AUTHORIZATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let owner_action_request_id =
        awake_authorization_renewal_id(&work.invocation, now_ms, sequence);
    let manifold_request_id = quest_awake_manifold_request_id(
        &work.invocation.operation_id,
        &work.invocation.device_id,
        &owner_action_request_id,
    );
    let mut runtime = state.runtime.lock().await;
    let mut candidate_adapter = runtime.adapter.clone();
    candidate_adapter.authorize_quest_awake(
        &QuestAwakeCommandAuthorization {
            manifold_request_id,
            owner_action_request_id,
            requester_id: "operator.fleet.local".to_owned(),
            operation_id: work.invocation.operation_id.clone(),
            preview_id: work.invocation.preview_id.clone(),
            device_id: work.invocation.device_id.clone(),
            identity_revision: work.invocation.identity_revision,
            action: work.invocation.action,
            duration_ms: work.invocation.duration_ms,
            watchdog_interval_ms: work.invocation.watchdog_interval_ms,
            watchdog_generation: work.invocation.watchdog_generation.clone(),
            issued_at_ms: now_ms,
            expires_at_ms,
        },
        now_ms,
    )?;
    let candidate_hub = runtime.hub.clone();
    let RuntimeState {
        adapter,
        state_store,
        owner_receipts,
        ..
    } = &mut *runtime;
    state_store
        .persist(
            &candidate_hub,
            &candidate_adapter,
            owner_receipts,
            i64::try_from(now_ms)
                .map_err(|_| "Windows awake authorization time is not representable".to_owned())?,
        )
        .map_err(|error| format!("cannot persist renewed Windows awake authorization: {error}"))?;
    *adapter = candidate_adapter;
    Ok(expires_at_ms)
}

fn awake_authorization_renewal_id(
    invocation: &QuestAwakeOwnerInvocation,
    now_ms: u64,
    sequence: u64,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"rusty.fleet.quest-awake.authorization-renewal.v1\0");
    for value in [
        invocation.request_id.as_bytes(),
        invocation.watchdog_generation.as_bytes(),
        &now_ms.to_be_bytes(),
        &sequence.to_be_bytes(),
    ] {
        digest.update(value);
        digest.update([0]);
    }
    format!(
        "awake-authorization-renewal-{}",
        &hex::encode(digest.finalize())[..32]
    )
}

async fn stop_windows_awake_watchdog(state: &LocalHubState, device_id: &str) -> bool {
    let control = {
        let runtime = state.runtime.lock().await;
        runtime.windows_awake_watchdogs.get(device_id).cloned()
    };
    let Some(control) = control else {
        return true;
    };
    control.cancel.store(true, Ordering::Release);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(35);
    while control.running.load(Ordering::Acquire) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    !control.running.load(Ordering::Acquire)
}

async fn acquire_awake_provider_slots(
    state: &LocalHubState,
    device_id: &str,
) -> Result<(OwnedSemaphorePermit, OwnedSemaphorePermit), String> {
    let (device_slot, provider_slots) = {
        let runtime = state.runtime.lock().await;
        let device_slot = runtime
            .awake_device_slots
            .get(device_id)
            .cloned()
            .ok_or_else(|| "Quest awake target has no configured device gate".to_owned())?;
        (device_slot, Arc::clone(&runtime.awake_provider_slots))
    };
    let device_permit = device_slot
        .acquire_owned()
        .await
        .map_err(|_| "Quest awake device gate is closed".to_owned())?;
    let provider_permit = provider_slots
        .acquire_owned()
        .await
        .map_err(|_| "Quest awake provider concurrency gate is closed".to_owned())?;
    Ok((device_permit, provider_permit))
}

async fn acquire_awake_provider_slots_until_cancelled(
    state: &LocalHubState,
    device_id: &str,
    cancel: &Arc<AtomicBool>,
) -> Option<(OwnedSemaphorePermit, OwnedSemaphorePermit)> {
    let (device_slot, provider_slots) = {
        let runtime = state.runtime.lock().await;
        (
            runtime.awake_device_slots.get(device_id)?.clone(),
            Arc::clone(&runtime.awake_provider_slots),
        )
    };
    let device_permit = acquire_slot_until_cancelled(device_slot, cancel).await?;
    let provider_permit = acquire_slot_until_cancelled(provider_slots, cancel).await?;
    Some((device_permit, provider_permit))
}

async fn acquire_slot_until_cancelled(
    slot: Arc<Semaphore>,
    cancel: &Arc<AtomicBool>,
) -> Option<OwnedSemaphorePermit> {
    tokio::select! {
        permit = slot.acquire_owned() => permit.ok(),
        () = wait_for_awake_cancellation(cancel) => None,
    }
}

async fn wait_for_awake_cancellation(cancel: &Arc<AtomicBool>) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn short_error_digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))[..16].to_owned()
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

async fn schedule_recovered_awake_work(state: LocalHubState) -> Result<(), String> {
    let now_ms = unix_time_ms()?;
    let mut runtime = state.runtime.lock().await;
    let operations = runtime.hub.quest_awake_operations();
    let recoverable_exists = operations.iter().any(|operation| {
        operation.confirmed_at_ms.is_some()
            && operation.targets.iter().any(|target| {
                matches!(
                    target.lifecycle,
                    CommandLifecycle::Accepted
                        | CommandLifecycle::Dispatched
                        | CommandLifecycle::Running
                ) || (operation.preview.action == QuestAwakeAction::StartWindowsWatchdog
                    && target.lifecycle == CommandLifecycle::Applied
                    && target
                        .receipt
                        .as_ref()
                        .is_some_and(|receipt| receipt.effective))
            })
    });
    if !recoverable_exists {
        return Ok(());
    }
    let configured = runtime.quest_awake_provider.clone().ok_or_else(|| {
        "recovered Quest awake work requires private provider configuration".to_owned()
    })?;
    let provider = QuestAwakeProviderConfig {
        executable_path: configured.executable_path,
        executable_sha256: configured.executable_sha256,
        adb_executable_path: configured.adb_executable_path,
        adb_executable_sha256: configured.adb_executable_sha256,
        adb_support_artifacts: configured
            .adb_support_artifacts
            .into_iter()
            .map(|artifact| QuestAwakePinnedArtifact {
                source_path: artifact.source_path,
                sha256: artifact.sha256,
            })
            .collect(),
        private_stage_root: configured.private_stage_root,
    };
    let serials = configured
        .targets
        .into_iter()
        .map(|target| (target.device_id, target.serial))
        .collect::<BTreeMap<_, _>>();
    let mut latest_mutating_intent = BTreeMap::<String, (i64, String)>::new();
    for operation in &operations {
        let Some(confirmed_at_ms) = operation.confirmed_at_ms else {
            continue;
        };
        if operation.preview.action == QuestAwakeAction::Status {
            continue;
        }
        for target in operation
            .targets
            .iter()
            .filter(|target| target.preflight.eligible)
        {
            let candidate = (confirmed_at_ms, operation.operation_id.clone());
            if latest_mutating_intent
                .get(&target.device_id)
                .is_none_or(|current| current < &candidate)
            {
                latest_mutating_intent.insert(target.device_id.clone(), candidate);
            }
        }
    }
    let mut candidate_hub = runtime.hub.clone();
    let mut pending_operation_ids = BTreeSet::new();
    let mut work = Vec::<(bool, AwakeOwnerWork)>::new();
    for operation in &operations {
        if operation.confirmed_at_ms.is_none() {
            continue;
        }
        for target in operation
            .targets
            .iter()
            .filter(|target| target.preflight.eligible)
        {
            let active_windows_receipt = operation.preview.action
                == QuestAwakeAction::StartWindowsWatchdog
                && target.lifecycle == CommandLifecycle::Applied
                && target
                    .receipt
                    .as_ref()
                    .is_some_and(|receipt| receipt.effective);
            let nonterminal = matches!(
                target.lifecycle,
                CommandLifecycle::Accepted
                    | CommandLifecycle::Dispatched
                    | CommandLifecycle::Running
            );
            if !nonterminal && !active_windows_receipt {
                continue;
            }
            let is_latest = operation.preview.action == QuestAwakeAction::Status
                || latest_mutating_intent
                    .get(&target.device_id)
                    .is_some_and(|(_, operation_id)| operation_id == &operation.operation_id);
            if !is_latest {
                if nonterminal {
                    candidate_hub
                        .fail_quest_awake_target(
                            &operation.operation_id,
                            &target.device_id,
                            "superseded_during_recovery".to_owned(),
                            now_ms,
                        )
                        .map_err(|error| error.to_string())?;
                }
                continue;
            }
            let serial = serials.get(&target.device_id).cloned().ok_or_else(|| {
                format!(
                    "recovered Quest awake target {} lacks exact serial configuration",
                    target.device_id
                )
            })?;
            if target.lifecycle == CommandLifecycle::Accepted {
                pending_operation_ids.insert(operation.operation_id.clone());
                continue;
            }
            let invocation = target.invocation.clone().ok_or_else(|| {
                format!(
                    "recovered Quest awake target {} lacks its exact durable invocation",
                    target.device_id
                )
            })?;
            work.push((
                operation.preview.action == QuestAwakeAction::StartWindowsWatchdog,
                AwakeOwnerWork {
                    invocation,
                    serial,
                    provider: provider.clone(),
                    authorization_expires_at_ms: 0,
                },
            ));
        }
    }
    let RuntimeState {
        hub,
        adapter,
        state_store,
        owner_receipts,
        inflight_awake_targets,
        windows_awake_watchdogs,
        ..
    } = &mut *runtime;
    state_store.persist(&candidate_hub, adapter, owner_receipts, now_ms)?;
    *hub = candidate_hub;
    for (is_windows_watchdog, item) in &work {
        let key = (
            item.invocation.operation_id.clone(),
            item.invocation.device_id.clone(),
        );
        if !inflight_awake_targets.insert(key) {
            continue;
        }
        if *is_windows_watchdog {
            windows_awake_watchdogs.insert(
                item.invocation.device_id.clone(),
                WindowsAwakeWatchdogControl {
                    generation: item.invocation.watchdog_generation.clone(),
                    cancel: Arc::new(AtomicBool::new(false)),
                    running: Arc::new(AtomicBool::new(true)),
                },
            );
        }
    }
    drop(runtime);
    for (is_windows_watchdog, item) in work {
        if is_windows_watchdog {
            spawn_windows_awake_watchdog(state.clone(), item);
        } else {
            spawn_awake_owner_work(state.clone(), item);
        }
    }
    for operation_id in pending_operation_ids {
        schedule_pending_awake_work(state.clone(), &operation_id).await;
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
    use std::collections::{BTreeMap, BTreeSet};
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
        QUEST_WIFI_ADB_ACTION_ID, QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA,
        QUEST_WIFI_ADB_RECEIPT_SCHEMA, QuestWifiAdbAction, QuestWifiAdbOwnerInvocation,
        QuestWifiAdbOwnerReceipt, QuestWifiAdbPreviewRequest, QuestWifiAdbRouteMode,
        QuestWifiAdbWearerApproval, ReachabilityState, SavedView, SavedViewMutationRequest,
        SignedFleetCheckIn, SupportState,
    };
    use fleet_hub::{FleetHub, HubPolicy, ObservationDecision, QuestWifiAdbPreviewPlan};
    use fleet_provider_catalog::{
        ActionKind, AuthenticationRequirement, ExpectedAction, ExpectedCapability,
        ProviderCatalogConfig,
    };
    use fleet_quest_connectivity_adapter::QuestConnectivityProviderConfig;
    use fleet_simulator::{BASE_TIME_MS, ScenarioBuilder};
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
        CatalogState, ConfiguredEnrollment, ConfiguredKioskDirectOperator,
        ConfiguredPackageUpdaterOwner, ConfiguredQuestAwakePinnedArtifact,
        ConfiguredQuestAwakeProvider, ConfiguredQuestAwakeTarget, ConnectivityOwnerWorkFailure,
        FleetManifoldAdapter, IngressRateLimiter, LocalHubConfig, LocalHubState,
        MAX_CHECKINS_PER_CREDENTIAL_PER_WINDOW, MAX_CONCURRENT_CONNECTIVITY_PROVIDER_CALLS,
        RuntimeState, checkin_router, connectivity_dispatch_slots, connectivity_invocation_expired,
        prepare_pending_connectivity_batch, router, schedule_recovered_owner_work,
        settle_connectivity_owner_result, spawn_windows_hotspot_owner_work_with_clock,
        state_slot_path, transport_request_id, unix_time_ms,
    };

    static STATE_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[tokio::test]
    async fn checkin_listener_mounts_only_authenticated_checkin_ingress() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[19_u8; 32]);
        let (mut config, key_id) = config(&signing_key, now_ms);
        config.checkin_bind = Some("192.0.2.10:18742".to_owned());
        assert!(config.validate().is_err());
        config.allow_non_loopback_checkin = true;
        assert!(config.validate().is_ok());
        config.checkin_bind = Some(config.bind.clone());
        assert!(config.validate().is_err());
        config.checkin_bind = Some("192.0.2.10:18742".to_owned());

        let state_directory = config.state_directory.clone();
        let state = LocalHubState::from_config(&config, now_ms).expect("valid dual ingress");
        let app = checkin_router(state);
        for (method, path, expected) in [
            ("GET", "/fleet/v1/health", StatusCode::NOT_FOUND),
            ("GET", "/fleet/v1/provider-catalog", StatusCode::NOT_FOUND),
            (
                "POST",
                "/fleet/v1/provider-catalog/refresh",
                StatusCode::NOT_FOUND,
            ),
            ("POST", "/fleet/v1/query", StatusCode::NOT_FOUND),
            (
                "POST",
                "/fleet/v1/operations/preview",
                StatusCode::NOT_FOUND,
            ),
            (
                "GET",
                "/fleet/v1/package-updater/offers",
                StatusCode::NOT_FOUND,
            ),
            ("POST", "/fleet/v1/checkins/", StatusCode::NOT_FOUND),
            ("POST", "/Fleet/v1/checkins", StatusCode::NOT_FOUND),
            ("POST", "/fleet/v1/%63heckins", StatusCode::NOT_FOUND),
            (
                "OPTIONS",
                "/fleet/v1/checkins",
                StatusCode::METHOD_NOT_ALLOWED,
            ),
        ] {
            let response = app
                .clone()
                .oneshot(json_method_request(method, path, Vec::new()))
                .await
                .expect("isolated route response");
            assert_eq!(response.status(), expected, "{method} {path}");
        }
        let wrong_method = app
            .clone()
            .oneshot(json_method_request("GET", "/fleet/v1/checkins", Vec::new()))
            .await
            .expect("method response");
        assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);

        let signed = signed_checkin(&signing_key, key_id.as_str(), now_ms, 1);
        let accepted = app
            .oneshot(json_request(
                "/fleet/v1/checkins",
                serde_json::to_vec(&signed).expect("signed JSON"),
            ))
            .await
            .expect("check-in response");
        assert_eq!(accepted.status(), StatusCode::OK);
        fs::remove_dir_all(state_directory).expect("remove state directory");
    }

    #[tokio::test]
    async fn provider_catalog_reads_snapshot_and_refreshes_without_retaining_prior_metadata() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[20_u8; 32]);
        let (mut config, _) = config(&signing_key, now_ms);
        config.provider_catalog = vec![ProviderCatalogConfig {
            catalog_id: "quest-connectivity".to_owned(),
            executable_path: None,
            executable_sha256: None,
            expected_provider_id: "questionable-file-manager.quest-connectivity-provider"
                .to_owned(),
            expected_provider_version: "1.0.0".to_owned(),
            expected_capabilities: vec![ExpectedCapability {
                id: "questionable-file-manager.quest-connectivity.wireless-adb".to_owned(),
                contract_versions: vec![
                    "questionable.file_manager.quest_connectivity.provider_request.v1".to_owned(),
                ],
                actions: vec![ExpectedAction {
                    id: "request_wireless_adb".to_owned(),
                    kind: ActionKind::Effect,
                    authentication_requirements: vec![
                        AuthenticationRequirement::ProcessAccessControl,
                        AuthenticationRequirement::CallerAuthorityExternal,
                        AuthenticationRequirement::ExactTargetBinding,
                    ],
                }],
                effect_owner: "rusty-kiosk.wireless-adb".to_owned(),
                receipt_schema: "questionable.file_manager.quest_connectivity.provider_receipt.v1"
                    .to_owned(),
            }],
        }];
        let state_directory = config.state_directory.clone();
        let state = LocalHubState::from_config(&config, now_ms).expect("catalog config");
        let app = router(state.clone());
        let snapshot = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/fleet/v1/provider-catalog")
                    .body(Body::empty())
                    .expect("snapshot request"),
            )
            .await
            .expect("snapshot response");
        assert_eq!(snapshot.status(), StatusCode::OK);
        let snapshot_json: Value = serde_json::from_slice(
            &to_bytes(snapshot.into_body(), 64 * 1024)
                .await
                .expect("snapshot body"),
        )
        .expect("snapshot JSON");
        assert_eq!(snapshot_json["revision"], 1);
        assert_eq!(snapshot_json["entries"][0]["state"], "unconfigured");
        assert!(snapshot_json["entries"][0].get("descriptor").is_none());

        let refreshed = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/fleet/v1/provider-catalog/refresh")
                    .body(Body::empty())
                    .expect("refresh request"),
            )
            .await
            .expect("refresh response");
        assert_eq!(refreshed.status(), StatusCode::OK);
        let refreshed_json: Value = serde_json::from_slice(
            &to_bytes(refreshed.into_body(), 64 * 1024)
                .await
                .expect("refresh body"),
        )
        .expect("refresh JSON");
        assert_eq!(refreshed_json["revision"], 3);
        assert_eq!(refreshed_json["entries"][0]["state"], "unconfigured");
        assert_eq!(refreshed_json["authorizes_execution"], false);
        assert_eq!(refreshed_json["metadata_only"], true);

        state.runtime.lock().await.provider_catalog_last_refresh_ms = Some(i64::MAX);
        let rollback = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/fleet/v1/provider-catalog/refresh")
                    .body(Body::empty())
                    .expect("rollback request"),
            )
            .await
            .expect("rollback response");
        assert_eq!(rollback.status(), StatusCode::CONFLICT);
        assert_eq!(
            state.runtime.lock().await.provider_catalog_snapshot.entries[0].state,
            CatalogState::Unconfigured
        );
        fs::remove_dir_all(state_directory).expect("remove state directory");
    }

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
    async fn quest_awake_routes_are_strict_and_inert_without_private_provider() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let (config, key_id) = config(&signing_key, now_ms);
        let state_directory = config.state_directory.clone();
        let app = router(LocalHubState::from_config(&config, now_ms).expect("valid config"));
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

        let request = serde_json::json!({
            "schema": "rusty.fleet.quest_awake_preview_request.v1",
            "action_id": "quest.awake-control",
            "action": "apply_bounded",
            "duration_ms": 28_800_000,
            "watchdog_interval_ms": 5_000,
            "targets": {"device.quest.1": identity_revision}
        });
        let unavailable = app
            .clone()
            .oneshot(json_request(
                "/fleet/v1/quest-awake/preview",
                serde_json::to_vec(&request).expect("awake preview JSON"),
            ))
            .await
            .expect("awake preview response");
        assert_eq!(unavailable.status(), StatusCode::NOT_IMPLEMENTED);

        let mut oversized_duration = request.clone();
        oversized_duration["duration_ms"] = serde_json::json!(28_800_001);
        let rejected = app
            .clone()
            .oneshot(json_request(
                "/fleet/v1/quest-awake/preview",
                serde_json::to_vec(&oversized_duration).expect("oversized preview JSON"),
            ))
            .await
            .expect("oversized preview response");
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let mut unknown_field = request;
        unknown_field["serial"] = serde_json::json!("must-not-cross-public-api");
        let rejected = app
            .oneshot(json_request(
                "/fleet/v1/quest-awake/preview",
                serde_json::to_vec(&unknown_field).expect("unknown-field preview JSON"),
            ))
            .await
            .expect("strict preview response");
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        fs::remove_dir_all(state_directory).expect("remove test state directory");
    }

    #[tokio::test]
    async fn quest_wifi_adb_routes_are_strict_inert_and_expose_no_proof_submission() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[17_u8; 32]);
        let (config, key_id) = config(&signing_key, now_ms);
        let state_directory = config.state_directory.clone();
        let app = router(LocalHubState::from_config(&config, now_ms).expect("valid config"));
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

        let request = serde_json::json!({
            "schema": "rusty.fleet.quest_wifi_adb_preview_request.v1",
            "action_id": "quest.wifi-adb-control",
            "action": "request_wireless_adb",
            "targets": {"device.quest.1": identity_revision}
        });
        let unavailable = app
            .clone()
            .oneshot(json_request(
                "/fleet/v1/quest-wifi-adb/preview",
                serde_json::to_vec(&request).expect("Wi-Fi ADB preview JSON"),
            ))
            .await
            .expect("preview response");
        assert_eq!(unavailable.status(), StatusCode::NOT_IMPLEMENTED);

        let mut private_field = request;
        private_field["serial"] = serde_json::json!("must-not-cross-public-api");
        let rejected = app
            .clone()
            .oneshot(json_request(
                "/fleet/v1/quest-wifi-adb/preview",
                serde_json::to_vec(&private_field).expect("strict preview JSON"),
            ))
            .await
            .expect("strict preview response");
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

        let no_proof_ingress = app
            .oneshot(json_request(
                "/fleet/v1/quest-wifi-adb/proofs",
                serde_json::to_vec(&serde_json::json!({"shape": "must-not-be-authority"}))
                    .expect("proof JSON"),
            ))
            .await
            .expect("missing proof route response");
        assert_eq!(
            no_proof_ingress.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "the operation read route must not accept proof writes"
        );
        fs::remove_dir_all(state_directory).expect("remove test state directory");
    }

    #[test]
    fn rejected_connectivity_receipts_fail_the_dispatched_target_without_leaking_reason() {
        for damage in ["late", "future", "binding"] {
            let (mut hub, operation_id, device_id, mut receipt) =
                dispatched_wifi_adb_receipt_fixture();
            let now_ms = BASE_TIME_MS + 4;
            match damage {
                "late" => receipt.observed_at_ms = BASE_TIME_MS + 60_001,
                "future" => receipt.observed_at_ms = BASE_TIME_MS + 40_000,
                "binding" => receipt.request_id = "request.wifi.other".to_owned(),
                _ => unreachable!("bounded test cases"),
            }
            settle_connectivity_owner_result(
                &mut hub,
                &operation_id,
                &device_id,
                Ok(receipt),
                if damage == "late" {
                    BASE_TIME_MS + 60_001
                } else {
                    now_ms
                },
            )
            .expect("receipt rejection becomes a terminal sanitized failure");
            let operation = hub
                .quest_wifi_adb_operation(&operation_id)
                .expect("settled operation");
            let target = &operation.targets[0];
            assert_eq!(target.lifecycle, CommandLifecycle::Failed, "{damage}");
            assert_eq!(
                target.failure_code.as_deref(),
                Some("provider_receipt_rejected"),
                "{damage}"
            );
            assert!(target.receipt.is_none(), "{damage}");
        }
    }

    #[test]
    fn connectivity_batch_dispatches_only_eight_then_fills_the_freed_slot() {
        let mut hub = FleetHub::new(HubPolicy::default());
        let mut targets = BTreeMap::new();
        for mut observation in ScenarioBuilder::new(9).build().initial {
            observation.source_time_ms = BASE_TIME_MS;
            observation.received_time_ms = 0;
            targets.insert(
                observation.identity.device_id.clone(),
                observation.identity.identity_revision,
            );
            assert!(matches!(
                hub.accept_observation(observation, BASE_TIME_MS),
                ObservationDecision::Accepted { .. }
            ));
        }
        let configured_targets = targets.keys().cloned().collect::<BTreeSet<_>>();
        let operation = hub
            .preview_quest_wifi_adb(QuestWifiAdbPreviewPlan {
                operation_id: "operation.wifi.nine-targets".to_owned(),
                preview_id: "preview.wifi.nine-targets".to_owned(),
                request: QuestWifiAdbPreviewRequest {
                    schema: QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA.to_owned(),
                    action_id: QUEST_WIFI_ADB_ACTION_ID.to_owned(),
                    action: QuestWifiAdbAction::Status,
                    targets,
                },
                created_at_ms: BASE_TIME_MS,
                expires_at_ms: BASE_TIME_MS + 60_000,
                provider_ready_devices: configured_targets.clone(),
            })
            .expect("nine-target preview");
        let operation = hub
            .confirm_quest_wifi_adb(
                &operation.operation_id,
                &operation.preview.preview_id,
                BASE_TIME_MS + 1,
            )
            .expect("confirm nine-target preview");
        let provider = QuestConnectivityProviderConfig {
            executable_path: PathBuf::from("provider.exe"),
            executable_sha256: "11".repeat(32),
            private_stage_root: PathBuf::from("private-stage"),
        };
        let adapter = FleetManifoldAdapter::new(vec![dotted("operator.fleet.local")]);
        let (mut dispatched_hub, dispatched_adapter, work) = prepare_pending_connectivity_batch(
            &hub,
            &adapter,
            &operation,
            &configured_targets,
            &BTreeSet::new(),
            &provider,
            BASE_TIME_MS + 2,
            u64::try_from(BASE_TIME_MS + 2).expect("positive time"),
            u64::try_from(BASE_TIME_MS + 60_000).expect("positive expiry"),
        );
        assert_eq!(work.len(), MAX_CONCURRENT_CONNECTIVITY_PROVIDER_CALLS);
        let dispatched = dispatched_hub
            .quest_wifi_adb_operation(&operation.operation_id)
            .expect("bounded dispatch");
        assert_eq!(
            dispatched
                .targets
                .iter()
                .filter(|target| target.lifecycle == CommandLifecycle::Dispatched)
                .count(),
            MAX_CONCURRENT_CONNECTIVITY_PROVIDER_CALLS
        );
        assert_eq!(
            dispatched
                .targets
                .iter()
                .filter(|target| target.lifecycle == CommandLifecycle::Accepted)
                .count(),
            1
        );

        let completed = &work[0].invocation;
        dispatched_hub
            .fail_quest_wifi_adb_target(
                &completed.operation_id,
                &completed.device_id,
                "provider_invocation_failed".to_owned(),
                BASE_TIME_MS + 3,
            )
            .expect("complete one occupied slot");
        let inflight = work
            .iter()
            .skip(1)
            .map(|item| {
                (
                    item.invocation.operation_id.clone(),
                    item.invocation.device_id.clone(),
                )
            })
            .collect::<BTreeSet<_>>();
        let operation = dispatched_hub
            .quest_wifi_adb_operation(&operation.operation_id)
            .expect("one queued target remains");
        let (refilled_hub, _, refilled) = prepare_pending_connectivity_batch(
            &dispatched_hub,
            &dispatched_adapter,
            &operation,
            &configured_targets,
            &inflight,
            &provider,
            BASE_TIME_MS + 4,
            u64::try_from(BASE_TIME_MS + 4).expect("positive time"),
            u64::try_from(BASE_TIME_MS + 60_000).expect("positive expiry"),
        );
        assert_eq!(refilled.len(), 1);
        let refilled_operation = refilled_hub
            .quest_wifi_adb_operation(&operation.operation_id)
            .expect("refilled operation");
        assert_eq!(
            refilled_operation
                .targets
                .iter()
                .filter(|target| target.lifecycle == CommandLifecycle::Accepted)
                .count(),
            0
        );
    }

    #[test]
    fn connectivity_dispatch_budget_has_no_waiting_overflow_and_expiry_is_inclusive() {
        assert_eq!(
            connectivity_dispatch_slots(0),
            MAX_CONCURRENT_CONNECTIVITY_PROVIDER_CALLS
        );
        assert_eq!(
            connectivity_dispatch_slots(MAX_CONCURRENT_CONNECTIVITY_PROVIDER_CALLS - 1),
            1
        );
        assert_eq!(
            connectivity_dispatch_slots(MAX_CONCURRENT_CONNECTIVITY_PROVIDER_CALLS),
            0
        );
        assert_eq!(
            connectivity_dispatch_slots(MAX_CONCURRENT_CONNECTIVITY_PROVIDER_CALLS + 1),
            0
        );
        let invocation = QuestWifiAdbOwnerInvocation {
            schema: "rusty.fleet.quest_wifi_adb_owner_invocation.v1".to_owned(),
            request_id: "request.wifi.expiry".to_owned(),
            operation_id: "operation.wifi.expiry".to_owned(),
            preview_id: "preview.wifi.expiry".to_owned(),
            device_id: "device.quest.expiry".to_owned(),
            identity_revision: 1,
            action: QuestWifiAdbAction::Status,
            issued_at_ms: BASE_TIME_MS,
            expires_at_ms: BASE_TIME_MS + 10,
        };
        assert!(!connectivity_invocation_expired(
            &invocation,
            BASE_TIME_MS + 9
        ));
        assert!(connectivity_invocation_expired(
            &invocation,
            BASE_TIME_MS + 10
        ));
        let (mut hub, operation_id, device_id, _) = dispatched_wifi_adb_receipt_fixture();
        settle_connectivity_owner_result(
            &mut hub,
            &operation_id,
            &device_id,
            Err(ConnectivityOwnerWorkFailure::InvocationExpired),
            BASE_TIME_MS + 60_000,
        )
        .expect("expired invocation becomes terminal");
        let operation = hub
            .quest_wifi_adb_operation(&operation_id)
            .expect("expired operation");
        assert_eq!(operation.targets[0].lifecycle, CommandLifecycle::Failed);
        assert_eq!(
            operation.targets[0].failure_code.as_deref(),
            Some("provider_invocation_expired")
        );
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
    async fn hotspot_settlement_survives_dropped_receiver_and_clock_failure() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[21_u8; 32]);
        let (config, _) = config(&signing_key, now_ms);
        let state_directory = config.state_directory.clone();
        let state = LocalHubState::from_config(&config, now_ms).expect("valid config");
        let operation_id = "hotspot-operation-detached".to_owned();
        {
            let mut runtime = state.runtime.lock().await;
            runtime
                .hub
                .preview_windows_hotspot(fleet_hub::WindowsHotspotPreviewPlan {
                    operation_id: operation_id.clone(),
                    preview_id: "hotspot-preview-detached".to_owned(),
                    lease_id: "hotspot-lease-detached".to_owned(),
                    generation: "hotspot-generation-detached".to_owned(),
                    request: fleet_contracts::WindowsHotspotPreviewRequest {
                        schema: fleet_contracts::WINDOWS_HOTSPOT_PREVIEW_REQUEST_SCHEMA.to_owned(),
                        action_id: fleet_contracts::WINDOWS_HOTSPOT_ACTION_ID.to_owned(),
                        action: fleet_contracts::WindowsHotspotAction::Status,
                    },
                    created_at_ms: now_ms,
                    expires_at_ms: now_ms + 60_000,
                    provider_ready: true,
                })
                .expect("preview");
            runtime
                .hub
                .confirm_windows_hotspot(&operation_id, "hotspot-preview-detached", now_ms + 1)
                .expect("confirm");
            runtime
                .hub
                .prepare_windows_hotspot_invocation(
                    &operation_id,
                    "hotspot-request-detached".to_owned(),
                    now_ms + 2,
                )
                .expect("prepare invocation");
            runtime
                .inflight_windows_hotspot_operations
                .insert(operation_id.clone());
        }
        let provider_root = state_directory.join("missing-provider");
        let receiver = spawn_windows_hotspot_owner_work_with_clock(
            state.clone(),
            operation_id.clone(),
            fleet_windows_hotspot_adapter::WindowsHotspotProviderConfig {
                executable_path: provider_root.join("rusty-hostess-hotspot-provider.exe"),
                executable_sha256: "a".repeat(64),
                private_stage_root: state_directory.join("private-stages"),
            },
            state
                .runtime
                .lock()
                .await
                .hub
                .windows_hotspot_operation(&operation_id)
                .expect("prepared operation")
                .invocation
                .expect("invocation"),
            now_ms + 2,
            || Err("injected clock failure".to_owned()),
        );
        drop(receiver);

        for _ in 0..100 {
            let settled = {
                let runtime = state.runtime.lock().await;
                runtime
                    .hub
                    .windows_hotspot_operation(&operation_id)
                    .expect("operation")
                    .lifecycle
                    == CommandLifecycle::Failed
            };
            if settled {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let runtime = state.runtime.lock().await;
        let operation = runtime
            .hub
            .windows_hotspot_operation(&operation_id)
            .expect("settled operation");
        assert_eq!(operation.lifecycle, CommandLifecycle::Failed);
        assert_eq!(
            operation.failure_code.as_deref(),
            Some("provider_provider_unavailable")
        );
        assert!(
            !runtime
                .inflight_windows_hotspot_operations
                .contains(&operation_id)
        );
        drop(runtime);

        let restored =
            LocalHubState::from_config(&config, now_ms + 3).expect("durable settled state");
        let restored_runtime = restored.runtime.lock().await;
        assert_eq!(
            restored_runtime
                .hub
                .windows_hotspot_operation(&operation_id)
                .expect("restored operation")
                .lifecycle,
            CommandLifecycle::Failed
        );
        drop(restored_runtime);
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
    async fn package_routes_persist_dispatch_ready_and_reject_untrusted_owner_evidence() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[11_u8; 32]);
        let (mut config, key_id) = config(&signing_key, now_ms);
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
            "schema": "rusty.fleet.package_install_release_preview_request.v1",
            "action_id": "packages.install-release",
            "release": {
                "kind": "manifest_url",
                "manifest_url": "https://updates.example.invalid/alpha/envelope.json"
            },
            "expected_package_name": "io.github.mesmerprism.rustykiosk",
            "expected_rollout_ring": "alpha",
            "targets": {"device.quest.1": identity_revision}
        });
        let preview = app
            .clone()
            .oneshot(json_request(
                "/fleet/v1/package-install-releases/preview",
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
        .expect("preview JSON");
        assert_eq!(preview_json["lifecycle"], "proposed");
        assert_eq!(preview_json["targets"][0]["stage"], "preview_ready");

        let operation_id = preview_json["operation_id"]
            .as_str()
            .expect("operation ID")
            .to_owned();
        let preview_id = preview_json["preview"]["preview_id"]
            .as_str()
            .expect("preview ID")
            .to_owned();
        let execute_request = serde_json::json!({
            "schema": "rusty.fleet.package_install_release_execute_request.v1",
            "operation_id": operation_id,
            "preview_id": preview_id
        });
        let execute = app
            .clone()
            .oneshot(json_request(
                &format!("/fleet/v1/package-install-releases/{operation_id}/execute"),
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
        .expect("execute JSON");
        assert_eq!(execute_json["lifecycle"], "accepted");
        assert_eq!(execute_json["targets"][0]["stage"], "dispatch_ready");
        assert!(
            execute_json["targets"][0]["invocation"]
                .as_object()
                .is_some()
        );
        let owner_action_request_id =
            execute_json["targets"][0]["invocation"]["owner_action_request_id"]
                .as_str()
                .expect("owner request")
                .to_owned();

        let acknowledgement = serde_json::json!({
            "schema": "rusty.fleet.package_updater_invocation_acknowledgement.v1",
            "operation_id": operation_id,
            "device_id": "device.quest.1",
            "owner_action_request_id": owner_action_request_id,
            "accepted": true,
            "code": "accepted",
            "acknowledged_at_ms": now_ms + 2
        });
        let rejected = app
            .clone()
            .oneshot(json_request(
                &format!("/fleet/v1/package-install-releases/{operation_id}/acknowledgements"),
                serde_json::to_vec(&acknowledgement).expect("acknowledgement JSON"),
            ))
            .await
            .expect("acknowledgement response");
        assert_eq!(rejected.status(), StatusCode::NOT_IMPLEMENTED);

        let digest = format!("sha256:{}", "a".repeat(64));
        let receipt = serde_json::json!({
            "schema": "rusty.fleet.package_updater_receipt_submission.v1",
            "effective_receipt": {
                "schema": "rusty.fleet.package_updater_effective_receipt.v1",
                "operation_id": operation_id,
                "device_id": "device.quest.1",
                "identity_revision": identity_revision,
                "owner_action_request_id": owner_action_request_id,
                "updater_receipt": {
                    "schema": "rusty.quest.package_update_receipt.v1",
                    "stage": "install_commit",
                    "decision": "accepted",
                    "code": "installed",
                    "observed_at_ms": now_ms + 2,
                    "envelope_sha256": format!("sha256:{}", "b".repeat(64)),
                    "signed_manifest_sha256": digest,
                    "key_id": "release-key-1",
                    "manifest_id": "release-15",
                    "package_name": "io.github.mesmerprism.rustykiosk",
                    "rollout_ring": "alpha",
                    "sequence": 15,
                    "version_code": 15,
                    "prior_checkpoint": null,
                    "accepted_checkpoint": {
                        "package_name": "io.github.mesmerprism.rustykiosk",
                        "rollout_ring": "alpha",
                        "sequence": 15,
                        "version_code": 15,
                        "signed_manifest_sha256": digest
                    },
                    "state_changed": true
                },
                "wrapped_at_ms": now_ms + 2
            }
        });
        let rejected_receipt = app
            .clone()
            .oneshot(json_request(
                &format!("/fleet/v1/package-install-releases/{operation_id}/receipts"),
                serde_json::to_vec(&receipt).expect("receipt JSON"),
            ))
            .await
            .expect("receipt response");
        assert_eq!(rejected_receipt.status(), StatusCode::NOT_IMPLEMENTED);

        let status = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/fleet/v1/package-install-releases/{operation_id}"))
                    .body(Body::empty())
                    .expect("status request"),
            )
            .await
            .expect("status response");
        let status_json: Value = serde_json::from_slice(
            &to_bytes(status.into_body(), 256 * 1024)
                .await
                .expect("status body"),
        )
        .expect("status JSON");
        assert_eq!(status_json["lifecycle"], "accepted");
        assert_eq!(status_json["targets"][0]["stage"], "dispatch_ready");
        drop(app);
        drop(state);

        config.package_updater_owner = Some(ConfiguredPackageUpdaterOwner {
            owner_id: "rusty-quest.package-updater".to_owned(),
            bearer_token: "owner-token-that-is-at-least-thirty-two-bytes".to_owned(),
        });
        let restored =
            LocalHubState::from_config(&config, now_ms + 3).expect("restore package operation");
        let restored_app = router(restored.clone());
        let offered_invocation = restored
            .runtime
            .lock()
            .await
            .hub
            .package_operation(&operation_id)
            .expect("restored package operation")
            .targets[0]
            .invocation
            .clone()
            .expect("prepared invocation");
        let claim_body = serde_json::to_vec(&serde_json::json!({
            "schema": "rusty.fleet.package_updater_claim_request.v1",
            "owner_id": "rusty-quest.package-updater",
            "request_id": "owner-claim-request-1",
            "operation_id": operation_id,
            "device_id": offered_invocation.device_id,
            "expected_invocation_sha256": super::json_sha256(&offered_invocation)
        }))
        .expect("claim JSON");
        let claim_response = restored_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/fleet/v1/package-updater/claims")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::CONTENT_LENGTH, claim_body.len().to_string())
                    .header(
                        header::AUTHORIZATION,
                        "Bearer owner-token-that-is-at-least-thirty-two-bytes",
                    )
                    .body(Body::from(claim_body))
                    .expect("claim request"),
            )
            .await
            .expect("claim response");
        assert_eq!(claim_response.status(), StatusCode::OK);
        let claim_json: Value = serde_json::from_slice(
            &to_bytes(claim_response.into_body(), 256 * 1024)
                .await
                .expect("claim body"),
        )
        .expect("claim response JSON");
        assert_eq!(claim_json["owner_id"], "rusty-quest.package-updater");
        assert_eq!(
            claim_json["invocation"]["owner_action_request_id"],
            owner_action_request_id
        );
        let restored_operation = restored
            .runtime
            .lock()
            .await
            .hub
            .package_operation(&operation_id)
            .expect("restored package operation");
        assert_eq!(restored_operation.lifecycle, CommandLifecycle::Accepted);
        assert_eq!(
            serde_json::to_value(restored_operation.targets[0].stage).expect("stage JSON"),
            serde_json::json!("dispatch_ready")
        );
        assert!(restored_operation.targets[0].owner_claim.is_some());
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
        config.package_updater_owner = Some(ConfiguredPackageUpdaterOwner {
            owner_id: "rusty-quest.package-updater".to_owned(),
            bearer_token: "owner-token-that-is-at-least-thirty-two-bytes".to_owned(),
        });
        assert_eq!(
            config
                .validate()
                .expect_err("owner ingress is loopback-only"),
            "package updater owner ingress requires a loopback bind"
        );
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

    #[test]
    fn package_owner_secret_comparison_is_equal_length_constant_time_and_debug_is_redacted() {
        assert!(super::constant_time_equal(&[7_u8; 32], &[7_u8; 32]));
        assert!(!super::constant_time_equal(&[7_u8; 32], &[8_u8; 32]));
        assert!(!super::constant_time_equal(&[7_u8; 31], &[7_u8; 32]));
        let owner = ConfiguredPackageUpdaterOwner {
            owner_id: "rusty-quest.package-updater".to_owned(),
            bearer_token: "a-private-owner-token-that-must-not-appear".to_owned(),
        };
        let debug = format!("{owner:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains(&owner.bearer_token));
    }

    #[test]
    fn quest_awake_private_artifacts_are_pinned_and_debug_redacted() {
        let now_ms = unix_time_ms().expect("current time");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let (mut config, _) = config(&signing_key, now_ms);
        let artifact_root = test_state_directory();
        fs::create_dir_all(&artifact_root).expect("create private artifact root");
        let provider = artifact_root.join("questionable-file-manager-awake-provider.exe");
        let adb = artifact_root.join("adb.exe");
        let adb_win_api = artifact_root.join("AdbWinApi.dll");
        let adb_win_usb_api = artifact_root.join("AdbWinUsbApi.dll");
        fs::write(&provider, b"provider-test-artifact").expect("write provider");
        fs::write(&adb, b"adb-test-artifact").expect("write adb");
        fs::write(&adb_win_api, b"adb-win-api-test-artifact").expect("write ADB API DLL");
        fs::write(&adb_win_usb_api, b"adb-win-usb-api-test-artifact")
            .expect("write ADB USB API DLL");
        config.quest_awake_provider = Some(ConfiguredQuestAwakeProvider {
            executable_path: provider.clone(),
            executable_sha256: hex::encode(Sha256::digest(b"provider-test-artifact")),
            adb_executable_path: adb.clone(),
            adb_executable_sha256: hex::encode(Sha256::digest(b"adb-test-artifact")),
            adb_support_artifacts: vec![
                ConfiguredQuestAwakePinnedArtifact {
                    source_path: adb_win_api.clone(),
                    sha256: hex::encode(Sha256::digest(b"adb-win-api-test-artifact")),
                },
                ConfiguredQuestAwakePinnedArtifact {
                    source_path: adb_win_usb_api.clone(),
                    sha256: hex::encode(Sha256::digest(b"adb-win-usb-api-test-artifact")),
                },
            ],
            private_stage_root: artifact_root.join("stage"),
            targets: vec![ConfiguredQuestAwakeTarget {
                device_id: "device.quest.1".to_owned(),
                serial: "private-serial-123".to_owned(),
            }],
        });
        config.validate().expect("valid pinned awake provider");
        let debug = format!("{:?}", config.quest_awake_provider);
        assert!(!debug.contains("private-serial-123"));
        assert!(!debug.contains(provider.to_string_lossy().as_ref()));
        assert!(!debug.contains(adb.to_string_lossy().as_ref()));
        assert!(!debug.contains(adb_win_api.to_string_lossy().as_ref()));
        assert!(!debug.contains(adb_win_usb_api.to_string_lossy().as_ref()));

        config
            .quest_awake_provider
            .as_mut()
            .expect("provider")
            .executable_sha256 = "0".repeat(64);
        assert!(config.validate().is_err());
        fs::remove_dir_all(artifact_root).expect("remove artifact root");
    }

    fn dispatched_wifi_adb_receipt_fixture() -> (FleetHub, String, String, QuestWifiAdbOwnerReceipt)
    {
        let observation = ScenarioBuilder::new(1).build().initial.remove(0);
        let device_id = observation.identity.device_id.clone();
        let identity_revision = observation.identity.identity_revision;
        let mut hub = FleetHub::new(HubPolicy::default());
        assert!(matches!(
            hub.accept_observation(observation, BASE_TIME_MS),
            ObservationDecision::Accepted { .. }
        ));
        let operation = hub
            .preview_quest_wifi_adb(QuestWifiAdbPreviewPlan {
                operation_id: "operation.wifi.receipt-settlement".to_owned(),
                preview_id: "preview.wifi.receipt-settlement".to_owned(),
                request: QuestWifiAdbPreviewRequest {
                    schema: QUEST_WIFI_ADB_PREVIEW_REQUEST_SCHEMA.to_owned(),
                    action_id: QUEST_WIFI_ADB_ACTION_ID.to_owned(),
                    action: QuestWifiAdbAction::RequestWirelessAdb,
                    targets: BTreeMap::from([(device_id.clone(), identity_revision)]),
                },
                created_at_ms: BASE_TIME_MS,
                expires_at_ms: BASE_TIME_MS + 60_000,
                provider_ready_devices: BTreeSet::from([device_id.clone()]),
            })
            .expect("Wi-Fi ADB preview");
        hub.confirm_quest_wifi_adb(
            &operation.operation_id,
            &operation.preview.preview_id,
            BASE_TIME_MS + 1,
        )
        .expect("confirm Wi-Fi ADB");
        hub.prepare_quest_wifi_adb_invocation(
            &operation.operation_id,
            &device_id,
            "request.wifi.receipt-settlement".to_owned(),
            BASE_TIME_MS + 2,
        )
        .expect("prepare Wi-Fi ADB invocation");
        hub.mark_quest_wifi_adb_dispatched(&operation.operation_id, &device_id, BASE_TIME_MS + 3)
            .expect("dispatch Wi-Fi ADB invocation");
        let receipt = QuestWifiAdbOwnerReceipt {
            schema: QUEST_WIFI_ADB_RECEIPT_SCHEMA.to_owned(),
            request_id: "request.wifi.receipt-settlement".to_owned(),
            operation_id: operation.operation_id.clone(),
            preview_id: operation.preview.preview_id,
            device_id: device_id.clone(),
            identity_revision,
            action: QuestWifiAdbAction::RequestWirelessAdb,
            route_mode: QuestWifiAdbRouteMode::ModernTls,
            request_delivered: true,
            kiosk_setting_applied: true,
            request_after_boot_enabled: None,
            wearer_approval: QuestWifiAdbWearerApproval::Pending,
            listener_discovered: false,
            effect_applied: true,
            outcome: "wireless_adb_request_applied".to_owned(),
            evidence_sha256: "11".repeat(32),
            observed_at_ms: BASE_TIME_MS + 4,
        };
        (hub, operation.operation_id, device_id, receipt)
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
                checkin_bind: None,
                allow_non_loopback_checkin: false,
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
                package_updater_owner: None,
                quest_awake_provider: None,
                quest_connectivity_provider: None,
                windows_hotspot_provider: None,
                provider_catalog: Vec::new(),
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
        observation.capabilities.capabilities.insert(
            "rusty-quest.package-updater".to_owned(),
            CapabilityState {
                capability_id: "rusty-quest.package-updater".to_owned(),
                support: SupportState::Supported,
                enablement: EnablementState::Enabled,
                authorization: AuthorizationState::Authorized,
                reachability: ReachabilityState::Reachable,
                freshness: FreshnessState::Current,
                evidence_revision: revision,
                observed_at_ms: now_ms,
                fresh_until_ms: now_ms + 60_000,
                owner: "rusty-quest".to_owned(),
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
