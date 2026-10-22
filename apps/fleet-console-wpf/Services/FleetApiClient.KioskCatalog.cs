// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later
using System.Net.Http.Json;
using RustyFleet.FleetConsole.Contracts;
namespace RustyFleet.FleetConsole.Services;
public sealed partial class FleetApiClient
{
    public async Task<KioskAppSearchResult> SearchKioskAppsAsync(KioskAppSearchRequest request, CancellationToken cancellationToken)
    {
        using var response = await _http.PostAsJsonAsync("/fleet/v1/kiosk-apps/search", request, FleetJson.Options, cancellationToken);
        var result = await ReadAsync<KioskAppSearchResult>(response, cancellationToken);
        KioskCatalogValidation.Search(result, request.Query);
        return result;
    }
    public async Task<KioskAppCandidatesResult> KioskAppCandidatesAsync(string identity, CancellationToken cancellationToken)
    {
        using var response = await _http.GetAsync($"/fleet/v1/kiosk-apps/{Uri.EscapeDataString(identity)}/candidates", cancellationToken);
        var result = await ReadAsync<KioskAppCandidatesResult>(response, cancellationToken);
        KioskCatalogValidation.Candidates(result, identity);
        return result;
    }
    public async Task<KioskCatalogRefreshResult> RefreshKioskCatalogAsync(KioskCatalogRefreshRequest request, CancellationToken cancellationToken)
    {
        using var response = await _hotspotExecuteHttp.PostAsJsonAsync("/fleet/v1/kiosk-catalog/refresh", request, FleetJson.Options, cancellationToken);
        var result = await ReadAsync<KioskCatalogRefreshResult>(response, cancellationToken);
        KioskCatalogValidation.Refresh(result, request);
        return result;
    }
}
