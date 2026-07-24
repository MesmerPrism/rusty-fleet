// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
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
    }

    public DataGrid FleetDataGrid => FleetGrid;

    public Button ApplyOrderingButton => ApplyLiveOrderingButton;

    public FrameworkElement InspectorRegion => InspectorPane;

    private async void OnFleetSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (FleetGrid.SelectedItem is DeviceRowViewModel selected)
        {
            await _viewModel.SelectDeviceAsync(selected);
        }
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
        FrameworkElement[] regions = [EndpointBox, SearchBox, FleetGrid, InspectorPane];
        _focusRegion = (_focusRegion + 1) % regions.Length;
        regions[_focusRegion].Focus();
    }
}
