// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Optional File Manager-owned Kiosk v2 catalog subprocess.
//!
//! Fleet supplies only a non-secret, exact catalog expectation. File Manager
//! owns profile resolution, endpoints, pairing material, session crypto,
//! transport, decryption, and Kiosk verification. The subprocess returns the
//! deliberately exportable verified exchange; Fleet rechecks it through the
//! existing production adapter.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fleet_kiosk_adapter::{
    KioskEncryptedOperatorV2Provider, KioskV2CatalogExpectation, MAX_KIOSK_V2_OWNER_CATALOG_BYTES,
    MAX_KIOSK_V2_OWNER_GRANT_RECEIPT_BYTES, MAX_KIOSK_V2_REQUEST_ENVELOPE_BYTES,
    MAX_KIOSK_V2_RESPONSE_ENVELOPE_BYTES, VerifiedKioskV2CatalogExchange,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const FILE_MANAGER_PROVIDER_REQUEST_SCHEMA: &str =
    "questionable.file_manager.fleet_kiosk_v2_catalog_request.v1";
pub const FILE_MANAGER_PROVIDER_RESPONSE_SCHEMA: &str =
    "questionable.file_manager.fleet_kiosk_v2_catalog_response.v1";

const MAX_PROVIDER_PROFILES: usize = 100;
const MAX_PROVIDER_REQUEST_BYTES: usize = 64 * 1024;
const MAX_PROVIDER_STDOUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PROVIDER_STDERR_BYTES: usize = 16 * 1024;
const MAX_PROVIDER_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(10);
const PROVIDER_ARGS: [&str; 3] = ["integration", "kiosk-v2-catalog", "--json"];
const PROVIDER_EXECUTABLE_NAME: &str = "questionable-file-manager-kiosk-v2-provider.exe";
const PROVIDER_STAGE_ROOT: &str = "kiosk-v2-provider-stage";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileManagerKioskV2ProviderConfig {
    pub executable: PathBuf,
    pub expected_executable_sha256: String,
    pub profiles: Vec<FileManagerKioskV2ProfileMapping>,
}

impl std::fmt::Debug for FileManagerKioskV2ProviderConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileManagerKioskV2ProviderConfig")
            .field("executable", &"[private absolute executable]")
            .field("profiles", &self.profiles)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileManagerKioskV2ProfileMapping {
    pub device_id: String,
    pub identity_revision: u64,
    pub profile_id: String,
}

