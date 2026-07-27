// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Text.Json.Serialization;

namespace RustyFleet.FleetConsole.Contracts;

public static class WindowsHotspotActions
{
    public const string ActionId = "host.windows-mobile-hotspot";
    public const string ResourceId = "windows.mobile-hotspot";
    public const string OwnerId = "rusty-hostess";
    public const string Status = "status";
    public const string Start = "start";
    public const string Ensure = "ensure";
    public const string Stop = "stop";
    public const string OwnershipNone = "none";
    public const string OwnershipFleet = "fleet";
    public const string OwnershipExternal = "external";
    public const string ResultVerified = "verified";
    public const string ResultFailed = "failed";
    public const string ResultRejected = "rejected";
    public const string ResultUnavailable = "unavailable";

    public static IReadOnlySet<string> All { get; } = new HashSet<string>(
        [Status, Start, Ensure, Stop],
        StringComparer.Ordinal);
}

public sealed record WindowsHotspotActionOption(
    string Action,
    string Label,
    string HelpText,
    string ConfirmationText);

[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed class WindowsHotspotPreviewRequest
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } =
        "rusty.fleet.windows_hotspot_preview_request.v1";

    [JsonPropertyName("action_id")]
    public string ActionId { get; init; } = WindowsHotspotActions.ActionId;

    [JsonPropertyName("action")]
    public string Action { get; init; } = WindowsHotspotActions.Status;
}

[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed class WindowsHotspotExecuteRequest
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } =
        "rusty.fleet.windows_hotspot_execute_request.v1";

    [JsonPropertyName("operation_id")]
    public string OperationId { get; init; } = string.Empty;

    [JsonPropertyName("preview_id")]
    public string PreviewId { get; init; } = string.Empty;
}

[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed class WindowsHotspotPreflight
{
    [JsonRequired]
    [JsonPropertyName("provider_ready")]
    public bool ProviderReady { get; init; }

    [JsonRequired]
    [JsonPropertyName("lease_available")]
    public bool LeaseAvailable { get; init; }

    [JsonRequired]
    [JsonPropertyName("active")]
    public bool Active { get; init; }

    [JsonRequired]
    [JsonPropertyName("ownership")]
    public string Ownership { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("ownership_generation")]
    public string? OwnershipGeneration { get; init; }

    [JsonRequired]
    [JsonPropertyName("eligible")]
    public bool Eligible { get; init; }

    [JsonRequired]
    [JsonPropertyName("reason_code")]
    public string ReasonCode { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("message")]
    public string Message { get; init; } = string.Empty;
}

[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed class WindowsHotspotPreview
{
    [JsonRequired]
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("preview_id")]
    public string PreviewId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("operation_id")]
    public string OperationId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("action_id")]
    public string ActionId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("resource_id")]
    public string ResourceId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("owner_id")]
    public string OwnerId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("action")]
    public string Action { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("created_at_ms")]
    public long CreatedAtMs { get; init; }

    [JsonRequired]
    [JsonPropertyName("expires_at_ms")]
    public long ExpiresAtMs { get; init; }

    [JsonRequired]
    [JsonPropertyName("fleet_revision")]
    public ulong FleetRevision { get; init; }

    [JsonRequired]
    [JsonPropertyName("preflight")]
    public WindowsHotspotPreflight Preflight { get; init; } = new();
}

[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed class WindowsHotspotLease
{
    [JsonRequired]
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("lease_id")]
    public string LeaseId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("resource_id")]
    public string ResourceId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("holder_operation_id")]
    public string HolderOperationId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("generation")]
    public string Generation { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("issued_at_ms")]
    public long IssuedAtMs { get; init; }

    [JsonRequired]
    [JsonPropertyName("expires_at_ms")]
    public long ExpiresAtMs { get; init; }
}

[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed class WindowsHotspotProviderRequest
{
    [JsonRequired]
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("request_id")]
    public string RequestId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("operation_id")]
    public string OperationId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("action")]
    public string Action { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("expires_at_utc")]
    public string ExpiresAtUtc { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("timeout_ms")]
    public uint TimeoutMs { get; init; }

    [JsonPropertyName("ownership_generation")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? OwnershipGeneration { get; init; }
}

[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed class WindowsHotspotProviderReceipt
{
    [JsonRequired]
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("request_id")]
    public string RequestId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("operation_id")]
    public string OperationId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("action")]
    public string Action { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("outcome")]
    public string Outcome { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("reason")]
    public string Reason { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("observed_at_utc")]
    public string ObservedAtUtc { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("capability_available")]
    public bool CapabilityAvailable { get; init; }

    [JsonRequired]
    [JsonPropertyName("capability")]
    public string Capability { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("operational_state")]
    public string OperationalState { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("client_count")]
    public uint ClientCount { get; init; }

    [JsonRequired]
    [JsonPropertyName("max_client_count")]
    public uint MaxClientCount { get; init; }

    [JsonRequired]
    [JsonPropertyName("band")]
    public string Band { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("source_connectivity")]
    public string SourceConnectivity { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("ownership_generation")]
    public string? OwnershipGeneration { get; init; }
}

[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed class WindowsHotspotOperation
{
    [JsonRequired]
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("operation_id")]
    public string OperationId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("action_id")]
    public string ActionId { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("lifecycle")]
    public string Lifecycle { get; init; } = string.Empty;

    [JsonRequired]
    [JsonPropertyName("preview")]
    public WindowsHotspotPreview Preview { get; init; } = new();

    [JsonRequired]
    [JsonPropertyName("confirmed_at_ms")]
    public long? ConfirmedAtMs { get; init; }

    [JsonRequired]
    [JsonPropertyName("lease")]
    public WindowsHotspotLease? Lease { get; init; }

    [JsonRequired]
    [JsonPropertyName("invocation")]
    public WindowsHotspotProviderRequest? Invocation { get; init; }

    [JsonRequired]
    [JsonPropertyName("receipt")]
    public WindowsHotspotProviderReceipt? Receipt { get; init; }

    [JsonRequired]
    [JsonPropertyName("failure_code")]
    public string? FailureCode { get; init; }

    [JsonRequired]
    [JsonPropertyName("updated_at_ms")]
    public long UpdatedAtMs { get; init; }
}
