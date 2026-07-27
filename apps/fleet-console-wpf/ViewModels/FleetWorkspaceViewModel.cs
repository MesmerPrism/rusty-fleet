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
    public const int WatchEventLimit = 10_000;

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
    private ulong _watchSequence;
    private bool _watchInitialized;
    private string? _watchFailureReason;
    private bool _lastRefreshSucceeded;
    private OperationLedger? _currentOperation;
    private string _operationSummaryText = "No kiosk operation preview";
    private string _operationStatusText =
        "Select exact devices, then preview Show Kiosk controls";
    private PackageInstallReleaseOperation? _currentPackageOperation;
    private string _packageManifestUrl = string.Empty;
    private string _packageName = string.Empty;
    private string _packageRolloutRing = "alpha";
    private string _packageOperationSummaryText = "No package operation preview";
    private string _packageOperationStatusText =
        "Enter a signed manifest URL and package identity, then select exact devices";
    private QuestAwakeOperation? _currentQuestAwakeOperation;
    private QuestAwakeActionOption _selectedQuestAwakeAction =
        new(
            QuestAwakeActions.ApplyBounded,
            "Meta keep-awake (up to 8 hours)",
            "Uses Meta's bounded proximity hold and wake controls.");
    private string _questAwakeDurationMinutes = "480";
    private string _questAwakeWatchdogIntervalSeconds = "5";
    private string _questAwakeSummaryText = "No headset awake-control preview";
    private string _questAwakeStatusText =
        "Choose an action, select exact devices, then preview";

    public FleetWorkspaceViewModel(Func<Uri, IFleetDataSource> sourceFactory)
    {
        _sourceFactory = sourceFactory;
        RowsView = CollectionViewSource.GetDefaultView(Rows);
        ConnectCommand = new AsyncCommand(ConnectAsync, () => !IsBusy);
        SynchronizeUpdatesCommand = new AsyncCommand(
            SynchronizeUpdatesAsync,
            () => !IsBusy && _source is not null);
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
        PreviewKioskShowControlsCommand = new AsyncCommand(
            PreviewKioskShowControlsAsync,
            () => !IsBusy && _source is not null && _batchSelection.Count > 0);
        ConfirmOperationCommand = new AsyncCommand(
            ConfirmOperationAsync,
            () => !IsBusy && _source is not null && CurrentOperation is not null);
        RefreshOperationCommand = new AsyncCommand(
            RefreshOperationAsync,
            () => !IsBusy && _source is not null && CurrentOperation is not null);
        DismissOperationCommand = new RelayCommand(
            DismissOperation,
            () => !IsBusy && CurrentOperation is not null);
        PreviewPackageInstallReleaseCommand = new AsyncCommand(
            PreviewPackageInstallReleaseAsync,
            () => !IsBusy &&
                  _source is not null &&
                  _batchSelection.Count > 0 &&
                  CurrentPackageOperation is null);
        ConfirmPackageInstallReleaseCommand = new AsyncCommand(
            ConfirmPackageInstallReleaseAsync,
            () => CanConfirmPackageInstallRelease);
        RefreshPackageInstallReleaseCommand = new AsyncCommand(
            RefreshPackageInstallReleaseAsync,
            () => !IsBusy && _source is not null && CurrentPackageOperation is not null);
        DismissPackageInstallReleaseCommand = new RelayCommand(
            DismissPackageInstallRelease,
            () => !IsBusy && CurrentPackageOperation is not null);
        PreviewQuestAwakeCommand = new AsyncCommand(
            PreviewQuestAwakeAsync,
            () => !IsBusy &&
                  _source is not null &&
                  _batchSelection.Count > 0 &&
                  CurrentQuestAwakeOperation is null);
        ConfirmQuestAwakeCommand = new AsyncCommand(
            ConfirmQuestAwakeAsync,
            () => CanConfirmQuestAwake);
        RefreshQuestAwakeCommand = new AsyncCommand(
            RefreshQuestAwakeAsync,
            () => !IsBusy &&
                  _source is not null &&
                  CurrentQuestAwakeOperation is not null);
        DismissQuestAwakeCommand = new RelayCommand(
            DismissQuestAwake,
            () => !IsBusy && CurrentQuestAwakeOperation is not null);
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

    public ObservableCollection<OperationTargetViewModel> OperationTargets { get; } = [];

    public ObservableCollection<PackageOperationTargetViewModel> PackageOperationTargets { get; } =
        [];

    public ObservableCollection<QuestAwakeTargetViewModel> QuestAwakeTargets { get; } = [];

    public ICollectionView RowsView { get; }

    public IReadOnlyList<string> FreshnessOptions { get; } =
        ["All", "Fresh", "Stale", "Offline", "Unknown"];

    public IReadOnlyList<string> GroupingOptions { get; } =
        ["None", "Cohort", "Model", "Freshness", "Application"];

    public IReadOnlyList<string> SortOptions { get; } =
        ["Device name", "Freshness", "Battery", "Model", "Application"];

    public IReadOnlyList<string> SortDirectionOptions { get; } =
        ["Ascending", "Descending"];

    public IReadOnlyList<QuestAwakeActionOption> QuestAwakeActionOptions { get; } =
    [
        new(
            QuestAwakeActions.Status,
            "Check current awake status",
            "Reads power, proximity-hold, and watchdog state without changing it."),
        new(
            QuestAwakeActions.ApplyBounded,
            "Meta keep-awake (up to 8 hours)",
            "Applies Meta's bounded proximity hold and wakes the headset."),
        new(
            QuestAwakeActions.StartWindowsWatchdog,
            "Keep awake with Windows watchdog",
            "Windows checks the headset and repairs drift while the Fleet Hub is running."),
        new(
            QuestAwakeActions.StartDeviceWatchdog,
            "Keep awake with Quest watchdog (stops on reboot)",
            "Runs the drift-repair watchdog on the headset. It stops when the headset reboots."),
        new(
            QuestAwakeActions.StopWatchdogs,
            "Stop watchdogs only (settings remain)",
            "Stops both watchdog modes without restoring normal sleep or proximity settings."),
        new(
            QuestAwakeActions.RestoreNormal,
            "Restore normal sleep settings",
            "Stops watchdogs, then restores normal sleep and proximity behavior.")
    ];

    public AsyncCommand ConnectCommand { get; }

    public AsyncCommand SynchronizeUpdatesCommand { get; }

    public AsyncCommand ApplySearchCommand { get; }

    public AsyncCommand ClearSearchCommand { get; }

    public RelayCommand ClearBatchSelectionCommand { get; }

    public RelayCommand SelectAllVisibleCommand { get; }

    public RelayCommand ApplyQueuedOrderingChangesCommand { get; }

    public AsyncCommand ApplySavedViewCommand { get; }

    public AsyncCommand DeleteSavedViewCommand { get; }

    public AsyncCommand PreviewKioskShowControlsCommand { get; }

    public AsyncCommand ConfirmOperationCommand { get; }

    public AsyncCommand RefreshOperationCommand { get; }

    public RelayCommand DismissOperationCommand { get; }

    public AsyncCommand PreviewPackageInstallReleaseCommand { get; }

    public AsyncCommand ConfirmPackageInstallReleaseCommand { get; }

    public AsyncCommand RefreshPackageInstallReleaseCommand { get; }

    public RelayCommand DismissPackageInstallReleaseCommand { get; }

    public AsyncCommand PreviewQuestAwakeCommand { get; }

    public AsyncCommand ConfirmQuestAwakeCommand { get; }

    public AsyncCommand RefreshQuestAwakeCommand { get; }

    public RelayCommand DismissQuestAwakeCommand { get; }

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

    public OperationLedger? CurrentOperation
    {
        get => _currentOperation;
        private set
        {
            if (SetProperty(ref _currentOperation, value))
            {
                ConfirmOperationCommand.RaiseCanExecuteChanged();
                RefreshOperationCommand.RaiseCanExecuteChanged();
                DismissOperationCommand.RaiseCanExecuteChanged();
            }
        }
    }

    public string OperationSummaryText
    {
        get => _operationSummaryText;
        private set => SetProperty(ref _operationSummaryText, value);
    }

    public string OperationStatusText
    {
        get => _operationStatusText;
        private set => SetProperty(ref _operationStatusText, value);
    }

    public string PackageManifestUrl
    {
        get => _packageManifestUrl;
        set => SetProperty(ref _packageManifestUrl, value);
    }

    public string PackageName
    {
        get => _packageName;
        set => SetProperty(ref _packageName, value);
    }

    public string PackageRolloutRing
    {
        get => _packageRolloutRing;
        set => SetProperty(ref _packageRolloutRing, value);
    }

    public PackageInstallReleaseOperation? CurrentPackageOperation
    {
        get => _currentPackageOperation;
        private set
        {
            if (SetProperty(ref _currentPackageOperation, value))
            {
                OnPropertyChanged(nameof(IsPackageInputLocked));
                OnPropertyChanged(nameof(PackageInputLockText));
                OnPropertyChanged(nameof(CanConfirmPackageInstallRelease));
                OnPropertyChanged(nameof(PackageConfirmationButtonText));
                PreviewPackageInstallReleaseCommand.RaiseCanExecuteChanged();
                ConfirmPackageInstallReleaseCommand.RaiseCanExecuteChanged();
                RefreshPackageInstallReleaseCommand.RaiseCanExecuteChanged();
                DismissPackageInstallReleaseCommand.RaiseCanExecuteChanged();
            }
        }
    }

    public bool IsPackageInputLocked => CurrentPackageOperation is not null;

    public string PackageInputLockText => IsPackageInputLocked
        ? "Locked to immutable preview"
        : string.Empty;

    public bool CanConfirmPackageInstallRelease =>
        !IsBusy &&
        _source is not null &&
        IsPackagePreviewReady(CurrentPackageOperation);

    public string PackageConfirmationButtonText =>
        CurrentPackageOperation?.Lifecycle == "accepted"
            ? "Preparation accepted"
            : "Confirm preparation";

    public string PackageOperationSummaryText
    {
        get => _packageOperationSummaryText;
        private set => SetProperty(ref _packageOperationSummaryText, value);
    }

    public string PackageOperationStatusText
    {
        get => _packageOperationStatusText;
        private set => SetProperty(ref _packageOperationStatusText, value);
    }

    public QuestAwakeActionOption SelectedQuestAwakeAction
    {
        get => _selectedQuestAwakeAction;
        set
        {
            if (value is not null &&
                CurrentQuestAwakeOperation is null &&
                SetProperty(ref _selectedQuestAwakeAction, value))
            {
                OnPropertyChanged(nameof(QuestAwakeActionHelpText));
                OnPropertyChanged(nameof(QuestAwakeConfirmationButtonText));
            }
        }
    }

    public string QuestAwakeActionHelpText => SelectedQuestAwakeAction.HelpText;

    public string QuestAwakeDurationMinutes
    {
        get => _questAwakeDurationMinutes;
        set
        {
            if (CurrentQuestAwakeOperation is null)
            {
                SetProperty(ref _questAwakeDurationMinutes, value);
            }
        }
    }

    public string QuestAwakeWatchdogIntervalSeconds
    {
        get => _questAwakeWatchdogIntervalSeconds;
        set
        {
            if (CurrentQuestAwakeOperation is null)
            {
                SetProperty(ref _questAwakeWatchdogIntervalSeconds, value);
            }
        }
    }

    public QuestAwakeOperation? CurrentQuestAwakeOperation
    {
        get => _currentQuestAwakeOperation;
        private set
        {
            if (SetProperty(ref _currentQuestAwakeOperation, value))
            {
                OnPropertyChanged(nameof(IsQuestAwakeInputLocked));
                OnPropertyChanged(nameof(IsQuestAwakeInputUnlocked));
                OnPropertyChanged(nameof(QuestAwakeInputLockText));
                OnPropertyChanged(nameof(CanConfirmQuestAwake));
                OnPropertyChanged(nameof(QuestAwakeConfirmationButtonText));
                PreviewQuestAwakeCommand.RaiseCanExecuteChanged();
                ConfirmQuestAwakeCommand.RaiseCanExecuteChanged();
                RefreshQuestAwakeCommand.RaiseCanExecuteChanged();
                DismissQuestAwakeCommand.RaiseCanExecuteChanged();
            }
        }
    }

    public bool IsQuestAwakeInputLocked => CurrentQuestAwakeOperation is not null;

    public bool IsQuestAwakeInputUnlocked => !IsQuestAwakeInputLocked;

    public string QuestAwakeInputLockText => IsQuestAwakeInputLocked
        ? "Locked to immutable preview"
        : string.Empty;

    public bool CanConfirmQuestAwake =>
        !IsBusy &&
        _source is not null &&
        CurrentQuestAwakeOperation is
        {
            Lifecycle: "proposed",
            ConfirmedAtMs: null
        } operation &&
        operation.Targets.Any(target => target.Preflight.Eligible);

    public string QuestAwakeConfirmationButtonText =>
        CurrentQuestAwakeOperation?.ConfirmedAtMs is not null
            ? "Confirmed"
            : SelectedQuestAwakeAction.Action == QuestAwakeActions.Status
                ? "Confirm status check"
                : "Confirm and apply";

    public string QuestAwakeSummaryText
    {
        get => _questAwakeSummaryText;
        private set => SetProperty(ref _questAwakeSummaryText, value);
    }

    public string QuestAwakeStatusText
    {
        get => _questAwakeStatusText;
        private set => SetProperty(ref _questAwakeStatusText, value);
    }

    public bool IsBusy
    {
        get => _isBusy;
        private set
        {
            if (SetProperty(ref _isBusy, value))
            {
                ConnectCommand.RaiseCanExecuteChanged();
                SynchronizeUpdatesCommand.RaiseCanExecuteChanged();
                ApplySearchCommand.RaiseCanExecuteChanged();
                ClearSearchCommand.RaiseCanExecuteChanged();
                ApplyQueuedOrderingChangesCommand.RaiseCanExecuteChanged();
                ApplySavedViewCommand.RaiseCanExecuteChanged();
                DeleteSavedViewCommand.RaiseCanExecuteChanged();
                PreviewKioskShowControlsCommand.RaiseCanExecuteChanged();
                ConfirmOperationCommand.RaiseCanExecuteChanged();
                RefreshOperationCommand.RaiseCanExecuteChanged();
                DismissOperationCommand.RaiseCanExecuteChanged();
                PreviewPackageInstallReleaseCommand.RaiseCanExecuteChanged();
                OnPropertyChanged(nameof(CanConfirmPackageInstallRelease));
                ConfirmPackageInstallReleaseCommand.RaiseCanExecuteChanged();
                RefreshPackageInstallReleaseCommand.RaiseCanExecuteChanged();
                DismissPackageInstallReleaseCommand.RaiseCanExecuteChanged();
                PreviewQuestAwakeCommand.RaiseCanExecuteChanged();
                OnPropertyChanged(nameof(CanConfirmQuestAwake));
                ConfirmQuestAwakeCommand.RaiseCanExecuteChanged();
                RefreshQuestAwakeCommand.RaiseCanExecuteChanged();
                DismissQuestAwakeCommand.RaiseCanExecuteChanged();
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

    public ulong WatchSequence => _watchSequence;

    public bool WatchInitialized => _watchInitialized;

    public string OrderingChangesText => HasQueuedOrderingChanges
        ? $"{_queuedOrderingChangeCount:N0} live row changes affect the current " +
          "order or grouping"
        : "Live ordering is current";

    public async Task InitializeAsync()
    {
        await RefreshAsync();
        if (_lastRefreshSucceeded)
        {
            await EstablishWatchCursorAsync();
        }
        await TryLoadSavedViewsAsync();
    }

    public async Task PreviewKioskShowControlsAsync()
    {
        if (_source is null)
        {
            OperationStatusText = "Not connected to a Fleet Hub";
            return;
        }

        if (_batchSelection.Count == 0)
        {
            OperationStatusText =
                "Select at least one exact device before previewing Show Kiosk controls";
            return;
        }

        var requestedTargets = new SortedDictionary<string, ulong>(
            _batchSelection,
            StringComparer.Ordinal);
        var request = new OperationPreviewRequest
        {
            Targets = requestedTargets
        };
        IsBusy = true;
        OperationStatusText =
            $"Requesting immutable preview for {requestedTargets.Count} exact device(s)";
        try
        {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            var operation = await _source.PreviewOperationAsync(request, timeout.Token);
            ValidateOperationBinding(operation, request.ActionId, requestedTargets);
            ProjectOperation(operation);
            OperationStatusText =
                "Preview ready · review every target, then explicitly confirm this operation";
        }
        catch (Exception error) when (IsProjectionFailure(error))
        {
            OperationStatusText =
                $"Preview failed · prior operation projection retained · {error.Message}";
        }
        finally
        {
            IsBusy = false;
        }
    }

    public async Task ConfirmOperationAsync()
    {
        var prior = CurrentOperation;
        if (_source is null || prior is null)
        {
            OperationStatusText = "Preview an operation before confirming it";
            return;
        }

        var request = new OperationExecuteRequest
        {
            OperationId = prior.OperationId,
            PreviewId = prior.Preview.PreviewId
        };
        IsBusy = true;
        OperationStatusText =
            $"Confirming operation {prior.OperationId} against immutable preview " +
            prior.Preview.PreviewId;
        try
        {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            var operation = await _source.ExecuteOperationAsync(request, timeout.Token);
            ValidateOperationBinding(
                operation,
                prior.ActionId,
                PreviewIdentities(prior));
            RequireSameOperation(prior, operation);
            ProjectOperation(operation);
            OperationStatusText =
                "Execution accepted · per-device results below remain server-owned";
        }
        catch (Exception error) when (IsProjectionFailure(error))
        {
            OperationStatusText =
                $"Confirmation failed · prior operation projection retained · {error.Message}";
        }
        finally
        {
            IsBusy = false;
        }
    }

    public async Task RefreshOperationAsync()
    {
        var prior = CurrentOperation;
        if (_source is null || prior is null)
        {
            OperationStatusText = "No operation is available to refresh";
            return;
        }

        IsBusy = true;
        OperationStatusText = $"Refreshing operation {prior.OperationId}";
        try
        {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            var operation = await _source.OperationAsync(
                prior.OperationId,
                timeout.Token);
            ValidateOperationBinding(
                operation,
                prior.ActionId,
                PreviewIdentities(prior));
            RequireSameOperation(prior, operation);
            ProjectOperation(operation);
            OperationStatusText =
                "Operation results refreshed from the Fleet Hub";
        }
        catch (Exception error) when (IsProjectionFailure(error))
        {
            OperationStatusText =
                $"Refresh failed · prior operation projection retained · {error.Message}";
        }
        finally
        {
            IsBusy = false;
        }
    }

    public void DismissOperation()
    {
        CurrentOperation = null;
        OperationTargets.Clear();
        OperationSummaryText = "No kiosk operation preview";
        OperationStatusText =
            "Select exact devices, then preview Show Kiosk controls";
    }

    public async Task PreviewPackageInstallReleaseAsync()
    {
        if (_source is null)
        {
            PackageOperationStatusText = "Not connected to a Fleet Hub";
            return;
        }
        if (_batchSelection.Count == 0)
        {
            PackageOperationStatusText =
                "Select at least one exact device before previewing a package operation";
            return;
        }

        var manifestUrl = PackageManifestUrl.Trim();
        var packageName = PackageName.Trim();
        var rolloutRing = PackageRolloutRing.Trim();
        if (!Uri.TryCreate(manifestUrl, UriKind.Absolute, out var manifest) ||
            manifest.Scheme != Uri.UriSchemeHttps ||
            !string.IsNullOrEmpty(manifest.UserInfo) ||
            !string.IsNullOrEmpty(manifest.Fragment) ||
            string.IsNullOrWhiteSpace(packageName) ||
            string.IsNullOrWhiteSpace(rolloutRing))
        {
            PackageOperationStatusText =
                "Use a credential-free HTTPS manifest URL, Android package identity, and rollout ring";
            return;
        }

        var requestedTargets = new SortedDictionary<string, ulong>(
            _batchSelection,
            StringComparer.Ordinal);
        var request = new PackageInstallReleasePreviewRequest
        {
            Release = new PackageReleaseReference
            {
                Kind = "manifest_url",
                ManifestUrl = manifestUrl
            },
            ExpectedPackageName = packageName,
            ExpectedRolloutRing = rolloutRing,
            Targets = requestedTargets
        };
        IsBusy = true;
        PackageOperationStatusText =
            $"Requesting immutable package preview for {requestedTargets.Count} exact device(s)";
        try
        {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            var operation = await _source.PreviewPackageInstallReleaseAsync(
                request,
                timeout.Token);
            ValidatePackageOperationBinding(operation, request);
            ProjectPackageOperation(operation);
            PackageOperationStatusText =
                "Preview ready · review every target and signed release reference before confirming";
        }
        catch (Exception error) when (IsProjectionFailure(error))
        {
            PackageOperationStatusText =
                $"Preview failed · prior package projection retained · {error.Message}";
        }
        finally
        {
            IsBusy = false;
        }
    }

    public async Task ConfirmPackageInstallReleaseAsync()
    {
        var prior = CurrentPackageOperation;
        if (_source is null || prior is null)
        {
            PackageOperationStatusText =
                "Preview a package operation before confirming it";
            return;
        }
        if (!IsPackagePreviewReady(prior))
        {
            PackageOperationStatusText = prior.Lifecycle == "accepted"
                ? "Preparation is already accepted · refresh or close the operation view"
                : "The current package operation is not ready for confirmation";
            return;
        }
        var request = new PackageInstallReleaseExecuteRequest
        {
            OperationId = prior.OperationId,
            PreviewId = prior.Preview.PreviewId
        };
        IsBusy = true;
        PackageOperationStatusText =
            $"Confirming package operation {prior.OperationId} against immutable preview " +
            prior.Preview.PreviewId;
        try
        {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            var operation = await _source.ExecutePackageInstallReleaseAsync(
                request,
                timeout.Token);
            ValidatePackageOperationBinding(
                operation,
                prior.Preview.Release,
                prior.Preview.ExpectedPackageName,
                prior.Preview.ExpectedRolloutRing,
                PackagePreviewIdentities(prior));
            RequireSamePackageOperation(prior, operation);
            ProjectPackageOperation(operation);
            PackageOperationStatusText =
                "Prepared for owner delivery · authenticated updater ingress is unavailable · " +
                "no package was dispatched or installed";
        }
        catch (Exception error) when (IsProjectionFailure(error))
        {
            PackageOperationStatusText =
                $"Confirmation failed · prior package projection retained · {error.Message}";
        }
        finally
        {
            IsBusy = false;
        }
    }

    public async Task RefreshPackageInstallReleaseAsync()
    {
        var prior = CurrentPackageOperation;
        if (_source is null || prior is null)
        {
            PackageOperationStatusText = "No package operation is available to refresh";
            return;
        }
        IsBusy = true;
        PackageOperationStatusText = $"Refreshing package operation {prior.OperationId}";
        try
        {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            var operation = await _source.PackageInstallReleaseAsync(
                prior.OperationId,
                timeout.Token);
            ValidatePackageOperationBinding(
                operation,
                prior.Preview.Release,
                prior.Preview.ExpectedPackageName,
                prior.Preview.ExpectedRolloutRing,
                PackagePreviewIdentities(prior));
            RequireSamePackageOperation(prior, operation);
            ProjectPackageOperation(operation);
            PackageOperationStatusText =
                "Package operation refreshed · owner ingress remains unavailable · " +
                "no package dispatch or installation is claimed";
        }
        catch (Exception error) when (IsProjectionFailure(error))
        {
            PackageOperationStatusText =
                $"Refresh failed · prior package projection retained · {error.Message}";
        }
        finally
        {
            IsBusy = false;
        }
    }

    public void DismissPackageInstallRelease()
    {
        CurrentPackageOperation = null;
        PackageOperationTargets.Clear();
        PackageOperationSummaryText = "No package operation preview";
        PackageOperationStatusText =
            "Enter a signed manifest URL and package identity, then select exact devices";
    }

    public async Task PreviewQuestAwakeAsync()
    {
        if (_source is null)
        {
            QuestAwakeStatusText = "Not connected to a Fleet Hub";
            return;
        }
        if (_batchSelection.Count == 0)
        {
            QuestAwakeStatusText =
                "Select at least one exact device before previewing awake controls";
            return;
        }
        if (!TryReadQuestAwakePolicy(
                out var durationMs,
                out var watchdogIntervalMs,
                out var validationMessage))
        {
            QuestAwakeStatusText = validationMessage;
            return;
        }

        var requestedTargets = new SortedDictionary<string, ulong>(
            _batchSelection,
            StringComparer.Ordinal);
        var request = new QuestAwakePreviewRequest
        {
            Action = SelectedQuestAwakeAction.Action,
            DurationMs = durationMs,
            WatchdogIntervalMs = watchdogIntervalMs,
            Targets = requestedTargets
        };
        IsBusy = true;
        QuestAwakeStatusText =
            $"Requesting immutable awake-control preview for " +
            $"{requestedTargets.Count} exact device(s)";
        try
        {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            var operation = await _source.PreviewQuestAwakeAsync(
                request,
                timeout.Token);
            ValidateQuestAwakeBinding(operation, request);
            ProjectQuestAwakeOperation(operation);
            QuestAwakeStatusText =
                "Preview ready · review every target and the exact action, then confirm";
        }
        catch (Exception error) when (IsProjectionFailure(error))
        {
            QuestAwakeStatusText =
                $"Preview failed · prior awake-control projection retained · {error.Message}";
        }
        finally
        {
            IsBusy = false;
        }
    }

    public async Task ConfirmQuestAwakeAsync()
    {
        var prior = CurrentQuestAwakeOperation;
        if (_source is null || prior is null)
        {
            QuestAwakeStatusText =
                "Preview an awake-control operation before confirming it";
            return;
        }
        if (!CanConfirmQuestAwake)
        {
            QuestAwakeStatusText = prior.ConfirmedAtMs is not null
                ? "This immutable awake-control preview is already confirmed"
                : "The current awake-control preview has no eligible target to confirm";
            return;
        }

        var request = new QuestAwakeExecuteRequest
        {
            OperationId = prior.OperationId,
            PreviewId = prior.Preview.PreviewId
        };
        IsBusy = true;
        QuestAwakeStatusText =
            $"Confirming operation {prior.OperationId} against immutable preview " +
            prior.Preview.PreviewId;
        try
        {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            var operation = await _source.ExecuteQuestAwakeAsync(
                request,
                timeout.Token);
            ValidateQuestAwakeBinding(
                operation,
                prior.Preview.Action,
                prior.Preview.DurationMs,
                prior.Preview.WatchdogIntervalMs,
                QuestAwakePreviewIdentities(prior));
            RequireSameQuestAwakeOperation(prior, operation);
            ProjectQuestAwakeOperation(operation);
            QuestAwakeStatusText =
                "Action accepted · refresh for current per-device readbacks";
        }
        catch (Exception error) when (IsProjectionFailure(error))
        {
            QuestAwakeStatusText =
                $"Confirmation failed · prior awake-control projection retained · " +
                error.Message;
        }
        finally
        {
            IsBusy = false;
        }
    }

    public async Task RefreshQuestAwakeAsync()
    {
        var prior = CurrentQuestAwakeOperation;
        if (_source is null || prior is null)
        {
            QuestAwakeStatusText =
                "No awake-control operation is available to refresh";
            return;
        }

        IsBusy = true;
        QuestAwakeStatusText =
            $"Refreshing awake-control operation {prior.OperationId}";
        try
        {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            var operation = await _source.QuestAwakeAsync(
                prior.OperationId,
                timeout.Token);
            ValidateQuestAwakeBinding(
                operation,
                prior.Preview.Action,
                prior.Preview.DurationMs,
                prior.Preview.WatchdogIntervalMs,
                QuestAwakePreviewIdentities(prior));
            RequireSameQuestAwakeOperation(prior, operation);
            ProjectQuestAwakeOperation(operation);
            QuestAwakeStatusText =
                "Awake-control results refreshed from the Fleet Hub";
        }
        catch (Exception error) when (IsProjectionFailure(error))
        {
            QuestAwakeStatusText =
                $"Refresh failed · prior awake-control projection retained · " +
                error.Message;
        }
        finally
        {
            IsBusy = false;
        }
    }

    public void DismissQuestAwake()
    {
        CurrentQuestAwakeOperation = null;
        QuestAwakeTargets.Clear();
        QuestAwakeSummaryText = "No headset awake-control preview";
        QuestAwakeStatusText =
            "Choose an action, select exact devices, then preview";
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

    public async Task SynchronizeUpdatesAsync()
    {
        if (_source is null)
        {
            StatusMessage = "Not connected to a Fleet Hub";
            return;
        }

        if (!_watchInitialized)
        {
            await EstablishWatchCursorAsync();
            if (!_watchInitialized)
            {
                var watchFailure = StatusMessage;
                await RefreshAsync();
                if (_lastRefreshSucceeded)
                {
                    StatusMessage =
                        $"Connected · canonical scope refreshed without watch · " +
                        $"{_watchFailureReason ?? watchFailure}";
                }
                return;
            }
        }

        var priorSequence = _watchSequence;
        IReadOnlyList<FleetWatchEvent> events;
        var sequenceReset = false;
        using var cancellation = new CancellationTokenSource(TimeSpan.FromSeconds(12));
        IsBusy = true;
        StatusMessage = $"Checking Hub updates after event {priorSequence:N0}";
        try
        {
            events = await _source.WatchAsync(
                priorSequence,
                WatchEventLimit,
                cancellation.Token);
            FleetProjectionValidation.ValidateWatchEvents(
                events,
                priorSequence,
                WatchEventLimit);

            if (events.Count == 0 && priorSequence > 0)
            {
                var baseline = await _source.WatchAsync(
                    0,
                    WatchEventLimit,
                    cancellation.Token);
                FleetProjectionValidation.ValidateWatchEvents(
                    baseline,
                    0,
                    WatchEventLimit);
                var baselineTail = baseline.LastOrDefault()?.EventSequence ?? 0;
                if (baselineTail < priorSequence)
                {
                    events = baseline;
                    sequenceReset = true;
                }
            }
        }
        catch (Exception error) when (
            error is HttpRequestException or JsonException or TaskCanceledException or
            InvalidOperationException or ArgumentOutOfRangeException)
        {
            StatusMessage = $"Update check failed · cached rows retained · {error.Message}";
            return;
        }
        finally
        {
            IsBusy = false;
        }

        await RefreshAsync();
        if (!_lastRefreshSucceeded)
        {
            return;
        }

        SetWatchCursor(events.LastOrDefault()?.EventSequence ??
                       (sequenceReset ? 0 : priorSequence));
        var accepted = events.Count(item => item.Decision.Kind == "accepted");
        var rejected = events.Count - accepted;
        StatusMessage = sequenceReset
            ? $"Connected · Hub watch sequence reset · canonical scope reloaded · " +
              $"{accepted:N0} accepted / {rejected:N0} rejected baseline events"
            : events.Count == 0
                ? "Connected · no new Hub events · canonical scope verified"
                : $"Connected · {accepted:N0} accepted / {rejected:N0} rejected Hub " +
                  $"events consumed · event {_watchSequence:N0}" +
                  (HasQueuedOrderingChanges ? " · ordering changes await application" : string.Empty);
    }

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
        _lastRefreshSucceeded = false;
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
            _lastRefreshSucceeded = true;
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
            ResetWatchCursor();
            await RefreshAsync();
            if (_lastRefreshSucceeded)
            {
                await EstablishWatchCursorAsync();
            }
            await TryLoadSavedViewsAsync();
        }
        catch (ArgumentException error)
        {
            StatusMessage = error.Message;
        }
    }

    private async Task EstablishWatchCursorAsync()
    {
        if (_source is null)
        {
            return;
        }

        using var cancellation = new CancellationTokenSource(TimeSpan.FromSeconds(12));
        try
        {
            var events = await _source.WatchAsync(
                0,
                WatchEventLimit,
                cancellation.Token);
            FleetProjectionValidation.ValidateWatchEvents(events, 0, WatchEventLimit);
            _watchFailureReason = null;
            SetWatchCursor(events.LastOrDefault()?.EventSequence ?? 0);
        }
        catch (Exception error) when (
            error is HttpRequestException or JsonException or TaskCanceledException or
            InvalidOperationException or ArgumentOutOfRangeException)
        {
            ResetWatchCursor();
            _watchFailureReason = error.Message;
            StatusMessage = $"Connected · canonical scope available · Hub watch unavailable · " +
                            error.Message;
        }
    }

    private void SetWatchCursor(ulong sequence)
    {
        _watchSequence = sequence;
        _watchInitialized = true;
        OnPropertyChanged(nameof(WatchSequence));
        OnPropertyChanged(nameof(WatchInitialized));
    }

    private void ResetWatchCursor()
    {
        _watchSequence = 0;
        _watchInitialized = false;
        _watchFailureReason = null;
        OnPropertyChanged(nameof(WatchSequence));
        OnPropertyChanged(nameof(WatchInitialized));
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
        PreviewKioskShowControlsCommand.RaiseCanExecuteChanged();
        PreviewPackageInstallReleaseCommand.RaiseCanExecuteChanged();
        PreviewQuestAwakeCommand.RaiseCanExecuteChanged();
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

    private void ProjectOperation(OperationLedger operation)
    {
        FleetProjectionValidation.ValidateOperationLedger(operation);
        var targets = operation.Targets
            .OrderBy(target => target.DeviceId, StringComparer.Ordinal)
            .Select(target => new OperationTargetViewModel(target))
            .ToArray();

        CurrentOperation = operation;
        OperationTargets.Clear();
        foreach (var target in targets)
        {
            OperationTargets.Add(target);
        }

        var eligible = operation.Targets.Count(target =>
            target.Preflight.Eligible);
        var excluded = operation.Targets.Count - eligible;
        OperationSummaryText =
            $"Show Kiosk controls · operation {operation.OperationId} · " +
            $"preview {operation.Preview.PreviewId} · " +
            $"{operation.Targets.Count} exact target(s) · {eligible} eligible · " +
            $"{excluded} excluded · " +
            $"lifecycle {DeviceRowViewModel.Title(operation.Lifecycle)}" +
            (operation.CleanupRequired ? " · cleanup required" : string.Empty);
    }

    private void ProjectPackageOperation(PackageInstallReleaseOperation operation)
    {
        FleetProjectionValidation.ValidatePackageInstallReleaseOperation(operation);
        var targets = operation.Targets
            .OrderBy(target => target.DeviceId, StringComparer.Ordinal)
            .Select(target => new PackageOperationTargetViewModel(target))
            .ToArray();
        CurrentPackageOperation = operation;
        PackageOperationTargets.Clear();
        foreach (var target in targets)
        {
            PackageOperationTargets.Add(target);
        }

        var eligible = operation.Targets.Count(target => target.Preflight.Eligible);
        var prepared = operation.Targets.Count(target => target.Stage == "dispatch_ready");
        var excluded = operation.Targets.Count - eligible;
        var releaseReference = operation.Preview.Release.Kind == "manifest_url"
            ? operation.Preview.Release.ManifestUrl
            : operation.Preview.Release.ReleaseId;
        PackageOperationSummaryText =
            $"Package install/release · operation {operation.OperationId} · " +
            $"preview {operation.Preview.PreviewId} · " +
            $"release {releaseReference} · " +
            $"package {operation.Preview.ExpectedPackageName} · " +
            $"ring {operation.Preview.ExpectedRolloutRing} · " +
            $"{operation.Targets.Count} exact target(s) · {eligible} eligible · " +
            $"{excluded} excluded · {prepared} prepared only · " +
            $"lifecycle {DeviceRowViewModel.Title(operation.Lifecycle)}";
    }

    private void ProjectQuestAwakeOperation(QuestAwakeOperation operation)
    {
        QuestAwakeProjectionValidation.ValidateOperation(operation);
        var targets = operation.Targets
            .OrderBy(target => target.DeviceId, StringComparer.Ordinal)
            .Select(target => new QuestAwakeTargetViewModel(target))
            .ToArray();
        CurrentQuestAwakeOperation = operation;
        QuestAwakeTargets.Clear();
        foreach (var target in targets)
        {
            QuestAwakeTargets.Add(target);
        }

        var eligible = operation.Targets.Count(target => target.Preflight.Eligible);
        var excluded = operation.Targets.Count - eligible;
        var effective = operation.Targets.Count(target =>
            target.Receipt?.Effective == true);
        QuestAwakeSummaryText =
            $"{QuestAwakeActionLabel(operation.Preview.Action)} · " +
            $"operation {operation.OperationId} · " +
            $"preview {operation.Preview.PreviewId} · " +
            $"duration {operation.Preview.DurationMs / 60_000d:0.#} min · " +
            $"check every {operation.Preview.WatchdogIntervalMs / 1_000d:0.#} sec · " +
            $"{operation.Targets.Count} exact target(s) · {eligible} eligible · " +
            $"{excluded} excluded · {effective} effective · " +
            $"lifecycle {DeviceRowViewModel.Title(operation.Lifecycle)}";
    }

    private static bool IsPackagePreviewReady(PackageInstallReleaseOperation? operation)
    {
        if (operation is null || operation.Lifecycle != "proposed")
        {
            return false;
        }

        var eligible = operation.Targets
            .Where(target => target.Preflight.Eligible)
            .ToArray();
        return eligible.Length > 0 &&
               eligible.All(target =>
                   target.Lifecycle == "proposed" &&
                   target.Stage == "preview_ready");
    }

    private static void ValidateOperationBinding(
        OperationLedger operation,
        string expectedActionId,
        IReadOnlyDictionary<string, ulong> expectedTargets)
    {
        FleetProjectionValidation.ValidateOperationLedger(operation);
        if (operation.ActionId != expectedActionId ||
            operation.Preview.Targets.Count != expectedTargets.Count ||
            expectedTargets.Any(target =>
                !operation.Preview.Targets.Any(previewTarget =>
                    previewTarget.DeviceId == target.Key &&
                    previewTarget.IdentityRevision == target.Value)))
        {
            throw new InvalidOperationException(
                "Fleet Hub returned operation evidence for a different action or immutable preview.");
        }
    }

    private static void RequireSameOperation(
        OperationLedger expected,
        OperationLedger actual)
    {
        if (actual.OperationId != expected.OperationId ||
            actual.Preview.PreviewId != expected.Preview.PreviewId)
        {
            throw new InvalidOperationException(
                "Fleet Hub returned a different operation or immutable preview.");
        }
    }

    private static IReadOnlyDictionary<string, ulong> PreviewIdentities(
        OperationLedger operation) =>
        operation.Preview.Targets.ToDictionary(
            target => target.DeviceId,
            target => target.IdentityRevision,
            StringComparer.Ordinal);

    private static IReadOnlyDictionary<string, ulong> PackagePreviewIdentities(
        PackageInstallReleaseOperation operation) =>
        operation.Preview.Targets.ToDictionary(
            target => target.DeviceId,
            target => target.IdentityRevision,
            StringComparer.Ordinal);

    private static void ValidatePackageOperationBinding(
        PackageInstallReleaseOperation operation,
        PackageInstallReleasePreviewRequest request) =>
        ValidatePackageOperationBinding(
            operation,
            request.Release,
            request.ExpectedPackageName,
            request.ExpectedRolloutRing,
            request.Targets);

    private static void ValidatePackageOperationBinding(
        PackageInstallReleaseOperation operation,
        PackageReleaseReference expectedRelease,
        string expectedPackageName,
        string expectedRolloutRing,
        IReadOnlyDictionary<string, ulong> expectedTargets)
    {
        FleetProjectionValidation.ValidatePackageInstallReleaseOperation(operation);
        if (operation.ActionId != PackageOperationActions.InstallRelease ||
            JsonSerializer.Serialize(operation.Preview.Release, FleetJson.Options) !=
            JsonSerializer.Serialize(expectedRelease, FleetJson.Options) ||
            operation.Preview.ExpectedPackageName != expectedPackageName ||
            operation.Preview.ExpectedRolloutRing != expectedRolloutRing ||
            operation.Preview.Targets.Count != expectedTargets.Count ||
            expectedTargets.Any(target =>
                operation.Preview.Targets.All(preflight =>
                    preflight.DeviceId != target.Key ||
                    preflight.IdentityRevision != target.Value)))
        {
            throw new InvalidOperationException(
                "Fleet Hub package operation does not bind the exact release and target identities.");
        }
    }

    private static void RequireSamePackageOperation(
        PackageInstallReleaseOperation prior,
        PackageInstallReleaseOperation current)
    {
        if (prior.OperationId != current.OperationId ||
            prior.Preview.PreviewId != current.Preview.PreviewId ||
            JsonSerializer.Serialize(prior.Preview, FleetJson.Options) !=
            JsonSerializer.Serialize(current.Preview, FleetJson.Options))
        {
            throw new InvalidOperationException(
                "Fleet Hub changed immutable package operation facts.");
        }
    }

    private bool TryReadQuestAwakePolicy(
        out uint durationMs,
        out uint watchdogIntervalMs,
        out string validationMessage)
    {
        durationMs = 0;
        watchdogIntervalMs = 0;
        if (!uint.TryParse(QuestAwakeDurationMinutes.Trim(), out var minutes) ||
            minutes is < 1 or > 480)
        {
            validationMessage =
                "Keep-awake duration must be a whole number from 1 through 480 minutes";
            return false;
        }
        if (!uint.TryParse(
                QuestAwakeWatchdogIntervalSeconds.Trim(),
                out var seconds) ||
            seconds is < 1 or > 60)
        {
            validationMessage =
                "Watchdog check interval must be a whole number from 1 through 60 seconds";
            return false;
        }

        durationMs = minutes * 60_000;
        watchdogIntervalMs = seconds * 1_000;
        validationMessage = string.Empty;
        return true;
    }

    private static void ValidateQuestAwakeBinding(
        QuestAwakeOperation operation,
        QuestAwakePreviewRequest request) =>
        ValidateQuestAwakeBinding(
            operation,
            request.Action,
            request.DurationMs,
            request.WatchdogIntervalMs,
            request.Targets);

    private static void ValidateQuestAwakeBinding(
        QuestAwakeOperation operation,
        string expectedAction,
        uint expectedDurationMs,
        uint expectedWatchdogIntervalMs,
        IReadOnlyDictionary<string, ulong> expectedTargets)
    {
        QuestAwakeProjectionValidation.ValidateOperation(operation);
        if (operation.ActionId != QuestAwakeActions.ActionId ||
            operation.Preview.Action != expectedAction ||
            operation.Preview.DurationMs != expectedDurationMs ||
            operation.Preview.WatchdogIntervalMs != expectedWatchdogIntervalMs ||
            operation.Preview.Targets.Count != expectedTargets.Count ||
            expectedTargets.Any(target =>
                operation.Preview.Targets.All(preflight =>
                    preflight.DeviceId != target.Key ||
                    preflight.IdentityRevision != target.Value)))
        {
            throw new InvalidOperationException(
                "Fleet Hub awake-control operation does not bind the exact action, policy, and target identities.");
        }
    }

    private static void RequireSameQuestAwakeOperation(
        QuestAwakeOperation prior,
        QuestAwakeOperation current)
    {
        if (prior.OperationId != current.OperationId ||
            prior.Preview.PreviewId != current.Preview.PreviewId ||
            JsonSerializer.Serialize(prior.Preview, FleetJson.Options) !=
            JsonSerializer.Serialize(current.Preview, FleetJson.Options))
        {
            throw new InvalidOperationException(
                "Fleet Hub changed immutable awake-control operation facts.");
        }
    }

    private static IReadOnlyDictionary<string, ulong> QuestAwakePreviewIdentities(
        QuestAwakeOperation operation) =>
        operation.Preview.Targets.ToDictionary(
            target => target.DeviceId,
            target => target.IdentityRevision,
            StringComparer.Ordinal);

    private static string QuestAwakeActionLabel(string action) =>
        action switch
        {
            QuestAwakeActions.Status => "Check current awake status",
            QuestAwakeActions.ApplyBounded => "Meta keep-awake (up to 8 hours)",
            QuestAwakeActions.StartWindowsWatchdog => "Windows watchdog",
            QuestAwakeActions.StartDeviceWatchdog =>
                "Quest watchdog (stops on reboot)",
            QuestAwakeActions.StopWatchdogs =>
                "Stop watchdogs only (settings remain)",
            QuestAwakeActions.RestoreNormal => "Restore normal sleep settings",
            _ => DeviceRowViewModel.Title(action)
        };

    private static bool IsProjectionFailure(Exception error) =>
        error is HttpRequestException or JsonException or TaskCanceledException or
            InvalidOperationException;

    private void ClearBatchSelection()
    {
        foreach (var row in Rows)
        {
            row.IsBatchSelected = false;
        }

        _batchSelection.Clear();
        OnPropertyChanged(nameof(BatchSelectionText));
        PreviewKioskShowControlsCommand.RaiseCanExecuteChanged();
        PreviewPackageInstallReleaseCommand.RaiseCanExecuteChanged();
        PreviewQuestAwakeCommand.RaiseCanExecuteChanged();
    }

    private void SelectAllVisible()
    {
        foreach (var row in Rows)
        {
            row.IsBatchSelected = true;
        }

        OnPropertyChanged(nameof(BatchSelectionText));
        PreviewKioskShowControlsCommand.RaiseCanExecuteChanged();
        PreviewPackageInstallReleaseCommand.RaiseCanExecuteChanged();
        PreviewQuestAwakeCommand.RaiseCanExecuteChanged();
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
            PreviewKioskShowControlsCommand.RaiseCanExecuteChanged();
            PreviewPackageInstallReleaseCommand.RaiseCanExecuteChanged();
            PreviewQuestAwakeCommand.RaiseCanExecuteChanged();
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
