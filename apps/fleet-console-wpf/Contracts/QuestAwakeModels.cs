// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Text.Json.Serialization;

namespace RustyFleet.FleetConsole.Contracts;

public static class QuestAwakeActions
{
    public const string ActionId = "quest.awake-control";
    public const string Status = "status";
    public const string ApplyBounded = "apply_bounded";
    public const string StartWindowsWatchdog = "start_windows_watchdog";
    public const string StartDeviceWatchdog = "start_device_watchdog";
    public const string StopWatchdogs = "stop_watchdogs";
    public const string RestoreNormal = "restore_normal";

    public const uint MinimumDurationMs = 60_000;
    public const uint MaximumDurationMs = 28_800_000;
    public const uint MinimumWatchdogIntervalMs = 1_000;
    public const uint MaximumWatchdogIntervalMs = 60_000;
    public const uint DefaultWatchdogIntervalMs = 5_000;

    public static IReadOnlySet<string> All { get; } = new HashSet<string>(
        [
            Status,
            ApplyBounded,
            StartWindowsWatchdog,
            StartDeviceWatchdog,
            StopWatchdogs,
            RestoreNormal
        ],
        StringComparer.Ordinal);
}

public sealed record QuestAwakeActionOption(
    string Action,
    string Label,
    string HelpText);

public sealed class QuestAwakePreviewRequest
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = "rusty.fleet.quest_awake_preview_request.v1";

    [JsonPropertyName("action_id")]
    public string ActionId { get; init; } = QuestAwakeActions.ActionId;

    [JsonPropertyName("action")]
    public string Action { get; init; } = QuestAwakeActions.Status;

    [JsonPropertyName("duration_ms")]
    public uint DurationMs { get; init; } = QuestAwakeActions.MaximumDurationMs;

    [JsonPropertyName("watchdog_interval_ms")]
    public uint WatchdogIntervalMs { get; init; } =
        QuestAwakeActions.DefaultWatchdogIntervalMs;

    [JsonPropertyName("targets")]
    public IReadOnlyDictionary<string, ulong> Targets { get; init; } =
        new SortedDictionary<string, ulong>(StringComparer.Ordinal);
}

public sealed class QuestAwakeExecuteRequest
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = "rusty.fleet.quest_awake_execute_request.v1";

    [JsonPropertyName("operation_id")]
    public string OperationId { get; init; } = string.Empty;

    [JsonPropertyName("preview_id")]
    public string PreviewId { get; init; } = string.Empty;
}

public sealed class QuestAwakeOwnerBinding
{
    [JsonPropertyName("owner_repo_id")]
    public string OwnerRepoId { get; init; } = string.Empty;

    [JsonPropertyName("capability_id")]
    public string CapabilityId { get; init; } = string.Empty;

    [JsonPropertyName("provider_contract")]
    public string ProviderContract { get; init; } = string.Empty;

    [JsonPropertyName("receipt_schema")]
    public string ReceiptSchema { get; init; } = string.Empty;

    [JsonPropertyName("transport")]
    public string Transport { get; init; } = string.Empty;

    [JsonPropertyName("application_proof")]
    public string ApplicationProof { get; init; } = string.Empty;
}

public sealed class QuestAwakeTargetPreflight
{
    [JsonPropertyName("device_id")]
    public string DeviceId { get; init; } = string.Empty;

    [JsonPropertyName("identity_revision")]
    public ulong IdentityRevision { get; init; }

    [JsonPropertyName("capability_id")]
    public string CapabilityId { get; init; } = string.Empty;

    [JsonPropertyName("capability_evidence_revision")]
    public ulong CapabilityEvidenceRevision { get; init; }

    [JsonPropertyName("capability_owner")]
    public string CapabilityOwner { get; init; } = string.Empty;

    [JsonPropertyName("support")]
    public string Support { get; init; } = string.Empty;

    [JsonPropertyName("enablement")]
    public string Enablement { get; init; } = string.Empty;

    [JsonPropertyName("authorization")]
    public string Authorization { get; init; } = string.Empty;

    [JsonPropertyName("reachability")]
    public string Reachability { get; init; } = string.Empty;

    [JsonPropertyName("freshness")]
    public string Freshness { get; init; } = string.Empty;

    [JsonPropertyName("observed_at_ms")]
    public long ObservedAtMs { get; init; }

    [JsonPropertyName("fresh_until_ms")]
    public long FreshUntilMs { get; init; }