impl FileManagerKioskV2ProviderConfig {
    pub fn validate_shape(&self) -> Result<(), &'static str> {
        if !self.executable.is_absolute() {
            return Err("provider_executable_not_absolute");
        }
        require_exe_path(&self.executable)?;
        if !valid_sha256(&self.expected_executable_sha256) {
            return Err("provider_executable_digest_invalid");
        }
        if self.profiles.is_empty() || self.profiles.len() > MAX_PROVIDER_PROFILES {
            return Err("provider_profile_count_invalid");
        }
        let mut devices = BTreeSet::new();
        let mut profiles = BTreeSet::new();
        for mapping in &self.profiles {
            if !valid_token(&mapping.device_id, 1, 256)
                || mapping.identity_revision == 0
                || !valid_token(&mapping.profile_id, 8, 128)
                || !devices.insert(mapping.device_id.clone())
                || !profiles.insert(mapping.profile_id.clone())
            {
                return Err("provider_profile_mapping_invalid");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExecutableIdentity {
    configured_path: PathBuf,
    canonical_path: PathBuf,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProcessOutput {
    exit_code: i32,
    stdout: Vec<u8>,
}

trait ProviderProcessRunner: Send {
    fn run(
        &mut self,
        executable: &Path,
        _profile_id: &str,
        stdin: &[u8],
    ) -> Result<ProcessOutput, &'static str>;
}

struct BoundedProviderProcessRunner {
    timeout: Duration,
}
impl Default for BoundedProviderProcessRunner {
    fn default() -> Self {
        Self {
            timeout: PROVIDER_TIMEOUT,
        }
    }
}

impl ProviderProcessRunner for BoundedProviderProcessRunner {
    fn run(
        &mut self,
        executable: &Path,
        _profile_id: &str,
        stdin: &[u8],
    ) -> Result<ProcessOutput, &'static str> {
        if stdin.len() > MAX_PROVIDER_REQUEST_BYTES {
            return Err("provider_request_too_large");
        }
        let mut command = Command::new(executable);
        command
            .arg(PROVIDER_ARGS[0])
            .arg(PROVIDER_ARGS[1])
            .arg(PROVIDER_ARGS[2])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(parent) = executable.parent() {
            command.current_dir(parent);
            command.env(
                "DOTNET_BUNDLE_EXTRACT_BASE_DIR",
                parent.join("bundle-extract"),
            );
        }
        for name in [
            "SystemRoot",
            "WINDIR",
            "TEMP",
            "TMP",
            "USERPROFILE",
            "LOCALAPPDATA",
            "APPDATA",
            "ProgramData",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000 | 0x0000_0004);
        }
        let child = command.spawn().map_err(|_| "provider_start_failed")?;
        let mut child = ChildGuard::new(child);
        #[cfg(windows)]
        let _job =
            fleet_onboarding_windows_kernel::ContainedJob::assign_and_resume(child.child_mut())
                .map_err(|_| "provider_containment_failed")?;
        let mut child_stdin = child
            .child_mut()
            .stdin
            .take()
            .ok_or("provider_pipe_failed")?;
        let child_stdout = child
            .child_mut()
            .stdout
            .take()
            .ok_or("provider_pipe_failed")?;
        let child_stderr = child
            .child_mut()
            .stderr
            .take()
            .ok_or("provider_pipe_failed")?;
        let request = stdin.to_vec();

        let (stdin_sender, stdin_receiver) = mpsc::channel();
        let (stdout_sender, stdout_receiver) = mpsc::channel();
        let (stderr_sender, stderr_receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = child_stdin
                .write_all(&request)
                .and_then(|()| child_stdin.flush())
                .map_err(|_| "provider_stdin_failed");
            let _ = stdin_sender.send(result);
        });
        thread::spawn(move || {
            let result = read_bounded(child_stdout, MAX_PROVIDER_STDOUT_BYTES);
            let _ = stdout_sender.send(result);
        });
        thread::spawn(move || {
            let result = read_bounded(child_stderr, MAX_PROVIDER_STDERR_BYTES);
            let _ = stderr_sender.send(result);
        });

        let deadline = Instant::now() + self.timeout;
        let mut stdin_complete = false;
        let mut stdout = None;
        let mut stderr = None;
        let mut process_status = None;
        let status = loop {
            if !stdin_complete {
                match stdin_receiver.try_recv() {
                    Ok(result) => {
                        result?;
                        stdin_complete = true;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err("provider_stdin_failed");
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if stdout.is_none() {
                match stdout_receiver.try_recv() {
                    Ok(result) => stdout = Some(result?),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err("provider_stdout_failed");
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if stderr.is_none() {
                match stderr_receiver.try_recv() {
                    Ok(result) => stderr = Some(result?),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err("provider_stderr_failed");
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if process_status.is_none() {
                process_status = child
                    .child_mut()
                    .try_wait()
                    .map_err(|_| "provider_wait_failed")?;
            }
            if let Some(status) = process_status
                && stdin_complete
                && stdout.is_some()
                && stderr.is_some()
            {
                break status;
            }
            if Instant::now() >= deadline {
                return Err("provider_timeout");
            }
            thread::sleep(Duration::from_millis(5));
        };
        if !stderr.ok_or("provider_stderr_failed")?.is_empty() {
            return Err("provider_stderr_not_empty");
        }
        Ok(ProcessOutput {
            exit_code: status.code().ok_or("provider_exit_code_unavailable")?,
            stdout: stdout.ok_or("provider_stdout_failed")?,
        })
    }
}

struct ChildGuard {
    child: std::process::Child,
}

impl ChildGuard {
    fn new(child: std::process::Child) -> Self {
        Self { child }
    }

    fn child_mut(&mut self) -> &mut std::process::Child {
        &mut self.child
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_bounded(mut reader: impl Read, limit: usize) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::with_capacity(limit.min(8 * 1024));
    reader
        .by_ref()
        .take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| "provider_output_read_failed")?;
    if bytes.len() > limit {
        return Err(if limit == MAX_PROVIDER_STDERR_BYTES {
            "provider_stderr_too_large"
        } else {
            "provider_stdout_too_large"
        });
    }
    Ok(bytes)
}

pub struct FileManagerKioskV2CatalogProvider {
    executable: ExecutableIdentity,
    staged_executable: ExecutableIdentity,
    stage_directory: Option<PathBuf>,
    stage_directory_handles: Vec<File>,
    profiles: BTreeMap<String, FileManagerKioskV2ProfileMapping>,
    runner: Box<dyn ProviderProcessRunner>,
}

impl FileManagerKioskV2CatalogProvider {
    pub fn new(
        config: &FileManagerKioskV2ProviderConfig,
        private_state_directory: &Path,
    ) -> Result<Self, &'static str> {
        let mut provider =
            Self::with_runner(config, Box::<BoundedProviderProcessRunner>::default())?;
        provider.stage_verified_executable(private_state_directory)?;
        Ok(provider)
    }

    fn with_runner(
        config: &FileManagerKioskV2ProviderConfig,
        runner: Box<dyn ProviderProcessRunner>,
    ) -> Result<Self, &'static str> {
        config.validate_shape()?;
        let executable = executable_identity(&config.executable)?;
        if executable.sha256 != config.expected_executable_sha256 {
            return Err("provider_executable_digest_mismatch");
        }
        let profiles = config
            .profiles
            .iter()
            .cloned()
            .map(|mapping| (mapping.device_id.clone(), mapping))
            .collect();
        Ok(Self {
            staged_executable: executable.clone(),
            executable,
            stage_directory: None,
            stage_directory_handles: Vec::new(),
            profiles,
            runner,
        })
    }

    fn recheck_staged_executable(&self) -> Result<(), &'static str> {
        let current = executable_identity(&self.staged_executable.configured_path)?;
        if current.canonical_path != self.staged_executable.canonical_path
            || current.sha256 != self.staged_executable.sha256
        {
            return Err("provider_executable_substituted");
        }
        Ok(())
    }

    pub fn profile_id_for(
        &self,
        device_id: &str,
        identity_revision: u64,
    ) -> Result<String, String> {
        self.profiles
            .get(device_id)
            .filter(|mapping| mapping.identity_revision == identity_revision)
            .map(|mapping| mapping.profile_id.clone())
            .ok_or_else(|| "provider_profile_not_configured".to_owned())
    }

    fn stage_verified_executable(
        &mut self,
        private_state_directory: &Path,
    ) -> Result<(), &'static str> {
        if !private_state_directory.is_absolute() {
            return Err("provider_stage_root_not_absolute");
        }
        std::fs::create_dir_all(private_state_directory)
            .map_err(|_| "provider_stage_root_unavailable")?;
        let private_state_handle = secure_and_lock_private_directory(private_state_directory)?;
        let canonical_state = private_state_directory
            .canonicalize()
            .map_err(|_| "provider_stage_root_unavailable")?;
        let stage_root = private_state_directory.join(PROVIDER_STAGE_ROOT);
        std::fs::create_dir_all(&stage_root).map_err(|_| "provider_stage_create_failed")?;
        let stage_root_handle = secure_and_lock_private_directory(&stage_root)?;
        let canonical_stage_root = stage_root
            .canonicalize()
            .map_err(|_| "provider_stage_root_unavailable")?;
        if canonical_stage_root.parent() != Some(canonical_state.as_path()) {
            return Err("provider_stage_root_escaped");
        }

        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| "provider_stage_identity_failed")?;
        let stage_directory = stage_root.join(format!("stage-{}", hex::encode(random)));
        std::fs::create_dir(&stage_directory).map_err(|_| "provider_stage_create_failed")?;
        let mut cleanup_guard = StageCleanupGuard::new(stage_directory.clone());
        let stage_directory_handle = secure_and_lock_private_directory(&stage_directory)?;
        let mut directory_handles = vec![
            private_state_handle,
            stage_root_handle,
            stage_directory_handle,
        ];
        let canonical_stage_directory = stage_directory
            .canonicalize()
            .map_err(|_| "provider_stage_create_failed")?;
        if canonical_stage_directory.parent() != Some(canonical_stage_root.as_path()) {
            return Err("provider_stage_root_escaped");
        }
        let staged_path = stage_directory.join(PROVIDER_EXECUTABLE_NAME);
        let result = (|| {
            let mut source = lock_verified_identity(&self.executable)?;
            source
                .seek(SeekFrom::Start(0))
                .map_err(|_| "provider_stage_copy_failed")?;
            let mut destination = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged_path)
                .map_err(|_| "provider_stage_copy_failed")?;
            std::io::copy(&mut source, &mut destination)
                .map_err(|_| "provider_stage_copy_failed")?;
            destination
                .sync_all()
                .map_err(|_| "provider_stage_copy_failed")?;
            drop(destination);
            let staged = executable_identity(&staged_path)?;
            if staged.sha256 != self.executable.sha256 {
                return Err("provider_stage_digest_mismatch");
            }
            let _staged_lock = lock_verified_identity(&staged)?;
            std::fs::create_dir(stage_directory.join("bundle-extract"))
                .map_err(|_| "provider_stage_create_failed")?;
            let extraction = stage_directory.join("bundle-extract");
            let extraction_handle = secure_and_lock_private_directory(&extraction)?;
            directory_handles.push(extraction_handle);
            self.staged_executable = staged;
            self.stage_directory = Some(stage_directory.clone());
            self.stage_directory_handles = std::mem::take(&mut directory_handles);
            cleanup_guard.disarm();
            Ok(())
        })();
        drop(directory_handles);
        result
    }
}

impl KioskEncryptedOperatorV2Provider for FileManagerKioskV2CatalogProvider {
    fn verified_catalog_exchange(
        &mut self,
        expectation: &KioskV2CatalogExpectation,
    ) -> Result<VerifiedKioskV2CatalogExchange, String> {
        let _locked_executable =
            lock_verified_identity(&self.staged_executable).map_err(str::to_owned)?;
        if expectation.scopes.as_slice() != [fleet_contracts::KIOSK_CATALOG_SUMMARY_SCOPE] {
            return Err("provider_scope_not_summary_only".to_owned());
        }
        let mapping = self
            .profiles
            .get(&expectation.device_id)
            .filter(|mapping| mapping.identity_revision == expectation.identity_revision)
            .ok_or_else(|| "provider_profile_not_configured".to_owned())?;
        let request = ProviderRequest::from_expectation(&mapping.profile_id, expectation);
        let request_bytes =
            serde_json::to_vec(&request).map_err(|_| "provider_request_invalid".to_owned())?;
        if request_bytes.len() > MAX_PROVIDER_REQUEST_BYTES {
            return Err("provider_request_too_large".to_owned());
        }
        let output = self
            .runner
            .run(
                &self.staged_executable.canonical_path,
                &mapping.profile_id,
                &request_bytes,
            )
            .map_err(str::to_owned)?;
        self.recheck_staged_executable().map_err(str::to_owned)?;
        parse_provider_response(
            &output.stdout,
            output.exit_code,
            &mapping.profile_id,
            &expectation.request_id,
        )
        .map_err(str::to_owned)
    }
}

fn lock_verified_identity(identity: &ExecutableIdentity) -> Result<File, &'static str> {
    let canonical_path = identity
        .configured_path
        .canonicalize()
        .map_err(|_| "provider_executable_unavailable")?;
    if canonical_path != identity.canonical_path {
        return Err("provider_executable_substituted");
    }
    require_exe_path(&canonical_path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        // Share reads only. Retaining this handle through process exit
        // prevents write/delete replacement between verification and
        // Command::spawn on the supported Windows host.
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        options.share_mode(FILE_SHARE_READ);
    }
    let mut file = options
        .open(&canonical_path)
        .map_err(|_| "provider_executable_unavailable")?;
    let metadata = file
        .metadata()
        .map_err(|_| "provider_executable_unavailable")?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_PROVIDER_EXECUTABLE_BYTES
    {
        return Err("provider_executable_invalid");
    }
    let mut hasher = Sha256::new();
    let mut chunk = [0_u8; 64 * 1024];
    let mut saw_pe_header = false;
    loop {
        let count = file
            .read(&mut chunk)
            .map_err(|_| "provider_executable_unavailable")?;
        if count == 0 {
            break;
        }
        if !saw_pe_header {
            if count < 2 || &chunk[..2] != b"MZ" {
                return Err("provider_executable_not_windows_app");
            }
            saw_pe_header = true;
        }
        hasher.update(&chunk[..count]);
    }
    if !saw_pe_header {
        return Err("provider_executable_invalid");
    }
    if hex::encode(hasher.finalize()) != identity.sha256 {
        return Err("provider_executable_substituted");
    }
    Ok(file)
}

impl Drop for FileManagerKioskV2CatalogProvider {
    fn drop(&mut self) {
        self.stage_directory_handles.clear();
        if let Some(stage_directory) = self.stage_directory.take() {
            let _ = remove_stage_tree_bounded(&stage_directory);
        }
    }
}

struct StageCleanupGuard {
    path: PathBuf,
    armed: bool,
}

impl StageCleanupGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StageCleanupGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = remove_stage_tree_bounded(&self.path);
        }
    }
}

