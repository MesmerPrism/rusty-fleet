// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Diagnostics;
using System.IO;
using System.Text.Json;
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
            var json = RunFleetctl(repoRoot, 1_000);
            var deserializeWatch = Stopwatch.StartNew();
            var projection = FleetJson.DeserializeQueryResult(json);
            deserializeWatch.Stop();
            Require(projection.Schema == "rusty.fleet.query_result.v1", "wrong query schema");
            Require(projection.TotalCount == 1_000, "1,000-device projection was not loaded");
            Require(projection.Rows.Count == 1_000, "query window is incomplete");
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
                FleetProjectionValidation.ValidateQueryResult(
                    liveResult,
                    liveSummary,
                    liveQuery);
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
                }

                liveHubChecked = true;
            }

            var queryJson = JsonSerializer.Serialize(
                FleetQuery.Create("Quest 0001", "Stale"),
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
            }

            var source = new StaticFleetDataSource(projection);
            var workspace = new FleetWorkspaceViewModel(source);
            var viewModelWatch = Stopwatch.StartNew();
            workspace.InitializeAsync().GetAwaiter().GetResult();
            viewModelWatch.Stop();
            Require(workspace.Rows.Count == 1_000, "view model did not retain full window");
            Require(viewModelWatch.Elapsed < TimeSpan.FromSeconds(2), "1,000-row view model exceeded 2 seconds");

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

            workspace.SearchText = "Quest 0001";
            workspace.SelectedFreshness = "Fresh";
            workspace.SelectedGrouping = "Cohort";
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
            Require(workspace.Rows.Count == 0, "offline filter retained fresh devices");
            Require(
                workspace.BatchSelectionText.Contains("1 hidden by scope", StringComparison.Ordinal),
                "hidden batch selection was not retained");
            Require(
                workspace.Inspector?.Title == first.DisplayName &&
                workspace.InspectorContextText.Contains(
                    "outside the active scope",
                    StringComparison.Ordinal),
                "selected-device context was lost outside the active scope");

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
            var inspectorPeer = new ScrollViewerAutomationPeer(
                (ScrollViewer)window.InspectorRegion);
            Require(
                inspectorPeer.GetName() == "Selected device inspector",
                "inspector automation peer lost its accessible name");
            var batchCheckBox = FindVisualDescendant<CheckBox>(grid) ??
                throw new InvalidOperationException("visible batch checkbox was not realized");
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
            Require(realized is > 0 and < 250, "virtualized grid realized an invalid row set");
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
                projection_rows = projection.Rows.Count,
                deserialization_ms = deserializeWatch.Elapsed.TotalMilliseconds,
                view_model_ms = viewModelWatch.Elapsed.TotalMilliseconds,
                window_ms = windowWatch.Elapsed.TotalMilliseconds,
                realized_rows = realized,
                grid_columns = grid.Columns.Count,
                column_widths = columnWidths,
                native_datagrid = true,
                recycling_virtualization = true,
                native_automation_peer = true,
                inspector_automation_peer = true,
                pointer_batch_toggle = true,
                accessible_batch_toggle = true,
                loopback_hub_only = true,
                bounded_hub_response = true,
                live_hub_checked = liveHubChecked,
                projection_identity_fail_closed = true,
                canonical_scope = true,
                grouped_virtualization = true,
                hidden_selection_preserved = true,
                inspector_outside_scope_preserved = true,
                theme_dependency = "none",
                batch_selection_preserved = true,
                inspector_capability_families = inspector.Capabilities.Count,
                rendered_image = renderPath
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

    private static string RunFleetctl(string repoRoot, int count)
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
        start.ArgumentList.Add("list");
        start.ArgumentList.Add(count.ToString(System.Globalization.CultureInfo.InvariantCulture));
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
        bool echoQuery = true,
        bool wrongInspectorIdentity = false) : IFleetDataSource
    {
        public FleetQuery? LastQuery { get; private set; }

        public int QueryCount { get; private set; }

        public Task<FleetQueryResult> QueryAsync(
            FleetQuery query,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            LastQuery = query;
            QueryCount++;
            var matched = projection.Rows
                .Where(row => Matches(query.Expression, row))
                .ToArray();
            var window = matched
                .Skip(query.Offset)
                .Take(query.Limit)
                .ToArray();
            return Task.FromResult(new FleetQueryResult
            {
                Schema = projection.Schema,
                Query = echoQuery ? query : projection.Query,
                ResultRevision = projection.ResultRevision,
                AsOfMs = projection.AsOfMs,
                TotalCount = matched.Length,
                WindowOffset = query.Offset,
                WindowCount = window.Length,
                Rows = window
            });
        }

        public Task<FleetSummaryProjection> SummaryAsync(CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            return Task.FromResult(new FleetSummaryProjection
            {
                Schema = "rusty.fleet.summary.v1",
                AsOfMs = projection.AsOfMs,
                Total = projection.TotalCount,
                Fresh = projection.Rows.Count(row => row.Freshness == "fresh"),
                Stale = projection.Rows.Count(row => row.Freshness == "stale"),
                Offline = projection.Rows.Count(row => row.Freshness == "offline"),
                Attention = projection.Rows.Count(row =>
                    row.Conditions.Values.Any(condition =>
                        condition.State is "degraded" or "failed" or "critical")),
                ActiveWork = projection.Rows.Sum(row => row.ActiveWorkCount)
            });
        }

        public Task<DeviceInspectorProjection> InspectAsync(
            string deviceId,
            CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            var row = wrongInspectorIdentity
                ? projection.Rows.First(item => item.Identity.DeviceId != deviceId)
                : projection.Rows.Single(item => item.Identity.DeviceId == deviceId);
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
