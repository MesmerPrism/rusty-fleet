// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later
using System.Collections.ObjectModel;
using RustyFleet.FleetConsole.Contracts;
namespace RustyFleet.FleetConsole.ViewModels;
public sealed partial class FleetWorkspaceViewModel
{
    private string _kioskAppSearchText = "";
    private string _kioskCatalogStatus = "No Kiosk catalogue read. Coverage is unknown.";
    private KioskAppMatch? _selectedKioskAppMatch;
    public ObservableCollection<KioskAppMatch> KioskAppMatches { get; } = [];
    public ObservableCollection<KioskAppCandidate> KioskAppCandidates { get; } = [];
    public AsyncCommand SearchKioskAppsCommand { get; private set; } = null!;
    public AsyncCommand ShowKioskAppCandidatesCommand { get; private set; } = null!;
    public AsyncCommand RefreshKioskCatalogCommand { get; private set; } = null!;
    public string KioskAppSearchText { get => _kioskAppSearchText; set { if (SetProperty(ref _kioskAppSearchText, value)) RaiseKioskCatalogCommands(); } }
    public string KioskCatalogStatus { get => _kioskCatalogStatus; private set => SetProperty(ref _kioskCatalogStatus, value); }
    public KioskAppMatch? SelectedKioskAppMatch { get => _selectedKioskAppMatch; set { if (SetProperty(ref _selectedKioskAppMatch, value)) RaiseKioskCatalogCommands(); } }
    private void InitializeKioskCatalog()
    {
        SearchKioskAppsCommand = new AsyncCommand(SearchKioskAppsAsync, () => !IsBusy && _source is not null && !string.IsNullOrWhiteSpace(KioskAppSearchText));
        ShowKioskAppCandidatesCommand = new AsyncCommand(ShowKioskAppCandidatesAsync, () => !IsBusy && _source is not null && SelectedKioskAppMatch is not null);
        RefreshKioskCatalogCommand = new AsyncCommand(RefreshKioskCatalogAsync, () => !IsBusy && _source is not null && SelectedDevice is not null);
    }
    private void RaiseKioskCatalogCommands()
    {
        SearchKioskAppsCommand?.RaiseCanExecuteChanged();
        ShowKioskAppCandidatesCommand?.RaiseCanExecuteChanged();
        RefreshKioskCatalogCommand?.RaiseCanExecuteChanged();
    }
    public async Task SearchKioskAppsAsync()
    {
        if (_source is null || IsBusy) return;
        var query = KioskAppSearchText;
        IsBusy = true;
        try
        {
            using var cancellation = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            var result = await _source.SearchKioskAppsAsync(new KioskAppSearchRequest { Query = query }, cancellation.Token);
            KioskCatalogValidation.Search(result, query);
            var selectedIdentity = SelectedKioskAppMatch?.AppIdentity;
            KioskAppMatches.Clear();
            foreach (var item in result.Matches) KioskAppMatches.Add(item);
            SelectedKioskAppMatch = KioskAppMatches.FirstOrDefault(item => item.AppIdentity == selectedIdentity);
            KioskAppCandidates.Clear();
            var coverage = result.Coverage;
            KioskCatalogStatus = $"{result.Matches.Count} exact app identities as of {DateTimeOffset.FromUnixTimeMilliseconds(result.AsOfMs):u} · catalogue coverage: {coverage.Current} current, {coverage.Stale} stale, {coverage.Absent} absent, {coverage.Incomplete} incomplete, {coverage.PermissionLimited} permission limited";
        }
        catch (Exception error) when (IsProjectionFailure(error) || error is NotSupportedException)
        {
            KioskAppMatches.Clear(); KioskAppCandidates.Clear(); SelectedKioskAppMatch = null;
            KioskCatalogStatus = $"Search unavailable · no current matches claimed · {error.Message}";
        }
        finally { IsBusy = false; }
    }
    public async Task ShowKioskAppCandidatesAsync()
    {
        if (_source is null || SelectedKioskAppMatch is not { } selected || IsBusy) return;
        IsBusy = true;
        try
        {
            using var cancellation = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            var result = await _source.KioskAppCandidatesAsync(selected.AppIdentity, cancellation.Token);
            KioskCatalogValidation.Candidates(result, selected.AppIdentity);
            KioskAppCandidates.Clear();
            foreach (var item in result.Candidates) KioskAppCandidates.Add(item);
            KioskCatalogStatus = $"{result.Candidates.Count} candidate devices as of {DateTimeOffset.FromUnixTimeMilliseconds(result.AsOfMs):u} · catalogue observations do not establish wearer availability";
        }
        catch (Exception error) when (IsProjectionFailure(error) || error is NotSupportedException)
        { KioskAppCandidates.Clear(); KioskCatalogStatus = $"Candidate read unavailable · {error.Message}"; }
        finally { IsBusy = false; }
    }
    public async Task RefreshKioskCatalogAsync()
    {
        if (_source is null || SelectedDevice is not { } selected || IsBusy) return;
        var request = new KioskCatalogRefreshRequest { DeviceId = selected.DeviceId, IdentityRevision = selected.Projection.Identity.IdentityRevision };
        IsBusy = true;
        try
        {
            using var cancellation = new CancellationTokenSource(TimeSpan.FromSeconds(35));
            var result = await _source.RefreshKioskCatalogAsync(request, cancellation.Token);
            KioskCatalogValidation.Refresh(result, request);
            KioskCatalogStatus = $"Summary read admitted for {result.DeviceId} · {result.EntryCount} entries · search to update matches";
        }
        catch (Exception error) when (IsProjectionFailure(error) || error is NotSupportedException)
        { KioskCatalogStatus = $"Catalogue refresh refused · {error.Message}"; }
        finally { IsBusy = false; }
    }
}
