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

foreach ($required in @(
    "e92c9e000246798748ccb208567f24f72398ed6bad21cc3d05fc81c38da34f56",
    "74b75142ba0e7a777eb8a01fbb8ceed5aeb7f9c744a8d3fb8ec2e0fc850c0431",
    "de1444187365b785f4ef74e24ccb40b10f34982f",
    "ee6faba86ef876f988e7b5ddaa552ee3a484f15b0e3704c79404f92b0bda9fc9",
    "validate_offline",
    "ContainedJob",
    "CREATE_SUSPENDED",
    "RetainedExecutable",
    "open_guarded_dir_component",
    "tool_path_identity_changed",
    "adversarial_ancestor_rename_recreate_is_blocked_through_process_creation",
    "machine-bound-developer-evidence",
    "not-portable-release-capsule",
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
    "machine-bound developer evidence",
    "not a portable or supported distribution artifact",
    "separately owner-issued release capsule",
    "Distribution work must remain blocked"
)) {
    Assert-True -Condition $onboardingDocsFlat.Contains($required, [StringComparison]::Ordinal) `
        -Message "Offline onboarding release-capsule quarantine is missing: $required"
}

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
$ownerHash = (
    Get-FileHash -LiteralPath $ownerFixture -Algorithm SHA256
).Hash.ToLowerInvariant()
Assert-True -Condition (
    $ownerHash -ceq
    "ee6faba86ef876f988e7b5ddaa552ee3a484f15b0e3704c79404f92b0bda9fc9"
) -Message "Pinned Rusty Quest owner-profile fixture changed."

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
