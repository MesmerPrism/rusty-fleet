// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Net.Http;
using System.Text.Json;
using System.Windows.Data;
using RustyFleet.FleetConsole.Contracts;
using RustyFleet.FleetConsole.Services;

namespace RustyFleet.FleetConsole.ViewModels;

public sealed class FleetWorkspaceViewModel : ObservableObject
{
    private readonly Func<Uri, IFleetDataSource>? _sourceFactory;
    private readonly Dictionary<string, ulong> _batchSelection = new(StringComparer.Ordinal);
    private IFleetDataSource? _source;
    private string _hubAddress = "http://127.0.0.1:8741/";
    private string _searchText = string.Empty;
    private string _selectedFreshness = "All";
    private string _selectedGrouping = "None";
    private string _appliedSearchText = string.Empty;
    private string _appliedFreshness = "All";
    private string _appliedGrouping = "None";
    private string _statusMessage = "Disconnected · enter a local Hub address and connect";
    private string _summaryText = "No fleet data loaded";
    private string _scopeText = "0 devices";
    private string _asOfText = "No accepted snapshot";
    private string _activeScopeText = "Active scope · all devices · grouped by none";
    private string _inspectorContextText = "No selected device";
    private bool _isBusy;
    private DeviceRowViewModel? _selectedDevice;
    private DeviceInspectorViewModel? _inspector;
    private string? _inspectedStableKey;
    private CancellationTokenSource? _requestCancellation;

    public FleetWorkspaceViewModel(Func<Uri, IFleetDataSource> sourceFactory)
    {
        _sourceFactory = sourceFactory;
        RowsView = CollectionViewSource.GetDefaultView(Rows);
        ConnectCommand = new AsyncCommand(ConnectAsync, () => !IsBusy);
        RefreshCommand = new AsyncCommand(RefreshAsync, () => !IsBusy && _source is not null);
        ApplySearchCommand = new AsyncCommand(ApplyScopeAsync, () => !IsBusy && _source is not null);
        ClearSearchCommand = new AsyncCommand(ClearSearchAsync, () => !IsBusy);
        ClearBatchSelectionCommand = new RelayCommand(ClearBatchSelection);
        SelectAllVisibleCommand = new RelayCommand(SelectAllVisible);
    }

    public FleetWorkspaceViewModel(IFleetDataSource source)
        : this(_ => source)
    {
        _source = source;
        StatusMessage = "Test data source ready";
    }

    public ObservableCollection<DeviceRowViewModel> Rows { get; } = [];

    public ICollectionView RowsView { get; }

    public IReadOnlyList<string> FreshnessOptions { get; } =
        ["All", "Fresh", "Stale", "Offline", "Unknown"];

    public IReadOnlyList<string> GroupingOptions { get; } =
        ["None", "Cohort", "Model", "Freshness", "Application"];

    public AsyncCommand ConnectCommand { get; }

    public AsyncCommand RefreshCommand { get; }

    public AsyncCommand ApplySearchCommand { get; }

    public AsyncCommand ClearSearchCommand { get; }

    public RelayCommand ClearBatchSelectionCommand { get; }

    public RelayCommand SelectAllVisibleCommand { get; }

    public string HubAddress
    {
        get => _hubAddress;
        set => SetProperty(ref _hubAddress, value);
    }

    public string SearchText
    {
        get => _searchText;
        set => SetProperty(ref _searchText, value);
    }

    public string SelectedFreshness
    {
        get => _selectedFreshness;
        set => SetProperty(ref _selectedFreshness, value);
    }

    public string SelectedGrouping
    {
        get => _selectedGrouping;
        set => SetProperty(ref _selectedGrouping, value);
    }

    public string StatusMessage
    {
        get => _statusMessage;
        private set => SetProperty(ref _statusMessage, value);
    }

    public string SummaryText
    {
        get => _summaryText;
        private set => SetProperty(ref _summaryText, value);
    }

    public string ScopeText
    {
        get => _scopeText;
        private set => SetProperty(ref _scopeText, value);
    }

    public string AsOfText
    {
        get => _asOfText;
        private set => SetProperty(ref _asOfText, value);
    }

    public string ActiveScopeText
    {
        get => _activeScopeText;
        private set => SetProperty(ref _activeScopeText, value);
    }

    public string InspectorContextText
    {
        get => _inspectorContextText;
        private set => SetProperty(ref _inspectorContextText, value);
    }

    public bool IsBusy
    {
        get => _isBusy;
        private set
        {
            if (SetProperty(ref _isBusy, value))
            {
                ConnectCommand.RaiseCanExecuteChanged();
                RefreshCommand.RaiseCanExecuteChanged();
                ApplySearchCommand.RaiseCanExecuteChanged();
                ClearSearchCommand.RaiseCanExecuteChanged();
            }
        }
    }

    public DeviceRowViewModel? SelectedDevice
    {
        get => _selectedDevice;
        private set => SetProperty(ref _selectedDevice, value);
    }

    public DeviceInspectorViewModel? Inspector
    {
        get => _inspector;
        private set => SetProperty(ref _inspector, value);
    }

