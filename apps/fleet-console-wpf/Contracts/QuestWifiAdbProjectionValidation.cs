// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Text.Json;

namespace RustyFleet.FleetConsole.Contracts;

public static class QuestWifiAdbProjectionValidation
{
    private static readonly IReadOnlySet<string> Lifecycles = new HashSet<string>(
        [
            "proposed", "accepted", "rejected", "dispatched", "running",
            "applied", "failed", "expired", "cancellation_requested",
            "cancelled", "cleanup_pending", "cleaned"
        ],
        StringComparer.Ordinal);

    public static void ValidateOperation(QuestWifiAdbOperation operation)
    {
        Require(
            operation.Schema == "rusty.fleet.quest_wifi_adb_operation.v1" &&
            IsPortableIdentifier(operation.OperationId, 256) &&
            operation.ActionId == QuestWifiAdbActions.ActionId &&
            Lifecycles.Contains(operation.Lifecycle),
            "Quest Wi-Fi ADB operation header");
        var preview = operation.Preview;
        Require(
            preview.Schema == "rusty.fleet.quest_wifi_adb_preview.v1" &&
            IsPortableIdentifier(preview.PreviewId, 256) &&
            preview.OperationId == operation.OperationId &&
            preview.ActionId == operation.ActionId &&
            QuestWifiAdbActions.All.Contains(preview.Action) &&
            preview.CreatedAtMs >= 0 &&
            preview.ExpiresAtMs > preview.CreatedAtMs &&
            preview.FleetRevision > 0 &&
            operation.UpdatedAtMs >= preview.CreatedAtMs,
            "Quest Wi-Fi ADB immutable preview");
        ValidateOwner(preview.Owner);
        Require(
            preview.Targets.Count is >= 1 and <= 10_000 &&
            operation.Targets.Count == preview.Targets.Count,
            "Quest Wi-Fi ADB target count");

        var preflights = new Dictionary<string, QuestWifiAdbTargetPreflight>(
            StringComparer.Ordinal);
        foreach (var preflight in preview.Targets)
        {
            ValidatePreflight(preflight, preview.CreatedAtMs);
            Require(
                preflights.TryAdd(preflight.DeviceId, preflight),
                "Quest Wi-Fi ADB duplicate preview target");
        }

        var ledgers = new HashSet<string>(StringComparer.Ordinal);
        foreach (var target in operation.Targets)
        {
            Require(
                ledgers.Add(target.DeviceId) &&
                preflights.TryGetValue(target.DeviceId, out var preflight) &&
                target.IdentityRevision == preflight.IdentityRevision &&
                JsonSerializer.Serialize(target.Preflight, FleetJson.Options) ==
                JsonSerializer.Serialize(preflight, FleetJson.Options) &&
                Lifecycles.Contains(target.Lifecycle) &&
                target.UpdatedAtMs >= preview.CreatedAtMs &&
                (target.FailureCode is null ||
                 IsPortableIdentifier(target.FailureCode, 256)),
                "Quest Wi-Fi ADB target binding");
            if (target.Invocation is not null)
            {
                ValidateInvocation(target.Invocation, operation, target);
            }
            if (target.Receipt is not null)
            {
                Require(target.Invocation is not null, "Quest Wi-Fi ADB receipt invocation");
                ValidateReceipt(target.Receipt, operation, target);
            }
            Require(
                target.Lifecycle != "applied" ||
                target.Receipt?.EffectApplied == true,
                "Quest Wi-Fi ADB applied target receipt");
            if (target.TermuxProof is not null)
            {
                ValidateTermuxProof(target.TermuxProof, operation, target);
            }
            Require(
                !target.TermuxUsable ||
                target.TermuxProof is
                {
                    Available: true,
                    ListenerDiscovered: true,
                    ShellIdentity: "uid=2000(shell)"
                } proof &&
                proof.FreshUntilMs > operation.UpdatedAtMs &&
                target.Receipt is
                {
                    EffectApplied: true,
                    RouteMode: "modern_tls"
                } receipt &&
                proof.ObservedAtMs >= receipt.ObservedAtMs,
                "Quest Wi-Fi ADB Termux usability proof");
        }
    }

    private static void ValidateOwner(QuestWifiAdbOwnerBinding owner) =>
        Require(
            owner.OwnerRepoId == "questionable-file-manager" &&
            owner.CapabilityId ==
            "questionable-file-manager.quest-wifi-adb-provider" &&
            owner.ProviderContract ==
            "questionable.file_manager.fleet_connectivity_provider.v1" &&
            owner.ReceiptSchema ==
            "questionable.file_manager.quest_wifi_adb_receipt.v1" &&
            owner.Transport == "pinned_local_subprocess" &&
            owner.PrivateTargetResolution ==
            "provider_owned_credential_profile",
            "Quest Wi-Fi ADB owner binding");

