// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Net.Http;
using System.Net.Http.Json;
using System.Text.Json;
using RustyFleet.FleetConsole.Contracts;

namespace RustyFleet.FleetConsole.Services;

public interface IFleetDataSource
{
    Task<FleetQueryResult> QueryAsync(FleetQuery query, CancellationToken cancellationToken);

    Task<FleetSummaryProjection> SummaryAsync(CancellationToken cancellationToken);

    Task<DeviceInspectorProjection> InspectAsync(
        string deviceId,
        CancellationToken cancellationToken);

    Task<DeviceDetailProjection> DetailAsync(
        string deviceId,
        CancellationToken cancellationToken);

    Task<IReadOnlyList<FleetWatchEvent>> WatchAsync(
        ulong afterSequence,
        int limit,
        CancellationToken cancellationToken);

    Task<SavedViewCollection> SavedViewsAsync(CancellationToken cancellationToken);

    Task<SavedViewMutationReceipt> UpsertSavedViewAsync(
        SavedViewMutationRequest request,
        CancellationToken cancellationToken);

    Task<SavedViewMutationReceipt> DeleteSavedViewAsync(
        string viewId,
        ulong expectedRevision,
        CancellationToken cancellationToken);
}

public sealed class FleetApiClient : IFleetDataSource, IDisposable
{
    public const long MaxResponseBytes = 16 * 1024 * 1024;

    private readonly HttpClient _http;

    public FleetApiClient(Uri baseAddress)
    {
        if (!baseAddress.IsAbsoluteUri ||
            baseAddress.Scheme != Uri.UriSchemeHttp ||
            !baseAddress.IsLoopback)
        {
            throw new ArgumentException(
                "The M1 Fleet Console accepts an absolute loopback HTTP Hub address.",
                nameof(baseAddress));
        }

        _http = new HttpClient
        {
            BaseAddress = baseAddress,
            Timeout = TimeSpan.FromSeconds(10),
            MaxResponseContentBufferSize = MaxResponseBytes
        };
    }

    public async Task<FleetQueryResult> QueryAsync(
        FleetQuery query,
        CancellationToken cancellationToken)
    {
        using var response = await _http.PostAsJsonAsync(
            "/fleet/v1/query",
            query,
            FleetJson.Options,
            cancellationToken);
        return await ReadAsync<FleetQueryResult>(response, cancellationToken);
    }

    public async Task<FleetSummaryProjection> SummaryAsync(CancellationToken cancellationToken)
    {
        using var response = await _http.GetAsync("/fleet/v1/summary", cancellationToken);
        return await ReadAsync<FleetSummaryProjection>(response, cancellationToken);
    }

    public async Task<DeviceInspectorProjection> InspectAsync(
        string deviceId,
        CancellationToken cancellationToken)
    {
        var encoded = Uri.EscapeDataString(deviceId);
        using var response = await _http.GetAsync(
            $"/fleet/v1/devices/{encoded}/inspect",
            cancellationToken);
        return await ReadAsync<DeviceInspectorProjection>(response, cancellationToken);
    }

    public async Task<DeviceDetailProjection> DetailAsync(
        string deviceId,
        CancellationToken cancellationToken)
    {
        var encoded = Uri.EscapeDataString(deviceId);
        using var response = await _http.GetAsync(
            $"/fleet/v1/devices/{encoded}",
            cancellationToken);
        return await ReadAsync<DeviceDetailProjection>(response, cancellationToken);
    }

    public async Task<IReadOnlyList<FleetWatchEvent>> WatchAsync(
        ulong afterSequence,
        int limit,
        CancellationToken cancellationToken)
    {
        if (limit is < 1 or > 10_000)
        {
            throw new ArgumentOutOfRangeException(
                nameof(limit),
                "Fleet watch limits must be between 1 and 10,000 events.");
        }

        using var response = await _http.GetAsync(
            $"/fleet/v1/watch?after_sequence={afterSequence}&limit={limit}",
            cancellationToken);
        return await ReadAsync<IReadOnlyList<FleetWatchEvent>>(
            response,
            cancellationToken);
    }

    public async Task<SavedViewCollection> SavedViewsAsync(
        CancellationToken cancellationToken)
    {
        using var response = await _http.GetAsync(
            "/fleet/v1/saved-views",
            cancellationToken);
        return await ReadAsync<SavedViewCollection>(response, cancellationToken);
    }

    public async Task<SavedViewMutationReceipt> UpsertSavedViewAsync(
        SavedViewMutationRequest request,
        CancellationToken cancellationToken)
    {
        var encoded = Uri.EscapeDataString(request.View.ViewId);
        using var response = await _http.PutAsJsonAsync(
            $"/fleet/v1/saved-views/{encoded}",
            request,
            FleetJson.Options,
            cancellationToken);
        return await ReadAsync<SavedViewMutationReceipt>(response, cancellationToken);
    }

    public async Task<SavedViewMutationReceipt> DeleteSavedViewAsync(
        string viewId,
        ulong expectedRevision,
        CancellationToken cancellationToken)
    {
        var encoded = Uri.EscapeDataString(viewId);
        using var response = await _http.DeleteAsync(
            $"/fleet/v1/saved-views/{encoded}?expected_revision={expectedRevision}",
            cancellationToken);
        return await ReadAsync<SavedViewMutationReceipt>(response, cancellationToken);
    }

    public void Dispose() => _http.Dispose();

    private static async Task<T> ReadAsync<T>(
        HttpResponseMessage response,
        CancellationToken cancellationToken)
    {
        response.EnsureSuccessStatusCode();
        await using var stream = await response.Content.ReadAsStreamAsync(cancellationToken);
        return await JsonSerializer.DeserializeAsync<T>(
            stream,
            FleetJson.Options,
            cancellationToken)
            ?? throw new JsonException($"Fleet Hub returned an empty {typeof(T).Name}.");
    }
}
