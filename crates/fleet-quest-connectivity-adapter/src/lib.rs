// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fleet_contracts::{
    QUEST_WIFI_ADB_RECEIPT_SCHEMA, QuestWifiAdbOwnerInvocation, QuestWifiAdbOwnerReceipt,
    ValidateContract,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const PROVIDER_FILE_NAME: &str = "questionable-file-manager-connectivity-provider.exe";
const PROVIDER_RESPONSE_SCHEMA: &str =
    "questionable.file_manager.quest_wifi_adb_provider_response.v1";
const MAX_PROVIDER_OUTPUT_BYTES: usize = 64 * 1024;
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(30);
static PROVIDER_STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestConnectivityProviderConfig {
    pub executable_path: PathBuf,
    pub executable_sha256: String,
    pub private_stage_root: PathBuf,
}

impl QuestConnectivityProviderConfig {
    pub fn validate(&self) -> Result<(), QuestConnectivityAdapterError> {
        if !self.executable_path.is_absolute()
            || !self.private_stage_root.is_absolute()
            || self
                .executable_path
                .file_name()
                .and_then(|value| value.to_str())
                != Some(PROVIDER_FILE_NAME)
            || !is_lower_sha256(&self.executable_sha256)
        {
            return Err(QuestConnectivityAdapterError::new(
                "provider_config_invalid",
                "Quest connectivity provider requires the exact absolute executable, lowercase SHA-256 pin, and an absolute private stage root",
            ));
        }
        Ok(())
    }

