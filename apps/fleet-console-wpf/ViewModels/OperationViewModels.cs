// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using RustyFleet.FleetConsole.Contracts;

namespace RustyFleet.FleetConsole.ViewModels;

public sealed class OperationTargetViewModel
{
    public OperationTargetViewModel(OperationTargetResult projection)
    {
        DeviceId = projection.DeviceId;
        IdentityRevision = projection.IdentityRevision;
        Eligibility = projection.Preflight.Eligible ? "Eligible" : "Excluded";
        Lifecycle = DeviceRowViewModel.Title(projection.Lifecycle);
        ReasonCode = projection.ReasonCode;
        Message = projection.Message;
        ReceiptId = projection.EffectiveReceipt?.ReceiptId;
        RetryDisposition = DeviceRowViewModel.Title(projection.RetryDisposition);
        CancelDisposition = DeviceRowViewModel.Title(projection.CancelDisposition);
        AccessibleName =
            $"Device {DeviceId}, identity revision {IdentityRevision}, " +
            $"eligibility {Eligibility}, lifecycle {Lifecycle}, " +
            $"reason {ReasonCode}, {Message}, retry {RetryDisposition}, " +
            $"cancellation {CancelDisposition}" +
            (ReceiptId is null ? string.Empty : $", receipt {ReceiptId}");
    }

    public string DeviceId { get; }

    public ulong IdentityRevision { get; }

    public string Eligibility { get; }

    public string Lifecycle { get; }

    public string ReasonCode { get; }

    public string Message { get; }

    public string? ReceiptId { get; }

    public string RetryDisposition { get; }

    public string CancelDisposition { get; }

    public string AccessibleName { get; }
}

public sealed class PackageOperationTargetViewModel
{
    public PackageOperationTargetViewModel(PackageInstallTargetLedger projection)
    {
        DeviceId = projection.DeviceId;
        IdentityRevision = projection.IdentityRevision;
        Eligibility = projection.Preflight.Eligible ? "Eligible" : "Excluded";
        Lifecycle = DeviceRowViewModel.Title(projection.Lifecycle);
        Stage = DeviceRowViewModel.Title(projection.Stage);
        ReasonCode = projection.ReasonCode;
        Message = projection.Message;
        OwnerDelivery = DescribeOwnerDelivery(projection);
        ApplicationProof = projection.EffectiveReceipt is null
            ? "No installed-version application proof is present."
            : "Exact installed-version application proof is present; cleanup is not claimed.";
        AccessibleName =
            $"Device {DeviceId}, identity revision {IdentityRevision}, " +
            $"eligibility {Eligibility}, lifecycle {Lifecycle}, stage {Stage}, " +
            $"reason {ReasonCode}, {Message}, {OwnerDelivery}. " +
            ApplicationProof;
    }

    public string DeviceId { get; }

    public ulong IdentityRevision { get; }

    public string Eligibility { get; }

    public string Lifecycle { get; }

    public string Stage { get; }

    public string ReasonCode { get; }

    public string Message { get; }

    public string OwnerDelivery { get; }

    public string ApplicationProof { get; }

    public string AccessibleName { get; }

    private static string DescribeOwnerDelivery(PackageInstallTargetLedger projection)
    {
        if (projection.EffectiveReceipt?.UpdaterReceipt.AcceptedCheckpoint is { } checkpoint)
        {
            return
                $"Installed version proven · install_commit accepted · " +
                $"version {checkpoint.VersionCode} · sequence {checkpoint.Sequence}";
        }

        if (projection.InvocationAcknowledgement is { } acknowledgement)
        {
            return acknowledgement.Accepted
                ? $"Owner acknowledged dispatch · {DeviceRowViewModel.Title(acknowledgement.Code)}"
                : $"Owner rejected dispatch · {DeviceRowViewModel.Title(acknowledgement.Code)}";
        }

        if (projection.OwnerClaim is { } claim)
        {
            var expiry = claim.ExpiresAtMs <= 253_402_300_799_999
                ? DateTimeOffset
                    .FromUnixTimeMilliseconds(claim.ExpiresAtMs)
                    .UtcDateTime
                    .ToString("yyyy-MM-dd HH:mm:ss 'UTC'")
                : $"Unix time {claim.ExpiresAtMs} ms";
            return $"Updater owner claim recorded · expires {expiry}";
        }

        if (projection.Invocation is not null)
        {
            var attempts = projection.PriorOwnerClaims.Count;
            return attempts == 0
                ? "Prepared · waiting for authenticated updater-owner claim"
                : $"Prepared · {attempts} prior owner claim attempt(s) retained · waiting for a new claim";
        }

        return "No owner invocation prepared";
    }
}