    private static void ValidatePreflight(
        QuestWifiAdbTargetPreflight target,
        long previewCreatedAtMs)
    {
        Require(
            IsPortableIdentifier(target.DeviceId, 256) &&
            target.IdentityRevision > 0 &&
            target.CapabilityId ==
            "questionable-file-manager.quest-wifi-adb-provider" &&
            target.CapabilityEvidenceRevision > 0 &&
            target.CapabilityOwner == "questionable-file-manager",
            "Quest Wi-Fi ADB preflight identity and owner");
        Require(
            target.Support is "supported" or "unsupported" or "unknown" &&
            target.Enablement is "enabled" or "disabled" or "unknown" &&
            target.Authorization is
                "authorized" or "unauthorized" or "restricted" or "unknown" &&
            target.Reachability is
                "reachable" or "disconnected" or "unavailable" or "unknown" &&
            target.Freshness is "current" or "stale" or "unknown" &&
            target.FreshUntilMs >= target.ObservedAtMs &&
            target.EvaluatedAtMs >= target.ObservedAtMs &&
            target.EvaluatedAtMs == previewCreatedAtMs,
            "Quest Wi-Fi ADB preflight facts");
        var expectedReason = ExpectedPreflightReason(target);
        Require(
            target.ReasonCode == expectedReason &&
            target.Eligible == (expectedReason == "ready") &&
            IsBoundedText(target.Message, 1_024),
            "Quest Wi-Fi ADB preflight decision");
    }

    private static string ExpectedPreflightReason(
        QuestWifiAdbTargetPreflight target)
    {
        if (target.Support == "unsupported")
        {
            return "unsupported";
        }
        if (target.Support == "unknown")
        {
            return "support_unknown";
        }
        if (target.Enablement == "disabled")
        {
            return "disabled";
        }
        if (target.Enablement == "unknown")
        {
            return "enablement_unknown";
        }
        if (target.Authorization != "authorized")
        {
            return target.Authorization switch
            {
                "unauthorized" => "unauthorized",
                "restricted" => "restricted",
                _ => "authorization_unknown"
            };
        }
        if (target.Reachability != "reachable")
        {
            return target.Reachability switch
            {
                "disconnected" => "disconnected",
                "unavailable" => "provider_unavailable",
                _ => "reachability_unknown"
            };
        }
        if (target.Freshness == "current" &&
            target.EvaluatedAtMs <= target.FreshUntilMs)
        {
            return "ready";
        }

        return target.Freshness == "unknown"
            ? "freshness_unknown"
            : "stale";
    }

    private static void ValidateInvocation(
        QuestWifiAdbOwnerInvocation invocation,
        QuestWifiAdbOperation operation,
        QuestWifiAdbTargetLedger target) =>
        Require(
            invocation.Schema ==
            "rusty.fleet.quest_wifi_adb_owner_invocation.v1" &&
            IsPortableIdentifier(invocation.RequestId, 256) &&
            invocation.OperationId == operation.OperationId &&
            invocation.PreviewId == operation.Preview.PreviewId &&
            invocation.DeviceId == target.DeviceId &&
            invocation.IdentityRevision == target.IdentityRevision &&
            invocation.Action == operation.Preview.Action &&
            invocation.IssuedAtMs >= operation.Preview.CreatedAtMs &&
            invocation.ExpiresAtMs > invocation.IssuedAtMs,
            "Quest Wi-Fi ADB owner invocation binding");

