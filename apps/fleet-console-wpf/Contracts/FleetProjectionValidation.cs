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

    public static void ValidateSavedViews(SavedViewCollection collection)
    {
        Require(
            collection.Schema == "rusty.fleet.saved_view_collection.v1",
            "saved-view collection schema");
        Require(collection.Revision > 0, "saved-view collection revision");
        Require(collection.Views.Count <= 128, "saved-view collection limit");
        string? priorId = null;
        foreach (var view in collection.Views)
        {
            ValidateSavedView(view);
            Require(
                priorId is null ||
                StringComparer.Ordinal.Compare(priorId, view.ViewId) < 0,
                "saved-view canonical ordering");
            priorId = view.ViewId;
        }
    }

    public static void ValidateSavedViewReceipt(SavedViewMutationReceipt receipt)
    {
        Require(
            receipt.Schema == "rusty.fleet.saved_view_mutation_receipt.v1",
            "saved-view receipt schema");
        Require(!string.IsNullOrWhiteSpace(receipt.ViewId), "saved-view receipt ID");
        Require(receipt.PreviousRevision > 0, "saved-view previous revision");
        Require(
            receipt.CurrentRevision >= receipt.PreviousRevision &&
            receipt.CurrentRevision - receipt.PreviousRevision <= 1,
            "saved-view receipt revision");
        Require(
            receipt.Changed ==
            (receipt.CurrentRevision > receipt.PreviousRevision),
            "saved-view changed revision");
        Require(receipt.Deleted == (receipt.View is null), "saved-view deletion receipt");
        Require(!receipt.Deleted || receipt.Changed, "saved-view deletion change");
        if (receipt.View is not null)
        {
            ValidateSavedView(receipt.View);
            Require(receipt.View.ViewId == receipt.ViewId, "saved-view receipt identity");
        }
    }

    private static void ValidateSavedView(SavedView view)
    {
        Require(view.Schema == "rusty.fleet.saved_view.v1", "saved-view schema");
        Require(
            IsValidSavedViewId(view.ViewId),
            "saved-view ID");
        Require(
            !string.IsNullOrWhiteSpace(view.Name) && view.Name.Length <= 256,
            "saved-view name");
        Require(view.Query.Schema == "rusty.fleet.query.v1", "saved-view query schema");
        Require(
            !string.IsNullOrWhiteSpace(view.Query.QueryId),
            "saved-view query ID");
        Require(
            view.Query.Limit is >= 1 and <= 10_000 &&
            view.Query.Sort.Count <= 8,
            "saved-view query bounds");
        Require(
            view.Columns.Count <= 64 &&
            view.Columns.All(column =>
                !string.IsNullOrWhiteSpace(column) && column.Length <= 128) &&
            view.Columns.Distinct(StringComparer.Ordinal).Count() == view.Columns.Count,
            "saved-view columns");
        Require(
            view.Density is "compact" or "standard" or "comfortable",
            "saved-view density");
        Require(
            view.Grouping is null ||
            (!string.IsNullOrWhiteSpace(view.Grouping) && view.Grouping.Length <= 128),
            "saved-view grouping");
        foreach (var value in new[]
                 {
                     view.Restoration.SelectedDeviceId,
                     view.Restoration.InspectorTab,
                     view.Restoration.ScrollAnchorDeviceId,
                     view.Restoration.FocusedRegion
                 })
        {
            Require(
                value is null ||
                (!string.IsNullOrWhiteSpace(value) && value.Length <= 128),
                "saved-view restoration text");
        }
        Require(view.SchemaVersion > 0, "saved-view schema version");
        Require(
            view.Restoration.CollapsedGroups.Count <= 512 &&
            view.Restoration.CollapsedGroups.All(group =>
                !string.IsNullOrWhiteSpace(group) && group.Length <= 128) &&
            view.Restoration.CollapsedGroups
                .Distinct(StringComparer.Ordinal).Count() ==
            view.Restoration.CollapsedGroups.Count,
            "saved-view collapsed groups");
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

    private static bool IsValidSavedViewId(string value)
    {
        if (value.Length is 0 or > 128)
        {
            return false;
        }

        return value.Split('.').All(segment =>
            segment.Length > 0 &&
            IsSavedViewIdEdge(segment[0]) &&
            IsSavedViewIdEdge(segment[^1]) &&
            segment.All(character =>
                IsSavedViewIdEdge(character) || character is '_' or '-'));
    }

    private static bool IsSavedViewIdEdge(char value) =>
        char.IsAsciiLetterLower(value) || char.IsAsciiDigit(value);
}
