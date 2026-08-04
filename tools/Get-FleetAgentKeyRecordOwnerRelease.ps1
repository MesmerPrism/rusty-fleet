# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $OutputDirectory,

    [switch] $Execute
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$pinPath = Join-Path $repoRoot "config\fleet-agent-key-record-owner-release.v1.json"
$validatorPath = Join-Path $PSScriptRoot "Test-FleetAgentKeyRecordOwnerRelease.ps1"
$pin = Get-Content -Raw -LiteralPath $pinPath | ConvertFrom-Json -Depth 30

if ($pin.schema -cne "rusty.fleet.fleet_agent_key_record_owner_release_pin.v1" -or
    $pin.owner_id -cne "rusty-quest" -or
    $pin.owner_repository -cne "https://github.com/MesmerPrism/rusty-quest" -or
    $pin.consumer_id -cne "rusty-fleet/fleet-onboard" -or
    $pin.capsule_version -cnotmatch '^[0-9]+\.[0-9]+\.[0-9]+$') {
    throw "Fleet key-record owner release pin is not supported"
}

$payload = @{}
foreach ($entry in @($pin.payload)) {
    if ($payload.ContainsKey([string] $entry.path)) {
        throw "Fleet key-record owner release pin has a duplicate payload path"
    }
    $payload[[string] $entry.path] = $entry
}

$expected = [ordered]@{
    "checksums.sha256" = [pscustomobject]@{
        sha256 = [string] $pin.checksums_sha256
        size_bytes = [long] $pin.checksums_size_bytes
    }
    "fleet-agent-key-record.exe" = $payload["fleet-agent-key-record.exe"]
    "LICENSE" = $payload["LICENSE"]
    "provenance.json" = $payload["provenance.json"]
    "release-manifest.json" = [pscustomobject]@{
        sha256 = [string] $pin.manifest_sha256
        size_bytes = [long] $pin.manifest_size_bytes
    }
    "SOURCE-NOTICE.md" = $payload["SOURCE-NOTICE.md"]
}

if ($payload.Count -ne 4 -or
    @($expected.GetEnumerator() | Where-Object {
        $null -eq $_.Value -or
        [string] $_.Value.sha256 -cnotmatch '^[0-9a-f]{64}$' -or
        [long] $_.Value.size_bytes -le 0
    }).Count -ne 0) {
    throw "Fleet key-record owner release pin has an invalid closed inventory"
}

$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
$outputParent = Split-Path -Parent $outputPath
if ([string]::IsNullOrWhiteSpace($outputParent) -or
    $outputPath -ceq [IO.Path]::GetPathRoot($outputPath)) {
    throw "OutputDirectory must name a dedicated child directory"
}

$releaseTag = "fleet-agent-key-record-v$($pin.capsule_version)"
$releaseBaseUrl = "$($pin.owner_repository)/releases/download/$releaseTag"
$files = @($expected.GetEnumerator() | ForEach-Object {
    [ordered]@{
        name = $_.Key
        url = "$releaseBaseUrl/$($_.Key)"
        sha256 = [string] $_.Value.sha256
        size_bytes = [long] $_.Value.size_bytes
    }
})

if (-not $Execute) {
    [ordered]@{
        schema = "rusty.fleet.owner_capsule_fetch_plan.v1"
        result = "planned"
        mutates = $false
        owner_id = [string] $pin.owner_id
        consumer_id = [string] $pin.consumer_id
        release_tag = $releaseTag
        output_directory = $outputPath
        files = $files
    } | ConvertTo-Json -Depth 8
    exit 0
}

if (Test-Path -LiteralPath $outputPath) {
    throw "OutputDirectory already exists"
}
[IO.Directory]::CreateDirectory($outputParent) | Out-Null
$candidate = Join-Path $outputParent (
    "." + [IO.Path]::GetFileName($outputPath) + ".candidate-" +
    [guid]::NewGuid().ToString("N")
)

try {
    [IO.Directory]::CreateDirectory($candidate) | Out-Null
    foreach ($file in $files) {
        $destination = Join-Path $candidate $file.name
        Invoke-WebRequest -Uri $file.url -OutFile $destination
        $item = Get-Item -LiteralPath $destination
        $sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $destination).
            Hash.ToLowerInvariant()
        if ($item.Length -ne [long] $file.size_bytes -or
            $sha256 -cne [string] $file.sha256) {
            throw "downloaded owner capsule file does not match the Fleet pin: $($file.name)"
        }
    }

    $validationLines = @(& pwsh -NoProfile -ExecutionPolicy Bypass -File `
        $validatorPath -CapsuleRoot $candidate)
    if ($LASTEXITCODE -ne 0) {
        throw "downloaded owner capsule failed the Fleet validator"
    }
    $validation = ($validationLines -join [Environment]::NewLine) |
        ConvertFrom-Json -Depth 30
    if ($validation.status -cne "pass" -or
        $validation.owner_id -cne "rusty-quest" -or
        $validation.consumer_id -cne "rusty-fleet/fleet-onboard" -or
        $validation.capsule_validity -cne
            "packaging-and-tool-provenance-only" -or
        $validation.onboarding_accepted -ne $false) {
        throw "downloaded owner capsule validation receipt is not exact"
    }

    [IO.Directory]::Move($candidate, $outputPath)
    [ordered]@{
        schema = "rusty.fleet.owner_capsule_fetch_receipt.v1"
        result = "pass"
        owner_id = [string] $pin.owner_id
        consumer_id = [string] $pin.consumer_id
        release_tag = $releaseTag
        output_directory = $outputPath
        manifest_sha256 = [string] $pin.manifest_sha256
        executable_sha256 = [string] $pin.executable_sha256
        source_commit = [string] $pin.source_commit
        source_tree = [string] $pin.source_tree
        files = $files
        claims = [ordered]@{
            capsule_validity = "packaging-and-tool-provenance-only"
            onboarding_accepted = $false
            live_authority = "rusty-manifold"
        }
    } | ConvertTo-Json -Depth 8
}
finally {
    if (Test-Path -LiteralPath $candidate) {
        Remove-Item -LiteralPath $candidate -Recurse -Force
    }
}