public sealed class QuestAwakeTargetViewModel
{
    public QuestAwakeTargetViewModel(QuestAwakeTargetLedger projection)
    {
        DeviceId = projection.DeviceId;
        IdentityRevision = projection.IdentityRevision;
        Eligibility = projection.Preflight.Eligible ? "Eligible" : "Excluded";
        Lifecycle = DeviceRowViewModel.Title(projection.Lifecycle);
        var receipt = projection.Receipt;
        PowerReadback = receipt is null
            ? "No power readback"
            : $"{DeviceRowViewModel.Title(receipt.Power.Wakefulness)} · " +
              $"display {DeviceRowViewModel.Title(receipt.Power.DisplayState)} · " +
              $"stay-on {YesNo(receipt.Power.StayOn)} · " +
              $"proximity {DeviceRowViewModel.Title(receipt.Power.ProximityState)}" +
              RemainingText(receipt.Power.ProximityHoldRemainingMs);
        WatchdogReadback = receipt is null
            ? "No watchdog readback"
            : $"Windows {ActiveStopped(receipt.WindowsWatchdogEffective)} · " +
              $"Quest {QuestWatchdogText(receipt)}";
        Result = receipt is null
            ? projection.FailureCode is null
                ? projection.Preflight.Message
                : $"Failed · {DeviceRowViewModel.Title(projection.FailureCode)}"
            : $"{(receipt.Effective ? "Effective" : "Not effective")} · " +
              $"{receipt.Outcome} · repairs {receipt.RepairCount:N0}" +
              SettingsText(receipt);
        AccessibleName =
            $"Device {DeviceId}, identity revision {IdentityRevision}, " +
            $"eligibility {Eligibility}, lifecycle {Lifecycle}. " +
            $"Power readback: {PowerReadback}. " +
            $"Watchdog readback: {WatchdogReadback}. Result: {Result}.";
    }

    public string DeviceId { get; }

    public ulong IdentityRevision { get; }

    public string Eligibility { get; }

    public string Lifecycle { get; }

    public string PowerReadback { get; }

    public string WatchdogReadback { get; }

    public string Result { get; }

    public string AccessibleName { get; }

    private static string YesNo(bool value) => value ? "yes" : "no";

    private static string ActiveStopped(bool value) => value ? "active" : "stopped";

    private static string RemainingText(uint? remainingMs) =>
        remainingMs is null
            ? string.Empty
            : $" · hold remaining {remainingMs.Value / 60_000d:0.#} min";

    private static string QuestWatchdogText(QuestAwakeOwnerReceipt receipt)
    {
        if (!receipt.DeviceWatchdogEffective)
        {
            return "stopped";
        }

        return receipt.DeviceWatchdog.Fresh
            ? "active and current · stops on reboot"
            : "active but stale · stops on reboot";
    }

    private static string SettingsText(QuestAwakeOwnerReceipt receipt)
    {
        if (receipt.SettingsRestored)
        {
            return " · normal settings restored";
        }

        return receipt.SettingsLeftUnchanged
            ? " · awake settings left unchanged"
            : string.Empty;
    }
}

