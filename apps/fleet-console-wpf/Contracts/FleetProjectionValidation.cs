// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Text.Json;

namespace RustyFleet.FleetConsole.Contracts;

public static class FleetProjectionValidation
{
    private static readonly HashSet<string> FreshnessStates =
    [
        "fresh",
        "stale",
        "offline",
        "unknown"
    ];

    private static readonly HashSet<string> WatchRejectionReasons =
    [
        "contract_invalid",
        "duplicate_revision",
        "stale_revision",
        "identity_revision_rollback",
        "identity_revision_changed_without_restart",
        "identity_conflict",
        "source_epoch_changed_without_restart",
        "source_epoch_replay",
        "source_epoch_evidence_limit_exceeded",
        "receive_time_regression"
    ];

    private static readonly HashSet<string> OperationLifecycles =
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
        "cancelled"
    ];

    public static void ValidateQueryResult(
        FleetQueryResult result,
        FleetSummaryProjection summary,
        FleetQuery requestedQuery)
    {
        Require(result.Schema == "rusty.fleet.query_result.v1", "query-result schema");
        Require(result.Query.Schema == "rusty.fleet.query.v1", "query schema");
        Require(result.ResultRevision > 0, "result revision");
        Require(
            JsonSerializer.Serialize(result.Query, FleetJson.Options) ==
            JsonSerializer.Serialize(requestedQuery, FleetJson.Options),
            "query correlation");
        Require(result.WindowOffset >= 0, "window offset");
        Require(result.WindowCount == result.Rows.Count, "window count");
        Require(result.TotalCount >= result.WindowCount, "total count");
        Require(
            (long)result.WindowOffset + result.WindowCount <= result.TotalCount,
            "window bounds");
        Require(result.Rows.Count <= requestedQuery.Limit, "requested window limit");
        ValidateSummary(summary);

        var identities = new HashSet<string>(StringComparer.Ordinal);
        foreach (var row in result.Rows)
        {
            ValidateRow(row);
            Require(
                identities.Add(
                    $"{row.Identity.DeviceId}@{row.Identity.IdentityRevision}"),
                "duplicate device identity");
        }
    }

    public static void ValidateInspector(
        DeviceInspectorProjection projection,
        DeviceRowProjection expectedRow)
    {
        Require(
            projection.Schema == "rusty.fleet.device_inspector.v1",
            "inspector schema");
        ValidateRow(projection.Row);
        Require(
            projection.Row.Identity.DeviceId == expectedRow.Identity.DeviceId &&
            projection.Row.Identity.IdentityRevision ==
            expectedRow.Identity.IdentityRevision,
            "inspector device identity");
        Require(projection.Attention.Count <= 64, "inspector attention limit");
        Require(projection.Streams.Count <= 32, "inspector stream limit");
        Require(projection.ActiveOperations.Count <= 128, "inspector operation limit");
        foreach (var operation in projection.ActiveOperations)
        {
            ValidateOperationElement(operation);
        }
    }

    public static void ValidateDetail(
        DeviceDetailProjection projection,
        DeviceRowProjection expectedRow)
    {
        Require(
            projection.Schema == "rusty.fleet.device_detail.v1",
            "device-detail schema");
        ValidateInspector(projection.Inspector, expectedRow);
        Require(projection.ConditionHistory.Count <= 128, "condition-history limit");
        Require(projection.OperationHistory.Count <= 1_000, "operation-history limit");
        foreach (var condition in projection.ConditionHistory)
        {
            ValidateCondition(condition);
        }
        foreach (var operation in projection.OperationHistory)
        {
            ValidateOperationElement(operation);
        }
    }

    private static void ValidateOperationElement(JsonElement operation)
    {
        var operationSchema =
            operation.ValueKind == JsonValueKind.Object &&
            operation.TryGetProperty("schema", out var schema)
                ? schema.GetString()
                : null;
        Require(
            operation.ValueKind == JsonValueKind.Object &&
            operationSchema is not null &&
            operation.TryGetProperty("operation_id", out var operationId) &&
            !string.IsNullOrWhiteSpace(operationId.GetString()) &&
            operation.TryGetProperty("action_id", out var actionId) &&
            !string.IsNullOrWhiteSpace(actionId.GetString()),
            "operation-history entry");
        if (operationSchema == "rusty.fleet.kiosk_show_controls_operation.v1")
        {
            var kioskOperation = JsonSerializer.Deserialize<OperationLedger>(
                operation.GetRawText(),
                FleetJson.Options) ?? throw new InvalidOperationException(
                    "Fleet Hub returned an empty Kiosk operation.");
            ValidateOperationLedger(kioskOperation);
            return;
        }

        Require(
            operationSchema == "rusty.fleet.operation_ledger.v1",
            "operation-history schema");
    }

    public static void ValidateOperationLedger(OperationLedger operation)
    {
        Require(
            operation.Schema == "rusty.fleet.kiosk_show_controls_operation.v1",
            "operation-ledger schema");
        Require(
            IsBoundedText(operation.OperationId, 128),
            "operation-ledger ID");
        Require(
            operation.ActionId == FleetOperationActions.KioskShowControls,
            "operation-ledger action");
        Require(
            OperationLifecycles.Contains(operation.Lifecycle),
            "operation-ledger lifecycle");
        Require(
            operation.MaxParallelism is >= 1 and <= 64,
            "operation parallelism");
        Require(
            operation.MaxAttemptsPerTarget is >= 1 and <= 8,
            "operation attempt limit");
        Require(!operation.CleanupRequired, "operation cleanup prohibition");

        var preview = operation.Preview;
        Require(
            preview.Schema == "rusty.fleet.kiosk_show_controls_preview.v1",
            "operation preview schema");
        Require(
            IsBoundedText(preview.PreviewId, 128),
            "operation preview ID");
        Require(
            preview.OperationId == operation.OperationId &&
            preview.ActionId == operation.ActionId &&
            preview.CreatedAtMs == operation.CreatedAtMs,
            "operation preview binding");
        Require(
            preview.ExpiresAtMs > preview.CreatedAtMs,
            "operation preview lifetime");
        Require(preview.FleetRevision > 0, "operation preview fleet revision");
        ValidateOwnerContract(preview.OwnerContract);
        Require(
            preview.Targets.Count is >= 1 and <= 10_000,
            "operation preview target count");
        string? priorPreviewDeviceId = null;
        foreach (var target in preview.Targets)
        {
            ValidatePreflight(target, preview);
            Require(
                priorPreviewDeviceId is null ||
                StringComparer.Ordinal.Compare(
                    priorPreviewDeviceId,
                    target.DeviceId) < 0,
                "operation preview target ordering");
            priorPreviewDeviceId = target.DeviceId;
        }

        Require(
            operation.Targets.Count == preview.Targets.Count,
            "operation target result count");
        string? priorTargetDeviceId = null;
        for (var index = 0; index < operation.Targets.Count; index++)
        {
            var target = operation.Targets[index];
            var previewTarget = preview.Targets[index];
            Require(
                IsBoundedText(target.DeviceId, 256) &&
                (priorTargetDeviceId is null ||
                 StringComparer.Ordinal.Compare(
                     priorTargetDeviceId,
                     target.DeviceId) < 0),
                "operation target device");
            priorTargetDeviceId = target.DeviceId;
            Require(
                target.DeviceId == target.Preflight.DeviceId &&
                target.IdentityRevision == target.Preflight.IdentityRevision &&
                JsonSerializer.Serialize(target.Preflight, FleetJson.Options) ==
                JsonSerializer.Serialize(previewTarget, FleetJson.Options),
                "operation target identity binding");
            ValidatePreflight(target.Preflight, preview);
            Require(
                target.AttemptCount <= operation.MaxAttemptsPerTarget,
                "operation target attempt limit");
            Require(
                target.DispatchedAtMs is null ||
                (target.DispatchedAtMs >= preview.CreatedAtMs &&
                 target.DispatchedAtMs <= preview.ExpiresAtMs &&
                 target.DispatchedAtMs <= target.LastTransitionMs),
                "operation target dispatch time");
            Require(
                target.DispatchedAtMs is null
                    ? target.OwnerDeadlineAtMs is null
                    : target.OwnerDeadlineAtMs > target.DispatchedAtMs &&
                      target.OwnerDeadlineAtMs <= preview.ExpiresAtMs,
                "operation target owner deadline");
            Require(
                OperationLifecycles.Contains(target.Lifecycle),
                "operation target lifecycle");
            Require(
                target.OwnerRequestIds.Count == target.AttemptCount &&
                target.OwnerRequestIds.Count <= operation.MaxAttemptsPerTarget &&
                target.OwnerRequestIds.Distinct(StringComparer.Ordinal).Count() ==
                target.OwnerRequestIds.Count &&
                target.OwnerRequestIds.All(IsOwnerRequestId),
                "operation target owner request history");
            Require(
                target.OwnerRequestId is null
                    ? target.OwnerRequestIds.Count == 0
                    : target.OwnerRequestIds.LastOrDefault() ==
                      target.OwnerRequestId &&
                      IsOwnerRequestId(target.OwnerRequestId),
                "operation target current owner request");
            Require(
                IsBoundedText(target.ReasonCode, 128) &&
                IsBoundedText(target.Message, 1_024),
                "operation target explanation");
            ValidateTargetState(target, operation);
            if (target.EffectiveReceipt is not null)
            {
                ValidateEffectiveReceipt(
                    target.EffectiveReceipt,
                    operation,
                    target);
            }
        }
        Require(
            operation.Lifecycle == DeriveOperationLifecycle(operation.Targets),
            "operation aggregate lifecycle");
        Require(
            operation.Targets.Count(target =>
                target.Lifecycle is "dispatched" or "running") <=
            operation.MaxParallelism,
            "operation in-flight parallelism");
    }

    public static void ValidatePackageInstallReleaseOperation(
        PackageInstallReleaseOperation operation)
    {
        Require(
            operation.Schema == "rusty.fleet.package_install_release_operation.v1" &&
            IsBoundedText(operation.OperationId, 256) &&
            operation.ActionId == PackageOperationActions.InstallRelease &&
            operation.CreatedAtMs >= 0 &&
            operation.MaxParallelism is >= 1 and <= 64 &&
            !operation.CleanupRequired,
            "package operation header");
        Require(
            operation.Lifecycle is
                "proposed" or "accepted" or "dispatched" or "running" or
                "applied" or "failed" or "expired" or "rejected" or
                "cancellation_requested" or "cancelled",
            "package operation lifecycle");

        var preview = operation.Preview;
        Require(
            preview.Schema == "rusty.fleet.package_install_release_preview.v1" &&
            IsBoundedText(preview.PreviewId, 256) &&
            preview.OperationId == operation.OperationId &&
            preview.ActionId == operation.ActionId &&
            preview.CreatedAtMs == operation.CreatedAtMs &&
            preview.ExpiresAtMs > preview.CreatedAtMs &&
            preview.FleetRevision > 0 &&
            IsAndroidPackageName(preview.ExpectedPackageName) &&
            IsPortableIdentifier(preview.ExpectedRolloutRing, 128),
            "package operation preview");
        ValidatePackageRelease(preview.Release);
        Require(
            preview.OwnerContract.OwnerRepoId == "rusty-quest" &&
            preview.OwnerContract.CapabilityId == "rusty-quest.package-updater" &&
            preview.OwnerContract.ManifestEnvelopeSchema ==
            "rusty.quest.package_update_manifest_envelope.v1" &&
            preview.OwnerContract.ReceiptSchema ==
            "rusty.quest.package_update_receipt.v1" &&
            preview.OwnerContract.InstallMode == "attended_package_installer" &&
            preview.OwnerContract.ApplicationProof ==
            "effective_installed_version_receipt",
            "package updater owner contract");
        Require(
            preview.Targets.Count is >= 1 and <= 10_000 &&
            operation.Targets.Count == preview.Targets.Count,
            "package operation target count");

        string? priorDeviceId = null;
        for (var index = 0; index < preview.Targets.Count; index++)
        {
            var preflight = preview.Targets[index];
            var target = operation.Targets[index];
            ValidatePackagePreflight(preflight, preview);
            Require(
                priorDeviceId is null ||
                StringComparer.Ordinal.Compare(priorDeviceId, preflight.DeviceId) < 0,
                "package operation target ordering");
            priorDeviceId = preflight.DeviceId;
            Require(
                target.DeviceId == preflight.DeviceId &&
                target.IdentityRevision == preflight.IdentityRevision &&
                JsonSerializer.Serialize(target.Preflight, FleetJson.Options) ==
                JsonSerializer.Serialize(preflight, FleetJson.Options) &&
                target.LastTransitionMs >= operation.CreatedAtMs &&
                IsPortableIdentifier(target.ReasonCode, 256) &&
                IsBoundedText(target.Message, 1_024),
                "package target binding");

            if (!preflight.Eligible)
            {
                Require(
                    target.Lifecycle == "rejected" &&
                    target.Stage == "preflight_rejected" &&
                    target.Invocation is null &&
                    target.OwnerClaim is null &&
                    target.PriorOwnerClaims.Count == 0 &&
                    target.ConsumedOwnerClaimIdentities.Count == 0 &&
                    target.InvocationAcknowledgement is null &&
                    target.OwnerProgress is null &&
                    target.EffectiveReceipt is null,
                    "excluded package target");
                continue;
            }

            ValidatePackageOwnerHistory(target, operation);

            if (target.Lifecycle == "proposed")
            {
                Require(
                    target.Stage == "preview_ready" &&
                    target.Invocation is null &&
                    target.OwnerClaim is null &&
                    target.InvocationAcknowledgement is null &&
                    target.OwnerProgress is null &&
                    target.EffectiveReceipt is null,
                    "preview-ready package target");
                continue;
            }

            if (target.Lifecycle == "accepted")
            {
                Require(
                    target.Stage is "approved" or "dispatch_ready" &&
                    target.InvocationAcknowledgement is null &&
                    target.OwnerProgress is null &&
                    target.EffectiveReceipt is null,
                    "accepted package target");
                if (target.Stage == "approved")
                {
                    Require(
                        target.Invocation is null && target.OwnerClaim is null,
                        "approved package target");
                }
                else
                {
                    Require(
                        target.Invocation is not null,
                        "dispatch-ready package target invocation");
                    ValidatePackageInvocation(target.Invocation!, operation, target);
                    if (target.OwnerClaim is not null)
                    {
                        ValidatePackageClaim(target.OwnerClaim, operation, target);
                    }
                }
                continue;
            }

            Require(
                target.Invocation is not null,
                "post-claim package target invocation");
            ValidatePackageInvocation(target.Invocation!, operation, target);

            if (target.Lifecycle is "dispatched" or "running")
            {
                Require(
                    target.InvocationAcknowledgement is not null &&
                    target.InvocationAcknowledgement.Accepted &&
                    target.EffectiveReceipt is null,
                    "in-flight package owner evidence");
                ValidatePackageAcknowledgement(
                    target.InvocationAcknowledgement!,
                    target.Invocation!);
                Require(
                    target.Lifecycle == "dispatched"
                        ? target.Stage == "owner_acknowledged" &&
                          target.OwnerProgress is null
                        : target.Stage is
                            "staged" or "awaiting_wearer" or
                            "cancellation_requested" or "recovery_required",
                    "in-flight package stage");
                if (target.Lifecycle == "running")
                {
                    Require(target.OwnerProgress is not null, "running package progress");
                    ValidatePackageUpdaterProgress(
                        target.OwnerProgress!,
                        target.Invocation!);
                    Require(
                        target.OwnerProgress!.Stage == target.Stage,
                        "running package progress stage");
                }
                continue;
            }

            if (target.Lifecycle == "applied")
            {
                Require(
                    target.Stage == "applied" &&
                    target.EffectiveReceipt is not null,
                    "applied package target");
                ValidatePackageEffectiveReceipt(
                    target.EffectiveReceipt!,
                    target.Invocation!,
                    operation,
                    target);
                if (target.OwnerProgress is not null)
                {
                    ValidatePackageUpdaterProgress(target.OwnerProgress, target.Invocation!);
                }
                continue;
            }

            if (target.Lifecycle is "failed" or "expired")
            {
                Require(
                    target.Stage ==
                        (target.Lifecycle == "failed" ? "failed" : "expired") &&
                    target.EffectiveReceipt is null,
                    "terminal package target without application proof");
                if (target.InvocationAcknowledgement is not null)
                {
                    ValidatePackageAcknowledgement(
                        target.InvocationAcknowledgement,
                        target.Invocation!);
                }
                if (target.OwnerProgress is not null)
                {
                    ValidatePackageUpdaterProgress(target.OwnerProgress, target.Invocation!);
                    Require(
                        target.OwnerProgress.Stage == target.Stage,
                        "terminal package progress stage");
                }
                continue;
            }

            Require(
                target.Lifecycle is "cancellation_requested" or "cancelled" &&
                target.Stage == target.Lifecycle &&
                target.OwnerProgress is not null &&
                target.EffectiveReceipt is null,
                "cancelled package target");
            ValidatePackageUpdaterProgress(target.OwnerProgress!, target.Invocation!);
            Require(
                target.OwnerProgress!.Stage == target.Stage,
                "cancelled package progress stage");
        }

        var eligible = operation.Targets
            .Where(target => target.Preflight.Eligible)
            .ToArray();
        var expectedLifecycle = DerivePackageOperationLifecycle(eligible);
        Require(
            operation.Lifecycle == expectedLifecycle,
            "package operation aggregate lifecycle");
        Require(
            operation.Targets.Count(target =>
                target.Lifecycle is "dispatched" or "running" ||
                (target.Lifecycle == "accepted" && target.OwnerClaim is not null)) <=
            operation.MaxParallelism,
            "package operation owner-delivery parallelism");
    }

    public static void ValidatePackageInstallReleaseProgress(
        PackageInstallReleaseOperation operation,
        PackageInstallReleaseProgress progress)
    {
        ValidatePackageInstallReleaseOperation(operation);
        var total = checked((uint)operation.Targets.Count);
        var eligible = checked((uint)operation.Targets.Count(target =>
            target.Preflight.Eligible));
        uint CountStage(string stage) => checked((uint)operation.Targets.Count(target =>
            target.Stage == stage));
        var claimed = checked((uint)operation.Targets.Count(target =>
            target.Lifecycle == "accepted" && target.OwnerClaim is not null));
        var dispatchReady = checked((uint)operation.Targets.Count(target =>
            target.Stage == "dispatch_ready" && target.OwnerClaim is null));
        var retainedClaims = checked((uint)operation.Targets.Count(target =>
            target.Lifecycle is "dispatched" or "running" ||
            (target.Lifecycle == "accepted" && target.OwnerClaim is not null)));
        var terminalTargets = checked((uint)operation.Targets.Count(target =>
            target.Lifecycle is "rejected" or "applied" or "failed" or
                "expired" or "cancelled"));
        var terminal = terminalTargets == total;
        var lastTransition = operation.Targets.Count == 0
            ? operation.CreatedAtMs
            : operation.Targets.Max(target => target.LastTransitionMs);

        Require(
            progress.Schema == "rusty.fleet.package_install_release_progress.v1" &&
            progress.OperationId == operation.OperationId &&
            IsPrefixedSha256(progress.OperationSha256) &&
            progress.Lifecycle == operation.Lifecycle &&
            progress.Retention is "active" or "archived" &&
            progress.CreatedAtMs == operation.CreatedAtMs &&
            progress.LastTransitionMs == lastTransition &&
            progress.TotalTargets == total &&
            progress.EligibleTargets == eligible &&
            progress.ExcludedTargets == total - eligible &&
            progress.PreviewReady == CountStage("preview_ready") &&
            progress.Approved == CountStage("approved") &&
            progress.DispatchReady == dispatchReady &&
            progress.Claimed == claimed &&
            progress.OwnerAcknowledged == CountStage("owner_acknowledged") &&
            progress.Staged == CountStage("staged") &&
            progress.AwaitingWearer == CountStage("awaiting_wearer") &&
            progress.CancellationRequested == CountStage("cancellation_requested") &&
            progress.RecoveryRequired == CountStage("recovery_required") &&
            progress.Applied == CountStage("applied") &&
            progress.Failed == CountStage("failed") &&
            progress.Expired == CountStage("expired") &&
            progress.Cancelled == CountStage("cancelled") &&
            progress.RetainedOwnerClaims == retainedClaims &&
            progress.MaxParallelism == operation.MaxParallelism &&
            progress.TerminalTargets == terminalTargets &&
            progress.Terminal == terminal &&
            progress.ArchiveEligible == (terminal && progress.Retention == "active") &&
            (progress.Retention == "active"
                ? progress.ArchivedAtMs is null
                : progress.ArchivedAtMs >= lastTransition),
            "package operation progress projection");
    }

    public static void ValidatePackageInstallReleaseArchive(
        PackageInstallReleaseArchive archive)
    {
        ValidatePackageInstallReleaseOperation(archive.Operation);
        ValidatePackageInstallReleaseProgress(archive.Operation, archive.Progress);
        Require(
            archive.Schema == "rusty.fleet.package_install_release_archive.v1" &&
            archive.OperationId == archive.Operation.OperationId &&
            archive.OperationSha256 == archive.Progress.OperationSha256 &&
            archive.ArchivedAtMs >= archive.Progress.LastTransitionMs &&
            archive.Progress.Retention == "archived" &&
            archive.Progress.ArchivedAtMs == archive.ArchivedAtMs &&
            archive.Progress.Terminal &&
            !archive.Progress.ArchiveEligible,
            "package operation archive");
    }

    private static void ValidatePackageOwnerHistory(
        PackageInstallTargetLedger target,
        PackageInstallReleaseOperation operation)
    {
        Require(target.PriorOwnerClaims.Count <= 16, "prior package owner claims");
        Require(
            target.ConsumedOwnerClaimIdentities.Count <= 64,
            "consumed package owner claim identities");

        var claimIds = new HashSet<string>(StringComparer.Ordinal);
        var requestIds = new HashSet<string>(StringComparer.Ordinal);
        foreach (var identity in target.ConsumedOwnerClaimIdentities)
        {
            Require(
                IsPortableIdentifier(identity.ClaimId, 256) &&
                IsPortableIdentifier(identity.RequestId, 256) &&
                claimIds.Add(identity.ClaimId) &&
                requestIds.Add(identity.RequestId),
                "consumed package owner claim identity");
        }

        foreach (var claim in target.PriorOwnerClaims)
        {
            ValidatePackageClaim(claim, operation, target);
            Require(
                target.ConsumedOwnerClaimIdentities.Any(identity =>
                    identity.ClaimId == claim.ClaimId &&
                    identity.RequestId == claim.RequestId),
                "prior package owner claim replay identity");
        }

        if (target.OwnerClaim is not null)
        {
            Require(
                target.ConsumedOwnerClaimIdentities.Any(identity =>
                    identity.ClaimId == target.OwnerClaim.ClaimId &&
                    identity.RequestId == target.OwnerClaim.RequestId),
                "active package owner claim replay identity");
        }
    }

    private static void ValidatePackageClaim(
        PackageUpdaterClaim claim,
        PackageInstallReleaseOperation operation,
        PackageInstallTargetLedger target)
    {
        Require(
            claim.Schema == "rusty.fleet.package_updater_claim.v1" &&
            IsPortableIdentifier(claim.ClaimId, 256) &&
            claim.OwnerId == "rusty-quest.package-updater" &&
            IsPortableIdentifier(claim.RequestId, 256) &&
            claim.ClaimedAtMs >= 0 &&
            claim.ExpiresAtMs > claim.ClaimedAtMs &&
            claim.ExpiresAtMs <= claim.Invocation.ExpiresAtMs &&
            IsPrefixedSha256(claim.InvocationSha256) &&
            IsPrefixedSha256(claim.ReleaseSha256) &&
            IsPrefixedSha256(claim.TargetSha256),
            "package owner claim");
        ValidatePackageInvocation(claim.Invocation, operation, target);
        Require(
            target.Invocation is not null &&
            JsonSerializer.Serialize(claim.Invocation, FleetJson.Options) ==
            JsonSerializer.Serialize(target.Invocation, FleetJson.Options),
            "package owner claim invocation binding");
    }

    private static void ValidatePackageAcknowledgement(
        PackageUpdaterInvocationAcknowledgement acknowledgement,
        PackageUpdaterInvocation invocation)
    {
        Require(
            acknowledgement.Schema ==
            "rusty.fleet.package_updater_invocation_acknowledgement.v1" &&
            acknowledgement.OperationId == invocation.OperationId &&
            acknowledgement.DeviceId == invocation.DeviceId &&
            acknowledgement.OwnerActionRequestId == invocation.OwnerActionRequestId &&
            IsPortableIdentifier(acknowledgement.Code, 256) &&
            acknowledgement.AcknowledgedAtMs >= 0,
            "package updater acknowledgement");
    }

    private static void ValidatePackageUpdaterProgress(
        PackageUpdaterProgress progress,
        PackageUpdaterInvocation invocation)
    {
        Require(
            progress.Schema == "rusty.fleet.package_updater_progress.v1" &&
            progress.OperationId == invocation.OperationId &&
            progress.DeviceId == invocation.DeviceId &&
            progress.OwnerActionRequestId == invocation.OwnerActionRequestId &&
            progress.Stage is "staged" or "awaiting_wearer" or
                "cancellation_requested" or "cancelled" or
                "recovery_required" or "failed" or "expired" &&
            IsPortableIdentifier(progress.Code, 256) &&
            IsBoundedText(progress.Message, 1_024) &&
            progress.ObservedAtMs >= 0 &&
            progress.ObservedAtMs <= invocation.ExpiresAtMs,
            "package updater progress");
    }

    private static void ValidatePackageEffectiveReceipt(
        PackageUpdaterEffectiveReceipt receipt,
        PackageUpdaterInvocation invocation,
        PackageInstallReleaseOperation operation,
        PackageInstallTargetLedger target)
    {
        Require(
            receipt.Schema == "rusty.fleet.package_updater_effective_receipt.v1" &&
            receipt.OperationId == operation.OperationId &&
            receipt.DeviceId == target.DeviceId &&
            receipt.IdentityRevision == target.IdentityRevision &&
            receipt.OwnerActionRequestId == invocation.OwnerActionRequestId &&
            receipt.WrappedAtMs >= 0 &&
            receipt.WrappedAtMs <= invocation.ExpiresAtMs,
            "package updater effective receipt binding");

        var update = receipt.UpdaterReceipt;
        var checkpoint = update.AcceptedCheckpoint;
        Require(
            update.Schema == "rusty.quest.package_update_receipt.v1" &&
            update.Stage == "install_commit" &&
            update.Decision == "accepted" &&
            IsPortableIdentifier(update.Code, 256) &&
            update.ObservedAtMs >= 0 &&
            (update.EnvelopeSha256 is null ||
                IsPrefixedSha256(update.EnvelopeSha256)) &&
            (update.SignedManifestSha256 is null ||
                IsPrefixedSha256(update.SignedManifestSha256)) &&
            checkpoint is not null,
            "package updater install-commit receipt");
        ValidatePackageCheckpoint(checkpoint!);
        Require(
            update.PackageName == invocation.ExpectedPackageName &&
            update.RolloutRing == invocation.ExpectedRolloutRing &&
            update.Sequence == checkpoint!.Sequence &&
            update.VersionCode == checkpoint.VersionCode &&
            update.SignedManifestSha256 == checkpoint.SignedManifestSha256 &&
            checkpoint.PackageName == invocation.ExpectedPackageName &&
            checkpoint.RolloutRing == invocation.ExpectedRolloutRing,
            "package updater installed-version proof");
        if (update.PriorCheckpoint is not null)
        {
            ValidatePackageCheckpoint(update.PriorCheckpoint);
        }
    }

    private static void ValidatePackageCheckpoint(PackageUpdateCheckpoint checkpoint)
    {
        Require(
            IsAndroidPackageName(checkpoint.PackageName) &&
            IsPortableIdentifier(checkpoint.RolloutRing, 128) &&
            checkpoint.Sequence > 0 &&
            checkpoint.VersionCode > 0 &&
            IsPrefixedSha256(checkpoint.SignedManifestSha256),
            "package updater checkpoint");
    }

    private static string DerivePackageOperationLifecycle(
        IReadOnlyList<PackageInstallTargetLedger> eligible)
    {
        if (eligible.Count == 0)
        {
            return "rejected";
        }
        if (eligible.Any(target => target.Lifecycle == "running"))
        {
            return "running";
        }
        if (eligible.Any(target => target.Lifecycle == "dispatched"))
        {
            return "dispatched";
        }
        if (eligible.Any(target => target.Lifecycle == "proposed"))
        {
            return "proposed";
        }
        if (eligible.Any(target => target.Lifecycle == "accepted"))
        {
            return "accepted";
        }
        if (eligible.All(target => target.Lifecycle == "applied"))
        {
            return "applied";
        }
        if (eligible.All(target => target.Lifecycle == "expired"))
        {
            return "expired";
        }
        return "failed";
    }

    private static void ValidatePackageRelease(PackageReleaseReference release)
    {
        if (release.Kind == "manifest_url")
        {
            Require(
                release.ReleaseId is null &&
                release.ManifestUrl is { Length: > 0 and <= 2_048 } &&
                Uri.TryCreate(release.ManifestUrl, UriKind.Absolute, out var manifest) &&
                manifest.Scheme == Uri.UriSchemeHttps &&
                string.IsNullOrEmpty(manifest.UserInfo) &&
                string.IsNullOrEmpty(manifest.Fragment),
                "package manifest URL");
            return;
        }

        Require(
            release.Kind == "release_id" &&
            release.ManifestUrl is null &&
            release.ReleaseId is not null &&
            IsPortablePathIdentifier(release.ReleaseId, 256),
            "package release ID");
    }

    private static void ValidatePackagePreflight(
        OperationTargetPreflight target,
        PackageInstallReleasePreview preview)
    {
        Require(
            IsBoundedText(target.DeviceId, 256) &&
            target.IdentityRevision > 0 &&
            target.CapabilityId == "rusty-quest.package-updater" &&
            target.CapabilityEvidenceRevision > 0 &&
            target.CapabilityOwner == "rusty-quest",
            "package preflight identity and owner");
        Require(
            target.Support is "supported" or "unsupported" or "unknown" &&
            target.Enablement is "enabled" or "disabled" or "unknown" &&
            target.Authorization is
                "authorized" or "unauthorized" or "restricted" or "unknown" &&
            target.Reachability is
                "reachable" or "disconnected" or "unavailable" or "unknown" &&
            target.Freshness is "current" or "stale" or "unknown",
            "package preflight capability facts");
        Require(
            target.FreshUntilMs >= target.ObservedAtMs &&
            target.EvaluatedAtMs >= target.ObservedAtMs &&
            target.EvaluatedAtMs >= preview.CreatedAtMs &&
            target.EvaluatedAtMs <= preview.ExpiresAtMs,
            "package preflight time window");
        var expectedReason = ExpectedPreflightReason(target);
        Require(
            target.ReasonCode == expectedReason &&
            target.Eligible == (expectedReason == "ready") &&
            IsBoundedText(target.Message, 1_024),
            "package preflight result");
    }

    private static void ValidatePackageInvocation(
        PackageUpdaterInvocation invocation,
        PackageInstallReleaseOperation operation,
        PackageInstallTargetLedger target)
    {
        Require(
            invocation.Schema == "rusty.fleet.package_updater_invocation.v1" &&
            invocation.OperationId == operation.OperationId &&
            invocation.PreviewId == operation.Preview.PreviewId &&
            invocation.DeviceId == target.DeviceId &&
            invocation.IdentityRevision == target.IdentityRevision &&
            IsPortableIdentifier(invocation.OwnerActionRequestId, 256) &&
            JsonSerializer.Serialize(invocation.Release, FleetJson.Options) ==
            JsonSerializer.Serialize(operation.Preview.Release, FleetJson.Options) &&
            invocation.ExpectedPackageName == operation.Preview.ExpectedPackageName &&
            invocation.ExpectedRolloutRing == operation.Preview.ExpectedRolloutRing &&
            invocation.ExpiresAtMs == operation.Preview.ExpiresAtMs,
            "package updater invocation binding");
        ValidatePackageRelease(invocation.Release);
    }

    private static string DeriveOperationLifecycle(
        IReadOnlyList<OperationTargetResult> targets)
    {
        var eligible = targets.Where(target => target.Preflight.Eligible).ToList();
        if (eligible.Count == 0)
        {
            return "rejected";
        }
        if (eligible.Any(target =>
                target.Lifecycle is "dispatched" or "running"))
        {
            return "running";
        }
        if (eligible.Any(target =>
                target.Lifecycle == "cancellation_requested"))
        {
            return "cancellation_requested";
        }
        if (eligible.Any(target => target.Lifecycle == "proposed"))
        {
            return "proposed";
        }
        if (eligible.Any(target => target.Lifecycle == "accepted"))
        {
            return "accepted";
        }
        if (eligible.All(target => target.Lifecycle == "applied"))
        {
            return "applied";
        }
        if (eligible.All(target => target.Lifecycle == "cancelled"))
        {
            return "cancelled";
        }
        if (eligible.All(target => target.Lifecycle == "expired"))
        {
            return "expired";
        }
        return "failed";
    }

    private static void ValidateOwnerContract(KioskOwnerContractBinding owner)
    {
        Require(
            owner.OwnerRepoId == "rusty-kiosk" &&
            owner.OwnerContractSchema == "rusty.kiosk.direct_operator.v1" &&
            owner.OwnerContractRevision ==
            "8954228f9ae67c5995a72569e3c9cdd3758f85c0" &&
            owner.CapabilityId == "rusty-kiosk.direct-operator" &&
            owner.RequestAuth == "hmac-sha256-v1" &&
            owner.ResponseAuth == "hmac-sha256-response-v1" &&
            owner.InvokeMethod == "POST" &&
            owner.InvokeTarget == "/v1/kiosk/invoke" &&
            owner.ResultMethod == "GET" &&
            owner.ResultTarget == "/v1/kiosk/result" &&
            owner.ResultRequestIdParameter == "request_id" &&
            owner.Port == 39_873 &&
            owner.MaxClockSkewSeconds == 90 &&
            owner.Command == "show-controls" &&
            owner.CommandValue is null &&
            owner.OwnerResultSchema == "rusty.kiosk.cli_result.v1",
            "Kiosk owner contract binding");
    }

    private static void ValidatePreflight(
        OperationTargetPreflight target,
        OperationPreview preview)
    {
        Require(
            IsBoundedText(target.DeviceId, 256) &&
            target.IdentityRevision > 0,
            "operation preflight identity");
        Require(
            target.CapabilityId == "rusty-kiosk.direct-operator" &&
            target.CapabilityEvidenceRevision > 0 &&
            target.CapabilityOwner == "rusty-kiosk",
            "operation preflight capability authority");
        Require(
            target.Support is "supported" or "unsupported" or "unknown" &&
            target.Enablement is "enabled" or "disabled" or "unknown" &&
            target.Authorization is
                "authorized" or "unauthorized" or "restricted" or "unknown" &&
            target.Reachability is
                "reachable" or "disconnected" or "unavailable" or "unknown" &&
            target.Freshness is "current" or "stale" or "unknown",
            "operation preflight capability facts");
        Require(
            target.FreshUntilMs >= target.ObservedAtMs &&
            target.EvaluatedAtMs >= target.ObservedAtMs &&
            target.EvaluatedAtMs >= preview.CreatedAtMs &&
            target.EvaluatedAtMs <= preview.ExpiresAtMs,
            "operation preflight time window");
        var expectedReason = ExpectedPreflightReason(target);
        Require(
            target.ReasonCode == expectedReason &&
            target.Eligible == (expectedReason == "ready") &&
            IsBoundedText(target.Message, 1_024),
            "operation preflight result");
    }

    private static string ExpectedPreflightReason(OperationTargetPreflight target) =>
        target.Support switch
        {
            "unsupported" => "unsupported",
            "unknown" => "support_unknown",
            _ => target.Enablement switch
            {
                "disabled" => "disabled",
                "unknown" => "enablement_unknown",
                _ => target.Authorization switch
                {
                    "unauthorized" => "unauthorized",
                    "restricted" => "restricted",
                    "unknown" => "authorization_unknown",
                    _ => target.Reachability switch
                    {
                        "disconnected" => "disconnected",
                        "unavailable" => "unavailable",
                        "unknown" => "reachability_unknown",
                        _ when target.Freshness == "current" &&
                               target.EvaluatedAtMs <= target.FreshUntilMs => "ready",
                        _ when target.Freshness == "unknown" => "freshness_unknown",
                        _ => "stale"
                    }
                }
            }
        };

    private static void ValidateTargetState(
        OperationTargetResult target,
        OperationLedger operation)
    {
        if (!target.Preflight.Eligible)
        {
            Require(
                target.Lifecycle == "rejected" &&
                target.AttemptCount == 0 &&
                target.DispatchedAtMs is null &&
                target.OwnerRequestId is null &&
                target.EffectiveReceipt is null &&
                target.RetryDisposition == "not_eligible" &&
                target.CancelDisposition == "terminal",
                "ineligible operation target state");
            return;
        }

        switch (target.Lifecycle)
        {
            case "proposed":
            case "accepted":
                Require(
                    target.AttemptCount == 0 &&
                    target.DispatchedAtMs is null &&
                    target.OwnerRequestId is null &&
                    target.EffectiveReceipt is null &&
                    target.RetryDisposition == "not_eligible" &&
                    target.CancelDisposition == "cancelable_before_dispatch",
                    "pre-dispatch operation target state");
                break;
            case "cancellation_requested":
                Require(
                    target.AttemptCount == 0 &&
                    target.DispatchedAtMs is null &&
                    target.OwnerRequestId is null &&
                    target.EffectiveReceipt is null &&
                    target.RetryDisposition == "not_eligible" &&
                    target.CancelDisposition ==
                    "cancellation_requested_before_dispatch",
                    "cancellation-requested operation target state");
                break;
            case "cancelled":
                Require(
                    target.AttemptCount == 0 &&
                    target.DispatchedAtMs is null &&
                    target.OwnerRequestId is null &&
                    target.EffectiveReceipt is null &&
                    target.RetryDisposition == "not_eligible" &&
                    target.CancelDisposition == "cancelled_before_dispatch",
                    "cancelled operation target state");
                break;
            case "dispatched":
            case "running":
                Require(
                    target.AttemptCount > 0 &&
                    target.DispatchedAtMs is not null &&
                    target.OwnerRequestId is not null &&
                    target.EffectiveReceipt is null &&
                    target.RetryDisposition == "not_eligible" &&
                    target.CancelDisposition == "not_cancelable_after_dispatch",
                    "in-flight operation target state");
                break;
            case "applied":
                Require(
                    target.AttemptCount > 0 &&
                    target.DispatchedAtMs is not null &&
                    target.OwnerRequestId is not null &&
                    target.EffectiveReceipt is not null &&
                    target.RetryDisposition == "not_eligible" &&
                    target.CancelDisposition == "terminal" &&
                    target.ReasonCode == "owner_effective_receipt",
                    "applied operation target state");
                break;
            case "failed":
            case "expired":
                Require(
                    target.AttemptCount > 0 &&
                    target.DispatchedAtMs is not null &&
                    target.OwnerRequestId is not null &&
                    target.EffectiveReceipt is null &&
                    target.RetryDisposition ==
                    (target.AttemptCount < operation.MaxAttemptsPerTarget
                        ? "new_owner_request_required"
                        : "not_eligible") &&
                    target.CancelDisposition == "terminal",
                    "failed operation target state");
                break;
            default:
                Require(false, "eligible operation target lifecycle");
                break;
        }
    }

    private static void ValidateEffectiveReceipt(
        KioskEffectiveReceipt receipt,
        OperationLedger operation,
        OperationTargetResult target)
    {
        ValidateOwnerContract(receipt.OwnerContract);
        Require(
            receipt.Schema == "rusty.fleet.kiosk_effective_receipt.v1" &&
            IsBoundedText(receipt.ReceiptId, 256),
            "operation receipt identity");
        Require(
            receipt.OperationId == operation.OperationId &&
            receipt.DeviceId == target.DeviceId &&
            receipt.IdentityRevision == target.IdentityRevision &&
            receipt.OwnerActionRequestId == target.OwnerRequestId &&
            IsOwnerRequestId(receipt.OwnerActionRequestId) &&
            IsOwnerRequestId(receipt.OwnerResultTransportRequestId) &&
            receipt.OwnerActionRequestId != receipt.OwnerResultTransportRequestId,
            "operation receipt binding");
        Require(
            receipt.ResponseStatus == 200 &&
            IsLowerHexSha256(receipt.ResponseContentSha256) &&
            IsLowerHexSha256(receipt.ResponseSignature) &&
            receipt.ResponseAuthVerified,
            "operation receipt response authentication");
        Require(
            receipt.OwnerResultSchema == "rusty.kiosk.cli_result.v1" &&
            receipt.OwnerCommand == "show-controls" &&
            receipt.OwnerAccepted &&
            receipt.OwnerCompleted &&
            receipt.ControlsOpen,
            "operation receipt effectiveness");
        Require(
            receipt.WrappedAtMs >= receipt.OwnerRecordedAtMs,
            "operation receipt time");
    }

    public static void ValidateWatchEvents(
        IReadOnlyList<FleetWatchEvent> events,
        ulong afterSequence,
        int requestedLimit)
    {
        Require(requestedLimit is >= 1 and <= 10_000, "watch requested limit");
        Require(events.Count <= requestedLimit, "watch response limit");

        var priorSequence = afterSequence;
        foreach (var watchEvent in events)
        {
            Require(
                watchEvent.Schema == "rusty.fleet.watch_event.v1",
                "watch-event schema");
            Require(
                watchEvent.EventSequence > priorSequence,
                "watch-event sequence");
            Require(watchEvent.ObservedAtMs >= 0, "watch-event observation time");
            Require(
                watchEvent.Decision.ResultRevision > 0,
                "watch-event result revision");
            Require(
                watchEvent.Decision.Kind is "accepted" or "rejected",
                "watch-event decision");

            if (watchEvent.Decision.Kind == "accepted")
            {
                Require(
                    !string.IsNullOrWhiteSpace(watchEvent.Decision.DeviceId),
                    "accepted watch-event device");
                Require(
                    watchEvent.Decision.SourceRevision is > 0,
                    "accepted watch-event source revision");
                Require(
                    watchEvent.Decision.Reason is null &&
                    watchEvent.Decision.Details.Count == 0,
                    "accepted watch-event rejection evidence");
            }
            else
            {
                Require(
                    watchEvent.Decision.Reason is not null &&
                    WatchRejectionReasons.Contains(watchEvent.Decision.Reason),
                    "rejected watch-event reason");
                Require(
                    watchEvent.Decision.Details.Count <= 64 &&
                    watchEvent.Decision.Details.All(detail =>
                        !string.IsNullOrWhiteSpace(detail) && detail.Length <= 1_024),
                    "rejected watch-event details");
            }

            priorSequence = watchEvent.EventSequence;
        }
    }

    public static void ValidateSavedViews(SavedViewCollection collection)
    {
        Require(
            collection.Schema == "rusty.fleet.saved_view_collection.v1",
            "saved-view collection schema");
        Require(collection.Revision > 0, "saved-view collection revision");
        Require(collection.Views.Count <= 128, "saved-view collection limit");
        string? priorId = null;
        foreach (var view in collection.Views)
        {
            ValidateSavedView(view);
            Require(
                priorId is null ||
                StringComparer.Ordinal.Compare(priorId, view.ViewId) < 0,
                "saved-view canonical ordering");
            priorId = view.ViewId;
        }
    }

    public static void ValidateSavedViewReceipt(SavedViewMutationReceipt receipt)
    {
        Require(
            receipt.Schema == "rusty.fleet.saved_view_mutation_receipt.v1",
            "saved-view receipt schema");
        Require(!string.IsNullOrWhiteSpace(receipt.ViewId), "saved-view receipt ID");
        Require(receipt.PreviousRevision > 0, "saved-view previous revision");
        Require(
            receipt.CurrentRevision >= receipt.PreviousRevision &&
            receipt.CurrentRevision - receipt.PreviousRevision <= 1,
            "saved-view receipt revision");
        Require(
            receipt.Changed ==
            (receipt.CurrentRevision > receipt.PreviousRevision),
            "saved-view changed revision");
        Require(receipt.Deleted == (receipt.View is null), "saved-view deletion receipt");
        Require(!receipt.Deleted || receipt.Changed, "saved-view deletion change");
        if (receipt.View is not null)
        {
            ValidateSavedView(receipt.View);
            Require(receipt.View.ViewId == receipt.ViewId, "saved-view receipt identity");
        }
    }

    private static void ValidateSavedView(SavedView view)
    {
        Require(view.Schema == "rusty.fleet.saved_view.v1", "saved-view schema");
        Require(
            IsValidSavedViewId(view.ViewId),
            "saved-view ID");
        Require(
            !string.IsNullOrWhiteSpace(view.Name) && view.Name.Length <= 256,
            "saved-view name");
        Require(view.Query.Schema == "rusty.fleet.query.v1", "saved-view query schema");
        Require(
            !string.IsNullOrWhiteSpace(view.Query.QueryId),
            "saved-view query ID");
        Require(
            view.Query.Limit is >= 1 and <= 10_000 &&
            view.Query.Sort.Count <= 8,
            "saved-view query bounds");
        Require(
            view.Columns.Count <= 64 &&
            view.Columns.All(column =>
                !string.IsNullOrWhiteSpace(column) && column.Length <= 128) &&
            view.Columns.Distinct(StringComparer.Ordinal).Count() == view.Columns.Count,
            "saved-view columns");
        Require(
            view.Density is "compact" or "standard" or "comfortable",
            "saved-view density");
        Require(
            view.Grouping is null ||
            (!string.IsNullOrWhiteSpace(view.Grouping) && view.Grouping.Length <= 128),
            "saved-view grouping");
        foreach (var value in new[]
                 {
                     view.Restoration.SelectedDeviceId,
                     view.Restoration.InspectorTab,
                     view.Restoration.ScrollAnchorDeviceId,
                     view.Restoration.FocusedRegion
                 })
        {
            Require(
                value is null ||
                (!string.IsNullOrWhiteSpace(value) && value.Length <= 128),
                "saved-view restoration text");
        }
        Require(view.SchemaVersion > 0, "saved-view schema version");
        Require(
            view.Restoration.CollapsedGroups.Count <= 512 &&
            view.Restoration.CollapsedGroups.All(group =>
                !string.IsNullOrWhiteSpace(group) && group.Length <= 128) &&
            view.Restoration.CollapsedGroups
                .Distinct(StringComparer.Ordinal).Count() ==
            view.Restoration.CollapsedGroups.Count,
            "saved-view collapsed groups");
    }

    private static void ValidateSummary(FleetSummaryProjection summary)
    {
        Require(summary.Schema == "rusty.fleet.summary.v1", "summary schema");
        Require(
            summary.Total >= 0 &&
            summary.Fresh >= 0 &&
            summary.Stale >= 0 &&
            summary.Offline >= 0 &&
            summary.Attention >= 0 &&
            summary.ActiveWork >= 0,
            "summary nonnegative counts");
        Require(
            (long)summary.Fresh + summary.Stale + summary.Offline <= summary.Total,
            "summary freshness counts");
        Require(summary.Attention <= summary.Total, "summary attention count");
    }

    private static void ValidateRow(DeviceRowProjection row)
    {
        Require(row.Schema == "rusty.fleet.device_row.v1", "device-row schema");
        Require(!string.IsNullOrWhiteSpace(row.Identity.DeviceId), "device ID");
        Require(row.Identity.IdentityRevision > 0, "identity revision");
        Require(!string.IsNullOrWhiteSpace(row.Identity.DisplayName), "display name");
        Require(!string.IsNullOrWhiteSpace(row.Identity.Model), "device model");
        Require(!string.IsNullOrWhiteSpace(row.Identity.HardwareClass), "hardware class");
        Require(row.Identity.Tags.Count <= 128, "identity tag limit");
        Require(
            row.Identity.Tags.All(entry =>
                !string.IsNullOrWhiteSpace(entry.Key) &&
                !string.IsNullOrWhiteSpace(entry.Value)),
            "identity tag values");
        Require(!string.IsNullOrWhiteSpace(row.SourceEpoch), "source epoch");
        Require(row.AcceptedRevision > 0, "accepted revision");
        Require(row.AgeMs >= 0, "accepted age");
        Require(FreshnessStates.Contains(row.Freshness), "freshness state");
        Require(
            row.BatteryPercent is null or >= 0 and <= 100,
            "battery percentage");
        Require(!string.IsNullOrWhiteSpace(row.KioskState), "kiosk state");
        Require(!string.IsNullOrWhiteSpace(row.Route), "route state");
        Require(row.Conditions.Count <= 16, "condition limit");
        Require(row.Capabilities.Capabilities.Count <= 128, "capability limit");

        foreach (var (key, condition) in row.Conditions)
        {
            Require(key == condition.Family, "condition key");
            ValidateCondition(condition);
        }

        foreach (var (key, capability) in row.Capabilities.Capabilities)
        {
            Require(key == capability.CapabilityId, "capability key");
            Require(!string.IsNullOrWhiteSpace(capability.Owner), "capability owner");
            Require(!string.IsNullOrWhiteSpace(capability.Reason), "capability reason");
            Require(capability.EvidenceRevision > 0, "capability evidence revision");
            Require(
                capability.FreshUntilMs >= capability.ObservedAtMs,
                "capability freshness");
        }
    }

    private static void ValidateCondition(StatusCondition condition)
    {
        Require(!string.IsNullOrWhiteSpace(condition.Family), "condition family");
        Require(!string.IsNullOrWhiteSpace(condition.State), "condition state");
        Require(!string.IsNullOrWhiteSpace(condition.Reason), "condition reason");
        Require(!string.IsNullOrWhiteSpace(condition.Message), "condition message");
        Require(!string.IsNullOrWhiteSpace(condition.Source.AdapterId), "condition adapter");
        Require(!string.IsNullOrWhiteSpace(condition.Source.Owner), "condition owner");
        Require(condition.AcceptedRevision > 0, "condition accepted revision");
        Require(condition.Source.AuthorityRevision > 0, "condition authority revision");
        Require(
            condition.FreshUntilMs >= condition.ReceivedTimeMs,
            "condition freshness");
    }

    private static void Require(bool condition, string field)
    {
        if (!condition)
        {
            throw new InvalidOperationException(
                $"Fleet Hub returned invalid projection evidence: {field}.");
        }
    }

    private static bool IsValidSavedViewId(string value)
    {
        if (value.Length is 0 or > 128)
        {
            return false;
        }

        return value.Split('.').All(segment =>
            segment.Length > 0 &&
            IsSavedViewIdEdge(segment[0]) &&
            IsSavedViewIdEdge(segment[^1]) &&
            segment.All(character =>
                IsSavedViewIdEdge(character) || character is '_' or '-'));
    }

    private static bool IsBoundedText(string value, int maximumLength) =>
        !string.IsNullOrWhiteSpace(value) && value.Length <= maximumLength;

    private static bool IsPortableIdentifier(string value, int maximumLength) =>
        value.Length is > 0 &&
        value.Length <= maximumLength &&
        value.All(character =>
            char.IsAsciiLetterOrDigit(character) || character is '.' or '_' or '-');

    private static bool IsPortablePathIdentifier(string value, int maximumLength) =>
        value.Length is > 0 &&
        value.Length <= maximumLength &&
        value.All(character =>
            char.IsAsciiLetterOrDigit(character) ||
            character is '.' or '_' or '-' or '/');

    private static bool IsAndroidPackageName(string value) =>
        value.Length is > 0 and <= 256 &&
        value.Contains('.') &&
        value.Split('.').All(segment =>
            segment.Length > 0 &&
            (char.IsAsciiLetter(segment[0]) || segment[0] == '_') &&
            segment.All(character =>
                char.IsAsciiLetterOrDigit(character) || character == '_'));

    private static bool IsOwnerRequestId(string value) =>
        value.Length is >= 8 and <= 64 &&
        value.All(character =>
            char.IsAsciiLetterOrDigit(character) || character is '_' or '-');

    private static bool IsLowerHexSha256(string value) =>
        value.Length == 64 &&
        value.All(character =>
            char.IsAsciiDigit(character) || character is >= 'a' and <= 'f');

    private static bool IsPrefixedSha256(string value) =>
        value.StartsWith("sha256:", StringComparison.Ordinal) &&
        IsLowerHexSha256(value[7..]);

    private static bool IsSavedViewIdEdge(char value) =>
        char.IsAsciiLetterLower(value) || char.IsAsciiDigit(value);
}
