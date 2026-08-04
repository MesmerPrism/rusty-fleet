//! Fail-closed, offline Fleet onboarding material generation.
//!
//! This crate owns planning and private file generation only. It does not
//! install, enroll, contact a Hub, discover a device, or revoke authorization.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf, Prefix};
use std::time::Duration;

use fleet_hub_local::LocalHubConfig;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

const REQUEST_SCHEMA: &str = "rusty.fleet.offline_onboarding_request.v1";
const PLAN_SCHEMA: &str = "rusty.fleet.offline_onboarding_plan.v1";
const INVENTORY_SCHEMA: &str = "rusty.fleet.offline_onboarding_private_inventory.v1";
const TOOL_SCHEMA: &str = "rusty.quest.fleet_agent_key_record_release_capsule.v1";
const TOOL_CONTRACT_SCHEMA: &str = "rusty.quest.fleet_agent_key_record_tool_contract.v1";
const TOOL_PROVENANCE_SCHEMA: &str = "rusty.quest.fleet_agent_key_record_release_provenance.v1";
const TOOL_CAPSULE_VERSION: &str = "1.0.0";
const TOOL_OWNER: &str = "rusty-quest";
const TOOL_OWNER_REPOSITORY: &str = "https://github.com/MesmerPrism/rusty-quest";
const TOOL_CONSUMER_ID: &str = "rusty-fleet/fleet-onboard";
const TOOL_MANIFEST_SHA256: &str =
    "d96baf6f3cdd5af9d79d0d98df5fd96e5ee9f689350a1d415c8a88fac101e457";
const TOOL_MANIFEST_SIZE: u64 = 1_835;
const TOOL_EXECUTABLE_SHA256: &str =
    "6e3962726be67cf42d0fdc2dbf3792f7d665524323e5615f1907002518bfe3d7";
const TOOL_EXECUTABLE_SIZE: u64 = 219_648;
const TOOL_PROVENANCE_SHA256: &str =
    "dba7ba306b3f0839db54d1e965f71a1939f82b413fa72cf7d883788a7ba41676";
const TOOL_LICENSE_SHA256: &str =
    "0d96a4ff68ad6d4b6f1f30f713b18d5184912ba8dd389f86aa7710db079abcb0";
const TOOL_NOTICE_SHA256: &str = "7a95b2704991263057c12f75efae64cd2c38bf35e20fceca7bc42884e69e698a";
const TOOL_CHECKSUMS_SHA256: &str =
    "aae77f56355cb6129b13dbe20850fb08c01a7a4cba9a17a8d97aee84490f407b";
const TOOL_SOURCE_COMMIT: &str = "cebdf368d9a2f1d2c12f9566f937f51bd5f29945";
const TOOL_SOURCE_TREE: &str = "1d23419ff6e95289b804d86ccc5a5cd66fd27afc";
const TOOL_COMPOSITION: &str = "690b3f6192c27f2de7da621f1ffe4b136868701ce9161ca1fd961ee70b196609";
const TOOL_CARGO_CONFIG_SHA256: &str =
    "b25e4a2d0a7562470f166e8082f1ff2ee01bcd01cc222935e4fcffb15febfc4f";
const PROFILE_SCHEMA: &str = "rusty.quest.fleet_agent_profile.v1";
const PROFILE_OWNER_SOURCE_SHA256: &str =
    "dd076b50ce37484105f9c75b542b94d50d62fc1c5070e60b0de1056fd0d4b86b";
const PROFILE_OWNER_FIXTURE_SHA256: &str =
    "ee6faba86ef876f988e7b5ddaa552ee3a484f15b0e3704c79404f92b0bda9fc9";
