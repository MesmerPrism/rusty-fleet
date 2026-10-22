// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later
using System.Text.Json.Serialization;
namespace RustyFleet.FleetConsole.Contracts;

[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed class KioskAppSearchRequest
{
    [JsonPropertyName("schema")] public string Schema { get; init; } = "rusty.fleet.kiosk_app_search_request.v1";
    [JsonPropertyName("query")] public string Query { get; init; } = "";
    [JsonPropertyName("limit")] public int Limit { get; init; } = 128;
}
public sealed class KioskCatalogCoverage
{
    [JsonPropertyName("current")] public int Current { get; init; }
    [JsonPropertyName("stale")] public int Stale { get; init; }
    [JsonPropertyName("absent")] public int Absent { get; init; }
    [JsonPropertyName("incomplete")] public int Incomplete { get; init; }
    [JsonPropertyName("permission_limited")] public int PermissionLimited { get; init; }
}
public sealed class KioskAppMatch
{
    [JsonPropertyName("app_identity")] public string AppIdentity { get; init; } = "";
    [JsonPropertyName("display_label")] public string DisplayLabel { get; init; } = "";
    [JsonPropertyName("disambiguator")] public string Disambiguator { get; init; } = "";
    [JsonPropertyName("package_name")] public string? PackageName { get; init; }
    [JsonPropertyName("matched_tags")] public IReadOnlyList<string> MatchedTags { get; init; } = [];
    [JsonPropertyName("current_device_count")] public int CurrentDeviceCount { get; init; }
    [JsonPropertyName("refresh_required_device_count")] public int RefreshRequiredDeviceCount { get; init; }
    public string Summary => $"{DisplayLabel} · {Disambiguator} · {CurrentDeviceCount} current";
}
public sealed class KioskAppSearchResult
{
    [JsonPropertyName("schema")] public string Schema { get; init; } = "";
    [JsonPropertyName("query")] public string Query { get; init; } = "";
    [JsonPropertyName("normalized_query")] public string NormalizedQuery { get; init; } = "";
    [JsonPropertyName("as_of_ms")] public long AsOfMs { get; init; }
    [JsonPropertyName("coverage")] public KioskCatalogCoverage Coverage { get; init; } = new();
    [JsonPropertyName("matches")] public IReadOnlyList<KioskAppMatch> Matches { get; init; } = [];
}
public sealed class KioskAppCandidate
{
    [JsonPropertyName("app_identity")] public string AppIdentity { get; init; } = "";
    [JsonPropertyName("display_label")] public string DisplayLabel { get; init; } = "";
    [JsonPropertyName("disambiguator")] public string Disambiguator { get; init; } = "";
    [JsonPropertyName("catalog_age_ms")] public long CatalogAgeMs { get; init; }
    [JsonPropertyName("catalog_freshness")] public string CatalogFreshness { get; init; } = "";
    [JsonPropertyName("catalog_complete")] public bool CatalogComplete { get; init; }
    [JsonPropertyName("grant_expires_at_ms")] public long? GrantExpiresAtMs { get; init; }
    [JsonPropertyName("row")] public DeviceRowProjection Row { get; init; } = new();
    public string Summary => $"{Row.Identity.DisplayName} · catalogue {CatalogFreshness}, {(CatalogComplete ? "complete" : "incomplete")} · device {Row.Freshness} · battery {Row.BatteryPercent?.ToString() ?? "unknown"}% · route {Row.Route} · grant admission expiry {(GrantExpiresAtMs is { } expiry ? DateTimeOffset.FromUnixTimeMilliseconds(expiry).ToString("u") : "absent")}";
}
public sealed class KioskAppCandidatesResult
{
    [JsonPropertyName("schema")] public string Schema { get; init; } = "";
    [JsonPropertyName("app_identity")] public string AppIdentity { get; init; } = "";
    [JsonPropertyName("as_of_ms")] public long AsOfMs { get; init; }
    [JsonPropertyName("candidates")] public IReadOnlyList<KioskAppCandidate> Candidates { get; init; } = [];
}
[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed class KioskCatalogRefreshRequest
{
    [JsonPropertyName("schema")] public string Schema { get; init; } = "rusty.fleet.kiosk_v2_catalog_refresh_request.v1";
    [JsonPropertyName("device_id")] public string DeviceId { get; init; } = "";
    [JsonPropertyName("identity_revision")] public ulong IdentityRevision { get; init; }
}
public sealed class KioskCatalogRefreshResult
{
    [JsonPropertyName("schema")] public string Schema { get; init; } = "";
    [JsonPropertyName("admitted")] public bool Admitted { get; init; }
    [JsonPropertyName("device_id")] public string DeviceId { get; init; } = "";
    [JsonPropertyName("identity_revision")] public ulong IdentityRevision { get; init; }
    [JsonPropertyName("entry_count")] public int EntryCount { get; init; }
    [JsonPropertyName("fresh_until_ms")] public long FreshUntilMs { get; init; }
    [JsonPropertyName("scopes")] public IReadOnlyList<string> Scopes { get; init; } = [];
}
public static class KioskCatalogValidation
{
    public static void Search(KioskAppSearchResult result, string query)
    {
        if (result.Schema != "rusty.fleet.kiosk_app_search_result.v1" || result.Query != query || result.AsOfMs <= 0 || result.Matches.Count > 128 ||
            result.Matches.Any(item => string.IsNullOrWhiteSpace(item.AppIdentity) || item.AppIdentity.Length > 160 || item.PackageName is not null || item.CurrentDeviceCount <= 0) ||
            result.Matches.Select(item => item.AppIdentity).Distinct(StringComparer.Ordinal).Count() != result.Matches.Count ||
            new[] { result.Coverage.Current, result.Coverage.Stale, result.Coverage.Absent, result.Coverage.Incomplete, result.Coverage.PermissionLimited }.Any(count => count < 0))
            throw new InvalidOperationException("Catalogue search response does not bind the exact bounded query.");
    }
    public static void Candidates(KioskAppCandidatesResult result, string identity)
    {
        if (result.Schema != "rusty.fleet.kiosk_app_candidates_result.v1" || result.AppIdentity != identity || result.AsOfMs <= 0 || result.Candidates.Count > 1000 || result.Candidates.Any(item => item.AppIdentity != identity || item.CatalogAgeMs < 0 || item.CatalogFreshness is not ("fresh" or "stale")))
            throw new InvalidOperationException("Catalogue candidates do not bind the exact app identity.");
        foreach (var candidate in result.Candidates)
            FleetProjectionValidation.ValidateInspector(new DeviceInspectorProjection { Schema = "rusty.fleet.device_inspector.v1", Row = candidate.Row }, candidate.Row);
    }
    public static void Refresh(KioskCatalogRefreshResult result, KioskCatalogRefreshRequest request)
    {
        if (result.Schema != "rusty.fleet.kiosk_v2_catalog_refresh_projection.v1" || !result.Admitted || result.DeviceId != request.DeviceId || result.IdentityRevision != request.IdentityRevision || result.EntryCount is < 0 or > 500 || result.FreshUntilMs <= 0 || !result.Scopes.SequenceEqual(new[] { "kiosk.catalog-summary" }))
            throw new InvalidOperationException("Catalogue refresh does not bind the exact summary read.");
    }
}
