// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Text.Json;
using System.Text.Json.Nodes;
using RustyFleet.FleetConsole.Contracts;

namespace RustyFleet.FleetConsole.Tests;

internal static class WindowsHotspotContractTests
{
    public static void Run()
    {
        var operation = ValidStatusOperation();
        WindowsHotspotProjectionValidation.ValidateOperation(operation);

        var previewRequest = JsonNode.Parse(
            JsonSerializer.Serialize(
                new WindowsHotspotPreviewRequest
                {
                    Action = WindowsHotspotActions.Status
                },
                FleetJson.Options))?.AsObject() ??
            throw new InvalidOperationException(
                "Windows hotspot preview request was empty.");
        Require(
            previewRequest.Select(property => property.Key).ToHashSet(
                StringComparer.Ordinal).SetEquals(
                ["schema", "action_id", "action"]),
            "Windows hotspot preview request fields changed.");

        var executeRequest = JsonNode.Parse(
            JsonSerializer.Serialize(
                new WindowsHotspotExecuteRequest
                {
                    OperationId = operation.OperationId,
                    PreviewId = operation.Preview.PreviewId
                },
                FleetJson.Options))?.AsObject() ??
            throw new InvalidOperationException(
                "Windows hotspot execute request was empty.");
        Require(
            executeRequest.Select(property => property.Key).ToHashSet(
                StringComparer.Ordinal).SetEquals(
                ["schema", "operation_id", "preview_id"]),
            "Windows hotspot execute request fields changed.");

        var withPrivateField = JsonNode.Parse(
            JsonSerializer.Serialize(operation, FleetJson.Options)) ??
            throw new InvalidOperationException(
                "Windows hotspot operation was empty.");
        withPrivateField["receipt"]!["ssid"] = "must-never-cross-this-contract";
        var privateFieldRejected = false;
        try
        {
            _ = JsonSerializer.Deserialize<WindowsHotspotOperation>(
                withPrivateField.ToJsonString(),
                FleetJson.Options);
        }
        catch (JsonException)
        {
            privateFieldRejected = true;
        }
        Require(
            privateFieldRejected,
            "Windows hotspot projection accepted an unknown private field.");

        var wrongBinding = JsonNode.Parse(
            JsonSerializer.Serialize(operation, FleetJson.Options)) ??
            throw new InvalidOperationException(
                "Windows hotspot operation was empty.");
        wrongBinding["receipt"]!["operation_id"] = "hotspot-operation-other";
        var damaged = JsonSerializer.Deserialize<WindowsHotspotOperation>(
            wrongBinding.ToJsonString(),
            FleetJson.Options) ??
            throw new InvalidOperationException(
                "Damaged Windows hotspot operation was empty.");
        var wrongBindingRejected = false;
        try
        {
            WindowsHotspotProjectionValidation.ValidateOperation(damaged);
        }
        catch (InvalidOperationException)
        {
            wrongBindingRejected = true;
        }
        Require(
            wrongBindingRejected,
            "Windows hotspot projection accepted a changed owner binding.");

        var unsupportedLifecycle = JsonNode.Parse(
            JsonSerializer.Serialize(operation, FleetJson.Options)) ??
            throw new InvalidOperationException(
                "Windows hotspot operation was empty.");
        unsupportedLifecycle["lifecycle"] = "running";
        var unsupported = JsonSerializer.Deserialize<WindowsHotspotOperation>(
            unsupportedLifecycle.ToJsonString(),
            FleetJson.Options) ??
            throw new InvalidOperationException(
                "Unsupported Windows hotspot operation was empty.");
        var unsupportedLifecycleRejected = false;
        try
        {
            WindowsHotspotProjectionValidation.ValidateOperation(unsupported);
        }
        catch (InvalidOperationException)
        {
            unsupportedLifecycleRejected = true;
        }
        Require(
            unsupportedLifecycleRejected,
            "Windows hotspot projection accepted a lifecycle the host workflow never emits.");

        var wrongTimeout = JsonNode.Parse(
            JsonSerializer.Serialize(operation, FleetJson.Options)) ??
            throw new InvalidOperationException(
                "Windows hotspot operation was empty.");
        wrongTimeout["invocation"]!["timeout_ms"] = 29_999;
        var wrongTimeoutOperation =
            JsonSerializer.Deserialize<WindowsHotspotOperation>(
                wrongTimeout.ToJsonString(),
                FleetJson.Options) ??
            throw new InvalidOperationException(
                "Wrong-timeout Windows hotspot operation was empty.");
        var wrongTimeoutRejected = false;
        try
        {
            WindowsHotspotProjectionValidation.ValidateOperation(
                wrongTimeoutOperation);
        }
        catch (InvalidOperationException)
        {
            wrongTimeoutRejected = true;
        }
        Require(
            wrongTimeoutRejected,
            "Windows hotspot projection accepted a provider timeout other than 30 seconds.");
    }