const MAX_STAGE_CLEANUP_ENTRIES: usize = 4_096;
const MAX_STAGE_CLEANUP_DEPTH: usize = 8;
const MAX_STAGE_CLEANUP_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Default)]
struct CleanupBudget {
    entries: usize,
    bytes: u64,
}

fn remove_stage_tree_bounded(path: &Path) -> Result<(), &'static str> {
    let parent = path.parent().ok_or("provider_stage_cleanup_invalid")?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| "provider_stage_cleanup_failed")?;
    let mut budget = CleanupBudget::default();
    remove_tree_bounded(path, &canonical_parent, 0, &mut budget)
}

fn remove_tree_bounded(
    path: &Path,
    canonical_parent: &Path,
    depth: usize,
    budget: &mut CleanupBudget,
) -> Result<(), &'static str> {
    if depth > MAX_STAGE_CLEANUP_DEPTH {
        return Err("provider_stage_cleanup_bound_exceeded");
    }
    reject_reparse(path)?;
    let canonical = path
        .canonicalize()
        .map_err(|_| "provider_stage_cleanup_failed")?;
    if canonical.parent() != Some(canonical_parent) {
        return Err("provider_stage_cleanup_invalid");
    }
    budget.entries = budget.entries.saturating_add(1);
    if budget.entries > MAX_STAGE_CLEANUP_ENTRIES {
        return Err("provider_stage_cleanup_bound_exceeded");
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "provider_stage_cleanup_failed")?;
    if metadata.is_file() {
        budget.bytes = budget.bytes.saturating_add(metadata.len());
        if budget.bytes > MAX_STAGE_CLEANUP_BYTES {
            return Err("provider_stage_cleanup_bound_exceeded");
        }
        return std::fs::remove_file(path).map_err(|_| "provider_stage_cleanup_failed");
    }
    if !metadata.is_dir() {
        return Err("provider_stage_cleanup_invalid");
    }
    for entry in std::fs::read_dir(path).map_err(|_| "provider_stage_cleanup_failed")? {
        let entry = entry.map_err(|_| "provider_stage_cleanup_failed")?;
        remove_tree_bounded(&entry.path(), &canonical, depth.saturating_add(1), budget)?;
    }
    std::fs::remove_dir(path).map_err(|_| "provider_stage_cleanup_failed")
}