public sealed class QuestWifiAdbTargetViewModel
{
    public QuestWifiAdbTargetViewModel(
        QuestWifiAdbTargetLedger projection,
        long operationUpdatedAtMs)
    {
        DeviceId = projection.DeviceId;
        IdentityRevision = projection.IdentityRevision;
        Eligibility = projection.Preflight.Eligible ? "Eligible" : "Excluded";
        Lifecycle = DeviceRowViewModel.Title(projection.Lifecycle);
        var receipt = projection.Receipt;
        Route = receipt is null
            ? "No route readback"
            : receipt.RouteMode switch
            {
                "modern_tls" => "Modern Wireless ADB (TLS)",
                "classic_tcpip" => "Classic USB tcpip route",
                _ => "No active route reported"
            };
        RequestDelivery = receipt is null
            ? "No delivery receipt"
            : receipt.RequestDelivered
                ? "Delivered"
                : "Not delivered";
        KioskSetting = receipt is null
            ? "No Kiosk readback"
            : receipt.RouteMode == "classic_tcpip"
                ? "Not applicable to classic USB tcpip"
                : receipt.KioskSettingApplied
                    ? "Applied"
                    : "Not applied";
        AfterBoot = receipt?.RequestAfterBootEnabled switch
        {
            true => "After-boot request enabled",
            false => "After-boot request disabled",
            _ => "After-boot state not reported"
        };
        WearerApproval = receipt is null
            ? "Unknown · no receipt"
            : receipt.WearerApproval switch
            {
                "pending" => "Pending · wearer must approve in headset",
                "rejected" => "Rejected by wearer",
                "not_applicable" => "Not applicable",
                _ => "Unknown · protected prompt is not automated"
            };
        Listener = receipt is null
            ? "No listener readback"
            : receipt.ListenerDiscovered
                ? $"{Route} listener observed"
                : $"{Route} listener not observed";
        Termux = TermuxText(
            projection.TermuxProof,
            projection.TermuxUsable,
            operationUpdatedAtMs);
        Result = receipt is null
            ? projection.FailureCode is null
                ? projection.Preflight.Message
                : $"Failed · {DeviceRowViewModel.Title(projection.FailureCode)}"
            : $"{(receipt.EffectApplied ? "Action applied" : "Not applied")} · " +
              receipt.Outcome;
        AccessibleName =
            $"Device {DeviceId}, identity revision {IdentityRevision}, " +
            $"eligibility {Eligibility}, lifecycle {Lifecycle}. Route: {Route}. " +
            $"Request delivery: {RequestDelivery}. Kiosk setting: {KioskSetting}. " +
            $"{AfterBoot}. Wearer approval: {WearerApproval}. Listener: {Listener}. " +
            $"Termux loopback: {Termux}. Result: {Result}.";
    }

    public string DeviceId { get; }

    public ulong IdentityRevision { get; }

    public string Eligibility { get; }

    public string Lifecycle { get; }

    public string Route { get; }

    public string RequestDelivery { get; }

    public string KioskSetting { get; }

    public string AfterBoot { get; }

    public string WearerApproval { get; }

    public string Listener { get; }

    public string Termux { get; }

    public string Result { get; }

    public string AccessibleName { get; }

    private static string TermuxText(
        QuestWifiAdbTermuxProof? proof,
        bool usable,
        long operationUpdatedAtMs)
    {
        if (proof is null)
        {
            return "Not proven · no admitted signed proof";
        }

        var freshness = proof.FreshUntilMs > operationUpdatedAtMs
            ? $"current until {FormatTime(proof.FreshUntilMs)}"
            : $"stale since {FormatTime(proof.FreshUntilMs)}";
        if (usable &&
            proof.Available &&
            proof.ShellIdentity == "uid=2000(shell)")
        {
            return $"Usable · enrolled signed proof · {freshness} · " +
                   "shell identity verified";
        }

        return $"Not usable · signed proof {freshness}";
    }

    private static string FormatTime(long unixMilliseconds)
    {
        try
        {
            return DateTimeOffset
                .FromUnixTimeMilliseconds(unixMilliseconds)
                .ToLocalTime()
                .ToString("yyyy-MM-dd HH:mm:ss zzz");
        }
        catch (ArgumentOutOfRangeException)
        {
            return "an invalid time";
        }
    }
}

