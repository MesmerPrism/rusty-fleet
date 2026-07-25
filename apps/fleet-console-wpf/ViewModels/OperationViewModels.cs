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