#[cfg(windows)]
fn reject_reparse(path: &Path) -> Result<(), &'static str> {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "provider_stage_path_unavailable")?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        Err("provider_stage_reparse_rejected")
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn reject_reparse(path: &Path) -> Result<(), &'static str> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "provider_stage_path_unavailable")?;
    if metadata.file_type().is_symlink() {
        Err("provider_stage_reparse_rejected")
    } else {
        Ok(())
    }
}

fn executable_identity(path: &Path) -> Result<ExecutableIdentity, &'static str> {
    require_exe_path(path)?;
    let canonical_path = path
        .canonicalize()
        .map_err(|_| "provider_executable_unavailable")?;
    require_exe_path(&canonical_path)?;
    let metadata =
        std::fs::metadata(&canonical_path).map_err(|_| "provider_executable_unavailable")?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_PROVIDER_EXECUTABLE_BYTES
    {
        return Err("provider_executable_invalid");
    }
    let mut file = File::open(&canonical_path).map_err(|_| "provider_executable_unavailable")?;
    let mut hasher = Sha256::new();
    let mut chunk = [0_u8; 64 * 1024];
    let mut saw_pe_header = false;
    loop {
        let count = file
            .read(&mut chunk)
            .map_err(|_| "provider_executable_unavailable")?;
        if count == 0 {
            break;
        }
        if !saw_pe_header {
            if count < 2 || &chunk[..2] != b"MZ" {
                return Err("provider_executable_not_windows_app");
            }
            saw_pe_header = true;
        }
        hasher.update(&chunk[..count]);
    }
    if !saw_pe_header {
        return Err("provider_executable_invalid");
    }
    Ok(ExecutableIdentity {
        configured_path: path.to_path_buf(),
        canonical_path,
        sha256: hex::encode(hasher.finalize()),
    })
}

fn require_exe_path(path: &Path) -> Result<(), &'static str> {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(PROVIDER_EXECUTABLE_NAME))
    {
        Ok(())
    } else {
        Err("provider_executable_not_windows_app")
    }
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct ProviderRequest<'a> {
    schema: &'static str,
    profile_id: &'a str,
    request_id: &'a str,
    device_id: &'a str,
    identity_revision: u64,
    capability_id: &'a str,
    capability_evidence_revision: u64,
    route_id: &'a str,
    expected_owner_epoch: Option<&'a str>,
    scopes: &'a [String],
    issued_at_ms: i64,
    expires_at_ms: i64,
}

