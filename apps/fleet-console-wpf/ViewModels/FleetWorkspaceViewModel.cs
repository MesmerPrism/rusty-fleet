// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Collections.ObjectModel;
using System.Net.Http;
using System.Text.Json;
using RustyFleet.FleetConsole.Contracts;
using RustyFleet.FleetConsole.Services;

namespace RustyFleet.FleetConsole.ViewModels;

public sealed class FleetWorkspaceViewModel : ObservableObject
{
    private readonly Func<Uri, IFleetDataSource>? _sourceFactory;
    private IFleetDataSource? _source;
    private string _hubAddress = "http://127.0.0.1:8741/";
    private string _searchText = string.Empty;
    private string _statusMessage = "Disconnected · enter a local Hub address and connect";
    private string _summaryText = "No fleet data loaded";
    private string _scopeText = "0 devices";
    private string _asOfText = "No accepted snapshot";
    private bool _isBusy;
    private DeviceRowViewModel? _selectedDevice;
    private DeviceInspectorViewModel? _inspector;
    private CancellationTokenSource? _requestCancellation;

    public FleetWorkspaceViewModel(Func<Uri, IFleetDataSource> sourceFactory)
    {
        _sourceFactory = sourceFactory;
        ConnectCommand = new AsyncCommand(ConnectAsync, () => !IsBusy);
        RefreshCommand = new AsyncCommand(RefreshAsync, () => !IsBusy && _source is not null);
        ApplySearchCommand = new AsyncCommand(RefreshAsync, () => !IsBusy && _source is not null);
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
            var selected = Rows.Count(row => row.IsBatchSelected);
            return $"{selected} selected · {Rows.Count} visible";
        }
    }

    public async Task InitializeAsync() => await RefreshAsync();

    public async Task SelectDeviceAsync(DeviceRowViewModel? device)
    {
        SelectedDevice = device;
        if (device is null)
        {
            Inspector = null;
            return;
        }

        Inspector = DeviceInspectorViewModel.FromRow(device.Projection);
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

            if (SelectedDevice?.StableKey == device.StableKey)
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
        OnPropertyChanged(nameof(BatchSelectionText));
    }

    public async Task RefreshAsync()
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
            var query = FleetQuery.Create(SearchText);
            var queryTask = _source.QueryAsync(query, _requestCancellation.Token);
            var summaryTask = _source.SummaryAsync(_requestCancellation.Token);
            await Task.WhenAll(queryTask, summaryTask);
            ApplyResult(await queryTask, await summaryTask, query);
            StatusMessage = "Connected · ordering stable · no background re-sort";
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

    private void ApplyResult(
        FleetQueryResult result,
        FleetSummaryProjection summary,
        FleetQuery requestedQuery)
    {
        FleetProjectionValidation.ValidateQueryResult(
            result,
            summary,
            requestedQuery);

        var selectedKey = SelectedDevice?.StableKey;
        var existing = Rows.ToDictionary(row => row.StableKey, StringComparer.Ordinal);
        var incomingKeys = new HashSet<string>(StringComparer.Ordinal);

        for (var index = 0; index < result.Rows.Count; index++)
        {
            var projection = result.Rows[index];
            var key = $"{projection.Identity.DeviceId}@{projection.Identity.IdentityRevision}";
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

        SelectedDevice = selectedKey is null
            ? null
            : Rows.FirstOrDefault(row => row.StableKey == selectedKey);
        if (SelectedDevice is null)
        {
            Inspector = null;
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
    }

    public async Task ClearSearchAsync()
    {
        SearchText = string.Empty;
        await RefreshAsync();
    }

    private void ClearBatchSelection()
    {
        foreach (var row in Rows)
        {
            row.IsBatchSelected = false;
        }

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
        if (eventArgs.PropertyName == nameof(DeviceRowViewModel.IsBatchSelected))
        {
            OnPropertyChanged(nameof(BatchSelectionText));
        }
    }

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
