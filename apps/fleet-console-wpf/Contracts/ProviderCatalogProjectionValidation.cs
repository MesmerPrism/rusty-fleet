// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.IO;

namespace RustyFleet.FleetConsole.Contracts;

public static class ProviderCatalogProjectionValidation
{
    private const string Schema = "rusty.fleet.provider_catalog.v1";
    private const string ContractCommit =
        "fc476166f9c05f941dff7e9183f5c893426c05ca";

    public static void Validate(ProviderCatalogProjection projection)
    {
        ArgumentNullException.ThrowIfNull(projection);
        if (projection.Schema != Schema ||
            projection.ContractSourceCommit != ContractCommit ||
            !projection.MetadataOnly ||
            projection.AuthorizesExecution ||
            projection.Revision == 0 ||
            projection.Entries.Count > 64)
        {
            throw new InvalidDataException(
                "Provider catalog header is not the pinned inert metadata contract.");
        }

        var identities = new HashSet<string>(StringComparer.Ordinal);
        foreach (var entry in projection.Entries)
        {
            if (string.IsNullOrEmpty(entry.CatalogId) ||
                entry.CatalogId.Length > 160 ||
                !identities.Add(entry.CatalogId) ||
                entry.State is not (
                    "unconfigured" or
                    "valid" or
                    "stale" or
                    "rejected" or
                    "unavailable") ||
                string.IsNullOrEmpty(entry.Reason) ||
                entry.Reason.Length > 160 ||
                (entry.State is "valid" or "stale") != entry.Descriptor.HasValue ||
                !entry.MetadataOnly ||
                entry.AuthorizesExecution)
            {
                throw new InvalidDataException(
                    "Provider catalog entry is invalid or claims authority.");
            }
        }
    }
}