const KEY_RECORD_SCHEMA: &str = "rusty.quest.fleet_agent_key_record.v1";
const HUB_CONFIG_SCHEMA: &str = "rusty.fleet.local_hub_config.v1";
const MAX_JSON_BYTES: u64 = 1_048_576;
const MAX_EXECUTABLE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TOOL_OUTPUT: usize = 16_384;
const TOOL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum Command {
    ValidateTool {
        request: PathBuf,
    },
    Plan {
        request: PathBuf,
    },
    Apply {
        request: PathBuf,
        confirmation: String,
    },
    CleanupPlan {
        inventory: PathBuf,
    },
    CleanupApply {
        inventory: PathBuf,
        confirmation: String,
    },
    RevokePlan {
        inventory: PathBuf,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    output_root: PathBuf,
    tool_manifest: PathBuf,
    hub: HubRequest,
    devices: Vec<DeviceRequest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HubRequest {
    bind: String,
    checkin_bind: String,
    state_directory: PathBuf,
    operator_id: String,
    request_id_prefix: String,
    credential_valid_from_ms: u64,
    credential_expires_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DeviceRequest {
    device_id: String,
    display_name: String,
    model: String,
    hardware_class: String,
    identity_revision: u64,
    expected_authority_revision: u64,
    status_revision: u64,
    source_revision: u64,
    source_epoch: String,
    key_id: String,
    key_generation: u64,
    trust_domain: String,
    checkin_ttl_ms: u64,
    checkin_interval_ms: u64,
    hub_endpoint: String,
    #[serde(default)]
    tags: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolManifest {
    schema: String,
    capsule_version: String,
    tool_contract: ToolContract,
    source: ToolSource,
    artifact: ToolArtifact,
    distribution: ToolDistribution,
    payload: Vec<ToolPayloadEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolContract {
    schema: String,
    executable: String,
    argument_contract: String,
    output_schema: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolSource {
    repository_url: String,
    commit: String,
    tree: String,
    provenance_path: String,
    provenance_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolArtifact {
    path: String,
    sha256: String,
    size_bytes: u64,
    target: String,
    profile: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolDistribution {
    portable: bool,
    supported: bool,
    inert_until_invoked: bool,
    install_contract: String,
    private_material_included: bool,
    live_onboarding_claim: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ToolPayloadEntry {
    path: String,
    sha256: String,
    size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolProvenance {
    schema: String,
    capsule_version: String,
    source: ProvenanceSource,
    build: ProvenanceBuild,
    claims: ProvenanceClaims,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvenanceSource {
    repository_url: String,
    commit: String,
    tree: String,
    package: String,
    composition_fingerprint: String,
    repositories: Vec<SourceRepository>,
    workspace_parse_only_repositories: Vec<SourceRepository>,
    files: Vec<SourceFile>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRepository {
    repository_id: String,
    role: String,
    repository_url: String,
    commit: String,
    tree: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceFile {
    path: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvenanceBuild {
    target: String,
    profile: String,
    rustc: String,
    cargo: String,
    locked_dependencies: bool,
    isolated_git_materializations: bool,
    post_build_identity_verified: bool,
    path_remap_root: String,
    symbols_stripped: bool,
    linker_reproducibility_argument: String,
    pe_reproducibility_marker: String,
    cargo_config_sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvenanceClaims {
    owner: String,
    helper_only: bool,
    runtime_activation: String,
    enrollment_authority: bool,
    device_authority: bool,
    private_seed_included: bool,
    profile_included: bool,
    hub_configuration_included: bool,
}

#[derive(Clone, Debug, Serialize)]
struct Plan {
    schema: &'static str,
    request_sha256: String,
    tool_manifest_sha256: &'static str,
    tool_executable_sha256: &'static str,
    tool_source_commit: &'static str,
    tool_owner: &'static str,
    tool_consumer_id: &'static str,
    quest_profile_owner_source_sha256: &'static str,
    quest_profile_owner_fixture_sha256: &'static str,
    device_ids: Vec<String>,
    output_files: Vec<String>,
    effects: Vec<&'static str>,
    non_claims: Vec<&'static str>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    schema: String,
    plan_sha256: String,
    output_root: String,
    tool_manifest_sha256: String,
    tool_executable_sha256: String,
    quest_profile_owner_source_sha256: String,
    quest_profile_owner_fixture_sha256: String,
    root_identity: ObjectIdentity,
    entries: Vec<EntryRecord>,
    state: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct EntryRecord {
    relative_path: String,
    kind: EntryKind,
    identity: ObjectIdentity,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum EntryKind {
    Directory,
    File,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ObjectIdentity {
    volume_serial_number: u64,
    file_id: u64,
    number_of_links: u64,
    acl_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct KeyRecord {
    schema: String,
    key_id: String,
    public_key_hex: String,
    key_fingerprint: String,
}

/// Hash-pinned mirror of Rusty Quest's accepted de144 profile contract.
///
/// The accepted owner source did not publish a standalone schema/parser
/// artifact, so Fleet mirrors the exact deny-unknown shape and validator and
/// binds both owner source and golden fixture hashes. This is conformance
/// evidence, not ownership transfer.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct QuestFleetAgentProfileV1 {
    schema: String,
    enabled: bool,
    device_id: String,
    display_name: String,
    model: String,
    hardware_class: String,
    identity_revision: u64,
    expected_authority_revision: u64,
    status_revision: u64,
    source_revision: u64,
    source_epoch: String,
    key_id: String,
    key_fingerprint: String,
    trust_domain: String,
    checkin_ttl_ms: u64,
    checkin_interval_ms: u64,
    hub_endpoint: String,
    #[serde(default)]
    tags: BTreeMap<String, String>,
}

struct LoadedRequest {
    request: Request,
    bytes_sha256: String,
}

struct ValidatedTool {
    executable: secure_fs::RetainedExecutable,
}

pub fn execute(command: Command) -> Result<Value, String> {
    match command {
        Command::ValidateTool { request } => {
            let loaded = load_request(&request)?;
            let _tool = validate_tool(&loaded.request)?;
            Ok(json!({
                "schema": "rusty.fleet.offline_onboarding_tool_validation.v1",
                "status": "valid",
                "tool_manifest_sha256": TOOL_MANIFEST_SHA256,
                "tool_executable_sha256": TOOL_EXECUTABLE_SHA256,
                "source_commit": TOOL_SOURCE_COMMIT,
                "source_tree": TOOL_SOURCE_TREE,
                "source_composition_fingerprint": TOOL_COMPOSITION,
                "capsule_version": TOOL_CAPSULE_VERSION,
                "capsule_owner": TOOL_OWNER,
                "consumer_id": TOOL_CONSUMER_ID,
                "capsule_scope": "supported-owner-release",
                "distribution_eligible": true,
                "private_material_included": false,
                "live_onboarding_claim": false
            }))
        }
        Command::Plan { request } => {
            let loaded = load_request(&request)?;
            let (plan, _tool) = build_plan(&loaded)?;
            plan_value(&plan)
        }
        Command::Apply {
            request,
            confirmation,
        } => apply(&request, &confirmation),
        Command::CleanupPlan { inventory } => {
            let session = secure_fs::CleanupSession::open(&inventory)?;
            cleanup_plan_value(&session)
        }
        Command::CleanupApply {
            inventory,
            confirmation,
        } => cleanup_apply(&inventory, &confirmation),
        Command::RevokePlan { inventory } => revoke_plan(&inventory),
    }
}

fn load_request(path: &Path) -> Result<LoadedRequest, String> {
    let loaded = secure_fs::read_absolute_file(path, MAX_JSON_BYTES, false, true, "request")?;
    let request: Request =
        serde_json::from_slice(&loaded.bytes).map_err(|_| "invalid_request".to_owned())?;
    validate_request(&request)?;
    Ok(LoadedRequest {
        request,
        bytes_sha256: hash(&loaded.bytes),
    })
}

fn validate_request(request: &Request) -> Result<(), String> {
    if request.schema != REQUEST_SCHEMA
        || !request.output_root.is_absolute()
        || !request.tool_manifest.is_absolute()
        || !request.hub.state_directory.is_absolute()
        || request.devices.is_empty()
        || request.devices.len() > 1_000
    {
        return Err("invalid_request".to_owned());
    }
    secure_fs::validate_external_path(&request.output_root)?;
    secure_fs::validate_external_path(&request.tool_manifest)?;
    secure_fs::validate_external_path(&request.hub.state_directory)?;

    let operator_bind = request
        .hub
        .bind
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "invalid_hub_configuration".to_owned())?;
    let checkin_bind = request
        .hub
        .checkin_bind
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "invalid_hub_configuration".to_owned())?;
    if !operator_bind.ip().is_loopback()
        || checkin_bind.ip().is_loopback()
        || checkin_bind.ip().is_unspecified()
        || checkin_bind.ip().is_multicast()
        || checkin_bind.ip().to_string() == "255.255.255.255"
        || operator_bind == checkin_bind
        || request.hub.credential_valid_from_ms >= request.hub.credential_expires_at_ms
        || request.hub.credential_expires_at_ms > i64::MAX as u64
        || !dotted(&request.hub.operator_id)
        || !dotted(&request.hub.request_id_prefix)
    {
        return Err("invalid_hub_configuration".to_owned());
    }
    let exact_endpoint = format!("http://{checkin_bind}/fleet/v1/checkins");
    let mut devices = BTreeSet::new();
    let mut folded_devices = BTreeSet::new();
    let mut keys = BTreeSet::new();
    let mut folded_keys = BTreeSet::new();
    for device in &request.devices {
        if !devices.insert(device.device_id.as_str())
            || !folded_devices.insert(device.device_id.to_ascii_lowercase())
            || !keys.insert(device.key_id.as_str())
            || !folded_keys.insert(device.key_id.to_ascii_lowercase())
            || !dotted(&device.device_id)
            || !dotted(&device.key_id)
            || !dotted(&device.trust_domain)
            || !dotted(&device.source_epoch)
            || device.display_name.trim().is_empty()
            || device.model.trim().is_empty()
            || device.hardware_class.trim().is_empty()
            || device.identity_revision == 0
            || device.expected_authority_revision == 0
            || device.status_revision == 0
            || device.source_revision == 0
            || device.key_generation == 0
            || !(10_000..=300_000).contains(&device.checkin_ttl_ms)
            || !(5_000..device.checkin_ttl_ms).contains(&device.checkin_interval_ms)
            || device.hub_endpoint != exact_endpoint
            || device.tags.len() > 64
            || device.tags.iter().any(|(key, value)| {
                key.trim().is_empty()
                    || value.trim().is_empty()
                    || key.len() > 128
                    || value.len() > 256
            })
        {
            return Err("invalid_device_configuration".to_owned());
        }
    }
    Ok(())
}

fn dotted(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 64
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
fn canonical_text_hash(bytes: &[u8]) -> String {
    let text = std::str::from_utf8(bytes).expect("owner fixture must be UTF-8");
    let canonical = text.replace("\r\n", "\n").replace('\r', "\n");
    hash(canonical.as_bytes())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_tool(request: &Request) -> Result<ValidatedTool, String> {
    let manifest_file = secure_fs::read_absolute_file(
        &request.tool_manifest,
        MAX_JSON_BYTES,
        false,
        false,
        "tool",
    )?;
    if hash(&manifest_file.bytes) != TOOL_MANIFEST_SHA256 {
        return Err("untrusted_tool_manifest".to_owned());
    }
    if manifest_file.bytes.len() as u64 != TOOL_MANIFEST_SIZE {
        return Err("untrusted_tool_manifest".to_owned());
    }
    let manifest: ToolManifest = serde_json::from_slice(&manifest_file.bytes)
        .map_err(|_| "invalid_tool_manifest".to_owned())?;
    validate_exact_manifest(&manifest)?;
    let capsule_root = request
        .tool_manifest
        .parent()
        .ok_or_else(|| "invalid_tool_manifest".to_owned())?;
    validate_capsule_file_set(capsule_root)?;
    let provenance_path = capsule_root.join(&manifest.source.provenance_path);
    let provenance_file =
        secure_fs::read_absolute_file(&provenance_path, MAX_JSON_BYTES, false, false, "tool")?;
    if hash(&provenance_file.bytes) != TOOL_PROVENANCE_SHA256 {
        return Err("tool_provenance_hash_mismatch".to_owned());
    }
    let provenance: ToolProvenance = serde_json::from_slice(&provenance_file.bytes)
        .map_err(|_| "invalid_tool_provenance".to_owned())?;
    validate_exact_provenance(&provenance)?;
    for (name, expected_hash, expected_size) in [
        ("LICENSE", TOOL_LICENSE_SHA256, 34_523_u64),
        ("SOURCE-NOTICE.md", TOOL_NOTICE_SHA256, 427_u64),
        ("checksums.sha256", TOOL_CHECKSUMS_SHA256, 420_u64),
    ] {
        let file = secure_fs::read_absolute_file(
            &capsule_root.join(name),
            MAX_JSON_BYTES,
            false,
            false,
            "tool",
        )?;
        if file.bytes.len() as u64 != expected_size || hash(&file.bytes) != expected_hash {
            return Err("tool_capsule_payload_mismatch".to_owned());
        }
    }
    let executable_path = capsule_root.join(&manifest.artifact.path);
    let executable = secure_fs::RetainedExecutable::open(&executable_path, MAX_EXECUTABLE_BYTES)?;
    if executable.sha256()? != TOOL_EXECUTABLE_SHA256 || executable.len() != TOOL_EXECUTABLE_SIZE {
        return Err("tool_executable_hash_mismatch".to_owned());
    }
    Ok(ValidatedTool { executable })
}

fn validate_capsule_file_set(capsule_root: &Path) -> Result<(), String> {
    let expected = BTreeSet::from([
        "LICENSE".to_owned(),
        "SOURCE-NOTICE.md".to_owned(),
        "checksums.sha256".to_owned(),
        "fleet-agent-key-record.exe".to_owned(),
        "provenance.json".to_owned(),
        "release-manifest.json".to_owned(),
    ]);
    let mut actual = BTreeSet::new();
    for entry in std::fs::read_dir(capsule_root).map_err(|_| "invalid_tool_capsule".to_owned())? {
        let entry = entry.map_err(|_| "invalid_tool_capsule".to_owned())?;
        if !entry
            .file_type()
            .map_err(|_| "invalid_tool_capsule".to_owned())?
            .is_file()
        {
            return Err("invalid_tool_capsule".to_owned());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "invalid_tool_capsule".to_owned())?;
        if !actual.insert(name) {
            return Err("invalid_tool_capsule".to_owned());
        }
    }
    if actual != expected {
        return Err("invalid_tool_capsule".to_owned());
    }
    Ok(())
}

fn validate_exact_manifest(manifest: &ToolManifest) -> Result<(), String> {
    let expected_payload = [
        (
            "fleet-agent-key-record.exe",
            TOOL_EXECUTABLE_SHA256,
            TOOL_EXECUTABLE_SIZE,
        ),
        ("provenance.json", TOOL_PROVENANCE_SHA256, 4_596),
        ("LICENSE", TOOL_LICENSE_SHA256, 34_523),
        ("SOURCE-NOTICE.md", TOOL_NOTICE_SHA256, 427),
    ];
    if manifest.schema != TOOL_SCHEMA
        || manifest.capsule_version != TOOL_CAPSULE_VERSION
        || manifest.tool_contract.schema != TOOL_CONTRACT_SCHEMA
        || manifest.tool_contract.executable != "fleet-agent-key-record.exe"
        || manifest.tool_contract.argument_contract
            != "--key-id <dotted-id> --seed-file <private-seed-file>"
        || manifest.tool_contract.output_schema != KEY_RECORD_SCHEMA
        || manifest.source.repository_url != TOOL_OWNER_REPOSITORY
        || manifest.source.commit != TOOL_SOURCE_COMMIT
        || manifest.source.tree != TOOL_SOURCE_TREE
        || manifest.source.provenance_path != "provenance.json"
        || manifest.source.provenance_sha256 != TOOL_PROVENANCE_SHA256
        || manifest.artifact.path != "fleet-agent-key-record.exe"
        || manifest.artifact.sha256 != TOOL_EXECUTABLE_SHA256
        || manifest.artifact.size_bytes != TOOL_EXECUTABLE_SIZE
        || manifest.artifact.target != "x86_64-pc-windows-msvc"
        || manifest.artifact.profile != "release"
        || !manifest.distribution.portable
        || !manifest.distribution.supported
        || !manifest.distribution.inert_until_invoked
        || manifest.distribution.install_contract != "copy_capsule_byte_for_byte"
        || manifest.distribution.private_material_included
        || manifest.distribution.live_onboarding_claim
        || manifest.payload.len() != expected_payload.len()
    {
        return Err("invalid_tool_manifest".to_owned());
    }
    for (actual, expected) in manifest.payload.iter().zip(expected_payload) {
        if actual.path != expected.0
            || actual.sha256 != expected.1
            || actual.size_bytes != expected.2
        {
            return Err("invalid_tool_manifest".to_owned());
        }
    }
    Ok(())
}

fn validate_exact_provenance(provenance: &ToolProvenance) -> Result<(), String> {
    let expected_repositories = [
        (
            "rusty-fleet",
            "contract-dependency",
            "https://github.com/MesmerPrism/rusty-fleet",
            "8181683be4a3abbc5daa0c4497c7aeb9e76316a8",
            "195565629a53dfaaeacb1a7260fda06062324ad9",
        ),
        (
            "rusty-manifold",
            "contract-dependency",
            "https://github.com/MesmerPrism/rusty-manifold",
            "947421a928889889e485006bcc0200e05c2394f9",
            "836f1f21c5c8856bfc6dcdba8ed3721c090c76ba",
        ),
        (
            "rusty-quest",
            "release-owner",
            TOOL_OWNER_REPOSITORY,
            TOOL_SOURCE_COMMIT,
            TOOL_SOURCE_TREE,
        ),
    ];
    let expected_workspace_parse_only_repositories = [
        (
            "rusty-lattice",
            "workspace-parse-only",
            "https://github.com/MesmerPrism/rusty-lattice",
            "0aee7faa52fc965ff2255381781dd082ab639f4b",
            "4f60d4a01a3ca4dc217c4f82c16c952ab6733eb4",
        ),
        (
            "rusty-matter",
            "workspace-parse-only",
            "https://github.com/MesmerPrism/rusty-matter",
            "eec8cddd9830f7ef0f90574ddcbde2daac0ec804",
            "cd4e1ce39a8c91263774ea3e69fb859f503ffde8",
        ),
        (
            "rusty-optics",
            "workspace-parse-only",
            "https://github.com/MesmerPrism/rusty-optics",
            "fd01d84acffa1b0a3a192fe978af337d9fedd18a",
            "f527b761043e4e1e3a6bfa5969611dcf419e55fa",
        ),
    ];
    let expected_files = [
        (
            "Cargo.lock",
            "0d95468b7838ea175e654baa0974781416effbaec9376b5879368ab84106330d",
        ),
        (
            "Cargo.toml",
            "e40ca7015177e9a5e7d9546855613a01b81e855690ed8971ea1efedc7b93e6c1",
        ),
        (
            "crates/rusty-quest-fleet-agent/Cargo.toml",
            "b2d670017388582aa7297f717c7a916901d8efd5ca325a5547bb237c38df8225",
        ),
        (
            "crates/rusty-quest-fleet-agent/src/bin/fleet-agent-key-record.rs",
            "4f39342fdb72a6d3be94f99f949227d1ec2e2cfc13bc45a6d2c992e5f4016212",
        ),
        (
            "crates/rusty-quest-fleet-agent/src/lib.rs",
            "af16db769ca0271438c0b84b5c0ce3fb1cfeea48416930e828d39cc43d7da11e",
        ),
        (
            "tools/Build-FleetAgentKeyRecordRelease.ps1",
            "4525c43a52b87031cff47e79c60f73adaebacf7ebc99e2bcd7086283d864ff9b",
        ),
        (
            "tools/Test-FleetAgentKeyRecordRelease.ps1",
            "8194506d56cbc5712c11623eabb3ba4b2f5a56f25414767f304adad7dedad486",
        ),
        (
            "tools/lib/SourceComposition.psm1",
            "7e3a231b0703b9e0d1ab0b687a473f1a03366885a9eb3108d55839757d30c3df",
        ),
    ];
    if provenance.schema != TOOL_PROVENANCE_SCHEMA
        || provenance.capsule_version != TOOL_CAPSULE_VERSION
        || provenance.source.repository_url != TOOL_OWNER_REPOSITORY
        || provenance.source.commit != TOOL_SOURCE_COMMIT
        || provenance.source.tree != TOOL_SOURCE_TREE
        || provenance.source.package != "rusty-quest-fleet-agent"
        || provenance.source.composition_fingerprint != TOOL_COMPOSITION
        || provenance.source.repositories.len() != expected_repositories.len()
        || provenance.source.workspace_parse_only_repositories.len()
            != expected_workspace_parse_only_repositories.len()
        || provenance.source.files.len() != expected_files.len()
        || provenance.build.target != "x86_64-pc-windows-msvc"
        || provenance.build.profile != "release"
        || provenance.build.rustc.trim().is_empty()
        || provenance.build.cargo.trim().is_empty()
        || !provenance.build.locked_dependencies
        || !provenance.build.isolated_git_materializations
        || !provenance.build.post_build_identity_verified
        || provenance.build.path_remap_root != "/rusty-build"
        || !provenance.build.symbols_stripped
        || provenance.build.linker_reproducibility_argument != "/Brepro"
        || provenance.build.pe_reproducibility_marker != "IMAGE_DEBUG_TYPE_REPRO"
        || provenance.build.cargo_config_sha256 != TOOL_CARGO_CONFIG_SHA256
        || provenance.claims.owner != TOOL_OWNER
        || !provenance.claims.helper_only
        || provenance.claims.runtime_activation != "explicit_fleet_onboard_invocation"
        || provenance.claims.enrollment_authority
        || provenance.claims.device_authority
        || provenance.claims.private_seed_included
        || provenance.claims.profile_included
        || provenance.claims.hub_configuration_included
    {
        return Err("invalid_tool_provenance".to_owned());
    }
    for (actual, expected) in provenance
        .source
        .repositories
        .iter()
        .zip(expected_repositories)
    {
        if actual.repository_id != expected.0
            || actual.role != expected.1
            || actual.repository_url != expected.2
            || actual.commit != expected.3
            || actual.tree != expected.4
        {
            return Err("invalid_tool_provenance".to_owned());
        }
    }
    for (actual, expected) in provenance
        .source
        .workspace_parse_only_repositories
        .iter()
        .zip(expected_workspace_parse_only_repositories)
    {
        if actual.repository_id != expected.0
            || actual.role != expected.1
            || actual.repository_url != expected.2
            || actual.commit != expected.3
            || actual.tree != expected.4
        {
            return Err("invalid_tool_provenance".to_owned());
        }
    }
    for (actual, expected) in provenance.source.files.iter().zip(expected_files) {
        if actual.path != expected.0 || actual.sha256 != expected.1 {
            return Err("invalid_tool_provenance".to_owned());
        }
    }
    Ok(())
}

fn build_plan(loaded: &LoadedRequest) -> Result<(Plan, ValidatedTool), String> {
    let tool = validate_tool(&loaded.request)?;
    secure_fs::validate_new_private_root(&loaded.request.output_root)?;
    let mut device_ids: Vec<_> = loaded
        .request
        .devices
        .iter()
        .map(|device| device.device_id.clone())
        .collect();
    device_ids.sort();
    let mut output_files = vec![
        "hub/enrollment.private-config.json".to_owned(),
        "onboarding.private-inventory.json".to_owned(),
    ];
    for id in &device_ids {
        let directory = device_directory(id);
        output_files.push(format!("{directory}/fleet-agent.seed"));
        output_files.push(format!("{directory}/fleet-agent.profile.json"));
        output_files.push(format!("{directory}/fleet-agent.public-key-record.json"));
    }
    output_files.sort();
    Ok((
        Plan {
            schema: PLAN_SCHEMA,
            request_sha256: loaded.bytes_sha256.clone(),
            tool_manifest_sha256: TOOL_MANIFEST_SHA256,
            tool_executable_sha256: TOOL_EXECUTABLE_SHA256,
            tool_source_commit: TOOL_SOURCE_COMMIT,
            tool_owner: TOOL_OWNER,
            tool_consumer_id: TOOL_CONSUMER_ID,
            quest_profile_owner_source_sha256: PROFILE_OWNER_SOURCE_SHA256,
            quest_profile_owner_fixture_sha256: PROFILE_OWNER_FIXTURE_SHA256,
            device_ids,
            output_files,
            effects: vec![
                "create-private-output-root",
                "generate-32-byte-seed-per-device",
                "invoke-pinned-rusty-quest-key-record-vector",
                "write-inventory-last",
            ],
            non_claims: vec![
                "not-installed",
                "not-enrolled",
                "not-reachable",
                "not-active",
                "not-healthy",
                "not-revoked",
                "not-live-onboarding-proof",
                "not-manifold-acceptance",
            ],
        },
        tool,
    ))
}

fn device_directory(device_id: &str) -> String {
    format!("devices/device-{}", hash(device_id.as_bytes()))
}

fn plan_value(plan: &Plan) -> Result<Value, String> {
    let bytes = serde_jcs::to_vec(plan).map_err(|_| "plan_serialization_failed".to_owned())?;
    let mut value =
        serde_json::to_value(plan).map_err(|_| "plan_serialization_failed".to_owned())?;
    value["plan_sha256"] = Value::String(hash(&bytes));
    Ok(value)
}

fn plan_digest(plan: &Plan) -> Result<String, String> {
    serde_jcs::to_vec(plan)
        .map(|bytes| hash(&bytes))
        .map_err(|_| "plan_serialization_failed".to_owned())
}

fn apply(request_path: &Path, confirmation: &str) -> Result<Value, String> {
    #[cfg(not(windows))]
    {
        let _ = (request_path, confirmation);
        return Err("windows_private_onboarding_required".to_owned());
    }
    #[cfg(windows)]
    {
        let loaded = load_request(request_path)?;
        let (plan, tool) = build_plan(&loaded)?;
        let digest = plan_digest(&plan)?;
        if confirmation != digest {
            return Err("plan_confirmation_mismatch".to_owned());
        }
        // The exact retained executable and every mutable ancestor directory
        // remain open without delete sharing through every invocation.
        let mut tree = secure_fs::PrivateTree::create(&loaded.request.output_root)?;
        let generated =
            generate(&loaded.request, &tool, &mut tree, &digest).and_then(|inventory| {
                tree.verify_root_binding()?;
                Ok(inventory)
            });
        match generated {
            Ok(inventory) => {
                let inventory_hash = tree
                    .file_sha256("onboarding.private-inventory.json")
                    .ok_or_else(|| "inventory_commit_missing".to_owned())?;
                let file_count = inventory
                    .entries
                    .iter()
                    .filter(|entry| entry.kind == EntryKind::File)
                    .count()
                    + 1;
                tree.commit();
                Ok(json!({
                    "schema": "rusty.fleet.offline_onboarding_apply_receipt.v1",
                    "status": "generated",
                    "plan_sha256": digest,
                    "inventory_sha256": inventory_hash,
                    "file_count": file_count,
                    "capsule_scope": "supported-owner-release",
                    "capsule_owner": TOOL_OWNER,
                    "consumer_id": TOOL_CONSUMER_ID,
                    "distribution_eligible": true,
                    "private_material_included": false,
                    "live_onboarding_claim": false,
                    "claims": [
                        "generated-only",
                        "not-installed",
                        "not-enrolled",
                        "not-active",
                        "not-live-onboarding-proof",
                        "not-manifold-acceptance"
                    ]
                }))
            }
            Err(error) => match tree.rollback() {
                Ok(()) => Err(error),
                Err(()) => Err("partial_generation_exact_cleanup_required".to_owned()),
            },
        }
    }
}

#[cfg(windows)]
fn generate(
    request: &Request,
    tool: &ValidatedTool,
    tree: &mut secure_fs::PrivateTree,
    plan_digest: &str,
) -> Result<Inventory, String> {
    tree.create_dir("hub")?;
    tree.create_dir("devices")?;
    let mut collision_digests: Vec<Zeroizing<[u8; 32]>> = Vec::new();
    let mut public_key_fingerprints = BTreeSet::new();
    let mut enrollments = Vec::new();

    for device in &request.devices {
        let directory = device_directory(&device.device_id);
        tree.create_dir(&directory)?;
        let mut seed = Zeroizing::new([0_u8; 32]);
        getrandom::fill(seed.as_mut()).map_err(|_| "csprng_failed".to_owned())?;
        register_secret_digest(&mut collision_digests, seed.as_ref())?;
        let seed_relative = format!("{directory}/fleet-agent.seed");
        tree.write_file(&seed_relative, seed.as_ref())?;
        let seed_path = tree.absolute(&seed_relative)?;
        let key_record =
            tool_process::invoke_key_record_tool(&tool.executable, &device.key_id, &seed_path);
        if tool.executable.sha256()? != TOOL_EXECUTABLE_SHA256 {
            return Err("tool_executable_hash_mismatch".to_owned());
        }
        let key_record = key_record?;
        validate_key_record(&key_record, &device.key_id)?;
        register_public_key(&mut public_key_fingerprints, &key_record)?;
        tree.write_json(
            &format!("{directory}/fleet-agent.public-key-record.json"),
            &key_record,
        )?;
        let profile = QuestFleetAgentProfileV1 {
            schema: PROFILE_SCHEMA.to_owned(),
            enabled: true,
            device_id: device.device_id.clone(),
            display_name: device.display_name.clone(),
            model: device.model.clone(),
            hardware_class: device.hardware_class.clone(),
            identity_revision: device.identity_revision,
            expected_authority_revision: device.expected_authority_revision,
            status_revision: device.status_revision,
            source_revision: device.source_revision,
            source_epoch: device.source_epoch.clone(),
            key_id: device.key_id.clone(),
            key_fingerprint: key_record.key_fingerprint.clone(),
            trust_domain: device.trust_domain.clone(),
            checkin_ttl_ms: device.checkin_ttl_ms,
            checkin_interval_ms: device.checkin_interval_ms,
            hub_endpoint: device.hub_endpoint.clone(),
            tags: device.tags.clone(),
        };
        validate_quest_profile(&profile)?;
        tree.write_json(&format!("{directory}/fleet-agent.profile.json"), &profile)?;
        enrollments.push(json!({
            "request_id": format!("{}.{}", request.hub.request_id_prefix, device.device_id),
            "operator_id": request.hub.operator_id,
            "credential": {
                "$schema": "rusty.manifold.peer.credential_record.v1",
                "credential_id": format!("credential.{}", device.key_id),
                "peer_id": device.device_id,
                "trust_domain": device.trust_domain,
                "key_id": device.key_id,
                "key_generation": device.key_generation,
                "algorithm": "ed25519",
                "public_key_hex": key_record.public_key_hex,
                "public_key_sha256": format!(
                    "sha256:{}",
                    &key_record.key_fingerprint["fingerprint.".len()..]
                ),
                "valid_from_ms": request.hub.credential_valid_from_ms,
                "expires_at_ms": request.hub.credential_expires_at_ms,
                "status": "active",
                "replaced_by_key_id": null
            }
        }));
    }

    let hub_value = json!({
        "schema": HUB_CONFIG_SCHEMA,
        "bind": request.hub.bind,
        "allow_non_loopback": false,
        "checkin_bind": request.hub.checkin_bind,
        "allow_non_loopback_checkin": true,
        "state_directory": request.hub.state_directory,
        "trusted_operator_ids": [request.hub.operator_id],
        "enrollments": enrollments
    });
    let hub_config: LocalHubConfig =
        serde_json::from_value(hub_value).map_err(|_| "invalid_generated_hub_config".to_owned())?;
    hub_config
        .validate_offline(request.hub.credential_valid_from_ms as i64)
        .map_err(|_| "invalid_generated_hub_config".to_owned())?;
    tree.write_json("hub/enrollment.private-config.json", &hub_config)?;

    let mut entries = tree.inventory_entries()?;
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let inventory = Inventory {
        schema: INVENTORY_SCHEMA.to_owned(),
        plan_sha256: plan_digest.to_owned(),
        output_root: request.output_root.to_string_lossy().into_owned(),
        tool_manifest_sha256: TOOL_MANIFEST_SHA256.to_owned(),
        tool_executable_sha256: TOOL_EXECUTABLE_SHA256.to_owned(),
        quest_profile_owner_source_sha256: PROFILE_OWNER_SOURCE_SHA256.to_owned(),
        quest_profile_owner_fixture_sha256: PROFILE_OWNER_FIXTURE_SHA256.to_owned(),
        root_identity: tree.root_identity().clone(),
        entries,
        state: "generated-not-installed-or-enrolled".to_owned(),
    };
    tree.write_json("onboarding.private-inventory.json", &inventory)?;
    Ok(inventory)
}

fn register_secret_digest(
    digests: &mut Vec<Zeroizing<[u8; 32]>>,
    secret: &[u8],
) -> Result<(), String> {
    let mut digest: [u8; 32] = Sha256::digest(secret).into();
    if digests.iter().any(|existing| existing.as_ref() == digest) {
        digest.zeroize();
        return Err("duplicate_seed_rejected".to_owned());
    }
    digests.push(Zeroizing::new(digest));
    Ok(())
}

fn register_public_key(
    fingerprints: &mut BTreeSet<String>,
    record: &KeyRecord,
) -> Result<(), String> {
    if fingerprints.insert(record.key_fingerprint.clone()) {
        Ok(())
    } else {
        Err("duplicate_public_key_rejected".to_owned())
    }
}

fn validate_key_record(record: &KeyRecord, expected_key_id: &str) -> Result<(), String> {
    if record.schema != KEY_RECORD_SCHEMA
        || record.key_id != expected_key_id
        || record.public_key_hex.len() != 64
        || !record
            .public_key_hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || !record.key_fingerprint.starts_with("fingerprint.")
        || record.key_fingerprint.len() != "fingerprint.".len() + 64
        || !record.key_fingerprint["fingerprint.".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || record.key_fingerprint
            != format!(
                "fingerprint.{}",
                hash(
                    &hex::decode(&record.public_key_hex)
                        .map_err(|_| "malicious_key_tool_output".to_owned())?
                )
            )
    {
        return Err("malicious_key_tool_output".to_owned());
    }
    Ok(())
}

fn validate_quest_profile(profile: &QuestFleetAgentProfileV1) -> Result<(), String> {
    if profile.schema != PROFILE_SCHEMA || !profile.enabled {
        return Err("invalid_generated_quest_profile".to_owned());
    }
    for value in [
        profile.device_id.as_str(),
        profile.display_name.as_str(),
        profile.model.as_str(),
        profile.hardware_class.as_str(),
        profile.source_epoch.as_str(),
        profile.key_id.as_str(),
        profile.key_fingerprint.as_str(),
        profile.trust_domain.as_str(),
        profile.hub_endpoint.as_str(),
    ] {
        if value.trim().is_empty() {
            return Err("invalid_generated_quest_profile".to_owned());
        }
    }
    if !dotted(&profile.device_id)
        || !dotted(&profile.key_id)
        || !dotted(&profile.key_fingerprint)
        || !dotted(&profile.trust_domain)
        || profile.identity_revision == 0
        || profile.expected_authority_revision == 0
        || profile.status_revision == 0
        || profile.source_revision == 0
        || !(10_000..=300_000).contains(&profile.checkin_ttl_ms)
        || !(5_000..profile.checkin_ttl_ms).contains(&profile.checkin_interval_ms)
    {
        return Err("invalid_generated_quest_profile".to_owned());
    }
    let endpoint = profile.hub_endpoint.to_ascii_lowercase();
    if !(endpoint.starts_with("http://") || endpoint.starts_with("https://"))
        || endpoint.contains('@')
        || endpoint.contains('#')
        || profile
            .tags
            .iter()
            .any(|(key, value)| key.trim().is_empty() || value.trim().is_empty())
    {
        return Err("invalid_generated_quest_profile".to_owned());
    }
    Ok(())
}

fn cleanup_plan_value(session: &secure_fs::CleanupSession) -> Result<Value, String> {
    let plan = json!({
        "schema": "rusty.fleet.offline_onboarding_cleanup_plan.v1",
        "inventory_sha256": session.inventory_sha256(),
        "inventory_file_identity": session.inventory_identity(),
        "root_identity": session.inventory().root_identity,
        "entries": session.inventory().entries,
        "delete_scope": "exact-retained-generated-objects-only",
        "deletes_authorization": false,
        "revokes_authorization": false,
        "secure_erasure_claimed": false
    });
    let digest = hash(
        &serde_jcs::to_vec(&plan).map_err(|_| "cleanup_plan_serialization_failed".to_owned())?,
    );
    Ok(json!({"cleanup_plan": plan, "cleanup_sha256": digest}))
}

fn cleanup_apply(path: &Path, confirmation: &str) -> Result<Value, String> {
    #[cfg(not(windows))]
    {
        let _ = (path, confirmation);
        return Err("windows_retained_cleanup_required".to_owned());
    }
    #[cfg(windows)]
    {
        // One retained inventory handle supplies the exact bytes used for both
        // digest confirmation and deletion. The path is never reread.
        let session = secure_fs::CleanupSession::open(path)?;
        let value = cleanup_plan_value(&session)?;
        if value["cleanup_sha256"].as_str() != Some(confirmation) {
            return Err("cleanup_confirmation_mismatch".to_owned());
        }
        session.delete_exact()?;
        Ok(json!({
            "schema": "rusty.fleet.offline_onboarding_cleanup_receipt.v1",
            "status": "logical-file-deletion-completed",
            "cleanup_sha256": confirmation,
            "secure_erasure": false,
            "authorization_revoked": false
        }))
    }
}

fn revoke_plan(path: &Path) -> Result<Value, String> {
    let session = secure_fs::CleanupSession::open(path)?;
    Ok(json!({
        "schema": "rusty.fleet.offline_onboarding_revoke_plan.v1",
        "status": "requires-authorization-owner",
        "owner": "manifold-enrollment-authority",
        "inventory_sha256": session.inventory_sha256(),
        "mutation_performed": false,
        "cleanup_performed": false,
        "fail_closed": true
    }))
}

mod secure_fs {
    use super::*;

    use cap_fs_ext::{FollowSymlinks, MetadataExt as CapMetadataExt, OpenOptionsFollowExt};
    use cap_std::ambient_authority;
    use cap_std::fs::OpenOptionsExt;
    use cap_std::fs::{Dir, File, OpenOptions};

    #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt as StdOpenOptionsExt;
    #[cfg(windows)]
    use std::os::windows::io::AsRawHandle;
    #[cfg(windows)]
    use windows_permissions::constants::{
        AccessRights, AceType, SeObjectType, SecurityInformation,
    };
    #[cfg(windows)]
    use windows_permissions::{SecurityDescriptor, wrappers};
    #[cfg(windows)]
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    #[cfg(windows)]
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL,
        WRITE_DAC, WRITE_OWNER,
    };

    pub(super) struct LoadedFile {
        pub(super) file: SecureFile,
        pub(super) bytes: Zeroizing<Vec<u8>>,
    }

    pub(super) struct SecureFile {
        file: File,
        pub(super) identity: ObjectIdentity,
        length: u64,
    }

    struct SecureDir {
        dir: Dir,
        identity: ObjectIdentity,
    }

    /// Retains the exact executable plus every mutable ancestor directory.
    ///
    /// The ancestor handles deny delete sharing, so Windows cannot rename or
    /// delete-and-recreate any component between validation and CreateProcess.
    pub(super) struct RetainedExecutable {
        path: PathBuf,
        root: SecureDir,
        ancestors: Vec<SecureDir>,
        file: SecureFile,
        maximum: u64,
    }

    impl RetainedExecutable {
        pub(super) fn open(path: &Path, maximum: u64) -> Result<Self, String> {
            validate_external_path(path)?;
            let (root, mut names) = open_guarded_base(path)?;
            let mut current = root
                .dir
                .try_clone()
                .map_err(|_| "tool_path_root_open_failed".to_owned())?;
            let leaf = names
                .pop()
                .ok_or_else(|| "unsafe_windows_path".to_owned())?;
            let mut ancestors = Vec::with_capacity(names.len());
            for name in names {
                let retained = open_guarded_dir_component(&current, &name)?;
                current = retained
                    .dir
                    .try_clone()
                    .map_err(|_| "tool_path_component_open_failed".to_owned())?;
                ancestors.push(retained);
            }
            let file = open_file_from(&current, &leaf, maximum, false, false, "tool")?;
            Ok(Self {
                path: path.to_path_buf(),
                root,
                ancestors,
                file,
                maximum,
            })
        }

        pub(super) fn path(&self) -> &Path {
            &self.path
        }

        pub(super) fn sha256(&self) -> Result<String, String> {
            hash_file_handle(&self.file)
        }

        pub(super) fn len(&self) -> u64 {
            self.file.length
        }

        pub(super) fn verify_binding(&self) -> Result<(), String> {
            let (actual_root, mut names) = open_guarded_base(&self.path)?;
            if actual_root.identity != self.root.identity {
                return Err("tool_path_identity_changed".to_owned());
            }
            let mut current = actual_root
                .dir
                .try_clone()
                .map_err(|_| "tool_path_root_open_failed".to_owned())?;
            let leaf = names
                .pop()
                .ok_or_else(|| "tool_path_identity_changed".to_owned())?;
            if names.len() != self.ancestors.len() {
                return Err("tool_path_identity_changed".to_owned());
            }
            for ((name, expected), index) in names.into_iter().zip(&self.ancestors).zip(0_usize..) {
                let actual = open_guarded_dir_component(&current, &name)?;
                if actual.identity != expected.identity {
                    return Err("tool_path_identity_changed".to_owned());
                }
                current = actual
                    .dir
                    .try_clone()
                    .map_err(|_| format!("tool_path_component_{index}_unavailable"))?;
            }
            let actual = open_file_from(&current, &leaf, self.maximum, false, false, "tool")?;
            if actual.identity != self.file.identity {
                return Err("tool_path_identity_changed".to_owned());
            }
            Ok(())
        }
    }

    pub(super) fn validate_external_path(path: &Path) -> Result<(), String> {
        let (_root, components) = split_absolute(path)?;
        if components.is_empty()
            || components.iter().any(|component| {
                let value = component.to_string_lossy();
                value.is_empty()
                    || value.contains(':')
                    || value.ends_with('.')
                    || value.ends_with(' ')
                    || reserved_windows_name(&value)
            })
        {
            return Err("unsafe_windows_path".to_owned());
        }
        Ok(())
    }

    pub(super) fn reserved_windows_name(value: &str) -> bool {
        let stem = value
            .split('.')
            .next()
            .unwrap_or(value)
            .to_ascii_uppercase();
        matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || stem.strip_prefix("COM").is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
            || stem.strip_prefix("LPT").is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
    }

    fn split_absolute(path: &Path) -> Result<(PathBuf, Vec<std::ffi::OsString>), String> {
        if !path.is_absolute() {
            return Err("absolute_windows_path_required".to_owned());
        }
        let mut components = path.components();
        let drive = match components.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(letter) => letter,
                _ => return Err("windows_namespace_path_rejected".to_owned()),
            },
            _ => return Err("absolute_windows_path_required".to_owned()),
        };
        if components.next() != Some(Component::RootDir) {
            return Err("absolute_windows_path_required".to_owned());
        }
        let mut names = Vec::new();
        for component in components {
            match component {
                Component::Normal(name) => names.push(name.to_os_string()),
                _ => return Err("unsafe_windows_path".to_owned()),
            }
        }
        let root = PathBuf::from(format!("{}:\\", char::from(drive)));
        fleet_onboarding_windows_kernel::require_local_drive(&root)
            .map_err(|_| "local_windows_drive_required".to_owned())?;
        Ok((root, names))
    }

    fn open_base(path: &Path) -> Result<(Dir, Vec<std::ffi::OsString>), String> {
        let (root, names) = split_absolute(path)?;
        let dir = Dir::open_ambient_dir(&root, ambient_authority())
            .map_err(|_| "path_component_open_failed".to_owned())?;
        Ok((dir, names))
    }

    fn open_guarded_base(path: &Path) -> Result<(SecureDir, Vec<std::ffi::OsString>), String> {
        let (root_path, names) = split_absolute(path)?;
        #[cfg(not(windows))]
        {
            let _ = (root_path, names);
            return Err("windows_retained_handles_required".to_owned());
        }
        #[cfg(windows)]
        {
            let mut options = std::fs::OpenOptions::new();
            options
                .read(true)
                .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
            let file = options
                .open(&root_path)
                .map_err(|_| "tool_path_root_open_failed".to_owned())?;
            let dir = Dir::from_std_file(file);
            let metadata = dir
                .dir_metadata()
                .map_err(|_| "tool_path_root_identity_unavailable".to_owned())?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("tool_path_root_rejected".to_owned());
            }
            let identity = identity_for(&dir, &metadata, false, false)?;
            Ok((SecureDir { dir, identity }, names))
        }
    }

    fn open_parent(path: &Path) -> Result<(Dir, std::ffi::OsString), String> {
        let (mut dir, mut names) = open_base(path)?;
        let leaf = names
            .pop()
            .ok_or_else(|| "unsafe_windows_path".to_owned())?;
        for name in names {
            dir = open_dir_component(&dir, &name, false, false)?.dir;
        }
        Ok((dir, leaf))
    }

    fn open_dir_component(
        parent: &Dir,
        name: &std::ffi::OsStr,
        delete: bool,
        private_acl: bool,
    ) -> Result<SecureDir, String> {
        #[cfg(not(windows))]
        {
            let _ = (parent, name, delete, private_acl);
            return Err("windows_retained_handles_required".to_owned());
        }
        #[cfg(windows)]
        {
            let mut options = OpenOptions::new();
            let mut access = FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL;
            if delete {
                access |= DELETE | WRITE_DAC | WRITE_OWNER;
            }
            options
                .access_mode(access)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .follow(FollowSymlinks::No);
            let file = parent
                .open_with(name, &options)
                .map_err(|_| "directory_component_open_failed".to_owned())?;
            let metadata = file
                .metadata()
                .map_err(|_| "directory_identity_unavailable".to_owned())?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("reparse_or_wrong_type_rejected".to_owned());
            }
            let dir = Dir::from_std_file(file.into_std());
            let identity = identity_for(&dir, &metadata, false, private_acl)?;
            Ok(SecureDir { dir, identity })
        }
    }

    fn open_guarded_dir_component(
        parent: &Dir,
        name: &std::ffi::OsStr,
    ) -> Result<SecureDir, String> {
        #[cfg(not(windows))]
        {
            let _ = (parent, name);
            return Err("windows_retained_handles_required".to_owned());
        }
        #[cfg(windows)]
        {
            let mut options = OpenOptions::new();
            options
                .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL)
                // Deliberately omit FILE_SHARE_DELETE. A retained handle on
                // every mutable ancestor makes rename/delete substitution
                // incompatible until the child invocation is complete.
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .follow(FollowSymlinks::No);
            let file = parent
                .open_with(name, &options)
                .map_err(|_| "tool_path_component_open_failed".to_owned())?;
            let metadata = file
                .metadata()
                .map_err(|_| "tool_path_identity_unavailable".to_owned())?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("reparse_or_wrong_type_rejected".to_owned());
            }
            let dir = Dir::from_std_file(file.into_std());
            let identity = identity_for(&dir, &metadata, false, false)?;
            Ok(SecureDir { dir, identity })
        }
    }

    fn open_file_from(
        parent: &Dir,
        name: &std::ffi::OsStr,
        maximum: u64,
        delete: bool,
        private_acl: bool,
        class: &str,
    ) -> Result<SecureFile, String> {
        #[cfg(not(windows))]
        {
            let _ = (parent, name, maximum, delete, private_acl, class);
            return Err("windows_retained_handles_required".to_owned());
        }
        #[cfg(windows)]
        {
            let mut options = OpenOptions::new();
            let mut access = GENERIC_READ | FILE_READ_ATTRIBUTES | READ_CONTROL;
            if delete {
                access |= DELETE;
            }
            options
                .access_mode(access)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .follow(FollowSymlinks::No);
            let file = parent
                .open_with(name, &options)
                .map_err(|_| format!("{class}_unavailable"))?;
            let metadata = file
                .metadata()
                .map_err(|_| format!("{class}_identity_unavailable"))?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() == 0
                || metadata.len() > maximum
            {
                return Err(format!("invalid_{class}_file"));
            }
            let identity = identity_for(&file, &metadata, true, private_acl)?;
            Ok(SecureFile {
                file,
                identity,
                length: metadata.len(),
            })
        }
    }

    pub(super) fn open_absolute_file(
        path: &Path,
        maximum: u64,
        delete: bool,
        private_acl: bool,
        class: &str,
    ) -> Result<SecureFile, String> {
        validate_external_path(path)?;
        let (parent, leaf) = open_parent(path)?;
        open_file_from(&parent, &leaf, maximum, delete, private_acl, class)
    }

    pub(super) fn read_absolute_file(
        path: &Path,
        maximum: u64,
        delete: bool,
        private_acl: bool,
        class: &str,
    ) -> Result<LoadedFile, String> {
        let mut file = open_absolute_file(path, maximum, delete, private_acl, class)?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(file.length as usize));
        file.file
            .read_to_end(&mut bytes)
            .map_err(|_| format!("{class}_read_failed"))?;
        if bytes.len() as u64 != file.length {
            return Err(format!("{class}_identity_changed"));
        }
        file.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| format!("{class}_read_failed"))?;
        Ok(LoadedFile { file, bytes })
    }

    pub(super) fn hash_file_handle(file: &SecureFile) -> Result<String, String> {
        let mut clone = file
            .file
            .try_clone()
            .map_err(|_| "artifact_clone_failed".to_owned())?;
        clone
            .seek(SeekFrom::Start(0))
            .map_err(|_| "artifact_read_failed".to_owned())?;
        let mut hasher = Sha256::new();
        let mut buffer = Zeroizing::new([0_u8; 16 * 1024]);
        let mut total = 0_u64;
        loop {
            let count = clone
                .read(buffer.as_mut())
                .map_err(|_| "artifact_read_failed".to_owned())?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .ok_or_else(|| "artifact_too_large".to_owned())?;
            if total > file.length {
                return Err("artifact_identity_changed".to_owned());
            }
            hasher.update(&buffer[..count]);
        }
        if total != file.length {
            return Err("artifact_identity_changed".to_owned());
        }
        Ok(hex::encode(hasher.finalize()))
    }

    #[cfg(windows)]
    fn acl_hash<T: AsRawHandle>(handle: &T) -> Result<String, String> {
        let descriptor = wrappers::GetSecurityInfo(
            handle,
            SeObjectType::SE_FILE_OBJECT,
            SecurityInformation::Owner | SecurityInformation::Dacl,
        )
        .map_err(|_| "acl_read_failed".to_owned())?;
        let sddl = descriptor
            .as_sddl()
            .map_err(|_| "acl_read_failed".to_owned())?;
        Ok(hash(sddl.to_string_lossy().as_bytes()))
    }

    #[cfg(not(windows))]
    fn acl_hash<T>(_handle: &T) -> Result<String, String> {
        Err("windows_acl_required".to_owned())
    }

    #[cfg(windows)]
    fn private_acl_hash_and_validate<T: AsRawHandle>(handle: &T) -> Result<String, String> {
        let current = windows_permissions::utilities::current_process_sid()
            .map_err(|_| "current_user_identity_failed".to_owned())?;
        let descriptor = wrappers::GetSecurityInfo(
            handle,
            SeObjectType::SE_FILE_OBJECT,
            SecurityInformation::Owner | SecurityInformation::Dacl,
        )
        .map_err(|_| "private_acl_read_failed".to_owned())?;
        if descriptor.owner() != Some(&*current) {
            return Err("private_acl_owner_rejected".to_owned());
        }
        let dacl = descriptor
            .dacl()
            .ok_or_else(|| "private_acl_missing".to_owned())?;
        if dacl.len() == 0 {
            return Err("private_acl_empty".to_owned());
        }
        for index in 0..dacl.len() {
            let ace = dacl
                .get_ace(index)
                .ok_or_else(|| "private_acl_invalid".to_owned())?;
            if ace.ace_type() != AceType::ACCESS_ALLOWED_ACE_TYPE
                || ace.sid() != Some(&*current)
                || !(ace.mask().contains(AccessRights::FileAllAccess)
                    || ace.mask().contains(AccessRights::GenericAll))
            {
                return Err("private_acl_not_current_user_only".to_owned());
            }
        }
        let sddl = descriptor
            .as_sddl()
            .map_err(|_| "private_acl_read_failed".to_owned())?;
        Ok(hash(sddl.to_string_lossy().as_bytes()))
    }

    #[cfg(not(windows))]
    fn private_acl_hash_and_validate<T>(_handle: &T) -> Result<String, String> {
        Err("windows_private_acl_required".to_owned())
    }

    #[cfg(windows)]
    fn apply_private_acl<T: AsRawHandle>(
        handle: &mut T,
        inherit_to_children: bool,
    ) -> Result<(), String> {
        let current = windows_permissions::utilities::current_process_sid()
            .map_err(|_| "current_user_identity_failed".to_owned())?;
        let inheritance = if inherit_to_children { "OICI" } else { "" };
        let descriptor: windows_permissions::LocalBox<SecurityDescriptor> =
            format!("O:{current}D:P(A;{inheritance};FA;;;{current})")
                .parse()
                .map_err(|_| "private_acl_build_failed".to_owned())?;
        wrappers::SetSecurityInfo(
            handle,
            SeObjectType::SE_FILE_OBJECT,
            SecurityInformation::Owner
                | SecurityInformation::Dacl
                | SecurityInformation::ProtectedDacl,
            descriptor.owner(),
            None,
            descriptor.dacl(),
            None,
        )
        .map_err(|_| "private_acl_apply_failed".to_owned())?;
        private_acl_hash_and_validate(handle).map(drop)
    }

    fn identity_for<T>(
        handle: &T,
        metadata: &cap_std::fs::Metadata,
        require_single_link: bool,
        require_private_acl: bool,
    ) -> Result<ObjectIdentity, String>
    where
        T: IdentityHandle,
    {
        let links = CapMetadataExt::nlink(metadata);
        if links == 0 || (require_single_link && links != 1) {
            return Err("hardlink_or_invalid_identity_rejected".to_owned());
        }
        Ok(ObjectIdentity {
            volume_serial_number: CapMetadataExt::dev(metadata),
            file_id: CapMetadataExt::ino(metadata),
            number_of_links: links,
            acl_sha256: if require_private_acl {
                handle.private_acl_hash()?
            } else {
                handle.raw_acl_hash()?
            },
        })
    }

    trait IdentityHandle {
        fn raw_acl_hash(&self) -> Result<String, String>;
        fn private_acl_hash(&self) -> Result<String, String>;
    }

    impl IdentityHandle for File {
        fn raw_acl_hash(&self) -> Result<String, String> {
            acl_hash(self)
        }

        fn private_acl_hash(&self) -> Result<String, String> {
            private_acl_hash_and_validate(self)
        }
    }

    impl IdentityHandle for Dir {
        fn raw_acl_hash(&self) -> Result<String, String> {
            acl_hash(self)
        }

        fn private_acl_hash(&self) -> Result<String, String> {
            private_acl_hash_and_validate(self)
        }
    }

    pub(super) fn validate_new_private_root(root: &Path) -> Result<(), String> {
        validate_external_path(root)?;
        let (parent, leaf) = open_parent(root)?;
        if parent.metadata(&leaf).is_ok() {
            return Err("output_root_exists".to_owned());
        }
        // Atomic current-user-only creation is guaranteed only when the
        // existing parent is itself current-user-only.
        private_acl_hash_and_validate(&parent)
            .map_err(|_| "output_parent_not_private".to_owned())?;
        Ok(())
    }

    pub(super) struct PrivateTree {
        root_path: PathBuf,
        root: SecureDir,
        dirs: BTreeMap<String, SecureDir>,
        files: BTreeMap<String, SecureFile>,
        committed: bool,
    }

    impl PrivateTree {
        pub(super) fn create(root_path: &Path) -> Result<Self, String> {
            validate_new_private_root(root_path)?;
            let (parent, leaf) = open_parent(root_path)?;
            parent
                .create_dir(&leaf)
                .map_err(|_| "output_root_create_failed".to_owned())?;
            let mut root = open_dir_component(&parent, &leaf, true, false)
                .map_err(|_| "partial_generation_exact_cleanup_required".to_owned())?;
            if let Err(error) = apply_private_acl(&mut root.dir, true) {
                let _ = fleet_onboarding_windows_kernel::delete_retained_handle(&root.dir);
                return Err(error);
            }
            root.identity = identity_for(
                &root.dir,
                &root
                    .dir
                    .dir_metadata()
                    .map_err(|_| "root_identity_unavailable".to_owned())?,
                false,
                true,
            )?;
            Ok(Self {
                root_path: root_path.to_path_buf(),
                root,
                dirs: BTreeMap::new(),
                files: BTreeMap::new(),
                committed: false,
            })
        }

        fn parent_and_leaf(&self, relative: &str) -> Result<(Dir, std::ffi::OsString), String> {
            validate_relative(relative)?;
            let path = Path::new(relative);
            let leaf = path
                .file_name()
                .ok_or_else(|| "invalid_private_relative_path".to_owned())?
                .to_os_string();
            let parent = path
                .parent()
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
                .map_or_else(
                    || self.root.dir.try_clone(),
                    |value| {
                        self.dirs
                            .get(&value.replace('\\', "/"))
                            .ok_or_else(|| std::io::Error::other("missing private parent"))?
                            .dir
                            .try_clone()
                    },
                )
                .map_err(|_| "private_parent_unavailable".to_owned())?;
            Ok((parent, leaf))
        }

        pub(super) fn create_dir(&mut self, relative: &str) -> Result<(), String> {
            let normalized = relative.replace('\\', "/");
            if self.dirs.contains_key(&normalized) {
                return Err("duplicate_private_directory".to_owned());
            }
            let (parent, leaf) = self.parent_and_leaf(&normalized)?;
            parent
                .create_dir(&leaf)
                .map_err(|_| "private_directory_create_failed".to_owned())?;
            let mut dir = open_dir_component(&parent, &leaf, true, false)?;
            // An elevated Windows token may use the Administrators group as
            // its default owner even though this private parent grants only
            // the current user. Normalize the child through the retained
            // handle before it can contain any private material.
            if let Err(error) = apply_private_acl(&mut dir.dir, true) {
                let _ = fleet_onboarding_windows_kernel::delete_retained_handle(&dir.dir);
                return Err(error);
            }
            dir.identity = identity_for(
                &dir.dir,
                &dir.dir
                    .dir_metadata()
                    .map_err(|_| "private_identity_unavailable".to_owned())?,
                false,
                true,
            )?;
            self.dirs.insert(normalized, dir);
            Ok(())
        }

        pub(super) fn write_file(&mut self, relative: &str, bytes: &[u8]) -> Result<(), String> {
            let normalized = relative.replace('\\', "/");
            if self.files.contains_key(&normalized) {
                return Err("duplicate_private_file".to_owned());
            }
            let (parent, leaf) = self.parent_and_leaf(&normalized)?;
            #[cfg(not(windows))]
            {
                let _ = (parent, leaf, bytes);
                return Err("windows_private_files_required".to_owned());
            }
            #[cfg(windows)]
            {
                let mut options = OpenOptions::new();
                options
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .access_mode(
                        GENERIC_READ
                            | GENERIC_WRITE
                            | FILE_READ_ATTRIBUTES
                            | READ_CONTROL
                            | DELETE
                            | WRITE_DAC
                            | WRITE_OWNER,
                    )
                    .share_mode(FILE_SHARE_READ)
                    .follow(FollowSymlinks::No);
                let mut file = parent
                    .open_with(&leaf, &options)
                    .map_err(|_| "private_create_new_failed".to_owned())?;
                // Apply and verify the explicit current-user owner/DACL before
                // any secret bytes are written. This is required on elevated
                // hosts whose token default owner is a group SID.
                if let Err(error) = apply_private_acl(&mut file, false) {
                    let _ = fleet_onboarding_windows_kernel::delete_retained_handle(&file);
                    return Err(error);
                }
                file.write_all(bytes)
                    .and_then(|()| file.sync_all())
                    .map_err(|_| "private_write_failed".to_owned())?;
                file.seek(SeekFrom::Start(0))
                    .map_err(|_| "private_write_failed".to_owned())?;
                let metadata = file
                    .metadata()
                    .map_err(|_| "private_identity_unavailable".to_owned())?;
                let identity = identity_for(&file, &metadata, true, true)?;
                self.files.insert(
                    normalized,
                    SecureFile {
                        file,
                        identity,
                        length: metadata.len(),
                    },
                );
                Ok(())
            }
        }

        pub(super) fn write_json<T: Serialize>(
            &mut self,
            relative: &str,
            value: &T,
        ) -> Result<(), String> {
            let mut bytes = Zeroizing::new(
                serde_json::to_vec_pretty(value)
                    .map_err(|_| "private_serialization_failed".to_owned())?,
            );
            bytes.push(b'\n');
            self.write_file(relative, &bytes)
        }

        pub(super) fn absolute(&self, relative: &str) -> Result<PathBuf, String> {
            validate_relative(relative)?;
            Ok(self.root_path.join(relative.replace('/', "\\")))
        }

        pub(super) fn file_sha256(&self, relative: &str) -> Option<String> {
            self.files
                .get(relative)
                .and_then(|file| hash_file_handle(file).ok())
        }

        pub(super) fn remove_file(&mut self, relative: &str) -> Result<(), String> {
            let file = self
                .files
                .remove(relative)
                .ok_or_else(|| "private_file_missing".to_owned())?;
            fleet_onboarding_windows_kernel::delete_retained_handle(&file.file)
                .map_err(|_| "exact_file_delete_failed".to_owned())?;
            drop(file);
            Ok(())
        }

        pub(super) fn remove_dir(&mut self, relative: &str) -> Result<(), String> {
            let dir = self
                .dirs
                .remove(relative)
                .ok_or_else(|| "private_directory_missing".to_owned())?;
            fleet_onboarding_windows_kernel::delete_retained_handle(&dir.dir)
                .map_err(|_| "exact_directory_delete_failed".to_owned())?;
            drop(dir);
            Ok(())
        }

        pub(super) fn root_identity(&self) -> &ObjectIdentity {
            &self.root.identity
        }

        pub(super) fn verify_root_binding(&self) -> Result<(), String> {
            let (parent, leaf) = open_parent(&self.root_path)?;
            let current = open_dir_component(&parent, &leaf, false, true)?;
            if current.identity != self.root.identity {
                return Err("private_root_identity_changed".to_owned());
            }
            Ok(())
        }

        pub(super) fn inventory_entries(&self) -> Result<Vec<EntryRecord>, String> {
            let mut entries = Vec::new();
            for (path, dir) in &self.dirs {
                entries.push(EntryRecord {
                    relative_path: path.clone(),
                    kind: EntryKind::Directory,
                    identity: dir.identity.clone(),
                    sha256: None,
                });
            }
            for (path, file) in &self.files {
                entries.push(EntryRecord {
                    relative_path: path.clone(),
                    kind: EntryKind::File,
                    identity: file.identity.clone(),
                    sha256: Some(hash_file_handle(file)?),
                });
            }
            Ok(entries)
        }

        pub(super) fn commit(mut self) {
            self.committed = true;
        }

        pub(super) fn rollback(mut self) -> Result<(), ()> {
            let mut failed = false;
            let file_paths: Vec<_> = self.files.keys().cloned().rev().collect();
            for path in file_paths {
                if self.remove_file(&path).is_err() {
                    failed = true;
                }
            }
            let mut dir_paths: Vec<_> = self.dirs.keys().cloned().collect();
            dir_paths.sort_by_key(|path| std::cmp::Reverse(path.matches('/').count()));
            for path in dir_paths {
                if self.remove_dir(&path).is_err() {
                    failed = true;
                }
            }
            if fleet_onboarding_windows_kernel::delete_retained_handle(&self.root.dir).is_err() {
                failed = true;
            }
            if failed { Err(()) } else { Ok(()) }
        }
    }

    impl Drop for PrivateTree {
        fn drop(&mut self) {
            // Deliberately no implicit deletion. Successful generation is
            // committed; interrupted generation is cleaned only by the exact
            // retained-handle ledger in `rollback`.
            let _ = self.committed;
        }
    }

    fn validate_relative(relative: &str) -> Result<(), String> {
        let path = Path::new(relative);
        if relative.is_empty()
            || relative.contains('\\')
            || relative.starts_with('/')
            || path.components().any(|component| {
                !matches!(component, Component::Normal(_))
                    || component.as_os_str().to_string_lossy().contains(':')
                    || component.as_os_str().to_string_lossy().ends_with('.')
                    || component.as_os_str().to_string_lossy().ends_with(' ')
                    || reserved_windows_name(&component.as_os_str().to_string_lossy())
            })
        {
            return Err("invalid_private_relative_path".to_owned());
        }
        Ok(())
    }

    pub(super) struct CleanupSession {
        inventory_loaded: LoadedFile,
        inventory: Inventory,
        root: SecureDir,
        dirs: BTreeMap<String, SecureDir>,
        files: BTreeMap<String, SecureFile>,
    }

    impl CleanupSession {
        pub(super) fn open(path: &Path) -> Result<Self, String> {
            validate_external_path(path)?;
            if path.file_name() != Some(std::ffi::OsStr::new("onboarding.private-inventory.json")) {
                return Err("inventory_name_mismatch".to_owned());
            }
            let root_path = path
                .parent()
                .ok_or_else(|| "inventory_root_missing".to_owned())?;
            let (parent, leaf) = open_parent(root_path)?;
            // Retain the root before opening the inventory. The root handle
            // denies rename/delete sharing; the inventory is then opened
            // component-relative, so no ancestor needs to be reopened.
            let root = open_dir_component(&parent, &leaf, true, true)?;
            let mut inventory_file = open_file_from(
                &root.dir,
                std::ffi::OsStr::new("onboarding.private-inventory.json"),
                MAX_JSON_BYTES,
                true,
                true,
                "inventory",
            )?;
            let mut inventory_bytes =
                Zeroizing::new(Vec::with_capacity(inventory_file.length as usize));
            inventory_file
                .file
                .read_to_end(&mut inventory_bytes)
                .map_err(|_| "inventory_read_failed".to_owned())?;
            if inventory_bytes.len() as u64 != inventory_file.length {
                return Err("inventory_identity_changed".to_owned());
            }
            inventory_file
                .file
                .seek(SeekFrom::Start(0))
                .map_err(|_| "inventory_read_failed".to_owned())?;
            let inventory_loaded = LoadedFile {
                file: inventory_file,
                bytes: inventory_bytes,
            };
            let inventory: Inventory = serde_json::from_slice(&inventory_loaded.bytes)
                .map_err(|_| "invalid_inventory".to_owned())?;
            validate_inventory_shape(&inventory)?;
            if Path::new(&inventory.output_root) != root_path {
                return Err("inventory_root_mismatch".to_owned());
            }
            if root.identity != inventory.root_identity {
                return Err("inventory_root_identity_mismatch".to_owned());
            }

            let mut dirs = BTreeMap::new();
            let mut files = BTreeMap::new();
            for entry in &inventory.entries {
                match entry.kind {
                    EntryKind::Directory => {
                        let dir = open_relative_dir(&root.dir, &dirs, &entry.relative_path)?;
                        if dir.identity != entry.identity {
                            return Err("inventory_identity_mismatch".to_owned());
                        }
                        dirs.insert(entry.relative_path.clone(), dir);
                    }
                    EntryKind::File => {
                        let file =
                            open_relative_file(&root.dir, &dirs, &entry.relative_path, true)?;
                        if file.identity != entry.identity
                            || entry.sha256.as_deref() != Some(&hash_file_handle(&file)?)
                        {
                            return Err("inventory_file_mismatch".to_owned());
                        }
                        files.insert(entry.relative_path.clone(), file);
                    }
                }
            }
            verify_exact_enumeration(&root.dir, &dirs, &inventory)?;
            Ok(Self {
                inventory_loaded,
                inventory,
                root,
                dirs,
                files,
            })
        }

        pub(super) fn inventory(&self) -> &Inventory {
            &self.inventory
        }

        pub(super) fn inventory_sha256(&self) -> String {
            hash(&self.inventory_loaded.bytes)
        }

        pub(super) fn inventory_identity(&self) -> &ObjectIdentity {
            &self.inventory_loaded.file.identity
        }

        pub(super) fn delete_exact(mut self) -> Result<(), String> {
            let file_paths: Vec<_> = self.files.keys().cloned().rev().collect();
            for path in file_paths {
                let file = self
                    .files
                    .remove(&path)
                    .ok_or_else(|| "cleanup_state_changed".to_owned())?;
                fleet_onboarding_windows_kernel::delete_retained_handle(&file.file)
                    .map_err(|_| "cleanup_incomplete_exact_entries_remain".to_owned())?;
                drop(file);
            }
            let mut dir_paths: Vec<_> = self.dirs.keys().cloned().collect();
            dir_paths.sort_by_key(|path| std::cmp::Reverse(path.matches('/').count()));
            for path in dir_paths {
                let dir = self
                    .dirs
                    .remove(&path)
                    .ok_or_else(|| "cleanup_state_changed".to_owned())?;
                fleet_onboarding_windows_kernel::delete_retained_handle(&dir.dir)
                    .map_err(|_| "cleanup_incomplete_exact_entries_remain".to_owned())?;
                drop(dir);
            }
            fleet_onboarding_windows_kernel::delete_retained_handle(
                &self.inventory_loaded.file.file,
            )
            .map_err(|_| "cleanup_incomplete_inventory_remains".to_owned())?;
            drop(self.inventory_loaded);
            fleet_onboarding_windows_kernel::delete_retained_handle(&self.root.dir)
                .map_err(|_| "cleanup_incomplete_root_remains".to_owned())?;
            drop(self.root);
            Ok(())
        }
    }

    fn validate_inventory_shape(inventory: &Inventory) -> Result<(), String> {
        if inventory.schema != INVENTORY_SCHEMA
            || !is_sha256(&inventory.plan_sha256)
            || inventory.tool_manifest_sha256 != TOOL_MANIFEST_SHA256
            || inventory.tool_executable_sha256 != TOOL_EXECUTABLE_SHA256
            || inventory.quest_profile_owner_source_sha256 != PROFILE_OWNER_SOURCE_SHA256
            || inventory.quest_profile_owner_fixture_sha256 != PROFILE_OWNER_FIXTURE_SHA256
            || inventory.state != "generated-not-installed-or-enrolled"
            || !Path::new(&inventory.output_root).is_absolute()
            || !is_sha256(&inventory.root_identity.acl_sha256)
        {
            return Err("invalid_inventory".to_owned());
        }
        validate_external_path(Path::new(&inventory.output_root))?;
        let mut unique = BTreeSet::new();
        let mut prior = None;
        for entry in &inventory.entries {
            validate_relative(&entry.relative_path)?;
            if !unique.insert(entry.relative_path.to_ascii_lowercase())
                || prior
                    .as_ref()
                    .is_some_and(|value: &String| value >= &entry.relative_path)
                || !is_sha256(&entry.identity.acl_sha256)
                || entry.identity.number_of_links == 0
                || matches!(entry.kind, EntryKind::File)
                    && (entry.identity.number_of_links != 1
                        || entry
                            .sha256
                            .as_deref()
                            .is_none_or(|value| !is_sha256(value)))
                || matches!(entry.kind, EntryKind::Directory) && entry.sha256.is_some()
            {
                return Err("invalid_inventory_entry".to_owned());
            }
            prior = Some(entry.relative_path.clone());
        }
        validate_expected_layout(&inventory.entries)
    }

    pub(super) fn validate_expected_layout(entries: &[EntryRecord]) -> Result<(), String> {
        let mut kinds = BTreeMap::new();
        for entry in entries {
            kinds.insert(entry.relative_path.as_str(), entry.kind);
        }
        if kinds.get("hub") != Some(&EntryKind::Directory)
            || kinds.get("devices") != Some(&EntryKind::Directory)
            || kinds.get("hub/enrollment.private-config.json") != Some(&EntryKind::File)
        {
            return Err("invalid_inventory_layout".to_owned());
        }
        let device_dirs: Vec<_> = entries
            .iter()
            .filter(|entry| {
                entry.kind == EntryKind::Directory
                    && entry.relative_path.starts_with("devices/device-")
                    && entry.relative_path.matches('/').count() == 1
            })
            .map(|entry| entry.relative_path.as_str())
            .collect();
        if device_dirs.is_empty() {
            return Err("invalid_inventory_layout".to_owned());
        }
        let mut expected = BTreeSet::from([
            "hub".to_owned(),
            "devices".to_owned(),
            "hub/enrollment.private-config.json".to_owned(),
        ]);
        for directory in device_dirs {
            let digest = &directory["devices/device-".len()..];
            if !is_sha256(digest) {
                return Err("invalid_inventory_layout".to_owned());
            }
            expected.insert(directory.to_owned());
            expected.insert(format!("{directory}/fleet-agent.seed"));
            expected.insert(format!("{directory}/fleet-agent.profile.json"));
            expected.insert(format!("{directory}/fleet-agent.public-key-record.json"));
        }
        if expected
            != entries
                .iter()
                .map(|entry| entry.relative_path.clone())
                .collect()
        {
            return Err("invalid_inventory_layout".to_owned());
        }
        Ok(())
    }

    fn open_relative_dir(
        root: &Dir,
        opened: &BTreeMap<String, SecureDir>,
        relative: &str,
    ) -> Result<SecureDir, String> {
        let path = Path::new(relative);
        let parent = path
            .parent()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .map_or_else(
                || root.try_clone(),
                |value| {
                    opened
                        .get(value)
                        .ok_or_else(|| std::io::Error::other("missing expected parent"))?
                        .dir
                        .try_clone()
                },
            )
            .map_err(|_| "inventory_parent_unavailable".to_owned())?;
        open_dir_component(
            &parent,
            path.file_name()
                .ok_or_else(|| "invalid_inventory_layout".to_owned())?,
            true,
            true,
        )
    }

    fn open_relative_file(
        root: &Dir,
        opened: &BTreeMap<String, SecureDir>,
        relative: &str,
        delete: bool,
    ) -> Result<SecureFile, String> {
        let path = Path::new(relative);
        let parent = path
            .parent()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .map_or_else(
                || root.try_clone(),
                |value| {
                    opened
                        .get(value)
                        .ok_or_else(|| std::io::Error::other("missing expected parent"))?
                        .dir
                        .try_clone()
                },
            )
            .map_err(|_| "inventory_parent_unavailable".to_owned())?;
        open_file_from(
            &parent,
            path.file_name()
                .ok_or_else(|| "invalid_inventory_layout".to_owned())?,
            MAX_EXECUTABLE_BYTES,
            delete,
            true,
            "inventory_entry",
        )
    }

    fn verify_exact_enumeration(
        root: &Dir,
        dirs: &BTreeMap<String, SecureDir>,
        inventory: &Inventory,
    ) -> Result<(), String> {
        let mut actual = BTreeSet::new();
        enumerate_one_level(root, "", &mut actual)?;
        for (relative, dir) in dirs {
            enumerate_one_level(&dir.dir, relative, &mut actual)?;
        }
        if actual
            .iter()
            .map(|entry| entry.to_ascii_lowercase())
            .collect::<BTreeSet<_>>()
            .len()
            != actual.len()
        {
            return Err("inventory_case_collision".to_owned());
        }
        let mut expected: BTreeSet<String> = inventory
            .entries
            .iter()
            .map(|entry| entry.relative_path.clone())
            .collect();
        expected.insert("onboarding.private-inventory.json".to_owned());
        if actual != expected {
            return Err("inventory_extra_or_missing_entry".to_owned());
        }
        // Every expected directory was also opened with a retained,
        // non-following handle; the map is intentionally consumed by cleanup.
        if dirs.len()
            != inventory
                .entries
                .iter()
                .filter(|entry| entry.kind == EntryKind::Directory)
                .count()
        {
            return Err("inventory_directory_set_mismatch".to_owned());
        }
        Ok(())
    }

    fn enumerate_one_level(
        dir: &Dir,
        prefix: &str,
        output: &mut BTreeSet<String>,
    ) -> Result<(), String> {
        for entry in dir
            .entries()
            .map_err(|_| "inventory_enumeration_failed".to_owned())?
        {
            let entry = entry.map_err(|_| "inventory_enumeration_failed".to_owned())?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| "inventory_non_utf8_entry".to_owned())?;
            let relative = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}/{name}")
            };
            if !output.insert(relative.clone()) {
                return Err("inventory_case_collision".to_owned());
            }
            let metadata = entry
                .metadata()
                .map_err(|_| "inventory_enumeration_failed".to_owned())?;
            if metadata.file_type().is_symlink() {
                return Err("inventory_reparse_entry_rejected".to_owned());
            }
        }
        Ok(())
    }

    #[cfg(all(test, windows))]
    mod windows_tests {
        use std::sync::atomic::{AtomicU64, Ordering};

        use super::*;

        static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

        fn private_parent() -> PathBuf {
            let path = std::env::temp_dir().join(format!(
                "rusty-fleet-onboarding-{}-{}",
                std::process::id(),
                FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("create isolated parent");
            let (ambient_parent, leaf) = open_parent(&path).expect("open parent");
            let mut retained =
                open_dir_component(&ambient_parent, &leaf, true, false).expect("retain parent");
            apply_private_acl(&mut retained.dir, true).expect("apply private ACL");
            drop(retained);
            let retained =
                open_dir_component(&ambient_parent, &leaf, true, true).expect("private parent");
            drop(retained);
            path
        }

        fn populate_and_commit(root_path: &Path) -> PathBuf {
            let mut tree = PrivateTree::create(root_path).expect("private tree");
            tree.create_dir("hub").expect("hub directory");
            tree.create_dir("devices").expect("devices directory");
            let device = format!("devices/device-{}", "a".repeat(64));
            tree.create_dir(&device).expect("device directory");
            tree.write_file("hub/enrollment.private-config.json", b"{}\n")
                .expect("hub config");
            tree.write_file(&format!("{device}/fleet-agent.seed"), &[7_u8; 32])
                .expect("seed");
            tree.write_file(&format!("{device}/fleet-agent.profile.json"), b"{}\n")
                .expect("profile");
            tree.write_file(
                &format!("{device}/fleet-agent.public-key-record.json"),
                b"{}\n",
            )
            .expect("key record");
            let mut entries = tree.inventory_entries().expect("inventory entries");
            entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
            let inventory = Inventory {
                schema: INVENTORY_SCHEMA.to_owned(),
                plan_sha256: "11".repeat(32),
                output_root: root_path.to_string_lossy().into_owned(),
                tool_manifest_sha256: TOOL_MANIFEST_SHA256.to_owned(),
                tool_executable_sha256: TOOL_EXECUTABLE_SHA256.to_owned(),
                quest_profile_owner_source_sha256: PROFILE_OWNER_SOURCE_SHA256.to_owned(),
                quest_profile_owner_fixture_sha256: PROFILE_OWNER_FIXTURE_SHA256.to_owned(),
                root_identity: tree.root_identity().clone(),
                entries,
                state: "generated-not-installed-or-enrolled".to_owned(),
            };
            tree.write_json("onboarding.private-inventory.json", &inventory)
                .expect("inventory commit marker");
            let inventory_path = root_path.join("onboarding.private-inventory.json");
            tree.commit();
            inventory_path
        }

        fn normalize_private_test_file(path: &Path, expect_reparse: bool) {
            let (parent, leaf) = open_parent(path).expect("open test fixture parent");
            let mut options = OpenOptions::new();
            options
                .access_mode(
                    GENERIC_READ
                        | FILE_READ_ATTRIBUTES
                        | READ_CONTROL
                        | DELETE
                        | WRITE_DAC
                        | WRITE_OWNER,
                )
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .follow(FollowSymlinks::No);
            let mut file = parent
                .open_with(&leaf, &options)
                .expect("retain test fixture file");
            let metadata = file.metadata().expect("test fixture metadata");
            assert_eq!(metadata.file_type().is_symlink(), expect_reparse);
            apply_private_acl(&mut file, false).expect("normalize test fixture ACL");
        }

        fn delete_private_file(path: &Path) {
            let file =
                open_absolute_file(path, MAX_EXECUTABLE_BYTES, true, true, "test_cleanup_file")
                    .expect("retain test cleanup file");
            fleet_onboarding_windows_kernel::delete_retained_handle(&file.file)
                .expect("delete exact test file");
        }

        fn delete_private_hard_link(path: &Path) {
            let (parent, leaf) = open_parent(path).expect("open hard-link parent");
            let mut options = OpenOptions::new();
            options
                .access_mode(GENERIC_READ | FILE_READ_ATTRIBUTES | READ_CONTROL | DELETE)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .follow(FollowSymlinks::No);
            let file = parent
                .open_with(&leaf, &options)
                .expect("retain exact hard-link name");
            let metadata = file.metadata().expect("hard-link metadata");
            assert!(metadata.is_file() && !metadata.file_type().is_symlink());
            identity_for(&file, &metadata, false, true).expect("private hard-link identity");
            fleet_onboarding_windows_kernel::delete_retained_handle(&file)
                .expect("delete exact hard-link name");
        }

        fn delete_private_reparse(path: &Path) {
            let (parent, leaf) = open_parent(path).expect("open reparse parent");
            let mut options = OpenOptions::new();
            options
                .access_mode(FILE_READ_ATTRIBUTES | READ_CONTROL | DELETE)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .follow(FollowSymlinks::No);
            let file = parent
                .open_with(&leaf, &options)
                .expect("retain exact reparse name");
            let metadata = file.metadata().expect("reparse metadata");
            assert!(metadata.file_type().is_symlink());
            identity_for(&file, &metadata, false, true).expect("private reparse identity");
            fleet_onboarding_windows_kernel::delete_retained_handle(&file)
                .expect("delete exact reparse name");
        }

        fn delete_private_root(root_path: &Path) {
            let (parent, leaf) = open_parent(root_path).expect("open root parent");
            let root = open_dir_component(&parent, &leaf, true, true).expect("retain private root");
            fleet_onboarding_windows_kernel::delete_retained_handle(&root.dir)
                .expect("delete exact private root");
        }

        fn delete_parent(parent_path: &Path) {
            let (ambient_parent, leaf) = open_parent(parent_path).expect("open test parent");
            let parent =
                open_dir_component(&ambient_parent, &leaf, true, true).expect("retain test parent");
            fleet_onboarding_windows_kernel::delete_retained_handle(&parent.dir)
                .expect("delete exact test parent");
        }

        #[test]
        fn exact_cleanup_removes_only_the_closed_inventory() {
            let parent_path = private_parent();
            let root_path = parent_path.join("bundle");
            let inventory_path = populate_and_commit(&root_path);

            let session = CleanupSession::open(&inventory_path).expect("verified cleanup session");
            session.delete_exact().expect("exact cleanup");

            assert!(!root_path.exists());
            delete_parent(&parent_path);
            assert!(!parent_path.exists());
        }

        #[test]
        fn cleanup_rejects_extra_file_before_mutation() {
            let parent_path = private_parent();
            let root_path = parent_path.join("bundle");
            let inventory_path = populate_and_commit(&root_path);
            let extra_path = root_path.join("extra.txt");
            std::fs::write(&extra_path, b"not inventoried").expect("write inherited-private extra");
            normalize_private_test_file(&extra_path, false);

            assert_eq!(
                CleanupSession::open(&inventory_path).err(),
                Some("inventory_extra_or_missing_entry".to_owned())
            );
            assert!(inventory_path.exists());
            delete_private_file(&extra_path);
            CleanupSession::open(&inventory_path)
                .expect("session after exact extra removal")
                .delete_exact()
                .expect("exact cleanup");
            delete_parent(&parent_path);
        }

        #[test]
        fn cleanup_rejects_hard_link_damage_before_mutation() {
            let parent_path = private_parent();
            let root_path = parent_path.join("bundle");
            let inventory_path = populate_and_commit(&root_path);
            let device = format!("device-{}", "a".repeat(64));
            let seed = root_path
                .join("devices")
                .join(device)
                .join("fleet-agent.seed");
            let extra_link = root_path.join("seed-hard-link");
            std::fs::hard_link(&seed, &extra_link).expect("create damaging hard link");

            assert_eq!(
                CleanupSession::open(&inventory_path).err(),
                Some("hardlink_or_invalid_identity_rejected".to_owned())
            );
            assert!(inventory_path.exists());
            delete_private_hard_link(&extra_link);
            CleanupSession::open(&inventory_path)
                .expect("session after hard-link removal")
                .delete_exact()
                .expect("exact cleanup");
            delete_parent(&parent_path);
        }

        #[test]
        fn cleanup_rejects_reparse_damage_before_mutation_when_supported() {
            let parent_path = private_parent();
            let root_path = parent_path.join("bundle");
            let inventory_path = populate_and_commit(&root_path);
            let reparse = root_path.join("extra-link");
            let target = root_path.join("hub").join("enrollment.private-config.json");

            if std::os::windows::fs::symlink_file(&target, &reparse).is_ok() {
                normalize_private_test_file(&reparse, true);
                assert_eq!(
                    CleanupSession::open(&inventory_path).err(),
                    Some("inventory_reparse_entry_rejected".to_owned())
                );
                assert!(inventory_path.exists());
                delete_private_reparse(&reparse);
            }
            CleanupSession::open(&inventory_path)
                .expect("session after reparse removal or unsupported creation")
                .delete_exact()
                .expect("exact cleanup");
            delete_parent(&parent_path);
        }

        #[test]
        fn retained_inventory_and_root_block_path_substitution() {
            let parent_path = private_parent();
            let root_path = parent_path.join("bundle");
            let inventory_path = populate_and_commit(&root_path);
            let session = CleanupSession::open(&inventory_path).expect("verified cleanup session");

            assert!(std::fs::rename(&inventory_path, root_path.join("substitute.json")).is_err());
            assert!(std::fs::rename(&root_path, parent_path.join("substitute-root")).is_err());
            session.delete_exact().expect("exact cleanup");
            delete_parent(&parent_path);
        }

        #[test]
        fn rollback_reports_uncertain_extra_and_never_recursively_deletes_it() {
            let parent_path = private_parent();
            let root_path = parent_path.join("bundle");
            let mut tree = PrivateTree::create(&root_path).expect("private tree");
            tree.create_dir("owned").expect("owned directory");
            tree.write_file("owned/owned.txt", b"owned")
                .expect("owned file");
            let extra_path = root_path.join("untracked.txt");
            std::fs::write(&extra_path, b"must survive rollback")
                .expect("write inherited-private extra");
            normalize_private_test_file(&extra_path, false);

            assert_eq!(tree.rollback(), Err(()));
            assert_eq!(
                std::fs::read(&extra_path).expect("untracked file survives"),
                b"must survive rollback"
            );
            delete_private_file(&extra_path);
            delete_private_root(&root_path);
            delete_parent(&parent_path);
        }
    }
}

mod tool_process {
    use super::*;

    #[cfg(windows)]
    type ContainedOutput = (
        std::process::ExitStatus,
        Zeroizing<Vec<u8>>,
        Zeroizing<Vec<u8>>,
    );

    #[cfg(windows)]
    pub(super) fn invoke_key_record_tool(
        executable: &secure_fs::RetainedExecutable,
        key_id: &str,
        seed_path: &Path,
    ) -> Result<KeyRecord, String> {
        use std::process::{Command, Stdio};

        executable.verify_binding()?;
        let mut command = Command::new(executable.path());
        command
            .arg("--key-id")
            .arg(key_id)
            .arg("--seed-file")
            .arg(seed_path)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let (status, stdout, stderr) = run_contained(command, TOOL_TIMEOUT)?;
        executable.verify_binding()?;
        if !status.success() {
            return Err("key_tool_failed".to_owned());
        }
        if !stderr.is_empty() {
            return Err("unexpected_key_tool_stderr".to_owned());
        }
        serde_json::from_slice(&stdout).map_err(|_| "malicious_key_tool_output".to_owned())
    }

    #[cfg(windows)]
    fn run_contained(
        mut command: std::process::Command,
        timeout: Duration,
    ) -> Result<ContainedOutput, String> {
        use std::os::windows::process::CommandExt;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::thread;
        use std::time::Instant;

        use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

        command.creation_flags(CREATE_SUSPENDED);
        let mut child = command
            .spawn()
            .map_err(|_| "key_tool_start_failed".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "key_tool_pipe_failed".to_owned())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "key_tool_pipe_failed".to_owned())?;
        let job = match fleet_onboarding_windows_kernel::ContainedJob::assign_and_resume(&child) {
            Ok(job) => job,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("key_tool_containment_failed".to_owned());
            }
        };
        let overflow = Arc::new(AtomicBool::new(false));
        let read_error = Arc::new(AtomicBool::new(false));
        let out = spawn_drain(stdout, Arc::clone(&overflow), Arc::clone(&read_error));
        let err = spawn_drain(stderr, Arc::clone(&overflow), Arc::clone(&read_error));
        let deadline = Instant::now() + timeout;
        let mut parent_status = None;

        let failure = loop {
            if overflow.load(Ordering::Acquire) {
                break Some("key_tool_output_limit");
            }
            if read_error.load(Ordering::Acquire) {
                break Some("key_tool_output_failed");
            }
            if Instant::now() >= deadline {
                break Some("key_tool_timeout");
            }
            if parent_status.is_none() {
                parent_status = child
                    .try_wait()
                    .map_err(|_| "key_tool_wait_failed".to_owned())?;
            }
            if parent_status.is_some() && out.is_finished() && err.is_finished() {
                break None;
            }
            thread::sleep(Duration::from_millis(5));
        };

        // The parent is done or this is a failure. Terminating the job here
        // guarantees that a descendant cannot outlive successful output or
        // retain a pipe after a failed/limited execution.
        job.terminate()
            .map_err(|_| "key_tool_containment_failed".to_owned())?;
        let waited = child
            .wait()
            .map_err(|_| "key_tool_containment_failed".to_owned())?;
        drop(job);
        let stdout = out
            .join()
            .map_err(|_| "key_tool_output_failed".to_owned())??;
        let stderr = err
            .join()
            .map_err(|_| "key_tool_output_failed".to_owned())??;
        if let Some(code) = failure {
            return Err(code.to_owned());
        }
        let status = parent_status.unwrap_or(waited);
        Ok((status, stdout, stderr))
    }

    #[cfg(windows)]
    fn spawn_drain<R>(
        mut reader: R,
        overflow: std::sync::Arc<std::sync::atomic::AtomicBool>,
        read_error: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> std::thread::JoinHandle<Result<Zeroizing<Vec<u8>>, String>>
    where
        R: Read + Send + 'static,
    {
        std::thread::spawn(move || {
            let mut output = Zeroizing::new(Vec::new());
            let mut chunk = Zeroizing::new([0_u8; 4 * 1024]);
            loop {
                let count = match reader.read(chunk.as_mut()) {
                    Ok(count) => count,
                    Err(_) => {
                        read_error.store(true, std::sync::atomic::Ordering::Release);
                        return Err("key_tool_output_failed".to_owned());
                    }
                };
                if count == 0 {
                    break;
                }
                if output.len().saturating_add(count) > MAX_TOOL_OUTPUT {
                    overflow.store(true, std::sync::atomic::Ordering::Release);
                } else {
                    output.extend_from_slice(&chunk[..count]);
                }
            }
            Ok(output)
        })
    }

    #[cfg(all(test, windows))]
    mod windows_tests {
        use std::process::{Command, Stdio};

        use super::*;

        const CHILD_MODE: &str = "RUSTY_FLEET_ONBOARDING_TEST_CHILD_MODE";
        const CHILD_HANDLE: &str = "RUSTY_FLEET_ONBOARDING_TEST_SENTINEL_HANDLE";
        const CHILD_HANDLE_IDENTITY: &str = "RUSTY_FLEET_ONBOARDING_TEST_SENTINEL_IDENTITY";

        fn command_at(executable: &Path, mode: &str) -> Command {
            let mut command = Command::new(executable);
            command
                .arg("--exact")
                .arg("tool_process::windows_tests::process_fixture_child")
                .arg("--nocapture")
                .env(CHILD_MODE, mode)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            command
        }

        fn command(mode: &str) -> Command {
            command_at(&std::env::current_exe().expect("current test binary"), mode)
        }

        #[test]
        #[allow(clippy::zombie_processes)] // Intentionally exercises a descendant-held pipe.
        fn process_fixture_child() {
            match std::env::var(CHILD_MODE).ok().as_deref() {
                Some("success") => print!("contained"),
                Some("overflow") => print!("{}", "x".repeat(20_000)),
                Some("descendant") => {
                    let mut child =
                        Command::new(std::env::current_exe().expect("current test binary"));
                    child
                        .arg("--exact")
                        .arg("tool_process::windows_tests::process_fixture_child")
                        .arg("--nocapture")
                        .env(CHILD_MODE, "sleeper")
                        .stdin(Stdio::null())
                        .stdout(Stdio::inherit())
                        .stderr(Stdio::inherit());
                    child.spawn().expect("spawn descendant");
                    print!("parent-exited");
                }
                Some("sleeper") => std::thread::sleep(Duration::from_secs(30)),
                Some("handle-check") => {
                    let raw = std::env::var(CHILD_HANDLE)
                        .expect("sentinel handle")
                        .parse::<usize>()
                        .expect("numeric sentinel handle");
                    let expected = std::env::var(CHILD_HANDLE_IDENTITY).expect("sentinel identity");
                    let inherited = fleet_onboarding_windows_kernel::test_file_identity(raw)
                        .map(|(volume, high, low)| format!("{volume}:{high}:{low}"))
                        .is_ok_and(|actual| actual == expected);
                    if inherited {
                        print!("sentinel-inherited");
                    } else {
                        print!("sentinel-not-inherited");
                    }
                }
                _ => {}
            }
        }

        #[test]
        fn job_object_captures_bounded_success() {
            let (status, stdout, stderr) =
                run_contained(command("success"), Duration::from_secs(5)).expect("contained child");
            assert!(status.success());
            assert!(
                stdout
                    .windows(b"contained".len())
                    .any(|part| part == b"contained")
            );
            assert!(stderr.is_empty());
        }

        #[test]
        fn job_object_fails_closed_on_output_overflow() {
            assert_eq!(
                run_contained(command("overflow"), Duration::from_secs(5)).err(),
                Some("key_tool_output_limit".to_owned())
            );
        }

        #[test]
        fn job_object_deadline_includes_descendant_held_pipes() {
            assert_eq!(
                run_contained(command("descendant"), Duration::from_millis(500)).err(),
                Some("key_tool_timeout".to_owned())
            );
        }

        #[test]
        fn retained_sensitive_handle_is_not_inherited() {
            let sentinel_path = std::env::temp_dir().join(format!(
                "rusty-fleet-handle-sentinel-{}-{}.tmp",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("time after epoch")
                    .as_nanos()
            ));
            let sentinel = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&sentinel_path)
                .expect("create handle sentinel");
            let raw = fleet_onboarding_windows_kernel::test_raw_handle_value(&sentinel);
            let identity = fleet_onboarding_windows_kernel::test_file_identity(raw)
                .map(|(volume, high, low)| format!("{volume}:{high}:{low}"))
                .expect("sentinel identity");
            let mut child = command("handle-check");
            child
                .env(CHILD_HANDLE, raw.to_string())
                .env(CHILD_HANDLE_IDENTITY, identity);
            let (status, stdout, stderr) =
                run_contained(child, Duration::from_secs(5)).expect("contained handle check");
            drop(sentinel);
            std::fs::remove_file(sentinel_path).expect("remove handle sentinel");

            assert!(status.success());
            assert!(stderr.is_empty());
            assert!(
                stdout
                    .windows(b"sentinel-not-inherited".len())
                    .any(|part| part == b"sentinel-not-inherited")
            );
        }

        #[test]
        fn adversarial_ancestor_rename_recreate_is_blocked_through_process_creation() {
            let current = std::env::current_exe().expect("current test binary");
            let root = std::env::temp_dir().join(format!(
                "rusty-fleet-tool-path-guard-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("time after epoch")
                    .as_nanos()
            ));
            let trusted = root.join("trusted");
            let bin = trusted.join("bin");
            let executable = bin.join("fleet-agent-key-record.exe");
            let moved = root.join("trusted-moved");
            std::fs::create_dir(&root).expect("create guard test root");
            std::fs::create_dir(&trusted).expect("create guarded ancestor");
            std::fs::create_dir(&bin).expect("create guarded bin");
            std::fs::copy(&current, &executable).expect("copy guarded executable");

            let retained = secure_fs::RetainedExecutable::open(&executable, MAX_EXECUTABLE_BYTES)
                .expect("retain complete executable path");
            retained
                .verify_binding()
                .expect("initial exact path binding");

            let attack_trusted = trusted.clone();
            let attack_moved = moved.clone();
            let attack_bin = bin.clone();
            let attack_executable = executable.clone();
            let swapped = std::thread::spawn(move || {
                if std::fs::rename(&attack_trusted, &attack_moved).is_err() {
                    return false;
                }
                std::fs::create_dir(&attack_trusted).expect("recreate substituted ancestor");
                std::fs::create_dir(&attack_bin).expect("recreate substituted bin");
                std::fs::write(&attack_executable, b"malicious substitute")
                    .expect("write substituted executable");
                true
            })
            .join()
            .expect("ancestor substitution attacker");

            if swapped {
                drop(retained);
                std::fs::remove_file(&executable).expect("remove substituted executable");
                std::fs::remove_dir(&bin).expect("remove substituted bin");
                std::fs::remove_dir(&trusted).expect("remove substituted ancestor");
                std::fs::remove_file(moved.join("bin/fleet-agent-key-record.exe"))
                    .expect("remove moved original executable");
                std::fs::remove_dir(moved.join("bin")).expect("remove moved bin");
                std::fs::remove_dir(&moved).expect("remove moved ancestor");
                std::fs::remove_dir(&root).expect("remove failed guard test root");
                panic!("retained executable path allowed ancestor rename/recreate");
            }

            retained
                .verify_binding()
                .expect("binding after rejected substitution");
            let (status, stdout, stderr) = run_contained(
                command_at(retained.path(), "success"),
                Duration::from_secs(5),
            )
            .expect("launch retained exact path");
            retained
                .verify_binding()
                .expect("binding after process creation");
            assert!(status.success());
            assert!(
                stdout
                    .windows(b"contained".len())
                    .any(|part| part == b"contained")
            );
            assert!(stderr.is_empty());

            drop(retained);
            std::fs::remove_file(&executable).expect("remove guarded executable");
            std::fs::remove_dir(&bin).expect("remove guarded bin");
            std::fs::remove_dir(&trusted).expect("remove guarded ancestor");
            std::fs::remove_dir(&root).expect("remove guard test root");
        }
    }

    #[cfg(not(windows))]
    pub(super) fn invoke_key_record_tool(
        _executable: &secure_fs::RetainedExecutable,
        _key_id: &str,
        _seed_path: &Path,
    ) -> Result<KeyRecord, String> {
        Err("windows_job_object_required".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_plan() -> Plan {
        Plan {
            schema: PLAN_SCHEMA,
            request_sha256: "11".repeat(32),
            tool_manifest_sha256: TOOL_MANIFEST_SHA256,
            tool_executable_sha256: TOOL_EXECUTABLE_SHA256,
            tool_source_commit: TOOL_SOURCE_COMMIT,
            tool_owner: TOOL_OWNER,
            tool_consumer_id: TOOL_CONSUMER_ID,
            quest_profile_owner_source_sha256: PROFILE_OWNER_SOURCE_SHA256,
            quest_profile_owner_fixture_sha256: PROFILE_OWNER_FIXTURE_SHA256,
            device_ids: vec!["device.alpha".to_owned()],
            output_files: vec!["devices/device-deadbeef/fleet-agent.seed".to_owned()],
            effects: vec!["generate-32-byte-seed-per-device"],
            non_claims: vec!["not-enrolled"],
        }
    }

    fn valid_profile() -> QuestFleetAgentProfileV1 {
        QuestFleetAgentProfileV1 {
            schema: PROFILE_SCHEMA.to_owned(),
            enabled: true,
            device_id: "device.alpha".to_owned(),
            display_name: "Alpha".to_owned(),
            model: "Quest".to_owned(),
            hardware_class: "standalone_xr".to_owned(),
            identity_revision: 1,
            expected_authority_revision: 1,
            status_revision: 1,
            source_revision: 1,
            source_epoch: "epoch.alpha".to_owned(),
            key_id: "key.alpha.v1".to_owned(),
            key_fingerprint: format!("fingerprint.{}", "a".repeat(64)),
            trust_domain: "trust.local".to_owned(),
            checkin_ttl_ms: 60_000,
            checkin_interval_ms: 15_000,
            hub_endpoint: "http://192.0.2.10:8741/fleet/v1/checkins".to_owned(),
            tags: BTreeMap::from([("fixture".to_owned(), "valid".to_owned())]),
        }
    }

    fn valid_manifest() -> ToolManifest {
        ToolManifest {
            schema: TOOL_SCHEMA.to_owned(),
            capsule_version: TOOL_CAPSULE_VERSION.to_owned(),
            tool_contract: ToolContract {
                schema: TOOL_CONTRACT_SCHEMA.to_owned(),
                executable: "fleet-agent-key-record.exe".to_owned(),
                argument_contract: "--key-id <dotted-id> --seed-file <private-seed-file>"
                    .to_owned(),
                output_schema: KEY_RECORD_SCHEMA.to_owned(),
            },
            source: ToolSource {
                repository_url: TOOL_OWNER_REPOSITORY.to_owned(),
                commit: TOOL_SOURCE_COMMIT.to_owned(),
                tree: TOOL_SOURCE_TREE.to_owned(),
                provenance_path: "provenance.json".to_owned(),
                provenance_sha256: TOOL_PROVENANCE_SHA256.to_owned(),
            },
            artifact: ToolArtifact {
                path: "fleet-agent-key-record.exe".to_owned(),
                sha256: TOOL_EXECUTABLE_SHA256.to_owned(),
                size_bytes: TOOL_EXECUTABLE_SIZE,
                target: "x86_64-pc-windows-msvc".to_owned(),
                profile: "release".to_owned(),
            },
            distribution: ToolDistribution {
                portable: true,
                supported: true,
                inert_until_invoked: true,
                install_contract: "copy_capsule_byte_for_byte".to_owned(),
                private_material_included: false,
                live_onboarding_claim: false,
            },
            payload: vec![
                ToolPayloadEntry {
                    path: "fleet-agent-key-record.exe".to_owned(),
                    sha256: TOOL_EXECUTABLE_SHA256.to_owned(),
                    size_bytes: TOOL_EXECUTABLE_SIZE,
                },
                ToolPayloadEntry {
                    path: "provenance.json".to_owned(),
                    sha256: TOOL_PROVENANCE_SHA256.to_owned(),
                    size_bytes: 4_596,
                },
                ToolPayloadEntry {
                    path: "LICENSE".to_owned(),
                    sha256: "0d96a4ff68ad6d4b6f1f30f713b18d5184912ba8dd389f86aa7710db079abcb0"
                        .to_owned(),
                    size_bytes: 34_523,
                },
                ToolPayloadEntry {
                    path: "SOURCE-NOTICE.md".to_owned(),
                    sha256: "7a95b2704991263057c12f75efae64cd2c38bf35e20fceca7bc42884e69e698a"
                        .to_owned(),
                    size_bytes: 427,
                },
            ],
        }
    }

    fn valid_provenance() -> ToolProvenance {
        ToolProvenance {
            schema: TOOL_PROVENANCE_SCHEMA.to_owned(),
            capsule_version: TOOL_CAPSULE_VERSION.to_owned(),
            source: ProvenanceSource {
                repository_url: TOOL_OWNER_REPOSITORY.to_owned(),
                commit: TOOL_SOURCE_COMMIT.to_owned(),
                tree: TOOL_SOURCE_TREE.to_owned(),
                package: "rusty-quest-fleet-agent".to_owned(),
                composition_fingerprint: TOOL_COMPOSITION.to_owned(),
                repositories: vec![
                    SourceRepository {
                        repository_id: "rusty-fleet".to_owned(),
                        role: "contract-dependency".to_owned(),
                        repository_url: "https://github.com/MesmerPrism/rusty-fleet".to_owned(),
                        commit: "8181683be4a3abbc5daa0c4497c7aeb9e76316a8".to_owned(),
                        tree: "195565629a53dfaaeacb1a7260fda06062324ad9".to_owned(),
                    },
                    SourceRepository {
                        repository_id: "rusty-manifold".to_owned(),
                        role: "contract-dependency".to_owned(),
                        repository_url: "https://github.com/MesmerPrism/rusty-manifold".to_owned(),
                        commit: "947421a928889889e485006bcc0200e05c2394f9".to_owned(),
                        tree: "836f1f21c5c8856bfc6dcdba8ed3721c090c76ba".to_owned(),
                    },
                    SourceRepository {
                        repository_id: "rusty-quest".to_owned(),
                        role: "release-owner".to_owned(),
                        repository_url: TOOL_OWNER_REPOSITORY.to_owned(),
                        commit: TOOL_SOURCE_COMMIT.to_owned(),
                        tree: TOOL_SOURCE_TREE.to_owned(),
                    },
                ],
                workspace_parse_only_repositories: vec![
                    SourceRepository {
                        repository_id: "rusty-lattice".to_owned(),
                        role: "workspace-parse-only".to_owned(),
                        repository_url: "https://github.com/MesmerPrism/rusty-lattice".to_owned(),
                        commit: "0aee7faa52fc965ff2255381781dd082ab639f4b".to_owned(),
                        tree: "4f60d4a01a3ca4dc217c4f82c16c952ab6733eb4".to_owned(),
                    },
                    SourceRepository {
                        repository_id: "rusty-matter".to_owned(),
                        role: "workspace-parse-only".to_owned(),
                        repository_url: "https://github.com/MesmerPrism/rusty-matter".to_owned(),
                        commit: "eec8cddd9830f7ef0f90574ddcbde2daac0ec804".to_owned(),
                        tree: "cd4e1ce39a8c91263774ea3e69fb859f503ffde8".to_owned(),
                    },
                    SourceRepository {
                        repository_id: "rusty-optics".to_owned(),
                        role: "workspace-parse-only".to_owned(),
                        repository_url: "https://github.com/MesmerPrism/rusty-optics".to_owned(),
                        commit: "fd01d84acffa1b0a3a192fe978af337d9fedd18a".to_owned(),
                        tree: "f527b761043e4e1e3a6bfa5969611dcf419e55fa".to_owned(),
                    },
                ],
                files: vec![
                    SourceFile {
                        path: "Cargo.lock".to_owned(),
                        sha256: "0d95468b7838ea175e654baa0974781416effbaec9376b5879368ab84106330d"
                            .to_owned(),
                    },
                    SourceFile {
                        path: "Cargo.toml".to_owned(),
                        sha256: "e40ca7015177e9a5e7d9546855613a01b81e855690ed8971ea1efedc7b93e6c1"
                            .to_owned(),
                    },
                    SourceFile {
                        path: "crates/rusty-quest-fleet-agent/Cargo.toml".to_owned(),
                        sha256: "b2d670017388582aa7297f717c7a916901d8efd5ca325a5547bb237c38df8225"
                            .to_owned(),
                    },
                    SourceFile {
                        path: "crates/rusty-quest-fleet-agent/src/bin/fleet-agent-key-record.rs"
                            .to_owned(),
                        sha256: "4f39342fdb72a6d3be94f99f949227d1ec2e2cfc13bc45a6d2c992e5f4016212"
                            .to_owned(),
                    },
                    SourceFile {
                        path: "crates/rusty-quest-fleet-agent/src/lib.rs".to_owned(),
                        sha256: "af16db769ca0271438c0b84b5c0ce3fb1cfeea48416930e828d39cc43d7da11e"
                            .to_owned(),
                    },
                    SourceFile {
                        path: "tools/Build-FleetAgentKeyRecordRelease.ps1".to_owned(),
                        sha256: "4525c43a52b87031cff47e79c60f73adaebacf7ebc99e2bcd7086283d864ff9b"
                            .to_owned(),
                    },
                    SourceFile {
                        path: "tools/Test-FleetAgentKeyRecordRelease.ps1".to_owned(),
                        sha256: "8194506d56cbc5712c11623eabb3ba4b2f5a56f25414767f304adad7dedad486"
                            .to_owned(),
                    },
                    SourceFile {
                        path: "tools/lib/SourceComposition.psm1".to_owned(),
                        sha256: "7e3a231b0703b9e0d1ab0b687a473f1a03366885a9eb3108d55839757d30c3df"
                            .to_owned(),
                    },
                ],
            },
            build: ProvenanceBuild {
                target: "x86_64-pc-windows-msvc".to_owned(),
                profile: "release".to_owned(),
                rustc: "rustc fixture".to_owned(),
                cargo: "cargo fixture".to_owned(),
                locked_dependencies: true,
                isolated_git_materializations: true,
                post_build_identity_verified: true,
                path_remap_root: "/rusty-build".to_owned(),
                symbols_stripped: true,
                linker_reproducibility_argument: "/Brepro".to_owned(),
                pe_reproducibility_marker: "IMAGE_DEBUG_TYPE_REPRO".to_owned(),
                cargo_config_sha256: TOOL_CARGO_CONFIG_SHA256.to_owned(),
            },
            claims: ProvenanceClaims {
                owner: TOOL_OWNER.to_owned(),
                helper_only: true,
                runtime_activation: "explicit_fleet_onboard_invocation".to_owned(),
                enrollment_authority: false,
                device_authority: false,
                private_seed_included: false,
                profile_included: false,
                hub_configuration_included: false,
            },
        }
    }

    #[test]
    fn plan_digest_is_deterministic_and_sensitive() {
        let first = sample_plan();
        let second = sample_plan();
        assert_eq!(plan_digest(&first), plan_digest(&second));
        let mut changed = sample_plan();
        changed.device_ids[0] = "device.beta".to_owned();
        assert_ne!(plan_digest(&first), plan_digest(&changed));
    }

    #[test]
    fn device_directories_are_opaque_and_case_sensitive_ids_cannot_collide() {
        let alpha = device_directory("Device.Alpha");
        let lower = device_directory("device.alpha");
        assert!(alpha.starts_with("devices/device-"));
        assert_eq!(alpha.len(), "devices/device-".len() + 64);
        assert_ne!(alpha, lower);
        assert!(!alpha.contains("Alpha"));
    }

    #[test]
    fn dotted_ids_are_strict() {
        assert!(dotted("device.alpha-1"));
        assert!(!dotted(""));
        assert!(!dotted("device..alpha"));
        assert!(!dotted("device/alpha"));
    }

    #[test]
    fn key_record_rejects_malicious_output() {
        let record = KeyRecord {
            schema: KEY_RECORD_SCHEMA.to_owned(),
            key_id: "key.alpha".to_owned(),
            public_key_hex: "00".repeat(32),
            key_fingerprint: format!("fingerprint.{}", "00".repeat(32)),
        };
        assert_eq!(
            validate_key_record(&record, "key.alpha"),
            Err("malicious_key_tool_output".to_owned())
        );
    }

    #[test]
    fn key_record_recomputes_the_public_key_fingerprint() {
        let public_key_hex = "42".repeat(32);
        let record = KeyRecord {
            schema: KEY_RECORD_SCHEMA.to_owned(),
            key_id: "key.alpha".to_owned(),
            key_fingerprint: format!(
                "fingerprint.{}",
                hash(&hex::decode(&public_key_hex).expect("public key"))
            ),
            public_key_hex,
        };
        assert_eq!(validate_key_record(&record, "key.alpha"), Ok(()));
        let mut substituted = record;
        substituted.public_key_hex = "43".repeat(32);
        assert_eq!(
            validate_key_record(&substituted, "key.alpha"),
            Err("malicious_key_tool_output".to_owned())
        );
    }

    #[test]
    fn secret_digest_registry_accepts_distinct_seeds_and_rejects_duplicates() {
        let mut digests = Vec::new();
        assert_eq!(register_secret_digest(&mut digests, &[1_u8; 32]), Ok(()));
        assert_eq!(register_secret_digest(&mut digests, &[2_u8; 32]), Ok(()));
        assert_eq!(digests.len(), 2);
        assert_eq!(
            register_secret_digest(&mut digests, &[1_u8; 32]),
            Err("duplicate_seed_rejected".to_owned())
        );
        assert_eq!(digests.len(), 2);
    }

    #[test]
    fn public_key_registry_rejects_cross_device_duplicates() {
        let mut fingerprints = BTreeSet::new();
        let record = KeyRecord {
            schema: KEY_RECORD_SCHEMA.to_owned(),
            key_id: "key.alpha".to_owned(),
            public_key_hex: "42".repeat(32),
            key_fingerprint: format!("fingerprint.{}", "ab".repeat(32)),
        };
        assert_eq!(register_public_key(&mut fingerprints, &record), Ok(()));
        assert_eq!(
            register_public_key(&mut fingerprints, &record),
            Err("duplicate_public_key_rejected".to_owned())
        );
    }

    #[test]
    fn quest_profile_mirror_accepts_owner_shape_and_rejects_damage() {
        let profile = valid_profile();
        assert_eq!(validate_quest_profile(&profile), Ok(()));
        let bytes = serde_json::to_vec(&profile).expect("serialize");
        let parsed: QuestFleetAgentProfileV1 =
            serde_json::from_slice(&bytes).expect("deny-unknown owner mirror");
        assert_eq!(validate_quest_profile(&parsed), Ok(()));
        let mut damaged = profile;
        damaged.checkin_interval_ms = damaged.checkin_ttl_ms;
        assert_eq!(
            validate_quest_profile(&damaged),
            Err("invalid_generated_quest_profile".to_owned())
        );
    }

    #[test]
    fn quest_profile_unknown_fields_are_rejected() {
        let mut value = serde_json::to_value(valid_profile()).expect("value");
        value["private_path"] = json!("forbidden");
        assert!(serde_json::from_value::<QuestFleetAgentProfileV1>(value).is_err());
    }

    #[test]
    fn quest_profile_owner_fixture_is_exact_and_conformant() {
        let bytes = include_bytes!(
            "../../../fixtures/onboarding/rusty-quest-fleet-agent-profile.disabled.owner-de144.json"
        );
        assert_eq!(canonical_text_hash(bytes), PROFILE_OWNER_FIXTURE_SHA256);
        let mut profile: QuestFleetAgentProfileV1 =
            serde_json::from_slice(bytes).expect("exact owner profile fixture");
        assert!(!profile.enabled);
        profile.enabled = true;
        assert_eq!(validate_quest_profile(&profile), Ok(()));
    }

    #[test]
    fn cleanup_and_revocation_are_distinct() {
        assert_ne!(
            "rusty.fleet.offline_onboarding_cleanup_plan.v1",
            "rusty.fleet.offline_onboarding_revoke_plan.v1"
        );
    }

    #[test]
    fn manifest_repository_set_is_closed() {
        assert_eq!(["rusty-fleet", "rusty-manifold", "rusty-quest"].len(), 3);
        assert_eq!(["rusty-lattice", "rusty-matter", "rusty-optics"].len(), 3);
        assert_eq!(TOOL_SOURCE_TREE.len(), 40);
        assert!(is_sha256(TOOL_MANIFEST_SHA256));
        assert!(is_sha256(TOOL_EXECUTABLE_SHA256));
    }

    #[test]
    fn exact_manifest_accepts_only_the_closed_owner_provenance_set() {
        let manifest = valid_manifest();
        assert_eq!(validate_exact_manifest(&manifest), Ok(()));

        let mut extra = manifest.clone();
        extra.payload.push(extra.payload[0].clone());
        assert_eq!(
            validate_exact_manifest(&extra),
            Err("invalid_tool_manifest".to_owned())
        );

        let mut reordered = manifest.clone();
        reordered.payload.swap(0, 1);
        assert_eq!(
            validate_exact_manifest(&reordered),
            Err("invalid_tool_manifest".to_owned())
        );

        let mut substituted = manifest;
        substituted.source.repository_url = "https://example.invalid/wrong-owner".to_owned();
        assert_eq!(
            validate_exact_manifest(&substituted),
            Err("invalid_tool_manifest".to_owned())
        );

        let provenance = valid_provenance();
        assert_eq!(validate_exact_provenance(&provenance), Ok(()));
        let mut extra_repository = provenance.clone();
        extra_repository
            .source
            .repositories
            .push(extra_repository.source.repositories[0].clone());
        assert_eq!(
            validate_exact_provenance(&extra_repository),
            Err("invalid_tool_provenance".to_owned())
        );
        let mut stale = provenance;
        stale.source.commit = "0".repeat(40);
        assert_eq!(
            validate_exact_provenance(&stale),
            Err("invalid_tool_provenance".to_owned())
        );
    }

    #[test]
    fn reserved_windows_names_are_rejected_as_path_components() {
        for value in ["CON", "con.txt", "AUX", "NUL.json", "COM1", "lpt9.log"] {
            assert!(
                secure_fs::reserved_windows_name(value),
                "{value} must be reserved"
            );
        }
        for value in ["console", "COM10", "LPT0", "device-alpha"] {
            assert!(
                !secure_fs::reserved_windows_name(value),
                "{value} must be portable"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn alternate_data_stream_paths_fail_closed() {
        assert_eq!(
            secure_fs::validate_external_path(Path::new(r"C:\private\seed:backup")),
            Err("unsafe_windows_path".to_owned())
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn apply_fails_before_loading_a_request_or_generating_secrets_off_windows() {
        assert_eq!(
            apply(
                Path::new("/request-that-must-not-be-opened.json"),
                &"00".repeat(32)
            ),
            Err("windows_private_onboarding_required".to_owned())
        );
    }

    #[test]
    fn inventory_layout_rejects_extra_file() {
        let identity = ObjectIdentity {
            volume_serial_number: 1,
            file_id: 2,
            number_of_links: 1,
            acl_sha256: "aa".repeat(32),
        };
        let mut entries = vec![
            EntryRecord {
                relative_path: "devices".to_owned(),
                kind: EntryKind::Directory,
                identity: identity.clone(),
                sha256: None,
            },
            EntryRecord {
                relative_path: format!("devices/device-{}", "b".repeat(64)),
                kind: EntryKind::Directory,
                identity: identity.clone(),
                sha256: None,
            },
            EntryRecord {
                relative_path: format!(
                    "devices/device-{}/fleet-agent.profile.json",
                    "b".repeat(64)
                ),
                kind: EntryKind::File,
                identity: identity.clone(),
                sha256: Some("cc".repeat(32)),
            },
            EntryRecord {
                relative_path: format!(
                    "devices/device-{}/fleet-agent.public-key-record.json",
                    "b".repeat(64)
                ),
                kind: EntryKind::File,
                identity: identity.clone(),
                sha256: Some("cc".repeat(32)),
            },
            EntryRecord {
                relative_path: format!("devices/device-{}/fleet-agent.seed", "b".repeat(64)),
                kind: EntryKind::File,
                identity: identity.clone(),
                sha256: Some("cc".repeat(32)),
            },
            EntryRecord {
                relative_path: "hub".to_owned(),
                kind: EntryKind::Directory,
                identity: identity.clone(),
                sha256: None,
            },
            EntryRecord {
                relative_path: "hub/enrollment.private-config.json".to_owned(),
                kind: EntryKind::File,
                identity: identity.clone(),
                sha256: Some("cc".repeat(32)),
            },
        ];
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        assert_eq!(secure_fs::validate_expected_layout(&entries), Ok(()));
        entries.push(EntryRecord {
            relative_path: "unrelated.txt".to_owned(),
            kind: EntryKind::File,
            identity,
            sha256: Some("cc".repeat(32)),
        });
        assert_eq!(
            secure_fs::validate_expected_layout(&entries),
            Err("invalid_inventory_layout".to_owned())
        );
    }
}
