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
    QUEST_AWAKE_PROVIDER_CONTRACT, QUEST_AWAKE_RECEIPT_SCHEMA, QuestAwakeAction,
    QuestAwakeOwnerInvocation, QuestAwakeOwnerReceipt, QuestAwakePowerReadback,
    QuestAwakeWatchdogReadback, ValidateContract,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PROVIDER_FILE_NAME: &str = "questionable-file-manager-awake-provider.exe";
const MAX_PROVIDER_OUTPUT_BYTES: usize = 64 * 1024;
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(30);
static PROVIDER_STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestAwakeProviderConfig {
    pub executable_path: PathBuf,
    pub executable_sha256: String,
    pub adb_executable_path: PathBuf,
    pub adb_executable_sha256: String,
    pub adb_support_artifacts: Vec<QuestAwakePinnedArtifact>,
    pub private_stage_root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestAwakePinnedArtifact {
    pub source_path: PathBuf,
    pub sha256: String,
}

impl QuestAwakeProviderConfig {
    pub fn validate(&self) -> Result<(), QuestAwakeAdapterError> {
        if !self.executable_path.is_absolute()
            || !self.adb_executable_path.is_absolute()
            || !self.private_stage_root.is_absolute()
            || self
                .executable_path
                .file_name()
                .and_then(|value| value.to_str())
                != Some(PROVIDER_FILE_NAME)
            || self
                .adb_executable_path
                .file_name()
                .and_then(|value| value.to_str())
                .is_none_or(|value| !value.eq_ignore_ascii_case("adb.exe"))
            || !is_lower_sha256(&self.executable_sha256)
            || !is_lower_sha256(&self.adb_executable_sha256)
        {
            return Err(QuestAwakeAdapterError::new(
                "provider_config_invalid",
                "Quest awake provider requires absolute exact-named provider and ADB executables, lowercase SHA-256 pins, and an absolute private stage root",
            ));
        }
        let support_names = self
            .adb_support_artifacts
            .iter()
            .filter_map(|artifact| {
                artifact
                    .source_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(str::to_ascii_lowercase)
            })
            .collect::<std::collections::BTreeSet<_>>();
        if self.adb_support_artifacts.len() != 2
            || support_names
                != std::collections::BTreeSet::from([
                    "adbwinapi.dll".to_owned(),
                    "adbwinusbapi.dll".to_owned(),
                ])
            || self.adb_support_artifacts.iter().any(|artifact| {
                !artifact.source_path.is_absolute() || !is_lower_sha256(&artifact.sha256)
            })
        {
            return Err(QuestAwakeAdapterError::new(
                "provider_config_invalid",
                "Quest awake provider requires exact SHA-pinned AdbWinApi.dll and AdbWinUsbApi.dll support artifacts",
            ));
        }
        Ok(())
    }

    pub fn verify_artifacts(&self) -> Result<(), QuestAwakeAdapterError> {
        self.validate()?;
        if sha256_file(&self.executable_path)? != self.executable_sha256 {
            return Err(QuestAwakeAdapterError::new(
                "provider_digest_mismatch",
                "Quest awake provider executable does not match its configured SHA-256",
            ));
        }
        if sha256_file(&self.adb_executable_path)? != self.adb_executable_sha256 {
            return Err(QuestAwakeAdapterError::new(
                "adb_digest_mismatch",
                "Quest awake ADB executable does not match its configured SHA-256",
            ));
        }
        for artifact in &self.adb_support_artifacts {
            if sha256_file(&artifact.source_path)? != artifact.sha256 {
                return Err(QuestAwakeAdapterError::new(
                    "adb_support_digest_mismatch",
                    "Quest awake ADB support artifact does not match its configured SHA-256",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestAwakeAdapterError {
    pub code: String,
    pub message: String,
}

impl QuestAwakeAdapterError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for QuestAwakeAdapterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for QuestAwakeAdapterError {}

pub trait QuestAwakeProviderTransport: Send + Sync {
    fn invoke(
        &self,
        config: &QuestAwakeProviderConfig,
        request_json: &[u8],
        stage_id: &str,
    ) -> Result<Vec<u8>, QuestAwakeAdapterError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessQuestAwakeProviderTransport;

impl QuestAwakeProviderTransport for ProcessQuestAwakeProviderTransport {
    fn invoke(
        &self,
        config: &QuestAwakeProviderConfig,
        request_json: &[u8],
        stage_id: &str,
    ) -> Result<Vec<u8>, QuestAwakeAdapterError> {
        config.verify_artifacts()?;
        if !is_portable_id(stage_id, 128) {
            return Err(QuestAwakeAdapterError::new(
                "provider_stage_invalid",
                "Quest awake provider stage identity is invalid",
            ));
        }
        fs::create_dir_all(&config.private_stage_root).map_err(|error| {
            QuestAwakeAdapterError::new(
                "provider_stage_failed",
                format!("cannot create private provider stage root: {error}"),
            )
        })?;
        if fs::symlink_metadata(&config.private_stage_root)
            .map(|metadata| metadata.file_type().is_symlink() || !metadata.is_dir())
            .unwrap_or(true)
        {
            return Err(QuestAwakeAdapterError::new(
                "provider_stage_invalid",
                "private provider stage root must be a real directory, not a symbolic link",
            ));
        }
        let stage = config.private_stage_root.join(stage_id);
        if stage.exists() {
            return Err(QuestAwakeAdapterError::new(
                "provider_stage_conflict",
                "Quest awake provider stage already exists",
            ));
        }
        fs::create_dir(&stage).map_err(|error| {
            QuestAwakeAdapterError::new(
                "provider_stage_failed",
                format!("cannot create exact provider stage: {error}"),
            )
        })?;
        let staged_executable = stage.join(PROVIDER_FILE_NAME);
        let staged_adb = stage.join("adb.exe");
        let bundle_extract = stage.join("bundle-extract");
        let result = (|| {
            fs::copy(&config.executable_path, &staged_executable).map_err(|error| {
                QuestAwakeAdapterError::new(
                    "provider_stage_failed",
                    format!("cannot stage Quest awake provider: {error}"),
                )
            })?;
            fs::create_dir(&bundle_extract).map_err(|error| {
                QuestAwakeAdapterError::new(
                    "provider_stage_failed",
                    format!("cannot create provider bundle extraction stage: {error}"),
                )
            })?;
            if sha256_file(&staged_executable)? != config.executable_sha256 {
                return Err(QuestAwakeAdapterError::new(
                    "provider_stage_digest_mismatch",
                    "staged Quest awake provider changed after copy",
                ));
            }
            fs::copy(&config.adb_executable_path, &staged_adb).map_err(|error| {
                QuestAwakeAdapterError::new(
                    "provider_stage_failed",
                    format!("cannot stage pinned Quest awake ADB: {error}"),
                )
            })?;
            if sha256_file(&staged_adb)? != config.adb_executable_sha256 {
                return Err(QuestAwakeAdapterError::new(
                    "adb_stage_digest_mismatch",
                    "staged Quest awake ADB changed during copy",
                ));
            }
            for artifact in &config.adb_support_artifacts {
                let file_name = artifact
                    .source_path
                    .file_name()
                    .expect("validated support artifact has a filename");
                let staged_support = stage.join(file_name);
                fs::copy(&artifact.source_path, &staged_support).map_err(|error| {
                    QuestAwakeAdapterError::new(
                        "provider_stage_failed",
                        format!("cannot stage pinned Quest awake ADB support file: {error}"),
                    )
                })?;
                if sha256_file(&staged_support)? != artifact.sha256 {
                    return Err(QuestAwakeAdapterError::new(
                        "adb_support_stage_digest_mismatch",
                        "staged Quest awake ADB support artifact changed during copy",
                    ));
                }
            }
            run_process(
                &staged_executable,
                &staged_adb,
                &bundle_extract,
                request_json,
            )
        })();
        let cleanup = fs::remove_dir_all(&stage);
        finish_provider_stage(result, cleanup)
    }
}

#[derive(Clone, Debug)]
pub struct QuestAwakeOwnerAdapter<T = ProcessQuestAwakeProviderTransport> {
    transport: T,
}

impl Default for QuestAwakeOwnerAdapter<ProcessQuestAwakeProviderTransport> {
    fn default() -> Self {
        Self {
            transport: ProcessQuestAwakeProviderTransport,
        }
    }
}

impl<T: QuestAwakeProviderTransport> QuestAwakeOwnerAdapter<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    pub fn invoke(
        &self,
        config: &QuestAwakeProviderConfig,
        invocation: &QuestAwakeOwnerInvocation,
        serial: &str,
        windows_watchdog_effective: bool,
    ) -> Result<QuestAwakeOwnerReceipt, QuestAwakeAdapterError> {
        invocation.validate().map_err(|failures| {
            QuestAwakeAdapterError::new(
                "invocation_invalid",
                failures
                    .iter()
                    .map(|failure| format!("{}:{}", failure.path, failure.code))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        if serial.is_empty()
            || serial.len() > 256
            || serial
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        {
            return Err(QuestAwakeAdapterError::new(
                "serial_invalid",
                "configured Quest awake serial is invalid",
            ));
        }
        let (provider_issued_at_ms, provider_expires_at_ms) =
            if invocation.action == QuestAwakeAction::StartWindowsWatchdog {
                let current = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|error| {
                        QuestAwakeAdapterError::new(
                            "clock_invalid",
                            format!("system clock precedes Unix epoch: {error}"),
                        )
                    })?
                    .as_millis();
                let current = i64::try_from(current).map_err(|_| {
                    QuestAwakeAdapterError::new("clock_invalid", "system time is not representable")
                })?;
                (current, current + 60_000)
            } else {
                (invocation.issued_at_ms, invocation.expires_at_ms)
            };
        let provider_request = ProviderRequest {
            contract_version: QUEST_AWAKE_PROVIDER_CONTRACT,
            request_id: &invocation.request_id,
            operation_id: &invocation.operation_id,
            preview_id: &invocation.preview_id,
            device_id: &invocation.device_id,
            identity_revision: invocation.identity_revision,
            action: invocation.action.provider_action(),
            duration_milliseconds: invocation.duration_ms,
            watchdog_interval_milliseconds: invocation.watchdog_interval_ms,
            watchdog_generation: &invocation.watchdog_generation,
            issued_at_unix_milliseconds: provider_issued_at_ms,
            expires_at_unix_milliseconds: provider_expires_at_ms,
            serial,
        };
        let request_json = serde_json::to_vec(&provider_request).map_err(|error| {
            QuestAwakeAdapterError::new(
                "provider_request_invalid",
                format!("cannot serialize Quest awake provider request: {error}"),
            )
        })?;
        let stage_id = unique_stage_id(&invocation.request_id)?;
        let output = self.transport.invoke(config, &request_json, &stage_id)?;
        if output.len() > MAX_PROVIDER_OUTPUT_BYTES {
            return Err(QuestAwakeAdapterError::new(
                "provider_output_oversized",
                "Quest awake provider response exceeds 64 KiB",
            ));
        }
        let response: ProviderResponse = serde_json::from_slice(&output).map_err(|error| {
            QuestAwakeAdapterError::new(
                "provider_response_invalid",
                format!("Quest awake provider response is not valid JSON: {error}"),
            )
        })?;
        if response.contract_version != QUEST_AWAKE_PROVIDER_CONTRACT {
            return Err(QuestAwakeAdapterError::new(
                "provider_contract_mismatch",
                "Quest awake provider returned a different contract version",
            ));
        }
        let provider = response.receipt.ok_or_else(|| {
            QuestAwakeAdapterError::new(
                response
                    .error
                    .as_deref()
                    .unwrap_or("provider_receipt_missing"),
                response
                    .message
                    .unwrap_or_else(|| "Quest awake provider returned no receipt".to_owned()),
            )
        })?;
        if provider.schema != QUEST_AWAKE_RECEIPT_SCHEMA
            || provider.contract_version != QUEST_AWAKE_PROVIDER_CONTRACT
            || provider.request_id != invocation.request_id
            || provider.operation_id != invocation.operation_id
            || provider.preview_id != invocation.preview_id
            || provider.device_id != invocation.device_id
            || provider.identity_revision != invocation.identity_revision
            || provider.action != invocation.action.provider_action()
            || provider.watchdog_generation != invocation.watchdog_generation
            || provider.requested_duration_milliseconds != invocation.duration_ms
            || provider.requested_watchdog_interval_milliseconds != invocation.watchdog_interval_ms
        {
            return Err(QuestAwakeAdapterError::new(
                "provider_receipt_binding_mismatch",
                "Quest awake provider receipt does not bind the exact Fleet invocation",
            ));
        }
        if !matches!(response.status.as_str(), "verified" | "pending") {
            return Err(QuestAwakeAdapterError::new(
                "provider_status_invalid",
                "Quest awake provider returned an unsupported receipt status",
            ));
        }
        let provider_reported_effective = provider.effective;
        if provider.device_watchdog.boot_id.len() < 8
            || provider.device_watchdog.boot_id.len() > 128
            || !provider
                .device_watchdog
                .boot_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(QuestAwakeAdapterError::new(
                "provider_boot_identity_invalid",
                "Quest awake provider returned no usable device boot identity",
            ));
        }
        let boot_id_sha256 = format!(
            "{:x}",
            Sha256::digest(provider.device_watchdog.boot_id.as_bytes())
        );
        let device_watchdog_effective = provider.device_watchdog.reported_active
            && provider.device_watchdog.fresh
            && provider.device_watchdog.generation == invocation.watchdog_generation
            && provider.device_watchdog.interval_milliseconds == invocation.watchdog_interval_ms
            && provider.device_watchdog_effective;
        let settings_left_unchanged = matches!(
            invocation.action,
            QuestAwakeAction::Status | QuestAwakeAction::StopWatchdogs
        ) && provider.settings_left_unchanged;
        let receipt = QuestAwakeOwnerReceipt {
            schema: provider.schema,
            request_id: provider.request_id,
            operation_id: provider.operation_id,
            preview_id: provider.preview_id,
            device_id: provider.device_id,
            identity_revision: provider.identity_revision,
            action: invocation.action,
            watchdog_generation: provider.watchdog_generation,
            requested_duration_ms: provider.requested_duration_milliseconds,
            requested_watchdog_interval_ms: provider.requested_watchdog_interval_milliseconds,
            stay_on_effective: provider.stay_on_effective,
            proximity_hold_effective: provider.proximity_hold_effective,
            wake_effective: provider.wake_effective,
            windows_watchdog_effective,
            device_watchdog_effective,
            settings_restored: provider.settings_restored,
            effective: false,
            settings_left_unchanged,
            outcome: provider.outcome,
            repair_count: provider.repair_count,
            power: QuestAwakePowerReadback {
                wakefulness: bound(provider.power_readback.wakefulness, 128),
                display_state: bound(provider.power_readback.display_state, 128),
                stay_on: provider.power_readback.stay_on,
                auto_sleep_disabled: provider.power_readback.auto_sleep_disabled,
                proximity_state: bound(provider.power_readback.proximity_state, 128),
                proximity_hold_duration_ms: provider
                    .power_readback
                    .proximity_hold_duration_milliseconds,
                proximity_hold_remaining_ms: provider
                    .power_readback
                    .proximity_hold_remaining_milliseconds,
                captured_at_ms: provider.power_readback.captured_at_unix_milliseconds,
            },
            device_watchdog: QuestAwakeWatchdogReadback {
                reported_active: provider.device_watchdog.reported_active,
                fresh: provider.device_watchdog.fresh,
                generation: bound(provider.device_watchdog.generation, 256),
                boot_id_sha256,
                interval_ms: provider.device_watchdog.interval_milliseconds,
                last_poll_ms: provider.device_watchdog.last_poll_unix_milliseconds,
                proximity_repair_count: provider.device_watchdog.proximity_repair_count,
                stay_on_repair_count: provider.device_watchdog.stay_on_repair_count,
                wake_repair_count: provider.device_watchdog.wake_repair_count,
                last_action: bound(provider.device_watchdog.last_action, 256),
                last_error: bound(provider.device_watchdog.last_error, 512),
            },
            evidence_sha256: provider.evidence_sha256,
            observed_at_ms: provider.observed_at_unix_milliseconds,
        };
        let mut receipt = receipt;
        receipt.effective = receipt.derived_effective();
        receipt.validate().map_err(|failures| {
            QuestAwakeAdapterError::new(
                "provider_receipt_invalid",
                failures
                    .iter()
                    .map(|failure| format!("{}:{}", failure.path, failure.code))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        if provider_reported_effective != receipt.effective {
            return Err(QuestAwakeAdapterError::new(
                "provider_effect_mismatch",
                "Quest awake provider effect summary contradicts the independent readbacks",
            ));
        }
        if (response.status == "verified") != receipt.effective {
            return Err(QuestAwakeAdapterError::new(
                "provider_optimistic_success",
                "Quest awake provider status contradicts the required independent readbacks",
            ));
        }
        Ok(receipt)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderRequest<'a> {
    contract_version: &'a str,
    request_id: &'a str,
    operation_id: &'a str,
    preview_id: &'a str,
    device_id: &'a str,
    identity_revision: u64,
    action: &'a str,
    duration_milliseconds: u32,
    watchdog_interval_milliseconds: u32,
    watchdog_generation: &'a str,
    issued_at_unix_milliseconds: i64,
    expires_at_unix_milliseconds: i64,
    serial: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct ProviderResponse {
    contract_version: String,
    status: String,
    receipt: Option<ProviderReceipt>,
    error: Option<String>,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct ProviderReceipt {
    schema: String,
    contract_version: String,
    request_id: String,
    operation_id: String,
    preview_id: String,
    device_id: String,
    identity_revision: u64,
    action: String,
    watchdog_generation: String,
    requested_duration_milliseconds: u32,
    requested_watchdog_interval_milliseconds: u32,
    stay_on_effective: bool,
    proximity_hold_effective: bool,
    wake_effective: bool,
    device_watchdog_effective: bool,
    settings_restored: bool,
    effective: bool,
    settings_left_unchanged: bool,
    outcome: String,
    repair_count: u32,
    power_readback: ProviderPowerReadback,
    device_watchdog: ProviderWatchdogReadback,
    evidence_sha256: String,
    observed_at_unix_milliseconds: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct ProviderPowerReadback {
    wakefulness: String,
    display_state: String,
    stay_on: bool,
    auto_sleep_disabled: Option<bool>,
    proximity_state: String,
    proximity_hold_duration_milliseconds: Option<u32>,
    proximity_hold_remaining_milliseconds: Option<u32>,
    captured_at_unix_milliseconds: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct ProviderWatchdogReadback {
    reported_active: bool,
    fresh: bool,
    generation: String,
    boot_id: String,
    interval_milliseconds: u32,
    last_poll_unix_milliseconds: i64,
    proximity_repair_count: u32,
    stay_on_repair_count: u32,
    wake_repair_count: u32,
    last_action: String,
    last_error: String,
}

fn run_process(
    executable: &Path,
    adb_executable: &Path,
    bundle_extract: &Path,
    request_json: &[u8],
) -> Result<Vec<u8>, QuestAwakeAdapterError> {
    let mut command = Command::new(executable);
    command
        .args(["integration", "quest-awake", "--json"])
        .env_clear()
        .env("DOTNET_BUNDLE_EXTRACT_BASE_DIR", bundle_extract)
        .env("QUESTIONABLE_FILE_MANAGER_ADB", adb_executable)
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
        "TEMP",
        "TMP",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn().map_err(|error| {
        QuestAwakeAdapterError::new(
            "provider_start_failed",
            format!("cannot start Quest awake provider: {error}"),
        )
    })?;
    let write_result = child.stdin.take().map_or_else(
        || {
            Err(QuestAwakeAdapterError::new(
                "provider_start_failed",
                "Quest awake provider stdin is unavailable",
            ))
        },
        |mut input| {
            input.write_all(request_json).map_err(|error| {
                QuestAwakeAdapterError::new(
                    "provider_write_failed",
                    format!("cannot write Quest awake provider request: {error}"),
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
        QuestAwakeAdapterError::new(
            "provider_start_failed",
            "Quest awake provider stdout is unavailable",
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        terminate_process_tree(&mut child);
        QuestAwakeAdapterError::new(
            "provider_start_failed",
            "Quest awake provider stderr is unavailable",
        )
    })?;
    let (stream_sender, stream_receiver) = mpsc::channel();
    drain_provider_stream(ProviderStream::Stdout, stdout, stream_sender.clone());
    drain_provider_stream(ProviderStream::Stderr, stderr, stream_sender);
    let mut stdout_result = None;
    let mut stderr_result = None;
    let started = Instant::now();
    loop {
        while let Ok((stream, result)) = stream_receiver.try_recv() {
            let bytes = match result {
                Ok(bytes) if bytes.len() <= MAX_PROVIDER_OUTPUT_BYTES => bytes,
                Ok(_) => {
                    terminate_process_tree(&mut child);
                    return Err(QuestAwakeAdapterError::new(
                        "provider_output_oversized",
                        "Quest awake provider output exceeds 64 KiB",
                    ));
                }
                Err(error) => {
                    terminate_process_tree(&mut child);
                    return Err(QuestAwakeAdapterError::new(
                        "provider_read_failed",
                        format!("cannot read Quest awake provider output: {error}"),
                    ));
                }
            };
            match stream {
                ProviderStream::Stdout => stdout_result = Some(bytes),
                ProviderStream::Stderr => stderr_result = Some(bytes),
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let drain_deadline = Instant::now() + Duration::from_secs(1);
                while (stdout_result.is_none() || stderr_result.is_none())
                    && Instant::now() < drain_deadline
                {
                    match stream_receiver.recv_timeout(Duration::from_millis(25)) {
                        Ok((stream, result)) => {
                            let bytes = result.map_err(|error| {
                                QuestAwakeAdapterError::new(
                                    "provider_read_failed",
                                    format!("cannot read Quest awake provider output: {error}"),
                                )
                            })?;
                            if bytes.len() > MAX_PROVIDER_OUTPUT_BYTES {
                                return Err(QuestAwakeAdapterError::new(
                                    "provider_output_oversized",
                                    "Quest awake provider output exceeds 64 KiB",
                                ));
                            }
                            match stream {
                                ProviderStream::Stdout => stdout_result = Some(bytes),
                                ProviderStream::Stderr => stderr_result = Some(bytes),
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                let stdout = stdout_result.ok_or_else(|| {
                    QuestAwakeAdapterError::new(
                        "provider_read_failed",
                        "Quest awake provider stdout did not close after process exit",
                    )
                })?;
                let stderr = stderr_result.ok_or_else(|| {
                    QuestAwakeAdapterError::new(
                        "provider_read_failed",
                        "Quest awake provider stderr did not close after process exit",
                    )
                })?;
                if !stderr.is_empty() {
                    return Err(QuestAwakeAdapterError::new(
                        "provider_stderr_rejected",
                        "Quest awake provider wrote unexpected standard-error evidence",
                    ));
                }
                if !status.success() && stdout.is_empty() {
                    return Err(QuestAwakeAdapterError::new(
                        "provider_failed",
                        format!("Quest awake provider exited with {status}"),
                    ));
                }
                return Ok(stdout);
            }
            Ok(None) if started.elapsed() < PROVIDER_TIMEOUT => {
                thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                terminate_process_tree(&mut child);
                return Err(QuestAwakeAdapterError::new(
                    "provider_timeout",
                    "Quest awake provider exceeded its 30-second deadline",
                ));
            }
            Err(error) => {
                terminate_process_tree(&mut child);
                return Err(QuestAwakeAdapterError::new(
                    "provider_wait_failed",
                    format!("cannot wait for Quest awake provider: {error}"),
                ));
            }
        }
    }
}

#[derive(Clone, Copy)]
enum ProviderStream {
    Stdout,
    Stderr,
}

fn drain_provider_stream<R: Read + Send + 'static>(
    stream: ProviderStream,
    reader: R,
    sender: mpsc::Sender<(ProviderStream, std::io::Result<Vec<u8>>)>,
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

fn sha256_file(path: &Path) -> Result<String, QuestAwakeAdapterError> {
    let metadata = fs::metadata(path).map_err(|error| {
        QuestAwakeAdapterError::new(
            "provider_unavailable",
            format!("cannot inspect Quest awake provider: {error}"),
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 256 * 1024 * 1024 {
        return Err(QuestAwakeAdapterError::new(
            "provider_file_invalid",
            "Quest awake provider must be a nonempty bounded regular file",
        ));
    }
    let bytes = fs::read(path).map_err(|error| {
        QuestAwakeAdapterError::new(
            "provider_unavailable",
            format!("cannot read Quest awake provider: {error}"),
        )
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn finish_provider_stage(
    result: Result<Vec<u8>, QuestAwakeAdapterError>,
    cleanup: std::io::Result<()>,
) -> Result<Vec<u8>, QuestAwakeAdapterError> {
    match (result, cleanup) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(_)) => Err(QuestAwakeAdapterError::new(
            "provider_cleanup_failed",
            "Quest awake provider stage could not be removed",
        )),
        (Err(error), Err(_)) => Err(QuestAwakeAdapterError::new(
            "provider_cleanup_failed",
            format!(
                "Quest awake provider stage could not be removed after {}",
                error.code
            ),
        )),
    }
}

fn short_digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))[..24].to_owned()
}

fn unique_stage_id(request_id: &str) -> Result<String, QuestAwakeAdapterError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            QuestAwakeAdapterError::new(
                "clock_invalid",
                format!("system clock precedes Unix epoch: {error}"),
            )
        })?
        .as_nanos();
    let sequence = PROVIDER_STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!(
        "awake-{}-{}-{sequence}-{nonce}",
        short_digest(request_id),
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

fn bound(mut value: String, maximum: usize) -> String {
    value.truncate(maximum);
    value
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use fleet_contracts::{QuestAwakeAction, QuestAwakeOwnerInvocation};
    use serde_json::{Value, json};

    use super::{
        QuestAwakeAdapterError, QuestAwakeOwnerAdapter, QuestAwakePinnedArtifact,
        QuestAwakeProviderConfig, QuestAwakeProviderTransport, finish_provider_stage,
    };

    #[derive(Clone, Default)]
    struct FakeTransport {
        requests: Arc<Mutex<Vec<Value>>>,
        damage_device_id: bool,
        force_active_watchdog: bool,
        empty_boot_id: bool,
    }

    impl QuestAwakeProviderTransport for FakeTransport {
        fn invoke(
            &self,
            _: &QuestAwakeProviderConfig,
            request_json: &[u8],
            _: &str,
        ) -> Result<Vec<u8>, QuestAwakeAdapterError> {
            let request: Value = serde_json::from_slice(request_json).expect("provider request");
            self.requests
                .lock()
                .expect("request lock")
                .push(request.clone());
            let action = request["action"].as_str().expect("action");
            let device_watchdog = action == "startDeviceWatchdog" || self.force_active_watchdog;
            let stop = action == "stopWatchdogs";
            let restore = action == "restoreNormal";
            let device_id = if self.damage_device_id {
                "different-device"
            } else {
                request["deviceId"].as_str().expect("device")
            };
            serde_json::to_vec(&json!({
                "contractVersion": "questionable.file_manager.fleet_awake_provider.v1",
                "status": "verified",
                "receipt": {
                    "schema": "questionable.file_manager.quest_awake_receipt.v1",
                    "contractVersion": "questionable.file_manager.fleet_awake_provider.v1",
                    "requestId": request["requestId"],
                    "operationId": request["operationId"],
                    "previewId": request["previewId"],
                    "deviceId": device_id,
                    "identityRevision": request["identityRevision"],
                    "action": action,
                    "watchdogGeneration": request["watchdogGeneration"],
                    "requestedDurationMilliseconds": request["durationMilliseconds"],
                    "requestedWatchdogIntervalMilliseconds": request["watchdogIntervalMilliseconds"],
                    "stayOnEffective": !restore,
                    "proximityHoldEffective": !restore,
                    "wakeEffective": !restore,
                    "deviceWatchdogEffective": device_watchdog,
                    "settingsRestored": restore,
                    "effective": true,
                    "settingsLeftUnchanged": stop,
                    "outcome": "verified_readback",
                    "repairCount": 0,
                    "powerReadback": {
                        "wakefulness": if restore {"Asleep"} else {"Awake"},
                        "displayState": if restore {"OFF"} else {"ON"},
                        "stayOn": !restore,
                        "autoSleepDisabled": !restore,
                        "proximityState": if restore {"FAR"} else {"CLOSE"},
                        "proximityHoldDurationMilliseconds": request["durationMilliseconds"],
                        "proximityHoldRemainingMilliseconds": request["durationMilliseconds"],
                        "capturedAtUnixMilliseconds": 1350
                    },
                    "deviceWatchdog": {
                        "reportedActive": device_watchdog,
                        "fresh": device_watchdog,
                        "generation": request["watchdogGeneration"],
                        "bootId": if self.empty_boot_id {""} else {"private-boot-id"},
                        "intervalMilliseconds": request["watchdogIntervalMilliseconds"],
                        "lastPollUnixMilliseconds": 1300,
                        "proximityRepairCount": 0,
                        "stayOnRepairCount": 0,
                        "wakeRepairCount": 0,
                        "lastAction": "observed",
                        "lastError": ""
                    },
                    "evidenceSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "observedAtUnixMilliseconds": 1400
                },
                "error": null,
                "message": null
            }))
            .map_err(|error| QuestAwakeAdapterError::new("test_response", error.to_string()))
        }
    }

    fn config() -> QuestAwakeProviderConfig {
        QuestAwakeProviderConfig {
            executable_path: PathBuf::from(
                r"C:\private\questionable-file-manager-awake-provider.exe",
            ),
            executable_sha256: "a".repeat(64),
            adb_executable_path: PathBuf::from(r"C:\private\adb.exe"),
            adb_executable_sha256: "b".repeat(64),
            adb_support_artifacts: vec![
                QuestAwakePinnedArtifact {
                    source_path: PathBuf::from(r"C:\private\AdbWinApi.dll"),
                    sha256: "c".repeat(64),
                },
                QuestAwakePinnedArtifact {
                    source_path: PathBuf::from(r"C:\private\AdbWinUsbApi.dll"),
                    sha256: "d".repeat(64),
                },
            ],
            private_stage_root: PathBuf::from(r"C:\private\stage"),
        }
    }

    fn invocation(action: QuestAwakeAction) -> QuestAwakeOwnerInvocation {
        QuestAwakeOwnerInvocation {
            schema: "rusty.fleet.quest_awake_owner_invocation.v1".to_owned(),
            request_id: "awake-request-1".to_owned(),
            operation_id: "awake-operation-1".to_owned(),
            preview_id: "awake-preview-1".to_owned(),
            device_id: "device.quest.1".to_owned(),
            identity_revision: 7,
            action,
            duration_ms: 28_800_000,
            watchdog_interval_ms: 5_000,
            watchdog_generation: "awake-generation-1".to_owned(),
            issued_at_ms: 1_200,
            expires_at_ms: 61_000,
        }
    }

    #[test]
    fn adapter_sends_private_serial_but_redacts_serial_and_boot_id_from_public_receipt() {
        let transport = FakeTransport::default();
        let requests = Arc::clone(&transport.requests);
        let receipt = QuestAwakeOwnerAdapter::new(transport)
            .invoke(
                &config(),
                &invocation(QuestAwakeAction::StartDeviceWatchdog),
                "private-serial-123",
                false,
            )
            .expect("verified device watchdog receipt");

        let requests = requests.lock().expect("request lock");
        assert_eq!(requests[0]["serial"], "private-serial-123");
        assert_eq!(requests[0]["action"], "startDeviceWatchdog");
        let public = serde_json::to_string(&receipt).expect("serialize public receipt");
        assert!(!public.contains("private-serial-123"));
        assert!(!public.contains("private-boot-id"));
        assert_eq!(receipt.device_watchdog.boot_id_sha256.len(), 64);
        assert!(receipt.effective);
    }

    #[test]
    fn adapter_rejects_receipts_that_do_not_bind_the_exact_device() {
        let transport = FakeTransport {
            damage_device_id: true,
            ..FakeTransport::default()
        };
        let error = QuestAwakeOwnerAdapter::new(transport)
            .invoke(
                &config(),
                &invocation(QuestAwakeAction::ApplyBounded),
                "private-serial-123",
                false,
            )
            .expect_err("mismatched provider receipt");
        assert_eq!(error.code, "provider_receipt_binding_mismatch");
        assert!(!error.message.contains("private-serial-123"));
    }

    #[test]
    fn stop_and_restore_project_distinct_effects() {
        let stop = QuestAwakeOwnerAdapter::new(FakeTransport::default())
            .invoke(
                &config(),
                &invocation(QuestAwakeAction::StopWatchdogs),
                "private-serial-123",
                false,
            )
            .expect("stop receipt");
        assert!(stop.settings_left_unchanged);
        assert!(!stop.settings_restored);

        let restore = QuestAwakeOwnerAdapter::new(FakeTransport::default())
            .invoke(
                &config(),
                &invocation(QuestAwakeAction::RestoreNormal),
                "private-serial-123",
                false,
            )
            .expect("restore receipt");
        assert!(!restore.settings_left_unchanged);
        assert!(restore.settings_restored);
    }

    #[test]
    fn stop_rejects_a_provider_receipt_when_the_device_watchdog_is_still_active() {
        let transport = FakeTransport {
            force_active_watchdog: true,
            ..FakeTransport::default()
        };
        let error = QuestAwakeOwnerAdapter::new(transport)
            .invoke(
                &config(),
                &invocation(QuestAwakeAction::StopWatchdogs),
                "private-serial-123",
                false,
            )
            .expect_err("an active device watchdog cannot prove stop");
        assert_eq!(error.code, "provider_effect_mismatch");
        assert!(!error.message.contains("private-serial-123"));
    }

    #[test]
    fn active_watchdog_requires_nonempty_boot_identity_before_hashing() {
        let transport = FakeTransport {
            empty_boot_id: true,
            ..FakeTransport::default()
        };
        let error = QuestAwakeOwnerAdapter::new(transport)
            .invoke(
                &config(),
                &invocation(QuestAwakeAction::StartDeviceWatchdog),
                "private-serial-123",
                false,
            )
            .expect_err("empty boot identity");
        assert_eq!(error.code, "provider_boot_identity_invalid");
    }

    #[test]
    fn cleanup_failure_supersedes_the_primary_error_without_leaking_paths() {
        let primary = QuestAwakeAdapterError::new(
            "provider_timeout",
            r"private failure at C:\sensitive\stage",
        );
        let cleanup = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            r"cannot remove C:\sensitive\stage",
        );
        let error = finish_provider_stage(Err(primary), Err(cleanup))
            .expect_err("cleanup failure must be explicit");
        assert_eq!(error.code, "provider_cleanup_failed");
        assert!(error.message.contains("provider_timeout"));
        assert!(!error.message.contains("sensitive"));
    }
}