impl<'a> ProviderRequest<'a> {
    fn from_expectation(profile_id: &'a str, expectation: &'a KioskV2CatalogExpectation) -> Self {
        Self {
            schema: FILE_MANAGER_PROVIDER_REQUEST_SCHEMA,
            profile_id,
            request_id: &expectation.request_id,
            device_id: &expectation.device_id,
            identity_revision: expectation.identity_revision,
            capability_id: &expectation.capability_id,
            capability_evidence_revision: expectation.capability_evidence_revision,
            route_id: &expectation.route_id,
            expected_owner_epoch: expectation.expected_owner_epoch.as_deref(),
            scopes: &expectation.scopes,
            issued_at_ms: expectation.issued_at_ms,
            expires_at_ms: expectation.expires_at_ms,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedProviderResponse {
    schema: String,
    profile_id: String,
    request_id: String,
    request_envelope_base64url: String,
    response_envelope_base64url: String,
    owner_catalog_json_base64url: String,
    owner_grant_receipt_base64url: String,
    response_request_id: String,
    replay_identity: String,
    capability_id: String,
    capability_evidence_revision: u64,
    grant_revision: u64,
    route_id: String,
    device_id: String,
    identity_revision: u64,
    kiosk_owner_epoch: String,
    issued_at_ms: i64,
    expires_at_ms: i64,
    scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FailedProviderResponse {
    schema: String,
    profile_id: String,
    request_id: String,
    error_code: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", deny_unknown_fields)]
enum ProviderResponse {
    #[serde(rename = "verified")]
    Verified(Box<VerifiedProviderResponse>),
    #[serde(rename = "unavailable")]
    Unavailable(FailedProviderResponse),
    #[serde(rename = "rejected")]
    Rejected(FailedProviderResponse),
    #[serde(rename = "failed")]
    Failed(FailedProviderResponse),
}

struct ExpectedProviderResponse<'a> {
    profile_id: &'a str,
    request_id: &'a str,
}

fn parse_provider_response(
    bytes: &[u8],
    exit_code: i32,
    expected_profile_id: &str,
    expected_request_id: &str,
) -> Result<VerifiedKioskV2CatalogExchange, &'static str> {
    if bytes.is_empty() || bytes.len() > MAX_PROVIDER_STDOUT_BYTES {
        return Err("provider_response_size_invalid");
    }
    let response: ProviderResponse =
        serde_json::from_slice(bytes).map_err(|_| "provider_response_invalid")?;
    let expected = ExpectedProviderResponse {
        profile_id: expected_profile_id,
        request_id: expected_request_id,
    };
    match response {
        ProviderResponse::Verified(verified) => {
            let VerifiedProviderResponse {
                schema,
                profile_id,
                request_id,
                request_envelope_base64url,
                response_envelope_base64url,
                owner_catalog_json_base64url,
                owner_grant_receipt_base64url,
                response_request_id,
                replay_identity,
                capability_id,
                capability_evidence_revision,
                grant_revision,
                route_id,
                device_id,
                identity_revision,
                kiosk_owner_epoch,
                issued_at_ms,
                expires_at_ms,
                scopes,
            } = *verified;
            if exit_code != 0 {
                return Err("provider_exit_status_mismatch");
            }
            validate_response_header(&schema, &profile_id, &request_id, &expected)?;
            if !valid_sha256(&replay_identity) {
                return Err("provider_replay_identity_invalid");
            }
            Ok(VerifiedKioskV2CatalogExchange {
                request_envelope: decode_canonical(
                    &request_envelope_base64url,
                    MAX_KIOSK_V2_REQUEST_ENVELOPE_BYTES,
                )?,
                response_envelope: decode_canonical(
                    &response_envelope_base64url,
                    MAX_KIOSK_V2_RESPONSE_ENVELOPE_BYTES,
                )?,
                owner_catalog_json: decode_canonical(
                    &owner_catalog_json_base64url,
                    MAX_KIOSK_V2_OWNER_CATALOG_BYTES,
                )?,
                owner_grant_receipt: decode_canonical(
                    &owner_grant_receipt_base64url,
                    MAX_KIOSK_V2_OWNER_GRANT_RECEIPT_BYTES,
                )?,
                response_request_id,
                replay_identity,
                capability_id,
                capability_evidence_revision,
                grant_revision,
                route_id,
                device_id,
                identity_revision,
                kiosk_owner_epoch,
                issued_at_ms,
                expires_at_ms,
                scopes,
            })
        }
        ProviderResponse::Unavailable(failure) => {
            validate_failure_response(exit_code, 3, &failure, &expected)
        }
        ProviderResponse::Rejected(failure) => {
            validate_failure_response(exit_code, 2, &failure, &expected)
        }
        ProviderResponse::Failed(failure) => {
            validate_failure_response(exit_code, 1, &failure, &expected)
        }
    }
}

fn validate_failure_response(
    exit_code: i32,
    expected_exit_code: i32,
    failure: &FailedProviderResponse,
    expected: &ExpectedProviderResponse<'_>,
) -> Result<VerifiedKioskV2CatalogExchange, &'static str> {
    if exit_code != expected_exit_code {
        return Err("provider_exit_status_mismatch");
    }
    validate_response_header(
        &failure.schema,
        &failure.profile_id,
        &failure.request_id,
        expected,
    )?;
    if !valid_token(&failure.error_code, 1, 128) {
        return Err("provider_response_invalid");
    }
    // Do not surface File Manager's code, message, or stderr: they may contain
    // endpoint, pairing, session, or decrypted owner context.
    Err("provider_owner_unavailable")
}

fn validate_response_header(
    schema: &str,
    profile_id: &str,
    request_id: &str,
    expected: &ExpectedProviderResponse<'_>,
) -> Result<(), &'static str> {
    if schema != FILE_MANAGER_PROVIDER_RESPONSE_SCHEMA {
        return Err("provider_response_schema_mismatch");
    }
    if profile_id != expected.profile_id {
        return Err("provider_profile_substituted");
    }
    if request_id != expected.request_id {
        return Err("provider_request_substituted");
    }
    Ok(())
}

fn decode_canonical(encoded: &str, maximum: usize) -> Result<Vec<u8>, &'static str> {
    let maximum_encoded = maximum
        .checked_add(2)
        .and_then(|value| value.checked_div(3))
        .and_then(|value| value.checked_mul(4))
        .ok_or("provider_response_size_invalid")?;
    if encoded.is_empty() || encoded.len() > maximum_encoded || encoded.contains('=') {
        return Err("provider_response_size_invalid");
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "provider_response_base64_invalid")?;
    if decoded.len() > maximum || URL_SAFE_NO_PAD.encode(&decoded) != encoded {
        return Err("provider_response_base64_invalid");
    }
    Ok(decoded)
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use fleet_contracts::{
        KIOSK_CATALOG_SUMMARY_SCOPE, KIOSK_DIRECT_OPERATOR_CAPABILITY_ID,
        KIOSK_V2_ENCRYPTED_ROUTE_ID,
    };

    use super::*;

    struct RecordedProcessCall {
        _executable: PathBuf,
        _profile_id: String,
        stdin: Vec<u8>,
    }

    struct RecordingRunner {
        output: ProcessOutput,
        seen: Arc<Mutex<Vec<RecordedProcessCall>>>,
    }

