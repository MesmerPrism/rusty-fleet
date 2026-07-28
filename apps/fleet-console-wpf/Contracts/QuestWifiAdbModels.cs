// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Text.Json.Serialization;

namespace RustyFleet.FleetConsole.Contracts;

public static class QuestWifiAdbActions
{
    public const string ActionId = "quest.wifi-adb-control";
    public const string Status = "status";
    public const string RequestWirelessAdb = "request_wireless_adb";
    public const string EnableRequestAfterBoot = "enable_request_after_boot";
    public const string DisableRequestAfterBoot = "disable_request_after_boot";
    public const string DisableWirelessAdb = "disable_wireless_adb";
    public const string EnableClassicTcpipFromUsb = "enable_classic_tcpip_from_usb";

    public static IReadOnlySet<string> All { get; } = new HashSet<string>(
        [
            Status,
            RequestWirelessAdb,
            EnableRequestAfterBoot,
            DisableRequestAfterBoot,
            DisableWirelessAdb,
            EnableClassicTcpipFromUsb
        ],
        StringComparer.Ordinal);
}

public sealed record QuestWifiAdbActionOption(
    string Action,
    string Label,
    string HelpText,
    string ConfirmationText,
    bool IsDestructive);

public sealed class QuestWifiAdbPreviewRequest
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } =
        "rusty.fleet.quest_wifi_adb_preview_request.v1";

    [JsonPropertyName("action_id")]
    public string ActionId { get; init; } = QuestWifiAdbActions.ActionId;

    [JsonPropertyName("action")]
    public string Action { get; init; } = QuestWifiAdbActions.Status;

    [JsonPropertyName("targets")]
    public IReadOnlyDictionary<string, ulong> Targets { get; init; } =
        new SortedDictionary<string, ulong>(StringComparer.Ordinal);
}

public sealed class QuestWifiAdbExecuteRequest
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } =
        "rusty.fleet.quest_wifi_adb_execute_request.v1";

    [JsonPropertyName("operation_id")]
    public string OperationId { get; init; } = string.Empty;

    [JsonPropertyName("preview_id")]
    public string PreviewId { get; init; } = string.Empty;
}

public sealed class QuestWifiAdbOwnerBinding
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

    [JsonPropertyName("private_target_resolution")]
    public string PrivateTargetResolution { get; init; } = string.Empty;
}

public sealed class QuestWifiAdbTargetPreflight
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

public sealed class QuestWifiAdbPreview
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

    [JsonPropertyName("owner")]
    public QuestWifiAdbOwnerBinding Owner { get; init; } = new();

    [JsonPropertyName("targets")]
    public IReadOnlyList<QuestWifiAdbTargetPreflight> Targets { get; init; } = [];
}

public sealed class QuestWifiAdbOwnerInvocation
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

    [JsonPropertyName("issued_at_ms")]
    public long IssuedAtMs { get; init; }

    [JsonPropertyName("expires_at_ms")]
    public long ExpiresAtMs { get; init; }
}

public sealed class QuestWifiAdbOwnerReceipt
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

    [JsonPropertyName("route_mode")]
    public string RouteMode { get; init; } = string.Empty;

    [JsonPropertyName("request_delivered")]
    public bool RequestDelivered { get; init; }

    [JsonPropertyName("kiosk_setting_applied")]
    public bool KioskSettingApplied { get; init; }

    [JsonPropertyName("request_after_boot_enabled")]
    public bool? RequestAfterBootEnabled { get; init; }

    [JsonPropertyName("wearer_approval")]
    public string WearerApproval { get; init; } = string.Empty;

    [JsonPropertyName("listener_discovered")]
    public bool ListenerDiscovered { get; init; }

    [JsonPropertyName("effect_applied")]
    public bool EffectApplied { get; init; }

    [JsonPropertyName("outcome")]
    public string Outcome { get; init; } = string.Empty;

    [JsonPropertyName("evidence_sha256")]
    public string EvidenceSha256 { get; init; } = string.Empty;

    [JsonPropertyName("observed_at_ms")]
    public long ObservedAtMs { get; init; }
}

public sealed class QuestWifiAdbTermuxProof
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonPropertyName("proof_id")]
    public string ProofId { get; init; } = string.Empty;

    [JsonPropertyName("owner_id")]
    public string OwnerId { get; init; } = string.Empty;

    [JsonPropertyName("device_id")]
    public string DeviceId { get; init; } = string.Empty;

    [JsonPropertyName("identity_revision")]
    public ulong IdentityRevision { get; init; }

    [JsonPropertyName("source_epoch")]
    public string SourceEpoch { get; init; } = string.Empty;

    [JsonPropertyName("source_revision")]
    public ulong SourceRevision { get; init; }

    [JsonPropertyName("route_mode")]
    public string RouteMode { get; init; } = string.Empty;

    [JsonPropertyName("discovery_mode")]
    public string DiscoveryMode { get; init; } = string.Empty;

    [JsonPropertyName("listener_discovered")]
    public bool ListenerDiscovered { get; init; }

    [JsonPropertyName("shell_identity")]
    public string? ShellIdentity { get; init; }

    [JsonPropertyName("available")]
    public bool Available { get; init; }

    [JsonPropertyName("evidence_sha256")]
    public string EvidenceSha256 { get; init; } = string.Empty;

    [JsonPropertyName("observed_at_ms")]
    public long ObservedAtMs { get; init; }

    [JsonPropertyName("fresh_until_ms")]
    public long FreshUntilMs { get; init; }
}

public sealed class QuestWifiAdbTargetLedger
{
    [JsonPropertyName("device_id")]
    public string DeviceId { get; init; } = string.Empty;

    [JsonPropertyName("identity_revision")]
    public ulong IdentityRevision { get; init; }

    [JsonPropertyName("preflight")]
    public QuestWifiAdbTargetPreflight Preflight { get; init; } = new();

    [JsonPropertyName("lifecycle")]
    public string Lifecycle { get; init; } = string.Empty;

    [JsonPropertyName("invocation")]
    public QuestWifiAdbOwnerInvocation? Invocation { get; init; }

    [JsonPropertyName("receipt")]
    public QuestWifiAdbOwnerReceipt? Receipt { get; init; }

    [JsonPropertyName("termux_proof")]
    public QuestWifiAdbTermuxProof? TermuxProof { get; init; }

    [JsonPropertyName("termux_usable")]
    public bool TermuxUsable { get; init; }

    [JsonPropertyName("failure_code")]
    public string? FailureCode { get; init; }

    [JsonPropertyName("updated_at_ms")]
    public long UpdatedAtMs { get; init; }
}

public sealed class QuestWifiAdbOperation
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
    public QuestWifiAdbPreview Preview { get; init; } = new();

    [JsonPropertyName("confirmed_at_ms")]
    public long? ConfirmedAtMs { get; init; }

    [JsonPropertyName("targets")]
    public IReadOnlyList<QuestWifiAdbTargetLedger> Targets { get; init; } = [];

    [JsonPropertyName("updated_at_ms")]
    public long UpdatedAtMs { get; init; }
}
