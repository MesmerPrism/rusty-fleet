# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))

function Assert-True {
    param(
        [Parameter(Mandatory)]
        [bool] $Condition,

        [Parameter(Mandatory)]
        [string] $Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

$enginePath = Join-Path $repoRoot "crates/fleet-onboarding/src/lib.rs"
$cliPath = Join-Path $repoRoot "apps/fleet-onboard/src/main.rs"
$onboardingDocsPath = Join-Path $repoRoot "docs/OFFLINE_ONBOARDING.md"
$engine = Get-Content -LiteralPath $enginePath -Raw
$cli = Get-Content -LiteralPath $cliPath -Raw
$onboardingDocs = Get-Content -LiteralPath $onboardingDocsPath -Raw
$onboardingDocsFlat = $onboardingDocs -replace "\s+", " "
$ownerReleasePinPath = Join-Path $repoRoot "config/fleet-agent-key-record-owner-release.v1.json"
$ownerReleaseValidatorPath = Join-Path $repoRoot "tools/Test-FleetAgentKeyRecordOwnerRelease.ps1"
$ownerReleaseSelfTestPath = Join-Path $repoRoot "tools/Test-FleetAgentKeyRecordOwnerReleaseSelfTest.ps1"
$ownerReleasePin = Get-Content -Raw -LiteralPath $ownerReleasePinPath | ConvertFrom-Json -Depth 30
$ownerReleaseValidator = Get-Content -Raw -LiteralPath $ownerReleaseValidatorPath
$ownerReleaseSelfTest = Get-Content -Raw -LiteralPath $ownerReleaseSelfTestPath
$ownerReleaseValid = Get-Content -Raw -LiteralPath (
    Join-Path $repoRoot "fixtures/onboarding/key-record-owner-release-scenarios.valid.json") |
    ConvertFrom-Json
$ownerReleaseDamaged = Get-Content -Raw -LiteralPath (
    Join-Path $repoRoot "fixtures/onboarding/key-record-owner-release-scenarios.damaged.json") |
    ConvertFrom-Json

foreach ($required in @(
    "1e3ae5456d7fa77a3733fa7df023f0b2ddda72b946e66fb3d1f1acdd6214680f",
    "c5ed362dffbe3701e051672ecaaed86902a9f0881d3f179ff47fa02ff07afeae",
    "e958d459025bf52fb8c6214ba41981db89f2b7d2832b3a222dbe30943367b0f9",
    "4a982368a29d41c3ecde8083b4aefc1e1bb1a4dc",
    "ee6faba86ef876f988e7b5ddaa552ee3a484f15b0e3704c79404f92b0bda9fc9",
    "validate_offline",
    "ContainedJob",
    "CREATE_SUSPENDED",
    "RetainedExecutable",
    "open_guarded_dir_component",
    "tool_path_identity_changed",
    "adversarial_ancestor_rename_recreate_is_blocked_through_process_creation",
    "supported-owner-release",
    "not-live-onboarding-proof",
    "not-manifold-acceptance",
    "delete_retained_handle",
    "partial_generation_exact_cleanup_required"
)) {
    Assert-True -Condition $engine.Contains($required, [StringComparison]::Ordinal) `
        -Message "Offline onboarding security binding is missing: $required"
}

foreach ($forbidden in @(
    "remove_dir_all(",
    "taskkill",
    "wait_with_output",
    "Command::new(`"cmd`")",
    "Command::new(`"powershell`")"
)) {
    Assert-True -Condition (-not $engine.Contains($forbidden, [StringComparison]::OrdinalIgnoreCase)) `
        -Message "Offline onboarding reintroduced a forbidden path/process primitive: $forbidden"
}

foreach ($required in @(
    "Planning and owner-release trust evidence",
    'consumer identity `rusty-fleet/fleet-onboard`',
    'owner identity `rusty-quest`',
    "owner_signature.present=false",
    "Capsule validity is packaging and helper provenance only",
    "Manifold remains the live enrollment and peer authority"
)) {
    Assert-True -Condition $onboardingDocsFlat.Contains($required, [StringComparison]::Ordinal) `
        -Message "Offline onboarding release-capsule quarantine is missing: $required"
}

Assert-True -Condition (
    $ownerReleasePin.schema -ceq "rusty.fleet.fleet_agent_key_record_owner_release_pin.v1" -and
    $ownerReleasePin.owner_id -ceq "rusty-quest" -and
    $ownerReleasePin.consumer_id -ceq "rusty-fleet/fleet-onboard" -and
    $ownerReleasePin.owner_signature.present -eq $false -and
    $ownerReleasePin.claims.capsule_validity -ceq "packaging-and-tool-provenance-only" -and
    $ownerReleasePin.claims.onboarding_accepted -eq $false -and
    $ownerReleasePin.claims.live_authority -ceq "rusty-manifold" -and
    @($ownerReleasePin.payload).Count -eq 4
) -Message "Fleet owner release pin does not preserve the reviewed owner/consumer boundary."
foreach ($required in @(
    "Assert-ExactProperties",
    "owner capsule contains an extra or missing file",
    "owner capsule provenance does not match the supported Fleet pin",
    "owner capsule contains prohibited private or machine-local material",
    "packaging-and-tool-provenance-only",
    "onboarding_accepted = `$false")) {
    Assert-True -Condition $ownerReleaseValidator.Contains($required, [StringComparison]::Ordinal) `
        -Message "Fleet owner release validator is missing: $required"
}
Assert-True -Condition (
    $ownerReleaseValid.schema -ceq
        "rusty.fleet.fleet_agent_key_record_owner_release_fixture_matrix.v1" -and
    $ownerReleaseValid.onboarding_accepted -eq $false -and
    $ownerReleaseDamaged.schema -ceq
        "rusty.fleet.fleet_agent_key_record_owner_release_damage_matrix.v1" -and
    @($ownerReleaseDamaged.cases).Count -eq 9 -and
    @($ownerReleaseDamaged.existing_onboarding_negatives) -contains "duplicate-device-id" -and
    @($ownerReleaseDamaged.existing_onboarding_negatives) -contains "duplicate-key-id" -and
    @($ownerReleaseDamaged.existing_onboarding_negatives) -contains "extra-private-inventory-file" -and
    $ownerReleaseSelfTest.Contains("wrong-fleet-consumer", [StringComparison]::Ordinal) -and
    $ownerReleaseSelfTest.Contains("duplicate-capsule", [StringComparison]::Ordinal) -and
    $ownerReleaseSelfTest.Contains("extra-repository", [StringComparison]::Ordinal)
) -Message "Fleet owner release valid/damaged fixture matrix is incomplete."

$verbs = @(
    "validate-tool",
    "plan",
    "apply",
    "cleanup-plan",
    "cleanup-apply",
    "revoke-plan"
)
foreach ($verb in $verbs) {
    Assert-True -Condition $cli.Contains($verb, [StringComparison]::Ordinal) `
        -Message "Standalone onboarding CLI is missing required verb: $verb"
}
$actualVerbs = @(
    [regex]::Matches($cli, '\("(?<verb>[a-z-]+)",\s*\d+\)') |
        ForEach-Object { $_.Groups["verb"].Value } |
        Sort-Object -Unique
)
$expectedVerbs = @($verbs | Sort-Object)
Assert-True -Condition (
    ($actualVerbs -join ",") -ceq ($expectedVerbs -join ",")
) -Message "Standalone onboarding CLI authority expanded beyond the six reviewed verbs."

$ownerFixture = Join-Path $repoRoot (
    "fixtures/onboarding/" +
    "rusty-quest-fleet-agent-profile.disabled.owner-de144.json")
$ownerText = [IO.File]::ReadAllText($ownerFixture).
    Replace("`r`n", "`n").
    Replace("`r", "`n")
$ownerHash = [Convert]::ToHexString(
    [Security.Cryptography.SHA256]::HashData(
        [Text.UTF8Encoding]::new($false).GetBytes($ownerText))
).ToLowerInvariant()
Assert-True -Condition (
    $ownerHash -ceq
    "ee6faba86ef876f988e7b5ddaa552ee3a484f15b0e3704c79404f92b0bda9fc9"
) -Message "Pinned canonical Rusty Quest owner-profile fixture changed."

$requestSchema = Get-Content -LiteralPath (
    Join-Path $repoRoot "schemas/rusty.fleet.offline_onboarding_request.v1.schema.json"
) -Raw | ConvertFrom-Json -Depth 100
Assert-True -Condition (
    $requestSchema.additionalProperties -eq $false -and
    -not $requestSchema.properties.PSObject.Properties.Name.Contains("tool_manifest_sha256")
) -Message "The request must locate the manifest without choosing its trust hash."

$inventorySchema = Get-Content -LiteralPath (
    Join-Path $repoRoot "schemas/rusty.fleet.offline_onboarding_private_inventory.v1.schema.json"
) -Raw | ConvertFrom-Json -Depth 100
Assert-True -Condition (
    @($inventorySchema.required) -contains "root_identity" -and
    @($inventorySchema.required) -contains "entries" -and
    $inventorySchema.properties.entries.items.additionalProperties -eq $false
) -Message "Private inventory must retain its closed object-identity contract."

Push-Location -LiteralPath $repoRoot
try {
    & cargo test --locked -p fleet-onboarding
    if ($LASTEXITCODE -ne 0) {
        throw "Fleet onboarding security tests failed."
    }
    & cargo test --locked -p fleet-hub-local offline_validator
    if ($LASTEXITCODE -ne 0) {
        throw "Fleet Hub offline-validator conformance tests failed."
    }
}
finally {
    Pop-Location
}

[ordered]@{
    schema = "rusty.fleet.offline_onboarding_security_validation.v1"
    cli_verbs = $verbs
    owner_fixture_sha256 = $ownerHash
    result = "pass"
} | ConvertTo-Json -Compress
