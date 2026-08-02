# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $CapsuleRoot,
    [string] $PinPath = (Join-Path $PSScriptRoot "..\config\fleet-agent-key-record-owner-release.v1.json")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-Sha256([string] $LiteralPath) {
    return (Get-FileHash -LiteralPath $LiteralPath -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-ExactProperties($Value, [string[]] $Names, [string] $Label) {
    $actual = @($Value.PSObject.Properties.Name | Sort-Object)
    $expected = @($Names | Sort-Object)
    if (@(Compare-Object $actual $expected -SyncWindow 0).Count -ne 0 -or
        $actual.Count -ne $expected.Count) {
        throw "$Label contains an unknown or missing field"
    }
}

$root = (Resolve-Path -LiteralPath $CapsuleRoot).Path
$pin = Get-Content -Raw -LiteralPath (Resolve-Path -LiteralPath $PinPath).Path |
    ConvertFrom-Json -Depth 30
Assert-ExactProperties $pin @(
    "schema", "owner_id", "owner_repository", "consumer_id", "capsule_schema",
    "capsule_version", "manifest_sha256", "executable_sha256",
    "executable_size_bytes", "provenance_sha256", "source_commit", "source_tree",
    "target", "payload", "owner_signature", "claims") "Fleet owner release pin"
Assert-ExactProperties $pin.owner_signature @("present", "authority", "reason") "owner signature boundary"
Assert-ExactProperties $pin.claims @(
    "capsule_validity", "onboarding_accepted", "device_activated", "device_reachable",
    "lease_issued", "peer_accepted", "live_authority") "capsule non-claims"
if ($pin.schema -cne "rusty.fleet.fleet_agent_key_record_owner_release_pin.v1" -or
    $pin.owner_id -cne "rusty-quest" -or
    $pin.owner_repository -cne "https://github.com/MesmerPrism/rusty-quest" -or
    $pin.consumer_id -cne "rusty-fleet/fleet-onboard" -or
    $pin.capsule_schema -cne "rusty.quest.fleet_agent_key_record_release_capsule.v1" -or
    $pin.capsule_version -cne "1.0.0" -or
    $pin.target -cne "x86_64-pc-windows-msvc" -or
    $pin.owner_signature.present -ne $false -or
    $pin.owner_signature.authority -cne "absent" -or
    $pin.owner_signature.reason -cne
        "rusty-quest-has-no-key-record-capsule-signing-or-revocation-authority" -or
    $pin.claims.capsule_validity -cne "packaging-and-tool-provenance-only" -or
    $pin.claims.onboarding_accepted -ne $false -or
    $pin.claims.device_activated -ne $false -or
    $pin.claims.device_reachable -ne $false -or
    $pin.claims.lease_issued -ne $false -or
    $pin.claims.peer_accepted -ne $false -or
    $pin.claims.live_authority -cne "rusty-manifold") {
    throw "Fleet owner release pin crosses the reviewed authority boundary"
}

$expectedFiles = @(
    "LICENSE", "SOURCE-NOTICE.md", "checksums.sha256", "fleet-agent-key-record.exe",
    "provenance.json", "release-manifest.json") | Sort-Object
$actualFiles = @(Get-ChildItem -LiteralPath $root -File -Recurse | ForEach-Object {
    [IO.Path]::GetRelativePath($root, $_.FullName).Replace("\", "/")
} | Sort-Object)
if (@(Compare-Object $actualFiles $expectedFiles -SyncWindow 0).Count -ne 0 -or
    $actualFiles.Count -ne $expectedFiles.Count) {
    throw "owner capsule contains an extra or missing file"
}

$manifestPath = Join-Path $root "release-manifest.json"
$manifestText = Get-Content -Raw -LiteralPath $manifestPath
if ((Get-Sha256 $manifestPath) -cne $pin.manifest_sha256) {
    throw "owner capsule manifest hash does not match the Fleet pin"
}
$manifest = $manifestText | ConvertFrom-Json -Depth 30
Assert-ExactProperties $manifest @(
    "schema", "capsule_version", "tool_contract", "source", "artifact",
    "distribution", "payload") "owner release manifest"
Assert-ExactProperties $manifest.source @(
    "repository_url", "commit", "tree", "provenance_path", "provenance_sha256") "owner source"
Assert-ExactProperties $manifest.artifact @(
    "path", "sha256", "size_bytes", "target", "profile") "owner artifact"
Assert-ExactProperties $manifest.distribution @(
    "portable", "supported", "inert_until_invoked", "install_contract",
    "private_material_included", "live_onboarding_claim") "owner distribution"
if ($manifest.schema -cne $pin.capsule_schema -or
    $manifest.capsule_version -cne $pin.capsule_version -or
    $manifest.source.repository_url -cne $pin.owner_repository -or
    $manifest.source.commit -cne $pin.source_commit -or
    $manifest.source.tree -cne $pin.source_tree -or
    $manifest.source.provenance_path -cne "provenance.json" -or
    $manifest.source.provenance_sha256 -cne $pin.provenance_sha256 -or
    $manifest.artifact.path -cne "fleet-agent-key-record.exe" -or
    $manifest.artifact.sha256 -cne $pin.executable_sha256 -or
    [long]$manifest.artifact.size_bytes -ne [long]$pin.executable_size_bytes -or
    $manifest.artifact.target -cne $pin.target -or
    $manifest.artifact.profile -cne "release" -or
    $manifest.distribution.portable -ne $true -or
    $manifest.distribution.supported -ne $true -or
    $manifest.distribution.inert_until_invoked -ne $true -or
    $manifest.distribution.install_contract -cne "copy_capsule_byte_for_byte" -or
    $manifest.distribution.private_material_included -ne $false -or
    $manifest.distribution.live_onboarding_claim -ne $false) {
    throw "owner capsule manifest does not match the supported Fleet pin"
}

$pinnedPayload = @($pin.payload)
$ownerPayload = @($manifest.payload)
if ($pinnedPayload.Count -ne 4 -or $ownerPayload.Count -ne 4) {
    throw "owner capsule payload is not closed"
}
for ($index = 0; $index -lt $pinnedPayload.Count; $index++) {
    $expected = $pinnedPayload[$index]
    $actual = $ownerPayload[$index]
    Assert-ExactProperties $expected @("path", "sha256", "size_bytes") "pinned payload"
    Assert-ExactProperties $actual @("path", "sha256", "size_bytes") "owner payload"
    $path = Join-Path $root ([string]$expected.path)
    if ($actual.path -cne $expected.path -or $actual.sha256 -cne $expected.sha256 -or
        [long]$actual.size_bytes -ne [long]$expected.size_bytes -or
        (Get-Sha256 $path) -cne $expected.sha256 -or
        [long](Get-Item -LiteralPath $path).Length -ne [long]$expected.size_bytes) {
        throw "owner capsule payload bytes do not match the Fleet pin"
    }
}

$provenancePath = Join-Path $root "provenance.json"
$provenanceText = Get-Content -Raw -LiteralPath $provenancePath
if ((Get-Sha256 $provenancePath) -cne $pin.provenance_sha256) {
    throw "owner capsule provenance hash does not match the Fleet pin"
}
$provenance = $provenanceText | ConvertFrom-Json -Depth 30
Assert-ExactProperties $provenance @("schema", "capsule_version", "source", "build", "claims") "owner provenance"
Assert-ExactProperties $provenance.source @(
    "repository_url", "commit", "tree", "package", "composition_fingerprint",
    "repositories", "files") "owner provenance source"
Assert-ExactProperties $provenance.claims @(
    "owner", "helper_only", "runtime_activation", "enrollment_authority",
    "device_authority", "private_seed_included", "profile_included",
    "hub_configuration_included") "owner provenance claims"
$repositoryIds = @($provenance.source.repositories | ForEach-Object { $_.repository_id })
if ($provenance.schema -cne "rusty.quest.fleet_agent_key_record_release_provenance.v1" -or
    $provenance.capsule_version -cne $pin.capsule_version -or
    $provenance.source.repository_url -cne $pin.owner_repository -or
    $provenance.source.commit -cne $pin.source_commit -or
    $provenance.source.tree -cne $pin.source_tree -or
    $provenance.source.package -cne "rusty-quest-fleet-agent" -or
    $repositoryIds.Count -ne 3 -or
    @(Compare-Object $repositoryIds @("rusty-fleet", "rusty-manifold", "rusty-quest") -SyncWindow 0).Count -ne 0 -or
    $provenance.claims.owner -cne $pin.owner_id -or
    $provenance.claims.helper_only -ne $true -or
    $provenance.claims.runtime_activation -cne "explicit_fleet_onboard_invocation" -or
    $provenance.claims.enrollment_authority -ne $false -or
    $provenance.claims.device_authority -ne $false -or
    $provenance.claims.private_seed_included -ne $false -or
    $provenance.claims.profile_included -ne $false -or
    $provenance.claims.hub_configuration_included -ne $false) {
    throw "owner capsule provenance does not match the supported Fleet pin"
}

$publicText = $manifestText + "`n" + $provenanceText
foreach ($pattern in @(
    '[A-Za-z]:\\', '\\\\[^\\]', 'BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY',
    'signing-seed\.bin', 'hub_endpoint', 'state_directory', 'device_serial',
    'pairing_code')) {
    if ($publicText -match $pattern) {
        throw "owner capsule contains prohibited private or machine-local material"
    }
}

[pscustomobject][ordered]@{
    schema = "rusty.fleet.fleet_agent_key_record_owner_release_validation.v1"
    status = "pass"
    owner_id = [string]$pin.owner_id
    consumer_id = [string]$pin.consumer_id
    capsule_version = [string]$pin.capsule_version
    manifest_sha256 = [string]$pin.manifest_sha256
    executable_sha256 = [string]$pin.executable_sha256
    source_commit = [string]$pin.source_commit
    source_tree = [string]$pin.source_tree
    owner_signature_present = $false
    capsule_validity = "packaging-and-tool-provenance-only"
    onboarding_accepted = $false
    live_authority = "rusty-manifold"
} | ConvertTo-Json -Depth 5
