# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $BundleRoot,

    [string] $ExpectedVersion,
    [string] $ExpectedFleetSignerThumbprint,
    [string] $ExpectedHostessSignerThumbprint
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
Import-Module (Join-Path $PSScriptRoot "Distribution.Common.psm1") -Force

$bundlePath = (Resolve-Path -LiteralPath $BundleRoot).Path
$manifestPath = Join-Path $bundlePath "metadata\release-manifest.json"
$checksumsPath = Join-Path $bundlePath "metadata\checksums.sha256"
$receiptPath = Join-Path $bundlePath "metadata\validation-receipt.json"
foreach ($requiredPath in @($manifestPath, $checksumsPath, $receiptPath)) {
    if (-not (Test-Path -LiteralPath $requiredPath -PathType Leaf)) {
        throw "required distribution metadata is missing: $requiredPath"
    }
}

$manifestText = Get-Content -LiteralPath $manifestPath -Raw
$manifest = $manifestText | ConvertFrom-Json -Depth 30
if ($manifest.schema -ne "rusty.fleet.windows_release_manifest.v1" -or
    $manifest.product_id -ne "rusty-fleet" -or
    $manifest.platform -ne "windows" -or
    $manifest.architecture -ne "x64") {
    throw "unsupported Rusty Fleet Windows release manifest identity"
}
if ($ExpectedVersion -and $manifest.version -ne $ExpectedVersion) {
    throw "bundle version does not match the expected version"
}
if ($manifest.build.kind -notin @("unsigned-dev", "signed-release") -or
    $manifest.build.reproducible_archive_for_identical_input_bytes -ne $true -or
    $manifest.build.source_to_artifact_binding -ne "clean_worktree_prebuild_assertion") {
    throw "bundle build metadata is invalid"
}
if ($manifest.distribution.binary_authority -ne "github_releases" -or
    $manifest.distribution.pages_role -ne
        "human_documentation_and_signed_metadata_only") {
    throw "GitHub Releases must remain binary truth and Pages must remain binary-free"
}
if ($manifest.build.kind -eq "signed-release") {
    if ($manifest.distribution.eligibility -ne "signed_release" -or
        $manifest.distribution.publication_allowed -ne $true) {
        throw "signed release distribution eligibility is invalid"
    }
}
elseif ($manifest.distribution.eligibility -ne "development_only" -or
    $manifest.distribution.publication_allowed -ne $false) {
    throw "unsigned development bundles must be explicitly non-publishable"
}

$componentIds = @($manifest.components | ForEach-Object { $_.component_id })
$expectedComponentIds = @(
    "fleet-console",
    "fleet-hub",
    "fleetctl",
    "fleet-onboard",
    "hostess-hotspot-provider"
)
if (@(Compare-Object $expectedComponentIds $componentIds).Count -ne 0 -or
    $componentIds.Count -ne $expectedComponentIds.Count) {
    throw "bundle must contain exactly the five declared runtime components"
}

$onboard = @($manifest.components |
    Where-Object { $_.component_id -eq "fleet-onboard" })
if ($onboard.Count -ne 1 -or
    $onboard[0].owner -ne "rusty-fleet" -or
    $onboard[0].kind -ne "offline_onboarding_generator" -or
    $onboard[0].entrypoint -ne "components/fleet-onboard/fleet-onboard.exe" -or
    $onboard[0].activation -ne "explicit_operator_invocation" -or
    $onboard[0].network_access -ne "absent" -or
    $onboard[0].output -ne "private_machine_bound_onboarding_only" -or
    $onboard[0].operational_readiness -ne
        "requires_separately_configured_signed_owner_key_record_tool" -or
    $onboard[0].bundled_owner_key_record_tool -ne $false -or
    $manifest.distribution.onboarding_ready -ne $false -or
    $manifest.distribution.onboarding_blocker -ne
        "signed_rusty_quest_owner_key_record_release_not_bundled") {
    throw "fleet-onboard component boundary is not exact"
}

$provider = @($manifest.components |
    Where-Object { $_.component_id -eq "hostess-hotspot-provider" })