    impl ProviderProcessRunner for RecordingRunner {
        fn run(
            &mut self,
            executable: &Path,
            profile_id: &str,
            stdin: &[u8],
        ) -> Result<ProcessOutput, &'static str> {
            self.seen
                .lock()
                .expect("recording lock")
                .push(RecordedProcessCall {
                    _executable: executable.to_path_buf(),
                    _profile_id: profile_id.to_owned(),
                    stdin: stdin.to_vec(),
                });
            Ok(self.output.clone())
        }
    }

    fn temporary_executable(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "rusty-fleet-provider-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("create fixture directory");
        let path = directory.join(PROVIDER_EXECUTABLE_NAME);
        std::fs::write(&path, b"MZ not executed; exact identity fixture").expect("write fixture");
        path
    }

    fn config(path: PathBuf) -> FileManagerKioskV2ProviderConfig {
        let digest = hex::encode(Sha256::digest(
            std::fs::read(&path).unwrap_or_else(|_| b"absent".to_vec()),
        ));
        FileManagerKioskV2ProviderConfig {
            executable: path,
            expected_executable_sha256: digest,
            profiles: vec![FileManagerKioskV2ProfileMapping {
                device_id: "device.quest.1".to_owned(),
                identity_revision: 7,
                profile_id: "opaque-profile-1".to_owned(),
            }],
        }
    }

    fn expectation() -> KioskV2CatalogExpectation {
        KioskV2CatalogExpectation {
            request_id: "catalog-request-0001".to_owned(),
            device_id: "device.quest.1".to_owned(),
            identity_revision: 7,
            capability_id: KIOSK_DIRECT_OPERATOR_CAPABILITY_ID.to_owned(),
            capability_evidence_revision: 9,
            route_id: KIOSK_V2_ENCRYPTED_ROUTE_ID.to_owned(),
            expected_owner_epoch: Some("KioskOwnerEpoch_001".to_owned()),
            scopes: vec![KIOSK_CATALOG_SUMMARY_SCOPE.to_owned()],
            issued_at_ms: 1_000,
            expires_at_ms: 31_000,
        }
    }

    fn success_response(profile_id: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema": FILE_MANAGER_PROVIDER_RESPONSE_SCHEMA,
            "status": "verified",
            "profile_id": profile_id,
            "request_id": "catalog-request-0001",
            "request_envelope_base64url": URL_SAFE_NO_PAD.encode(b"request"),
            "response_envelope_base64url": URL_SAFE_NO_PAD.encode(b"response"),
            "owner_catalog_json_base64url": URL_SAFE_NO_PAD.encode(br#"{"catalog":"owner"}"#),
            "owner_grant_receipt_base64url": URL_SAFE_NO_PAD.encode(b"receipt"),
            "response_request_id": "catalog-request-0001",
            "replay_identity": "b".repeat(64),
            "capability_id": KIOSK_DIRECT_OPERATOR_CAPABILITY_ID,
            "capability_evidence_revision": 9,
            "grant_revision": 4,
            "route_id": KIOSK_V2_ENCRYPTED_ROUTE_ID,
            "device_id": "device.quest.1",
            "identity_revision": 7,
            "kiosk_owner_epoch": "KioskOwnerEpoch_001",
            "issued_at_ms": 1_000,
            "expires_at_ms": 31_000,
            "scopes": [KIOSK_CATALOG_SUMMARY_SCOPE]
        }))
        .expect("response")
    }

    #[test]
    fn request_omits_manifold_barrier_and_binds_exact_profile() {
        let executable = temporary_executable("request");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let runner = RecordingRunner {
            output: ProcessOutput {
                exit_code: 0,
                stdout: success_response("opaque-profile-1"),
            },
            seen: Arc::clone(&seen),
        };
        let mut provider = FileManagerKioskV2CatalogProvider::with_runner(
            &config(executable.clone()),
            Box::new(runner),
        )
        .expect("provider");
        provider
            .verified_catalog_exchange(&expectation())
            .expect("verified");
        let records = seen.lock().expect("seen");
        let request: serde_json::Value =
            serde_json::from_slice(&records[0].stdin).expect("request JSON");
        assert_eq!(request["profile_id"], "opaque-profile-1");
        assert!(request.get("manifold_revocation_barrier").is_none());
        assert_eq!(
            request["scopes"],
            serde_json::json!([KIOSK_CATALOG_SUMMARY_SCOPE])
        );
        drop(records);
        std::fs::remove_file(executable).expect("remove fixture");
    }

    #[test]
    fn profile_response_and_executable_substitution_fail_closed() {
        let executable = temporary_executable("substitution");
        let runner = RecordingRunner {
            output: ProcessOutput {
                exit_code: 0,
                stdout: success_response("other-profile"),
            },
            seen: Arc::new(Mutex::new(Vec::new())),
        };
        let mut provider = FileManagerKioskV2CatalogProvider::with_runner(
            &config(executable.clone()),
            Box::new(runner),
        )
        .expect("provider");
        assert_eq!(
            provider
                .verified_catalog_exchange(&expectation())
                .expect_err("profile substitution"),
            "provider_profile_substituted"
        );
        std::fs::write(&executable, b"MZ substituted executable").expect("substitute fixture");
        assert_eq!(
            provider
                .verified_catalog_exchange(&expectation())
                .expect_err("executable substitution"),
            "provider_executable_substituted"
        );
        std::fs::remove_file(executable).expect("remove fixture");
    }

    #[test]
    fn noncanonical_base64_and_unknown_response_fields_are_rejected() {
        let mut padded: serde_json::Value =
            serde_json::from_slice(&success_response("opaque-profile-1")).expect("response");
        padded["request_envelope_base64url"] = serde_json::json!("cmVxdWVzdA==");
        assert_eq!(
            parse_provider_response(
                &serde_json::to_vec(&padded).expect("padded"),
                0,
                "opaque-profile-1",
                "catalog-request-0001"
            ),
            Err("provider_response_size_invalid")
        );
        padded["request_envelope_base64url"] = serde_json::json!("cmVxdWVzdA");
        padded["pairing_key"] = serde_json::json!("must-never-be-accepted");
        assert_eq!(
            parse_provider_response(
                &serde_json::to_vec(&padded).expect("unknown"),
                0,
                "opaque-profile-1",
                "catalog-request-0001"
            ),
            Err("provider_response_invalid")
        );
    }

    #[test]
    fn structured_failure_requires_its_exact_exit_code() {
        let unavailable = serde_json::to_vec(&serde_json::json!({
            "schema": FILE_MANAGER_PROVIDER_RESPONSE_SCHEMA,
            "status": "unavailable",
            "profile_id": "opaque-profile-1",
            "request_id": "catalog-request-0001",
            "error_code": "profile_unavailable"
        }))
        .expect("failure response");
        assert_eq!(
            parse_provider_response(&unavailable, 3, "opaque-profile-1", "catalog-request-0001"),
            Err("provider_owner_unavailable")
        );
        assert_eq!(
            parse_provider_response(&unavailable, 0, "opaque-profile-1", "catalog-request-0001"),
            Err("provider_exit_status_mismatch")
        );
    }

    #[test]
    fn config_rejects_duplicate_or_unbounded_exact_mappings() {
        let mut damaged = config(PathBuf::from("relative.exe"));
        assert_eq!(
            damaged.validate_shape(),
            Err("provider_executable_not_absolute")
        );
        damaged.executable = std::env::temp_dir().join(PROVIDER_EXECUTABLE_NAME);
        damaged.expected_executable_sha256 = "a".repeat(64);
        damaged.profiles.push(damaged.profiles[0].clone());
        assert_eq!(
            damaged.validate_shape(),
            Err("provider_profile_mapping_invalid")
        );
        damaged.profiles.truncate(1);
        damaged.executable = std::env::temp_dir().join("provider.cmd");
        assert_eq!(
            damaged.validate_shape(),
            Err("provider_executable_not_windows_app")
        );
    }

    #[test]
    fn concrete_provider_stages_only_the_pinned_dedicated_artifact() {
        let executable = temporary_executable("stage");
        let sidecar = executable.with_file_name("questionable-file-manager.dll");
        std::fs::write(&sidecar, b"must not be staged").expect("write sidecar");
        let private_state =
            std::env::temp_dir().join(format!("rusty-fleet-provider-state-{}", std::process::id()));
        std::fs::create_dir_all(&private_state).expect("private state");
        let provider =
            FileManagerKioskV2CatalogProvider::new(&config(executable.clone()), &private_state)
                .expect("staged provider");
        let stage_directory = provider
            .stage_directory
            .clone()
            .expect("concrete provider stage");
        assert_ne!(
            provider.executable.canonical_path,
            provider.staged_executable.canonical_path
        );
        assert_eq!(
            provider.executable.sha256,
            provider.staged_executable.sha256
        );
        assert_eq!(
            std::fs::read_dir(&stage_directory)
                .expect("stage entries")
                .map(|entry| entry.expect("stage entry").file_name())
                .collect::<BTreeSet<_>>(),
            [
                std::ffi::OsString::from(PROVIDER_EXECUTABLE_NAME),
                std::ffi::OsString::from("bundle-extract"),
            ]
            .into_iter()
            .collect()
        );
        assert!(
            !stage_directory
                .join("questionable-file-manager.dll")
                .exists()
        );
        drop(provider);
        assert!(!stage_directory.exists());
        std::fs::remove_file(&executable).expect("remove executable");
        std::fs::remove_file(sidecar).expect("remove sidecar");
        std::fs::remove_dir(executable.parent().expect("fixture parent"))
            .expect("remove fixture directory");
        std::fs::remove_dir_all(private_state).expect("remove private state");
    }
}