    [JsonPropertyName("evaluated_at_ms")]
    public long EvaluatedAtMs { get; init; }

    [JsonPropertyName("eligible")]
    public bool Eligible { get; init; }

    [JsonPropertyName("reason_code")]
    public string ReasonCode { get; init; } = string.Empty;

    [JsonPropertyName("message")]
    public string Message { get; init; } = string.Empty;
}

public sealed class QuestAwakePreview
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonPropertyName("preview_id")]
    public string PreviewId { get; init; } = string.Empty;

    [JsonPropertyName("operation_id")]
    public string OperationId { get; init; } = string.Empty;

    [JsonPropertyName("action_id")]
    public string ActionId { get; init; } = string.Empty;

    [JsonPropertyName("action")]
    public string Action { get; init; } = string.Empty;

    [JsonPropertyName("created_at_ms")]
    public long CreatedAtMs { get; init; }

    [JsonPropertyName("expires_at_ms")]
    public long ExpiresAtMs { get; init; }

    [JsonPropertyName("fleet_revision")]
    public ulong FleetRevision { get; init; }

    [JsonPropertyName("duration_ms")]
    public uint DurationMs { get; init; }

    [JsonPropertyName("watchdog_interval_ms")]
    public uint WatchdogIntervalMs { get; init; }

    [JsonPropertyName("watchdog_generation")]
    public string WatchdogGeneration { get; init; } = string.Empty;

    [JsonPropertyName("owner")]
    public QuestAwakeOwnerBinding Owner { get; init; } = new();

    [JsonPropertyName("targets")]
    public IReadOnlyList<QuestAwakeTargetPreflight> Targets { get; init; } = [];
}

public sealed class QuestAwakeOwnerInvocation
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonPropertyName("request_id")]
    public string RequestId { get; init; } = string.Empty;

    [JsonPropertyName("operation_id")]
    public string OperationId { get; init; } = string.Empty;

    [JsonPropertyName("preview_id")]
    public string PreviewId { get; init; } = string.Empty;

    [JsonPropertyName("device_id")]
    public string DeviceId { get; init; } = string.Empty;

    [JsonPropertyName("identity_revision")]
    public ulong IdentityRevision { get; init; }

    [JsonPropertyName("action")]
    public string Action { get; init; } = string.Empty;

    [JsonPropertyName("duration_ms")]
    public uint DurationMs { get; init; }

    [JsonPropertyName("watchdog_interval_ms")]
    public uint WatchdogIntervalMs { get; init; }

    [JsonPropertyName("watchdog_generation")]
    public string WatchdogGeneration { get; init; } = string.Empty;

    [JsonPropertyName("issued_at_ms")]
    public long IssuedAtMs { get; init; }

    [JsonPropertyName("expires_at_ms")]
    public long ExpiresAtMs { get; init; }
}

public sealed class QuestAwakePowerReadback
{
    [JsonPropertyName("wakefulness")]
    public string Wakefulness { get; init; } = string.Empty;

    [JsonPropertyName("display_state")]
    public string DisplayState { get; init; } = string.Empty;

    [JsonPropertyName("stay_on")]
    public bool StayOn { get; init; }

    [JsonPropertyName("auto_sleep_disabled")]
    public bool? AutoSleepDisabled { get; init; }

    [JsonPropertyName("proximity_state")]
    public string ProximityState { get; init; } = string.Empty;

    [JsonPropertyName("proximity_hold_duration_ms")]
    public uint? ProximityHoldDurationMs { get; init; }

    [JsonPropertyName("proximity_hold_remaining_ms")]
    public uint? ProximityHoldRemainingMs { get; init; }

    [JsonPropertyName("captured_at_ms")]
    public long CapturedAtMs { get; init; }
}

public sealed class QuestAwakeWatchdogReadback
{
    [JsonPropertyName("reported_active")]
    public bool ReportedActive { get; init; }

    [JsonPropertyName("fresh")]
    public bool Fresh { get; init; }

    [JsonPropertyName("generation")]
    public string Generation { get; init; } = string.Empty;

    [JsonPropertyName("boot_id_sha256")]
    public string BootIdSha256 { get; init; } = string.Empty;

    [JsonPropertyName("interval_ms")]
    public uint IntervalMs { get; init; }

    [JsonPropertyName("last_poll_ms")]
    public long LastPollMs { get; init; }

    [JsonPropertyName("proximity_repair_count")]
    public uint ProximityRepairCount { get; init; }

    [JsonPropertyName("stay_on_repair_count")]
    public uint StayOnRepairCount { get; init; }