if ($provider.Count -ne 1 -or
    $provider[0].owner -ne "rusty-hostess" -or
    $provider[0].entrypoint -ne "providers/hostess-hotspot-provider/rusty-hostess-hotspot-provider.exe" -or
    $provider[0].contract.action_id -ne "host.windows-mobile-hotspot" -or
    ($provider[0].contract.arguments -join " ") -ne "integration windows-hotspot --json" -or
    $provider[0].contract.stdin_schema -ne "rusty.hostess.windows_hotspot.provider_request.v1" -or
    $provider[0].contract.stdout_schema -ne "rusty.hostess.windows_hotspot.provider_receipt.v1" -or
    $provider[0].contract.process_results.verified -ne 0 -or
    $provider[0].contract.process_results.failed -ne 1 -or
    $provider[0].contract.process_results.rejected -ne 2 -or
    $provider[0].contract.process_results.unavailable -ne 3 -or
    $provider[0].provenance.supplied_externally -ne $true -or
    $provider[0].provenance.source_revision -cnotmatch "^[0-9a-f]{40}$" -or
    $provider[0].provenance.source_tree -cnotmatch "^[0-9a-f]{40}$" -or
    $provider[0].provenance.product_version -cne (
        "$($provider[0].provenance.provider_version)+" +
        $provider[0].provenance.source_revision
    ) -or
    $provider[0].provenance.owner_document_schema -ne
        "rusty.hostess.windows_hotspot.release_provenance.v1") {
    throw "Hostess hotspot provider contract or provenance is not exact"
}
Assert-RustyFleetSha256 `
    -Value $provider[0].provenance.artifact_sha256 `
    -Name "Hostess provider provenance digest"
