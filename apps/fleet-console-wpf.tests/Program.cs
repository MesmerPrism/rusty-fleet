// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Diagnostics;
using System.IO;
using System.Net.Http;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Automation.Peers;
using System.Windows.Automation.Provider;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using RustyFleet.FleetConsole.Contracts;
using RustyFleet.FleetConsole.Services;
using RustyFleet.FleetConsole.ViewModels;

namespace RustyFleet.FleetConsole.Tests;

internal static class Program
{
    [STAThread]
    private static int Main(string[] arguments)
    {
        try
        {
            var repoRoot = ReadRepoRoot(arguments);
            WindowsHotspotContractTests.Run();
            ProviderCatalogContractTests.Run();
            var kioskOperationFixture = JsonSerializer.Deserialize<OperationLedger>(
                File.ReadAllText(
                    Path.Combine(
                        repoRoot,
                        "fixtures",
                        "contracts",
                        "kiosk-show-controls-operation.valid.json")),
                FleetJson.Options) ?? throw new JsonException(
                    "Kiosk show-controls operation fixture was empty.");
            FleetProjectionValidation.ValidateOperationLedger(kioskOperationFixture);
            var damagedKioskOperationRejected = false;
            try
            {
                var damagedOperation = JsonSerializer.Deserialize<OperationLedger>(
                    File.ReadAllText(
                        Path.Combine(
                            repoRoot,
                            "fixtures",
                            "contracts",
                            "kiosk-show-controls-operation.damaged.json")),
                    FleetJson.Options) ?? throw new JsonException(
                        "Damaged Kiosk show-controls operation fixture was empty.");
                FleetProjectionValidation.ValidateOperationLedger(damagedOperation);
            }
            catch (InvalidOperationException)
            {
                damagedKioskOperationRejected = true;
            }

            Require(
                damagedKioskOperationRejected,
                "damaged Rust-owned Kiosk show-controls fixture was accepted");
            var packageOperationFixture =
                JsonSerializer.Deserialize<PackageInstallReleaseOperation>(
                    File.ReadAllText(
                        Path.Combine(
                            repoRoot,
                            "fixtures",
                            "contracts",
                            "package-install-release-operation.valid.json")),
                    FleetJson.Options) ?? throw new JsonException(
                    "Package install/release operation fixture was empty.");
            FleetProjectionValidation.ValidatePackageInstallReleaseOperation(
                packageOperationFixture);
            var damagedPackageOperationRejected = false;
            try
            {
                var damagedPackageOperation =
                    JsonSerializer.Deserialize<PackageInstallReleaseOperation>(
                        File.ReadAllText(
                            Path.Combine(
                                repoRoot,
                                "fixtures",
                                "contracts",
                                "package-install-release-operation.damaged.json")),
                        FleetJson.Options) ?? throw new JsonException(
                        "Damaged package install/release operation fixture was empty.");
                FleetProjectionValidation.ValidatePackageInstallReleaseOperation(
                    damagedPackageOperation);
            }
            catch (InvalidOperationException)
            {
                damagedPackageOperationRejected = true;
            }

            Require(
                damagedPackageOperationRejected,
                "damaged Rust-owned package install/release fixture was accepted");
            var json = RunFleetctl(
                repoRoot,
                "operator-fixture",
                "mixed-freshness",
                "50");
            var deserializeWatch = Stopwatch.StartNew();
            FleetQueryResult projection;
            FleetSummaryProjection fixtureSummary;
            using (var fixtureDocument = JsonDocument.Parse(json))
            {
                var fixture = fixtureDocument.RootElement;
                Require(
                    fixture.GetProperty("schema").GetString() ==
                    "rusty.fleet.operator_fixture.v1",
                    "wrong operator fixture schema");
                Require(
                    fixture.GetProperty("profile").GetString() == "mixed_freshness",
                    "wrong operator fixture profile");
                projection = FleetJson.DeserializeQueryResult(
                    fixture.GetProperty("query_result").GetRawText());
                fixtureSummary = JsonSerializer.Deserialize<FleetSummaryProjection>(
                    fixture.GetProperty("summary").GetRawText(),
                    FleetJson.Options) ?? throw new JsonException(
                    "Fleet fixture summary was empty.");
            }
            deserializeWatch.Stop();
            Require(projection.Schema == "rusty.fleet.query_result.v1", "wrong query schema");
            Require(projection.TotalCount == 50, "50-device projection was not loaded");
            Require(projection.Rows.Count == 50, "query window is incomplete");
            Require(
                fixtureSummary is
                {
                    Total: 50,
                    Fresh: 25,
                    Stale: 13,
                    Offline: 12
                },
                "mixed-freshness summary drifted");
            ValidateRealisticScaleWindow(repoRoot, 10);
            ValidateRealisticScaleWindow(repoRoot, 100);
            var stressEvidence = ValidateVirtualizationStress(repoRoot);
            var downgradedRows = projection.Rows
                .Where(row =>
                    row.Capabilities.Capabilities.TryGetValue(
                        "participating_app_control",
                        out var capability) &&
                    capability.Authorization == "unauthorized")
                .ToArray();
            Require(
                downgradedRows.Length == 6 &&
                new DeviceRowViewModel(downgradedRows[0]).ControlText.Contains(
                    "Unauthorized",
                    StringComparison.Ordinal),
                "capability downgrade was not truthfully projected");
            Require(
                projection.Rows.Any(row =>
                    row.Freshness == "stale" &&
                    new DeviceRowViewModel(row).FreshnessText.StartsWith(
                        "Stale",
                        StringComparison.Ordinal)) &&
                projection.Rows.Any(row =>
                    row.Freshness == "offline" &&
                    new DeviceRowViewModel(row).RouteText == "Offline"),
                "stale/offline row grammar was not projected");
            using var loopbackClient = new FleetApiClient(new Uri("http://127.0.0.1:8741/"));
            var remoteRejected = false;
            try
            {
                using var _ = new FleetApiClient(new Uri("http://192.0.2.10:8741/"));
            }
            catch (ArgumentException)
            {
                remoteRejected = true;
            }

            Require(remoteRejected, "Fleet Console accepted a non-loopback Hub");
            Require(
                FleetApiClient.MaxResponseBytes == 16 * 1024 * 1024,
                "Fleet Console response budget drifted");
            var hubAddress = ReadOptionalArgument(arguments, "--hub-address");
            var liveHubChecked = false;
            if (hubAddress is not null)
            {
                using var liveClient = new FleetApiClient(new Uri(hubAddress));
                var liveQuery = FleetQuery.Create(null, limit: 100);
                var liveResult = liveClient.QueryAsync(
                        liveQuery,
                        CancellationToken.None)
                    .GetAwaiter()
                    .GetResult();
                var liveSummary = liveClient.SummaryAsync(CancellationToken.None)
                    .GetAwaiter()
                    .GetResult();
                var liveWatchEvents = liveClient.WatchAsync(
                        0,
                        100,
                        CancellationToken.None)
                    .GetAwaiter()
                    .GetResult();
                FleetProjectionValidation.ValidateQueryResult(
                    liveResult,
                    liveSummary,
                    liveQuery);
                FleetProjectionValidation.ValidateWatchEvents(
                    liveWatchEvents,
                    0,
                    100);
                if (liveResult.Rows.Count > 0)
                {
                    var liveRow = liveResult.Rows[0];
                    var liveInspector = liveClient.InspectAsync(
                            liveRow.Identity.DeviceId,
                            CancellationToken.None)
                        .GetAwaiter()
                        .GetResult();
                    FleetProjectionValidation.ValidateInspector(
                        liveInspector,
                        liveRow);
                    var liveDetail = liveClient.DetailAsync(
                            liveRow.Identity.DeviceId,
                            CancellationToken.None)
                        .GetAwaiter()
                        .GetResult();
                    FleetProjectionValidation.ValidateDetail(
                        liveDetail,
                        liveRow);
                }

                liveHubChecked = true;
            }

            var queryJson = JsonSerializer.Serialize(
                FleetQuery.Create(
                    "Quest 0001",
                    "Stale",
                    sortField: "battery_percent",
                    sortDirection: "descending"),
                FleetJson.Options);
            using (var document = JsonDocument.Parse(queryJson))
            {
                var expression = document.RootElement.GetProperty("expression");
                Require(expression.GetProperty("kind").GetString() == "and", "facets are not canonical AND");
                var expressions = expression.GetProperty("expressions");
                Require(
                    expressions.GetArrayLength() == 2 &&
                    expressions[0].GetProperty("kind").GetString() == "or" &&
                    expressions[0].GetProperty("expressions").GetArrayLength() == 2,
                    "search must target display name and device ID");
                Require(
                    expressions[1].GetProperty("field").GetString() == "freshness" &&
                    expressions[1].GetProperty("comparison").GetString() == "equals" &&
                    expressions[1].GetProperty("value").GetString() == "stale",
                    "freshness facet is not canonical");
                var sort = document.RootElement.GetProperty("sort");
                Require(
                    sort.GetArrayLength() == 1 &&
                    sort[0].GetProperty("field").GetString() == "battery_percent" &&
                    sort[0].GetProperty("direction").GetString() == "descending",
                    "sort scope is not canonical");
            }

            var source = new StaticFleetDataSource(
                projection,
                canonicalSummary: fixtureSummary);
            var workspace = new FleetWorkspaceViewModel(source);
            var viewModelWatch = Stopwatch.StartNew();
            workspace.InitializeAsync().GetAwaiter().GetResult();
            viewModelWatch.Stop();
            Require(workspace.Rows.Count == 50, "view model did not retain realistic fleet window");
            Require(viewModelWatch.Elapsed < TimeSpan.FromSeconds(2), "50-row view model exceeded 2 seconds");

            var first = workspace.Rows[0];
            var batchScopeChanged = false;
            workspace.PropertyChanged += (_, eventArgs) =>
            {
                if (eventArgs.PropertyName == nameof(workspace.BatchSelectionText))
                {
                    batchScopeChanged = true;
                }
            };
            first.IsBatchSelected = true;
            Require(batchScopeChanged, "direct batch selection did not update visible scope");
            workspace.SelectDeviceAsync(first).GetAwaiter().GetResult();
            var inspector = workspace.Inspector ??
                            throw new InvalidOperationException("inspector did not select device");
            Require(inspector.Title == first.DisplayName, "inspector selected the wrong device");
            Require(
                inspector.Capabilities.Count >= 3,
                "inspector did not preserve independent capabilities");
            var selectedStableKey = first.StableKey;
            var selectedBatchText = workspace.BatchSelectionText;
            Require(
                workspace.OpenFullDetailAsync("status").GetAwaiter().GetResult() &&
                workspace.IsDetailOpen &&
                workspace.SelectedDetailTab == "status" &&
                workspace.Detail?.Title == first.DisplayName,
                "full detail did not project the selected device");
            workspace.CloseFullDetail();
            Require(
                !workspace.IsDetailOpen &&
                workspace.SelectedDevice?.StableKey == selectedStableKey &&
                workspace.BatchSelectionText == selectedBatchText,
                "returning from full detail lost fleet context");

            workspace.SelectedSort = "Device name";
            workspace.SelectedSortDirection = "Descending";
            workspace.ApplyScopeAsync().GetAwaiter().GetResult();
            Require(
                source.LastQuery?.Sort is
                [
                    {
                        Field: "display_name",
                        Direction: "descending"
                    }
                ],
                "sort editor did not send the canonical Hub sort");
            Require(
                workspace.Rows[0].DisplayName == projection.Rows[^1].Identity.DisplayName,
                "canonical descending device-name sort was not projected");
            Require(
                first.IsBatchSelected &&
                workspace.SelectedDevice?.StableKey == first.StableKey,
                "explicit sorting lost batch or inspector identity");
            Require(
                workspace.ActiveScopeText.Contains(
                    "sorted by device name descending",
                    StringComparison.Ordinal),
                "applied sort is not visibly serialized");

            workspace.SelectedSort = "Battery";
            workspace.RefreshAsync().GetAwaiter().GetResult();
            Require(
                source.LastQuery?.Sort[0].Field == "display_name" &&
                source.LastQuery.Sort[0].Direction == "descending" &&
                workspace.SelectedSort == "Battery",
                "background refresh did not preserve the applied sort independently " +
                "from the editor");

            workspace.SearchText = "Quest 0001";
            workspace.SelectedFreshness = "Fresh";
            workspace.SelectedGrouping = "Cohort";
            workspace.SelectedSort = "Device name";
            workspace.SelectedSortDirection = "Ascending";
            workspace.ApplyScopeAsync().GetAwaiter().GetResult();
            Require(source.LastQuery?.Expression is not null, "search was not sent to the data source");
            Require(workspace.Rows.Count == 1, "combined scope did not narrow the projection");
            Require(
                workspace.ActiveScopeText.Contains("freshness = fresh", StringComparison.Ordinal) &&
                workspace.ActiveScopeText.Contains("grouped by cohort", StringComparison.Ordinal),
                "active scope is not visibly serialized");
            Require(workspace.RowsView.Groups?.Count == 1, "cohort grouping was not applied");

            workspace.SearchText = string.Empty;
            workspace.SelectedFreshness = "Offline";
            workspace.ApplyScopeAsync().GetAwaiter().GetResult();
            Require(
                workspace.Rows.Count == fixtureSummary.Offline,
                "offline filter did not match the canonical Hub summary");
            Require(
                workspace.BatchSelectionText.Contains("1 hidden by scope", StringComparison.Ordinal),
                "hidden batch selection was not retained");
            Require(
                workspace.Inspector?.Title == first.DisplayName &&
                workspace.InspectorContextText.Contains(
                    "outside the active scope",
                    StringComparison.Ordinal),
                "selected-device context was lost outside the active scope");

            workspace.SelectedFreshness = "Unknown";
            workspace.ApplyScopeAsync().GetAwaiter().GetResult();
            Require(workspace.Rows.Count == 0, "unknown filter did not produce an empty scope");
            Require(
                workspace.BatchSelectionText.Contains("1 hidden by scope", StringComparison.Ordinal) &&
                workspace.InspectorContextText.Contains(
                    "outside the active scope",
                    StringComparison.Ordinal),
                "empty scope lost selection or inspector context");

            var queryCountBeforeClear = source.QueryCount;
            workspace.ClearSearchAsync().GetAwaiter().GetResult();
            Require(workspace.SearchText.Length == 0, "clear search retained text");
            Require(source.QueryCount == queryCountBeforeClear + 1, "clear search did not reapply scope");
            Require(source.LastQuery?.Expression is null, "clear search retained a query expression");
            Require(
                workspace.Rows[0].IsBatchSelected &&
                workspace.SelectedDevice?.StableKey == workspace.Rows[0].StableKey,
                "clearing scope did not restore batch and inspection context");
            first = workspace.Rows[0];
            var firstReference = workspace.Rows[0];
            workspace.RefreshAsync().GetAwaiter().GetResult();
            Require(
                ReferenceEquals(firstReference, workspace.Rows[0]),
                "refresh replaced a stable interaction-bound row");
            Require(workspace.Rows[0].IsBatchSelected, "batch selection was lost on refresh");
            workspace.SelectedGrouping = "Cohort";
            workspace.ApplyScopeAsync().GetAwaiter().GetResult();
            Require(
                workspace.RowsView.Groups is { Count: 2 },
                "full cohort grouping did not retain both simulator cohorts");

            var savedSource = new StaticFleetDataSource(
                projection,
                canonicalSummary: fixtureSummary);
            var savedWorkspace = new FleetWorkspaceViewModel(savedSource);
            savedWorkspace.InitializeAsync().GetAwaiter().GetResult();
            savedWorkspace.SearchText = "Quest 0001";
            savedWorkspace.SelectedFreshness = "Fresh";
            savedWorkspace.SelectedGrouping = "Cohort";
            savedWorkspace.ApplyScopeAsync().GetAwaiter().GetResult();
            var savedSelection = savedWorkspace.Rows.Single();
            savedWorkspace.SelectDeviceAsync(savedSelection).GetAwaiter().GetResult();
            savedWorkspace.OpenFullDetailAsync("status").GetAwaiter().GetResult();
            savedWorkspace.SavedViewName = "Lab focus";
            savedWorkspace.SaveCurrentViewAsync(
                    [
                        "selection", "attention", "device", "age", "power",
                        "application"
                    ],
                    "detail")
                .GetAwaiter()
                .GetResult();
            Require(
                savedWorkspace.SavedViews is
                [
                    {
                        ViewId: "view.operator.lab_focus",
                        Name: "Lab focus",
                        Grouping: "cohort"
                    }
                ],
                "saved-view mutation was not reloaded from canonical Hub state");
            var savedView = savedWorkspace.SavedViews[0];
            var savedViewJson = JsonSerializer.Serialize(savedView.Query, FleetJson.Options);
            SavedView? restoredEvent = null;
            savedWorkspace.SavedViewRestorationRequested += view => restoredEvent = view;

            savedWorkspace.SearchText = string.Empty;
            savedWorkspace.SelectedFreshness = "Offline";
            savedWorkspace.SelectedGrouping = "None";
            savedWorkspace.ApplyScopeAsync().GetAwaiter().GetResult();
            Require(
                savedWorkspace.Rows.Count == fixtureSummary.Offline,
                "saved-view precondition did not leave the saved scope");
            savedWorkspace.SelectedSavedView = savedView;
            savedWorkspace.ApplySavedViewAsync().GetAwaiter().GetResult();
            Require(
                JsonSerializer.Serialize(savedSource.LastQuery, FleetJson.Options) ==
                savedViewJson &&
                savedWorkspace.Rows.Count == 1 &&
                savedWorkspace.RowsView.Groups is { Count: 1 },
                "saved view did not restore the exact canonical query and grouping");
            Require(
                savedWorkspace.SelectedDevice?.DeviceId == savedSelection.DeviceId &&
                restoredEvent?.Restoration is
                {
                    FocusedRegion: "detail",
                    InspectorTab: "status"
                } &&
                savedWorkspace.IsDetailOpen &&
                savedWorkspace.SelectedDetailTab == "status" &&
                savedWorkspace.ActiveScopeText.StartsWith(
                    "Saved view “Lab focus”",
                    StringComparison.Ordinal),
                "saved view did not restore selected-device and focus evidence");

            savedWorkspace.DeleteSavedViewAsync().GetAwaiter().GetResult();
            Require(
                savedWorkspace.SavedViews.Count == 0 &&
                savedSource.SavedViewsAsync(CancellationToken.None)
                    .GetAwaiter()
                    .GetResult().Revision == 3,
                "saved-view deletion did not preserve canonical revision state");

            var operationSource = new StaticFleetDataSource(
                projection,
                canonicalSummary: fixtureSummary);
            var operationWorkspace = new FleetWorkspaceViewModel(operationSource);
            operationWorkspace.InitializeAsync().GetAwaiter().GetResult();
            var operationFirst = operationWorkspace.Rows[0];
            var operationSecond = operationWorkspace.Rows[1];
            operationFirst.IsBatchSelected = true;
            operationSecond.IsBatchSelected = true;
            operationWorkspace.SelectDeviceAsync(operationFirst).GetAwaiter().GetResult();
            var operationStableKeys = operationWorkspace.Rows
                .Select(row => row.StableKey)
                .ToArray();
            var operationScope = operationWorkspace.ActiveScopeText;
            var operationBatch = operationWorkspace.BatchSelectionText;
            var operationSelected = operationWorkspace.SelectedDevice?.StableKey;

            operationWorkspace.PreviewKioskShowControlsAsync()
                .GetAwaiter()
                .GetResult();
            var preview = operationWorkspace.CurrentOperation ??
                          throw new InvalidOperationException(
                              "kiosk show-controls preview was not projected");
            Require(
                operationSource.LastPreviewRequest is
                {
                    ActionId: FleetOperationActions.KioskShowControls
                } previewRequest &&
                previewRequest.Targets.Count == 2 &&
                previewRequest.Targets[operationFirst.DeviceId] ==
                operationFirst.Projection.Identity.IdentityRevision &&
                previewRequest.Targets[operationSecond.DeviceId] ==
                operationSecond.Projection.Identity.IdentityRevision,
                "operation preview did not bind the exact selected identity revisions");
            Require(
                preview.Targets.Any(target => target.Preflight.Eligible) &&
                preview.Targets.Any(target => !target.Preflight.Eligible) &&
                operationWorkspace.OperationTargets.All(target =>
                    target.AccessibleName.Contains("eligibility", StringComparison.Ordinal) &&
                    target.AccessibleName.Contains("lifecycle", StringComparison.Ordinal)),
                "mixed operation eligibility was not projected with non-color names");
            Require(
                operationWorkspace.SelectedDevice?.StableKey == operationSelected &&
                operationWorkspace.BatchSelectionText == operationBatch &&
                operationWorkspace.ActiveScopeText == operationScope &&
                operationWorkspace.Rows.Select(row => row.StableKey)
                    .SequenceEqual(operationStableKeys),
                "operation preview changed fleet scope, selection, inspector, or ordering");

            operationWorkspace.ConfirmOperationAsync().GetAwaiter().GetResult();
            var executed = operationWorkspace.CurrentOperation ??
                           throw new InvalidOperationException(
                               "confirmed operation was not projected");
            Require(
                operationSource.LastExecuteRequest is { } executeRequest &&
                executeRequest.OperationId == preview.OperationId &&
                executeRequest.PreviewId == preview.Preview.PreviewId,
                "operation confirmation did not bind the previewed operation and immutable preview");
            Require(
                executed.Targets.Any(target =>
                    target.Lifecycle == "applied" &&
                    !string.IsNullOrWhiteSpace(
                        target.EffectiveReceipt?.ReceiptId)) &&
                executed.Targets.Any(target =>
                    target.Lifecycle == "rejected" &&
                    !target.Preflight.Eligible),
                "per-target terminal and excluded results were not retained");
            Require(
                operationWorkspace.SelectedDevice?.StableKey == operationSelected &&
                operationWorkspace.BatchSelectionText == operationBatch &&
                operationWorkspace.ActiveScopeText == operationScope &&
                operationWorkspace.Rows.Select(row => row.StableKey)
                    .SequenceEqual(operationStableKeys),
                "operation confirmation changed fleet context");

            var retainedOperation = operationWorkspace.CurrentOperation;
            operationSource.DamageNextOperationResponse = true;
            operationWorkspace.RefreshOperationAsync().GetAwaiter().GetResult();
            Require(
                ReferenceEquals(retainedOperation, operationWorkspace.CurrentOperation) &&
                operationWorkspace.OperationStatusText.StartsWith(
                    "Refresh failed · prior operation projection retained",
                    StringComparison.Ordinal),
                "mismatched operation evidence did not fail closed");

            operationWorkspace.ClearBatchSelectionCommand.Execute(null);
            operationWorkspace.SelectAllVisibleCommand.Execute(null);
            var packageIdentities = operationWorkspace.Rows.ToDictionary(
                row => row.DeviceId,
                row => row.Projection.Identity.IdentityRevision,
                StringComparer.Ordinal);
            var packageBatch = operationWorkspace.BatchSelectionText;
            operationWorkspace.PackageManifestUrl =
                "https://updates.example.invalid/labs/envelope.json";
            operationWorkspace.PackageName = "org.example.kiosk";
            operationWorkspace.PackageRolloutRing = "labs";
            operationWorkspace.PreviewPackageInstallReleaseAsync()
                .GetAwaiter()
                .GetResult();
            var packagePreview = operationWorkspace.CurrentPackageOperation ??
                                 throw new InvalidOperationException(
                                     "package install/release preview was not projected");
            Require(
                operationSource.LastPackagePreviewRequest is { } packagePreviewRequest &&
                packagePreviewRequest.Release.Kind == "manifest_url" &&
                packagePreviewRequest.Release.ManifestUrl ==
                operationWorkspace.PackageManifestUrl &&
                packagePreviewRequest.ExpectedPackageName ==
                operationWorkspace.PackageName &&
                packagePreviewRequest.ExpectedRolloutRing ==
                operationWorkspace.PackageRolloutRing &&
                packagePreviewRequest.Targets.Count == 50 &&
                packageIdentities.All(target =>
                    packagePreviewRequest.Targets.TryGetValue(
                        target.Key,
                        out var identityRevision) &&
                    identityRevision == target.Value),
                "package preview did not bind the exact signed release and target identities");
            Require(
                operationWorkspace.IsPackageInputLocked &&
                operationWorkspace.PackageInputLockText ==
                "Locked to immutable preview" &&
                operationWorkspace.CanConfirmPackageInstallRelease &&
                operationWorkspace.ConfirmPackageInstallReleaseCommand.CanExecute(null) &&
                operationWorkspace.PackageOperationSummaryText.Contains(
                    operationWorkspace.PackageManifestUrl,
                    StringComparison.Ordinal) &&
                operationWorkspace.PackageOperationSummaryText.Contains(
                    $"package {operationWorkspace.PackageName}",
                    StringComparison.Ordinal) &&
                operationWorkspace.PackageOperationSummaryText.Contains(
                    $"ring {operationWorkspace.PackageRolloutRing}",
                    StringComparison.Ordinal),
                "active package preview did not visibly lock and summarize its immutable release binding");
            Require(
                packagePreview.Targets.All(target =>
                    target.Lifecycle == "proposed" &&
                    target.Stage == "preview_ready" &&
                    target.Invocation is null &&
                    target.InvocationAcknowledgement is null &&
                    target.EffectiveReceipt is null),
                "package preview claimed owner delivery evidence");

            operationWorkspace.ConfirmPackageInstallReleaseAsync()
                .GetAwaiter()
                .GetResult();
            var preparedPackage = operationWorkspace.CurrentPackageOperation ??
                                  throw new InvalidOperationException(
                                      "confirmed package operation was not projected");
            Require(
                operationSource.LastPackageExecuteRequest is { } packageExecuteRequest &&
                packageExecuteRequest.OperationId == packagePreview.OperationId &&
                packageExecuteRequest.PreviewId == packagePreview.Preview.PreviewId,
                "package confirmation did not bind the immutable preview");
            Require(
                preparedPackage.Lifecycle == "accepted" &&
                preparedPackage.MaxParallelism == 1 &&
                preparedPackage.Targets.Count == 50 &&
                preparedPackage.Targets.All(target =>
                    target.Lifecycle == "accepted" &&
                    target.Stage == "dispatch_ready" &&
                    target.Invocation is not null &&
                    target.InvocationAcknowledgement is null &&
                    target.EffectiveReceipt is null) &&
                operationWorkspace.PackageOperationTargets.All(target =>
                    target.AccessibleName.Contains(
                        "No package dispatch or installation is claimed",
                        StringComparison.Ordinal)) &&
                operationWorkspace.PackageOperationStatusText.Contains(
                    "no package was dispatched or installed",
                    StringComparison.Ordinal) &&
                operationSource.PackageExecuteCount == 1 &&
                !operationWorkspace.CanConfirmPackageInstallRelease &&
                !operationWorkspace.ConfirmPackageInstallReleaseCommand.CanExecute(null) &&
                operationWorkspace.PackageConfirmationButtonText ==
                "Preparation accepted",
                "package preparation exceeded owner authority or stranded a target");
            Require(
                operationWorkspace.SelectedDevice?.StableKey == operationSelected &&
                operationWorkspace.BatchSelectionText == packageBatch &&
                operationWorkspace.ActiveScopeText == operationScope &&
                operationWorkspace.Rows.Select(row => row.StableKey)
                    .SequenceEqual(operationStableKeys),
                "package operation changed fleet context");

            operationWorkspace.ConfirmPackageInstallReleaseAsync()
                .GetAwaiter()
                .GetResult();
            Require(
                operationSource.PackageExecuteCount == 1 &&
                operationWorkspace.PackageOperationStatusText.StartsWith(
                    "Preparation is already accepted",
                    StringComparison.Ordinal),
                "accepted package preparation could be submitted twice");

            var retainedPackageOperation = operationWorkspace.CurrentPackageOperation;
            operationSource.DamageNextPackageOperationResponse = true;
            operationWorkspace.RefreshPackageInstallReleaseAsync()
                .GetAwaiter()
                .GetResult();
            Require(
                ReferenceEquals(
                    retainedPackageOperation,
                    operationWorkspace.CurrentPackageOperation) &&
                operationWorkspace.PackageOperationStatusText.StartsWith(
                    "Refresh failed · prior package projection retained",
                    StringComparison.Ordinal),
                "damaged package operation evidence did not fail closed");

            operationWorkspace.ClearBatchSelectionCommand.Execute(null);
            operationWorkspace.SelectedWindowsHotspotAction =
                operationWorkspace.WindowsHotspotActionOptions.Single(option =>
                    option.Action == WindowsHotspotActions.Start);
            operationWorkspace.PreviewWindowsHotspotAsync()
                .GetAwaiter()
                .GetResult();
            var hotspotPreview =
                operationWorkspace.CurrentWindowsHotspotOperation ??
                throw new InvalidOperationException(
                    "Windows host Mobile Hotspot preview was not projected");
            Require(
                operationWorkspace.WindowsHotspotActionOptions.Count == 4 &&
                operationSource.LastWindowsHotspotPreviewRequest is
                {
                    ActionId: WindowsHotspotActions.ActionId,
                    Action: WindowsHotspotActions.Start
                } &&
                operationWorkspace.BatchSelectionText.StartsWith(
                    "0 selected",
                    StringComparison.Ordinal) &&
                operationWorkspace.IsWindowsHotspotInputLocked &&
                operationWorkspace.CanConfirmWindowsHotspot &&
                operationWorkspace.WindowsHotspotConfirmationText.Contains(
                    "will not adopt an external hotspot",
                    StringComparison.Ordinal) &&
                operationWorkspace.WindowsHotspotSummaryText.Contains(
                    "host-scoped singleton",
                    StringComparison.Ordinal),
                "host hotspot preview was device-scoped or did not freeze the typed action");

            operationSource.ThrowAfterWindowsHotspotSettlement = true;
            operationWorkspace.ConfirmWindowsHotspotAsync()
                .GetAwaiter()
                .GetResult();
            Require(
                ReferenceEquals(
                    hotspotPreview,
                    operationWorkspace.CurrentWindowsHotspotOperation) &&
                operationSource.LastWindowsHotspotExecuteRequest is
                { } hotspotExecute &&
                hotspotExecute.OperationId == hotspotPreview.OperationId &&
                hotspotExecute.PreviewId ==
                hotspotPreview.Preview.PreviewId &&
                operationWorkspace.WindowsHotspotStatusText.Contains(
                    "may still settle",
                    StringComparison.Ordinal) &&
                operationWorkspace.WindowsHotspotStatusText.Contains(
                    "use Refresh",
                    StringComparison.Ordinal),
                "lost host-hotspot execute response did not retain the immutable preview and recovery path");

            operationWorkspace.RefreshWindowsHotspotAsync()
                .GetAwaiter()
                .GetResult();
            var settledHotspot =
                operationWorkspace.CurrentWindowsHotspotOperation ??
                throw new InvalidOperationException(
                    "settled Windows host Mobile Hotspot operation was not projected");
            var hotspotResult =
                operationWorkspace.WindowsHotspotResult ??
                throw new InvalidOperationException(
                    "Windows host Mobile Hotspot result was not projected");
            var publicHotspotProjection =
                operationWorkspace.WindowsHotspotSummaryText + " " +
                operationWorkspace.WindowsHotspotStatusText + " " +
                hotspotResult.AccessibleName;
            Require(
                settledHotspot.Lifecycle == "applied" &&
                hotspotResult.Ownership == "None" &&
                hotspotResult.OperationalState == "On" &&
                hotspotResult.Outcome == "Verified" &&
                hotspotResult.Clients.Contains(
                    "1 of 8",
                    StringComparison.Ordinal) &&
                hotspotResult.Band == "5 GHz" &&
                hotspotResult.SourceConnectivity == "Internet access" &&
                operationWorkspace.WindowsHotspotStatusText.Contains(
                    "Durable operation refreshed",
                    StringComparison.Ordinal) &&
                operationWorkspace.WindowsHotspotStatusText.Contains(
                    "does not perform a new live read",
                    StringComparison.Ordinal) &&
                !publicHotspotProjection.Contains(
                    "hotspot-operation",
                    StringComparison.Ordinal) &&
                !publicHotspotProjection.Contains(
                    "hotspot-preview",
                    StringComparison.Ordinal) &&
                !publicHotspotProjection.Contains(
                    "hotspot-request",
                    StringComparison.Ordinal) &&
                !publicHotspotProjection.Contains(
                    "generation",
                    StringComparison.OrdinalIgnoreCase) &&
                !publicHotspotProjection.Contains(
                    "ssid",
                    StringComparison.OrdinalIgnoreCase) &&
                !publicHotspotProjection.Contains(
                    "passphrase",
                    StringComparison.OrdinalIgnoreCase),
                "host-hotspot recovery or public projection lost durable semantics or exposed private evidence");

            var retainedHotspot = operationWorkspace.CurrentWindowsHotspotOperation;
            operationSource.DamageNextWindowsHotspotResponse = true;
            operationWorkspace.RefreshWindowsHotspotAsync()
                .GetAwaiter()
                .GetResult();
            Require(
                ReferenceEquals(
                    retainedHotspot,
                    operationWorkspace.CurrentWindowsHotspotOperation) &&
                operationWorkspace.WindowsHotspotStatusText.Contains(
                    "invalid hotspot projection",
                    StringComparison.Ordinal) &&
                !operationWorkspace.WindowsHotspotStatusText.Contains(
                    "preview_id",
                    StringComparison.OrdinalIgnoreCase),
                "damaged host-hotspot evidence was not rejected with bounded error copy");

            operationWorkspace.DismissWindowsHotspot();
            Require(
                operationWorkspace.CurrentWindowsHotspotOperation is null &&
                operationWorkspace.WindowsHotspotResult is null &&
                operationSource.LastWindowsHotspotOperation?.Lifecycle ==
                "applied" &&
                operationWorkspace.WindowsHotspotStatusText.Contains(
                    "no hotspot was stopped",
                    StringComparison.Ordinal),
                "dismissing the Console projection mutated or misrepresented host state");

            operationSource.WindowsHotspotOwnership =
                WindowsHotspotActions.OwnershipExternal;
            operationWorkspace.SelectedWindowsHotspotAction =
                operationWorkspace.WindowsHotspotActionOptions.Single(option =>
                    option.Action == WindowsHotspotActions.Start);
            operationWorkspace.PreviewWindowsHotspotAsync()
                .GetAwaiter()
                .GetResult();
            Require(
                operationWorkspace.CurrentWindowsHotspotOperation is
                {
                    Lifecycle: "rejected"
                } externalHotspot &&
                externalHotspot.Preview.Preflight.ReasonCode ==
                "external_hotspot_not_owned" &&
                operationWorkspace.WindowsHotspotResult?.Ownership.Contains(
                    "observe only",
                    StringComparison.Ordinal) == true &&
                !operationWorkspace.CanConfirmWindowsHotspot,
                "external Windows hotspot was adopted or offered a mutating confirmation");

            operationWorkspace.DismissWindowsHotspot();
            operationSource.WindowsHotspotOwnership =
                WindowsHotspotActions.OwnershipFleet;
            operationWorkspace.SelectedWindowsHotspotAction =
                operationWorkspace.WindowsHotspotActionOptions.Single(option =>
                    option.Action == WindowsHotspotActions.Stop);
            Require(
                operationWorkspace.WindowsHotspotConfirmationText.Contains(
                    "exact Fleet-owned",
                    StringComparison.Ordinal) &&
                operationWorkspace.WindowsHotspotConfirmationText.Contains(
                    "may disconnect connected clients",
                    StringComparison.Ordinal),
                "host hotspot stop confirmation did not name ownership and disconnection impact");
            operationWorkspace.PreviewWindowsHotspotAsync()
                .GetAwaiter()
                .GetResult();
            Require(
                operationWorkspace.CanConfirmWindowsHotspot &&
                operationWorkspace.WindowsHotspotConfirmButtonText ==
                "Confirm stop",
                "exact Fleet-owned hotspot stop preview was not confirmable");
            Require(
                operationWorkspace.SelectedDevice?.StableKey ==
                operationSelected &&
                operationWorkspace.ActiveScopeText == operationScope &&
                operationWorkspace.Rows.Select(row => row.StableKey)
                    .SequenceEqual(operationStableKeys),
                "host-scoped hotspot workflow changed fleet scope, inspector, or ordering");

            operationWorkspace.SelectAllVisibleCommand.Execute(null);
            operationWorkspace.SelectedQuestAwakeAction =
                operationWorkspace.QuestAwakeActionOptions.Single(option =>
                    option.Action == QuestAwakeActions.StartDeviceWatchdog);
            operationWorkspace.QuestAwakeDurationMinutes = "481";
            operationWorkspace.PreviewQuestAwakeAsync()
                .GetAwaiter()
                .GetResult();
            Require(
                operationWorkspace.CurrentQuestAwakeOperation is null &&
                operationSource.LastQuestAwakePreviewRequest is null &&
                operationWorkspace.QuestAwakeStatusText.Contains(
                    "1 through 480 minutes",
                    StringComparison.Ordinal),
                "awake-control duration above Meta's eight-hour bound was accepted");
            operationWorkspace.QuestAwakeDurationMinutes = "480";
            operationWorkspace.QuestAwakeWatchdogIntervalSeconds = "5";
            operationWorkspace.PreviewQuestAwakeAsync()
                .GetAwaiter()
                .GetResult();
            var awakePreview = operationWorkspace.CurrentQuestAwakeOperation ??
                               throw new InvalidOperationException(
                                   "awake-control preview was not projected");
            Require(
                operationSource.LastQuestAwakePreviewRequest is
                {
                    Action: QuestAwakeActions.StartDeviceWatchdog,
                    DurationMs: QuestAwakeActions.MaximumDurationMs,
                    WatchdogIntervalMs:
                        QuestAwakeActions.DefaultWatchdogIntervalMs
                } awakePreviewRequest &&
                awakePreviewRequest.Targets.Count == 50 &&
                packageIdentities.All(target =>
                    awakePreviewRequest.Targets.TryGetValue(
                        target.Key,
                        out var identityRevision) &&
                    identityRevision == target.Value) &&
                operationWorkspace.IsQuestAwakeInputLocked &&
                operationWorkspace.CanConfirmQuestAwake &&
                operationWorkspace.QuestAwakeSummaryText.Contains(
                    "Quest watchdog (stops on reboot)",
                    StringComparison.Ordinal),
                "awake-control preview did not freeze the exact action, policy, and target identities");

            operationWorkspace.ConfirmQuestAwakeAsync()
                .GetAwaiter()
                .GetResult();
            var appliedAwake = operationWorkspace.CurrentQuestAwakeOperation ??
                               throw new InvalidOperationException(
                                   "confirmed awake-control operation was not projected");
            Require(
                operationSource.LastQuestAwakeExecuteRequest is
                { } awakeExecuteRequest &&
                awakeExecuteRequest.OperationId == awakePreview.OperationId &&
                awakeExecuteRequest.PreviewId ==
                awakePreview.Preview.PreviewId &&
                appliedAwake.Targets.All(target =>
                    target.Lifecycle == "applied" &&
                    target.Receipt is
                    {
                        Effective: true,
                        DeviceWatchdogEffective: true,
                        WindowsWatchdogEffective: false
                    }) &&
                operationWorkspace.QuestAwakeTargets.All(target =>
                    target.WatchdogReadback.Contains(
                        "stops on reboot",
                        StringComparison.Ordinal) &&
                    target.AccessibleName.Contains(
                        "Power",
                        StringComparison.OrdinalIgnoreCase)) &&
                !operationWorkspace.CanConfirmQuestAwake &&
                operationWorkspace.QuestAwakeConfirmationButtonText ==
                "Confirmed",
                "awake-control confirmation lost effective independent readbacks or allowed duplicate confirmation");

            var retainedAwakeOperation =
                operationWorkspace.CurrentQuestAwakeOperation;
            operationSource.DamageNextQuestAwakeResponse = true;
            operationWorkspace.RefreshQuestAwakeAsync()
                .GetAwaiter()
                .GetResult();
            Require(
                ReferenceEquals(
                    retainedAwakeOperation,
                    operationWorkspace.CurrentQuestAwakeOperation) &&
                operationWorkspace.QuestAwakeStatusText.StartsWith(
                    "Refresh failed · prior awake-control projection retained",
                    StringComparison.Ordinal),
                "damaged awake-control identity evidence did not fail closed");
            var nonStopOverrideNode = JsonNode.Parse(
                JsonSerializer.Serialize(
                    retainedAwakeOperation,
                    FleetJson.Options)) ??
                throw new InvalidOperationException(
                    "awake-control operation could not be cloned");
            nonStopOverrideNode["targets"]![0]!["invocation"]![
                "watchdog_generation"] = "unexpected-device-generation";
            nonStopOverrideNode["targets"]![0]!["receipt"]![
                "watchdog_generation"] = "unexpected-device-generation";
            var nonStopOverride = JsonSerializer.Deserialize<QuestAwakeOperation>(
                                      nonStopOverrideNode.ToJsonString(),
                                      FleetJson.Options) ??
                                  throw new InvalidOperationException(
                                      "awake-control override damage could not be projected");
            var nonStopOverrideRejected = false;
            try
            {
                QuestAwakeProjectionValidation.ValidateOperation(
                    nonStopOverride);
            }
            catch (InvalidOperationException)
            {
                nonStopOverrideRejected = true;
            }

            Require(
                nonStopOverrideRejected,
                "non-stop awake action accepted a watchdog generation outside the immutable preview");
            Require(
                operationWorkspace.QuestAwakeActionOptions.Any(option =>
                    option.Action == QuestAwakeActions.StopWatchdogs &&
                    option.Label.Contains(
                        "settings remain",
                        StringComparison.Ordinal)) &&
                operationWorkspace.QuestAwakeActionOptions.Any(option =>
                    option.Action == QuestAwakeActions.RestoreNormal) &&
                operationWorkspace.QuestAwakeActionOptions.Any(option =>
                    option.Action == QuestAwakeActions.StartWindowsWatchdog),
                "awake-control modes did not keep stop, restore, and Windows watchdog actions separate");
            var singleAwakeTarget = new SortedDictionary<string, ulong>(
                StringComparer.Ordinal)
            {
                [operationFirst.DeviceId] =
                    operationFirst.Projection.Identity.IdentityRevision
            };
            foreach (var action in new[]
                     {
                         QuestAwakeActions.StopWatchdogs,
                         QuestAwakeActions.RestoreNormal
                     })
            {
                var observedGeneration =
                    action == QuestAwakeActions.StopWatchdogs
                        ? "observed-device-watchdog-stop-0042"
                        : "observed-device-watchdog-restore-0084";
                operationSource.QuestAwakeInvocationGenerationOverride =
                    observedGeneration;
                var request = new QuestAwakePreviewRequest
                {
                    Action = action,
                    Targets = singleAwakeTarget
                };
                var previewed = operationSource.PreviewQuestAwakeAsync(
                        request,
                        CancellationToken.None)
                    .GetAwaiter()
                    .GetResult();
                var verified = operationSource.ExecuteQuestAwakeAsync(
                        new QuestAwakeExecuteRequest
                        {
                            OperationId = previewed.OperationId,
                            PreviewId = previewed.Preview.PreviewId
                        },
                        CancellationToken.None)
                    .GetAwaiter()
                    .GetResult();
                QuestAwakeProjectionValidation.ValidateOperation(verified);
                var verifiedTarget = verified.Targets.Single();
                var awakeReceipt = verifiedTarget.Receipt ??
                                   throw new InvalidOperationException(
                                       "awake-control stop/restore receipt was absent");
                Require(
                    verified.Preview.WatchdogGeneration != observedGeneration &&
                    verifiedTarget.Invocation?.WatchdogGeneration ==
                    observedGeneration &&
                    awakeReceipt.WatchdogGeneration == observedGeneration &&
                    (action == QuestAwakeActions.StopWatchdogs
                        ? awakeReceipt.SettingsLeftUnchanged &&
                          !awakeReceipt.SettingsRestored &&
                          !awakeReceipt.WindowsWatchdogEffective &&
                          !awakeReceipt.DeviceWatchdogEffective
                        : awakeReceipt.SettingsRestored &&
                          !awakeReceipt.SettingsLeftUnchanged &&
                          !awakeReceipt.WindowsWatchdogEffective &&
                          !awakeReceipt.DeviceWatchdogEffective),
                    "stop/restore did not bind the target-specific observed watchdog generation and separate receipt semantics");
                if (action == QuestAwakeActions.RestoreNormal)
                {
                    operationSource.DamageNextQuestAwakeReceiptGeneration = true;
                    var mismatched = operationSource.QuestAwakeAsync(
                            verified.OperationId,
                            CancellationToken.None)
                        .GetAwaiter()
                        .GetResult();
                    var mismatchRejected = false;
                    try
                    {
                        QuestAwakeProjectionValidation.ValidateOperation(
                            mismatched);
                    }
                    catch (InvalidOperationException)
                    {
                        mismatchRejected = true;
                    }

                    Require(
                        mismatchRejected,
                        "awake-control receipt generation was not bound to its exact invocation generation");
                }
            }

            operationWorkspace.DismissQuestWifiAdb();
            operationWorkspace.SelectedQuestWifiAdbModernAction =
                operationWorkspace.QuestWifiAdbModernActionOptions.Single(option =>
                    option.Action == QuestWifiAdbActions.DisableWirelessAdb);
            Require(
                operationWorkspace.QuestWifiAdbConfirmationText.Contains(
                    "may disconnect active tools",
                    StringComparison.Ordinal) &&
                operationWorkspace.QuestWifiAdbConfirmationText.Contains(
                    "after-boot request is a separate setting",
                    StringComparison.Ordinal) &&
                operationWorkspace.QuestWifiAdbConfirmationButtonText ==
                "Confirm disable Wireless ADB",
                "destructive Wireless ADB confirmation copy was not explicit");
            operationWorkspace.IsQuestWifiAdbClassicRoute = true;
            Require(
                operationWorkspace.SelectedQuestWifiAdbAction.Action ==
                QuestWifiAdbActions.EnableClassicTcpipFromUsb &&
                operationWorkspace.QuestWifiAdbConfirmationText.Contains(
                    "separate classic",
                    StringComparison.Ordinal) &&
                !operationWorkspace.QuestWifiAdbConfirmationText.Contains(
                    "accept",
                    StringComparison.OrdinalIgnoreCase),
                "classic USB tcpip was conflated with the modern Kiosk route");
            operationWorkspace.SelectedQuestWifiAdbModernAction =
                operationWorkspace.QuestWifiAdbModernActionOptions.Single(option =>
                    option.Action == QuestWifiAdbActions.RequestWirelessAdb);
            Require(
                operationWorkspace.IsQuestWifiAdbModernRoute &&
                operationWorkspace.QuestWifiAdbConfirmationText.Contains(
                    "cannot accept or automate",
                    StringComparison.Ordinal),
                "modern Wireless ADB did not keep protected wearer approval explicit");

            operationWorkspace.PreviewQuestWifiAdbAsync()
                .GetAwaiter()
                .GetResult();
            var connectivityPreview =
                operationWorkspace.CurrentQuestWifiAdbOperation ??
                throw new InvalidOperationException(
                    "Quest connectivity preview was not projected");
            Require(
                operationSource.LastQuestWifiAdbPreviewRequest is
                {
                    Action: QuestWifiAdbActions.RequestWirelessAdb
                } connectivityRequest &&
                connectivityRequest.Targets.Count == 50 &&
                packageIdentities.All(target =>
                    connectivityRequest.Targets.TryGetValue(
                        target.Key,
                        out var identityRevision) &&
                    identityRevision == target.Value) &&
                operationWorkspace.IsQuestWifiAdbInputLocked &&
                operationWorkspace.CanConfirmQuestWifiAdb &&
                operationWorkspace.QuestWifiAdbSummaryText.Contains(
                    "Request Wireless ADB",
                    StringComparison.Ordinal),
                "Quest connectivity preview did not freeze the exact typed action and identities");

            operationWorkspace.ConfirmQuestWifiAdbAsync()
                .GetAwaiter()
                .GetResult();
            var appliedConnectivity =
                operationWorkspace.CurrentQuestWifiAdbOperation ??
                throw new InvalidOperationException(
                    "confirmed Quest connectivity operation was not projected");
            Require(
                operationSource.LastQuestWifiAdbExecuteRequest is
                { } connectivityExecute &&
                connectivityExecute.OperationId ==
                connectivityPreview.OperationId &&
                connectivityExecute.PreviewId ==
                connectivityPreview.Preview.PreviewId &&
                appliedConnectivity.Targets.All(target =>
                    target.Receipt is
                    {
                        RequestDelivered: true,
                        KioskSettingApplied: true,
                        WearerApproval: "pending",
                        ListenerDiscovered: true,
                        EffectApplied: true
                    } &&
                    target.TermuxUsable &&
                    target.TermuxProof is
                    {
                        Available: true,
                        ShellIdentity: "uid=2000(shell)"
                    }) &&
                operationWorkspace.QuestWifiAdbTargets.All(target =>
                    target.RequestDelivery == "Delivered" &&
                    target.KioskSetting == "Applied" &&
                    target.WearerApproval.Contains(
                        "wearer must approve in headset",
                        StringComparison.Ordinal) &&
                    target.Listener.Contains(
                        "listener observed",
                        StringComparison.Ordinal) &&
                    target.Termux.Contains(
                        "enrolled signed proof",
                        StringComparison.Ordinal) &&
                    target.Termux.Contains(
                        "current until",
                        StringComparison.Ordinal)),
                "Quest connectivity independent evidence or signed Termux freshness was collapsed");

            var retainedConnectivity =
                operationWorkspace.CurrentQuestWifiAdbOperation;
            operationSource.DamageNextQuestWifiAdbResponse = true;
            operationWorkspace.RefreshQuestWifiAdbAsync()
                .GetAwaiter()
                .GetResult();
            Require(
                ReferenceEquals(
                    retainedConnectivity,
                    operationWorkspace.CurrentQuestWifiAdbOperation) &&
                operationWorkspace.QuestWifiAdbStatusText.StartsWith(
                    "Refresh failed · prior connectivity projection retained",
                    StringComparison.Ordinal),
                "damaged Quest connectivity identity evidence did not fail closed");

            var classicPreview = operationSource.PreviewQuestWifiAdbAsync(
                    new QuestWifiAdbPreviewRequest
                    {
                        Action =
                            QuestWifiAdbActions.EnableClassicTcpipFromUsb,
                        Targets = singleAwakeTarget
                    },
                    CancellationToken.None)
                .GetAwaiter()
                .GetResult();
            var classicApplied = operationSource.ExecuteQuestWifiAdbAsync(
                    new QuestWifiAdbExecuteRequest
                    {
                        OperationId = classicPreview.OperationId,
                        PreviewId = classicPreview.Preview.PreviewId
                    },
                    CancellationToken.None)
                .GetAwaiter()
                .GetResult();
            QuestWifiAdbProjectionValidation.ValidateOperation(classicApplied);
            var classicReceipt = classicApplied.Targets.Single().Receipt ??
                                 throw new InvalidOperationException(
                                     "classic connectivity receipt was absent");
            Require(
                classicReceipt.RouteMode == "classic_tcpip" &&
                classicReceipt.RequestDelivered &&
                !classicReceipt.KioskSettingApplied &&
                classicReceipt.WearerApproval == "not_applicable" &&
                classicReceipt.ListenerDiscovered &&
                classicApplied.Targets.Single().TermuxProof is null &&
                !classicApplied.Targets.Single().TermuxUsable,
                "classic USB tcpip receipt claimed modern Kiosk, wearer, or Termux facts");

            var falseTermuxNode = JsonNode.Parse(
                JsonSerializer.Serialize(classicApplied, FleetJson.Options)) ??
                throw new InvalidOperationException(
                    "classic connectivity operation could not be cloned");
            falseTermuxNode["targets"]![0]!["termux_usable"] = true;
            var falseTermux = JsonSerializer.Deserialize<QuestWifiAdbOperation>(
                                  falseTermuxNode.ToJsonString(),
                                  FleetJson.Options) ??
                              throw new InvalidOperationException(
                                  "false Termux operation could not be projected");
            var falseTermuxRejected = false;
            try
            {
                QuestWifiAdbProjectionValidation.ValidateOperation(falseTermux);
            }
            catch (InvalidOperationException)
            {
                falseTermuxRejected = true;
            }

            Require(
                falseTermuxRejected,
                "Termux usability was accepted without an admitted signed shell proof");
            var automatedApprovalNode = JsonNode.Parse(
                JsonSerializer.Serialize(
                    appliedConnectivity,
                    FleetJson.Options)) ??
                throw new InvalidOperationException(
                    "modern connectivity operation could not be cloned");
            automatedApprovalNode["targets"]![0]!["receipt"]![
                "wearer_approval"] = "accepted";
            var automatedApproval =
                JsonSerializer.Deserialize<QuestWifiAdbOperation>(
                    automatedApprovalNode.ToJsonString(),
                    FleetJson.Options) ??
                throw new InvalidOperationException(
                    "automated-approval damage could not be projected");
            var automatedApprovalRejected = false;
            try
            {
                QuestWifiAdbProjectionValidation.ValidateOperation(
                    automatedApproval);
            }
            catch (InvalidOperationException)
            {
                automatedApprovalRejected = true;
            }

            Require(
                automatedApprovalRejected,
                "provider receipt was allowed to claim protected wearer acceptance");

            var operationWindow = new MainWindow(operationWorkspace)
            {
                ShowActivated = false,
                ShowInTaskbar = false,
                WindowStyle = WindowStyle.None,
                Width = 1_500,
                Height = 900
            };
            Require(
                !operationWindow.BatchOperationsControl.IsExpanded &&
                AutomationProperties.GetName(
                    operationWindow.BatchOperationsControl) == "Operations" &&
                AutomationProperties.GetHelpText(
                    operationWindow.BatchOperationsControl).Contains(
                    "independent of selected devices",
                    StringComparison.Ordinal),
                "operations were not collapsed and neutrally labeled by default");
            operationWindow.BatchOperationsControl.IsExpanded = true;
            var operationRoot = (FrameworkElement)operationWindow.Content;
            operationRoot.Measure(new Size(1_500, 900));
            operationRoot.Arrange(new Rect(0, 0, 1_500, 900));
            operationRoot.UpdateLayout();
            Require(
                AutomationProperties.GetName(
                    operationWindow.WindowsHotspotOperationRegion) ==
                "Windows host Mobile Hotspot operation" &&
                AutomationProperties.GetHelpText(
                    operationWindow.WindowsHotspotOperationRegion).Contains(
                    "independent of selected devices",
                    StringComparison.Ordinal) &&
                operationWindow.WindowsHotspotActionControl.Items.Count == 4 &&
                AutomationProperties.GetName(
                    operationWindow.PreviewWindowsHotspotControl).Contains(
                    "Windows host",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetHelpText(
                    operationWindow.PreviewWindowsHotspotControl).Contains(
                    "without changing",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetName(
                    operationWindow.ConfirmWindowsHotspotControl).Contains(
                    "exact immutable",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetHelpText(
                    operationWindow.RefreshWindowsHotspotControl).Contains(
                    "durable operation record",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetHelpText(
                    operationWindow.DismissWindowsHotspotControl).Contains(
                    "does not stop the hotspot",
                    StringComparison.Ordinal) &&
                !operationWindow.WindowsHotspotActionControl.IsEnabled &&
                operationWindow.WindowsHotspotResultRegion.Visibility ==
                Visibility.Visible &&
                !AutomationProperties.GetName(
                    operationWindow.WindowsHotspotResultRegion).Contains(
                    "generation",
                    StringComparison.OrdinalIgnoreCase),
                "Windows host hotspot controls lost host scope, immutable-preview, recovery, or sanitized-result semantics");
            Require(
                AutomationProperties.GetName(operationWindow.KioskOperationRegion) ==
                "Kiosk show-controls operation" &&
                AutomationProperties.GetName(
                    operationWindow.PreviewKioskShowControlsControl).Contains(
                    "exact selected devices",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetName(
                    operationWindow.ConfirmKioskShowControlsControl).Contains(
                    "previewed operation",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetName(operationWindow.RefreshOperationControl) ==
                "Refresh operation results" &&
                AutomationProperties.GetName(operationWindow.DismissOperationControl) ==
                "Dismiss operation projection",
                "kiosk operation controls were not visibly and accessibly exposed");
            Require(
                AutomationProperties.GetName(operationWindow.PackageOperationRegion) ==
                "Package install and release operation" &&
                AutomationProperties.GetName(
                    operationWindow.PackageManifestUrlControl) ==
                "Signed package manifest HTTPS URL" &&
                AutomationProperties.GetName(
                    operationWindow.PreviewPackageInstallReleaseControl).Contains(
                    "exact selected devices",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetName(
                    operationWindow.ConfirmPackageInstallReleaseControl).Contains(
                    "exact package preview",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetHelpText(
                    operationWindow.ConfirmPackageInstallReleaseControl).Contains(
                    "cannot approve Android PackageInstaller",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetName(
                    operationWindow.RefreshPackageInstallReleaseControl) ==
                "Refresh package operation results" &&
                AutomationProperties.GetName(
                    operationWindow.DismissPackageInstallReleaseControl) ==
                "Close package operation view" &&
                AutomationProperties.GetHelpText(
                    operationWindow.DismissPackageInstallReleaseControl).Contains(
                    "does not cancel",
                    StringComparison.Ordinal) &&
                operationWindow.PackageManifestUrlControl.IsReadOnly &&
                operationWindow.PackageNameControl.IsReadOnly &&
                operationWindow.PackageRolloutRingControl.IsReadOnly &&
                operationWindow.ConfirmPackageInstallReleaseControl.Content?.ToString() ==
                "Preparation accepted" &&
                !operationWindow.ConfirmPackageInstallReleaseControl.IsEnabled,
                "package operation controls were not visibly and accessibly bounded");
            Require(
                AutomationProperties.GetName(
                    operationWindow.QuestAwakeOperationRegion) ==
                "Headset awake controls" &&
                AutomationProperties.GetHelpText(
                    operationWindow.QuestAwakeOperationRegion).Contains(
                    "stops on reboot",
                    StringComparison.Ordinal) &&
                operationWindow.QuestAwakeActionControl.Items.Count == 6 &&
                AutomationProperties.GetName(
                    operationWindow.QuestAwakeDurationControl) ==
                "Keep-awake duration in minutes" &&
                AutomationProperties.GetHelpText(
                    operationWindow.QuestAwakeDurationControl).Contains(
                    "480",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetName(
                    operationWindow.PreviewQuestAwakeControl).Contains(
                    "exact selected devices",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetName(
                    operationWindow.ConfirmQuestAwakeControl).Contains(
                    "immutable",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetName(
                    operationWindow.RefreshQuestAwakeControl) ==
                "Refresh headset awake-control results" &&
                AutomationProperties.GetHelpText(
                    operationWindow.DismissQuestAwakeControl).Contains(
                    "does not stop a watchdog",
                    StringComparison.Ordinal) &&
                operationWindow.QuestAwakeDurationControl.IsReadOnly &&
                operationWindow.QuestAwakeIntervalControl.IsReadOnly &&
                !operationWindow.QuestAwakeActionControl.IsEnabled &&
                !operationWindow.ConfirmQuestAwakeControl.IsEnabled,
                "headset awake controls did not expose immutable-preview and safety boundaries");
            var awakeGrid = operationWindow.QuestAwakeTargetsControl;
            awakeGrid.BringIntoView();
            operationRoot.UpdateLayout();
            Require(
                awakeGrid.Columns.Count == 5 &&
                awakeGrid.IsReadOnly &&
                awakeGrid.EnableRowVirtualization &&
                awakeGrid.EnableColumnVirtualization &&
                VirtualizingPanel.GetIsVirtualizing(awakeGrid) &&
                VirtualizingPanel.GetVirtualizationMode(awakeGrid) ==
                VirtualizationMode.Recycling &&
                AutomationProperties.GetName(awakeGrid) ==
                "Headset awake-control target results" &&
                AutomationProperties.GetHelpText(awakeGrid).Contains(
                    "No headset serials or private paths",
                    StringComparison.Ordinal),
                "awake-control target ledger lost bounded native DataGrid or privacy semantics");
            Require(
                AutomationProperties.GetName(
                    operationWindow.QuestWifiAdbOperationRegion) ==
                "Quest Wireless ADB connectivity" &&
                AutomationProperties.GetHelpText(
                    operationWindow.QuestWifiAdbOperationRegion).Contains(
                    "does not prove wearer approval",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetHelpText(
                    operationWindow.QuestWifiAdbOperationRegion).Contains(
                    "Only a current admitted signed shell proof",
                    StringComparison.Ordinal) &&
                operationWindow.QuestWifiAdbModernActionControl.Items.Count == 5 &&
                AutomationProperties.GetName(
                    operationWindow.QuestWifiAdbModernRouteControl).Contains(
                    "modern Wireless ADB",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetName(
                    operationWindow.QuestWifiAdbClassicRouteControl).Contains(
                    "separate classic USB tcpip",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetHelpText(
                    operationWindow.QuestWifiAdbClassicRouteControl).Contains(
                    "not modern TLS",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetName(
                    operationWindow.PreviewQuestWifiAdbControl).Contains(
                    "exact selected devices",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetHelpText(
                    operationWindow.ConfirmQuestWifiAdbControl).Contains(
                    "cannot accept or automate",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetHelpText(
                    operationWindow.DismissQuestWifiAdbControl).Contains(
                    "does not disable Wireless ADB",
                    StringComparison.Ordinal) &&
                !operationWindow.QuestWifiAdbModernActionControl.IsEnabled &&
                !operationWindow.QuestWifiAdbModernRouteControl.IsEnabled &&
                !operationWindow.QuestWifiAdbClassicRouteControl.IsEnabled &&
                !operationWindow.ConfirmQuestWifiAdbControl.IsEnabled,
                "Quest connectivity controls did not expose route, approval, and immutable-preview boundaries");
            var connectivityGrid =
                operationWindow.QuestWifiAdbTargetsControl;
            connectivityGrid.BringIntoView();
            operationRoot.UpdateLayout();
            Require(
                connectivityGrid.Columns.Count == 8 &&
                connectivityGrid.IsReadOnly &&
                connectivityGrid.EnableRowVirtualization &&
                connectivityGrid.EnableColumnVirtualization &&
                VirtualizingPanel.GetIsVirtualizing(connectivityGrid) &&
                VirtualizingPanel.GetVirtualizationMode(connectivityGrid) ==
                VirtualizationMode.Recycling &&
                AutomationProperties.GetName(connectivityGrid) ==
                "Quest connectivity target results" &&
                AutomationProperties.GetHelpText(connectivityGrid).Contains(
                    "independent request-delivery",
                    StringComparison.Ordinal) &&
                AutomationProperties.GetHelpText(connectivityGrid).Contains(
                    "No serials, credentials, pairing codes, or private paths",
                    StringComparison.Ordinal),
                "Quest connectivity target ledger lost independent evidence, privacy, or native virtualization semantics");
            var packageGrid = operationWindow.PackageOperationTargetsControl;
            packageGrid.BringIntoView();
            operationRoot.UpdateLayout();
            Require(
                packageGrid.Columns.Count == 4 &&
                packageGrid.IsReadOnly &&
                packageGrid.EnableRowVirtualization &&
                packageGrid.EnableColumnVirtualization &&
                VirtualizingPanel.GetIsVirtualizing(packageGrid) &&
                VirtualizingPanel.GetVirtualizationMode(packageGrid) ==
                VirtualizationMode.Recycling &&
                AutomationProperties.GetName(packageGrid) ==
                "Package operation targets",
                "package target ledger lost native bounded DataGrid semantics");
            var realizedPackageRows = CountVisualDescendants<DataGridRow>(packageGrid);
            var firstPackageRow = FindVisualDescendant<DataGridRow>(packageGrid) ??
                                  throw new InvalidOperationException(
                                      "package target ledger did not realize a row");
            var firstPackageTarget =
                firstPackageRow.Item as PackageOperationTargetViewModel ??
                throw new InvalidOperationException(
                    "package target row projected the wrong item type");
            var initialPackageRows = FindVisualDescendants<DataGridRow>(
                packageGrid);
            var initialPackageTargets = initialPackageRows.ToDictionary(
                row => row,
                row => row.Item as PackageOperationTargetViewModel ??
                       throw new InvalidOperationException(
                           "package target row projected the wrong item type"));
            var firstPackagePeer = new DataGridRowAutomationPeer(firstPackageRow);
            Require(
                realizedPackageRows is > 0 and < 50 &&
                AutomationProperties.GetName(firstPackageRow) ==
                firstPackageTarget.AccessibleName &&
                firstPackagePeer.GetName() == firstPackageTarget.AccessibleName,
                "package target rows were not virtualized with device-specific UI Automation names");
            Require(
                firstPackageRow.Focusable,
                "package target rows were not keyboard-focusable");
            Require(
                firstPackagePeer.GetAutomationControlType() ==
                AutomationControlType.DataItem,
                $"package target row exposed {firstPackagePeer.GetAutomationControlType()} instead of a native data-item peer");
            Require(
                firstPackageTarget.AccessibleName.Contains(
                    "No package dispatch or installation is claimed.",
                    StringComparison.Ordinal),
                "package target row omitted the no-dispatch/no-install boundary");
            var recycledNameVerified = false;
            foreach (var targetIndex in new[] { 10, 20, 30, 40, 49 })
            {
                var target = operationWorkspace.PackageOperationTargets[targetIndex];
                packageGrid.ScrollIntoView(target);
                packageGrid.UpdateLayout();
                foreach (var row in FindVisualDescendants<DataGridRow>(packageGrid))
                {
                    var currentTarget =
                        row.Item as PackageOperationTargetViewModel ??
                        throw new InvalidOperationException(
                            "package target row projected the wrong item type");
                    if (initialPackageTargets.TryGetValue(
                            row,
                            out var priorTarget) &&
                        !ReferenceEquals(priorTarget, currentTarget))
                    {
                        Require(
                            AutomationProperties.GetName(row) ==
                            currentTarget.AccessibleName &&
                            new DataGridRowAutomationPeer(row).GetName() ==
                            currentTarget.AccessibleName,
                            "a recycled package target row retained stale UI Automation evidence");
                        recycledNameVerified = true;
                    }

                    initialPackageTargets[row] = currentTarget;
                }
            }

            var lastPackageTarget = operationWorkspace.PackageOperationTargets[^1];
            packageGrid.ScrollIntoView(lastPackageTarget);
            packageGrid.UpdateLayout();
            var lastPackageRow =
                packageGrid.ItemContainerGenerator.ContainerFromItem(
                    lastPackageTarget) as DataGridRow ??
                throw new InvalidOperationException(
                    "package target ledger did not realize the requested later row");
            var lastPackagePeer = new DataGridRowAutomationPeer(lastPackageRow);
            Require(
                AutomationProperties.GetName(lastPackageRow) ==
                lastPackageTarget.AccessibleName &&
                lastPackagePeer.GetName() == lastPackageTarget.AccessibleName &&
                recycledNameVerified,
                "recycled package target rows retained stale UI Automation evidence");
            operationWindow.Width = 1_000;
            operationWindow.Height = 640;
            operationRoot.Measure(new Size(1_000, 640));
            operationRoot.Arrange(new Rect(0, 0, 1_000, 640));
            ScrollElementIntoView(
                operationWindow.ConfirmPackageInstallReleaseControl,
                operationWindow.BatchOperationsScrollControl,
                operationRoot);
            var confirmationVisibleAtMinimum = IsFullyVisibleWithin(
                operationWindow.ConfirmPackageInstallReleaseControl,
                operationWindow.BatchOperationsScrollControl);
            var confirmationOriginAtMinimum =
                operationWindow.ConfirmPackageInstallReleaseControl
                    .TransformToAncestor(
                        operationWindow.BatchOperationsScrollControl)
                    .Transform(new Point(0, 0));
            ScrollElementIntoView(
                packageGrid,
                operationWindow.BatchOperationsScrollControl,
                operationRoot);
            var ledgerVisibleHeightAtMinimum = VisibleHeightWithin(
                packageGrid,
                operationWindow.BatchOperationsScrollControl);
            Require(
                operationWindow.BatchOperationsControl.ActualHeight <= 180 &&
                operationWindow.BatchOperationsScrollControl.ScrollableHeight > 0,
                $"expanded batch region was not bounded and scrollable: height {operationWindow.BatchOperationsControl.ActualHeight}, scrollable {operationWindow.BatchOperationsScrollControl.ScrollableHeight}");
            Require(
                operationWindow.FleetDataGrid.ActualHeight >= 80,
                $"expanded batch region reduced the minimum-window fleet grid to {operationWindow.FleetDataGrid.ActualHeight}px");
            Require(
                confirmationVisibleAtMinimum,
                $"the package confirmation control could not be brought into the minimum-window batch viewport: origin {confirmationOriginAtMinimum}, size {operationWindow.ConfirmPackageInstallReleaseControl.ActualWidth}x{operationWindow.ConfirmPackageInstallReleaseControl.ActualHeight}, visibility {operationWindow.ConfirmPackageInstallReleaseControl.Visibility}, viewport {operationWindow.BatchOperationsScrollControl.ActualWidth}x{operationWindow.BatchOperationsScrollControl.ActualHeight}, offset {operationWindow.BatchOperationsScrollControl.VerticalOffset}");
            Require(
                ledgerVisibleHeightAtMinimum >= 44,
                $"the package target ledger exposed only {ledgerVisibleHeightAtMinimum}px in the minimum-window batch viewport");
            operationWindow.Close();

            var liveSource = new StaticFleetDataSource(
                projection,
                canonicalSummary: fixtureSummary);
            var liveWorkspace = new FleetWorkspaceViewModel(liveSource);
            liveWorkspace.InitializeAsync().GetAwaiter().GetResult();
            Require(
                liveWorkspace.WatchInitialized &&
                liveWorkspace.WatchSequence == 0 &&
                liveSource.LastWatchAfterSequence == 0 &&
                liveSource.LastWatchLimit == FleetWorkspaceViewModel.WatchEventLimit,
                "WPF watch cursor was not established from the canonical Hub projection");
            liveWorkspace.SelectedGrouping = "Cohort";
            liveWorkspace.ApplyScopeAsync().GetAwaiter().GetResult();
            var liveFirst = liveWorkspace.Rows[0];
            var liveSecond = liveWorkspace.Rows[1];
            var liveSecondOriginalPower = liveSecond.PowerText;
            var liveStableKeys = liveWorkspace.Rows
                .Select(row => row.StableKey)
                .ToArray();
            liveFirst.IsBatchSelected = true;
            liveWorkspace.SelectDeviceAsync(liveFirst).GetAwaiter().GetResult();

            var changedSecond = RewriteOperatorRow(
                liveSecond.Projection,
                batteryPercent: 7,
                cohort: "lab-z");
            var changedRows = liveSource.Projection.Rows
                .Skip(1)
                .Reverse()
                .Select(row => row.Identity.DeviceId == changedSecond.Identity.DeviceId
                    ? changedSecond
                    : row)
                .ToArray();
            var changedProjection = RewriteProjection(
                liveSource.Projection,
                changedRows,
                liveSource.Projection.ResultRevision + 1);
            liveSource.Projection = changedProjection;
            liveSource.WatchEvents =
            [
                CreateWatchEvent(
                    1,
                    "accepted",
                    changedProjection.ResultRevision,
                    changedSecond.Identity.DeviceId,
                    sourceRevision: 2)
            ];
            liveWorkspace.SynchronizeUpdatesAsync().GetAwaiter().GetResult();

            Require(
                liveWorkspace.HasQueuedOrderingChanges &&
                liveWorkspace.OrderingChangesText.Contains(
                    "affect the current order or grouping",
                    StringComparison.Ordinal),
                "background order and grouping changes were not queued");
            Require(
                liveWorkspace.WatchSequence == 1 &&
                liveSource.LastWatchAfterSequence == 0 &&
                liveWorkspace.StatusMessage.Contains(
                    "1 accepted / 0 rejected Hub events consumed",
                    StringComparison.Ordinal),
                "canonical watch event did not trigger a cursor-bound scope synchronization");
            Require(
                liveWorkspace.Rows.Select(row => row.StableKey)
                    .SequenceEqual(liveStableKeys),
                "background refresh moved interaction-bound rows");
            Require(
                ReferenceEquals(liveFirst, liveWorkspace.Rows[0]) &&
                liveFirst.IsBatchSelected &&
                liveWorkspace.SelectedDevice?.StableKey == liveFirst.StableKey,
                "queued ordering lost row identity, selection, or inspection context");
            Require(
                liveSecond.PowerText.StartsWith("7%", StringComparison.Ordinal),
                "background refresh did not update safe shared-row values in place");
            Require(
                liveWorkspace.RowsView.Groups is { Count: 2 },
                "queued group change moved a row before operator application");
            Require(
                liveWorkspace.ApplyQueuedOrderingChangesCommand.CanExecute(null),
                "queued ordering action was not enabled");
            var queuedWindow = new MainWindow(liveWorkspace)
            {
                ShowActivated = false,
                ShowInTaskbar = false,
                WindowStyle = WindowStyle.None,
                Width = 1_500,
                Height = 900
            };
            var queuedRoot = (FrameworkElement)queuedWindow.Content;
            queuedRoot.Measure(new Size(1_500, 900));
            queuedRoot.Arrange(new Rect(0, 0, 1_500, 900));
            queuedRoot.UpdateLayout();
            Require(
                queuedWindow.ApplyOrderingButton.IsEnabled &&
                AutomationProperties.GetName(
                    queuedWindow.ApplyOrderingButton).Contains(
                    "affect the current order or grouping",
                    StringComparison.Ordinal),
                "queued ordering action was not visibly and accessibly exposed");
            Require(
                AutomationProperties.GetName(queuedWindow.SortFieldControl) ==
                "Sort devices by" &&
                AutomationProperties.GetName(queuedWindow.SortDirectionControl) ==
                "Sort direction" &&
                queuedWindow.SortFieldControl.Items.Count == 5 &&
                queuedWindow.SortDirectionControl.Items.Count == 2,
                "sort controls were not visibly and accessibly exposed");
            queuedWindow.Close();

            liveSource.Projection = RewriteProjection(
                projection,
                projection.Rows,
                changedProjection.ResultRevision + 1);
            liveWorkspace.RefreshAsync().GetAwaiter().GetResult();
            Require(
                !liveWorkspace.HasQueuedOrderingChanges &&
                liveWorkspace.Rows.Select(row => row.StableKey)
                    .SequenceEqual(liveStableKeys) &&
                liveSecond.PowerText == liveSecondOriginalPower &&
                liveWorkspace.RowsView.Groups is { Count: 2 },
                "a superseding current snapshot did not clear obsolete queued changes");

            liveSource.Projection = RewriteProjection(
                changedProjection,
                changedRows,
                changedProjection.ResultRevision + 2);
            liveWorkspace.RefreshAsync().GetAwaiter().GetResult();
            Require(
                liveWorkspace.HasQueuedOrderingChanges,
                "latest changed snapshot was not queued after supersession");
            liveWorkspace.ApplyQueuedOrderingChangesCommand.Execute(null);
            var firstCanonicallySortedChangedRow = changedRows
                .OrderBy(row => row.Identity.DisplayName, StringComparer.Ordinal)
                .First();
            Require(
                !liveWorkspace.HasQueuedOrderingChanges &&
                liveWorkspace.Rows.Count == changedRows.Length &&
                liveWorkspace.Rows[0].StableKey ==
                $"{firstCanonicallySortedChangedRow.Identity.DeviceId}@" +
                $"{firstCanonicallySortedChangedRow.Identity.IdentityRevision}",
                "explicit live-order application did not adopt the queued snapshot");
            Require(
                liveWorkspace.RowsView.Groups is { Count: 3 },
                "explicit live-order application did not adopt the queued grouping change");
            Require(
                liveWorkspace.BatchSelectionText.Contains(
                    "1 hidden by scope",
                    StringComparison.Ordinal) &&
                liveWorkspace.InspectorContextText.Contains(
                    "outside the active scope",
                    StringComparison.Ordinal),
                "explicit membership application lost hidden selection or cached inspection");

            liveSource.WatchEvents =
            [
                liveSource.WatchEvents[0],
                CreateWatchEvent(
                    2,
                    "rejected",
                    liveSource.Projection.ResultRevision,
                    liveSecond.DeviceId,
                    sourceRevision: 2,
                    reason: "stale_revision")
            ];
            var queryCountBeforeRejectedEvent = liveSource.QueryCount;
            liveWorkspace.SynchronizeUpdatesAsync().GetAwaiter().GetResult();
            Require(
                liveWorkspace.WatchSequence == 2 &&
                liveSource.QueryCount == queryCountBeforeRejectedEvent + 1 &&
                liveWorkspace.StatusMessage.Contains(
                    "0 accepted / 1 rejected Hub events consumed",
                    StringComparison.Ordinal),
                "rejected watch evidence was not distinguished from accepted device state");

            liveSource.WatchEvents =
            [
                CreateWatchEvent(
                    1,
                    "accepted",
                    liveSource.Projection.ResultRevision,
                    liveFirst.DeviceId,
                    sourceRevision: 1)
            ];
            liveWorkspace.SynchronizeUpdatesAsync().GetAwaiter().GetResult();
            Require(
                liveWorkspace.WatchSequence == 1 &&
                liveWorkspace.StatusMessage.Contains(
                    "Hub watch sequence reset",
                    StringComparison.Ordinal),
                "Hub watch sequence reset did not rebase the cursor and canonical scope");

            liveSource.WatchEvents =
            [
                CreateWatchEvent(
                    3,
                    "accepted",
                    liveSource.Projection.ResultRevision,
                    liveFirst.DeviceId,
                    sourceRevision: 3),
                CreateWatchEvent(
                    2,
                    "accepted",
                    liveSource.Projection.ResultRevision,
                    liveSecond.DeviceId,
                    sourceRevision: 2)
            ];
            var queryCountBeforeDamagedWatch = liveSource.QueryCount;
            liveWorkspace.SynchronizeUpdatesAsync().GetAwaiter().GetResult();
            Require(
                liveWorkspace.WatchSequence == 1 &&
                liveSource.QueryCount == queryCountBeforeDamagedWatch &&
                liveWorkspace.StatusMessage.StartsWith(
                    "Update check failed",
                    StringComparison.Ordinal),
                "out-of-order watch evidence did not fail closed with cached rows retained");

            var degradedWatchSource = new StaticFleetDataSource(
                projection,
                canonicalSummary: fixtureSummary,
                watchUnavailable: true);
            var degradedWatchWorkspace = new FleetWorkspaceViewModel(degradedWatchSource);
            degradedWatchWorkspace.InitializeAsync().GetAwaiter().GetResult();
            Require(
                !degradedWatchWorkspace.WatchInitialized &&
                degradedWatchWorkspace.Rows.Count == projection.Rows.Count &&
                degradedWatchWorkspace.StatusMessage.Contains(
                    "Hub watch unavailable",
                    StringComparison.Ordinal),
                "watch loss removed the canonical no-watch monitoring surface");
            var queryCountBeforeWatchFallback = degradedWatchSource.QueryCount;
            degradedWatchWorkspace.SynchronizeUpdatesAsync().GetAwaiter().GetResult();
            Require(
                degradedWatchSource.QueryCount == queryCountBeforeWatchFallback + 1 &&
                degradedWatchWorkspace.Rows.Count == projection.Rows.Count &&
                degradedWatchWorkspace.StatusMessage.Contains(
                    "canonical scope refreshed without watch",
                    StringComparison.Ordinal),
                "watch loss did not degrade to a canonical manual refresh");

            var mismatchedQueryWorkspace = new FleetWorkspaceViewModel(
                new StaticFleetDataSource(projection, echoQuery: false));
            mismatchedQueryWorkspace.InitializeAsync().GetAwaiter().GetResult();
            Require(
                mismatchedQueryWorkspace.Rows.Count == 0 &&
                mismatchedQueryWorkspace.StatusMessage.StartsWith(
                    "Refresh failed",
                    StringComparison.Ordinal),
                "mismatched query evidence did not fail closed");

            var mismatchedInspectorWorkspace = new FleetWorkspaceViewModel(
                new StaticFleetDataSource(projection, wrongInspectorIdentity: true));
            mismatchedInspectorWorkspace.InitializeAsync().GetAwaiter().GetResult();
            var mismatchedInspectorRow = mismatchedInspectorWorkspace.Rows[0];
            mismatchedInspectorWorkspace
                .SelectDeviceAsync(mismatchedInspectorRow)
                .GetAwaiter()
                .GetResult();
            Require(
                mismatchedInspectorWorkspace.Inspector?.Title ==
                mismatchedInspectorRow.DisplayName &&
                mismatchedInspectorWorkspace.StatusMessage.Contains(
                    "cached row",
                    StringComparison.Ordinal),
                "wrong-device inspector evidence replaced the cached identity");

            var mismatchedDetailWorkspace = new FleetWorkspaceViewModel(
                new StaticFleetDataSource(projection, wrongDetailIdentity: true));
            mismatchedDetailWorkspace.InitializeAsync().GetAwaiter().GetResult();
            var mismatchedDetailRow = mismatchedDetailWorkspace.Rows[0];
            mismatchedDetailWorkspace
                .SelectDeviceAsync(mismatchedDetailRow)
                .GetAwaiter()
                .GetResult();
            Require(
                !mismatchedDetailWorkspace
                    .OpenFullDetailAsync()
                    .GetAwaiter()
                    .GetResult() &&
                !mismatchedDetailWorkspace.IsDetailOpen &&
                mismatchedDetailWorkspace.SelectedDevice?.StableKey ==
                mismatchedDetailRow.StableKey &&
                mismatchedDetailWorkspace.StatusMessage.Contains(
                    "fleet context retained",
                    StringComparison.Ordinal),
                "wrong-device detail evidence did not fail closed");

            var presentWindow = arguments.Contains("--present", StringComparer.Ordinal);
            var windowWatch = Stopwatch.StartNew();
            var window = new MainWindow(workspace)
            {
                ShowActivated = presentWindow,
                ShowInTaskbar = presentWindow,
                WindowStyle = presentWindow
                    ? WindowStyle.SingleBorderWindow
                    : WindowStyle.None,
                Width = 1_500,
                Height = 900
            };
            var rootVisual = (FrameworkElement)window.Content;
            if (presentWindow)
            {
                window.Show();
                window.Activate();
            }
            else
            {
                rootVisual.Measure(new Size(1_500, 900));
                rootVisual.Arrange(new Rect(0, 0, 1_500, 900));
            }
            rootVisual.UpdateLayout();
            window.Dispatcher.Invoke(() => { }, DispatcherPriority.ApplicationIdle);
            windowWatch.Stop();
            var renderPath = ReadOptionalValue(arguments, "--render");
            var detailRenderPath = ReadOptionalValue(arguments, "--render-detail");
            if (renderPath is not null)
            {
                RenderVisual(rootVisual, renderPath);
            }

            var grid = window.FleetDataGrid;
            Require(grid.Columns.Count == 12, "fleet grid column contract drifted");
            Require(VirtualizingPanel.GetIsVirtualizing(grid), "row virtualization is disabled");
            Require(
                VirtualizingPanel.GetVirtualizationMode(grid) == VirtualizationMode.Recycling,
                "row recycling is disabled");
            Require(
                VirtualizingPanel.GetIsVirtualizingWhenGrouping(grid),
                "grouped rows do not retain virtualization");
            Require(grid.EnableRowVirtualization, "DataGrid row virtualization is disabled");
            Require(grid.EnableColumnVirtualization, "DataGrid column virtualization is disabled");
            Require(
                AutomationProperties.GetName(grid) == "Fleet devices",
                "fleet grid has no stable accessible name");
            Require(
                AutomationProperties.GetName(window.InspectorRegion) == "Selected device inspector",
                "inspector has no stable accessible name");
            Require(window.InspectorRegion.Focusable, "inspector cannot receive keyboard focus");
            Require(
                AutomationProperties.GetName(window.SavedViewControl) ==
                "Saved fleet view" &&
                AutomationProperties.GetName(window.SavedViewNameControl) ==
                "Saved-view name",
                "saved-view controls were not visibly and accessibly exposed");
            Require(
                window.SynchronizeUpdatesButton.Content?.ToString() == "Check updates" &&
                AutomationProperties.GetName(window.SynchronizeUpdatesButton) ==
                "Check for Fleet Hub updates" &&
                AutomationProperties.GetHelpText(window.SynchronizeUpdatesButton).Contains(
                    "bounded monotonic Hub events",
                    StringComparison.Ordinal),
                "watch synchronization was not visibly and accessibly exposed");
            var inspectorPeer = new ScrollViewerAutomationPeer(
                (ScrollViewer)window.InspectorRegion);
            Require(
                inspectorPeer.GetName() == "Selected device inspector",
                "inspector automation peer lost its accessible name");
            var batchCheckBox = FindVisualDescendant<CheckBox>(grid) ??
                throw new InvalidOperationException(
                    "visible batch checkbox was not realized; " +
                    $"rows={workspace.Rows.Count}; items={grid.Items.Count}; " +
                    $"grid={grid.ActualWidth:N0}x{grid.ActualHeight:N0}; " +
                    $"columns={string.Join(",", grid.Columns.Select(column =>
                        $"{column.SortMemberPath}:{column.Visibility}:{column.ActualWidth:N0}"))}");
            Require(
                batchCheckBox is { IsEnabled: true, IsHitTestVisible: true },
                "visible batch checkbox cannot be operated with a pointer");
            var batchPeer = new CheckBoxAutomationPeer(batchCheckBox);
            Require(
                batchPeer.GetName() == first.BatchSelectionName,
                "batch checkbox lost its device-specific accessible name");
            var toggleProvider =
                batchPeer.GetPattern(PatternInterface.Toggle) as IToggleProvider ??
                throw new InvalidOperationException(
                    "batch checkbox has no UI Automation toggle pattern");
            toggleProvider.Toggle();
            window.Dispatcher.Invoke(() => { }, DispatcherPriority.DataBind);
            Require(
                batchCheckBox.IsChecked == false && !first.IsBatchSelected,
                $"native UI Automation did not toggle batch membership: " +
                $"checkbox={batchCheckBox.IsChecked}, model={first.IsBatchSelected}");

            var realized = CountVisualDescendants<DataGridRow>(grid);
            Require(realized is > 0 and < 50, "virtualized grid realized an invalid row set");
            var columnWidths = grid.Columns
                .Select(column => Math.Round(column.ActualWidth, 1))
                .ToArray();
            Require(
                columnWidths.Take(11).All(width => width >= 70),
                $"fleet grid compressed a default status column below its readable minimum: {string.Join(", ", columnWidths)}");

            var peer = new DataGridAutomationPeer(grid);
            Require(
                peer.GetAutomationControlType() == AutomationControlType.DataGrid,
                "native DataGrid automation peer was not preserved");
            Require(peer.GetName() == "Fleet devices", "automation peer lost grid name");

            var detailContextKey = workspace.SelectedDevice?.StableKey;
            var detailBatchContext = workspace.BatchSelectionText;
            Require(
                workspace.OpenFullDetailAsync("history").GetAwaiter().GetResult(),
                "full-detail window projection failed");
            window.Dispatcher.Invoke(() => { }, DispatcherPriority.DataBind);
            rootVisual.UpdateLayout();
            if (detailRenderPath is not null)
            {
                RenderVisual(rootVisual, detailRenderPath);
            }
            Require(
                window.FullDeviceDetailRegion.Visibility == Visibility.Visible &&
                AutomationProperties.GetName(window.FullDeviceDetailRegion) ==
                "Full device detail" &&
                window.FullDeviceDetailTabs.SelectedValue?.ToString() == "history",
                "full-detail region or saved tab lacked accessible state");
            Require(
                AutomationProperties.GetName(window.ReturnToFleetControl) ==
                "Return to fleet view" &&
                window.ReturnToFleetControl.IsEnabled,
                "full-detail return control is inaccessible");
            var returnPeer = new ButtonAutomationPeer(window.ReturnToFleetControl);
            var returnProvider =
                returnPeer.GetPattern(PatternInterface.Invoke) as IInvokeProvider ??
                throw new InvalidOperationException(
                    "full-detail return control has no UI Automation invoke pattern");
            returnProvider.Invoke();
            window.Dispatcher.Invoke(() => { }, DispatcherPriority.ApplicationIdle);
            Require(
                window.FullDeviceDetailRegion.Visibility == Visibility.Collapsed &&
                workspace.SelectedDevice?.StableKey == detailContextKey &&
                workspace.BatchSelectionText == detailBatchContext,
                "full-detail return did not preserve fleet scope and selection");

            if (presentWindow && window.IsVisible)
            {
                var presentationFrame = new DispatcherFrame();
                void StopPresentation(object? sender, EventArgs eventArgs) =>
                    presentationFrame.Continue = false;

                window.Closed += StopPresentation;
                Dispatcher.PushFrame(presentationFrame);
                window.Closed -= StopPresentation;
            }
            else
            {
                window.Close();
            }

            var receipt = new
            {
                schema = "rusty.fleet.wpf_validation.v1",
                result = "pass",
                projection_rows = stressEvidence.ProjectionRows,
                legacy_generic_projection_fields =
                    "deprecated stress aliases retained for repository-gate compatibility",
                normal_projection_rows = projection.Rows.Count,
                deserialization_ms = deserializeWatch.Elapsed.TotalMilliseconds,
                view_model_ms = viewModelWatch.Elapsed.TotalMilliseconds,
                window_ms = windowWatch.Elapsed.TotalMilliseconds,
                realized_rows = stressEvidence.RealizedRows,
                normal_realized_rows = realized,
                grid_columns = grid.Columns.Count,
                column_widths = columnWidths,
                operator_offscreen_layout_row_matrix = new[] { 10, 50, 100 },
                operator_source_fixture_matrix = new[] { 50, 50, 250 },
                normal_window_rows = 50,
                presented_window_exercised = presentWindow,
                stress_projection_rows = stressEvidence.ProjectionRows,
                stress_realized_rows = stressEvidence.RealizedRows,
                native_datagrid = true,
                recycling_virtualization = true,
                native_automation_peer = true,
                inspector_automation_peer = true,
                pointer_batch_toggle = true,
                accessible_batch_toggle = true,
                loopback_hub_only = true,
                bounded_hub_response = true,
                live_hub_checked = liveHubChecked,
                canonical_watch_projection = true,
                watch_cursor_bounded = true,
                watch_sequence_reset_rebased = true,
                rejected_watch_event_distinguished = true,
                damaged_watch_fail_closed = true,
                watch_unavailable_query_fallback = true,
                watch_sync_accessible = true,
                projection_identity_fail_closed = true,
                mixed_freshness_fixture = true,
                fresh_rows = stressEvidence.Summary.Fresh,
                stale_rows = stressEvidence.Summary.Stale,
                offline_rows = stressEvidence.Summary.Offline,
                capability_downgrade_rows = stressEvidence.DowngradedRows,
                normal_fresh_rows = fixtureSummary.Fresh,
                normal_stale_rows = fixtureSummary.Stale,
                normal_offline_rows = fixtureSummary.Offline,
                normal_capability_downgrade_rows = downgradedRows.Length,
                stress_fresh_rows = stressEvidence.Summary.Fresh,
                stress_stale_rows = stressEvidence.Summary.Stale,
                stress_offline_rows = stressEvidence.Summary.Offline,
                stress_capability_downgrade_rows = stressEvidence.DowngradedRows,
                mixed_state_grammar = true,
                canonical_scope = true,
                canonical_sort = true,
                applied_sort_preserved = true,
                saved_view_crud = true,
                saved_view_exact_query_restored = true,
                saved_view_navigation_restored = true,
                saved_view_detail_tab_restored = true,
                kiosk_show_controls_preview = true,
                kiosk_show_controls_exact_targets = true,
                kiosk_show_controls_explicit_confirmation = true,
                kiosk_show_controls_per_target_results = true,
                kiosk_show_controls_accessible = true,
                kiosk_show_controls_fail_closed = true,
                kiosk_show_controls_rust_fixture_aligned = true,
                package_install_release_preview = true,
                package_install_release_exact_release_and_targets = true,
                package_install_release_explicit_confirmation = true,
                package_install_release_dispatch_ready_only = true,
                package_install_release_all_targets_prepared = true,
                package_install_release_owner_ingress_unavailable = true,
                package_install_release_single_shot = true,
                package_install_release_accessible = true,
                package_target_native_row_peer = true,
                package_target_recycled_name = true,
                package_target_rows_focusable = true,
                package_install_release_fail_closed = true,
                package_install_release_rust_fixture_aligned = true,
                windows_hotspot_host_scoped = true,
                windows_hotspot_exact_immutable_preview = true,
                windows_hotspot_explicit_confirmation = true,
                windows_hotspot_lost_response_recoverable = true,
                windows_hotspot_durable_refresh_not_live = true,
                windows_hotspot_external_observe_only = true,
                windows_hotspot_exact_fleet_stop = true,
                windows_hotspot_sanitized_projection = true,
                windows_hotspot_accessible = true,
                windows_hotspot_fail_closed = true,
                quest_awake_preview = true,
                quest_awake_exact_action_policy_and_targets = true,
                quest_awake_eight_hour_bound = true,
                quest_awake_explicit_confirmation = true,
                quest_awake_independent_readbacks = true,
                quest_awake_watchdog_modes_separate = true,
                quest_awake_stop_and_restore_separate = true,
                quest_awake_stop_restore_observed_generation = true,
                quest_awake_non_stop_preview_generation_bound = true,
                quest_awake_receipt_invocation_generation_bound = true,
                quest_awake_accessible = true,
                quest_awake_private_bindings_hidden = true,
                quest_awake_fail_closed = true,
                quest_wifi_adb_preview = true,
                quest_wifi_adb_exact_action_and_targets = true,
                quest_wifi_adb_explicit_confirmation = true,
                quest_wifi_adb_destructive_copy = true,
                quest_wifi_adb_request_delivery_independent = true,
                quest_wifi_adb_kiosk_setting_independent = true,
                quest_wifi_adb_wearer_approval_not_automated = true,
                quest_wifi_adb_modern_listener_independent = true,
                quest_wifi_adb_signed_termux_proof_current = true,
                quest_wifi_adb_termux_without_proof_rejected = true,
                quest_wifi_adb_classic_usb_route_separate = true,
                quest_wifi_adb_accessible = true,
                quest_wifi_adb_private_bindings_hidden = true,
                quest_wifi_adb_fail_closed = true,
                batch_operations_collapsed_by_default = true,
                minimum_window_layout = true,
                expanded_minimum_window_layout = true,
                empty_scope_preserved = true,
                grouped_virtualization = true,
                stable_live_ordering = true,
                explicit_order_application = true,
                safe_in_place_value_refresh = true,
                hidden_selection_preserved = true,
                inspector_outside_scope_preserved = true,
                full_detail_parity = true,
                full_detail_identity_fail_closed = true,
                full_detail_context_restored = true,
                full_detail_accessible = true,
                theme_dependency = "none",
                batch_selection_preserved = true,
                inspector_capability_families = inspector.Capabilities.Count,
                rendered_image = renderPath,
                rendered_detail_image = detailRenderPath
            };
            Console.WriteLine(JsonSerializer.Serialize(receipt, new JsonSerializerOptions
            {
                WriteIndented = true
            }));
            return 0;
        }
        catch (Exception error)
        {
            Console.Error.WriteLine(JsonSerializer.Serialize(new
            {
                schema = "rusty.fleet.wpf_validation.v1",
                result = "fail",
                error = error.Message
            }));
            return 1;
        }
    }

    private static string ReadRepoRoot(string[] arguments)
    {
        var index = Array.IndexOf(arguments, "--repo-root");
        if (index < 0 || index + 1 >= arguments.Length)
        {
            throw new ArgumentException("--repo-root <path> is required");
        }

        var root = Path.GetFullPath(arguments[index + 1]);
        if (!File.Exists(Path.Combine(root, "Cargo.toml")))
        {
            throw new DirectoryNotFoundException("Repository root does not contain Cargo.toml.");
        }

        return root;
    }

    private static string? ReadOptionalValue(string[] arguments, string name)
    {
        var value = ReadOptionalArgument(arguments, name);
        return value is null ? null : Path.GetFullPath(value);
    }

    private static string? ReadOptionalArgument(string[] arguments, string name)
    {
        var index = Array.IndexOf(arguments, name);
        if (index < 0)
        {
            return null;
        }

        if (index + 1 >= arguments.Length)
        {
            throw new ArgumentException($"{name} requires a value");
        }

        return arguments[index + 1];
    }

    private static void RenderVisual(FrameworkElement visual, string path)
    {
        var directory = Path.GetDirectoryName(path) ??
                        throw new ArgumentException("Render path has no parent directory.");
        Directory.CreateDirectory(directory);
        var bitmap = new RenderTargetBitmap(
            1_500,
            900,
            96,
            96,
            PixelFormats.Pbgra32);
        bitmap.Render(visual);
        var encoder = new PngBitmapEncoder();
        encoder.Frames.Add(BitmapFrame.Create(bitmap));
        using var stream = File.Create(path);
        encoder.Save(stream);
    }

    private static DeviceRowProjection RewriteOperatorRow(
        DeviceRowProjection row,
        int batteryPercent,
        string cohort)
    {
        var node = JsonNode.Parse(
                JsonSerializer.Serialize(row, FleetJson.Options))
            ?.AsObject() ?? throw new JsonException("Operator row clone was empty.");
        node["battery_percent"] = batteryPercent;
        var identity = node["identity"]?.AsObject() ??
                       throw new JsonException("Operator row identity was empty.");
        var tags = identity["tags"]?.AsObject() ??
                   throw new JsonException("Operator row tags were empty.");
        tags["cohort"] = cohort;
        return JsonSerializer.Deserialize<DeviceRowProjection>(
                   node.ToJsonString(),
                   FleetJson.Options)
               ?? throw new JsonException("Operator row clone could not be read.");
    }

    private static FleetQueryResult RewriteProjection(
        FleetQueryResult projection,
        IReadOnlyList<DeviceRowProjection> rows,
        ulong resultRevision) => new()
        {
            Schema = projection.Schema,
            Query = projection.Query,
            ResultRevision = resultRevision,
            AsOfMs = projection.AsOfMs + 1_000,
            TotalCount = rows.Count,
            WindowOffset = 0,
            WindowCount = rows.Count,
            Rows = rows
        };

    private static FleetWatchEvent CreateWatchEvent(
        ulong eventSequence,
        string decision,
        ulong resultRevision,
        string? deviceId,
        ulong? sourceRevision,
        string? reason = null) => new()
        {
            Schema = "rusty.fleet.watch_event.v1",
            EventSequence = eventSequence,
            ObservedAtMs = 1_800_000_000_000 + (long)eventSequence,
            Decision = new ObservationDecision
            {
                Kind = decision,
                ResultRevision = resultRevision,
                DeviceId = deviceId,
                SourceRevision = sourceRevision,
                Reason = reason,
                Details = reason is null ? [] : ["synthetic watch rejection"]
            }
        };

    private static T? FindVisualDescendant<T>(DependencyObject? parent)
        where T : DependencyObject
    {
        if (parent is null)
        {
            return null;
        }

        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(parent); index++)
        {
            var child = VisualTreeHelper.GetChild(parent, index);
            if (child is T match)
            {
                return match;
            }

            var descendant = FindVisualDescendant<T>(child);
            if (descendant is not null)
            {
                return descendant;
            }
        }

        return null;
    }

    private static void ScrollElementIntoView(
        FrameworkElement element,
        ScrollViewer scrollViewer,
        FrameworkElement layoutRoot)
    {
        var origin = element
            .TransformToAncestor(scrollViewer)
            .Transform(new Point(0, 0));
        scrollViewer.ScrollToVerticalOffset(
            Math.Max(0, scrollViewer.VerticalOffset + origin.Y - 4));
        layoutRoot.UpdateLayout();
    }

    private static bool IsFullyVisibleWithin(
        FrameworkElement element,
        FrameworkElement ancestor)
    {
        if (element.Visibility != Visibility.Visible ||
            element.ActualWidth <= 0 ||
            element.ActualHeight <= 0)
        {
            return false;
        }

        var bounds = element
            .TransformToAncestor(ancestor)
            .TransformBounds(
                new Rect(0, 0, element.ActualWidth, element.ActualHeight));
        return bounds.Left >= 0 &&
               bounds.Top >= 0 &&
               bounds.Right <= ancestor.ActualWidth + 0.5 &&
               bounds.Bottom <= ancestor.ActualHeight + 0.5;
    }

    private static double VisibleHeightWithin(
        FrameworkElement element,
        FrameworkElement ancestor)
    {
        if (element.Visibility != Visibility.Visible ||
            element.ActualWidth <= 0 ||
            element.ActualHeight <= 0)
        {
            return 0;
        }

        var bounds = element
            .TransformToAncestor(ancestor)
            .TransformBounds(
                new Rect(0, 0, element.ActualWidth, element.ActualHeight));
        var intersection = Rect.Intersect(
            bounds,
            new Rect(0, 0, ancestor.ActualWidth, ancestor.ActualHeight));
        return intersection.IsEmpty ? 0 : intersection.Height;
    }

    private static IReadOnlyList<T> FindVisualDescendants<T>(
        DependencyObject? parent)
        where T : DependencyObject
    {
        var results = new List<T>();
        CollectVisualDescendants(parent, results);
        return results;
    }

    private static void CollectVisualDescendants<T>(
        DependencyObject? parent,
        ICollection<T> results)
        where T : DependencyObject
    {
        if (parent is null)
        {
            return;
        }

        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(parent); index++)
        {
            var child = VisualTreeHelper.GetChild(parent, index);
            if (child is T match)
            {
                results.Add(match);
            }

            CollectVisualDescendants(child, results);
        }
    }

    private static int CountVisualDescendants<T>(DependencyObject? parent)
        where T : DependencyObject
    {
        if (parent is null)
        {
            return 0;
        }

        var count = 0;
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(parent); index++)
        {
            var child = VisualTreeHelper.GetChild(parent, index);
            if (child is T)
            {
                count++;
            }

            count += CountVisualDescendants<T>(child);
        }

        return count;
    }

    private static (
        int ProjectionRows,
        int RealizedRows,
        FleetSummaryProjection Summary,
        int DowngradedRows) ValidateVirtualizationStress(string repoRoot)
    {
        var json = RunFleetctl(
            repoRoot,
            "operator-fixture",
            "mixed-freshness",
            "1000");
        FleetQueryResult projection;
        FleetSummaryProjection summary;
        using (var document = JsonDocument.Parse(json))
        {
            projection = FleetJson.DeserializeQueryResult(
                document.RootElement.GetProperty("query_result").GetRawText());
            summary = JsonSerializer.Deserialize<FleetSummaryProjection>(
                document.RootElement.GetProperty("summary").GetRawText(),
                FleetJson.Options) ?? throw new JsonException(
                "Fleet stress summary was empty.");
        }

        var source = new StaticFleetDataSource(
            projection,
            canonicalSummary: summary);
        var workspace = new FleetWorkspaceViewModel(source);
        workspace.InitializeAsync().GetAwaiter().GetResult();
        var window = new MainWindow(workspace)
        {
            ShowActivated = false,
            ShowInTaskbar = false,
            WindowStyle = WindowStyle.None,
            Width = 1_500,
            Height = 900
        };
        var root = (FrameworkElement)window.Content;
        root.Measure(new Size(1_500, 900));
        root.Arrange(new Rect(0, 0, 1_500, 900));
        root.UpdateLayout();
        var realizedRows = CountVisualDescendants<DataGridRow>(
            window.FleetDataGrid);
        var downgradedRows = projection.Rows.Count(row =>
            row.Capabilities.Capabilities.TryGetValue(
                "participating_app_control",
                out var capability) &&
            capability.Authorization == "unauthorized");
        Require(
            projection.Rows.Count == 1_000 &&
            summary is
            {
                Total: 1_000,
                Fresh: 500,
                Stale: 250,
                Offline: 250
            } &&
            downgradedRows == 125 &&
            realizedRows is > 0 and < 250 &&
            VirtualizingPanel.GetIsVirtualizing(window.FleetDataGrid) &&
            VirtualizingPanel.GetVirtualizationMode(window.FleetDataGrid) ==
            VirtualizationMode.Recycling,
            "the non-presented 1,000-row stress sentinel lost bounded virtualization");
        window.Close();
        return (
            projection.Rows.Count,
            realizedRows,
            summary,
            downgradedRows);
    }

    private static void ValidateRealisticScaleWindow(string repoRoot, int deviceCount)
    {
        var sourceFixtureCount = deviceCount <= 50 ? 50 : 250;
        var json = RunFleetctl(
            repoRoot,
            "operator-fixture",
            "mixed-freshness",
            sourceFixtureCount.ToString(
                System.Globalization.CultureInfo.InvariantCulture));
        FleetQueryResult sourceProjection;
        using (var document = JsonDocument.Parse(json))
        {
            sourceProjection = FleetJson.DeserializeQueryResult(
                document.RootElement.GetProperty("query_result").GetRawText());
        }

        var projection = new FleetQueryResult
        {
            Schema = sourceProjection.Schema,
            Query = sourceProjection.Query,
            ResultRevision = sourceProjection.ResultRevision,
            AsOfMs = sourceProjection.AsOfMs,
            TotalCount = deviceCount,
            WindowOffset = 0,
            WindowCount = deviceCount,
            Rows = sourceProjection.Rows.Take(deviceCount).ToArray()
        };
        Require(
            projection.TotalCount == deviceCount &&
            projection.Rows.Count == deviceCount,
            $"{deviceCount}-device operator fixture was incomplete");
        var source = new StaticFleetDataSource(projection);
        var workspace = new FleetWorkspaceViewModel(source);
        workspace.InitializeAsync().GetAwaiter().GetResult();
        var window = new MainWindow(workspace)
        {
            ShowActivated = false,
            ShowInTaskbar = false,
            WindowStyle = WindowStyle.None,
            Width = 1_500,
            Height = 900
        };
        var root = (FrameworkElement)window.Content;
        root.Measure(new Size(1_500, 900));
        root.Arrange(new Rect(0, 0, 1_500, 900));
        root.UpdateLayout();
        var grid = window.FleetDataGrid;
        var realized = CountVisualDescendants<DataGridRow>(grid);
        Require(
            grid.Items.Count == deviceCount &&
            realized > 0 &&
            (deviceCount < 100 || realized < deviceCount) &&
            VirtualizingPanel.GetIsVirtualizing(grid) &&
            VirtualizingPanel.GetVirtualizationMode(grid) ==
            VirtualizationMode.Recycling,
            $"{deviceCount}-device WPF scale fixture lost bounded recycling virtualization");
        window.Close();

        if (deviceCount == 10)
        {
            var minimumWindow = new MainWindow(workspace)
            {
                ShowActivated = false,
                ShowInTaskbar = false,
                WindowStyle = WindowStyle.None,
                Width = 1_000,
                Height = 640
            };
            var minimumRoot = (FrameworkElement)minimumWindow.Content;
            minimumRoot.Measure(new Size(1_000, 640));
            minimumRoot.Arrange(new Rect(0, 0, 1_000, 640));
            minimumRoot.UpdateLayout();
            Require(
                !minimumWindow.BatchOperationsControl.IsExpanded &&
                minimumWindow.FleetDataGrid.ActualHeight >= 120 &&
                Grid.GetColumnSpan(minimumWindow.PackageManifestUrlControl) == 3 &&
                Grid.GetRow(minimumWindow.PackageNameControl) == 1 &&
                Grid.GetRow(minimumWindow.PackageRolloutRingControl) == 1,
                "the declared minimum window lost its fleet workspace or responsive package layout");
            minimumWindow.Close();
        }
    }

    private static string RunFleetctl(string repoRoot, params string[] arguments)
    {
        var start = new ProcessStartInfo
        {
            FileName = "cargo",
            WorkingDirectory = repoRoot,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false
        };
        start.ArgumentList.Add("run");
        start.ArgumentList.Add("--quiet");
        start.ArgumentList.Add("--locked");
        start.ArgumentList.Add("-p");
        start.ArgumentList.Add("fleetctl");
        start.ArgumentList.Add("--");
        foreach (var argument in arguments)
        {
            start.ArgumentList.Add(argument);
        }
        using var process = Process.Start(start) ??
                            throw new InvalidOperationException("Unable to start fleetctl.");
        var output = process.StandardOutput.ReadToEnd();
        var error = process.StandardError.ReadToEnd();
        process.WaitForExit();
        if (process.ExitCode != 0)
        {
            throw new InvalidOperationException($"fleetctl failed: {error}");
        }

        return output;
    }

    private static void Require(bool condition, string message)
    {
        if (!condition)
        {
            throw new InvalidOperationException(message);
        }
    }

    private sealed class StaticFleetDataSource(
        FleetQueryResult projection,
        FleetSummaryProjection? canonicalSummary = null,
        bool echoQuery = true,
        bool wrongInspectorIdentity = false,
        bool wrongDetailIdentity = false,
        IReadOnlyList<FleetWatchEvent>? watchEvents = null,
        bool watchUnavailable = false) : IFleetDataSource
    {
        private readonly SortedDictionary<string, SavedView> _savedViews =
            new(StringComparer.Ordinal);
        private ulong _savedViewRevision = 1;

        public FleetQueryResult Projection { get; set; } = projection;

        public FleetQuery? LastQuery { get; private set; }

        public int QueryCount { get; private set; }

        public IReadOnlyList<FleetWatchEvent> WatchEvents { get; set; } =
            watchEvents ?? [];

        public ulong? LastWatchAfterSequence { get; private set; }

        public int? LastWatchLimit { get; private set; }

        public OperationPreviewRequest? LastPreviewRequest { get; private set; }

        public OperationExecuteRequest? LastExecuteRequest { get; private set; }

        public OperationLedger? LastOperation { get; private set; }

        public bool DamageNextOperationResponse { get; set; }

        public PackageInstallReleasePreviewRequest? LastPackagePreviewRequest { get; private set; }

        public PackageInstallReleaseExecuteRequest? LastPackageExecuteRequest { get; private set; }

        public int PackageExecuteCount { get; private set; }

        public PackageInstallReleaseOperation? LastPackageOperation { get; private set; }

        public bool DamageNextPackageOperationResponse { get; set; }

        public WindowsHotspotPreviewRequest? LastWindowsHotspotPreviewRequest { get; private set; }

        public WindowsHotspotExecuteRequest? LastWindowsHotspotExecuteRequest { get; private set; }

        public WindowsHotspotOperation? LastWindowsHotspotOperation { get; private set; }

        public string WindowsHotspotOwnership { get; set; } =
            WindowsHotspotActions.OwnershipNone;

        public bool WindowsHotspotProviderReady { get; set; } = true;

        public bool WindowsHotspotLeaseAvailable { get; set; } = true;

        public bool DamageNextWindowsHotspotResponse { get; set; }

        public bool ThrowAfterWindowsHotspotSettlement { get; set; }

        public QuestAwakePreviewRequest? LastQuestAwakePreviewRequest { get; private set; }

        public QuestAwakeExecuteRequest? LastQuestAwakeExecuteRequest { get; private set; }

        public QuestAwakeOperation? LastQuestAwakeOperation { get; private set; }

        public bool DamageNextQuestAwakeResponse { get; set; }

        public bool DamageNextQuestAwakeReceiptGeneration { get; set; }

        public string? QuestAwakeInvocationGenerationOverride { get; set; }

        public QuestWifiAdbPreviewRequest? LastQuestWifiAdbPreviewRequest { get; private set; }

        public QuestWifiAdbExecuteRequest? LastQuestWifiAdbExecuteRequest { get; private set; }

        public QuestWifiAdbOperation? LastQuestWifiAdbOperation { get; private set; }

        public bool DamageNextQuestWifiAdbResponse { get; set; }

        public Task<FleetQueryResult> QueryAsync(
            FleetQuery query,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            LastQuery = query;
            QueryCount++;
            var matched = Projection.Rows
                .Where(row => Matches(query.Expression, row))
                .ToArray();
            Array.Sort(matched, (left, right) => CompareRows(left, right, query));
            var window = matched
                .Skip(query.Offset)
                .Take(query.Limit)
                .ToArray();
            return Task.FromResult(new FleetQueryResult
            {
                Schema = Projection.Schema,
                Query = echoQuery ? query : Projection.Query,
                ResultRevision = Projection.ResultRevision,
                AsOfMs = Projection.AsOfMs,
                TotalCount = matched.Length,
                WindowOffset = query.Offset,
                WindowCount = window.Length,
                Rows = window
            });
        }

        private static int CompareRows(
            DeviceRowProjection left,
            DeviceRowProjection right,
            FleetQuery query)
        {
            foreach (var key in query.Sort)
            {
                var comparison = key.Field switch
                {
                    "device_id" => StringComparer.Ordinal.Compare(
                        left.Identity.DeviceId,
                        right.Identity.DeviceId),
                    "display_name" => StringComparer.Ordinal.Compare(
                        left.Identity.DisplayName,
                        right.Identity.DisplayName),
                    "model" => StringComparer.Ordinal.Compare(
                        left.Identity.Model,
                        right.Identity.Model),
                    "freshness" => FreshnessRank(left.Freshness)
                        .CompareTo(FreshnessRank(right.Freshness)),
                    "battery_percent" => Nullable.Compare(
                        left.BatteryPercent,
                        right.BatteryPercent),
                    "foreground_app" => StringComparer.Ordinal.Compare(
                        left.ForegroundApp,
                        right.ForegroundApp),
                    "kiosk_state" => StringComparer.Ordinal.Compare(
                        left.KioskState,
                        right.KioskState),
                    _ => 0
                };
                if (key.Direction == "descending")
                {
                    comparison = -comparison;
                }

                if (comparison != 0)
                {
                    return comparison;
                }
            }

            var displayNameComparison = StringComparer.Ordinal.Compare(
                left.Identity.DisplayName,
                right.Identity.DisplayName);
            return displayNameComparison != 0
                ? displayNameComparison
                : StringComparer.Ordinal.Compare(
                    left.Identity.DeviceId,
                    right.Identity.DeviceId);
        }

        private static int FreshnessRank(string freshness) =>
            freshness switch
            {
                "fresh" => 0,
                "stale" => 1,
                "offline" => 2,
                _ => 3
            };

        public Task<FleetSummaryProjection> SummaryAsync(CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (canonicalSummary is not null)
            {
                return Task.FromResult(canonicalSummary);
            }

            return Task.FromResult(new FleetSummaryProjection
            {
                Schema = "rusty.fleet.summary.v1",
                AsOfMs = Projection.AsOfMs,
                Total = Projection.TotalCount,
                Fresh = Projection.Rows.Count(row => row.Freshness == "fresh"),
                Stale = Projection.Rows.Count(row => row.Freshness == "stale"),
                Offline = Projection.Rows.Count(row => row.Freshness == "offline"),
                Attention = Projection.Rows.Count(row =>
                    row.Conditions.Values.Any(condition =>
                        condition.State is "degraded" or "failed" or "critical")),
                ActiveWork = Projection.Rows.Sum(row => row.ActiveWorkCount)
            });
        }

        public Task<DeviceInspectorProjection> InspectAsync(
            string deviceId,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            var row = wrongInspectorIdentity
                ? Projection.Rows.First(item => item.Identity.DeviceId != deviceId)
                : Projection.Rows.Single(item => item.Identity.DeviceId == deviceId);
            return Task.FromResult(new DeviceInspectorProjection
            {
                Schema = "rusty.fleet.device_inspector.v1",
                Row = row,
                Attention = row.Conditions.Values
                    .Where(condition => condition.State is
                        "stale" or "unauthorized" or "restricted" or "degraded" or
                        "failed" or "critical")
                    .ToArray()
            });
        }

        public Task<DeviceDetailProjection> DetailAsync(
            string deviceId,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            var row = wrongDetailIdentity
                ? Projection.Rows.First(item => item.Identity.DeviceId != deviceId)
                : Projection.Rows.Single(item => item.Identity.DeviceId == deviceId);
            var inspector = new DeviceInspectorProjection
            {
                Schema = "rusty.fleet.device_inspector.v1",
                Row = row,
                Attention = row.Conditions.Values
                    .Where(condition => condition.State is
                        "stale" or "unauthorized" or "restricted" or "degraded" or
                        "failed" or "critical")
                    .ToArray()
            };
            return Task.FromResult(new DeviceDetailProjection
            {
                Schema = "rusty.fleet.device_detail.v1",
                Inspector = inspector,
                ConditionHistory = row.Conditions.Values.ToArray(),
                OperationHistory = []
            });
        }

        public Task<IReadOnlyList<FleetWatchEvent>> WatchAsync(
            ulong afterSequence,
            int limit,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (watchUnavailable)
            {
                throw new HttpRequestException("synthetic watch route unavailable");
            }
            LastWatchAfterSequence = afterSequence;
            LastWatchLimit = limit;
            return Task.FromResult<IReadOnlyList<FleetWatchEvent>>(
                WatchEvents
                    .Where(watchEvent => watchEvent.EventSequence > afterSequence)
                    .Take(Math.Min(limit, FleetWorkspaceViewModel.WatchEventLimit))
                    .ToArray());
        }

        public Task<SavedViewCollection> SavedViewsAsync(
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            return Task.FromResult(new SavedViewCollection
            {
                Schema = "rusty.fleet.saved_view_collection.v1",
                Revision = _savedViewRevision,
                Views = _savedViews.Values.ToArray()
            });
        }

        public Task<SavedViewMutationReceipt> UpsertSavedViewAsync(
            SavedViewMutationRequest request,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (request.ExpectedRevision != _savedViewRevision)
            {
                throw new InvalidOperationException("saved-view revision conflict");
            }

            var changed = !_savedViews.TryGetValue(request.View.ViewId, out var existing) ||
                          JsonSerializer.Serialize(existing, FleetJson.Options) !=
                          JsonSerializer.Serialize(request.View, FleetJson.Options);
            var previous = _savedViewRevision;
            if (changed)
            {
                _savedViews[request.View.ViewId] = request.View;
                _savedViewRevision++;
            }

            return Task.FromResult(new SavedViewMutationReceipt
            {
                Schema = "rusty.fleet.saved_view_mutation_receipt.v1",
                ViewId = request.View.ViewId,
                PreviousRevision = previous,
                CurrentRevision = _savedViewRevision,
                Changed = changed,
                Deleted = false,
                View = request.View
            });
        }

        public Task<SavedViewMutationReceipt> DeleteSavedViewAsync(
            string viewId,
            ulong expectedRevision,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (expectedRevision != _savedViewRevision)
            {
                throw new InvalidOperationException("saved-view revision conflict");
            }

            if (!_savedViews.Remove(viewId))
            {
                throw new InvalidOperationException("saved view not found");
            }

            var previous = _savedViewRevision;
            _savedViewRevision++;
            return Task.FromResult(new SavedViewMutationReceipt
            {
                Schema = "rusty.fleet.saved_view_mutation_receipt.v1",
                ViewId = viewId,
                PreviousRevision = previous,
                CurrentRevision = _savedViewRevision,
                Changed = true,
                Deleted = true,
                View = null
            });
        }

        public Task<OperationLedger> PreviewOperationAsync(
            OperationPreviewRequest request,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            LastPreviewRequest = request;
            LastOperation = CreateOperation(request.Targets, executed: false);
            return Task.FromResult(ReturnOperation());
        }

        public Task<OperationLedger> ExecuteOperationAsync(
            OperationExecuteRequest request,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            LastExecuteRequest = request;
            if (LastOperation is null ||
                LastOperation.OperationId != request.OperationId ||
                LastOperation.Preview.PreviewId != request.PreviewId)
            {
                throw new InvalidOperationException(
                    "execute request did not match the synthetic preview");
            }

            LastOperation = CreateOperation(
                LastOperation.Preview.Targets.ToDictionary(
                    target => target.DeviceId,
                    target => target.IdentityRevision,
                    StringComparer.Ordinal),
                executed: true);
            return Task.FromResult(ReturnOperation());
        }

        public Task<OperationLedger> OperationAsync(
            string operationId,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (LastOperation is null || LastOperation.OperationId != operationId)
            {
                throw new InvalidOperationException("synthetic operation not found");
            }

            return Task.FromResult(ReturnOperation());
        }

        public Task<PackageInstallReleaseOperation> PreviewPackageInstallReleaseAsync(
            PackageInstallReleasePreviewRequest request,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            LastPackagePreviewRequest = request;
            LastPackageOperation = CreatePackageOperation(request, prepared: false);
            return Task.FromResult(ReturnPackageOperation());
        }

        public Task<PackageInstallReleaseOperation> ExecutePackageInstallReleaseAsync(
            PackageInstallReleaseExecuteRequest request,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            PackageExecuteCount++;
            LastPackageExecuteRequest = request;
            if (LastPackageOperation is null ||
                LastPackageOperation.OperationId != request.OperationId ||
                LastPackageOperation.Preview.PreviewId != request.PreviewId ||
                LastPackagePreviewRequest is null)
            {
                throw new InvalidOperationException(
                    "package execute request did not match the synthetic preview");
            }

            LastPackageOperation = CreatePackageOperation(
                LastPackagePreviewRequest,
                prepared: true);
            return Task.FromResult(ReturnPackageOperation());
        }

        public Task<PackageInstallReleaseOperation> PackageInstallReleaseAsync(
            string operationId,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (LastPackageOperation is null ||
                LastPackageOperation.OperationId != operationId)
            {
                throw new InvalidOperationException(
                    "synthetic package operation not found");
            }

            return Task.FromResult(ReturnPackageOperation());
        }

        public Task<WindowsHotspotOperation> PreviewWindowsHotspotAsync(
            WindowsHotspotPreviewRequest request,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            LastWindowsHotspotPreviewRequest = request;
            LastWindowsHotspotOperation =
                CreateWindowsHotspotOperation(request, executed: false);
            return Task.FromResult(ReturnWindowsHotspotOperation());
        }

        public Task<WindowsHotspotOperation> ExecuteWindowsHotspotAsync(
            WindowsHotspotExecuteRequest request,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            LastWindowsHotspotExecuteRequest = request;
            if (LastWindowsHotspotOperation is null ||
                LastWindowsHotspotPreviewRequest is null ||
                LastWindowsHotspotOperation.OperationId != request.OperationId ||
                LastWindowsHotspotOperation.Preview.PreviewId != request.PreviewId)
            {
                throw new InvalidOperationException(
                    "host-hotspot execute request did not match the synthetic preview");
            }

            LastWindowsHotspotOperation = CreateWindowsHotspotOperation(
                LastWindowsHotspotPreviewRequest,
                executed: true);
            if (ThrowAfterWindowsHotspotSettlement)
            {
                ThrowAfterWindowsHotspotSettlement = false;
                throw new TaskCanceledException(
                    "synthetic response lost after hotspot settlement");
            }

            return Task.FromResult(ReturnWindowsHotspotOperation());
        }

        public Task<WindowsHotspotOperation> WindowsHotspotAsync(
            string operationId,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (LastWindowsHotspotOperation is null ||
                LastWindowsHotspotOperation.OperationId != operationId)
            {
                throw new InvalidOperationException(
                    "synthetic host-hotspot operation not found");
            }

            return Task.FromResult(ReturnWindowsHotspotOperation());
        }

        public Task<QuestAwakeOperation> PreviewQuestAwakeAsync(
            QuestAwakePreviewRequest request,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            LastQuestAwakePreviewRequest = request;
            LastQuestAwakeOperation = CreateQuestAwakeOperation(
                request,
                executed: false);
            return Task.FromResult(ReturnQuestAwakeOperation());
        }

        public Task<QuestAwakeOperation> ExecuteQuestAwakeAsync(
            QuestAwakeExecuteRequest request,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            LastQuestAwakeExecuteRequest = request;
            if (LastQuestAwakeOperation is null ||
                LastQuestAwakePreviewRequest is null ||
                LastQuestAwakeOperation.OperationId != request.OperationId ||
                LastQuestAwakeOperation.Preview.PreviewId != request.PreviewId)
            {
                throw new InvalidOperationException(
                    "awake-control execute request did not match the synthetic preview");
            }

            LastQuestAwakeOperation = CreateQuestAwakeOperation(
                LastQuestAwakePreviewRequest,
                executed: true);
            return Task.FromResult(ReturnQuestAwakeOperation());
        }

        public Task<QuestAwakeOperation> QuestAwakeAsync(
            string operationId,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (LastQuestAwakeOperation is null ||
                LastQuestAwakeOperation.OperationId != operationId)
            {
                throw new InvalidOperationException(
                    "synthetic awake-control operation not found");
            }

            return Task.FromResult(ReturnQuestAwakeOperation());
        }

        public Task<QuestWifiAdbOperation> PreviewQuestWifiAdbAsync(
            QuestWifiAdbPreviewRequest request,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            LastQuestWifiAdbPreviewRequest = request;
            LastQuestWifiAdbOperation = CreateQuestWifiAdbOperation(
                request,
                executed: false);
            return Task.FromResult(ReturnQuestWifiAdbOperation());
        }

        public Task<QuestWifiAdbOperation> ExecuteQuestWifiAdbAsync(
            QuestWifiAdbExecuteRequest request,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            LastQuestWifiAdbExecuteRequest = request;
            if (LastQuestWifiAdbOperation is null ||
                LastQuestWifiAdbPreviewRequest is null ||
                LastQuestWifiAdbOperation.OperationId != request.OperationId ||
                LastQuestWifiAdbOperation.Preview.PreviewId != request.PreviewId)
            {
                throw new InvalidOperationException(
                    "connectivity execute request did not match the synthetic preview");
            }

            LastQuestWifiAdbOperation = CreateQuestWifiAdbOperation(
                LastQuestWifiAdbPreviewRequest,
                executed: true);
            return Task.FromResult(ReturnQuestWifiAdbOperation());
        }

        public Task<QuestWifiAdbOperation> QuestWifiAdbAsync(
            string operationId,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (LastQuestWifiAdbOperation is null ||
                LastQuestWifiAdbOperation.OperationId != operationId)
            {
                throw new InvalidOperationException(
                    "synthetic connectivity operation not found");
            }

            return Task.FromResult(ReturnQuestWifiAdbOperation());
        }

        private OperationLedger ReturnOperation()
        {
            var operation = LastOperation ??
                            throw new InvalidOperationException(
                                "synthetic operation was not created");
            if (!DamageNextOperationResponse)
            {
                return operation;
            }

            DamageNextOperationResponse = false;
            var damagedTargets = operation.Targets.ToArray();
            var first = damagedTargets[0];
            damagedTargets[0] = new OperationTargetResult
            {
                DeviceId = first.DeviceId,
                IdentityRevision = first.IdentityRevision + 1,
                Preflight = first.Preflight,
                Lifecycle = first.Lifecycle,
                DispatchedAtMs = first.DispatchedAtMs,
                OwnerDeadlineAtMs = first.OwnerDeadlineAtMs,
                AttemptCount = first.AttemptCount,
                OwnerRequestIds = first.OwnerRequestIds,
                OwnerRequestId = first.OwnerRequestId,
                EffectiveReceipt = first.EffectiveReceipt,
                RetryDisposition = first.RetryDisposition,
                CancelDisposition = first.CancelDisposition,
                ReasonCode = first.ReasonCode,
                Message = first.Message,
                LastTransitionMs = first.LastTransitionMs
            };
            return new OperationLedger
            {
                Schema = operation.Schema,
                OperationId = operation.OperationId,
                ActionId = operation.ActionId,
                CreatedAtMs = operation.CreatedAtMs,
                Lifecycle = operation.Lifecycle,
                MaxParallelism = operation.MaxParallelism,
                MaxAttemptsPerTarget = operation.MaxAttemptsPerTarget,
                CleanupRequired = operation.CleanupRequired,
                Targets = damagedTargets,
                Preview = operation.Preview
            };
        }

        private OperationLedger CreateOperation(
            IReadOnlyDictionary<string, ulong> identities,
            bool executed)
        {
            var canonicalIdentities = new SortedDictionary<string, ulong>(
                StringComparer.Ordinal);
            foreach (var identity in identities)
            {
                canonicalIdentities.Add(identity.Key, identity.Value);
            }

            var createdAt = Projection.AsOfMs;
            var ownerContract = CreateOwnerContract();
            var preflights = canonicalIdentities
                .Select((identity, index) =>
                    new OperationTargetPreflight
                    {
                        DeviceId = identity.Key,
                        IdentityRevision = identity.Value,
                        CapabilityId = "rusty-kiosk.direct-operator",
                        CapabilityEvidenceRevision = (ulong)(31 + index),
                        CapabilityOwner = "rusty-kiosk",
                        Support = "supported",
                        Enablement = "enabled",
                        Authorization = index == 0
                            ? "authorized"
                            : "unauthorized",
                        Reachability = "reachable",
                        Freshness = "current",
                        ObservedAtMs = createdAt - 100,
                        FreshUntilMs = createdAt + 30_000,
                        EvaluatedAtMs = createdAt + 100,
                        Eligible = index == 0,
                        ReasonCode = index == 0 ? "ready" : "unauthorized",
                        Message = index == 0
                            ? "Kiosk direct operator is current and ready."
                            : "Kiosk direct operator is not authorized for this target."
                    })
                .ToArray();
            var operationId = "operation-kiosk-show-controls-0001";
            var targets = preflights
                .Select(preflight =>
                {
                    var eligible = preflight.Eligible;
                    var ownerRequestId = eligible && executed
                        ? "fleetreq-0001"
                        : null;
                    return new OperationTargetResult
                    {
                        DeviceId = preflight.DeviceId,
                        IdentityRevision = preflight.IdentityRevision,
                        Preflight = preflight,
                        Lifecycle = eligible
                            ? executed
                                ? "applied"
                                : "proposed"
                            : "rejected",
                        DispatchedAtMs = eligible && executed
                            ? createdAt + 200
                            : null,
                        OwnerDeadlineAtMs = eligible && executed
                            ? createdAt + 60_000
                            : null,
                        AttemptCount = (byte)(eligible && executed ? 1 : 0),
                        OwnerRequestIds = ownerRequestId is null
                            ? []
                            : [ownerRequestId],
                        OwnerRequestId = ownerRequestId,
                        EffectiveReceipt = ownerRequestId is null
                            ? null
                            : new KioskEffectiveReceipt
                            {
                                Schema = "rusty.fleet.kiosk_effective_receipt.v1",
                                ReceiptId = "receipt-kiosk-show-controls-0001",
                                OperationId = operationId,
                                DeviceId = preflight.DeviceId,
                                IdentityRevision = preflight.IdentityRevision,
                                OwnerContract = ownerContract,
                                OwnerActionRequestId = ownerRequestId,
                                OwnerResultTransportRequestId = "fleet-poll-0001",
                                OwnerCommand = "show-controls",
                                ResponseStatus = 200,
                                ResponseContentSha256 = new string('a', 64),
                                ResponseSignature = new string('b', 64),
                                ResponseAuthVerified = true,
                                OwnerResultSchema = "rusty.kiosk.cli_result.v1",
                                OwnerAccepted = true,
                                OwnerCompleted = true,
                                OwnerRecordedAtMs = createdAt + 1_500,
                                ControlsOpen = true,
                                WrappedAtMs = createdAt + 1_600
                            },
                        RetryDisposition = "not_eligible",
                        CancelDisposition = eligible && !executed
                            ? "cancelable_before_dispatch"
                            : "terminal",
                        ReasonCode = eligible
                            ? executed
                                ? "owner_effective_receipt"
                                : "ready"
                            : "unauthorized",
                        Message = eligible
                            ? executed
                                ? "Kiosk reported the controls surface open."
                                : "Exact identity is eligible for Show Kiosk controls"
                            : "Target was excluded before dispatch.",
                        LastTransitionMs = createdAt + (executed ? 1_600 : 100)
                    };
                })
                .ToArray();
            return new OperationLedger
            {
                Schema = "rusty.fleet.kiosk_show_controls_operation.v1",
                OperationId = operationId,
                ActionId = FleetOperationActions.KioskShowControls,
                CreatedAtMs = createdAt,
                Preview = new OperationPreview
                {
                    Schema = "rusty.fleet.kiosk_show_controls_preview.v1",
                    PreviewId = "preview-kiosk-show-controls-0001",
                    OperationId = operationId,
                    ActionId = FleetOperationActions.KioskShowControls,
                    CreatedAtMs = createdAt,
                    ExpiresAtMs = createdAt + 60_000,
                    FleetRevision = Projection.ResultRevision,
                    OwnerContract = ownerContract,
                    Targets = preflights
                },
                Lifecycle = executed ? "applied" : "proposed",
                MaxParallelism = 8,
                MaxAttemptsPerTarget = 3,
                CleanupRequired = false,
                Targets = targets
            };
        }

        private PackageInstallReleaseOperation CreatePackageOperation(
            PackageInstallReleasePreviewRequest request,
            bool prepared)
        {
            var canonicalIdentities = new SortedDictionary<string, ulong>(
                StringComparer.Ordinal);
            foreach (var target in request.Targets)
            {
                canonicalIdentities.Add(target.Key, target.Value);
            }
            var createdAt = Projection.AsOfMs;
            var operationId = "operation-package-install-release-0001";
            var previewId = "preview-package-install-release-0001";
            var preflights = canonicalIdentities
                .Select((identity, index) =>
                    new OperationTargetPreflight
                    {
                        DeviceId = identity.Key,
                        IdentityRevision = identity.Value,
                        CapabilityId = "rusty-quest.package-updater",
                        CapabilityEvidenceRevision = (ulong)(61 + index),
                        CapabilityOwner = "rusty-quest",
                        Support = "supported",
                        Enablement = "enabled",
                        Authorization = "authorized",
                        Reachability = "reachable",
                        Freshness = "current",
                        ObservedAtMs = createdAt - 100,
                        FreshUntilMs = createdAt + 30_000,
                        EvaluatedAtMs = createdAt + 100,
                        Eligible = true,
                        ReasonCode = "ready",
                        Message = "Attended package updater is current and ready."
                    })
                .ToArray();
            var targets = preflights
                .Select((preflight, index) =>
                {
                    var ownerRequestId = $"package-owner-{index + 1:D4}";
                    return new PackageInstallTargetLedger
                    {
                        DeviceId = preflight.DeviceId,
                        IdentityRevision = preflight.IdentityRevision,
                        Preflight = preflight,
                        Lifecycle = prepared ? "accepted" : "proposed",
                        Stage = prepared ? "dispatch_ready" : "preview_ready",
                        Invocation = prepared
                            ? new PackageUpdaterInvocation
                            {
                                Schema = "rusty.fleet.package_updater_invocation.v1",
                                OperationId = operationId,
                                PreviewId = previewId,
                                DeviceId = preflight.DeviceId,
                                IdentityRevision = preflight.IdentityRevision,
                                OwnerActionRequestId = ownerRequestId,
                                Release = request.Release,
                                ExpectedPackageName = request.ExpectedPackageName,
                                ExpectedRolloutRing = request.ExpectedRolloutRing,
                                ExpiresAtMs = createdAt + 60_000
                            }
                            : null,
                        InvocationAcknowledgement = null,
                        EffectiveReceipt = null,
                        ReasonCode = prepared
                            ? "owner_dispatch_ready"
                            : "preview_ready",
                        Message = prepared
                            ? "Exact updater invocation is ready for delivery; application remains unproven"
                            : "Target is ready for explicit confirmation",
                        LastTransitionMs = createdAt + (prepared ? 200 : 100)
                    };
                })
                .ToArray();
            return new PackageInstallReleaseOperation
            {
                Schema = "rusty.fleet.package_install_release_operation.v1",
                OperationId = operationId,
                ActionId = PackageOperationActions.InstallRelease,
                CreatedAtMs = createdAt,
                Preview = new PackageInstallReleasePreview
                {
                    Schema = "rusty.fleet.package_install_release_preview.v1",
                    PreviewId = previewId,
                    OperationId = operationId,
                    ActionId = PackageOperationActions.InstallRelease,
                    CreatedAtMs = createdAt,
                    ExpiresAtMs = createdAt + 60_000,
                    FleetRevision = Projection.ResultRevision,
                    Release = request.Release,
                    ExpectedPackageName = request.ExpectedPackageName,
                    ExpectedRolloutRing = request.ExpectedRolloutRing,
                    OwnerContract = new PackageUpdaterOwnerContractBinding
                    {
                        OwnerRepoId = "rusty-quest",
                        CapabilityId = "rusty-quest.package-updater",
                        ManifestEnvelopeSchema =
                            "rusty.quest.package_update_manifest_envelope.v1",
                        ReceiptSchema = "rusty.quest.package_update_receipt.v1",
                        InstallMode = "attended_package_installer",
                        ApplicationProof = "effective_installed_version_receipt"
                    },
                    Targets = preflights
                },
                Lifecycle = prepared ? "accepted" : "proposed",
                MaxParallelism = 1,
                CleanupRequired = false,
                Targets = targets
            };
        }

        private WindowsHotspotOperation CreateWindowsHotspotOperation(
            WindowsHotspotPreviewRequest request,
            bool executed)
        {
            const string operationId = "hotspot-operation-0001";
            const string previewId = "hotspot-preview-0001";
            const string leaseGeneration = "hotspot-lease-generation-0001";
            const string ownershipGeneration = "hotspot-owned-generation-0001";
            var createdAt = Projection.AsOfMs;
            var active =
                WindowsHotspotOwnership != WindowsHotspotActions.OwnershipNone;
            var observedGeneration =
                WindowsHotspotOwnership == WindowsHotspotActions.OwnershipFleet
                    ? ownershipGeneration
                    : null;
            var actionEligible = request.Action switch
            {
                WindowsHotspotActions.Status => true,
                WindowsHotspotActions.Start or WindowsHotspotActions.Ensure =>
                    WindowsHotspotOwnership !=
                    WindowsHotspotActions.OwnershipExternal,
                WindowsHotspotActions.Stop =>
                    WindowsHotspotOwnership ==
                    WindowsHotspotActions.OwnershipFleet,
                _ => false
            };
            var eligible =
                WindowsHotspotProviderReady &&
                WindowsHotspotLeaseAvailable &&
                actionEligible;
            var reason = !WindowsHotspotProviderReady
                ? "provider_unavailable"
                : !WindowsHotspotLeaseAvailable
                    ? "resource_leased"
                    : WindowsHotspotOwnership ==
                      WindowsHotspotActions.OwnershipExternal
                        ? "external_hotspot_not_owned"
                        : request.Action == WindowsHotspotActions.Stop &&
                          !actionEligible
                            ? "fleet_ownership_required"
                            : "ready";
            var lifecycle = eligible
                ? executed ? "applied" : "proposed"
                : "rejected";
            var invocationGeneration = request.Action switch
            {
                WindowsHotspotActions.Ensure =>
                    observedGeneration,
                WindowsHotspotActions.Stop =>
                    observedGeneration,
                _ => null
            };
            var invocation = executed
                ? new WindowsHotspotProviderRequest
                {
                    Schema =
                        "rusty.hostess.windows_hotspot.provider_request.v1",
                    RequestId = "hotspot-request-0001",
                    OperationId = operationId,
                    Action = request.Action,
                    ExpiresAtUtc = DateTimeOffset
                        .FromUnixTimeMilliseconds(createdAt + 60_000)
                        .UtcDateTime
                        .ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'"),
                    TimeoutMs = 30_000,
                    OwnershipGeneration = invocationGeneration
                }
                : null;
            var receipt = executed
                ? new WindowsHotspotProviderReceipt
                {
                    Schema =
                        "rusty.hostess.windows_hotspot.provider_receipt.v1",
                    RequestId = "hotspot-request-0001",
                    OperationId = operationId,
                    Action = request.Action,
                    Outcome = WindowsHotspotActions.ResultVerified,
                    Reason = "effective_readback_verified",
                    ObservedAtUtc = DateTimeOffset
                        .FromUnixTimeMilliseconds(createdAt + 300)
                        .UtcDateTime
                        .ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'"),
                    CapabilityAvailable = true,
                    Capability = "Available",
                    OperationalState =
                        request.Action == WindowsHotspotActions.Stop
                            ? "Off"
                            : request.Action == WindowsHotspotActions.Status
                                ? active ? "On" : "Off"
                                : "On",
                    ClientCount = request.Action == WindowsHotspotActions.Stop
                        ? 0u
                        : 1u,
                    MaxClientCount = 8,
                    Band = "FiveGigahertz",
                    SourceConnectivity = "Internet",
                    OwnershipGeneration = request.Action switch
                    {
                        WindowsHotspotActions.Start or
                        WindowsHotspotActions.Ensure =>
                            ownershipGeneration,
                        WindowsHotspotActions.Status =>
                            observedGeneration,
                        _ => null
                    }
                }
                : null;

            return new WindowsHotspotOperation
            {
                Schema = "rusty.fleet.windows_hotspot_operation.v1",
                OperationId = operationId,
                ActionId = WindowsHotspotActions.ActionId,
                Lifecycle = lifecycle,
                Preview = new WindowsHotspotPreview
                {
                    Schema = "rusty.fleet.windows_hotspot_preview.v1",
                    PreviewId = previewId,
                    OperationId = operationId,
                    ActionId = WindowsHotspotActions.ActionId,
                    ResourceId = WindowsHotspotActions.ResourceId,
                    OwnerId = WindowsHotspotActions.OwnerId,
                    Action = request.Action,
                    CreatedAtMs = createdAt,
                    ExpiresAtMs = createdAt + 60_000,
                    FleetRevision = Projection.ResultRevision,
                    Preflight = new WindowsHotspotPreflight
                    {
                        ProviderReady = WindowsHotspotProviderReady,
                        LeaseAvailable = WindowsHotspotLeaseAvailable,
                        Active = active,
                        Ownership = WindowsHotspotOwnership,
                        OwnershipGeneration = observedGeneration,
                        Eligible = eligible,
                        ReasonCode = reason,
                        Message = eligible
                            ? "Host-scoped hotspot operation is ready for explicit confirmation."
                            : "Host-scoped hotspot operation is not eligible."
                    }
                },
                ConfirmedAtMs = executed ? createdAt + 50 : null,
                Lease = new WindowsHotspotLease
                {
                    Schema = "rusty.fleet.host_resource_lease.v1",
                    LeaseId = "hotspot-lease-0001",
                    ResourceId = WindowsHotspotActions.ResourceId,
                    HolderOperationId = operationId,
                    Generation = leaseGeneration,
                    IssuedAtMs = createdAt,
                    ExpiresAtMs = createdAt + 60_000
                },
                Invocation = invocation,
                Receipt = receipt,
                FailureCode = eligible
                    ? null
                    : reason,
                UpdatedAtMs = createdAt + (executed ? 300 : 10)
            };
        }

        private WindowsHotspotOperation ReturnWindowsHotspotOperation()
        {
            var operation = LastWindowsHotspotOperation ??
                            throw new InvalidOperationException(
                                "synthetic host-hotspot operation was not created");
            if (!DamageNextWindowsHotspotResponse)
            {
                return operation;
            }

            DamageNextWindowsHotspotResponse = false;
            var damaged = JsonNode.Parse(
                JsonSerializer.Serialize(operation, FleetJson.Options)) ??
                throw new InvalidOperationException(
                    "synthetic host-hotspot operation could not be cloned");
            damaged["preview"]!["preview_id"] =
                "hotspot-preview-damaged";
            return JsonSerializer.Deserialize<WindowsHotspotOperation>(
                       damaged.ToJsonString(),
                       FleetJson.Options) ??
                   throw new InvalidOperationException(
                       "damaged host-hotspot operation could not be projected");
        }

        private QuestAwakeOperation CreateQuestAwakeOperation(
            QuestAwakePreviewRequest request,
            bool executed)
        {
            var identities = new SortedDictionary<string, ulong>(
                StringComparer.Ordinal);
            foreach (var target in request.Targets)
            {
                identities.Add(target.Key, target.Value);
            }
            var createdAt = Projection.AsOfMs;
            var operationId = "awake-operation-0001";
            var previewId = "awake-preview-0001";
            var generation = "awake-generation-0001";
            var invocationGeneration =
                executed &&
                (request.Action is
                    QuestAwakeActions.StopWatchdogs or
                    QuestAwakeActions.RestoreNormal) &&
                QuestAwakeInvocationGenerationOverride is not null
                    ? QuestAwakeInvocationGenerationOverride
                    : generation;
            var preflights = identities
                .Select((identity, index) =>
                    new QuestAwakeTargetPreflight
                    {
                        DeviceId = identity.Key,
                        IdentityRevision = identity.Value,
                        CapabilityId =
                            "questionable-file-manager.quest-awake-provider",
                        CapabilityEvidenceRevision = (ulong)(91 + index),
                        CapabilityOwner = "questionable-file-manager",
                        Support = "supported",
                        Enablement = "enabled",
                        Authorization = "authorized",
                        Reachability = "reachable",
                        Freshness = "current",
                        ObservedAtMs = createdAt - 100,
                        FreshUntilMs = createdAt + 30_000,
                        EvaluatedAtMs = createdAt,
                        Eligible = true,
                        ReasonCode = "ready",
                        Message = "Pinned headset awake provider is current and ready."
                    })
                .ToArray();
            var targets = preflights
                .Select((preflight, index) =>
                {
                    var requestId = $"fleetawake-{index + 1:D4}";
                    var invocation = executed
                        ? new QuestAwakeOwnerInvocation
                        {
                            Schema =
                                "rusty.fleet.quest_awake_owner_invocation.v1",
                            RequestId = requestId,
                            OperationId = operationId,
                            PreviewId = previewId,
                            DeviceId = preflight.DeviceId,
                            IdentityRevision = preflight.IdentityRevision,
                            Action = request.Action,
                            DurationMs = request.DurationMs,
                            WatchdogIntervalMs = request.WatchdogIntervalMs,
                            WatchdogGeneration = invocationGeneration,
                            IssuedAtMs = createdAt + 100,
                            ExpiresAtMs = createdAt + 60_000
                        }
                        : null;
                    var receipt = executed
                        ? CreateQuestAwakeReceipt(
                            request,
                            preflight,
                            operationId,
                            previewId,
                            invocationGeneration,
                            requestId,
                            createdAt)
                        : null;
                    return new QuestAwakeTargetLedger
                    {
                        DeviceId = preflight.DeviceId,
                        IdentityRevision = preflight.IdentityRevision,
                        Preflight = preflight,
                        Lifecycle = executed ? "applied" : "proposed",
                        Invocation = invocation,
                        Receipt = receipt,
                        FailureCode = null,
                        UpdatedAtMs = createdAt + (executed ? 300 : 10)
                    };
                })
                .ToArray();
            return new QuestAwakeOperation
            {
                Schema = "rusty.fleet.quest_awake_operation.v1",
                OperationId = operationId,
                ActionId = QuestAwakeActions.ActionId,
                Lifecycle = executed ? "applied" : "proposed",
                Preview = new QuestAwakePreview
                {
                    Schema = "rusty.fleet.quest_awake_preview.v1",
                    PreviewId = previewId,
                    OperationId = operationId,
                    ActionId = QuestAwakeActions.ActionId,
                    Action = request.Action,
                    CreatedAtMs = createdAt,
                    ExpiresAtMs = createdAt + 60_000,
                    FleetRevision = Projection.ResultRevision,
                    DurationMs = request.DurationMs,
                    WatchdogIntervalMs = request.WatchdogIntervalMs,
                    WatchdogGeneration = generation,
                    Owner = new QuestAwakeOwnerBinding
                    {
                        OwnerRepoId = "questionable-file-manager",
                        CapabilityId =
                            "questionable-file-manager.quest-awake-provider",
                        ProviderContract =
                            "questionable.file_manager.fleet_awake_provider.v1",
                        ReceiptSchema =
                            "questionable.file_manager.quest_awake_receipt.v1",
                        Transport = "pinned_local_subprocess",
                        ApplicationProof =
                            "fresh_effective_power_and_watchdog_readback"
                    },
                    Targets = preflights
                },
                ConfirmedAtMs = executed ? createdAt + 50 : null,
                Targets = targets,
                UpdatedAtMs = createdAt + (executed ? 300 : 10)
            };
        }

        private static QuestAwakeOwnerReceipt CreateQuestAwakeReceipt(
            QuestAwakePreviewRequest request,
            QuestAwakeTargetPreflight preflight,
            string operationId,
            string previewId,
            string generation,
            string requestId,
            long createdAt)
        {
            var keepAwake = request.Action is
                QuestAwakeActions.ApplyBounded or
                QuestAwakeActions.StartWindowsWatchdog or
                QuestAwakeActions.StartDeviceWatchdog;
            var windows =
                request.Action == QuestAwakeActions.StartWindowsWatchdog;
            var device =
                request.Action == QuestAwakeActions.StartDeviceWatchdog;
            var settingsLeft =
                request.Action == QuestAwakeActions.StopWatchdogs;
            var restored =
                request.Action == QuestAwakeActions.RestoreNormal;
            return new QuestAwakeOwnerReceipt
            {
                Schema = "questionable.file_manager.quest_awake_receipt.v1",
                RequestId = requestId,
                OperationId = operationId,
                PreviewId = previewId,
                DeviceId = preflight.DeviceId,
                IdentityRevision = preflight.IdentityRevision,
                Action = request.Action,
                WatchdogGeneration = generation,
                RequestedDurationMs = request.DurationMs,
                RequestedWatchdogIntervalMs = request.WatchdogIntervalMs,
                StayOnEffective = keepAwake,
                ProximityHoldEffective = keepAwake,
                WakeEffective = keepAwake,
                WindowsWatchdogEffective = windows,
                DeviceWatchdogEffective = device,
                SettingsRestored = restored,
                Effective = true,
                SettingsLeftUnchanged = settingsLeft,
                Outcome = request.Action == QuestAwakeActions.Status
                    ? "Current awake state read"
                    : "Requested awake state verified",
                RepairCount = keepAwake ? 1u : 0u,
                Power = new QuestAwakePowerReadback
                {
                    Wakefulness = keepAwake ? "awake" : "asleep",
                    DisplayState = keepAwake ? "on" : "off",
                    StayOn = keepAwake,
                    AutoSleepDisabled = keepAwake,
                    ProximityState = keepAwake ? "close" : "open",
                    ProximityHoldDurationMs = keepAwake
                        ? request.DurationMs
                        : null,
                    ProximityHoldRemainingMs = keepAwake
                        ? request.DurationMs - 1_000
                        : null,
                    CapturedAtMs = createdAt + 300
                },
                DeviceWatchdog = new QuestAwakeWatchdogReadback
                {
                    ReportedActive = device,
                    Fresh = device,
                    Generation = device ? generation : string.Empty,
                    BootIdSha256 = new string('b', 64),
                    IntervalMs = device ? request.WatchdogIntervalMs : 0,
                    LastPollMs = device ? createdAt + 250 : 0,
                    ProximityRepairCount = device ? 1u : 0u,
                    StayOnRepairCount = 0,
                    WakeRepairCount = 0,
                    LastAction = device ? "proximity repaired" : "inactive",
                    LastError = string.Empty
                },
                EvidenceSha256 = new string('a', 64),
                ObservedAtMs = createdAt + 300
            };
        }

        private PackageInstallReleaseOperation ReturnPackageOperation()
        {
            var operation = LastPackageOperation ??
                            throw new InvalidOperationException(
                                "synthetic package operation was not created");
            if (!DamageNextPackageOperationResponse)
            {
                return operation;
            }

            DamageNextPackageOperationResponse = false;
            var targets = operation.Targets.ToArray();
            var first = targets[0];
            targets[0] = new PackageInstallTargetLedger
            {
                DeviceId = first.DeviceId,
                IdentityRevision = first.IdentityRevision + 1,
                Preflight = first.Preflight,
                Lifecycle = first.Lifecycle,
                Stage = first.Stage,
                Invocation = first.Invocation,
                InvocationAcknowledgement = first.InvocationAcknowledgement,
                EffectiveReceipt = first.EffectiveReceipt,
                ReasonCode = first.ReasonCode,
                Message = first.Message,
                LastTransitionMs = first.LastTransitionMs
            };
            return new PackageInstallReleaseOperation
            {
                Schema = operation.Schema,
                OperationId = operation.OperationId,
                ActionId = operation.ActionId,
                CreatedAtMs = operation.CreatedAtMs,
                Preview = operation.Preview,
                Lifecycle = operation.Lifecycle,
                MaxParallelism = operation.MaxParallelism,
                CleanupRequired = operation.CleanupRequired,
                Targets = targets
            };
        }

        private QuestAwakeOperation ReturnQuestAwakeOperation()
        {
            var operation = LastQuestAwakeOperation ??
                            throw new InvalidOperationException(
                                "synthetic awake-control operation was not created");
            if (DamageNextQuestAwakeReceiptGeneration)
            {
                DamageNextQuestAwakeReceiptGeneration = false;
                var damaged = JsonNode.Parse(
                    JsonSerializer.Serialize(operation, FleetJson.Options)) ??
                    throw new InvalidOperationException(
                        "synthetic awake-control operation could not be cloned");
                damaged["targets"]![0]!["receipt"]!["watchdog_generation"] =
                    "mismatched-observed-generation";
                return JsonSerializer.Deserialize<QuestAwakeOperation>(
                           damaged.ToJsonString(),
                           FleetJson.Options) ??
                       throw new InvalidOperationException(
                           "damaged awake-control operation could not be projected");
            }
            if (!DamageNextQuestAwakeResponse)
            {
                return operation;
            }

            DamageNextQuestAwakeResponse = false;
            var targets = operation.Targets.ToArray();
            var first = targets[0];
            targets[0] = new QuestAwakeTargetLedger
            {
                DeviceId = first.DeviceId,
                IdentityRevision = first.IdentityRevision + 1,
                Preflight = first.Preflight,
                Lifecycle = first.Lifecycle,
                Invocation = first.Invocation,
                Receipt = first.Receipt,
                FailureCode = first.FailureCode,
                UpdatedAtMs = first.UpdatedAtMs
            };
            return new QuestAwakeOperation
            {
                Schema = operation.Schema,
                OperationId = operation.OperationId,
                ActionId = operation.ActionId,
                Lifecycle = operation.Lifecycle,
                Preview = operation.Preview,
                ConfirmedAtMs = operation.ConfirmedAtMs,
                Targets = targets,
                UpdatedAtMs = operation.UpdatedAtMs
            };
        }

        private QuestWifiAdbOperation CreateQuestWifiAdbOperation(
            QuestWifiAdbPreviewRequest request,
            bool executed)
        {
            var identities = new SortedDictionary<string, ulong>(
                StringComparer.Ordinal);
            foreach (var target in request.Targets)
            {
                identities.Add(target.Key, target.Value);
            }

            var createdAt = Projection.AsOfMs;
            var operationId = "wifi-adb-operation-0001";
            var previewId = "wifi-adb-preview-0001";
            var preflights = identities
                .Select((identity, index) =>
                    new QuestWifiAdbTargetPreflight
                    {
                        DeviceId = identity.Key,
                        IdentityRevision = identity.Value,
                        CapabilityId =
                            "questionable-file-manager.quest-wifi-adb-provider",
                        CapabilityEvidenceRevision = (ulong)(121 + index),
                        CapabilityOwner = "questionable-file-manager",
                        Support = "supported",
                        Enablement = "enabled",
                        Authorization = "authorized",
                        Reachability = "reachable",
                        Freshness = "current",
                        ObservedAtMs = createdAt - 100,
                        FreshUntilMs = createdAt + 30_000,
                        EvaluatedAtMs = createdAt,
                        Eligible = true,
                        ReasonCode = "ready",
                        Message =
                            "Pinned Quest connectivity provider is current and ready."
                    })
                .ToArray();
            var targets = preflights
                .Select((preflight, index) =>
                {
                    var requestId = $"fleetwifi-{index + 1:D4}";
                    var invocation = executed
                        ? new QuestWifiAdbOwnerInvocation
                        {
                            Schema =
                                "rusty.fleet.quest_wifi_adb_owner_invocation.v1",
                            RequestId = requestId,
                            OperationId = operationId,
                            PreviewId = previewId,
                            DeviceId = preflight.DeviceId,
                            IdentityRevision = preflight.IdentityRevision,
                            Action = request.Action,
                            IssuedAtMs = createdAt + 100,
                            ExpiresAtMs = createdAt + 60_000
                        }
                        : null;
                    var receipt = executed
                        ? CreateQuestWifiAdbReceipt(
                            request.Action,
                            preflight,
                            operationId,
                            previewId,
                            requestId,
                            createdAt)
                        : null;
                    var proof = executed &&
                                request.Action ==
                                QuestWifiAdbActions.RequestWirelessAdb
                        ? new QuestWifiAdbTermuxProof
                        {
                            Schema =
                                "rusty.fleet.quest_wifi_adb_termux_proof.v1",
                            ProofId = $"termux-proof-{index + 1:D4}",
                            OwnerId = "quest-termux-lab.loopback-proof",
                            DeviceId = preflight.DeviceId,
                            IdentityRevision = preflight.IdentityRevision,
                            SourceEpoch = "boot-epoch-connectivity-0001",
                            SourceRevision = (ulong)(501 + index),
                            RouteMode = "modern_tls",
                            DiscoveryMode = "tls_nsd",
                            ListenerDiscovered = true,
                            ShellIdentity = "uid=2000(shell)",
                            Available = true,
                            EvidenceSha256 = new string('c', 64),
                            ObservedAtMs = createdAt + 400,
                            FreshUntilMs = createdAt + 60_400
                        }
                        : null;
                    return new QuestWifiAdbTargetLedger
                    {
                        DeviceId = preflight.DeviceId,
                        IdentityRevision = preflight.IdentityRevision,
                        Preflight = preflight,
                        Lifecycle = executed ? "applied" : "proposed",
                        Invocation = invocation,
                        Receipt = receipt,
                        TermuxProof = proof,
                        TermuxUsable = proof?.Available == true,
                        FailureCode = null,
                        UpdatedAtMs = createdAt + (proof is null ? 300 : 400)
                    };
                })
                .ToArray();
            return new QuestWifiAdbOperation
            {
                Schema = "rusty.fleet.quest_wifi_adb_operation.v1",
                OperationId = operationId,
                ActionId = QuestWifiAdbActions.ActionId,
                Lifecycle = executed ? "applied" : "proposed",
                Preview = new QuestWifiAdbPreview
                {
                    Schema = "rusty.fleet.quest_wifi_adb_preview.v1",
                    PreviewId = previewId,
                    OperationId = operationId,
                    ActionId = QuestWifiAdbActions.ActionId,
                    Action = request.Action,
                    CreatedAtMs = createdAt,
                    ExpiresAtMs = createdAt + 60_000,
                    FleetRevision = Projection.ResultRevision,
                    Owner = new QuestWifiAdbOwnerBinding
                    {
                        OwnerRepoId = "questionable-file-manager",
                        CapabilityId =
                            "questionable-file-manager.quest-wifi-adb-provider",
                        ProviderContract =
                            "questionable.file_manager.fleet_connectivity_provider.v1",
                        ReceiptSchema =
                            "questionable.file_manager.quest_wifi_adb_receipt.v1",
                        Transport = "pinned_local_subprocess",
                        PrivateTargetResolution =
                            "provider_owned_credential_profile"
                    },
                    Targets = preflights
                },
                ConfirmedAtMs = executed ? createdAt + 50 : null,
                Targets = targets,
                UpdatedAtMs = createdAt +
                              (executed &&
                               request.Action ==
                               QuestWifiAdbActions.RequestWirelessAdb
                                  ? 400
                                  : 300)
            };
        }

        private static QuestWifiAdbOwnerReceipt CreateQuestWifiAdbReceipt(
            string action,
            QuestWifiAdbTargetPreflight preflight,
            string operationId,
            string previewId,
            string requestId,
            long createdAt)
        {
            var requestWireless =
                action == QuestWifiAdbActions.RequestWirelessAdb;
            var classic =
                action == QuestWifiAdbActions.EnableClassicTcpipFromUsb;
            var kiosk = !classic && action != QuestWifiAdbActions.Status;
            return new QuestWifiAdbOwnerReceipt
            {
                Schema =
                    "questionable.file_manager.quest_wifi_adb_receipt.v1",
                RequestId = requestId,
                OperationId = operationId,
                PreviewId = previewId,
                DeviceId = preflight.DeviceId,
                IdentityRevision = preflight.IdentityRevision,
                Action = action,
                RouteMode = requestWireless
                    ? "modern_tls"
                    : classic
                        ? "classic_tcpip"
                        : "none",
                RequestDelivered = true,
                KioskSettingApplied = kiosk,
                RequestAfterBootEnabled = action switch
                {
                    QuestWifiAdbActions.EnableRequestAfterBoot => true,
                    QuestWifiAdbActions.DisableRequestAfterBoot => false,
                    _ => null
                },
                WearerApproval = requestWireless
                    ? "pending"
                    : classic
                        ? "not_applicable"
                        : "unknown",
                ListenerDiscovered = requestWireless || classic,
                EffectApplied = true,
                Outcome = action switch
                {
                    QuestWifiAdbActions.Status =>
                        "Current connectivity facts read",
                    QuestWifiAdbActions.RequestWirelessAdb =>
                        "Wireless ADB requested; wearer approval remains pending",
                    QuestWifiAdbActions.EnableRequestAfterBoot =>
                        "After-boot request enabled",
                    QuestWifiAdbActions.DisableRequestAfterBoot =>
                        "After-boot request disabled",
                    QuestWifiAdbActions.DisableWirelessAdb =>
                        "Wireless ADB disabled",
                    _ => "Classic USB tcpip listener observed"
                },
                EvidenceSha256 = new string('d', 64),
                ObservedAtMs = createdAt + 300
            };
        }

        private QuestWifiAdbOperation ReturnQuestWifiAdbOperation()
        {
            var operation = LastQuestWifiAdbOperation ??
                            throw new InvalidOperationException(
                                "synthetic connectivity operation was not created");
            if (!DamageNextQuestWifiAdbResponse)
            {
                return operation;
            }

            DamageNextQuestWifiAdbResponse = false;
            var damaged = JsonNode.Parse(
                JsonSerializer.Serialize(operation, FleetJson.Options)) ??
                throw new InvalidOperationException(
                    "synthetic connectivity operation could not be cloned");
            damaged["targets"]![0]!["identity_revision"] =
                operation.Targets[0].IdentityRevision + 1;
            return JsonSerializer.Deserialize<QuestWifiAdbOperation>(
                       damaged.ToJsonString(),
                       FleetJson.Options) ??
                   throw new InvalidOperationException(
                       "damaged connectivity operation could not be projected");
        }

        private static KioskOwnerContractBinding CreateOwnerContract() =>
            new()
            {
                OwnerRepoId = "rusty-kiosk",
                OwnerContractSchema = "rusty.kiosk.direct_operator.v1",
                OwnerContractRevision =
                    "8954228f9ae67c5995a72569e3c9cdd3758f85c0",
                CapabilityId = "rusty-kiosk.direct-operator",
                RequestAuth = "hmac-sha256-v1",
                ResponseAuth = "hmac-sha256-response-v1",
                InvokeMethod = "POST",
                InvokeTarget = "/v1/kiosk/invoke",
                ResultMethod = "GET",
                ResultTarget = "/v1/kiosk/result",
                ResultRequestIdParameter = "request_id",
                Port = 39_873,
                MaxClockSkewSeconds = 90,
                Command = "show-controls",
                CommandValue = null,
                OwnerResultSchema = "rusty.kiosk.cli_result.v1"
            };

        private static bool Matches(object? expression, DeviceRowProjection row)
        {
            if (expression is null)
            {
                return true;
            }

            return MatchesElement(
                JsonSerializer.SerializeToElement(expression, FleetJson.Options),
                row);
        }

        private static bool MatchesElement(
            JsonElement expression,
            DeviceRowProjection row)
        {
            var kind = expression.GetProperty("kind").GetString();
            if (kind is "and" or "or")
            {
                var values = expression.GetProperty("expressions")
                    .EnumerateArray()
                    .Select(item => MatchesElement(item, row));
                return kind == "and" ? values.All(value => value) : values.Any(value => value);
            }

            if (kind != "predicate")
            {
                throw new InvalidOperationException($"Unsupported test query kind {kind}.");
            }

            var field = expression.GetProperty("field").GetString();
            var comparison = expression.GetProperty("comparison").GetString();
            var expected = expression.GetProperty("value").GetString() ?? string.Empty;
            var actual = field switch
            {
                "display_name" => row.Identity.DisplayName,
                "device_id" => row.Identity.DeviceId,
                "freshness" => row.Freshness,
                _ => throw new InvalidOperationException(
                    $"Unsupported test query field {field}.")
            };
            return comparison switch
            {
                "contains" => actual.Contains(expected, StringComparison.OrdinalIgnoreCase),
                "equals" => actual.Equals(expected, StringComparison.OrdinalIgnoreCase),
                _ => throw new InvalidOperationException(
                    $"Unsupported test comparison {comparison}.")
            };
        }
    }
}