    public string BatchSelectionText
    {
        get
        {
            var visibleSelected = Rows.Count(row => row.IsBatchSelected);
            var hiddenSelected = _batchSelection.Count - visibleSelected;
            return hiddenSelected > 0
                ? $"{_batchSelection.Count} selected · {hiddenSelected} hidden by scope · " +
                  $"{Rows.Count} shown"
                : $"{_batchSelection.Count} selected · {Rows.Count} shown";
        }
    }

    public async Task InitializeAsync() => await RefreshAsync();

    public async Task SelectDeviceAsync(DeviceRowViewModel? device)
    {
        SelectedDevice = device;
        if (device is null)
        {
            _inspectedStableKey = null;
            Inspector = null;
            InspectorContextText = "No selected device";
            return;
        }

        _inspectedStableKey = device.StableKey;
        Inspector = DeviceInspectorViewModel.FromRow(device.Projection);
        InspectorContextText = "Selected device is in the active scope";
        if (_source is null)
        {
            return;
        }

        try
        {
            var projection = await _source.InspectAsync(
                device.DeviceId,
                CancellationToken.None);
            FleetProjectionValidation.ValidateInspector(
                projection,
                device.Projection);

            if (_inspectedStableKey == device.StableKey)
            {
                Inspector = new DeviceInspectorViewModel(projection);
            }
        }
        catch (Exception error) when (
            error is HttpRequestException or JsonException or TaskCanceledException or
            InvalidOperationException)
        {
            StatusMessage = $"Inspector retained cached row · {error.Message}";
        }
    }

    public void ToggleBatchSelection(DeviceRowViewModel? device)
    {
        if (device is null)
        {
            return;
        }

        device.IsBatchSelected = !device.IsBatchSelected;
    }

    public Task RefreshAsync() => LoadScopeAsync(
        _appliedSearchText,
        _appliedFreshness,
        _appliedGrouping,
        acceptEditorScope: false);

    public Task ApplyScopeAsync() => LoadScopeAsync(
        SearchText,
        SelectedFreshness,
        SelectedGrouping,
        acceptEditorScope: true);

    private async Task LoadScopeAsync(
        string searchText,
        string freshness,
        string grouping,
        bool acceptEditorScope)
    {
        if (_source is null)
        {
            StatusMessage = "Not connected to a Fleet Hub";
            return;
        }

        _requestCancellation?.Cancel();
        _requestCancellation?.Dispose();
        _requestCancellation = new CancellationTokenSource(TimeSpan.FromSeconds(12));
        IsBusy = true;
        StatusMessage = "Refreshing canonical fleet scope";
        try
        {
            var query = FleetQuery.Create(searchText, freshness);
            var queryTask = _source.QueryAsync(query, _requestCancellation.Token);
            var summaryTask = _source.SummaryAsync(_requestCancellation.Token);
            await Task.WhenAll(queryTask, summaryTask);
            var invalidatedSelections = ApplyResult(
                await queryTask,
                await summaryTask,
                query);
            if (acceptEditorScope)
            {
                _appliedSearchText = searchText.Trim();
                _appliedFreshness = NormalizeOption(freshness, "All");
                _appliedGrouping = NormalizeOption(grouping, "None");
                ApplyGrouping(_appliedGrouping);
                UpdateActiveScopeText();
            }
            else
            {
                RowsView.Refresh();
            }

            StatusMessage = invalidatedSelections == 0
                ? "Connected · ordering stable · no background re-sort"
                : $"Connected · {invalidatedSelections} batch selection invalidated by " +
                  "an identity revision";
        }
        catch (Exception error) when (
            error is HttpRequestException or JsonException or TaskCanceledException or
            InvalidOperationException)
        {
            StatusMessage = $"Refresh failed · cached rows retained · {error.Message}";
        }
        finally
        {
            IsBusy = false;
        }
    }

    private async Task ConnectAsync()
    {
        if (_sourceFactory is null ||
            !Uri.TryCreate(HubAddress, UriKind.Absolute, out var hub))
        {
            StatusMessage = "Enter a valid absolute local Hub address";
            return;
        }

        try
        {
            var replacement = _sourceFactory(hub);
            if (!ReferenceEquals(_source, replacement) && _source is IDisposable disposable)
            {
                disposable.Dispose();
            }

            _source = replacement;
            await RefreshAsync();
        }
        catch (ArgumentException error)
        {
            StatusMessage = error.Message;
        }
    }

