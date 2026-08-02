# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $CapsuleRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$validator = Join-Path $PSScriptRoot "Test-FleetAgentKeyRecordOwnerRelease.ps1"
$basePin = Join-Path $repoRoot "config\fleet-agent-key-record-owner-release.v1.json"
$matrix = Get-Content -Raw -LiteralPath (
    Join-Path $repoRoot "fixtures\onboarding\key-record-owner-release-scenarios.damaged.json") |
    ConvertFrom-Json
if ($matrix.schema -cne
        "rusty.fleet.fleet_agent_key_record_owner_release_damage_matrix.v1" -or
    @($matrix.cases).Count -ne 15) {
    throw "Fleet owner release damage matrix is incomplete"
}

function Write-Json([string] $LiteralPath, $Value) {
    $json = ($Value | ConvertTo-Json -Depth 30) -replace "`r`n", "`n"
    [IO.File]::WriteAllText(
        $LiteralPath,
        $json.TrimEnd("`r", "`n") + "`n",
        [Text.UTF8Encoding]::new($false))
}

function Invoke-MustReject([string] $Name, [scriptblock] $Mutate) {
    $temp = Join-Path ([IO.Path]::GetTempPath()) (
        "rusty-fleet-owner-release-selftest-" + [guid]::NewGuid().ToString("N"))
    $capsule = Join-Path $temp "capsule"
    $pin = Join-Path $temp "pin.json"
    try {
        [IO.Directory]::CreateDirectory($temp) | Out-Null
        Copy-Item -LiteralPath $CapsuleRoot -Destination $capsule -Recurse
        Copy-Item -LiteralPath $basePin -Destination $pin
        & $Mutate $capsule $pin
        & pwsh -NoProfile -ExecutionPolicy Bypass -File $validator `
            -CapsuleRoot $capsule -PinPath $pin *> $null
        if ($LASTEXITCODE -eq 0) { throw "damage case unexpectedly passed: $Name" }
    }
    finally {
        if (Test-Path -LiteralPath $temp) {
            Remove-Item -LiteralPath $temp -Recurse -Force
        }
    }
}

& pwsh -NoProfile -ExecutionPolicy Bypass -File $validator `
    -CapsuleRoot $CapsuleRoot -PinPath $basePin *> $null
if ($LASTEXITCODE -ne 0) { throw "valid owner release did not pass self-test preflight" }
& pwsh -NoProfile -ExecutionPolicy Bypass -File $validator -PolicySelfTest *> $null
if ($LASTEXITCODE -ne 0) { throw "owner executable policy self-test failed" }

Invoke-MustReject "wrong-owner" {
    param($capsule, $pin)
    $value = Get-Content -Raw $pin | ConvertFrom-Json
    $value.owner_id = "wrong-owner"
    Write-Json $pin $value
}
Invoke-MustReject "wrong-fleet-consumer" {
    param($capsule, $pin)
    $value = Get-Content -Raw $pin | ConvertFrom-Json
    $value.consumer_id = "rusty-fleet/wrong-consumer"
    Write-Json $pin $value
}
Invoke-MustReject "capsule-substitution" {
    param($capsule, $pin)
    [IO.File]::AppendAllText((Join-Path $capsule "fleet-agent-key-record.exe"), "damage")
}
Invoke-MustReject "duplicate-capsule" {
    param($capsule, $pin)
    Copy-Item (Join-Path $capsule "release-manifest.json") `
        (Join-Path $capsule "release-manifest-copy.json")
}
Invoke-MustReject "stale-source-revision" {
    param($capsule, $pin)
    $path = Join-Path $capsule "release-manifest.json"
    $value = Get-Content -Raw $path | ConvertFrom-Json
    $value.source.commit = "0" * 40
    Write-Json $path $value
}
Invoke-MustReject "stale-provenance" {
    param($capsule, $pin)
    [IO.File]::AppendAllText((Join-Path $capsule "provenance.json"), " ")
}
Invoke-MustReject "secret-leakage" {
    param($capsule, $pin)
    [IO.File]::WriteAllText((Join-Path $capsule "signing-seed.bin"), "forbidden")
}
Invoke-MustReject "extra-repository" {
    param($capsule, $pin)
    $path = Join-Path $capsule "provenance.json"
    $value = Get-Content -Raw $path | ConvertFrom-Json
    $value.source.repositories = @($value.source.repositories) + $value.source.repositories[0]
    Write-Json $path $value
}
Invoke-MustReject "extra-package-file" {
    param($capsule, $pin)
    [IO.File]::WriteAllText((Join-Path $capsule "unexpected.bin"), "forbidden")
}
Invoke-MustReject "extra-parse-only-repository" {
    param($capsule, $pin)
    $path = Join-Path $capsule "provenance.json"
    $value = Get-Content -Raw $path | ConvertFrom-Json
    $value.source.workspace_parse_only_repositories =
        @($value.source.workspace_parse_only_repositories) +
        $value.source.workspace_parse_only_repositories[0]
    Write-Json $path $value
}
Invoke-MustReject "non-reproducible-provenance" {
    param($capsule, $pin)
    $path = Join-Path $capsule "provenance.json"
    $value = Get-Content -Raw $path | ConvertFrom-Json
    $value.build.pe_reproducibility_marker = "absent"
    Write-Json $path $value
}
Invoke-MustReject "license-substitution" {
    param($capsule, $pin)
    [IO.File]::AppendAllText((Join-Path $capsule "LICENSE"), "damage")
}
Invoke-MustReject "source-notice-substitution" {
    param($capsule, $pin)
    [IO.File]::AppendAllText((Join-Path $capsule "SOURCE-NOTICE.md"), "damage")
}

Write-Output "Rusty Fleet key-record owner release self-test passed"
