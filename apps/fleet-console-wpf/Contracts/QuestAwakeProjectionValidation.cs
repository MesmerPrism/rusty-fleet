// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Text.Json;

namespace RustyFleet.FleetConsole.Contracts;

public static class QuestAwakeProjectionValidation
{
    private static readonly IReadOnlySet<string> Lifecycles = new HashSet<string>(
        [
            "proposed",
            "accepted",
            "rejected",
            "dispatched",
            "running",
            "applied",
            "failed",
            "expired",
            "cancellation_requested",
            "cancelled",
            "cleanup_pending",
            "cleaned"
        ],
        StringComparer.Ordinal);

    public static void ValidateOperation(QuestAwakeOperation operation)
    {
        Require(
            operation.Schema == "rusty.fleet.quest_awake_operation.v1" &&
            IsPortableIdentifier(operation.OperationId, 256) &&
            operation.ActionId == QuestAwakeActions.ActionId &&
            Lifecycles.Contains(operation.Lifecycle),
            "Quest awake operation header");

        var preview = operation.Preview;
        Require(
            preview.Schema == "rusty.fleet.quest_awake_preview.v1" &&
            IsPortableIdentifier(preview.PreviewId, 256) &&
            preview.OperationId == operation.OperationId &&
            preview.ActionId == operation.ActionId &&
            QuestAwakeActions.All.Contains(preview.Action) &&
            preview.CreatedAtMs >= 0 &&
            preview.ExpiresAtMs > preview.CreatedAtMs &&
            preview.FleetRevision > 0 &&
            IsPortableIdentifier(preview.WatchdogGeneration, 256) &&
            operation.UpdatedAtMs >= preview.CreatedAtMs,
            "Quest awake immutable preview");
        ValidatePolicy(preview.DurationMs, preview.WatchdogIntervalMs);
        ValidateOwner(preview.Owner);
        Require(
            preview.Targets.Count is >= 1 and <= 10_000 &&
            operation.Targets.Count == preview.Targets.Count,
            "Quest awake target count");

        var preflights = new Dictionary<string, QuestAwakeTargetPreflight>(
            StringComparer.Ordinal);
        foreach (var preflight in preview.Targets)
        {
            ValidatePreflight(preflight, preview.CreatedAtMs);
            Require(
                preflights.TryAdd(preflight.DeviceId, preflight),
                "Quest awake duplicate preview target");
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
                "Quest awake target binding");
            if (target.Invocation is not null)
            {
                ValidateInvocation(target.Invocation, operation, target);
            }

            if (target.Receipt is not null)
            {
                Require(target.Invocation is not null, "Quest awake receipt invocation");
                ValidateReceipt(target.Receipt, operation, target);
            }
        }
    }

    private static void ValidateOwner(QuestAwakeOwnerBinding owner) =>
        Require(
            owner.OwnerRepoId == "questionable-file-manager" &&
            owner.CapabilityId ==
            "questionable-file-manager.quest-awake-provider" &&
            owner.ProviderContract ==
            "questionable.file_manager.fleet_awake_provider.v1" &&
            owner.ReceiptSchema ==
            "questionable.file_manager.quest_awake_receipt.v1" &&
            owner.Transport == "pinned_local_subprocess" &&
            owner.ApplicationProof ==
            "fresh_effective_power_and_watchdog_readback",
            "Quest awake owner binding");

    private static void ValidatePreflight(
        QuestAwakeTargetPreflight target,
        long previewCreatedAtMs)
    {
        Require(
            IsPortableIdentifier(target.DeviceId, 256) &&
            target.IdentityRevision > 0 &&
            target.CapabilityId ==
            "questionable-file-manager.quest-awake-provider" &&
            target.CapabilityEvidenceRevision > 0 &&
            target.CapabilityOwner == "questionable-file-manager",
            "Quest awake preflight identity and owner");
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
            "Quest awake preflight facts");
        var expectedReason = ExpectedPreflightReason(target);
        Require(
            target.ReasonCode == expectedReason &&
            target.Eligible == (expectedReason == "ready") &&
            IsBoundedText(target.Message, 1_024),
            "Quest awake preflight decision");
    }

    private static string ExpectedPreflightReason(QuestAwakeTargetPreflight target)
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
        QuestAwakeOwnerInvocation invocation,
        QuestAwakeOperation operation,
        QuestAwakeTargetLedger target)
    {
        Require(
            invocation.Schema ==
            "rusty.fleet.quest_awake_owner_invocation.v1" &&
            IsPortableIdentifier(invocation.RequestId, 256) &&
            invocation.OperationId == operation.OperationId &&
            invocation.PreviewId == operation.Preview.PreviewId &&
            invocation.DeviceId == target.DeviceId &&
            invocation.IdentityRevision == target.IdentityRevision &&
            invocation.Action == operation.Preview.Action &&
            invocation.DurationMs == operation.Preview.DurationMs &&
            invocation.WatchdogIntervalMs ==
            operation.Preview.WatchdogIntervalMs &&
            IsPortableIdentifier(invocation.WatchdogGeneration, 256) &&
            (IsStopOrRestore(operation.Preview.Action) ||
             invocation.WatchdogGeneration ==
             operation.Preview.WatchdogGeneration) &&
            invocation.IssuedAtMs >= operation.Preview.CreatedAtMs &&
            invocation.ExpiresAtMs > invocation.IssuedAtMs,
            "Quest awake owner invocation binding");
        ValidatePolicy(invocation.DurationMs, invocation.WatchdogIntervalMs);
    }

    private static void ValidateReceipt(
        QuestAwakeOwnerReceipt receipt,
        QuestAwakeOperation operation,
        QuestAwakeTargetLedger target)
    {
        var invocation = target.Invocation!;
        Require(
            receipt.Schema ==
            "questionable.file_manager.quest_awake_receipt.v1" &&
            receipt.RequestId == invocation.RequestId &&
            receipt.OperationId == operation.OperationId &&
            receipt.PreviewId == operation.Preview.PreviewId &&
            receipt.DeviceId == target.DeviceId &&
            receipt.IdentityRevision == target.IdentityRevision &&
            receipt.Action == operation.Preview.Action &&
            receipt.WatchdogGeneration ==
            invocation.WatchdogGeneration &&
            receipt.RequestedDurationMs == operation.Preview.DurationMs &&
            receipt.RequestedWatchdogIntervalMs ==
            operation.Preview.WatchdogIntervalMs &&
            IsBoundedText(receipt.Outcome, 1_024) &&
            IsLowerHexSha256(receipt.EvidenceSha256) &&
            receipt.ObservedAtMs >= invocation.IssuedAtMs,
            "Quest awake effective receipt binding");
        ValidatePolicy(
            receipt.RequestedDurationMs,
            receipt.RequestedWatchdogIntervalMs);
        Require(
            IsBoundedText(receipt.Power.Wakefulness, 128) &&
            IsBoundedText(receipt.Power.DisplayState, 128) &&
            IsBoundedText(receipt.Power.ProximityState, 128) &&
            receipt.Power.CapturedAtMs >= invocation.IssuedAtMs &&
            receipt.DeviceWatchdog.IntervalMs <=
            QuestAwakeActions.MaximumWatchdogIntervalMs &&
            receipt.DeviceWatchdog.LastAction.Length <= 1_024 &&
            receipt.DeviceWatchdog.LastError.Length <= 1_024 &&
            (string.IsNullOrEmpty(receipt.DeviceWatchdog.BootIdSha256) ||
             IsLowerHexSha256(receipt.DeviceWatchdog.BootIdSha256)),
            "Quest awake readback evidence");

        var expectedEffective = receipt.Action switch
        {
            QuestAwakeActions.Status => true,
            QuestAwakeActions.ApplyBounded =>
                receipt.StayOnEffective &&
                receipt.ProximityHoldEffective &&
                receipt.WakeEffective &&
                !receipt.WindowsWatchdogEffective &&
                !receipt.DeviceWatchdogEffective,
            QuestAwakeActions.StartWindowsWatchdog =>
                receipt.StayOnEffective &&
                receipt.ProximityHoldEffective &&
                receipt.WakeEffective &&
                receipt.WindowsWatchdogEffective,
            QuestAwakeActions.StartDeviceWatchdog =>
                receipt.StayOnEffective &&
                receipt.ProximityHoldEffective &&
                receipt.WakeEffective &&
                receipt.DeviceWatchdogEffective,
            QuestAwakeActions.StopWatchdogs =>
                !receipt.WindowsWatchdogEffective &&
                !receipt.DeviceWatchdogEffective &&
                receipt.SettingsLeftUnchanged &&
                !receipt.SettingsRestored,
            QuestAwakeActions.RestoreNormal =>
                !receipt.WindowsWatchdogEffective &&
                !receipt.DeviceWatchdogEffective &&
                receipt.SettingsRestored,
            _ => false
        };
        Require(
            receipt.Effective == expectedEffective,
            "Quest awake independent effective readbacks");
    }

    private static bool IsStopOrRestore(string action) =>
        action is QuestAwakeActions.StopWatchdogs or
            QuestAwakeActions.RestoreNormal;

    private static void ValidatePolicy(uint durationMs, uint watchdogIntervalMs) =>
        Require(
            durationMs is >= QuestAwakeActions.MinimumDurationMs and
                <= QuestAwakeActions.MaximumDurationMs &&
            watchdogIntervalMs is >= QuestAwakeActions.MinimumWatchdogIntervalMs and
                <= QuestAwakeActions.MaximumWatchdogIntervalMs,
            "Quest awake duration and watchdog interval");

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
