// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Globalization;

namespace RustyFleet.FleetConsole.Contracts;

public static class WindowsHotspotProjectionValidation
{
    private static readonly IReadOnlySet<string> Lifecycles = new HashSet<string>(
        [
            "proposed", "accepted", "rejected", "dispatched",
            "applied", "failed", "expired"
        ],
        StringComparer.Ordinal);

    private static readonly IReadOnlySet<string> Outcomes = new HashSet<string>(
        ["verified", "failed", "rejected", "unavailable"],
        StringComparer.Ordinal);

    public static void ValidateOperation(WindowsHotspotOperation operation)
    {
        Require(
            operation.Schema == "rusty.fleet.windows_hotspot_operation.v1" &&
            IsPortableIdentifier(operation.OperationId, 128) &&
            operation.ActionId == WindowsHotspotActions.ActionId &&
            Lifecycles.Contains(operation.Lifecycle),
            "Windows hotspot operation header");

        var preview = operation.Preview;
        Require(
            preview.Schema == "rusty.fleet.windows_hotspot_preview.v1" &&
            IsPortableIdentifier(preview.PreviewId, 128) &&
            preview.OperationId == operation.OperationId &&
            preview.ActionId == operation.ActionId &&
            preview.ResourceId == WindowsHotspotActions.ResourceId &&
            preview.OwnerId == WindowsHotspotActions.OwnerId &&
            WindowsHotspotActions.All.Contains(preview.Action) &&
            preview.CreatedAtMs >= 0 &&
            preview.ExpiresAtMs > preview.CreatedAtMs &&
            preview.ExpiresAtMs - preview.CreatedAtMs <= 5 * 60_000 &&
            preview.FleetRevision > 0 &&
            operation.UpdatedAtMs >= preview.CreatedAtMs,
            "Windows hotspot immutable preview");

        ValidatePreflight(preview);
        ValidateLease(operation);

        if (operation.ConfirmedAtMs is not null)
        {
            Require(
                operation.ConfirmedAtMs >= preview.CreatedAtMs &&
                operation.ConfirmedAtMs <= preview.ExpiresAtMs,
                "Windows hotspot confirmation time");
        }

        if (operation.Invocation is not null)
        {
            Require(
                operation.ConfirmedAtMs is not null,
                "Windows hotspot invocation confirmation");
            ValidateInvocation(operation.Invocation, operation);
        }

        if (operation.Receipt is not null)
        {
            Require(
                operation.Invocation is not null,
                "Windows hotspot receipt invocation");
            ValidateReceipt(operation.Receipt, operation);
        }

        ValidateLifecycle(operation);
    }

    private static void ValidatePreflight(WindowsHotspotPreview preview)
    {
        var preflight = preview.Preflight;
        Require(
            preflight.Ownership is "none" or "fleet" or "external" &&
            (preflight.OwnershipGeneration is null ||
             IsPortableIdentifier(preflight.OwnershipGeneration, 128)) &&
            IsBoundedText(preflight.Message, 1_024),
            "Windows hotspot preflight facts");

        var observationConsistent = preflight.Ownership switch
        {
            "none" => !preflight.Active &&
                      preflight.OwnershipGeneration is null,
            "fleet" => preflight.Active &&
                       preflight.OwnershipGeneration is not null,
            "external" => preflight.Active &&
                          preflight.OwnershipGeneration is null,
            _ => false
        };
        Require(observationConsistent, "Windows hotspot ownership observation");

        var actionEligible = preview.Action switch
        {
            WindowsHotspotActions.Status => true,
            WindowsHotspotActions.Start or WindowsHotspotActions.Ensure =>
                preflight.Ownership != "external",
            WindowsHotspotActions.Stop =>
                preflight.Active &&
                preflight.Ownership == "fleet" &&
                preflight.OwnershipGeneration is not null,
            _ => false
        };
        var eligible =
            preflight.ProviderReady &&
            preflight.LeaseAvailable &&
            actionEligible;
        var reason = !preflight.ProviderReady
            ? "provider_unavailable"
            : !preflight.LeaseAvailable
                ? "resource_leased"
                : preflight.Ownership == "external"
                    ? "external_hotspot_not_owned"
                    : preview.Action == WindowsHotspotActions.Stop &&
                      !actionEligible
                        ? "fleet_ownership_required"
                        : "ready";
        Require(
            preflight.Eligible == eligible &&
            preflight.ReasonCode == reason,
            "Windows hotspot preflight decision");
    }