    private static WindowsHotspotOperation ValidStatusOperation()
    {
        const long createdAtMs = 1_785_158_139_000;
        const long expiresAtMs = createdAtMs + 60_000;
        const string operationId = "hotspot-operation-contract";
        const string previewId = "hotspot-preview-contract";
        const string requestId = "hotspot-request-contract";
        return new WindowsHotspotOperation
        {
            Schema = "rusty.fleet.windows_hotspot_operation.v1",
            OperationId = operationId,
            ActionId = WindowsHotspotActions.ActionId,
            Lifecycle = "applied",
            Preview = new WindowsHotspotPreview
            {
                Schema = "rusty.fleet.windows_hotspot_preview.v1",
                PreviewId = previewId,
                OperationId = operationId,
                ActionId = WindowsHotspotActions.ActionId,
                ResourceId = WindowsHotspotActions.ResourceId,
                OwnerId = WindowsHotspotActions.OwnerId,
                Action = WindowsHotspotActions.Status,
                CreatedAtMs = createdAtMs,
                ExpiresAtMs = expiresAtMs,
                FleetRevision = 1,
                Preflight = new WindowsHotspotPreflight
                {
                    ProviderReady = true,
                    LeaseAvailable = true,
                    Active = false,
                    Ownership = "none",
                    Eligible = true,
                    ReasonCode = "ready",
                    Message =
                        "Host-scoped hotspot operation is ready for explicit confirmation"
                }
            },
            ConfirmedAtMs = createdAtMs + 1,
            Lease = new WindowsHotspotLease
            {
                Schema = "rusty.fleet.host_resource_lease.v1",
                LeaseId = "hotspot-lease-contract",
                ResourceId = WindowsHotspotActions.ResourceId,
                HolderOperationId = operationId,
                Generation = "hotspot-generation-contract",
                IssuedAtMs = createdAtMs,
                ExpiresAtMs = expiresAtMs
            },
            Invocation = new WindowsHotspotProviderRequest
            {
                Schema =
                    "rusty.hostess.windows_hotspot.provider_request.v1",
                RequestId = requestId,
                OperationId = operationId,
                Action = WindowsHotspotActions.Status,
                ExpiresAtUtc = Utc(expiresAtMs),
                TimeoutMs = 30_000
            },
            Receipt = new WindowsHotspotProviderReceipt
            {
                Schema =
                    "rusty.hostess.windows_hotspot.provider_receipt.v1",
                RequestId = requestId,
                OperationId = operationId,
                Action = WindowsHotspotActions.Status,
                Outcome = "verified",
                Reason = "status.readback_verified",
                ObservedAtUtc = Utc(createdAtMs + 2),
                CapabilityAvailable = true,
                Capability = "Enabled",
                OperationalState = "Off",
                ClientCount = 0,
                MaxClientCount = 8,
                Band = "FiveGigahertz",
                SourceConnectivity = "Internet"
            },
            UpdatedAtMs = createdAtMs + 2
        };
    }

    private static string Utc(long unixTimeMs) =>
        DateTimeOffset
            .FromUnixTimeMilliseconds(unixTimeMs)
            .UtcDateTime
            .ToString("O");

    private static void Require(bool condition, string message)
    {
        if (!condition)
        {
            throw new InvalidOperationException(message);
        }
    }
}