public sealed class WindowsHotspotResultViewModel
{
    public WindowsHotspotResultViewModel(WindowsHotspotOperation operation)
    {
        var preflight = operation.Preview.Preflight;
        Provider = preflight.ProviderReady ? "Ready" : "Unavailable";
        Lease = preflight.LeaseAvailable ? "Available" : "In use";
        Activity = preflight.Active ? "On" : "Off";
        Ownership = preflight.Ownership switch
        {
            WindowsHotspotActions.OwnershipFleet => "Fleet-owned",
            WindowsHotspotActions.OwnershipExternal =>
                "External · observe only; Fleet will not adopt or stop it",
            WindowsHotspotActions.OwnershipNone => "None",
            _ => "Unknown"
        };
        Eligibility = preflight.Eligible
            ? "Eligible · ready for explicit confirmation"
            : PreflightReason(preflight.ReasonCode);
        Lifecycle = DeviceRowViewModel.Title(operation.Lifecycle);
        Expires = FormatUnixTime(operation.Preview.ExpiresAtMs);

        var receipt = operation.Receipt;
        Outcome = receipt is null
            ? operation.FailureCode is null
                ? "No result yet"
                : "Operation failed"
            : receipt.Outcome switch
            {
                WindowsHotspotActions.ResultVerified => "Verified",
                WindowsHotspotActions.ResultFailed => "Failed",
                WindowsHotspotActions.ResultRejected => "Rejected",
                WindowsHotspotActions.ResultUnavailable => "Unavailable",
                _ => "Unknown result"
            };
        Observed = receipt is null
            ? "No provider observation"
            : FormatUtcTime(receipt.ObservedAtUtc);
        Capability = receipt is null
            ? "No capability readback"
            : receipt.CapabilityAvailable
                ? "Available"
                : "Unavailable";
        OperationalState = receipt is null
            ? "No operational readback"
            : receipt.OperationalState switch
            {
                "On" => "On",
                "Off" => "Off",
                "InTransition" => "Changing",
                "Unknown" => "Unknown",
                _ => "Unknown"
            };
        Clients = receipt is null
            ? "No client readback"
            : receipt.MaxClientCount == 0
                ? $"{receipt.ClientCount:N0} connected · maximum not reported"
                : $"{receipt.ClientCount:N0} of {receipt.MaxClientCount:N0} connected";
        Band = receipt is null ? "Not reported" : SafeBand(receipt.Band);
        SourceConnectivity = receipt is null
            ? "Not reported"
            : SafeConnectivity(receipt.SourceConnectivity);
        AccessibleName =
            $"Windows host Mobile Hotspot. Provider {Provider}. Lease {Lease}. " +
            $"Hotspot {Activity}. Ownership {Ownership}. {Eligibility}. " +
            $"Lifecycle {Lifecycle}. Preview expires {Expires}. Outcome {Outcome}. " +
            $"Observed {Observed}. Capability {Capability}. " +
            $"Operational state {OperationalState}. Clients {Clients}. " +
            $"Band {Band}. Source connectivity {SourceConnectivity}.";
    }

    public string Provider { get; }

    public string Lease { get; }

    public string Activity { get; }

    public string Ownership { get; }

    public string Eligibility { get; }

    public string Lifecycle { get; }

    public string Expires { get; }

    public string Outcome { get; }

    public string Observed { get; }

    public string Capability { get; }

    public string OperationalState { get; }

    public string Clients { get; }

    public string Band { get; }

    public string SourceConnectivity { get; }

    public string AccessibleName { get; }

    private static string PreflightReason(string reason) =>
        reason switch
        {
            "provider_unavailable" => "Not eligible · provider unavailable",
            "resource_leased" => "Not eligible · another host operation holds the lease",
            "external_hotspot_not_owned" =>
                "Not eligible · external hotspot is observe only",
            "fleet_ownership_required" =>
                "Not eligible · stop requires exact Fleet ownership",
            "ready" => "Eligible · ready for explicit confirmation",
            _ => "Not eligible · reason unavailable"
        };

    private static string SafeBand(string value) =>
        value switch
        {
            "Auto" => "Automatic",
            "TwoPointFourGigahertz" => "2.4 GHz",
            "FiveGigahertz" => "5 GHz",
            "SixGigahertz" => "6 GHz",
            "Unknown" => "Unknown",
            _ => "Unknown"
        };

    private static string SafeConnectivity(string value) =>
        value switch
        {
            "Internet" => "Internet access",
            "InternetAccess" => "Internet access",
            "ConstrainedInternetAccess" => "Constrained internet access",
            "LocalAccess" => "Local access",
            "None" => "No connectivity",
            "Unknown" => "Unknown",
            _ => "Unknown"
        };

    private static string FormatUnixTime(long unixMilliseconds)
    {
        try
        {
            return DateTimeOffset
                .FromUnixTimeMilliseconds(unixMilliseconds)
                .ToLocalTime()
                .ToString("yyyy-MM-dd HH:mm:ss zzz");
        }
        catch (ArgumentOutOfRangeException)
        {
            return "invalid time";
        }
    }

    private static string FormatUtcTime(string value) =>
        DateTimeOffset.TryParse(value, out var timestamp) &&
        timestamp.Offset == TimeSpan.Zero
            ? timestamp.ToLocalTime().ToString("yyyy-MM-dd HH:mm:ss zzz")
            : "invalid time";
}
