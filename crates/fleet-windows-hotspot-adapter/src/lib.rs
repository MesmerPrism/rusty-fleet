// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fleet_contracts::{
    ValidateContract, WINDOWS_HOTSPOT_PROVIDER_FILE, WindowsHotspotProviderReceipt,
    WindowsHotspotProviderRequest, WindowsHotspotResult,
};
use sha2::{Digest, Sha256};

const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(30);
static STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowsHotspotProviderConfig {
    pub executable_path: PathBuf,
    pub executable_sha256: String,
    pub private_stage_root: PathBuf,
}

impl WindowsHotspotProviderConfig {
    pub fn validate(&self) -> Result<(), WindowsHotspotAdapterError> {
        if !self.executable_path.is_absolute()
            || !self.private_stage_root.is_absolute()
            || self
                .executable_path
                .file_name()
                .and_then(|value| value.to_str())
                != Some(WINDOWS_HOTSPOT_PROVIDER_FILE)
            || !is_lower_sha256(&self.executable_sha256)
        {
            return Err(WindowsHotspotAdapterError::new(
                "provider_config_invalid",
                "provider requires its exact absolute filename, lowercase SHA-256 pin, and absolute private stage root",
            ));
        }
        Ok(())
    }

    pub fn verify_artifact(&self) -> Result<(), WindowsHotspotAdapterError> {
        self.validate()?;
        if sha256_file(&self.executable_path)? != self.executable_sha256 {
            return Err(WindowsHotspotAdapterError::new(
                "provider_digest_mismatch",
                "Windows hotspot provider does not match its configured SHA-256",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowsHotspotAdapterError {
    pub code: String,
    pub message: String,
}

impl WindowsHotspotAdapterError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for WindowsHotspotAdapterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for WindowsHotspotAdapterError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowsHotspotProviderOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
}

pub trait WindowsHotspotProviderTransport: Send + Sync {
    fn invoke(
        &self,
        config: &WindowsHotspotProviderConfig,
        request_json: &[u8],
        stage_id: &str,
    ) -> Result<WindowsHotspotProviderOutput, WindowsHotspotAdapterError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessWindowsHotspotProviderTransport;

impl WindowsHotspotProviderTransport for ProcessWindowsHotspotProviderTransport {
    fn invoke(
        &self,
        config: &WindowsHotspotProviderConfig,
        request_json: &[u8],
        stage_id: &str,
    ) -> Result<WindowsHotspotProviderOutput, WindowsHotspotAdapterError> {
        config.verify_artifact()?;
        if !portable_id(stage_id) {
            return Err(WindowsHotspotAdapterError::new(
                "provider_stage_invalid",
                "provider stage identity is invalid",
            ));
        }
        fs::create_dir_all(&config.private_stage_root).map_err(|error| {
            WindowsHotspotAdapterError::new(
                "provider_stage_failed",
                format!("cannot create private stage root: {error}"),
            )
        })?;
        if fs::symlink_metadata(&config.private_stage_root)
            .map(|metadata| metadata.file_type().is_symlink() || !metadata.is_dir())
            .unwrap_or(true)
        {
            return Err(WindowsHotspotAdapterError::new(
                "provider_stage_invalid",
                "private stage root must be a real directory",
            ));
        }
        let stage = config.private_stage_root.join(stage_id);
        if stage.exists() {
            return Err(WindowsHotspotAdapterError::new(
                "provider_stage_conflict",
                "provider launch stage already exists",
            ));
        }
        fs::create_dir(&stage).map_err(|error| {
            WindowsHotspotAdapterError::new(
                "provider_stage_failed",
                format!("cannot create provider stage: {error}"),
            )
        })?;
        let executable = stage.join(WINDOWS_HOTSPOT_PROVIDER_FILE);
        let bundle_extract = stage.join("bundle-extract");
        let result = (|| {
            fs::copy(&config.executable_path, &executable).map_err(|error| {
                WindowsHotspotAdapterError::new(
                    "provider_stage_failed",
                    format!("cannot stage provider artifact: {error}"),
                )
            })?;
            fs::create_dir(&bundle_extract).map_err(|error| {
                WindowsHotspotAdapterError::new(
                    "provider_stage_failed",
                    format!("cannot create private bundle extraction directory: {error}"),
                )
            })?;
            if sha256_file(&executable)? != config.executable_sha256 {
                return Err(WindowsHotspotAdapterError::new(
                    "provider_stage_digest_mismatch",
                    "staged provider changed after copy",
                ));
            }
            run_process(&executable, &bundle_extract, request_json, PROVIDER_TIMEOUT)
        })();
        let cleanup = fs::remove_dir_all(&stage);
        match (result, cleanup) {
            (Ok(output), Ok(())) => Ok(output),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(_)) => Err(WindowsHotspotAdapterError::new(
                "provider_cleanup_failed",
                "private provider stage could not be removed",
            )),
            (Err(error), Err(_)) => Err(WindowsHotspotAdapterError::new(
                "provider_cleanup_failed",
                format!("provider stage cleanup failed after {}", error.code),
            )),
        }
    }
}

#[derive(Clone, Debug)]
pub struct WindowsHotspotOwnerAdapter<T = ProcessWindowsHotspotProviderTransport> {
    transport: T,
}

impl Default for WindowsHotspotOwnerAdapter<ProcessWindowsHotspotProviderTransport> {
    fn default() -> Self {
        Self {
            transport: ProcessWindowsHotspotProviderTransport,
        }
    }
}

impl<T: WindowsHotspotProviderTransport> WindowsHotspotOwnerAdapter<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    pub fn invoke(
        &self,
        config: &WindowsHotspotProviderConfig,
        request: &WindowsHotspotProviderRequest,
    ) -> Result<WindowsHotspotProviderReceipt, WindowsHotspotAdapterError> {
        request.validate().map_err(|_| {
            WindowsHotspotAdapterError::new(
                "invocation_invalid",
                "Fleet rejected an invalid provider invocation",
            )
        })?;
        let request_json = serde_json::to_vec(request).map_err(|error| {
            WindowsHotspotAdapterError::new(
                "provider_request_invalid",
                format!("cannot serialize provider request: {error}"),
            )
        })?;
        let output = self.transport.invoke(
            config,
            &request_json,
            &unique_stage_id(&request.request_id)?,
        )?;
        if output.stdout.len() > MAX_OUTPUT_BYTES {
            return Err(WindowsHotspotAdapterError::new(
                "provider_output_oversized",
                "provider stdout exceeds 64 KiB",
            ));
        }
        let receipt: WindowsHotspotProviderReceipt = serde_json::from_slice(&output.stdout)
            .map_err(|error| {
                WindowsHotspotAdapterError::new(
                    "provider_receipt_invalid",
                    format!("provider stdout is not one strict receipt: {error}"),
                )
            })?;
        let expected_exit = match receipt.outcome {
            WindowsHotspotResult::Verified => 0,
            WindowsHotspotResult::Failed => 1,
            WindowsHotspotResult::Rejected => 2,
            WindowsHotspotResult::Unavailable => 3,
        };
        if output.exit_code != expected_exit {
            return Err(WindowsHotspotAdapterError::new(
                "provider_exit_status_mismatch",
                "provider exit code contradicts its structured result",
            ));
        }
        receipt.validate().map_err(|_| {
            WindowsHotspotAdapterError::new(
                "provider_receipt_invalid",
                "provider receipt violates the Fleet/Hostess contract",
            )
        })?;
        if receipt.request_id != request.request_id
            || receipt.operation_id != request.operation_id
            || receipt.action != request.action
        {
            return Err(WindowsHotspotAdapterError::new(
                "provider_receipt_binding_mismatch",
                "provider receipt does not bind the exact request",
            ));
        }
        Ok(receipt)
    }
}

fn run_process(
    executable: &Path,
    bundle_extract: &Path,
    request_json: &[u8],
    timeout: Duration,
) -> Result<WindowsHotspotProviderOutput, WindowsHotspotAdapterError> {
    let mut command = Command::new(executable);
    command
        .args(["integration", "windows-hotspot", "--json"])
        .current_dir(bundle_extract)
        .env_clear()
        .env("DOTNET_BUNDLE_EXTRACT_BASE_DIR", bundle_extract)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "SystemRoot",
        "SystemDrive",
        "WINDIR",
        "ProgramData",
        "TEMP",
        "TMP",
        "LOCALAPPDATA",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn().map_err(|error| {
        WindowsHotspotAdapterError::new(
            "provider_start_failed",
            format!("cannot start provider: {error}"),
        )
    })?;
    if child
        .stdin
        .take()
        .ok_or_else(|| {
            WindowsHotspotAdapterError::new("provider_start_failed", "provider stdin unavailable")
        })?
        .write_all(request_json)
        .is_err()
    {
        terminate_process_tree(&mut child);
        return Err(WindowsHotspotAdapterError::new(
            "provider_write_failed",
            "cannot write provider request",
        ));
    }
    let stdout = child.stdout.take().ok_or_else(|| {
        terminate_process_tree(&mut child);
        WindowsHotspotAdapterError::new("provider_start_failed", "provider stdout unavailable")
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        terminate_process_tree(&mut child);
        WindowsHotspotAdapterError::new("provider_start_failed", "provider stderr unavailable")
    })?;
    let (sender, receiver) = mpsc::channel();
    drain_stream(0, stdout, sender.clone());
    drain_stream(1, stderr, sender);
    let started = Instant::now();
    let deadline = started + timeout;
    let mut stdout_result = None;
    let mut stderr_result = None;
    loop {
        while let Ok((stream, result)) = receiver.try_recv() {
            let bytes = match result {
                Ok(bytes) => bytes,
                Err(error) => {
                    if Instant::now() >= deadline {
                        return Err(terminate_timed_out_provider(&mut child));
                    }
                    terminate_process_tree(&mut child);
                    return Err(WindowsHotspotAdapterError::new(
                        "provider_read_failed",
                        format!("cannot read provider output: {error}"),
                    ));
                }
            };
            if bytes.len() > MAX_OUTPUT_BYTES {
                terminate_process_tree(&mut child);
                return Err(WindowsHotspotAdapterError::new(
                    "provider_output_oversized",
                    "provider output exceeds 64 KiB",
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
                while stdout_result.is_none() || stderr_result.is_none() {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(terminate_timed_out_provider(&mut child));
                    }
                    match receiver.recv_timeout(remaining.min(Duration::from_millis(25))) {
                        Ok((stream, result)) => {
                            let bytes = match result {
                                Ok(bytes) => bytes,
                                Err(error) => {
                                    if Instant::now() >= deadline {
                                        return Err(terminate_timed_out_provider(&mut child));
                                    }
                                    terminate_process_tree(&mut child);
                                    return Err(WindowsHotspotAdapterError::new(
                                        "provider_read_failed",
                                        format!("cannot read provider output: {error}"),
                                    ));
                                }
                            };
                            if stream == 0 {
                                stdout_result = Some(bytes);
                            } else {
                                stderr_result = Some(bytes);
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            terminate_process_tree(&mut child);
                            return Err(WindowsHotspotAdapterError::new(
                                "provider_read_failed",
                                "provider output readers disconnected before closing both streams",
                            ));
                        }
                    }
                }
                let (Some(stdout), Some(stderr)) = (stdout_result, stderr_result) else {
                    terminate_process_tree(&mut child);
                    return Err(WindowsHotspotAdapterError::new(
                        "provider_read_failed",
                        "provider output did not close both streams",
                    ));
                };
                if !stderr.is_empty() {
                    return Err(WindowsHotspotAdapterError::new(
                        "provider_stderr_rejected",
                        "provider emitted unstructured stderr",
                    ));
                }
                return Ok(WindowsHotspotProviderOutput {
                    exit_code: status.code().ok_or_else(|| {
                        WindowsHotspotAdapterError::new(
                            "provider_exit_invalid",
                            "provider has no reportable exit code",
                        )
                    })?,
                    stdout,
                });
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => return Err(terminate_timed_out_provider(&mut child)),
            Err(error) => {
                terminate_process_tree(&mut child);
                return Err(WindowsHotspotAdapterError::new(
                    "provider_wait_failed",
                    format!("cannot observe provider exit: {error}"),
                ));
            }
        }
    }
}

fn terminate_timed_out_provider(child: &mut Child) -> WindowsHotspotAdapterError {
    terminate_process_tree(child);
    WindowsHotspotAdapterError::new(
        "provider_timeout",
        "provider exceeded its bounded deadline and its process tree was terminated",
    )
}

fn drain_stream<R: Read + Send + 'static>(
    stream: u8,
    mut reader: R,
    sender: mpsc::Sender<(u8, std::io::Result<Vec<u8>>)>,
) {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader.read_to_end(&mut bytes).map(|_| bytes);
        let _ = sender.send((stream, result));
    });
}

fn terminate_process_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        let powershell = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .map(|root| {
                root.join("System32")
                    .join("WindowsPowerShell")
                    .join("v1.0")
                    .join("powershell.exe")
            })
            .unwrap_or_else(|| PathBuf::from("powershell.exe"));
        let script = format!(
            "$root={};$all=@(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId);$pending=@($root);$ordered=@();while($pending.Count -gt 0){{$current=$pending[0];$pending=@($pending | Select-Object -Skip 1);$ordered+=@($current);$pending+=@($all | Where-Object ParentProcessId -eq $current | ForEach-Object ProcessId)}};[array]::Reverse($ordered);foreach($id in $ordered){{Stop-Process -Id $id -Force -ErrorAction SilentlyContinue}}",
            child.id()
        );
        let _ = Command::new(powershell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &script,
            ])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let taskkill = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .map(|root| root.join("System32").join("taskkill.exe"))
            .unwrap_or_else(|| PathBuf::from("taskkill.exe"));
        let _ = Command::new(taskkill)
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn sha256_file(path: &Path) -> Result<String, WindowsHotspotAdapterError> {
    let metadata = fs::metadata(path).map_err(|error| {
        WindowsHotspotAdapterError::new(
            "provider_unavailable",
            format!("cannot inspect provider: {error}"),
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 256 * 1024 * 1024 {
        return Err(WindowsHotspotAdapterError::new(
            "provider_file_invalid",
            "provider must be a nonempty bounded regular file",
        ));
    }
    let bytes = fs::read(path).map_err(|error| {
        WindowsHotspotAdapterError::new(
            "provider_unavailable",
            format!("cannot read provider: {error}"),
        )
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn unique_stage_id(request_id: &str) -> Result<String, WindowsHotspotAdapterError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| WindowsHotspotAdapterError::new("clock_invalid", "system clock invalid"))?
        .as_nanos();
    let sequence = STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let digest = hex::encode(Sha256::digest(request_id.as_bytes()));
    Ok(format!(
        "hotspot-{}-{}-{sequence}-{nonce}",
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

fn portable_id(value: &str) -> bool {
    (1..=160).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fleet_contracts::{
        WINDOWS_HOTSPOT_PROVIDER_RECEIPT_SCHEMA, WINDOWS_HOTSPOT_PROVIDER_REQUEST_SCHEMA,
        WindowsHotspotAction,
    };

    #[derive(Clone)]
    struct FakeTransport {
        output: WindowsHotspotProviderOutput,
    }

    impl WindowsHotspotProviderTransport for FakeTransport {
        fn invoke(
            &self,
            _config: &WindowsHotspotProviderConfig,
            _request_json: &[u8],
            _stage_id: &str,
        ) -> Result<WindowsHotspotProviderOutput, WindowsHotspotAdapterError> {
            Ok(self.output.clone())
        }
    }

    fn request() -> WindowsHotspotProviderRequest {
        WindowsHotspotProviderRequest {
            schema: WINDOWS_HOTSPOT_PROVIDER_REQUEST_SCHEMA.to_owned(),
            request_id: "request.1".to_owned(),
            operation_id: "operation.1".to_owned(),
            action: WindowsHotspotAction::Start,
            expires_at_utc: "2026-07-27T12:00:00.0000000Z".to_owned(),
            timeout_ms: 30_000,
            ownership_generation: None,
        }
    }

    fn receipt() -> WindowsHotspotProviderReceipt {
        WindowsHotspotProviderReceipt {
            schema: WINDOWS_HOTSPOT_PROVIDER_RECEIPT_SCHEMA.to_owned(),
            request_id: "request.1".to_owned(),
            operation_id: "operation.1".to_owned(),
            action: WindowsHotspotAction::Start,
            outcome: WindowsHotspotResult::Verified,
            reason: "start.readback_verified".to_owned(),
            observed_at_utc: "2026-07-27T11:59:59.0000000Z".to_owned(),
            capability_available: true,
            capability: "Enabled".to_owned(),
            operational_state: "On".to_owned(),
            client_count: 0,
            max_client_count: 8,
            band: "FiveGigahertz".to_owned(),
            source_connectivity: "Internet".to_owned(),
            ownership_generation: Some("generation.1".to_owned()),
        }
    }

    fn config() -> WindowsHotspotProviderConfig {
        WindowsHotspotProviderConfig {
            executable_path: PathBuf::from("C:\\private\\rusty-hostess-hotspot-provider.exe"),
            executable_sha256: "a".repeat(64),
            private_stage_root: PathBuf::from("C:\\private\\stages"),
        }
    }

    #[test]
    fn accepts_exact_bound_receipt_and_exit_mapping() {
        let output = WindowsHotspotProviderOutput {
            exit_code: 0,
            stdout: serde_json::to_vec(&receipt()).expect("receipt"),
        };
        let adapter = WindowsHotspotOwnerAdapter::new(FakeTransport { output });
        assert_eq!(adapter.invoke(&config(), &request()), Ok(receipt()));
    }

    #[test]
    fn rejects_exit_mismatch_unknown_private_fields_and_binding_changes() {
        let output = WindowsHotspotProviderOutput {
            exit_code: 2,
            stdout: serde_json::to_vec(&receipt()).expect("receipt"),
        };
        let adapter = WindowsHotspotOwnerAdapter::new(FakeTransport { output });
        assert_eq!(
            adapter
                .invoke(&config(), &request())
                .expect_err("mismatch")
                .code,
            "provider_exit_status_mismatch"
        );
        let mut value = serde_json::to_value(receipt()).expect("value");
        value["ssid"] = serde_json::json!("private");
        let adapter = WindowsHotspotOwnerAdapter::new(FakeTransport {
            output: WindowsHotspotProviderOutput {
                exit_code: 0,
                stdout: serde_json::to_vec(&value).expect("value"),
            },
        });
        assert_eq!(
            adapter
                .invoke(&config(), &request())
                .expect_err("private field")
                .code,
            "provider_receipt_invalid"
        );
        let mut changed = receipt();
        changed.operation_id = "operation.other".to_owned();
        let adapter = WindowsHotspotOwnerAdapter::new(FakeTransport {
            output: WindowsHotspotProviderOutput {
                exit_code: 0,
                stdout: serde_json::to_vec(&changed).expect("changed"),
            },
        });
        assert_eq!(
            adapter
                .invoke(&config(), &request())
                .expect_err("binding")
                .code,
            "provider_receipt_binding_mismatch"
        );
    }

    #[test]
    fn rejects_missing_provider_bad_hash_and_filename() {
        let missing = WindowsHotspotProviderConfig {
            executable_path: PathBuf::from("C:\\missing\\rusty-hostess-hotspot-provider.exe"),
            ..config()
        };
        assert_eq!(
            missing.verify_artifact().expect_err("missing").code,
            "provider_unavailable"
        );
        let wrong = WindowsHotspotProviderConfig {
            executable_path: PathBuf::from("C:\\private\\other.exe"),
            ..config()
        };
        assert_eq!(
            wrong.validate().expect_err("filename").code,
            "provider_config_invalid"
        );
        let uppercase = WindowsHotspotProviderConfig {
            executable_sha256: "A".repeat(64),
            ..config()
        };
        assert_eq!(
            uppercase.validate().expect_err("hash").code,
            "provider_config_invalid"
        );
    }

    #[cfg(windows)]
    #[test]
    fn timeout_terminates_process_tree_before_child_can_write() {
        let root = std::env::temp_dir().join(format!(
            "rusty-fleet-hotspot-timeout-{}",
            std::process::id()
        ));
        let bundle = root.join("bundle");
        fs::create_dir_all(&bundle).expect("bundle");
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("slow-provider.cmd");
        let error =
            run_process(&script, &bundle, b"{}", Duration::from_millis(150)).expect_err("timeout");
        assert_eq!(error.code, "provider_timeout");
        thread::sleep(Duration::from_secs(6));
        assert!(!bundle.join("child.txt").exists());
        let _ = fs::remove_dir_all(root);
    }
}
