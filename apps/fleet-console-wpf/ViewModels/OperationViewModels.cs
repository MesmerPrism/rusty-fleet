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
