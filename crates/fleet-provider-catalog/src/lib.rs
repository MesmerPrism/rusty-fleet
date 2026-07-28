// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Inert provider capability discovery.
//!
//! The catalog invokes one exact, target-free description route. Its output is
//! metadata only: it cannot authorize an operation, resolve a target, construct
//! an invocation, or prove backend health.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DISCOVERY_SCHEMA: &str = "rusty.quest.workflow.provider_capability_discovery.v1";
pub const CATALOG_SCHEMA: &str = "rusty.fleet.provider_catalog.v1";
pub const CONTRACT_SOURCE_COMMIT: &str = "fc476166f9c05f941dff7e9183f5c893426c05ca";
pub const CONTRACT_SOURCE_TREE: &str = "dbb7d894e60626f48ba51f88bdecff7429c9997e";
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
const DESCRIPTION_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCatalogConfig {
    pub catalog_id: String,
    #[serde(default)]
    pub executable_path: Option<PathBuf>,
    #[serde(default)]
    pub executable_sha256: Option<String>,
    pub expected_provider_id: String,
    pub expected_provider_version: String,
    pub expected_capabilities: Vec<ExpectedCapability>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedCapability {
    pub id: String,
    pub contract_versions: Vec<String>,
    pub actions: Vec<ExpectedAction>,
    pub effect_owner: String,
    pub receipt_schema: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedAction {
    pub id: String,
    pub kind: ActionKind,
    pub authentication_requirements: Vec<AuthenticationRequirement>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionKind {
    Observe,
    Effect,
    Cleanup,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderCatalogProjection {
    pub schema: &'static str,
    pub contract_source_commit: &'static str,
    pub entries: Vec<ProviderCatalogEntry>,
    pub metadata_only: bool,
    pub authorizes_execution: bool,
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refreshed_at_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderCatalogEntry {
    pub catalog_id: String,
    pub state: CatalogState,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<ProviderCapabilityDiscovery>,
    pub metadata_only: bool,
    pub authorizes_execution: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CatalogState {
    Unconfigured,
    Valid,
    Stale,
    Rejected,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCapabilityDiscovery {
    pub schema: String,
    pub provider: DiscoveryProvider,
    pub placement: Placement,
    pub availability: DiscoveryAvailability,
    pub description_authentication: String,
    pub authorizes_execution: bool,
    pub target_specific: bool,
    pub capabilities: Vec<DiscoveryCapability>,
    pub exclusions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryProvider {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Placement {
    WindowsHostProcess,
    HostCli,
    QuestApplication,
    ManagedService,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryAvailability {
    pub status: AvailabilityStatus,
    pub observed_at_utc: String,
    pub expires_at_utc: String,
    pub maximum_age_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AvailabilityStatus {
    DescriptorAvailable,
    Unavailable,
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryCapability {
    pub id: String,
    pub contract_versions: Vec<String>,
    pub actions: Vec<DiscoveryAction>,
    pub effect_owner: String,
    pub receipt_schema: String,
    pub exclusions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryAction {
    pub id: String,
    pub kind: ActionKind,
    pub authentication_requirements: Vec<AuthenticationRequirement>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthenticationRequirement {
    None,
    ProcessAccessControl,
    CallerAuthorityExternal,
    ExactTargetBinding,
    CurrentIdentityRevision,
    EffectOwnerProfile,
    OwnerSessionGrant,
    WearerApproval,
    OwnershipGeneration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescribeOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
}

pub trait ProviderDescriptionTransport: Send + Sync {
    fn describe(&self, executable: &Path) -> Result<DescribeOutput, CatalogError>;
}

#[derive(Clone, Debug, Default)]
pub struct ProcessProviderDescriptionTransport;

impl ProviderDescriptionTransport for ProcessProviderDescriptionTransport {
    fn describe(&self, executable: &Path) -> Result<DescribeOutput, CatalogError> {
        run_description(executable, DESCRIPTION_TIMEOUT)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogError {
    pub code: &'static str,
    pub message: String,
}

impl CatalogError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProviderCatalog<T = ProcessProviderDescriptionTransport> {
    transport: T,
}

impl Default for ProviderCatalog<ProcessProviderDescriptionTransport> {
    fn default() -> Self {
        Self {
            transport: ProcessProviderDescriptionTransport,
        }
    }
}

impl<T: ProviderDescriptionTransport> ProviderCatalog<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    #[must_use]
    pub fn inspect_all(&self, configs: &[ProviderCatalogConfig]) -> ProviderCatalogProjection {
        let Ok(now_ticks) = utc_now_ticks() else {
            return ProviderCatalogProjection {
                schema: CATALOG_SCHEMA,
                contract_source_commit: CONTRACT_SOURCE_COMMIT,
                entries: configs
                    .iter()
                    .map(|config| ProviderCatalogEntry {
                        catalog_id: config.catalog_id.clone(),
                        state: CatalogState::Rejected,
                        reason: "catalog-clock-invalid".to_owned(),
                        descriptor: None,
                        metadata_only: true,
                        authorizes_execution: false,
                    })
                    .collect(),
                metadata_only: true,
                authorizes_execution: false,
                revision: 0,
                refreshed_at_ms: None,
            };
        };
        let mut entries = thread::scope(|scope| {
            let mut handles = Vec::with_capacity(configs.len());
            for (index, config) in configs.iter().enumerate() {
                handles.push((index, scope.spawn(move || self.inspect(config, now_ticks))));
            }
            handles
                .into_iter()
                .map(|(index, handle)| {
                    (
                        index,
                        handle.join().unwrap_or_else(|_| ProviderCatalogEntry {
                            catalog_id: configs[index].catalog_id.clone(),
                            state: CatalogState::Unavailable,
                            reason: "provider-description-worker-failed".to_owned(),
                            descriptor: None,
                            metadata_only: true,
                            authorizes_execution: false,
                        }),
                    )
                })
                .collect::<Vec<_>>()
        });
        entries.sort_by_key(|(index, _)| *index);
        ProviderCatalogProjection {
            schema: CATALOG_SCHEMA,
            contract_source_commit: CONTRACT_SOURCE_COMMIT,
            entries: entries.into_iter().map(|(_, entry)| entry).collect(),
            metadata_only: true,
            authorizes_execution: false,
            revision: 0,
            refreshed_at_ms: None,
        }
    }

    #[must_use]
    pub fn inspect(&self, config: &ProviderCatalogConfig, now_ticks: i128) -> ProviderCatalogEntry {
        let rejected = |reason: &str| ProviderCatalogEntry {
            catalog_id: config.catalog_id.clone(),
            state: CatalogState::Rejected,
            reason: reason.to_owned(),
            descriptor: None,
            metadata_only: true,
            authorizes_execution: false,
        };
        if config.validate().is_err() {
            return rejected("catalog-config-rejected");
        }
        let (Some(path), Some(expected_sha256)) =
            (&config.executable_path, &config.executable_sha256)
        else {
            return ProviderCatalogEntry {
                catalog_id: config.catalog_id.clone(),
                state: CatalogState::Unconfigured,
                reason: "provider-not-configured".to_owned(),
                descriptor: None,
                metadata_only: true,
                authorizes_execution: false,
            };
        };
        let before = match artifact_identity(path) {
            Ok(identity) if &identity.sha256 == expected_sha256 => identity,
            Ok(_) => return rejected("provider-artifact-hash-mismatch"),
            Err(_) => {
                return ProviderCatalogEntry {
                    catalog_id: config.catalog_id.clone(),
                    state: CatalogState::Unavailable,
                    reason: "provider-artifact-unavailable".to_owned(),
                    descriptor: None,
                    metadata_only: true,
                    authorizes_execution: false,
                };
            }
        };
        let output = self.transport.describe(&before.canonical_path);
        match artifact_identity(path) {
            Ok(after) if after == before => {}
            _ => return rejected("provider-artifact-identity-changed"),
        }
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                return ProviderCatalogEntry {
                    catalog_id: config.catalog_id.clone(),
                    state: CatalogState::Unavailable,
                    reason: error.code.to_owned(),
                    descriptor: None,
                    metadata_only: true,
                    authorizes_execution: false,
                };
            }
        };
        if output.exit_code != 0 {
            return ProviderCatalogEntry {
                catalog_id: config.catalog_id.clone(),
                state: CatalogState::Unavailable,
                reason: "provider-description-exit-nonzero".to_owned(),
                descriptor: None,
                metadata_only: true,
                authorizes_execution: false,
            };
        }
        if output.stdout.len() > MAX_OUTPUT_BYTES {
            return rejected("provider-description-oversized");
        }
        let descriptor: ProviderCapabilityDiscovery = match serde_json::from_slice(&output.stdout) {
            Ok(descriptor) => descriptor,
            Err(_) => return rejected("provider-description-invalid-json"),
        };
        match validate_descriptor(&descriptor, config, now_ticks) {
            Ok(()) => ProviderCatalogEntry {
                catalog_id: config.catalog_id.clone(),
                state: CatalogState::Valid,
                reason: "descriptor-valid-metadata-only".to_owned(),
                descriptor: Some(descriptor),
                metadata_only: true,
                authorizes_execution: false,
            },
            Err(error) if error.code == "descriptor_stale" => ProviderCatalogEntry {
                catalog_id: config.catalog_id.clone(),
                state: CatalogState::Stale,
                reason: error.code.to_owned(),
                descriptor: Some(descriptor),
                metadata_only: true,
                authorizes_execution: false,
            },
            Err(error) => rejected(error.code),
        }
    }
}

impl ProviderCatalogConfig {
    pub fn validate(&self) -> Result<(), CatalogError> {
        if !portable_identifier(&self.catalog_id, 3, 160, false)
            || !safe_discovery_identifier(&self.expected_provider_id)
            || !semantic_version(&self.expected_provider_version)
            || self.expected_capabilities.is_empty()
            || self.expected_capabilities.len() > 64
        {
            return Err(CatalogError::new(
                "catalog_config_invalid",
                "catalog identity, version, or capability expectations are invalid",
            ));
        }
        match (&self.executable_path, &self.executable_sha256) {
            (None, None) => {}
            (Some(path), Some(digest)) if path.is_absolute() && is_lower_sha256(digest) => {}
            _ => {
                return Err(CatalogError::new(
                    "catalog_config_invalid",
                    "provider path and lowercase SHA-256 must be configured together",
                ));
            }
        }
        let mut capability_ids = BTreeSet::new();
        for capability in &self.expected_capabilities {
            if !safe_discovery_identifier(&capability.id)
                || !safe_discovery_identifier(&capability.effect_owner)
                || !safe_discovery_identifier(&capability.receipt_schema)
                || !capability_ids.insert(&capability.id)
                || capability.contract_versions.is_empty()
                || capability.contract_versions.len() > 8
                || capability.actions.is_empty()
                || capability.actions.len() > 64
            {
                return Err(CatalogError::new(
                    "catalog_config_invalid",
                    "expected capability binding is invalid",
                ));
            }
            let mut contracts = BTreeSet::new();
            if capability
                .contract_versions
                .iter()
                .any(|contract| !safe_discovery_identifier(contract) || !contracts.insert(contract))
            {
                return Err(CatalogError::new(
                    "catalog_config_invalid",
                    "expected contract binding is invalid",
                ));
            }
            let mut actions = BTreeSet::new();
            if capability.actions.iter().any(|action| {
                !safe_discovery_identifier(&action.id)
                    || !actions.insert(&action.id)
                    || action.authentication_requirements.is_empty()
                    || action.authentication_requirements.len() > 8
                    || action
                        .authentication_requirements
                        .iter()
                        .collect::<BTreeSet<_>>()
                        .len()
                        != action.authentication_requirements.len()
                    || (action
                        .authentication_requirements
                        .contains(&AuthenticationRequirement::None)
                        && action.authentication_requirements.len() != 1)
            }) {
                return Err(CatalogError::new(
                    "catalog_config_invalid",
                    "expected action binding is invalid",
                ));
            }
        }
        Ok(())
    }
}

pub fn validate_descriptor(
    descriptor: &ProviderCapabilityDiscovery,
    config: &ProviderCatalogConfig,
    now_ticks: i128,
) -> Result<(), CatalogError> {
    if descriptor.schema != DISCOVERY_SCHEMA
        || descriptor.description_authentication != "none"
        || descriptor.authorizes_execution
        || descriptor.target_specific
    {
        return Err(CatalogError::new(
            "descriptor_contract_rejected",
            "descriptor header is not inert discovery v1",
        ));
    }
    if descriptor.availability.status != AvailabilityStatus::DescriptorAvailable {
        return Err(CatalogError::new(
            "descriptor_availability_not_valid",
            "provider described itself as unavailable or disabled",
        ));
    }
    if !safe_discovery_identifier(&descriptor.provider.id)
        || !semantic_version(&descriptor.provider.version)
        || descriptor.provider.id != config.expected_provider_id
        || descriptor.provider.version != config.expected_provider_version
    {
        return Err(CatalogError::new(
            "descriptor_provider_binding_rejected",
            "provider identity or version does not match its machine-local pin",
        ));
    }
    let observed = parse_rfc3339_ticks(&descriptor.availability.observed_at_utc)?;
    let expires = parse_rfc3339_ticks(&descriptor.availability.expires_at_utc)?;
    if descriptor.availability.maximum_age_seconds == 0
        || descriptor.availability.maximum_age_seconds > 600
        || expires <= observed
        || expires - observed
            != i128::from(descriptor.availability.maximum_age_seconds) * 10_000_000
    {
        return Err(CatalogError::new(
            "descriptor_freshness_rejected",
            "descriptor freshness interval is invalid",
        ));
    }
    if observed > now_ticks {
        return Err(CatalogError::new(
            "descriptor_future_observation",
            "descriptor observation is in the future",
        ));
    }
    if descriptor.capabilities.is_empty()
        || descriptor.capabilities.len() > 64
        || descriptor.exclusions.is_empty()
        || descriptor.exclusions.len() > 32
        || !bounded_exclusions(&descriptor.exclusions)
    {
        return Err(CatalogError::new(
            "descriptor_structure_rejected",
            "descriptor collection bounds are invalid",
        ));
    }
    let mut capability_ids = BTreeSet::new();
    for capability in &descriptor.capabilities {
        if !capability_ids.insert(&capability.id)
            || !safe_discovery_identifier(&capability.id)
            || !safe_discovery_identifier(&capability.effect_owner)
            || !safe_discovery_identifier(&capability.receipt_schema)
            || capability.contract_versions.is_empty()
            || capability.contract_versions.len() > 8
            || capability.actions.is_empty()
            || capability.actions.len() > 64
            || !bounded_exclusions(&capability.exclusions)
        {
            return Err(CatalogError::new(
                "descriptor_capability_rejected",
                "descriptor capability semantics are invalid",
            ));
        }
        let mut contracts = BTreeSet::new();
        if capability
            .contract_versions
            .iter()
            .any(|contract| !safe_discovery_identifier(contract) || !contracts.insert(contract))
        {
            return Err(CatalogError::new(
                "descriptor_contract_binding_rejected",
                "descriptor contract identifiers are invalid or duplicated",
            ));
        }
        let mut action_ids = BTreeSet::new();
        for action in &capability.actions {
            if !action_ids.insert(&action.id)
                || !safe_discovery_identifier(&action.id)
                || action.authentication_requirements.is_empty()
                || action.authentication_requirements.len() > 8
            {
                return Err(CatalogError::new(
                    "descriptor_action_rejected",
                    "descriptor action semantics are invalid",
                ));
            }
            let unique = action
                .authentication_requirements
                .iter()
                .collect::<BTreeSet<_>>();
            if unique.len() != action.authentication_requirements.len()
                || (action
                    .authentication_requirements
                    .contains(&AuthenticationRequirement::None)
                    && action.authentication_requirements.len() != 1)
            {
                return Err(CatalogError::new(
                    "descriptor_authentication_rejected",
                    "descriptor authentication requirements are invalid",
                ));
            }
        }
    }
    if descriptor.capabilities.len() != config.expected_capabilities.len() {
        return Err(CatalogError::new(
            "descriptor_expected_capability_mismatch",
            "descriptor capability set differs from its machine-local expectation",
        ));
    }
    for expected in &config.expected_capabilities {
        let Some(actual) = descriptor
            .capabilities
            .iter()
            .find(|capability| capability.id == expected.id)
        else {
            return Err(CatalogError::new(
                "descriptor_expected_capability_mismatch",
                "an expected capability is absent",
            ));
        };
        if actual.effect_owner != expected.effect_owner
            || actual.receipt_schema != expected.receipt_schema
            || actual.contract_versions.len() != expected.contract_versions.len()
            || !expected
                .contract_versions
                .iter()
                .all(|contract| actual.contract_versions.contains(contract))
            || actual.actions.len() != expected.actions.len()
            || expected.actions.iter().any(|action| {
                !actual.actions.iter().any(|candidate| {
                    candidate.id == action.id
                        && candidate.kind == action.kind
                        && candidate.authentication_requirements
                            == action.authentication_requirements
                })
            })
        {
            return Err(CatalogError::new(
                "descriptor_expected_binding_mismatch",
                "capability, action, contract, owner, or receipt binding differs",
            ));
        }
    }
    if expires <= now_ticks {
        return Err(CatalogError::new(
            "descriptor_stale",
            "descriptor has expired",
        ));
    }
    Ok(())
}

fn bounded_exclusions(values: &[String]) -> bool {
    !values.is_empty()
        && values.len() <= 32
        && values.iter().all(|value| {
            portable_identifier(value, 3, 128, false)
                && value.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'-')
                })
        })
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

fn safe_discovery_identifier(value: &str) -> bool {
    if !portable_identifier(value, 3, 192, true) {
        return false;
    }
    let mut normalized = value.to_ascii_lowercase().replace(['.', '_'], "-");
    while normalized.contains("--") {
        normalized = normalized.replace("--", "-");
    }
    let tokens = normalized
        .split('-')
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if ["shell", "exec", "execute", "mcp", "command"]
        .iter()
        .any(|blocked| tokens.contains(blocked))
        || normalized == "adb"
        || [
            "raw-shell",
            "raw-adb",
            "generic-adb",
            "adb-args",
            "adb-command",
            "run-adb",
            "execute-command",
            "mcp-execute",
            "arbitrary-command",
            "raw-args",
        ]
        .iter()
        .any(|blocked| bounded_sequence(&normalized, blocked))
    {
        return false;
    }
    !tokens.contains(&"adb")
        || bounded_sequence(&normalized, "wifi-adb")
        || bounded_sequence(&normalized, "wireless-adb")
}

fn bounded_sequence(value: &str, sequence: &str) -> bool {
    value == sequence
        || value.starts_with(&format!("{sequence}-"))
        || value.ends_with(&format!("-{sequence}"))
        || value.contains(&format!("-{sequence}-"))
}

fn portable_identifier(value: &str, minimum: usize, maximum: usize, uppercase: bool) -> bool {
    (minimum..=maximum).contains(&value.len())
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value.bytes().all(|byte| {
            byte.is_ascii_digit()
                || byte.is_ascii_lowercase()
                || (uppercase && byte.is_ascii_uppercase())
                || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn semantic_version(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 64
        || value.bytes().any(|byte| {
            !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-'))
        })
    {
        return false;
    }
    let (core, suffix) = value
        .split_once('-')
        .map_or((value, None), |(a, b)| (a, Some(b)));
    core.split('.').count() == 3
        && core
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && suffix.is_none_or(|suffix| {
            !suffix.is_empty()
                && suffix
                    .split('.')
                    .all(|part| !part.is_empty() && !part.starts_with('-') && !part.ends_with('-'))
        })
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ArtifactIdentity {
    canonical_path: PathBuf,
    length: u64,
    modified_nanos: u128,
    #[cfg(windows)]
    creation_time: u64,
    #[cfg(windows)]
    last_write_time: u64,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    sha256: String,
}

fn artifact_identity(path: &Path) -> Result<ArtifactIdentity, CatalogError> {
    reject_reparse_or_remote_path(path)?;
    let canonical_path = fs::canonicalize(path).map_err(|_| {
        CatalogError::new(
            "provider_artifact_unavailable",
            "configured provider artifact is unavailable",
        )
    })?;
    let metadata = fs::metadata(&canonical_path).map_err(|_| {
        CatalogError::new(
            "provider_artifact_unavailable",
            "configured provider artifact is unavailable",
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_EXECUTABLE_BYTES {
        return Err(CatalogError::new(
            "provider_artifact_invalid",
            "configured provider artifact is not a bounded regular file",
        ));
    }
    let bytes = fs::read(&canonical_path).map_err(|_| {
        CatalogError::new(
            "provider_artifact_unavailable",
            "configured provider artifact cannot be read",
        )
    })?;
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_nanos());
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Ok(ArtifactIdentity {
            canonical_path,
            length: metadata.len(),
            modified_nanos,
            creation_time: metadata.creation_time(),
            last_write_time: metadata.last_write_time(),
            sha256: hex::encode(Sha256::digest(bytes)),
        })
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(ArtifactIdentity {
            canonical_path,
            length: metadata.len(),
            modified_nanos,
            device: metadata.dev(),
            inode: metadata.ino(),
            sha256: hex::encode(Sha256::digest(bytes)),
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        Ok(ArtifactIdentity {
            canonical_path,
            length: metadata.len(),
            modified_nanos,
            sha256: hex::encode(Sha256::digest(bytes)),
        })
    }
}

fn reject_reparse_or_remote_path(path: &Path) -> Result<(), CatalogError> {
    if !path.is_absolute() {
        return Err(CatalogError::new(
            "provider_artifact_path_rejected",
            "provider artifact path must be absolute",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use std::path::{Component, Prefix};
        match path.components().next() {
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) => {}
            _ => {
                return Err(CatalogError::new(
                    "provider_artifact_path_rejected",
                    "provider artifact must be on a local drive path",
                ));
            }
        }
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component);
            if matches!(component, Component::Prefix(_) | Component::RootDir) {
                continue;
            }
            let metadata = fs::symlink_metadata(&current).map_err(|_| {
                CatalogError::new(
                    "provider_artifact_unavailable",
                    "provider artifact path component is unavailable",
                )
            })?;
            if metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0 {
                return Err(CatalogError::new(
                    "provider_artifact_reparse_rejected",
                    "provider artifact path cannot contain a reparse point",
                ));
            }
        }
    }
    #[cfg(not(windows))]
    {
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component);
            if fs::symlink_metadata(&current)
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err(CatalogError::new(
                    "provider_artifact_reparse_rejected",
                    "provider artifact path cannot contain a symbolic link",
                ));
            }
        }
    }
    Ok(())
}

fn run_description(executable: &Path, timeout: Duration) -> Result<DescribeOutput, CatalogError> {
    let mut command = Command::new(executable);
    command
        .arg("--describe-json")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in ["SystemRoot", "SystemDrive", "WINDIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn().map_err(|_| {
        CatalogError::new(
            "provider_description_start_failed",
            "configured provider description process could not start",
        )
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        CatalogError::new(
            "provider_description_pipe_failed",
            "provider description stdout is unavailable",
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        CatalogError::new(
            "provider_description_pipe_failed",
            "provider description stderr is unavailable",
        )
    })?;
    let (sender, receiver) = mpsc::channel();
    drain_stream(0, stdout, sender.clone());
    drain_stream(1, stderr, sender);
    let started = Instant::now();
    let mut stdout_result = None;
    let mut stderr_result = None;
    loop {
        while let Ok((stream, result)) = receiver.try_recv() {
            let bytes = match result {
                Ok(bytes) => bytes,
                Err(_) => {
                    terminate_process_tree(&mut child);
                    return Err(CatalogError::new(
                        "provider_description_read_failed",
                        "provider description output could not be read",
                    ));
                }
            };
            if bytes.len() > MAX_OUTPUT_BYTES {
                terminate_process_tree(&mut child);
                return Err(CatalogError::new(
                    "provider_description_oversized",
                    "provider description output exceeds 64 KiB",
                ));
            }
            if stream == 0 {
                stdout_result = Some(bytes);
            } else {
                stderr_result = Some(bytes);
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let deadline = Instant::now() + Duration::from_secs(1);
                while (stdout_result.is_none() || stderr_result.is_none())
                    && Instant::now() < deadline
                {
                    if let Ok((stream, result)) = receiver.recv_timeout(Duration::from_millis(20)) {
                        let bytes = match result {
                            Ok(bytes) => bytes,
                            Err(_) => {
                                terminate_process_tree(&mut child);
                                return Err(CatalogError::new(
                                    "provider_description_read_failed",
                                    "provider description output could not be read",
                                ));
                            }
                        };
                        if bytes.len() > MAX_OUTPUT_BYTES {
                            return Err(CatalogError::new(
                                "provider_description_oversized",
                                "provider description output exceeds 64 KiB",
                            ));
                        }
                        if stream == 0 {
                            stdout_result = Some(bytes);
                        } else {
                            stderr_result = Some(bytes);
                        }
                    }
                }
                let stdout = stdout_result.ok_or_else(|| {
                    CatalogError::new(
                        "provider_description_read_failed",
                        "provider description stdout did not close",
                    )
                })?;
                let stderr = stderr_result.ok_or_else(|| {
                    CatalogError::new(
                        "provider_description_read_failed",
                        "provider description stderr did not close",
                    )
                })?;
                if !stderr.is_empty() {
                    return Err(CatalogError::new(
                        "provider_description_stderr_rejected",
                        "provider description emitted unstructured stderr",
                    ));
                }
                return Ok(DescribeOutput {
                    exit_code: status.code().ok_or_else(|| {
                        CatalogError::new(
                            "provider_description_exit_invalid",
                            "provider description has no reportable exit code",
                        )
                    })?,
                    stdout,
                });
            }
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                terminate_process_tree(&mut child);
                return Err(CatalogError::new(
                    "provider_description_timeout",
                    "provider description exceeded its bounded deadline",
                ));
            }
            Err(_) => {
                terminate_process_tree(&mut child);
                return Err(CatalogError::new(
                    "provider_description_wait_failed",
                    "provider description status could not be read",
                ));
            }
        }
    }
}

fn drain_stream<R: Read + Send + 'static>(
    stream: u8,
    mut reader: R,
    sender: mpsc::Sender<(u8, std::io::Result<Vec<u8>>)>,
) {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        let result = loop {
            match reader.read(&mut chunk) {
                Ok(0) => break Ok(bytes),
                Ok(count) if bytes.len().saturating_add(count) <= MAX_OUTPUT_BYTES => {
                    bytes.extend_from_slice(&chunk[..count]);
                }
                Ok(_) => {
                    break Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "provider description output exceeded the bounded cap",
                    ));
                }
                Err(error) => break Err(error),
            }
        };
        let _ = sender.send((stream, result));
    });
}

fn terminate_process_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let taskkill = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .map(|root| root.join("System32").join("taskkill.exe"))
            .unwrap_or_else(|| PathBuf::from("taskkill.exe"));
        let _ = Command::new(taskkill)
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .env_clear()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn utc_now_ticks() -> Result<i128, CatalogError> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        CatalogError::new(
            "catalog_clock_invalid",
            "system clock is before the Unix epoch",
        )
    })?;
    Ok(i128::from(elapsed.as_secs()) * 10_000_000 + i128::from(elapsed.subsec_nanos() / 100))
}

fn parse_rfc3339_ticks(value: &str) -> Result<i128, CatalogError> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !matches!(bytes.get(10), Some(b'T' | b't'))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return Err(timestamp_error());
    }
    let year = parse_digits(bytes, 0, 4)?;
    let month = parse_digits(bytes, 5, 2)?;
    let day = parse_digits(bytes, 8, 2)?;
    let hour = parse_digits(bytes, 11, 2)?;
    let minute = parse_digits(bytes, 14, 2)?;
    let second = parse_digits(bytes, 17, 2)?;
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return Err(timestamp_error());
    }
    let mut cursor = 19;
    let mut fraction_ticks = 0_i128;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        let digits = cursor - start;
        if digits == 0
            || digits > 32
            || (digits > 7 && bytes[start + 7..cursor].iter().any(|byte| *byte != b'0'))
        {
            return Err(timestamp_error());
        }
        let tick_digits = digits.min(7);
        fraction_ticks = i128::from(parse_digits(bytes, start, tick_digits)?);
        for _ in tick_digits..7 {
            fraction_ticks *= 10;
        }
    }
    let offset_minutes = match bytes.get(cursor) {
        Some(b'Z' | b'z') if cursor + 1 == bytes.len() => 0_i64,
        Some(sign @ (b'+' | b'-')) if cursor + 6 == bytes.len() => {
            if bytes.get(cursor + 3) != Some(&b':') {
                return Err(timestamp_error());
            }
            let hours = parse_digits(bytes, cursor + 1, 2)?;
            let minutes = parse_digits(bytes, cursor + 4, 2)?;
            if hours > 23 || minutes > 59 {
                return Err(timestamp_error());
            }
            let value = i64::from(hours * 60 + minutes);
            if *sign == b'-' { -value } else { value }
        }
        _ => return Err(timestamp_error()),
    };
    let leap = i128::from(second == 60);
    let normalized_second = if second == 60 { 59 } else { second };
    let days = days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    let local_seconds = i128::from(days) * 86_400
        + i128::from(hour * 3600 + minute * 60 + normalized_second)
        + leap;
    Ok((local_seconds - i128::from(offset_minutes) * 60) * 10_000_000 + fraction_ticks)
}

fn parse_digits(bytes: &[u8], start: usize, count: usize) -> Result<u32, CatalogError> {
    let slice = bytes
        .get(start..start + count)
        .ok_or_else(timestamp_error)?;
    if !slice.iter().all(u8::is_ascii_digit) {
        return Err(timestamp_error());
    }
    Ok(slice
        .iter()
        .fold(0_u32, |value, byte| value * 10 + u32::from(byte - b'0')))
}

const fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - if month <= 2 { 1 } else { 0 };
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn timestamp_error() -> CatalogError {
    CatalogError::new(
        "descriptor_timestamp_rejected",
        "descriptor timestamp is not an exact supported RFC3339 instant",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FakeTransport {
        output: Result<DescribeOutput, CatalogError>,
    }

    impl ProviderDescriptionTransport for FakeTransport {
        fn describe(&self, _executable: &Path) -> Result<DescribeOutput, CatalogError> {
            self.output.clone()
        }
    }

    #[derive(Clone)]
    struct SwappingTransport;

    impl ProviderDescriptionTransport for SwappingTransport {
        fn describe(&self, executable: &Path) -> Result<DescribeOutput, CatalogError> {
            fs::write(executable, b"swapped-provider").expect("swap fixture");
            Ok(DescribeOutput {
                exit_code: 0,
                stdout: serde_json::to_vec(&descriptor()).expect("descriptor"),
            })
        }
    }

    fn config(path: Option<PathBuf>, digest: Option<String>) -> ProviderCatalogConfig {
        ProviderCatalogConfig {
            catalog_id: "quest-connectivity".to_owned(),
            executable_path: path,
            executable_sha256: digest,
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
                        AuthenticationRequirement::CallerAuthorityExternal,
                        AuthenticationRequirement::ExactTargetBinding,
                    ],
                }],
                effect_owner: "rusty-kiosk.wireless-adb".to_owned(),
                receipt_schema: "questionable.file_manager.quest_connectivity.provider_receipt.v1"
                    .to_owned(),
            }],
        }
    }

    fn descriptor() -> ProviderCapabilityDiscovery {
        ProviderCapabilityDiscovery {
            schema: DISCOVERY_SCHEMA.to_owned(),
            provider: DiscoveryProvider {
                id: "questionable-file-manager.quest-connectivity-provider".to_owned(),
                version: "1.0.0".to_owned(),
            },
            placement: Placement::WindowsHostProcess,
            availability: DiscoveryAvailability {
                status: AvailabilityStatus::DescriptorAvailable,
                observed_at_utc: "2026-07-27T10:00:00.0000000Z".to_owned(),
                expires_at_utc: "2026-07-27T10:05:00.0000000Z".to_owned(),
                maximum_age_seconds: 300,
            },
            description_authentication: "none".to_owned(),
            authorizes_execution: false,
            target_specific: false,
            capabilities: vec![DiscoveryCapability {
                id: "questionable-file-manager.quest-connectivity.wireless-adb".to_owned(),
                contract_versions: vec![
                    "questionable.file_manager.quest_connectivity.provider_request.v1".to_owned(),
                ],
                actions: vec![DiscoveryAction {
                    id: "request_wireless_adb".to_owned(),
                    kind: ActionKind::Effect,
                    authentication_requirements: vec![
                        AuthenticationRequirement::CallerAuthorityExternal,
                        AuthenticationRequirement::ExactTargetBinding,
                    ],
                }],
                effect_owner: "rusty-kiosk.wireless-adb".to_owned(),
                receipt_schema: "questionable.file_manager.quest_connectivity.provider_receipt.v1"
                    .to_owned(),
                exclusions: vec!["no-target-resolution".to_owned()],
            }],
            exclusions: vec!["no-execution-grant".to_owned()],
        }
    }

    #[test]
    fn semantic_validator_accepts_exact_binding_and_offset_or_leap_timestamps() {
        let now = parse_rfc3339_ticks("2026-07-27T10:02:00Z").expect("now");
        assert_eq!(
            validate_descriptor(&descriptor(), &config(None, None), now),
            Ok(())
        );
        assert_eq!(
            parse_rfc3339_ticks("2026-07-27T12:00:00.1234567+02:00"),
            parse_rfc3339_ticks("2026-07-27T10:00:00.1234567Z")
        );
        assert_eq!(
            parse_rfc3339_ticks("1990-12-31T23:59:60Z"),
            parse_rfc3339_ticks("1991-01-01T00:00:00Z")
        );
        assert!(parse_rfc3339_ticks("2026-07-27 10:00:00Z").is_err());
        assert!(parse_rfc3339_ticks("2026-07-27T10:00:00+24:00").is_err());
        assert!(parse_rfc3339_ticks("1990-12-31T23:59:61Z").is_err());
    }

    #[test]
    fn rejects_stale_future_interval_drift_duplicate_and_executable_vocabulary() {
        let now = parse_rfc3339_ticks("2026-07-27T10:02:00Z").expect("now");
        let mut damaged = descriptor();
        damaged.availability.expires_at_utc = "2026-07-27T10:01:00Z".to_owned();
        damaged.availability.maximum_age_seconds = 60;
        assert_eq!(
            validate_descriptor(&damaged, &config(None, None), now)
                .expect_err("stale")
                .code,
            "descriptor_stale"
        );
        let mut damaged = descriptor();
        damaged.availability.observed_at_utc = "2026-07-27T10:02:01Z".to_owned();
        damaged.availability.expires_at_utc = "2026-07-27T10:07:01Z".to_owned();
        assert_eq!(
            validate_descriptor(&damaged, &config(None, None), now)
                .expect_err("future")
                .code,
            "descriptor_future_observation"
        );
        let mut damaged = descriptor();
        damaged.availability.expires_at_utc = "2026-07-27T10:05:00.0000001Z".to_owned();
        assert_eq!(
            validate_descriptor(&damaged, &config(None, None), now)
                .expect_err("drift")
                .code,
            "descriptor_freshness_rejected"
        );
        let mut damaged = descriptor();
        damaged.capabilities.push(damaged.capabilities[0].clone());
        assert_eq!(
            validate_descriptor(&damaged, &config(None, None), now)
                .expect_err("duplicate")
                .code,
            "descriptor_capability_rejected"
        );
        let mut damaged = descriptor();
        damaged.capabilities[0].actions[0].id = "command-wireless-adb".to_owned();
        assert_eq!(
            validate_descriptor(&damaged, &config(None, None), now)
                .expect_err("vocabulary")
                .code,
            "descriptor_action_rejected"
        );
    }

    #[test]
    fn unconfigured_and_hash_mismatch_are_fail_closed_and_path_free() {
        let catalog = ProviderCatalog::new(FakeTransport {
            output: Err(CatalogError::new("should-not-run", "must remain inert")),
        });
        let entry = catalog.inspect(&config(None, None), 0);
        assert_eq!(entry.state, CatalogState::Unconfigured);
        assert!(entry.descriptor.is_none());
        assert!(!entry.authorizes_execution);

        let root = std::env::temp_dir().join(format!(
            "rusty-fleet-provider-catalog-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temp");
        let executable = root.join("provider.exe");
        fs::write(&executable, b"not-an-executable").expect("fixture");
        let entry = catalog.inspect(&config(Some(executable), Some("a".repeat(64))), 0);
        assert_eq!(entry.state, CatalogState::Rejected);
        assert_eq!(entry.reason, "provider-artifact-hash-mismatch");
        let json = serde_json::to_string(&entry).expect("json");
        assert!(!json.contains(root.to_string_lossy().as_ref()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_artifact_swap_and_rejects_oversized_or_multi_document_output() {
        let root = std::env::temp_dir().join(format!(
            "rusty-fleet-provider-catalog-swap-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temp");
        let executable = root.join("provider.exe");
        fs::write(&executable, b"initial-provider").expect("fixture");
        let digest = hex::encode(Sha256::digest(b"initial-provider"));
        let catalog = ProviderCatalog::new(SwappingTransport);
        let entry = catalog.inspect(
            &config(Some(executable.clone()), Some(digest)),
            parse_rfc3339_ticks("2026-07-27T10:02:00Z").expect("now"),
        );
        assert_eq!(entry.state, CatalogState::Rejected);
        assert_eq!(entry.reason, "provider-artifact-identity-changed");

        fs::write(&executable, b"stable-provider").expect("stable");
        let digest = hex::encode(Sha256::digest(b"stable-provider"));
        let oversized = ProviderCatalog::new(FakeTransport {
            output: Ok(DescribeOutput {
                exit_code: 0,
                stdout: vec![b' '; MAX_OUTPUT_BYTES + 1],
            }),
        })
        .inspect(&config(Some(executable.clone()), Some(digest.clone())), 0);
        assert_eq!(oversized.reason, "provider-description-oversized");
        let mut documents = serde_json::to_vec(&descriptor()).expect("descriptor");
        documents.extend_from_slice(br#" {}"#);
        let multiple = ProviderCatalog::new(FakeTransport {
            output: Ok(DescribeOutput {
                exit_code: 0,
                stdout: documents,
            }),
        })
        .inspect(&config(Some(executable), Some(digest)), 0);
        assert_eq!(multiple.reason, "provider-description-invalid-json");
        let _ = fs::remove_dir_all(root);
    }
}
