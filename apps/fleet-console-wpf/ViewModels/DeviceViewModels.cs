// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.ComponentModel;
using System.Runtime.CompilerServices;
using RustyFleet.FleetConsole.Contracts;

namespace RustyFleet.FleetConsole.ViewModels;

public abstract class ObservableObject : INotifyPropertyChanged
{
    public event PropertyChangedEventHandler? PropertyChanged;

    protected bool SetProperty<T>(
        ref T field,
        T value,
        [CallerMemberName] string? propertyName = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return false;
        }

        field = value;
        OnPropertyChanged(propertyName);
        return true;
    }

    protected void OnPropertyChanged([CallerMemberName] string? propertyName = null) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
}

public sealed class DeviceRowViewModel(DeviceRowProjection projection) : ObservableObject
{
    private static readonly HashSet<string> ExceptionalStates =
    [
        "stale",
        "unauthorized",
        "restricted",
        "disconnected",
        "unavailable",
        "degraded",
        "failed",
        "critical"
    ];

    private DeviceRowProjection _projection = projection;
    private bool _isBatchSelected;

    public DeviceRowProjection Projection => _projection;

    public string StableKey =>
        $"{_projection.Identity.DeviceId}@{_projection.Identity.IdentityRevision}";

    public string DeviceId => _projection.Identity.DeviceId;

    public string DisplayName => _projection.Identity.DisplayName;

    public string Model => _projection.Identity.Model;

    public string DeviceSummary => $"{DisplayName}\n{Model} · {DeviceId}";

    public string AttentionSummary
    {
        get
        {
            var count = _projection.Conditions.Values.Count(
                condition => ExceptionalStates.Contains(condition.State));
            return count == 0 ? "None" : $"{count} attention";
        }
    }

    public string AgeText => FormatAge(_projection.AgeMs);

    public string FreshnessText => $"{Title(_projection.Freshness)} · {AgeText}";

    public string RouteText => Title(_projection.Route);

    public string FreshnessGroup => Title(_projection.Freshness);

    public string PowerText => _projection.BatteryPercent is int battery
        ? $"{battery}%{(_projection.Charging == true ? " · charging" : string.Empty)}"
        : "Unknown";

    public string AppKioskText
    {
        get
        {
            var app = _projection.Application?.PackageName ??
                      _projection.ForegroundApp ??
                      "Unknown app";
            return $"{app} · {Title(_projection.KioskState)}";
        }
    }

    public string ApplicationGroup =>
        _projection.Application?.PackageName ??
        _projection.ForegroundApp ??
        "Unknown app";

    public string ControlText =>
        $"Mon {DescribeCapability(FindCapability("monitoring", "capability.monitoring"))} · " +
        $"App {DescribeCapability(FindCapability(
            "participating_app_control",
            "capability.participating_app_control"))}";

    public string PrivilegedText
    {
        get
        {
            var privileged = _projection.Capabilities.Capabilities
                .Where(entry =>
                    entry.Key.Contains("adb", StringComparison.OrdinalIgnoreCase) ||
                    entry.Key.Contains("file", StringComparison.OrdinalIgnoreCase))
                .Select(entry => $"{ShortCapability(entry.Key)} {DescribeCapability(entry.Value)}")
                .ToArray();
            return privileged.Length == 0 ? "Not reported" : string.Join(" · ", privileged);
        }
    }

    public string StreamsText => _projection.StreamCount == 0
        ? "None"
        : $"{_projection.StreamCount} available";

    public string WorkText => _projection.ActiveWorkCount == 0
        ? "None"
        : $"{_projection.ActiveWorkCount} active";

    public string TagsText => _projection.Identity.Tags.TryGetValue("cohort", out var cohort)
        ? cohort
        : string.Join(", ", _projection.Identity.Tags.Select(entry => $"{entry.Key}={entry.Value}"));

    public string CohortGroup => _projection.Identity.Tags.TryGetValue("cohort", out var cohort)
        ? cohort
        : "No cohort";

    public string AccessibleName =>
        $"{DisplayName}, {Model}, {FreshnessText}, route {RouteText}, power {PowerText}, " +
        $"application {AppKioskText}, control {ControlText}, privileged {PrivilegedText}";

    public string BatchSelectionName => $"Include {DisplayName} in batch selection";

    public bool IsBatchSelected
    {
        get => _isBatchSelected;
        set => SetProperty(ref _isBatchSelected, value);
    }

    public void Update(DeviceRowProjection projection)
    {
        _projection = projection;
        foreach (var property in new[]
        {
            nameof(Projection),
            nameof(DeviceId),
            nameof(DisplayName),
            nameof(Model),
            nameof(DeviceSummary),
            nameof(AttentionSummary),
            nameof(AgeText),
            nameof(FreshnessText),
            nameof(FreshnessGroup),
            nameof(RouteText),
            nameof(PowerText),
            nameof(AppKioskText),
            nameof(ApplicationGroup),
            nameof(ControlText),
            nameof(PrivilegedText),
            nameof(StreamsText),
            nameof(WorkText),
            nameof(TagsText),
            nameof(CohortGroup),
            nameof(AccessibleName),
            nameof(BatchSelectionName)
        })
        {
            OnPropertyChanged(property);
        }
    }

    private CapabilityState? FindCapability(params string[] names)
    {
        foreach (var name in names)
        {
            if (_projection.Capabilities.Capabilities.TryGetValue(name, out var capability))
            {
                return capability;
            }
        }

        return null;
    }

    private static string DescribeCapability(CapabilityState? capability)
    {
        if (capability is null)
        {
            return "Unknown";
        }

        if (capability.Support == "unsupported")
        {
            return "Unsupported";
        }

        if (capability.Enablement == "disabled")
        {
            return "Disabled";
        }

        if (capability.Authorization is "unauthorized" or "restricted")
        {
            return Title(capability.Authorization);
        }

        if (capability.Reachability != "reachable")
        {
            return Title(capability.Reachability);
        }

        return capability.Freshness == "current" ? "Ready" : Title(capability.Freshness);
    }

    private static string ShortCapability(string value)
    {
        if (value.Contains("wifi", StringComparison.OrdinalIgnoreCase))
        {
            return "Wi-Fi";
        }

        if (value.Contains("usb", StringComparison.OrdinalIgnoreCase))
        {
            return "USB";
        }

        return value.Contains("file", StringComparison.OrdinalIgnoreCase) ? "File" : "ADB";
    }

    internal static string Title(string value) =>
        string.Join(
            " ",
            value.Split('_', StringSplitOptions.RemoveEmptyEntries)
                .Select(part => char.ToUpperInvariant(part[0]) + part[1..]));

    internal static string FormatAge(long ageMs)
    {
        if (ageMs < 1_000)
        {
            return "now";
        }

        var age = TimeSpan.FromMilliseconds(ageMs);
        if (age.TotalMinutes < 1)
        {
            return $"{Math.Floor(age.TotalSeconds)}s";
        }

        if (age.TotalHours < 1)
        {
            return $"{Math.Floor(age.TotalMinutes)}m";
        }

        if (age.TotalDays < 1)
        {
            return $"{Math.Floor(age.TotalHours)}h";
        }

        return $"{Math.Floor(age.TotalDays)}d";
    }
}