foreach ($url in @(
    $provider[0].provenance.source_repository,
    $provider[0].provenance.source_availability_url
)) {
    Assert-RustyFleetHttpsUrl -Value $url -Name "Hostess provider provenance URL"
}
$providerPath = Join-Path $bundlePath $provider[0].entrypoint.Replace("/", "\")
$ownerMetadataDirectory = Join-Path $bundlePath (
    "providers\hostess-hotspot-provider\provenance"
)
$ownerProvenance = Read-RustyFleetHostessProvenance `
    -MetadataDirectory $ownerMetadataDirectory `
    -ProviderPath $providerPath `
    -ProviderSha256 $provider[0].provenance.artifact_sha256 `
    -BuildKind $manifest.build.kind `
    -ExpectedSignerThumbprint $ExpectedHostessSignerThumbprint
if ($provider[0].provenance.owner_document_path -ne
        "providers/hostess-hotspot-provider/provenance/rusty-hostess-hotspot-provider.provenance.json" -or
    $provider[0].provenance.license_path -ne
        "providers/hostess-hotspot-provider/provenance/LICENSE" -or
    $provider[0].provenance.third_party_notices_path -ne
        "providers/hostess-hotspot-provider/provenance/THIRD-PARTY-NOTICES.txt" -or
    $provider[0].provenance.owner_document_sha256 -cne
        $ownerProvenance.provenance_sha256 -or
    $provider[0].provenance.source_repository -ne
        $ownerProvenance.provenance.source.repository -or
    $provider[0].provenance.source_revision -ne
        $ownerProvenance.provenance.source.revision -or
    $provider[0].provenance.source_tree -ne
        $ownerProvenance.provenance.source.tree -or
    $provider[0].provenance.source_availability_url -ne
        $ownerProvenance.provenance.source.availability_url -or
    $provider[0].provenance.source_availability_state -ne
        $ownerProvenance.provenance.source.availability_state -or
    $provider[0].provenance.source_verified_at_utc -ne
        $ownerProvenance.provenance.source.verified_at_utc -or
    $provider[0].provenance.product_version -ne
        $ownerProvenance.provenance.artifact.product_version -or
    $provider[0].provenance.provider_version -ne
        $ownerProvenance.provenance.provider_version -or
    $provider[0].provenance.unsigned_artifact_sha256 -cne
        $ownerProvenance.provenance.build.unsigned_artifact_sha256 -or
    [long] $provider[0].provenance.unsigned_artifact_size_bytes -ne
        [long] $ownerProvenance.provenance.build.unsigned_artifact_size_bytes -or
    $provider[0].provenance.canonical_payload_sha256 -cne
        $ownerProvenance.provenance.build.canonical_payload_sha256 -or
    [long] $provider[0].provenance.canonical_payload_size_bytes -ne
        [long] $ownerProvenance.provenance.build.canonical_payload_size_bytes -or
    [int] $provider[0].provenance.dependency_count -ne
        @($ownerProvenance.provenance.dependencies).Count -or
    [int] $provider[0].provenance.bundled_native_library_count -ne
        @($ownerProvenance.provenance.bundled_native_libraries).Count -or
    $provider[0].provenance.signing_state -ne
        $ownerProvenance.provenance.signing.state -or
    $provider[0].provenance.authorized_signer_thumbprint -ne
        $ownerProvenance.provenance.signing.authorized_thumbprint -or
    $provider[0].provenance.distribution_eligibility -ne
        $ownerProvenance.provenance.distribution.eligibility) {
    throw "Fleet projection does not preserve the Hostess owner provenance"
}

if ($manifest.build.kind -eq "signed-release") {
    if ($manifest.build.authenticode_required -ne $true -or
        $manifest.build.source_tree_clean -ne $true -or
        $ExpectedFleetSignerThumbprint -cnotmatch "^[0-9A-Fa-f]{40}$" -or
        $manifest.build.authorized_fleet_signer_thumbprint -cne
            $ExpectedFleetSignerThumbprint.ToUpperInvariant()) {
        throw "signed release does not require signatures and a clean source tree"
    }
    foreach ($component in $manifest.components) {
        $executable = Join-Path $bundlePath $component.entrypoint.Replace("/", "\")
        $signature = Get-AuthenticodeSignature -LiteralPath $executable
        if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
            throw "signed release component does not have a valid Authenticode signature"
        }
        if ($component.owner -eq "rusty-fleet" -and
            $signature.SignerCertificate.Thumbprint -cne
                $ExpectedFleetSignerThumbprint.ToUpperInvariant()) {
            throw "signed Fleet component does not match the authorized signer pin"
        }
    }
}

if ($manifest.install.activation -ne "explicit_operator_start" -or
    $manifest.install.authority -ne "RustyFleet-Setup.exe" -or
    $manifest.install.plan_protocol -ne "rusty.fleet.guided_installer_plan.v1" -or
    $manifest.install.service_registration -ne "absent" -or
    $manifest.update.strategy -ne "setup_owned_side_by_side_manifest" -or
    $manifest.update.rollback.supported -ne $true -or
    $manifest.update.rollback.mode -ne
        "setup_owned_pointer_to_previous_fully_verified_release") {
    throw "install, update, or rollback metadata is incomplete"
}
if (@($manifest.excluded_payload_classes) -notcontains "credentials" -or
    @($manifest.excluded_payload_classes) -notcontains "private_configuration" -or
    @($manifest.excluded_payload_classes) -notcontains "adb") {
    throw "required private and ADB exclusions are missing"
}
if ($manifestText -match "[A-Za-z]:\\" -or $manifestText -match "\\\\[^\\]") {
    throw "manifest contains a machine-local path"
}

$inventory = @($manifest.payload)
if ($inventory.Count -lt 6) {
    throw "bundle payload inventory is unexpectedly small"
}
$inventoryPaths = [System.Collections.Generic.HashSet[string]]::new(
    [StringComparer]::Ordinal
)
foreach ($item in $inventory) {
    $relativePath = [string] $item.path
    Assert-RustyFleetPayloadPath -RelativePath $relativePath
    if (-not $inventoryPaths.Add($relativePath)) {
        throw "duplicate payload path: $relativePath"
    }
    Assert-RustyFleetSha256 -Value $item.sha256 -Name "payload digest"
    $path = Join-Path $bundlePath $relativePath.Replace("/", "\")
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "manifest payload is missing: $relativePath"
    }
    if ((Get-RustyFleetSha256 -LiteralPath $path) -cne $item.sha256) {
        throw "payload digest mismatch: $relativePath"
    }
    if ((Get-Item -LiteralPath $path).Length -ne [long] $item.size_bytes) {
        throw "payload size mismatch: $relativePath"
    }
}

foreach ($component in $manifest.components) {
    if (-not $inventoryPaths.Contains([string] $component.entrypoint)) {
        throw "component entrypoint is not in the payload inventory: $($component.component_id)"
    }
}

$allowedPaths = [System.Collections.Generic.HashSet[string]]::new(
    [StringComparer]::Ordinal
)
foreach ($path in $inventoryPaths) {
    $allowedPaths.Add($path) | Out-Null
}
foreach ($metadata in @(
    "metadata/release-manifest.json",
    "metadata/checksums.sha256",
    "metadata/validation-receipt.json"
)) {
    $allowedPaths.Add($metadata) | Out-Null
}
$actualPaths = Get-ChildItem -LiteralPath $bundlePath -File -Recurse |
    ForEach-Object {
        Get-RustyFleetRelativePath -Root $bundlePath -LiteralPath $_.FullName
    }
foreach ($actualPath in $actualPaths) {
    if (-not $allowedPaths.Contains($actualPath)) {
        throw "unmanifested bundle file: $actualPath"
    }
}
if ($actualPaths.Count -ne $allowedPaths.Count) {
    throw "bundle composition does not exactly match its manifest"
}

$checksumEntries = @{}
foreach ($line in Get-Content -LiteralPath $checksumsPath) {
    if ($line -notmatch "^([0-9a-f]{64}) \*(.+)$") {
        throw "malformed checksum line"
    }
    if ($checksumEntries.ContainsKey($Matches[2])) {
        throw "duplicate checksum path"
    }
    $checksumEntries[$Matches[2]] = $Matches[1]
}
$expectedChecksumPaths = @($inventoryPaths) + @("metadata/release-manifest.json")
if ($checksumEntries.Count -ne $expectedChecksumPaths.Count) {
    throw "checksum inventory count does not match the manifest"
}
foreach ($relativePath in $expectedChecksumPaths) {
    $path = Join-Path $bundlePath $relativePath.Replace("/", "\")
    if (-not $checksumEntries.ContainsKey($relativePath) -or
        $checksumEntries[$relativePath] -cne (Get-RustyFleetSha256 -LiteralPath $path)) {
        throw "checksum file does not match: $relativePath"
    }
}

$receipt = Get-Content -LiteralPath $receiptPath -Raw |
    ConvertFrom-Json -Depth 20
if ($receipt.schema -ne "rusty.fleet.windows_distribution_validation_receipt.v1" -or
    $receipt.result -ne "pass" -or
    $receipt.version -ne $manifest.version -or
    $receipt.payload_files -ne $inventory.Count -or
    $receipt.runtime_components -ne $expectedComponentIds.Count -or
    $receipt.manifest_sha256 -cne (Get-RustyFleetSha256 -LiteralPath $manifestPath) -or
    $receipt.checksums_sha256 -cne (Get-RustyFleetSha256 -LiteralPath $checksumsPath) -or
    $receipt.hostess_owner_provenance_sha256 -cne
        $ownerProvenance.provenance_sha256 -or
    $receipt.distribution_eligibility -ne $manifest.distribution.eligibility -or
    $receipt.publication_allowed -ne $manifest.distribution.publication_allowed) {
    throw "distribution validation receipt is not bound to this bundle"
}

[ordered]@{
    schema = "rusty.fleet.windows_distribution_offline_check.v1"
    result = "pass"
    version = $manifest.version
    payload_files = $inventory.Count
    runtime_components = $componentIds.Count
    hostess_provider_sha256 = $provider[0].provenance.artifact_sha256
    exact_composition = $true
    private_payloads_absent = $true
} | ConvertTo-Json -Depth 10