    private static void ValidateReceipt(
        QuestWifiAdbOwnerReceipt receipt,
        QuestWifiAdbOperation operation,
        QuestWifiAdbTargetLedger target)
    {
        var invocation = target.Invocation!;
        Require(
            receipt.Schema ==
            "questionable.file_manager.quest_wifi_adb_receipt.v1" &&
            receipt.RequestId == invocation.RequestId &&
            receipt.OperationId == operation.OperationId &&
            receipt.PreviewId == operation.Preview.PreviewId &&
            receipt.DeviceId == target.DeviceId &&
            receipt.IdentityRevision == target.IdentityRevision &&
            receipt.Action == operation.Preview.Action &&
            receipt.ObservedAtMs >= invocation.IssuedAtMs &&
            IsBoundedText(receipt.Outcome, 256) &&
            IsLowerHexSha256(receipt.EvidenceSha256),
            "Quest Wi-Fi ADB receipt binding");
        Require(
            receipt.RouteMode is "none" or "modern_tls" or "classic_tcpip" &&
            receipt.WearerApproval is
                "not_applicable" or "pending" or "rejected" or "unknown",
            "Quest Wi-Fi ADB route and wearer facts");
        if (receipt.Action == QuestWifiAdbActions.EnableClassicTcpipFromUsb)
        {
            Require(
                receipt.RouteMode == "classic_tcpip" &&
                !receipt.KioskSettingApplied &&
                receipt.RequestAfterBootEnabled is null &&
                receipt.WearerApproval == "not_applicable",
                "classic USB tcpip separation");
        }
        else
        {
            Require(
                receipt.RouteMode != "classic_tcpip",
                "modern Wireless ADB route separation");
        }
        if (receipt.Action == QuestWifiAdbActions.RequestWirelessAdb)
        {
            Require(
                receipt.RouteMode == "modern_tls",
                "modern Wireless ADB route");
        }
        Require(
            receipt.EffectApplied == DerivedEffectApplied(receipt),
            "Quest Wi-Fi ADB independent effect readbacks");
    }

    private static bool DerivedEffectApplied(QuestWifiAdbOwnerReceipt receipt) =>
        receipt.Action switch
        {
            QuestWifiAdbActions.Status => receipt.RequestDelivered,
            QuestWifiAdbActions.RequestWirelessAdb =>
                receipt.RequestDelivered &&
                receipt.KioskSettingApplied &&
                receipt.WearerApproval == "pending",
            QuestWifiAdbActions.EnableRequestAfterBoot =>
                receipt.RequestDelivered &&
                receipt.KioskSettingApplied &&
                receipt.RequestAfterBootEnabled == true,
            QuestWifiAdbActions.DisableRequestAfterBoot =>
                receipt.RequestDelivered &&
                receipt.KioskSettingApplied &&
                receipt.RequestAfterBootEnabled == false,
            QuestWifiAdbActions.DisableWirelessAdb =>
                receipt.RequestDelivered && receipt.KioskSettingApplied,
            QuestWifiAdbActions.EnableClassicTcpipFromUsb =>
                receipt.RequestDelivered &&
                !receipt.KioskSettingApplied &&
                receipt.RouteMode == "classic_tcpip" &&
                receipt.ListenerDiscovered,
            _ => false
        };

    private static void ValidateTermuxProof(
        QuestWifiAdbTermuxProof proof,
        QuestWifiAdbOperation operation,
        QuestWifiAdbTargetLedger target)
    {
        Require(
            proof.Schema == "rusty.fleet.quest_wifi_adb_termux_proof.v1" &&
            proof.OwnerId == "quest-termux-lab.loopback-proof" &&
            IsPortableIdentifier(proof.ProofId, 256) &&
            proof.DeviceId == target.DeviceId &&
            proof.IdentityRevision == target.IdentityRevision &&
            IsPortableIdentifier(proof.SourceEpoch, 256) &&
            proof.SourceRevision > 0 &&
            proof.RouteMode == "modern_tls" &&
            proof.DiscoveryMode is "tls_nsd" or "tls_mdns" &&
            proof.ObservedAtMs >= 0 &&
            proof.FreshUntilMs > proof.ObservedAtMs &&
            proof.FreshUntilMs - proof.ObservedAtMs <= 60_000 &&
            IsLowerHexSha256(proof.EvidenceSha256) &&
            proof.Available ==
            (proof.ListenerDiscovered &&
             proof.ShellIdentity == "uid=2000(shell)") &&
            (proof.Available ||
             (!proof.ListenerDiscovered && proof.ShellIdentity is null)) &&
            target.Receipt?.RouteMode == proof.RouteMode,
            "signed Termux loopback proof");
    }

    private static bool IsPortableIdentifier(string value, int maximumLength) =>
        value.Length is > 0 &&
        value.Length <= maximumLength &&
        value.All(character =>
            char.IsAsciiLetterOrDigit(character) || character is '.' or '_' or '-');

    private static bool IsBoundedText(string value, int maximumLength) =>
        !string.IsNullOrWhiteSpace(value) && value.Length <= maximumLength;

    private static bool IsLowerHexSha256(string value) =>
        value.Length == 64 &&
        value.All(character =>
            char.IsAsciiDigit(character) || character is >= 'a' and <= 'f');

    private static void Require(bool condition, string field)
    {
        if (!condition)
        {
            throw new InvalidOperationException(
                $"Fleet Hub returned invalid projection evidence: {field}.");
        }
    }
}
