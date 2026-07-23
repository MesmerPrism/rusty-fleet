// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Text.Json;

namespace RustyFleet.FleetConsole.Contracts;

public static class FleetProjectionValidation
{
    private static readonly HashSet<string> FreshnessStates =
    [
        "fresh",
        "stale",
        "offline",
        "unknown"
    ];

    public static void ValidateQueryResult(
        FleetQueryResult result,
        FleetSummaryProjection summary,
        FleetQuery requestedQuery)
    {
        Require(result.Schema == "rusty.fleet.query_result.v1", "query-result schema");
        Require(result.Query.Schema == "rusty.fleet.query.v1", "query schema");
        Require(result.ResultRevision > 0, "result revision");
        Require(
            JsonSerializer.Serialize(result.Query, FleetJson.Options) ==
            JsonSerializer.Serialize(requestedQuery, FleetJson.Options),
            "query correlation");
        Require(result.WindowOffset >= 0, "window offset");
        Require(result.WindowCount == result.Rows.Count, "window count");
        Require(result.TotalCount >= result.WindowCount, "total count");
        Require(
            (long)result.WindowOffset + result.WindowCount <= result.TotalCount,
            "window bounds");
        Require(result.Rows.Count <= requestedQuery.Limit, "requested window limit");
        ValidateSummary(summary);

        var identities = new HashSet<string>(StringComparer.Ordinal);
        foreach (var row in result.Rows)
        {
            ValidateRow(row);
            Require(
                identities.Add(
                    $"{row.Identity.DeviceId}@{row.Identity.IdentityRevision}"),
                "duplicate device identity");
        }
    }

    public static void ValidateInspector(
        DeviceInspectorProjection projection,
        DeviceRowProjection expectedRow)
    {
        Require(
            projection.Schema == "rusty.fleet.device_inspector.v1",
            "inspector schema");
        ValidateRow(projection.Row);
        Require(
            projection.Row.Identity.DeviceId == expectedRow.Identity.DeviceId &&
            projection.Row.Identity.IdentityRevision ==
            expectedRow.Identity.IdentityRevision,
            "inspector device identity");
        Require(projection.Attention.Count <= 64, "inspector attention limit");
        Require(projection.Streams.Count <= 32, "inspector stream limit");
        Require(projection.ActiveOperations.Count <= 128, "inspector operation limit");
    }

    private static void ValidateSummary(FleetSummaryProjection summary)
    {
        Require(summary.Schema == "rusty.fleet.summary.v1", "summary schema");
        Require(
            summary.Total >= 0 &&
            summary.Fresh >= 0 &&
            summary.Stale >= 0 &&
            summary.Offline >= 0 &&
            summary.Attention >= 0 &&
            summary.ActiveWork >= 0,
            "summary nonnegative counts");
        Require(
            (long)summary.Fresh + summary.Stale + summary.Offline <= summary.Total,
            "summary freshness counts");
        Require(summary.Attention <= summary.Total, "summary attention count");
    }

    private static void ValidateRow(DeviceRowProjection row)
    {
        Require(row.Schema == "rusty.fleet.device_row.v1", "device-row schema");
        Require(!string.IsNullOrWhiteSpace(row.Identity.DeviceId), "device ID");
        Require(row.Identity.IdentityRevision > 0, "identity revision");
        Require(!string.IsNullOrWhiteSpace(row.Identity.DisplayName), "display name");
        Require(!string.IsNullOrWhiteSpace(row.Identity.Model), "device model");
        Require(!string.IsNullOrWhiteSpace(row.Identity.HardwareClass), "hardware class");
        Require(row.Identity.Tags.Count <= 128, "identity tag limit");
        Require(
            row.Identity.Tags.All(entry =>
                !string.IsNullOrWhiteSpace(entry.Key) &&
                !string.IsNullOrWhiteSpace(entry.Value)),
            "identity tag values");
        Require(!string.IsNullOrWhiteSpace(row.SourceEpoch), "source epoch");
        Require(row.AcceptedRevision > 0, "accepted revision");
        Require(row.AgeMs >= 0, "accepted age");
        Require(FreshnessStates.Contains(row.Freshness), "freshness state");
        Require(
            row.BatteryPercent is null or >= 0 and <= 100,
            "battery percentage");
        Require(!string.IsNullOrWhiteSpace(row.KioskState), "kiosk state");
        Require(!string.IsNullOrWhiteSpace(row.Route), "route state");
        Require(row.Conditions.Count <= 16, "condition limit");
        Require(row.Capabilities.Capabilities.Count <= 128, "capability limit");

        foreach (var (key, condition) in row.Conditions)
        {
            Require(key == condition.Family, "condition key");
            Require(!string.IsNullOrWhiteSpace(condition.Reason), "condition reason");
            Require(!string.IsNullOrWhiteSpace(condition.Message), "condition message");
            Require(!string.IsNullOrWhiteSpace(condition.Source.AdapterId), "condition adapter");
            Require(!string.IsNullOrWhiteSpace(condition.Source.Owner), "condition owner");
            Require(condition.AcceptedRevision > 0, "condition accepted revision");
            Require(condition.Source.AuthorityRevision > 0, "condition authority revision");
            Require(
                condition.FreshUntilMs >= condition.ReceivedTimeMs,
                "condition freshness");
        }

        foreach (var (key, capability) in row.Capabilities.Capabilities)
        {
            Require(key == capability.CapabilityId, "capability key");
            Require(!string.IsNullOrWhiteSpace(capability.Owner), "capability owner");
            Require(!string.IsNullOrWhiteSpace(capability.Reason), "capability reason");
            Require(capability.EvidenceRevision > 0, "capability evidence revision");
            Require(
                capability.FreshUntilMs >= capability.ObservedAtMs,
                "capability freshness");
        }
    }

    private static void Require(bool condition, string field)
    {
        if (!condition)
        {
            throw new InvalidOperationException(
                $"Fleet Hub returned invalid projection evidence: {field}.");
        }
    }
}
