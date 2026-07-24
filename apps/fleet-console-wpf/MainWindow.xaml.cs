// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using RustyFleet.FleetConsole.Contracts;
using RustyFleet.FleetConsole.Services;
using RustyFleet.FleetConsole.ViewModels;

namespace RustyFleet.FleetConsole;

public partial class MainWindow : Window
{
    private readonly FleetWorkspaceViewModel _viewModel;
    private int _focusRegion;

    public MainWindow()
        : this(new FleetWorkspaceViewModel(uri => new FleetApiClient(uri)))
    {
    }

    public MainWindow(FleetWorkspaceViewModel viewModel)
    {
        InitializeComponent();
        _viewModel = viewModel;
        DataContext = viewModel;
        for (var index = 0; index < FleetGrid.Columns.Count; index++)
        {
            FleetGrid.Columns[index].Visibility = Visibility.Visible;
            FleetGrid.Columns[index].DisplayIndex = index;
        }
        _viewModel.SavedViewRestorationRequested += RestoreSavedView;
    }

    public DataGrid FleetDataGrid => FleetGrid;

    public Button ApplyOrderingButton => ApplyLiveOrderingButton;

    public ComboBox SortFieldControl => SortFieldBox;

    public ComboBox SortDirectionControl => SortDirectionBox;

    public ComboBox SavedViewControl => SavedViewBox;

    public TextBox SavedViewNameControl => SavedViewNameBox;

    public FrameworkElement InspectorRegion => InspectorPane;

    protected override void OnClosed(EventArgs e)
    {
        _viewModel.SavedViewRestorationRequested -= RestoreSavedView;
        base.OnClosed(e);
    }

    private async void OnFleetSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (FleetGrid.SelectedItem is DeviceRowViewModel selected)
        {
            await _viewModel.SelectDeviceAsync(selected);
        }
    }

    private async void OnSaveCurrentView(object sender, RoutedEventArgs e)
    {
        var columns = FleetGrid.Columns
            .Where(column => column.Visibility == Visibility.Visible)
            .OrderBy(column => column.DisplayIndex)
            .Select(column => column.SortMemberPath)
            .Where(value => !string.IsNullOrWhiteSpace(value))
            .ToArray();
        await _viewModel.SaveCurrentViewAsync(columns, FocusedRegion());
    }

    private void RestoreSavedView(SavedView view)
    {
        var known = FleetGrid.Columns
            .Where(column => !string.IsNullOrWhiteSpace(column.SortMemberPath))
            .ToDictionary(column => column.SortMemberPath, StringComparer.Ordinal);
        var requested = view.Columns
            .Where(known.ContainsKey)
            .Distinct(StringComparer.Ordinal)
            .ToArray();
        if (requested.Length > 0)
        {
            foreach (var column in FleetGrid.Columns)
            {
                column.Visibility = Visibility.Collapsed;
            }

            for (var index = 0; index < requested.Length; index++)
            {
                var column = known[requested[index]];
                column.Visibility = Visibility.Visible;
                column.DisplayIndex = index;
            }
        }

        var anchorId = view.Restoration.ScrollAnchorDeviceId ??
                       view.Restoration.SelectedDeviceId;
        var anchor = anchorId is null
            ? null
            : _viewModel.Rows.FirstOrDefault(row => row.DeviceId == anchorId);
        if (anchor is not null)
        {
            FleetGrid.ScrollIntoView(anchor);
        }

        switch (view.Restoration.FocusedRegion)
        {
            case "search":
                SearchBox.Focus();
                break;
            case "saved_views":
                SavedViewBox.Focus();
                break;
            case "inspector":
                InspectorPane.Focus();
                break;
            case "grid":
                FleetGrid.Focus();
                break;
        }
    }

    private string FocusedRegion()
    {
        if (SearchBox.IsKeyboardFocusWithin)
        {
            return "search";
        }

        if (SavedViewBox.IsKeyboardFocusWithin ||
            SavedViewNameBox.IsKeyboardFocusWithin)
        {
            return "saved_views";
        }

        if (FleetGrid.IsKeyboardFocusWithin)
        {
            return "grid";
        }

        return InspectorPane.IsKeyboardFocusWithin ? "inspector" : "shell";
    }

    private async void OnPreviewKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.F && Keyboard.Modifiers.HasFlag(ModifierKeys.Control))
        {
            SearchBox.Focus();
            SearchBox.SelectAll();
            e.Handled = true;
            return;
        }

        if (e.Key == Key.F6)
        {
            FocusNextRegion();
            e.Handled = true;
            return;
        }

        if (!FleetGrid.IsKeyboardFocusWithin ||
            FleetGrid.SelectedItem is not DeviceRowViewModel selected)
        {
            return;
        }

        if (e.Key == Key.Space)
        {
            _viewModel.ToggleBatchSelection(selected);
            e.Handled = true;
        }
        else if (e.Key == Key.Enter)
        {
            await _viewModel.SelectDeviceAsync(selected);
            InspectorPane.Focus();
            e.Handled = true;
        }
    }

    private void FocusNextRegion()
    {
        FrameworkElement[] regions =
            [EndpointBox, SearchBox, SavedViewBox, FleetGrid, InspectorPane];
        _focusRegion = (_focusRegion + 1) % regions.Length;
        regions[_focusRegion].Focus();
    }
}