    [JsonPropertyName("wake_repair_count")]
    public uint WakeRepairCount { get; init; }

    [JsonPropertyName("last_action")]
    public string LastAction { get; init; } = string.Empty;

    [JsonPropertyName("last_error")]
    public string LastError { get; init; } = string.Empty;
}

public sealed class QuestAwakeOwnerReceipt
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonPropertyName("request_id")]
    public string RequestId { get; init; } = string.Empty;

    [JsonPropertyName("operation_id")]
    public string OperationId { get; init; } = string.Empty;

    [JsonPropertyName("preview_id")]
    public string PreviewId { get; init; } = string.Empty;

    [JsonPropertyName("device_id")]
    public string DeviceId { get; init; } = string.Empty;

    [JsonPropertyName("identity_revision")]
    public ulong IdentityRevision { get; init; }

    [JsonPropertyName("action")]
    public string Action { get; init; } = string.Empty;

    [JsonPropertyName("watchdog_generation")]
    public string WatchdogGeneration { get; init; } = string.Empty;

    [JsonPropertyName("requested_duration_ms")]
    public uint RequestedDurationMs { get; init; }

    [JsonPropertyName("requested_watchdog_interval_ms")]
    public uint RequestedWatchdogIntervalMs { get; init; }

    [JsonPropertyName("stay_on_effective")]
    public bool StayOnEffective { get; init; }

    [JsonPropertyName("proximity_hold_effective")]
    public bool ProximityHoldEffective { get; init; }

    [JsonPropertyName("wake_effective")]
    public bool WakeEffective { get; init; }

    [JsonPropertyName("windows_watchdog_effective")]
    public bool WindowsWatchdogEffective { get; init; }

    [JsonPropertyName("device_watchdog_effective")]
    public bool DeviceWatchdogEffective { get; init; }

    [JsonPropertyName("settings_restored")]
    public bool SettingsRestored { get; init; }

    [JsonPropertyName("effective")]
    public bool Effective { get; init; }

    [JsonPropertyName("settings_left_unchanged")]
    public bool SettingsLeftUnchanged { get; init; }

    [JsonPropertyName("outcome")]
    public string Outcome { get; init; } = string.Empty;

    [JsonPropertyName("repair_count")]
    public uint RepairCount { get; init; }

    [JsonPropertyName("power")]
    public QuestAwakePowerReadback Power { get; init; } = new();

    [JsonPropertyName("device_watchdog")]
    public QuestAwakeWatchdogReadback DeviceWatchdog { get; init; } = new();

    [JsonPropertyName("evidence_sha256")]
    public string EvidenceSha256 { get; init; } = string.Empty;

    [JsonPropertyName("observed_at_ms")]
    public long ObservedAtMs { get; init; }
}

public sealed class QuestAwakeTargetLedger
{
    [JsonPropertyName("device_id")]
    public string DeviceId { get; init; } = string.Empty;

    [JsonPropertyName("identity_revision")]
    public ulong IdentityRevision { get; init; }

    [JsonPropertyName("preflight")]
    public QuestAwakeTargetPreflight Preflight { get; init; } = new();

    [JsonPropertyName("lifecycle")]
    public string Lifecycle { get; init; } = string.Empty;

    [JsonPropertyName("invocation")]
    public QuestAwakeOwnerInvocation? Invocation { get; init; }

    [JsonPropertyName("receipt")]
    public QuestAwakeOwnerReceipt? Receipt { get; init; }

    [JsonPropertyName("failure_code")]
    public string? FailureCode { get; init; }

    [JsonPropertyName("updated_at_ms")]
    public long UpdatedAtMs { get; init; }
}

public sealed class QuestAwakeOperation
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonPropertyName("operation_id")]
    public string OperationId { get; init; } = string.Empty;

    [JsonPropertyName("action_id")]
    public string ActionId { get; init; } = string.Empty;

    [JsonPropertyName("lifecycle")]
    public string Lifecycle { get; init; } = string.Empty;

    [JsonPropertyName("preview")]
    public QuestAwakePreview Preview { get; init; } = new();

    [JsonPropertyName("confirmed_at_ms")]
    public long? ConfirmedAtMs { get; init; }

    [JsonPropertyName("targets")]
    public IReadOnlyList<QuestAwakeTargetLedger> Targets { get; init; } = [];

    [JsonPropertyName("updated_at_ms")]
    public long UpdatedAtMs { get; init; }
}