    pub fn verify_artifact(&self) -> Result<(), QuestConnectivityAdapterError> {
        self.validate()?;
        if sha256_file(&self.executable_path)? != self.executable_sha256 {
            return Err(QuestConnectivityAdapterError::new(
                "provider_digest_mismatch",
                "Quest connectivity provider does not match its configured SHA-256",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestConnectivityAdapterError {
    pub code: String,
    pub message: String,
}

impl QuestConnectivityAdapterError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for QuestConnectivityAdapterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for QuestConnectivityAdapterError {}

pub trait QuestConnectivityProviderTransport: Send + Sync {
    fn invoke(
        &self,
        config: &QuestConnectivityProviderConfig,
        request_json: &[u8],
        stage_id: &str,
    ) -> Result<QuestConnectivityProviderOutput, QuestConnectivityAdapterError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestConnectivityProviderOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessQuestConnectivityProviderTransport;

impl QuestConnectivityProviderTransport for ProcessQuestConnectivityProviderTransport {
    fn invoke(
        &self,
        config: &QuestConnectivityProviderConfig,
        request_json: &[u8],
        stage_id: &str,
    ) -> Result<QuestConnectivityProviderOutput, QuestConnectivityAdapterError> {
        config.verify_artifact()?;
        if !is_portable_id(stage_id, 128) {
            return Err(QuestConnectivityAdapterError::new(
                "provider_stage_invalid",
                "Quest connectivity provider stage identity is invalid",
            ));
        }
        fs::create_dir_all(&config.private_stage_root).map_err(|error| {
            QuestConnectivityAdapterError::new(
                "provider_stage_failed",
                format!("cannot create private provider stage root: {error}"),
            )
        })?;
        if fs::symlink_metadata(&config.private_stage_root)
            .map(|metadata| metadata.file_type().is_symlink() || !metadata.is_dir())
            .unwrap_or(true)
        {
            return Err(QuestConnectivityAdapterError::new(
                "provider_stage_invalid",
                "private provider stage root must be a real directory",
            ));
        }
        let stage = config.private_stage_root.join(stage_id);
        if stage.exists() {
            return Err(QuestConnectivityAdapterError::new(
                "provider_stage_conflict",
                "Quest connectivity provider stage already exists",
            ));
        }
        fs::create_dir(&stage).map_err(|error| {
            QuestConnectivityAdapterError::new(
                "provider_stage_failed",
                format!("cannot create provider stage: {error}"),
            )
        })?;
        let staged_executable = stage.join(PROVIDER_FILE_NAME);
        let bundle_extract = stage.join("bundle-extract");
        let result = (|| {
            fs::copy(&config.executable_path, &staged_executable).map_err(|error| {
                QuestConnectivityAdapterError::new(
                    "provider_stage_failed",
                    format!("cannot stage Quest connectivity provider: {error}"),
                )
            })?;
            fs::create_dir(&bundle_extract).map_err(|error| {
                QuestConnectivityAdapterError::new(
                    "provider_stage_failed",
                    format!("cannot create provider bundle extraction stage: {error}"),
                )
            })?;
            if sha256_file(&staged_executable)? != config.executable_sha256 {
                return Err(QuestConnectivityAdapterError::new(
                    "provider_stage_digest_mismatch",
                    "staged Quest connectivity provider changed after copy",
                ));
            }
            run_process(&staged_executable, &bundle_extract, request_json)
        })();
        let cleanup = fs::remove_dir_all(&stage);
        match (result, cleanup) {
            (Ok(output), Ok(())) => Ok(output),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(_)) => Err(QuestConnectivityAdapterError::new(
                "provider_cleanup_failed",
                "Quest connectivity provider stage could not be removed",
            )),
            (Err(error), Err(_)) => Err(QuestConnectivityAdapterError::new(
                "provider_cleanup_failed",
                format!(
                    "Quest connectivity provider stage could not be removed after {}",
                    error.code
                ),
            )),
        }
    }
}

#[derive(Clone, Debug)]
pub struct QuestConnectivityOwnerAdapter<T = ProcessQuestConnectivityProviderTransport> {
    transport: T,
}

impl Default for QuestConnectivityOwnerAdapter<ProcessQuestConnectivityProviderTransport> {
    fn default() -> Self {
        Self {
            transport: ProcessQuestConnectivityProviderTransport,
        }
    }
}

impl<T: QuestConnectivityProviderTransport> QuestConnectivityOwnerAdapter<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    pub fn invoke(
        &self,
        config: &QuestConnectivityProviderConfig,
        invocation: &QuestWifiAdbOwnerInvocation,
    ) -> Result<QuestWifiAdbOwnerReceipt, QuestConnectivityAdapterError> {
        invocation.validate().map_err(|failures| {
            QuestConnectivityAdapterError::new(
                "invocation_invalid",
                failures
                    .iter()
                    .map(|failure| format!("{}:{}", failure.path, failure.code))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        let request_json = serde_json::to_vec(invocation).map_err(|error| {
            QuestConnectivityAdapterError::new(
                "provider_request_invalid",
                format!("cannot serialize Quest connectivity request: {error}"),
            )
        })?;
        let output = self.transport.invoke(
            config,
            &request_json,
            &unique_stage_id(&invocation.request_id)?,
        )?;
        if output.stdout.len() > MAX_PROVIDER_OUTPUT_BYTES {
            return Err(QuestConnectivityAdapterError::new(
                "provider_output_oversized",
                "Quest connectivity provider response exceeds 64 KiB",
            ));
        }
        let response: ProviderResponse =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                QuestConnectivityAdapterError::new(
                    "provider_response_invalid",
                    format!("Quest connectivity provider response is not strict JSON: {error}"),
                )
            })?;
        if response.schema != PROVIDER_RESPONSE_SCHEMA {
            return Err(QuestConnectivityAdapterError::new(
                "provider_contract_mismatch",
                "Quest connectivity provider returned a different response schema",
            ));
        }
        let expected_exit_code = response.expected_exit_code()?;
        if output.exit_code != expected_exit_code {
            return Err(QuestConnectivityAdapterError::new(
                "provider_exit_status_mismatch",
                format!(
                    "Quest connectivity provider exit code {} contradicts structured status {}",
                    output.exit_code, response.status
                ),
            ));
        }
        if !matches!(response.status.as_str(), "verified" | "pending") {
            return Err(QuestConnectivityAdapterError::new(
                response.error.as_deref().unwrap_or("provider_failed"),
                response.message.unwrap_or_else(|| {
                    "Quest connectivity provider did not return a receipt".into()
                }),
            ));
        }
        let receipt = response.receipt.ok_or_else(|| {
            QuestConnectivityAdapterError::new(
                "provider_receipt_missing",
                "Quest connectivity provider returned no receipt",
            )
        })?;
        if receipt.schema != QUEST_WIFI_ADB_RECEIPT_SCHEMA
            || receipt.request_id != invocation.request_id
            || receipt.operation_id != invocation.operation_id
            || receipt.preview_id != invocation.preview_id
            || receipt.device_id != invocation.device_id
            || receipt.identity_revision != invocation.identity_revision
            || receipt.action != invocation.action
        {
            return Err(QuestConnectivityAdapterError::new(
                "provider_receipt_binding_mismatch",
                "Quest connectivity receipt does not bind the exact Fleet invocation",
            ));
        }
        receipt.validate().map_err(|failures| {
            QuestConnectivityAdapterError::new(
                "provider_receipt_invalid",
                failures
                    .iter()
                    .map(|failure| format!("{}:{}", failure.path, failure.code))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        if (response.status == "verified") != receipt.effect_applied {
            return Err(QuestConnectivityAdapterError::new(
                "provider_optimistic_success",
                "provider status contradicts the independently validated effect facts",
            ));
        }
        Ok(receipt)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderResponse {
    schema: String,
    status: String,
    receipt: Option<QuestWifiAdbOwnerReceipt>,
    error: Option<String>,
    message: Option<String>,
}

impl ProviderResponse {
    fn expected_exit_code(&self) -> Result<i32, QuestConnectivityAdapterError> {
        match self.status.as_str() {
            "verified" => Ok(0),
            "failed" => Ok(1),
            "rejected" => Ok(2),
            "pending" => Ok(3),
            "cancelled" => Ok(4),
            _ => Err(QuestConnectivityAdapterError::new(
                "provider_status_invalid",
                "Quest connectivity provider returned an unsupported structured status",
            )),
        }
    }
}

fn run_process(
    executable: &Path,
    bundle_extract: &Path,
    request_json: &[u8],
) -> Result<QuestConnectivityProviderOutput, QuestConnectivityAdapterError> {
    run_process_with_timeout(executable, bundle_extract, request_json, PROVIDER_TIMEOUT)
}

fn run_process_with_timeout(
    executable: &Path,
    bundle_extract: &Path,
    request_json: &[u8],
    timeout: Duration,
) -> Result<QuestConnectivityProviderOutput, QuestConnectivityAdapterError> {
    let mut command = Command::new(executable);
    command
        .args(["integration", "quest-connectivity", "--json"])
        .env_clear()
        .env("DOTNET_BUNDLE_EXTRACT_BASE_DIR", bundle_extract)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "SystemRoot",
        "WINDIR",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "LOCALAPPDATA",
        "APPDATA",
        "TEMP",
        "TMP",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn().map_err(|error| {
        QuestConnectivityAdapterError::new(
            "provider_start_failed",
            format!("cannot start Quest connectivity provider: {error}"),
        )
    })?;
    let write_result = child.stdin.take().map_or_else(
        || {
            Err(QuestConnectivityAdapterError::new(
                "provider_start_failed",
                "Quest connectivity provider stdin is unavailable",
            ))
        },
        |mut input| {
            input.write_all(request_json).map_err(|error| {
                QuestConnectivityAdapterError::new(
                    "provider_write_failed",
                    format!("cannot write provider request: {error}"),
                )
            })
        },
    );
    if let Err(error) = write_result {
        terminate_process_tree(&mut child);
        return Err(error);
    }
    let stdout = child.stdout.take().ok_or_else(|| {
        terminate_process_tree(&mut child);
        QuestConnectivityAdapterError::new(
            "provider_start_failed",
            "Quest connectivity provider stdout is unavailable",
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        terminate_process_tree(&mut child);
        QuestConnectivityAdapterError::new(
            "provider_start_failed",
            "Quest connectivity provider stderr is unavailable",
        )
    })?;
    let (sender, receiver) = mpsc::channel();
    drain_stream(0, stdout, sender.clone());
    drain_stream(1, stderr, sender);
    let mut stdout_result = None;
    let mut stderr_result = None;
    let started = Instant::now();
    loop {
        while let Ok((stream, result)) = receiver.try_recv() {
            let bytes = result.map_err(|error| {
                QuestConnectivityAdapterError::new(
                    "provider_read_failed",
                    format!("cannot read provider output: {error}"),
                )
            })?;
            if bytes.len() > MAX_PROVIDER_OUTPUT_BYTES {
                terminate_process_tree(&mut child);
                return Err(QuestConnectivityAdapterError::new(
                    "provider_output_oversized",
                    "Quest connectivity provider output exceeds 64 KiB",
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
                    match receiver.recv_timeout(Duration::from_millis(25)) {
                        Ok((stream, result)) => {
                            let bytes = result.map_err(|error| {
                                QuestConnectivityAdapterError::new(
                                    "provider_read_failed",
                                    format!("cannot read provider output: {error}"),
                                )
                            })?;
                            if bytes.len() > MAX_PROVIDER_OUTPUT_BYTES {
                                return Err(QuestConnectivityAdapterError::new(
                                    "provider_output_oversized",
                                    "Quest connectivity provider output exceeds 64 KiB",
                                ));
                            }
                            if stream == 0 {
                                stdout_result = Some(bytes);
                            } else {
                                stderr_result = Some(bytes);
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                let stdout = stdout_result.ok_or_else(|| {
                    QuestConnectivityAdapterError::new(
                        "provider_read_failed",
                        "provider stdout did not close after process exit",
                    )
                })?;
                let stderr = stderr_result.ok_or_else(|| {
                    QuestConnectivityAdapterError::new(
                        "provider_read_failed",
                        "provider stderr did not close after process exit",
                    )
                })?;
                if !stderr.is_empty() {
                    return Err(QuestConnectivityAdapterError::new(
                        "provider_stderr_rejected",
                        "provider wrote unexpected standard-error evidence",
                    ));
                }
                let exit_code = status.code().ok_or_else(|| {
                    QuestConnectivityAdapterError::new(
                        "provider_exit_invalid",
                        "provider exited without a reportable exit code",
                    )
                })?;
                if stdout.is_empty() {
                    return Err(QuestConnectivityAdapterError::new(
                        "provider_failed",
                        format!("provider exited with code {exit_code} and no structured response"),
                    ));
                }
                return Ok(QuestConnectivityProviderOutput { exit_code, stdout });
            }
            Ok(None) if started.elapsed() < timeout => {
                thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                terminate_process_tree(&mut child);
                return Err(QuestConnectivityAdapterError::new(
                    "provider_timeout",
                    "provider exceeded its configured execution deadline",
                ));
            }
            Err(error) => {
                terminate_process_tree(&mut child);
                return Err(QuestConnectivityAdapterError::new(
                    "provider_wait_failed",
                    format!("cannot wait for provider: {error}"),
                ));
            }
        }
    }
}

fn drain_stream<R: Read + Send + 'static>(
    stream: u8,
    reader: R,
    sender: mpsc::Sender<(u8, std::io::Result<Vec<u8>>)>,
) {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader
            .take((MAX_PROVIDER_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send((stream, result));
    });
}

fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let taskkill = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .map(|root| root.join("System32").join("taskkill.exe"))
            .unwrap_or_else(|| PathBuf::from("taskkill.exe"));
        let _ = Command::new(taskkill)
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn sha256_file(path: &Path) -> Result<String, QuestConnectivityAdapterError> {
    let metadata = fs::metadata(path).map_err(|error| {
        QuestConnectivityAdapterError::new(
            "provider_unavailable",
            format!("cannot inspect Quest connectivity provider: {error}"),
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 256 * 1024 * 1024 {
        return Err(QuestConnectivityAdapterError::new(
            "provider_file_invalid",
            "provider must be a nonempty bounded regular file",
        ));
    }
    let bytes = fs::read(path).map_err(|error| {
        QuestConnectivityAdapterError::new(
            "provider_unavailable",
            format!("cannot read Quest connectivity provider: {error}"),
        )
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn unique_stage_id(request_id: &str) -> Result<String, QuestConnectivityAdapterError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            QuestConnectivityAdapterError::new(
                "clock_invalid",
                format!("system clock precedes Unix epoch: {error}"),
            )
        })?
        .as_nanos();
    let sequence = PROVIDER_STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let digest = hex::encode(Sha256::digest(request_id.as_bytes()));
    Ok(format!(
        "connectivity-{}-{}-{sequence}-{nonce}",
        &digest[..24],
        std::process::id()
    ))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_portable_id(value: &str, maximum: usize) -> bool {
    (1..=maximum).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::fs;
    #[cfg(windows)]
    use std::path::Path;
    use std::path::PathBuf;
    #[cfg(windows)]
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    #[cfg(windows)]
    use std::thread;
    #[cfg(windows)]
    use std::time::Duration;

    use fleet_contracts::{
        QuestWifiAdbAction, QuestWifiAdbOwnerInvocation, QuestWifiAdbRouteMode,
        QuestWifiAdbWearerApproval,
    };

    #[cfg(windows)]
    use super::{
        PROVIDER_FILE_NAME, ProcessQuestConnectivityProviderTransport, run_process_with_timeout,
        sha256_file,
    };
    use super::{
        ProviderResponse, QuestConnectivityAdapterError, QuestConnectivityOwnerAdapter,
        QuestConnectivityProviderConfig, QuestConnectivityProviderOutput,
        QuestConnectivityProviderTransport,
    };

    #[derive(Clone, Default)]
    struct FakeTransport {
        requests: Arc<Mutex<Vec<serde_json::Value>>>,
        damage_device: bool,
        exit_code: i32,
        pending: bool,
    }

    impl QuestConnectivityProviderTransport for FakeTransport {
        fn invoke(
            &self,
            _: &QuestConnectivityProviderConfig,
            request_json: &[u8],
            _: &str,
        ) -> Result<QuestConnectivityProviderOutput, QuestConnectivityAdapterError> {
            let request: serde_json::Value =
                serde_json::from_slice(request_json).expect("provider request");
            self.requests
                .lock()
                .expect("request lock")
                .push(request.clone());
            let device = if self.damage_device {
                serde_json::json!("different-device")
            } else {
                request["device_id"].clone()
            };
            let stdout = serde_json::to_vec(&serde_json::json!({
                "schema": "questionable.file_manager.quest_wifi_adb_provider_response.v1",
                "status": if self.pending {"pending"} else {"verified"},
                "receipt": {
                    "schema": "questionable.file_manager.quest_wifi_adb_receipt.v1",
                    "request_id": request["request_id"],
                    "operation_id": request["operation_id"],
                    "preview_id": request["preview_id"],
                    "device_id": device,
                    "identity_revision": request["identity_revision"],
                    "action": request["action"],
                    "route_mode": "modern_tls",
                    "request_delivered": true,
                    "kiosk_setting_applied": !self.pending,
                    "request_after_boot_enabled": null,
                    "wearer_approval": "pending",
                    "listener_discovered": false,
                    "effect_applied": !self.pending,
                    "outcome": if self.pending {"wearer_approval_pending"} else {"request_applied_approval_pending"},
                    "evidence_sha256": "11".repeat(32),
                    "observed_at_ms": request["issued_at_ms"],
                },
                "error": null,
                "message": null
            }))
            .expect("response");
            Ok(QuestConnectivityProviderOutput {
                exit_code: self.exit_code,
                stdout,
            })
        }
    }

    fn invocation() -> QuestWifiAdbOwnerInvocation {
        QuestWifiAdbOwnerInvocation {
            schema: "rusty.fleet.quest_wifi_adb_owner_invocation.v1".to_owned(),
            request_id: "request.wifi.1".to_owned(),
            operation_id: "operation.wifi.1".to_owned(),
            preview_id: "preview.wifi.1".to_owned(),
            device_id: "device.quest.1".to_owned(),
            identity_revision: 7,
            action: QuestWifiAdbAction::RequestWirelessAdb,
            issued_at_ms: 1_000,
            expires_at_ms: 61_000,
        }
    }

    fn config() -> QuestConnectivityProviderConfig {
        QuestConnectivityProviderConfig {
            executable_path: PathBuf::from(
                "C:\\private\\questionable-file-manager-connectivity-provider.exe",
            ),
            executable_sha256: "22".repeat(32),
            private_stage_root: PathBuf::from("C:\\private\\stages"),
        }
    }

    #[cfg(windows)]
    struct TestDirectory(PathBuf);

    #[cfg(windows)]
    impl TestDirectory {
        fn create(label: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "fleet-connectivity-adapter-{label}-{}-{}",
                std::process::id(),
                super::PROVIDER_STAGE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir(&directory).expect("create isolated test directory");
            Self(directory)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    #[cfg(windows)]
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(windows)]
    fn build_process_fixture(directory: &Path) -> PathBuf {
        const SOURCE: &str = r###"
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

fn write_stdout(value: &str) {
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(value.as_bytes()).expect("stdout");
    stdout.flush().expect("flush");
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() == 3 && args[1] == "--child" {
        thread::sleep(Duration::from_secs(2));
        fs::write(&args[2], b"descendant survived").expect("marker");
        return;
    }
    if args.len() != 4
        || args[1] != "integration"
        || args[2] != "quest-connectivity"
        || args[3] != "--json"
    {
        std::process::exit(90);
    }

    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .expect("stdin");
    if input == "stage-env" {
        let bundle = env::var_os("DOTNET_BUNDLE_EXTRACT_BASE_DIR");
        let isolated = env::var_os("PATH").is_none()
            && bundle
                .as_ref()
                .map(std::path::Path::new)
                .is_some_and(|path| {
                    path.is_dir()
                        && path.file_name().and_then(|value| value.to_str())
                            == Some("bundle-extract")
                })
            && env::current_exe()
                .ok()
                .and_then(|path| path.file_name().map(|value| value.to_owned()))
                .and_then(|value| value.to_str().map(str::to_owned))
                .as_deref()
                == Some("questionable-file-manager-connectivity-provider.exe");
        if !isolated {
            std::process::exit(91);
        }
        write_stdout("stage-env-ok");
        return;
    }
    if let Some(marker) = input.strip_prefix("tree:") {
        Command::new(env::current_exe().expect("current exe"))
            .args(["--child", marker])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn descendant");
        thread::sleep(Duration::from_secs(60));
        return;
    }
    if input.contains("\"request_id\":\"request.exit-mismatch\"") {
        write_stdout(
            r#"{"schema":"questionable.file_manager.quest_wifi_adb_provider_response.v1","status":"verified","receipt":{"schema":"questionable.file_manager.quest_wifi_adb_receipt.v1","request_id":"request.exit-mismatch","operation_id":"operation.wifi.1","preview_id":"preview.wifi.1","device_id":"device.quest.1","identity_revision":7,"action":"request_wireless_adb","route_mode":"modern_tls","request_delivered":true,"kiosk_setting_applied":true,"request_after_boot_enabled":null,"wearer_approval":"pending","listener_discovered":false,"effect_applied":true,"outcome":"request_applied_approval_pending","evidence_sha256":"1111111111111111111111111111111111111111111111111111111111111111","observed_at_ms":1000},"error":null,"message":null}"#,
        );
        std::process::exit(1);
    }
    std::process::exit(92);
}
"###;
        let source = directory.join("process-fixture.rs");
        let executable = directory.join(PROVIDER_FILE_NAME);
        fs::write(&source, SOURCE).expect("write process fixture source");
        let output = Command::new("rustc")
            .args(["--edition=2024"])
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .output()
            .expect("compile process fixture");
        assert!(
            output.status.success(),
            "fixture compilation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        executable
    }

    #[cfg(windows)]
    fn real_config(
        executable: PathBuf,
        private_stage_root: PathBuf,
    ) -> QuestConnectivityProviderConfig {
        QuestConnectivityProviderConfig {
            executable_sha256: sha256_file(&executable).expect("fixture digest"),
            executable_path: executable,
            private_stage_root,
        }
    }

    #[test]
    fn adapter_binds_request_and_keeps_termux_unusable_without_proof() {
        let transport = FakeTransport::default();
        let adapter = QuestConnectivityOwnerAdapter::new(transport.clone());
        let receipt = adapter
            .invoke(&config(), &invocation())
            .expect("valid provider receipt");
        assert_eq!(receipt.route_mode, QuestWifiAdbRouteMode::ModernTls);
        assert_eq!(receipt.wearer_approval, QuestWifiAdbWearerApproval::Pending);
        assert!(receipt.effect_applied);
        let requests = transport.requests.lock().expect("request lock");
        assert_eq!(requests[0]["device_id"], "device.quest.1");
        assert!(requests[0].get("serial").is_none());
        assert!(requests[0].get("endpoint").is_none());
        assert!(requests[0].get("pairing_code").is_none());
    }

    #[test]
    fn adapter_rejects_receipt_for_another_device() {
        let transport = FakeTransport {
            damage_device: true,
            ..FakeTransport::default()
        };
        let error = QuestConnectivityOwnerAdapter::new(transport)
            .invoke(&config(), &invocation())
            .expect_err("wrong device must fail");
        assert_eq!(error.code, "provider_receipt_binding_mismatch");
    }

    #[test]
    fn adapter_rejects_valid_verified_stdout_from_nonzero_exit() {
        let transport = FakeTransport {
            exit_code: 1,
            ..FakeTransport::default()
        };
        let error = QuestConnectivityOwnerAdapter::new(transport)
            .invoke(&config(), &invocation())
            .expect_err("verified stdout with failed exit must not be admitted");
        assert_eq!(error.code, "provider_exit_status_mismatch");
    }

    #[test]
    fn adapter_accepts_pending_receipt_only_with_exit_code_three() {
        let receipt = QuestConnectivityOwnerAdapter::new(FakeTransport {
            exit_code: 3,
            pending: true,
            ..FakeTransport::default()
        })
        .invoke(&config(), &invocation())
        .expect("pending provider receipt with exact exit status");
        assert!(!receipt.effect_applied);
        assert_eq!(receipt.outcome, "wearer_approval_pending");

        let error = QuestConnectivityOwnerAdapter::new(FakeTransport {
            exit_code: 0,
            pending: true,
            ..FakeTransport::default()
        })
        .invoke(&config(), &invocation())
        .expect_err("pending stdout with verified exit must not be admitted");
        assert_eq!(error.code, "provider_exit_status_mismatch");
    }

    #[test]
    fn structured_status_has_one_exact_provider_exit_code() {
        for (status, exit_code) in [
            ("verified", 0),
            ("failed", 1),
            ("rejected", 2),
            ("pending", 3),
            ("cancelled", 4),
        ] {
            let response = ProviderResponse {
                schema: super::PROVIDER_RESPONSE_SCHEMA.to_owned(),
                status: status.to_owned(),
                receipt: None,
                error: None,
                message: None,
            };
            assert_eq!(
                response.expected_exit_code().expect("supported status"),
                exit_code
            );
        }

        let unknown = ProviderResponse {
            schema: super::PROVIDER_RESPONSE_SCHEMA.to_owned(),
            status: "unexpected".to_owned(),
            receipt: None,
            error: None,
            message: None,
        };
        assert_eq!(
            unknown
                .expected_exit_code()
                .expect_err("unknown status")
                .code,
            "provider_status_invalid"
        );
    }

    #[cfg(windows)]
    #[test]
    fn process_transport_enforces_hash_stage_and_environment_isolation() {
        let test = TestDirectory::create("stage");
        let executable = build_process_fixture(test.path());
        let stage_root = test.path().join("stages");
        let config = real_config(executable, stage_root.clone());
        let output = ProcessQuestConnectivityProviderTransport
            .invoke(&config, b"stage-env", "stage.env.1")
            .expect("isolated staged process");
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout, b"stage-env-ok");
        assert!(
            fs::read_dir(&stage_root)
                .expect("stage root")
                .next()
                .is_none(),
            "per-launch stage must be removed"
        );

        let damaged = QuestConnectivityProviderConfig {
            executable_sha256: "00".repeat(32),
            ..config
        };
        let error = ProcessQuestConnectivityProviderTransport
            .invoke(&damaged, b"stage-env", "stage.env.2")
            .expect_err("source artifact hash mismatch");
        assert_eq!(error.code, "provider_digest_mismatch");
    }

    #[cfg(windows)]
    #[test]
    fn real_process_verified_stdout_with_exit_one_is_rejected() {
        let test = TestDirectory::create("exit");
        let executable = build_process_fixture(test.path());
        let config = real_config(executable, test.path().join("stages"));
        let mut invocation = invocation();
        invocation.request_id = "request.exit-mismatch".to_owned();
        let error = QuestConnectivityOwnerAdapter::default()
            .invoke(&config, &invocation)
            .expect_err("valid verified stdout cannot override exit one");
        assert_eq!(error.code, "provider_exit_status_mismatch");
    }

    #[cfg(windows)]
    #[test]
    fn timeout_terminates_provider_descendant_tree() {
        let test = TestDirectory::create("timeout");
        let executable = build_process_fixture(test.path());
        let bundle_extract = test.path().join("bundle-extract");
        fs::create_dir(&bundle_extract).expect("bundle extraction directory");
        let marker = test.path().join("descendant-marker");
        let request = format!("tree:{}", marker.display());
        let error = run_process_with_timeout(
            &executable,
            &bundle_extract,
            request.as_bytes(),
            Duration::from_millis(250),
        )
        .expect_err("fixture must exceed deadline");
        assert_eq!(error.code, "provider_timeout");
        thread::sleep(Duration::from_millis(2_500));
        assert!(
            !marker.exists(),
            "provider descendant survived timeout tree termination"
        );
    }
}
