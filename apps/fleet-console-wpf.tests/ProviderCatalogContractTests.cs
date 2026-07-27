// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Text.Json;
using System.IO;
using RustyFleet.FleetConsole.Contracts;

namespace RustyFleet.FleetConsole.Tests;

internal static class ProviderCatalogContractTests
{
    public static void Run()
    {
        var projection = new ProviderCatalogProjection(
            "rusty.fleet.provider_catalog.v1",
            "fc476166f9c05f941dff7e9183f5c893426c05ca",
            [
                new ProviderCatalogEntry(
                    "quest-connectivity",
                    "unconfigured",
                    "provider-not-configured",
                    Descriptor: null,
                    MetadataOnly: true,
                    AuthorizesExecution: false)
            ],
            MetadataOnly: true,
            AuthorizesExecution: false,
            Revision: 1,
            RefreshedAtMs: null);
        ProviderCatalogProjectionValidation.Validate(projection);

        var authorityRejected = false;
        try
        {
            ProviderCatalogProjectionValidation.Validate(
                projection with { AuthorizesExecution = true });
        }
        catch (InvalidDataException)
        {
            authorityRejected = true;
        }
        Require(authorityRejected, "provider metadata claimed execution authority");

        var unknown = JsonSerializer.Serialize(projection, FleetJson.Options)
            .TrimEnd('}')
            + ",\"target\":\"forbidden\"}";
        var unknownRejected = false;
        try
        {
            _ = JsonSerializer.Deserialize<ProviderCatalogProjection>(
                unknown,
                FleetJson.Options);
        }
        catch (JsonException)
        {
            unknownRejected = true;
        }
        Require(unknownRejected, "provider metadata accepted an unknown target field");
    }

    private static void Require(bool condition, string message)
    {
        if (!condition)
            throw new InvalidOperationException(message);
    }
}
