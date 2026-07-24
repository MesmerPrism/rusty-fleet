// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Text.Json;
using System.Text.Json.Serialization;

namespace RustyFleet.FleetConsole.Contracts;

public static class FleetJson
{
    public static JsonSerializerOptions Options { get; } = new()
    {
        PropertyNameCaseInsensitive = false,
        WriteIndented = false
    };

    public static FleetQueryResult DeserializeQueryResult(string json) =>
        JsonSerializer.Deserialize<FleetQueryResult>(json, Options)
        ?? throw new JsonException("Fleet query result was empty.");
}

public sealed class FleetQuery
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = "rusty.fleet.query.v1";

    [JsonPropertyName("query_id")]
    public string QueryId { get; init; } = "fleet-console";

    [JsonPropertyName("expression")]
    public object? Expression { get; init; }

    [JsonPropertyName("sort")]
    public IReadOnlyList<FleetSortKey> Sort { get; init; } =
    [
        new() { Field = "display_name", Direction = "ascending" }
    ];

    [JsonPropertyName("offset")]
    public int Offset { get; init; }

    [JsonPropertyName("limit")]
    public int Limit { get; init; } = 1_000;

    public static FleetQuery Create(
        string? searchText,
        string? freshness = null,
        int limit = 1_000,
        string sortField = "display_name",
        string sortDirection = "ascending")
    {
        var normalized = searchText?.Trim();
        var terms = new List<object>();
        if (!string.IsNullOrEmpty(normalized))
        {
            terms.Add(new Dictionary<string, object?>
            {
                ["kind"] = "or",
                ["expressions"] = new object[]
                {
                    Predicate("display_name", "contains", normalized),
                    Predicate("device_id", "contains", normalized)
                }
            });
        }

        var normalizedFreshness = freshness?.Trim().ToLowerInvariant();
        if (!string.IsNullOrEmpty(normalizedFreshness) &&
            normalizedFreshness != "all")
        {
            terms.Add(Predicate(
                "freshness",
                "equals",
                normalizedFreshness));
        }

        return new FleetQuery
        {
            QueryId = $"fleet-console-{Guid.NewGuid():N}",
            Expression = terms.Count switch
            {
                0 => null,
                1 => terms[0],
                _ => new Dictionary<string, object?>
                {
                    ["kind"] = "and",
                    ["expressions"] = terms
                }
            },
            Sort =
            [
                new FleetSortKey
                {
                    Field = sortField,
                    Direction = sortDirection
                }
            ],
            Limit = limit
        };
    }

    private static Dictionary<string, object?> Predicate(
        string field,
        string comparison,
        string value) => new()
    {
        ["kind"] = "predicate",
        ["field"] = field,
        ["comparison"] = comparison,
        ["value"] = value,
        ["qualifier"] = null
    };
}

public sealed class FleetSortKey
{
    [JsonPropertyName("field")]
    public string Field { get; init; } = string.Empty;

    [JsonPropertyName("direction")]
    public string Direction { get; init; } = string.Empty;

    [JsonPropertyName("qualifier")]
    public string? Qualifier { get; init; }
}

public sealed class FleetQueryResult
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonPropertyName("query")]
    public FleetQuery Query { get; init; } = new();

    [JsonPropertyName("result_revision")]
    public ulong ResultRevision { get; init; }

    [JsonPropertyName("as_of_ms")]
    public long AsOfMs { get; init; }

    [JsonPropertyName("total_count")]
    public int TotalCount { get; init; }

    [JsonPropertyName("window_offset")]
    public int WindowOffset { get; init; }

    [JsonPropertyName("window_count")]
    public int WindowCount { get; init; }

    [JsonPropertyName("rows")]
    public IReadOnlyList<DeviceRowProjection> Rows { get; init; } = [];
}

public sealed class NavigationRestoration
{
    [JsonPropertyName("selected_device_id")]
    public string? SelectedDeviceId { get; init; }

    [JsonPropertyName("inspector_tab")]
    public string? InspectorTab { get; init; }

    [JsonPropertyName("scroll_anchor_device_id")]
    public string? ScrollAnchorDeviceId { get; init; }

    [JsonPropertyName("focused_region")]
    public string? FocusedRegion { get; init; }

    [JsonPropertyName("collapsed_groups")]
    public IReadOnlyList<string> CollapsedGroups { get; init; } = [];
}

