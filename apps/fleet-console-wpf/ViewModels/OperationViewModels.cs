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
        OwnerDelivery = projection.Stage == "dispatch_ready"
            ? "Prepared only · owner ingress unavailable"
            : "Not prepared";
        AccessibleName =
            $"Device {DeviceId}, identity revision {IdentityRevision}, " +
            $"eligibility {Eligibility}, lifecycle {Lifecycle}, stage {Stage}, " +
            $"reason {ReasonCode}, {Message}, {OwnerDelivery}. " +
            "No package dispatch or installation is claimed.";
    }

    public string DeviceId { get; }

    public ulong IdentityRevision { get; }

    public string Eligibility { get; }

    public string Lifecycle { get; }

    public string Stage { get; }

    public string ReasonCode { get; }

    public string Message { get; }

    public string OwnerDelivery { get; }

    public string AccessibleName { get; }
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
