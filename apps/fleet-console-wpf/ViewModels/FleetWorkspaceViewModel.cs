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
    private readonly Dictionary<string, string> _displayedGroupValues =
        new(StringComparer.Ordinal);
    private IFleetDataSource? _source;
    private string _hubAddress = "http://127.0.0.1:8741/";
    private string _searchText = string.Empty;
    private string _selectedFreshness = "All";
    private string _selectedGrouping = "None";
    private string _selectedSort = "Device name";
    private string _selectedSortDirection = "Ascending";
    private string _appliedSearchText = string.Empty;
    private string _appliedFreshness = "All";
    private string _appliedGrouping = "None";
    private string _appliedSort = "Device name";
    private string _appliedSortDirection = "Ascending";
    private FleetQuery _appliedQuery = FleetQuery.Create(null);
    private string? _activeSavedViewName;
    private bool _appliedEditorScopeKnown = true;
    private ulong _savedViewRevision = 1;
    private SavedView? _selectedSavedView;
    private string _savedViewName = string.Empty;
    private string _statusMessage = "Disconnected · enter a local Hub address and connect";
    private string _summaryText = "No fleet data loaded";
    private string _scopeText = "0 devices";
    private string _asOfText = "No accepted snapshot";
    private string _activeScopeText =
        "Active scope · all devices · sorted by device name ascending · grouped by none";
    private string _inspectorContextText = "No selected device";
    private bool _isBusy;
    private DeviceRowViewModel? _selectedDevice;
    private DeviceInspectorViewModel? _inspector;
    private DeviceDetailViewModel? _detail;
    private bool _isDetailOpen;
    private string _selectedDetailTab = "overview";
    private string? _inspectedStableKey;
    private CancellationTokenSource? _requestCancellation;
    private FleetQueryResult? _queuedResult;
    private FleetSummaryProjection? _queuedSummary;
    private FleetQuery? _queuedQuery;
    private int _queuedOrderingChangeCount;

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
        ApplyQueuedOrderingChangesCommand = new RelayCommand(
            ApplyQueuedOrderingChanges,
            () => HasQueuedOrderingChanges && !IsBusy);
        ApplySavedViewCommand = new AsyncCommand(
            ApplySavedViewAsync,
            () => !IsBusy && _source is not null && SelectedSavedView is not null);
        DeleteSavedViewCommand = new AsyncCommand(
            DeleteSavedViewAsync,
            () => !IsBusy && _source is not null && SelectedSavedView is not null);
        if (RowsView is ICollectionViewLiveShaping liveView &&
            liveView.CanChangeLiveGrouping)
        {
            liveView.IsLiveGrouping = false;
        }

        if (RowsView is ICollectionViewLiveShaping sortableView &&
            sortableView.CanChangeLiveSorting)
        {
            sortableView.IsLiveSorting = false;
        }
    }

    public FleetWorkspaceViewModel(IFleetDataSource source)
        : this(_ => source)
    {
        _source = source;
        StatusMessage = "Test data source ready";
    }

    public ObservableCollection<DeviceRowViewModel> Rows { get; } = [];

    public ObservableCollection<SavedView> SavedViews { get; } = [];

    public ICollectionView RowsView { get; }

    public IReadOnlyList<string> FreshnessOptions { get; } =
        ["All", "Fresh", "Stale", "Offline", "Unknown"];

    public IReadOnlyList<string> GroupingOptions { get; } =
        ["None", "Cohort", "Model", "Freshness", "Application"];

    public IReadOnlyList<string> SortOptions { get; } =
        ["Device name", "Freshness", "Battery", "Model", "Application"];

    public IReadOnlyList<string> SortDirectionOptions { get; } =
        ["Ascending", "Descending"];

    public AsyncCommand ConnectCommand { get; }

    public AsyncCommand RefreshCommand { get; }

    public AsyncCommand ApplySearchCommand { get; }

    public AsyncCommand ClearSearchCommand { get; }

    public RelayCommand ClearBatchSelectionCommand { get; }

    public RelayCommand SelectAllVisibleCommand { get; }

    public RelayCommand ApplyQueuedOrderingChangesCommand { get; }

    public AsyncCommand ApplySavedViewCommand { get; }

    public AsyncCommand DeleteSavedViewCommand { get; }

    public event Action<SavedView>? SavedViewRestorationRequested;

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

    public string SelectedSort
    {
        get => _selectedSort;
        set => SetProperty(ref _selectedSort, value);
    }

    public string SelectedSortDirection
    {
        get => _selectedSortDirection;
        set => SetProperty(ref _selectedSortDirection, value);
    }

    public SavedView? SelectedSavedView
    {
        get => _selectedSavedView;
        set
        {
            if (SetProperty(ref _selectedSavedView, value))
            {
                if (value is not null)
                {
                    SavedViewName = value.Name;
                }

                ApplySavedViewCommand.RaiseCanExecuteChanged();
                DeleteSavedViewCommand.RaiseCanExecuteChanged();
            }
        }
    }

    public string SavedViewName
    {
        get => _savedViewName;
        set => SetProperty(ref _savedViewName, value);
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
                ApplyQueuedOrderingChangesCommand.RaiseCanExecuteChanged();
                ApplySavedViewCommand.RaiseCanExecuteChanged();
                DeleteSavedViewCommand.RaiseCanExecuteChanged();
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

    public DeviceDetailViewModel? Detail
    {
        get => _detail;
        private set => SetProperty(ref _detail, value);
    }

    public bool IsDetailOpen
    {
        get => _isDetailOpen;
        private set => SetProperty(ref _isDetailOpen, value);
    }

    public string SelectedDetailTab
    {
        get => _selectedDetailTab;
        set
        {
            if (IsSupportedDetailTab(value))
            {
                SetProperty(ref _selectedDetailTab, value);
            }
        }
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

    public bool HasQueuedOrderingChanges => _queuedResult is not null;

    public string OrderingChangesText => HasQueuedOrderingChanges
        ? $"{_queuedOrderingChangeCount:N0} live row changes affect the current " +
          "order or grouping"
        : "Live ordering is current";

    public async Task InitializeAsync()
    {
        await RefreshAsync();
        await TryLoadSavedViewsAsync();
    }

    public async Task SelectDeviceAsync(DeviceRowViewModel? device)
    {
        CloseFullDetail();
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

    public async Task<bool> OpenFullDetailAsync(string tab = "overview")
    {
        var device = SelectedDevice;
        if (device is null || _source is null)
        {
            StatusMessage = "Select a device before opening full detail";
            return false;
        }

        try
        {
            var projection = await _source.DetailAsync(
                device.DeviceId,
                CancellationToken.None);
            FleetProjectionValidation.ValidateDetail(projection, device.Projection);
            if (SelectedDevice?.StableKey != device.StableKey)
            {
                StatusMessage = "Full detail was discarded because selection changed";
                return false;
            }

            Detail = new DeviceDetailViewModel(projection);
            SelectedDetailTab = IsSupportedDetailTab(tab) ? tab : "overview";
            IsDetailOpen = true;
            StatusMessage =
                $"Full detail · {device.DisplayName} · accepted revision " +
                device.Projection.AcceptedRevision;
            return true;
        }
        catch (Exception error) when (
            error is HttpRequestException or JsonException or TaskCanceledException or
            InvalidOperationException)
        {
            Detail = null;
            IsDetailOpen = false;
            StatusMessage = $"Full detail unavailable · fleet context retained · {error.Message}";
            return false;
        }
    }

    public void CloseFullDetail()
    {
        if (!IsDetailOpen && Detail is null)
        {
            return;
        }

        IsDetailOpen = false;
        Detail = null;
        SelectedDetailTab = "overview";
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
        _appliedSort,
        _appliedSortDirection,
        acceptEditorScope: false,
        exactQuery: _appliedQuery,
        preserveCurrentOrdering: true);

    public Task ApplyScopeAsync() => LoadScopeAsync(
        SearchText,
        SelectedFreshness,
        SelectedGrouping,
        SelectedSort,
        SelectedSortDirection,
        acceptEditorScope: true,
        exactQuery: null,
        preserveCurrentOrdering: false);

    private async Task LoadScopeAsync(
        string searchText,
        string freshness,
        string grouping,
        string sort,
        string sortDirection,
        bool acceptEditorScope,
        FleetQuery? exactQuery,
        bool preserveCurrentOrdering)
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
            var query = exactQuery ?? FleetQuery.Create(
                searchText,
                freshness,
                sortField: CanonicalSortField(sort),
                sortDirection: CanonicalSortDirection(sortDirection));
            var queryTask = _source.QueryAsync(query, _requestCancellation.Token);
            var summaryTask = _source.SummaryAsync(_requestCancellation.Token);
            await Task.WhenAll(queryTask, summaryTask);
            var preserveOrdering = preserveCurrentOrdering && Rows.Count > 0;
            var invalidatedSelections = ApplyResult(
                await queryTask,
                await summaryTask,
                query,
                preserveOrdering);
            if (acceptEditorScope)
            {
                _appliedSearchText = searchText.Trim();
                _appliedFreshness = NormalizeOption(freshness, "All");
                _appliedGrouping = NormalizeOption(grouping, "None");
                _appliedSort = NormalizeOption(sort, "Device name");
                _appliedSortDirection = NormalizeOption(sortDirection, "Ascending");
                _appliedQuery = query;
                _activeSavedViewName = null;
                _appliedEditorScopeKnown = true;
                ApplyGrouping(_appliedGrouping);
                UpdateActiveScopeText();
            }
            else if (!preserveOrdering)
            {
                ApplyGrouping(_appliedGrouping);
            }

            StatusMessage = HasQueuedOrderingChanges
                ? $"Connected · {_queuedOrderingChangeCount:N0} live row changes queued · " +
                  "shared values refreshed in place"
                : invalidatedSelections == 0
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
            await TryLoadSavedViewsAsync();
        }
        catch (ArgumentException error)
        {
            StatusMessage = error.Message;
        }
    }

    private int ApplyResult(
        FleetQueryResult result,
        FleetSummaryProjection summary,
        FleetQuery requestedQuery,
        bool preserveOrdering)
    {
        FleetProjectionValidation.ValidateQueryResult(
            result,
            summary,
            requestedQuery);

        var existing = Rows.ToDictionary(row => row.StableKey, StringComparer.Ordinal);
        var orderingChanges = preserveOrdering
            ? CountOrderingChanges(result.Rows)
            : 0;
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
                foreach (var selectedRow in Rows.Where(row =>
                             row.DeviceId == projection.Identity.DeviceId &&
                             row.IsBatchSelected))
                {
                    selectedRow.IsBatchSelected = false;
                }

                invalidatedSelections++;
            }

            incomingKeys.Add(key);
            if (existing.TryGetValue(key, out var row))
            {
                row.Update(projection);
                if (!preserveOrdering)
                {
                    var currentIndex = Rows.IndexOf(row);
                    if (currentIndex != index)
                    {
                        Rows.Move(currentIndex, index);
                    }
                }
            }
            else if (!preserveOrdering)
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

        if (!preserveOrdering)
        {
            for (var index = Rows.Count - 1; index >= 0; index--)
            {
                if (!incomingKeys.Contains(Rows[index].StableKey))
                {
                    Rows[index].PropertyChanged -= OnRowPropertyChanged;
                    Rows.RemoveAt(index);
                }
            }
        }

        if (preserveOrdering && orderingChanges > 0)
        {
            SetQueuedOrderingChanges(
                result,
                summary,
                requestedQuery,
                orderingChanges);
        }
        else
        {
            ClearQueuedOrderingChanges();
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
        ScopeText = HasQueuedOrderingChanges
            ? $"{Rows.Count:N0} displayed · {result.TotalCount:N0} currently matching · " +
              $"result revision {result.ResultRevision}"
            : $"{result.WindowCount:N0} shown · {result.TotalCount:N0} matching · " +
              $"result revision {result.ResultRevision}";
        AsOfText = $"As of {FormatInstant(result.AsOfMs)}";
        OnPropertyChanged(nameof(BatchSelectionText));
        return invalidatedSelections;
    }

    private int CountOrderingChanges(
        IReadOnlyList<DeviceRowProjection> incomingRows)
    {
        var currentPositions = Rows
            .Select((row, index) => (row.StableKey, index))
            .ToDictionary(item => item.StableKey, item => item.index, StringComparer.Ordinal);
        var incomingPositions = incomingRows
            .Select((row, index) => (
                Key: $"{row.Identity.DeviceId}@{row.Identity.IdentityRevision}",
                Index: index,
                Projection: row))
            .ToDictionary(item => item.Key, item => item, StringComparer.Ordinal);
        var affected = new HashSet<string>(StringComparer.Ordinal);

        foreach (var (key, currentIndex) in currentPositions)
        {
            if (!incomingPositions.TryGetValue(key, out var incoming))
            {
                affected.Add(key);
                continue;
            }

            if (currentIndex != incoming.Index ||
                !_displayedGroupValues.TryGetValue(key, out var displayedGroup) ||
                displayedGroup != GroupValue(incoming.Projection, _appliedGrouping))
            {
                affected.Add(key);
            }
        }

        foreach (var key in incomingPositions.Keys)
        {
            if (!currentPositions.ContainsKey(key))
            {
                affected.Add(key);
            }
        }

        return affected.Count;
    }

    private static string GroupValue(DeviceRowProjection row, string grouping) =>
        grouping switch
        {
            "Cohort" => row.Identity.Tags.TryGetValue("cohort", out var cohort)
                ? cohort
                : "Unassigned",
            "Model" => row.Identity.Model,
            "Freshness" => row.Freshness,
            "Application" => string.IsNullOrWhiteSpace(row.ForegroundApp)
                ? "No participating app"
                : row.ForegroundApp,
            _ => string.Empty
        };

    private void SetQueuedOrderingChanges(
        FleetQueryResult result,
        FleetSummaryProjection summary,
        FleetQuery query,
        int count)
    {
        _queuedResult = result;
        _queuedSummary = summary;
        _queuedQuery = query;
        _queuedOrderingChangeCount = count;
        OnPropertyChanged(nameof(HasQueuedOrderingChanges));
        OnPropertyChanged(nameof(OrderingChangesText));
        ApplyQueuedOrderingChangesCommand.RaiseCanExecuteChanged();
    }

    private void ClearQueuedOrderingChanges()
    {
        var changed = _queuedResult is not null || _queuedOrderingChangeCount != 0;
        _queuedResult = null;
        _queuedSummary = null;
        _queuedQuery = null;
        _queuedOrderingChangeCount = 0;
        if (changed)
        {
            OnPropertyChanged(nameof(HasQueuedOrderingChanges));
            OnPropertyChanged(nameof(OrderingChangesText));
            ApplyQueuedOrderingChangesCommand.RaiseCanExecuteChanged();
        }
    }

    private void ApplyQueuedOrderingChanges()
    {
        if (_queuedResult is null || _queuedSummary is null || _queuedQuery is null)
        {
            return;
        }

        var result = _queuedResult;
        var summary = _queuedSummary;
        var query = _queuedQuery;
        var invalidatedSelections = ApplyResult(
            result,
            summary,
            query,
            preserveOrdering: false);
        ApplyGrouping(_appliedGrouping);
        StatusMessage = invalidatedSelections == 0
            ? $"Connected · live ordering applied at result revision {result.ResultRevision}"
            : $"Connected · live ordering applied · {invalidatedSelections} batch selection " +
              "invalidated by an identity revision";
    }

    public async Task SaveCurrentViewAsync(
        IReadOnlyList<string> columns,
        string focusedRegion)
    {
        if (_source is null)
        {
            StatusMessage = "Not connected to a Fleet Hub";
            return;
        }

        var name = SavedViewName.Trim();
        if (name.Length is 0 or > 256)
        {
            StatusMessage = "Saved-view name must contain 1–256 characters";
            return;
        }

        var existingByName = SavedViews.FirstOrDefault(view =>
            string.Equals(view.Name, name, StringComparison.OrdinalIgnoreCase));
        var viewId = SelectedSavedView?.ViewId ??
                     existingByName?.ViewId ??
                     CreateSavedViewId(name);
        var view = new SavedView
        {
            ViewId = viewId,
            Name = name,
            Query = _appliedQuery,
            Columns = columns,
            Density = "standard",
            Grouping = _appliedGrouping == "None"
                ? null
                : _appliedGrouping.ToLowerInvariant(),
            Restoration = new NavigationRestoration
            {
                SelectedDeviceId = SelectedDevice?.DeviceId,
                InspectorTab = SelectedDevice is null
                    ? null
                    : IsDetailOpen
                        ? SelectedDetailTab
                        : "overview",
                ScrollAnchorDeviceId = SelectedDevice?.DeviceId,
                FocusedRegion = focusedRegion,
                CollapsedGroups = []
            }
        };

        try
        {
            var receipt = await _source.UpsertSavedViewAsync(
                new SavedViewMutationRequest
                {
                    ExpectedRevision = _savedViewRevision,
                    View = view
                },
                CancellationToken.None);
            FleetProjectionValidation.ValidateSavedViewReceipt(receipt);
            await LoadSavedViewsAsync(viewId);
            StatusMessage = receipt.Changed
                ? $"Saved view “{name}” at revision {receipt.CurrentRevision}"
                : $"Saved view “{name}” was already current";
        }
        catch (Exception error) when (
            error is HttpRequestException or JsonException or TaskCanceledException or
            InvalidOperationException)
        {
            await TryLoadSavedViewsAsync();
            StatusMessage = $"Save failed · canonical saved views reloaded · {error.Message}";
        }
    }

    public async Task ApplySavedViewAsync()
    {
        var view = SelectedSavedView;
        if (_source is null || view is null)
        {
            return;
        }

        var grouping = FromSavedGrouping(view.Grouping);
        var groupingKnown = view.Grouping is null || grouping != "None";
        var editorKnown = TryProjectSimpleScope(
            view.Query,
            out var searchText,
            out var freshness,
            out var sort,
            out var sortDirection);
        SearchText = searchText;
        SelectedFreshness = freshness;
        SelectedGrouping = grouping;
        SelectedSort = sort;
        SelectedSortDirection = sortDirection;
        _appliedSearchText = searchText;
        _appliedFreshness = freshness;
        _appliedGrouping = grouping;
        _appliedSort = sort;
        _appliedSortDirection = sortDirection;
        _appliedQuery = view.Query;
        _activeSavedViewName = view.Name;
        _appliedEditorScopeKnown = editorKnown;

        await LoadScopeAsync(
            searchText,
            freshness,
            grouping,
            sort,
            sortDirection,
            acceptEditorScope: false,
            exactQuery: view.Query,
            preserveCurrentOrdering: false);
        ApplyGrouping(grouping);
        UpdateActiveScopeText();

        var restoredDevice = view.Restoration.SelectedDeviceId is null
            ? null
            : Rows.FirstOrDefault(row =>
                row.DeviceId == view.Restoration.SelectedDeviceId);
        await SelectDeviceAsync(restoredDevice);
        if (restoredDevice is not null &&
            view.Restoration.InspectorTab is { } tab &&
            tab != "overview" &&
            IsSupportedDetailTab(tab))
        {
            await OpenFullDetailAsync(tab);
        }
        SavedViewRestorationRequested?.Invoke(view);

        var skipped = new List<string>();
        if (!editorKnown)
        {
            skipped.Add("advanced filter is exact but read-only in the simple scope editor");
        }
        if (view.Restoration.InspectorTab is { } unavailableTab &&
            !IsSupportedDetailTab(unavailableTab))
        {
            skipped.Add($"inspector tab “{unavailableTab}” is not available in M1");
        }
        if (view.Restoration.CollapsedGroups.Count > 0)
        {
            skipped.Add("collapsed groups are not yet restorable");
        }
        if (view.Restoration.SelectedDeviceId is not null &&
            restoredDevice is null)
        {
            skipped.Add("selected device is outside the restored result");
        }
        if (view.Restoration.FocusedRegion is not null &&
            view.Restoration.FocusedRegion is not
                ("shell" or "search" or "saved_views" or "grid" or "inspector" or "detail"))
        {
            skipped.Add($"focus region “{view.Restoration.FocusedRegion}” is unavailable");
        }
        if (!groupingKnown)
        {
            skipped.Add($"grouping “{view.Grouping}” is not available in M1");
        }
        if (view.Density != "standard")
        {
            skipped.Add($"density “{view.Density}” is not available in M1");
        }
        if (view.SchemaVersion != 1)
        {
            skipped.Add($"saved-view schema version {view.SchemaVersion} is newer than M1");
        }
        var knownColumns = new HashSet<string>(
            [
                "selection", "attention", "device", "age", "route", "power",
                "application", "control", "privileged", "streams", "work", "tags"
            ],
            StringComparer.Ordinal);
        var unknownColumns = view.Columns.Count(column => !knownColumns.Contains(column));
        if (unknownColumns > 0)
        {
            skipped.Add($"{unknownColumns} unknown column(s) were ignored");
        }
        StatusMessage = skipped.Count == 0
            ? $"Applied saved view “{view.Name}”"
            : $"Applied saved view “{view.Name}” · {string.Join("; ", skipped)}";
    }

    public async Task DeleteSavedViewAsync()
    {
        var view = SelectedSavedView;
        if (_source is null || view is null)
        {
            return;
        }

        try
        {
            var receipt = await _source.DeleteSavedViewAsync(
                view.ViewId,
                _savedViewRevision,
                CancellationToken.None);
            FleetProjectionValidation.ValidateSavedViewReceipt(receipt);
            if (_activeSavedViewName == view.Name)
            {
                _activeSavedViewName = null;
                UpdateActiveScopeText();
            }

            await LoadSavedViewsAsync();
            StatusMessage =
                $"Deleted saved view “{view.Name}” at revision {receipt.CurrentRevision}";
        }
        catch (Exception error) when (
            error is HttpRequestException or JsonException or TaskCanceledException or
            InvalidOperationException)
        {
            await TryLoadSavedViewsAsync();
            StatusMessage = $"Delete failed · canonical saved views reloaded · {error.Message}";
        }
    }

    private async Task<bool> TryLoadSavedViewsAsync(string? selectViewId = null)
    {
        try
        {
            await LoadSavedViewsAsync(selectViewId);
            return true;
        }
        catch (Exception error) when (
            error is HttpRequestException or JsonException or TaskCanceledException or
            InvalidOperationException)
        {
            StatusMessage =
                $"Fleet scope remains available · saved views unavailable · {error.Message}";
            return false;
        }
    }

    private async Task LoadSavedViewsAsync(string? selectViewId = null)
    {
        if (_source is null)
        {
            return;
        }

        var selectedId = selectViewId ?? SelectedSavedView?.ViewId;
        var collection = await _source.SavedViewsAsync(CancellationToken.None);
        FleetProjectionValidation.ValidateSavedViews(collection);
        _savedViewRevision = collection.Revision;
        SavedViews.Clear();
        foreach (var view in collection.Views)
        {
            SavedViews.Add(view);
        }

        SelectedSavedView = selectedId is null
            ? null
            : SavedViews.FirstOrDefault(view => view.ViewId == selectedId);
    }

    public async Task ClearSearchAsync()
    {
        SearchText = string.Empty;
        SelectedFreshness = "All";
        SelectedGrouping = "None";
        SelectedSort = "Device name";
        SelectedSortDirection = "Ascending";
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

        _displayedGroupValues.Clear();
        foreach (var row in Rows)
        {
            _displayedGroupValues[row.StableKey] =
                GroupValue(row.Projection, grouping);
        }
    }

    private void UpdateActiveScopeText()
    {
        var parts = new List<string>
        {
            _activeSavedViewName is null
                ? "Active scope"
                : $"Saved view “{_activeSavedViewName}”"
        };
        if (_appliedEditorScopeKnown)
        {
            parts.Add(string.IsNullOrWhiteSpace(_appliedSearchText)
                ? "all identities"
                : $"identity contains “{_appliedSearchText}”");
            if (_appliedFreshness != "All")
            {
                parts.Add($"freshness = {_appliedFreshness.ToLowerInvariant()}");
            }
        }
        else
        {
            parts.Add("canonical advanced filter");
        }

        parts.Add(
            $"sorted by {_appliedSort.ToLowerInvariant()} " +
            _appliedSortDirection.ToLowerInvariant());
        parts.Add($"grouped by {_appliedGrouping.ToLowerInvariant()}");
        ActiveScopeText = string.Join(" · ", parts);
    }

    private string CreateSavedViewId(string name)
    {
        var slug = new string(name
            .ToLowerInvariant()
            .Select(character => char.IsAsciiLetterOrDigit(character) ? character : '_')
            .ToArray())
            .Trim('_');
        if (slug.Length == 0)
        {
            slug = "operator";
        }

        if (slug.Length > 80)
        {
            slug = slug[..80];
        }

        var candidate = $"view.operator.{slug}";
        return SavedViews.All(view => view.ViewId != candidate)
            ? candidate
            : $"{candidate}.{Guid.NewGuid():N}"[..Math.Min(128, candidate.Length + 33)];
    }

    private static string FromSavedGrouping(string? grouping) =>
        grouping?.ToLowerInvariant() switch
        {
            "cohort" => "Cohort",
            "model" => "Model",
            "freshness" => "Freshness",
            "application" => "Application",
            _ => "None"
        };

    private static bool IsSupportedDetailTab(string value) =>
        value is "overview" or "status" or "capabilities" or "work" or "streams" or "history";

    private static bool TryProjectSimpleScope(
        FleetQuery query,
        out string searchText,
        out string freshness,
        out string sort,
        out string sortDirection)
    {
        searchText = string.Empty;
        freshness = "All";
        sort = "Device name";
        sortDirection = "Ascending";
        if (query.Sort.Count != 1 || query.Sort[0].Qualifier is not null)
        {
            return false;
        }

        sort = query.Sort[0].Field switch
        {
            "display_name" => "Device name",
            "freshness" => "Freshness",
            "battery_percent" => "Battery",
            "model" => "Model",
            "foreground_app" => "Application",
            _ => string.Empty
        };
        sortDirection = query.Sort[0].Direction switch
        {
            "ascending" => "Ascending",
            "descending" => "Descending",
            _ => string.Empty
        };
        if (sort.Length == 0 || sortDirection.Length == 0)
        {
            return false;
        }

        if (query.Expression is null)
        {
            return true;
        }

        var expression = JsonSerializer.SerializeToElement(
            query.Expression,
            FleetJson.Options);
        return TryReadSimpleExpression(
            expression,
            ref searchText,
            ref freshness);
    }

    private static bool TryReadSimpleExpression(
        JsonElement expression,
        ref string searchText,
        ref string freshness)
    {
        if (!expression.TryGetProperty("kind", out var kindElement))
        {
            return false;
        }

        var kind = kindElement.GetString();
        if (kind == "and")
        {
            if (!expression.TryGetProperty("expressions", out var terms) ||
                terms.ValueKind != JsonValueKind.Array)
            {
                return false;
            }

            foreach (var term in terms.EnumerateArray())
            {
                if (!TryReadSimpleExpression(term, ref searchText, ref freshness))
                {
                    return false;
                }
            }

            return true;
        }

        if (kind == "predicate")
        {
            if (!TryReadPredicate(
                    expression,
                    out var field,
                    out var comparison,
                    out var value) ||
                field != "freshness" ||
                comparison != "equals" ||
                freshness != "All" ||
                value is not ("fresh" or "stale" or "offline" or "unknown"))
            {
                return false;
            }

            freshness = char.ToUpperInvariant(value[0]) + value[1..];
            return true;
        }

        if (kind != "or" ||
            searchText.Length != 0 ||
            !expression.TryGetProperty("expressions", out var alternatives) ||
            alternatives.ValueKind != JsonValueKind.Array)
        {
            return false;
        }

        var predicates = alternatives.EnumerateArray().ToArray();
        if (predicates.Length != 2 ||
            !TryReadPredicate(
                predicates[0],
                out var firstField,
                out var firstComparison,
                out var firstValue) ||
            !TryReadPredicate(
                predicates[1],
                out var secondField,
                out var secondComparison,
                out var secondValue) ||
            firstComparison != "contains" ||
            secondComparison != "contains" ||
            firstValue != secondValue ||
            new HashSet<string>([firstField, secondField], StringComparer.Ordinal)
                .SetEquals(["display_name", "device_id"]))
        {
            return false;
        }

        searchText = firstValue;
        return true;
    }

    private static bool TryReadPredicate(
        JsonElement predicate,
        out string field,
        out string comparison,
        out string value)
    {
        field = string.Empty;
        comparison = string.Empty;
        value = string.Empty;
        return predicate.TryGetProperty("kind", out var kind) &&
               kind.GetString() == "predicate" &&
               predicate.TryGetProperty("field", out var fieldElement) &&
               (field = fieldElement.GetString() ?? string.Empty).Length > 0 &&
               predicate.TryGetProperty("comparison", out var comparisonElement) &&
               (comparison = comparisonElement.GetString() ?? string.Empty).Length > 0 &&
               predicate.TryGetProperty("value", out var valueElement) &&
               valueElement.ValueKind == JsonValueKind.String &&
               (value = valueElement.GetString() ?? string.Empty).Length > 0;
    }

    private static string CanonicalSortField(string? value) =>
        NormalizeOption(value, "Device name") switch
        {
            "Freshness" => "freshness",
            "Battery" => "battery_percent",
            "Model" => "model",
            "Application" => "foreground_app",
            _ => "display_name"
        };

    private static string CanonicalSortDirection(string? value) =>
        string.Equals(
            NormalizeOption(value, "Ascending"),
            "Descending",
            StringComparison.Ordinal)
            ? "descending"
            : "ascending";

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