#[cfg(windows)]
fn secure_and_lock_private_directory(path: &Path) -> Result<File, &'static str> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_permissions::constants::{SeObjectType, SecurityInformation};
    use windows_permissions::{SecurityDescriptor, wrappers};
    fleet_onboarding_windows_kernel::require_local_drive(
        path.ancestors()
            .last()
            .ok_or("provider_stage_root_invalid")?,
    )
    .map_err(|_| "provider_stage_local_drive_required")?;
    for ancestor in path.ancestors() {
        reject_reparse(ancestor)?;
    }
    let mut handle = OpenOptions::new()
        .read(true)
        .access_mode(0x0002_0000 | 0x0004_0000 | 0x0008_0000 | 0x80)
        .share_mode(3)
        .custom_flags(0x0200_0000 | 0x0020_0000)
        .open(path)
        .map_err(|_| "provider_stage_lock_failed")?;
    let sid = windows_permissions::utilities::current_process_sid()
        .map_err(|_| "provider_stage_identity_failed")?;
    let descriptor: windows_permissions::LocalBox<SecurityDescriptor> =
        format!("O:{sid}D:P(A;OICI;FA;;;{sid})")
            .parse()
            .map_err(|_| "provider_stage_acl_failed")?;
    wrappers::SetSecurityInfo(
        &mut handle,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Owner | SecurityInformation::Dacl | SecurityInformation::ProtectedDacl,
        descriptor.owner(),
        None,
        descriptor.dacl(),
        None,
    )
    .map_err(|_| "provider_stage_acl_failed")?;
    let actual = wrappers::GetSecurityInfo(
        &handle,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Owner | SecurityInformation::Dacl,
    )
    .map_err(|_| "provider_stage_acl_failed")?;
    use windows_permissions::constants::{AccessRights, AceType};
    let acl = actual.dacl().ok_or("provider_stage_acl_missing")?;
    let sddl = actual.as_sddl().map_err(|_| "provider_stage_acl_failed")?;
    if actual.owner() != Some(&*sid) || acl.len() != 1 || !sddl.to_string_lossy().contains("D:P") {
        return Err("provider_stage_acl_mismatch");
    }
    let ace = acl.get_ace(0).ok_or("provider_stage_acl_missing")?;
    if ace.ace_type() != AceType::ACCESS_ALLOWED_ACE_TYPE
        || ace.sid() != Some(&*sid)
        || !(ace.mask().contains(AccessRights::FileAllAccess)
            || ace.mask().contains(AccessRights::GenericAll))
    {
        return Err("provider_stage_acl_mismatch");
    }
    Ok(handle)
}
#[cfg(not(windows))]
fn secure_and_lock_private_directory(_path: &Path) -> Result<File, &'static str> {
    Err("provider_requires_windows")
}
// Exact optional cross-owner source evidence. The ordinary suite remains host-only.
#[cfg(test)]
mod interop_tests {
    use super::*;
    use fleet_contracts::ValidateContract;
    use fleet_kiosk_adapter::{KioskV2AdapterError, ingest_production_catalog};
    struct VerifiedOnce(Option<VerifiedKioskV2CatalogExchange>);
    impl KioskEncryptedOperatorV2Provider for VerifiedOnce {
        fn verified_catalog_exchange(
            &mut self,
            _: &KioskV2CatalogExpectation,
        ) -> Result<VerifiedKioskV2CatalogExchange, String> {
            self.0.take().ok_or_else(|| "consumed".to_owned())
        }
    }
    #[test]
    fn actual_kiosk_qfm_bytes_join_strict_fleet_parser_and_admission() {
        let Ok(response_path) = std::env::var("FLEET_KIOSK_INTEROP_RESPONSE") else {
            eprintln!("cross-owner fixture not selected");
            return;
        };
        let request_path =
            std::env::var("FLEET_KIOSK_INTEROP_REQUEST").expect("explicit matching request");
        let bytes = std::fs::read(response_path).expect("immutable response");
        assert_eq!(
            crate::sha256_hex(&bytes),
            "63fe2bcb5fc62006967412836aef467a0f4c2528a39d3575264f4814ba2f0e25"
        );
        let request_bytes = std::fs::read(request_path).expect("request");
        assert_eq!(
            crate::sha256_hex(&request_bytes),
            "09a5af81a1a985318e35241714a8cc8198886d7a3581d2cb64822462721541fa"
        );
        let request: serde_json::Value =
            serde_json::from_slice(&request_bytes).expect("request JSON");
        let string = |key: &str| request[key].as_str().expect("string").to_owned();
        let expected = KioskV2CatalogExpectation {
            request_id: string("request_id"),
            device_id: string("device_id"),
            identity_revision: request["identity_revision"].as_u64().expect("revision"),
            capability_id: string("capability_id"),
            capability_evidence_revision: request["capability_evidence_revision"]
                .as_u64()
                .expect("revision"),
            route_id: string("route_id"),
            expected_owner_epoch: Some(string("expected_owner_epoch")),
            scopes: serde_json::from_value(request["scopes"].clone()).expect("scopes"),
            issued_at_ms: request["issued_at_ms"].as_i64().expect("time"),
            expires_at_ms: request["expires_at_ms"].as_i64().expect("time"),
        };
        let parse = |raw: &[u8]| {
            parse_provider_response(raw, 0, &string("profile_id"), &expected.request_id)
        };
        let exchange = parse(&bytes).expect("actual QFM host response");
        let received_at_ms = 1_900_000_000_001;
        let admission = ingest_production_catalog(
            &mut VerifiedOnce(Some(exchange.clone())),
            &expected,
            received_at_ms,
        )
        .expect("actual production owner summary accepted by Fleet");
        assert!(admission.validate().is_ok());
        assert_eq!(admission.snapshot.entries.len(), 1);
        assert!(!admission.snapshot.grants.app_launch);
        assert!(!admission.snapshot.grants.catalog_detail);
        for (field, value) in [
            ("response_request_id", serde_json::json!("wrong-request")),
            ("capability_id", serde_json::json!("wrong-capability")),
            ("capability_evidence_revision", serde_json::json!(99)),
            ("grant_revision", serde_json::json!(0)),
            ("route_id", serde_json::json!("wrong-route")),
            ("device_id", serde_json::json!("wrong-device")),
            ("identity_revision", serde_json::json!(99)),
            (
                "kiosk_owner_epoch",
                serde_json::json!("WrongOwnerEpoch_0001"),
            ),
            ("issued_at_ms", serde_json::json!(expected.issued_at_ms + 1)),
            (
                "expires_at_ms",
                serde_json::json!(expected.expires_at_ms + 1),
            ),
            ("scopes", serde_json::json!(["kiosk.catalog-detail"])),
            ("replay_identity", serde_json::json!("invalid")),
        ] {
            let mut changed: serde_json::Value = serde_json::from_slice(&bytes).expect("response");
            changed[field] = value;
            if let Ok(exchange) = parse(&serde_json::to_vec(&changed).expect("damaged")) {
                assert!(
                    ingest_production_catalog(
                        &mut VerifiedOnce(Some(exchange)),
                        &expected,
                        received_at_ms
                    )
                    .is_err(),
                    "substitution {field}"
                );
            }
        }
        let mut wrong = admission.clone();
        wrong.binding.capability_id = "other.capability".to_owned();
        assert!(wrong.validate().is_err());
        let mut wrong = admission.clone();
        wrong.binding.expires_at_ms = wrong.binding.issued_at_ms + 30_001;
        wrong
            .snapshot
            .operator_authority
            .as_mut()
            .expect("authority")
            .expires_at_ms = wrong.binding.expires_at_ms;
        assert!(wrong.validate().is_err());
        assert_eq!(
            ingest_production_catalog(
                &mut VerifiedOnce(Some(exchange)),
                &expected,
                expected.expires_at_ms + 1
            ),
            Err(KioskV2AdapterError::InvalidExpectation)
        );
    }
}

#[cfg(all(test, windows))]
#[path = "kiosk_provider_process_tests.rs"]
mod process_tests;