    private static void ValidateLease(WindowsHotspotOperation operation)
    {
        var lease = operation.Lease;
        Require(
            lease is not null &&
            lease.Schema == "rusty.fleet.host_resource_lease.v1" &&
            IsPortableIdentifier(lease.LeaseId, 128) &&
            lease.ResourceId == WindowsHotspotActions.ResourceId &&
            lease.HolderOperationId == operation.OperationId &&
            IsPortableIdentifier(lease.Generation, 128) &&
            lease.IssuedAtMs == operation.Preview.CreatedAtMs &&
            lease.ExpiresAtMs == operation.Preview.ExpiresAtMs,
            "Windows hotspot singleton lease");
    }

    private static void ValidateInvocation(
        WindowsHotspotProviderRequest invocation,
        WindowsHotspotOperation operation)
    {
        Require(
            invocation.Schema ==
            "rusty.hostess.windows_hotspot.provider_request.v1" &&
            IsPortableIdentifier(invocation.RequestId, 128) &&
            invocation.OperationId == operation.OperationId &&
            invocation.Action == operation.Preview.Action &&
            invocation.TimeoutMs == 30_000 &&
            TryReadUtc(invocation.ExpiresAtUtc, out var expiresAt) &&
            expiresAt.ToUnixTimeMilliseconds() ==
            operation.Preview.ExpiresAtMs,
            "Windows hotspot Hostess invocation");

        var generationValid = invocation.Action switch
        {
            WindowsHotspotActions.Status or WindowsHotspotActions.Start =>
                invocation.OwnershipGeneration is null,
            WindowsHotspotActions.Ensure =>
                invocation.OwnershipGeneration is null ||
                invocation.OwnershipGeneration ==
                operation.Preview.Preflight.OwnershipGeneration,
            WindowsHotspotActions.Stop =>
                invocation.OwnershipGeneration is not null &&
                invocation.OwnershipGeneration ==
                operation.Preview.Preflight.OwnershipGeneration,
            _ => false
        };
        Require(generationValid, "Windows hotspot invocation ownership generation");
    }

    private static void ValidateReceipt(
        WindowsHotspotProviderReceipt receipt,
        WindowsHotspotOperation operation)
    {
        var invocation = operation.Invocation!;
        Require(
            receipt.Schema ==
            "rusty.hostess.windows_hotspot.provider_receipt.v1" &&
            receipt.RequestId == invocation.RequestId &&
            receipt.OperationId == operation.OperationId &&
            receipt.Action == operation.Preview.Action &&
            Outcomes.Contains(receipt.Outcome) &&
            IsBoundedText(receipt.Reason, 1_024) &&
            TryReadUtc(receipt.ObservedAtUtc, out _) &&
            IsBoundedText(receipt.Capability, 256) &&
            IsBoundedText(receipt.OperationalState, 256) &&
            IsBoundedText(receipt.Band, 256) &&
            IsBoundedText(receipt.SourceConnectivity, 256) &&
            (receipt.MaxClientCount == 0 ||
             receipt.ClientCount <= receipt.MaxClientCount) &&
            (receipt.OwnershipGeneration is null ||
             IsPortableIdentifier(receipt.OwnershipGeneration, 128)),
            "Windows hotspot Hostess receipt");

        if (receipt.Outcome != "verified")
        {
            return;
        }

        var effectiveReadback = receipt.Action switch
        {
            WindowsHotspotActions.Status => true,
            WindowsHotspotActions.Start =>
                receipt.OperationalState == "On" &&
                receipt.OwnershipGeneration is not null,
            WindowsHotspotActions.Ensure =>
                receipt.OperationalState == "On" &&
                receipt.OwnershipGeneration is not null &&
                (invocation.OwnershipGeneration is null ||
                 receipt.OwnershipGeneration ==
                 invocation.OwnershipGeneration),
            WindowsHotspotActions.Stop =>
                receipt.OperationalState == "Off" &&
                receipt.OwnershipGeneration is null &&
                invocation.OwnershipGeneration is not null,
            _ => false
        };
        Require(effectiveReadback, "Windows hotspot effective readback");
    }

