// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Text.Json;
using System.Text.Json.Serialization;

namespace RustyFleet.FleetConsole.Contracts;

[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed record ProviderCatalogProjection(
    [property: JsonPropertyName("schema")] string Schema,
    [property: JsonPropertyName("contract_source_commit")] string ContractSourceCommit,
    [property: JsonPropertyName("entries")] IReadOnlyList<ProviderCatalogEntry> Entries,
    [property: JsonPropertyName("metadata_only")] bool MetadataOnly,
    [property: JsonPropertyName("authorizes_execution")] bool AuthorizesExecution,
    [property: JsonPropertyName("revision")] ulong Revision,
    [property: JsonPropertyName("refreshed_at_ms")] long? RefreshedAtMs);

[JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
public sealed record ProviderCatalogEntry(
    [property: JsonPropertyName("catalog_id")] string CatalogId,
    [property: JsonPropertyName("state")] string State,
    [property: JsonPropertyName("reason")] string Reason,
    [property: JsonPropertyName("descriptor")] JsonElement? Descriptor,
    [property: JsonPropertyName("metadata_only")] bool MetadataOnly,
    [property: JsonPropertyName("authorizes_execution")] bool AuthorizesExecution);