    private int ApplyResult(
        FleetQueryResult result,
        FleetSummaryProjection summary,
        FleetQuery requestedQuery)
    {
        FleetProjectionValidation.ValidateQueryResult(
            result,
            summary,
            requestedQuery);

        var existing = Rows.ToDictionary(row => row.StableKey, StringComparer.Ordinal);
        var incomingKeys = new HashSet<string>(StringComparer.Ordinal);
        var invalidatedSelections = 0;

        for (var index = 0; index < result.Rows.Count; index++)
        {
            var projection = result.Rows[index];
            var key = $"{projection.Identity.DeviceId}@{projection.Identity.IdentityRevision}";
            if (_batchSelection.TryGetValue(
                    projection.Identity.DeviceId,
                    out var selectedRevision) &&
                selectedRevision != projection.Identity.IdentityRevision)
            {
                _batchSelection.Remove(projection.Identity.DeviceId);
                invalidatedSelections++;
            }

            incomingKeys.Add(key);
            if (existing.TryGetValue(key, out var row))
            {
                row.Update(projection);
                var currentIndex = Rows.IndexOf(row);
                if (currentIndex != index)
                {
                    Rows.Move(currentIndex, index);
                }
            }
            else
            {
                var newRow = new DeviceRowViewModel(projection);
                newRow.IsBatchSelected =
                    _batchSelection.TryGetValue(
                        projection.Identity.DeviceId,
                        out var batchRevision) &&
                    batchRevision == projection.Identity.IdentityRevision;
                newRow.PropertyChanged += OnRowPropertyChanged;
                Rows.Insert(index, newRow);
            }
        }

        for (var index = Rows.Count - 1; index >= 0; index--)
        {
            if (!incomingKeys.Contains(Rows[index].StableKey))
            {
                Rows[index].PropertyChanged -= OnRowPropertyChanged;
                Rows.RemoveAt(index);
            }
        }

        SelectedDevice = _inspectedStableKey is null
            ? null
            : Rows.FirstOrDefault(row => row.StableKey == _inspectedStableKey);
        if (Inspector is not null)
        {
            InspectorContextText = SelectedDevice is null
                ? "Selected device is outside the active scope · cached accepted evidence"
                : "Selected device is in the active scope";
        }

        SummaryText =
            $"{summary.Total:N0} devices · {summary.Fresh:N0} fresh · {summary.Stale:N0} stale · " +
            $"{summary.Offline:N0} offline · {summary.Attention:N0} attention · " +
            $"{summary.ActiveWork:N0} active work";
        ScopeText =
            $"{result.WindowCount:N0} shown · {result.TotalCount:N0} matching · " +
            $"result revision {result.ResultRevision}";
        AsOfText = $"As of {FormatInstant(result.AsOfMs)}";
        OnPropertyChanged(nameof(BatchSelectionText));
        return invalidatedSelections;
    }

    public async Task ClearSearchAsync()
    {
        SearchText = string.Empty;
        SelectedFreshness = "All";
        SelectedGrouping = "None";
        await ApplyScopeAsync();
    }

    private void ClearBatchSelection()
    {
        foreach (var row in Rows)
        {
            row.IsBatchSelected = false;
        }

        _batchSelection.Clear();
        OnPropertyChanged(nameof(BatchSelectionText));
    }

    private void SelectAllVisible()
    {
        foreach (var row in Rows)
        {
            row.IsBatchSelected = true;
        }

        OnPropertyChanged(nameof(BatchSelectionText));
    }

    private void OnRowPropertyChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs eventArgs)
    {
        if (eventArgs.PropertyName == nameof(DeviceRowViewModel.IsBatchSelected) &&
            sender is DeviceRowViewModel row)
        {
            if (row.IsBatchSelected)
            {
                _batchSelection[row.DeviceId] = row.Projection.Identity.IdentityRevision;
            }
            else if (_batchSelection.TryGetValue(row.DeviceId, out var revision) &&
                     revision == row.Projection.Identity.IdentityRevision)
            {
                _batchSelection.Remove(row.DeviceId);
            }

            OnPropertyChanged(nameof(BatchSelectionText));
        }
    }

    private void ApplyGrouping(string grouping)
    {
        using (RowsView.DeferRefresh())
        {
            RowsView.GroupDescriptions.Clear();
            var propertyName = grouping switch
            {
                "Cohort" => nameof(DeviceRowViewModel.CohortGroup),
                "Model" => nameof(DeviceRowViewModel.Model),
                "Freshness" => nameof(DeviceRowViewModel.FreshnessGroup),
                "Application" => nameof(DeviceRowViewModel.ApplicationGroup),
                _ => null
            };
            if (propertyName is not null)
            {
                RowsView.GroupDescriptions.Add(
                    new PropertyGroupDescription(propertyName));
            }
        }
    }

    private void UpdateActiveScopeText()
    {
        var parts = new List<string> { "Active scope" };
        parts.Add(string.IsNullOrWhiteSpace(_appliedSearchText)
            ? "all identities"
            : $"identity contains “{_appliedSearchText}”");
        if (_appliedFreshness != "All")
        {
            parts.Add($"freshness = {_appliedFreshness.ToLowerInvariant()}");
        }

        parts.Add($"grouped by {_appliedGrouping.ToLowerInvariant()}");
        ActiveScopeText = string.Join(" · ", parts);
    }

    private static string NormalizeOption(string? value, string fallback) =>
        string.IsNullOrWhiteSpace(value) ? fallback : value.Trim();

    private static string FormatInstant(long value)
    {
        try
        {
            return DateTimeOffset.FromUnixTimeMilliseconds(value)
                .ToLocalTime()
                .ToString("yyyy-MM-dd HH:mm:ss");
        }
        catch (ArgumentOutOfRangeException)
        {
            return value.ToString(System.Globalization.CultureInfo.InvariantCulture);
        }
    }
}