public sealed record InspectorFact(string Label, string Value, string Detail);

public sealed class DeviceInspectorViewModel
{
    public DeviceInspectorViewModel(DeviceInspectorProjection projection)
    {
        var row = projection.Row;
        Title = row.Identity.DisplayName;
        Identity = $"{row.Identity.Model} · {row.Identity.DeviceId} · identity revision " +
                   row.Identity.IdentityRevision;
        Freshness = $"{DeviceRowViewModel.Title(row.Freshness)} · " +
                    $"{DeviceRowViewModel.FormatAge(row.AgeMs)} · accepted revision " +
                    row.AcceptedRevision;
        Attention = projection.Attention.Count == 0
            ? "No current actionable conditions"
            : string.Join(
                "; ",
                projection.Attention.Select(condition =>
                    $"{DeviceRowViewModel.Title(condition.State)}: {condition.Message}"));
        Facts =
        [
            new(
                "Power",
                row.BatteryPercent is int battery ? $"{battery}%" : "Unknown",
                row.Power is null
                    ? "No power provenance was reported."
                    : $"Owner {row.Power.Provenance.Owner}; observed " +
                      FormatInstant(row.Power.Provenance.ObservedAtMs)),
            new(
                "Application",
                row.Application?.PackageName ?? row.ForegroundApp ?? "Unknown",
                row.Application is null
                    ? "No participating-app observation was reported."
                    : $"{DeviceRowViewModel.Title(row.Application.ForegroundState)}; " +
                      $"authority {DeviceRowViewModel.Title(row.Application.ForegroundAuthority)}"),
            new(
                "Kiosk",
                DeviceRowViewModel.Title(row.KioskState),
                "Kiosk state remains independent from application foreground state."),
            new(
                "Route",
                DeviceRowViewModel.Title(row.Route),
                $"Source epoch {row.SourceEpoch}")
        ];
        Capabilities = row.Capabilities.Capabilities
            .OrderBy(entry => entry.Key, StringComparer.Ordinal)
            .Select(entry => new InspectorFact(
                entry.Key,
                CapabilityValue(entry.Value),
                $"Owner {entry.Value.Owner}; evidence revision {entry.Value.EvidenceRevision}; " +
                $"reason {entry.Value.Reason}"))
            .ToArray();
        Conditions = row.Conditions.Values
            .OrderBy(condition => condition.Family, StringComparer.Ordinal)
            .Select(condition => new InspectorFact(
                DeviceRowViewModel.Title(condition.Family),
                DeviceRowViewModel.Title(condition.State),
                $"{condition.Message}; owner {condition.Source.Owner}; authority revision " +
                condition.Source.AuthorityRevision))
            .ToArray();
        Work = projection.ActiveOperations.Count == 0
            ? "No active operations"
            : $"{projection.ActiveOperations.Count} active operations";
        Streams = projection.Streams.Count == 0
            ? "No selected streams"
            : $"{projection.Streams.Count} streams";
    }

    public string Title { get; }

    public string Identity { get; }

    public string Freshness { get; }

    public string Attention { get; }

    public IReadOnlyList<InspectorFact> Facts { get; }

    public IReadOnlyList<InspectorFact> Capabilities { get; }

    public IReadOnlyList<InspectorFact> Conditions { get; }

    public string Work { get; }

    public string Streams { get; }

    public static DeviceInspectorViewModel FromRow(DeviceRowProjection row) => new(
        new DeviceInspectorProjection
        {
            Schema = "rusty.fleet.device_inspector.v1",
            Row = row,
            Attention = row.Conditions.Values
                .Where(condition => condition.State is
                    "stale" or "unauthorized" or "restricted" or "degraded" or
                    "failed" or "critical")
                .ToArray()
        });

    private static string CapabilityValue(CapabilityState capability) =>
        $"{DeviceRowViewModel.Title(capability.Support)} · " +
        $"{DeviceRowViewModel.Title(capability.Enablement)} · " +
        $"{DeviceRowViewModel.Title(capability.Authorization)} · " +
        $"{DeviceRowViewModel.Title(capability.Reachability)} · " +
        DeviceRowViewModel.Title(capability.Freshness);

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