    private static void ValidateLifecycle(WindowsHotspotOperation operation)
    {
        var preview = operation.Preview;
        switch (operation.Lifecycle)
        {
            case "proposed":
                Require(
                    preview.Preflight.Eligible &&
                    operation.ConfirmedAtMs is null &&
                    operation.Invocation is null &&
                    operation.Receipt is null &&
                    operation.FailureCode is null,
                    "Windows hotspot proposed state");
                break;
            case "rejected":
                Require(
                    !preview.Preflight.Eligible &&
                    operation.ConfirmedAtMs is null &&
                    operation.Invocation is null &&
                    operation.Receipt is null &&
                    operation.FailureCode == preview.Preflight.ReasonCode,
                    "Windows hotspot rejected state");
                break;
            case "accepted":
                Require(
                    preview.Preflight.Eligible &&
                    operation.ConfirmedAtMs is not null &&
                    operation.Invocation is null &&
                    operation.Receipt is null &&
                    operation.FailureCode is null,
                    "Windows hotspot accepted state");
                break;
            case "dispatched":
                Require(
                    preview.Preflight.Eligible &&
                    operation.ConfirmedAtMs is not null &&
                    operation.Invocation is not null &&
                    operation.Receipt is null &&
                    operation.FailureCode is null,
                    "Windows hotspot dispatched state");
                break;
            case "applied":
                Require(
                    preview.Preflight.Eligible &&
                    operation.ConfirmedAtMs is not null &&
                    operation.Invocation is not null &&
                    operation.Receipt?.Outcome == "verified" &&
                    operation.FailureCode is null,
                    "Windows hotspot applied state");
                break;
            case "failed":
                Require(
                    preview.Preflight.Eligible &&
                    operation.ConfirmedAtMs is not null &&
                    operation.Invocation is not null &&
                    operation.FailureCode is not null &&
                    IsPortableIdentifier(operation.FailureCode, 256) &&
                    (operation.Receipt is null ||
                     operation.Receipt.Outcome != "verified") &&
                    (operation.Receipt is null ||
                     operation.FailureCode == operation.Receipt.Reason),
                    "Windows hotspot failed state");
                break;
            case "expired":
                Require(
                    operation.Receipt is null &&
                    operation.FailureCode == "lease_expired",
                    "Windows hotspot expired state");
                break;
        }
    }

    private static bool TryReadUtc(
        string value,
        out DateTimeOffset timestamp) =>
        DateTimeOffset.TryParse(
            value,
            CultureInfo.InvariantCulture,
            DateTimeStyles.AssumeUniversal |
            DateTimeStyles.AdjustToUniversal,
            out timestamp) &&
        timestamp.Offset == TimeSpan.Zero &&
        (value.EndsWith('Z') ||
         value.EndsWith("+00:00", StringComparison.Ordinal));

    private static bool IsPortableIdentifier(string value, int maximumLength) =>
        value.Length is > 0 &&
        value.Length <= maximumLength &&
        value.All(character =>
            char.IsAsciiLetterOrDigit(character) ||
            character is '.' or '_' or '-');

    private static bool IsBoundedText(string value, int maximumLength) =>
        !string.IsNullOrWhiteSpace(value) &&
        value.Length <= maximumLength &&
        !value.Contains('\r') &&
        !value.Contains('\n');

    private static void Require(bool condition, string field)
    {
        if (!condition)
        {
            throw new InvalidOperationException(
                $"Fleet Hub returned invalid projection evidence: {field}.");
        }
    }
}