public sealed class SavedView
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = "rusty.fleet.saved_view.v1";

    [JsonPropertyName("view_id")]
    public string ViewId { get; init; } = string.Empty;

    [JsonPropertyName("name")]
    public string Name { get; init; } = string.Empty;

    [JsonPropertyName("query")]
    public FleetQuery Query { get; init; } = new();

    [JsonPropertyName("columns")]
    public IReadOnlyList<string> Columns { get; init; } = [];

    [JsonPropertyName("density")]
    public string Density { get; init; } = "standard";

    [JsonPropertyName("grouping")]
    public string? Grouping { get; init; }

    [JsonPropertyName("restoration")]
    public NavigationRestoration Restoration { get; init; } = new();

    [JsonPropertyName("schema_version")]
    public uint SchemaVersion { get; init; } = 1;

    [JsonIgnore]
    public string DisplayName => Name;
}

public sealed class SavedViewCollection
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonPropertyName("revision")]
    public ulong Revision { get; init; }

    [JsonPropertyName("views")]
    public IReadOnlyList<SavedView> Views { get; init; } = [];
}

public sealed class SavedViewMutationRequest
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } =
        "rusty.fleet.saved_view_mutation_request.v1";

    [JsonPropertyName("expected_revision")]
    public ulong ExpectedRevision { get; init; }

    [JsonPropertyName("view")]
    public SavedView View { get; init; } = new();
}

public sealed class SavedViewMutationReceipt
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonPropertyName("view_id")]
    public string ViewId { get; init; } = string.Empty;

    [JsonPropertyName("previous_revision")]
    public ulong PreviousRevision { get; init; }

    [JsonPropertyName("current_revision")]
    public ulong CurrentRevision { get; init; }

    [JsonPropertyName("changed")]
    public bool Changed { get; init; }

    [JsonPropertyName("deleted")]
    public bool Deleted { get; init; }

    [JsonPropertyName("view")]
    public SavedView? View { get; init; }
}

public sealed class FleetSummaryProjection
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonPropertyName("as_of_ms")]
    public long AsOfMs { get; init; }

    [JsonPropertyName("total")]
    public int Total { get; init; }

    [JsonPropertyName("fresh")]
    public int Fresh { get; init; }

    [JsonPropertyName("stale")]
    public int Stale { get; init; }

    [JsonPropertyName("offline")]
    public int Offline { get; init; }

    [JsonPropertyName("attention")]
    public int Attention { get; init; }

    [JsonPropertyName("active_work")]
    public int ActiveWork { get; init; }
}

public sealed class DeviceRowProjection
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonPropertyName("identity")]
    public DeviceIdentity Identity { get; init; } = new();

    [JsonPropertyName("source_epoch")]
    public string SourceEpoch { get; init; } = string.Empty;

    [JsonPropertyName("accepted_revision")]
    public ulong AcceptedRevision { get; init; }

    [JsonPropertyName("accepted_at_ms")]
    public long AcceptedAtMs { get; init; }

    [JsonPropertyName("age_ms")]
    public long AgeMs { get; init; }

    [JsonPropertyName("freshness")]
    public string Freshness { get; init; } = "unknown";

    [JsonPropertyName("battery_percent")]
    public int? BatteryPercent { get; init; }

    [JsonPropertyName("charging")]
    public bool? Charging { get; init; }

    [JsonPropertyName("foreground_app")]
    public string? ForegroundApp { get; init; }

    [JsonPropertyName("agent")]
    public ApplicationObservation? Agent { get; init; }

    [JsonPropertyName("power")]
    public PowerObservation? Power { get; init; }

    [JsonPropertyName("application")]
    public ApplicationObservation? Application { get; init; }

    [JsonPropertyName("kiosk_state")]
    public string KioskState { get; init; } = "unknown";

    [JsonPropertyName("route")]
    public string Route { get; init; } = "unknown";

    [JsonPropertyName("conditions")]
    public IReadOnlyDictionary<string, StatusCondition> Conditions { get; init; } =
        new Dictionary<string, StatusCondition>();

    [JsonPropertyName("capabilities")]
    public CapabilitySnapshot Capabilities { get; init; } = new();

    [JsonPropertyName("stream_count")]
    public int StreamCount { get; init; }

    [JsonPropertyName("active_work_count")]
    public int ActiveWorkCount { get; init; }
}

public sealed class DeviceIdentity
{
    [JsonPropertyName("device_id")]
    public string DeviceId { get; init; } = string.Empty;

    [JsonPropertyName("identity_revision")]
    public ulong IdentityRevision { get; init; }

    [JsonPropertyName("display_name")]
    public string DisplayName { get; init; } = string.Empty;

    [JsonPropertyName("model")]
    public string Model { get; init; } = string.Empty;

    [JsonPropertyName("hardware_class")]
    public string HardwareClass { get; init; } = string.Empty;

    [JsonPropertyName("tags")]
    public IReadOnlyDictionary<string, string> Tags { get; init; } =
        new Dictionary<string, string>();
}

public sealed class ApplicationObservation
{
    [JsonPropertyName("package_name")]
    public string? PackageName { get; init; }

    [JsonPropertyName("lifecycle")]
    public string Lifecycle { get; init; } = "unknown";

    [JsonPropertyName("foreground_state")]
    public string ForegroundState { get; init; } = "unknown";

    [JsonPropertyName("foreground_authority")]
    public string ForegroundAuthority { get; init; } = "unknown";

    [JsonPropertyName("provenance")]
    public FactProvenance Provenance { get; init; } = new();
}

public sealed class PowerObservation
{
    [JsonPropertyName("battery_percent")]
    public int BatteryPercent { get; init; }

    [JsonPropertyName("charging")]
    public bool Charging { get; init; }

    [JsonPropertyName("provenance")]
    public FactProvenance Provenance { get; init; } = new();
}

public sealed class FactProvenance
{
    [JsonPropertyName("owner")]
    public string Owner { get; init; } = string.Empty;

    [JsonPropertyName("adapter_id")]
    public string AdapterId { get; init; } = string.Empty;

    [JsonPropertyName("observed_at_ms")]
    public long ObservedAtMs { get; init; }

    [JsonPropertyName("fresh_until_ms")]
    public long FreshUntilMs { get; init; }
}

public sealed class CapabilitySnapshot
{
    [JsonPropertyName("capabilities")]
    public IReadOnlyDictionary<string, CapabilityState> Capabilities { get; init; } =
        new Dictionary<string, CapabilityState>();
}

public sealed class CapabilityState
{
    [JsonPropertyName("capability_id")]
    public string CapabilityId { get; init; } = string.Empty;

    [JsonPropertyName("support")]
    public string Support { get; init; } = "unknown";

    [JsonPropertyName("enablement")]
    public string Enablement { get; init; } = "unknown";

    [JsonPropertyName("authorization")]
    public string Authorization { get; init; } = "unknown";

    [JsonPropertyName("reachability")]
    public string Reachability { get; init; } = "unknown";

    [JsonPropertyName("freshness")]
    public string Freshness { get; init; } = "unknown";

    [JsonPropertyName("evidence_revision")]
    public ulong EvidenceRevision { get; init; }

    [JsonPropertyName("observed_at_ms")]
    public long ObservedAtMs { get; init; }

    [JsonPropertyName("fresh_until_ms")]
    public long FreshUntilMs { get; init; }

    [JsonPropertyName("owner")]
    public string Owner { get; init; } = string.Empty;

    [JsonPropertyName("reason")]
    public string Reason { get; init; } = string.Empty;
}

public sealed class StatusCondition
{
    [JsonPropertyName("family")]
    public string Family { get; init; } = string.Empty;

    [JsonPropertyName("state")]
    public string State { get; init; } = "unknown";

    [JsonPropertyName("reason")]
    public string Reason { get; init; } = string.Empty;

    [JsonPropertyName("message")]
    public string Message { get; init; } = string.Empty;

    [JsonPropertyName("source_time_ms")]
    public long SourceTimeMs { get; init; }

    [JsonPropertyName("received_time_ms")]
    public long ReceivedTimeMs { get; init; }

    [JsonPropertyName("accepted_revision")]
    public ulong AcceptedRevision { get; init; }

    [JsonPropertyName("fresh_until_ms")]
    public long FreshUntilMs { get; init; }

    [JsonPropertyName("source")]
    public StatusSource Source { get; init; } = new();

    [JsonPropertyName("sensitivity")]
    public string Sensitivity { get; init; } = "operator";
}

public sealed class StatusSource
{
    [JsonPropertyName("adapter_id")]
    public string AdapterId { get; init; } = string.Empty;

    [JsonPropertyName("owner")]
    public string Owner { get; init; } = string.Empty;

    [JsonPropertyName("authority_revision")]
    public ulong AuthorityRevision { get; init; }
}

public sealed class DeviceInspectorProjection
{
    [JsonPropertyName("schema")]
    public string Schema { get; init; } = string.Empty;

    [JsonPropertyName("row")]
    public DeviceRowProjection Row { get; init; } = new();

    [JsonPropertyName("attention")]
    public IReadOnlyList<StatusCondition> Attention { get; init; } = [];

    [JsonPropertyName("streams")]
    public IReadOnlyList<JsonElement> Streams { get; init; } = [];

    [JsonPropertyName("active_operations")]
    public IReadOnlyList<JsonElement> ActiveOperations { get; init; } = [];
}
