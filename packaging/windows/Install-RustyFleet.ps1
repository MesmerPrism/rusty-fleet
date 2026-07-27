# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [ValidateSet("Plan", "Install", "Rollback")]
    [string] $Action = "Plan",

    [string] $BundleRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
    [string] $InstallRoot = (Join-Path $env:LOCALAPPDATA "RustyFleet"),
    [switch] $Execute
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
Import-Module (Join-Path $PSScriptRoot "Distribution.Common.psm1") -Force

if ($Action -eq "Plan" -and $Execute) {
    throw "Plan never accepts -Execute; choose Install or Rollback explicitly"
}

$bundlePath = (Resolve-Path -LiteralPath $BundleRoot).Path
$installPath = [System.IO.Path]::GetFullPath($InstallRoot)
$installRootInfo = [System.IO.DirectoryInfo]::new($installPath)
if (-not $installRootInfo.Parent -or $installRootInfo.FullName.Length -lt 8) {
    throw "InstallRoot must be a dedicated child directory"
}

$validatorPath = Join-Path $PSScriptRoot "Test-WindowsBundle.ps1"
if (-not (Test-Path -LiteralPath $validatorPath -PathType Leaf)) {
    throw "the distribution validator is missing"
}
& $validatorPath -BundleRoot $bundlePath | Out-Null
$manifestPath = Join-Path $bundlePath "metadata\release-manifest.json"
$manifest = Get-Content -LiteralPath $manifestPath -Raw |
    ConvertFrom-Json -Depth 30
$manifestSha256 = Get-RustyFleetSha256 -LiteralPath $manifestPath

$statePath = Join-Path $installPath "state\current.json"
$state = $null
if (Test-Path -LiteralPath $statePath -PathType Leaf) {
    $state = Get-Content -LiteralPath $statePath -Raw |
        ConvertFrom-Json -Depth 20
    if ($state.schema -ne "rusty.fleet.windows_install_state.v1") {
        throw "existing install state has an unsupported schema"
    }
}

$current = if ($state) { $state.current } else { $null }
$history = @()
if ($state) {
    $history = @($state.history)
}
$effectiveAction = if ($Action -eq "Plan") { "Install" } else { $Action }
$targetVersion = $manifest.version
$targetManifestSha256 = $manifestSha256

if ($effectiveAction -eq "Rollback") {
    if ($history.Count -eq 0) {
        throw "there is no previous verified release to roll back to"
    }
    $rollback = $history[0]
    $targetVersion = [string] $rollback.version
    $targetManifestSha256 = [string] $rollback.manifest_sha256
    $rollbackRoot = Join-Path $installPath "releases\$targetVersion"
    if (-not (Test-Path -LiteralPath $rollbackRoot -PathType Container)) {
        throw "the previous release directory is missing"
    }
    & $validatorPath -BundleRoot $rollbackRoot -ExpectedVersion $targetVersion | Out-Null
    $rollbackManifest = Join-Path $rollbackRoot "metadata\release-manifest.json"
    if ((Get-RustyFleetSha256 -LiteralPath $rollbackManifest) -cne $targetManifestSha256) {
        throw "the previous release no longer matches installed rollback metadata"
    }
}

$plan = [ordered]@{
    schema = "rusty.fleet.windows_install_plan.v1"
    action = $effectiveAction.ToLowerInvariant()
    execute = [bool] $Execute
    install_mode = "per_user_side_by_side"
    target_version = $targetVersion
    target_manifest_sha256 = $targetManifestSha256
    current_version = if ($current) { $current.version } else { $null }
    previous_version = if ($history.Count -gt 0) { $history[0].version } else { $null }
    operations = if ($effectiveAction -eq "Install") {
        @(
            "verify source manifest, checksums, validation receipt, and exact payload",
            "copy into a version-specific staging directory",
            "verify the staged release before activation",
            "move the verified release into the side-by-side release store",
            "atomically update the current-version pointer",
            "preserve the previous verified release for rollback"
        )
    }
    else {
        @(
            "verify the previous installed release and its recorded manifest digest",
            "atomically switch only the current-version pointer",
            "preserve both releases without deleting files"
        )
    }
    side_effects_excluded = @(
        "no service registration",
        "no process launch",
        "no credential creation",
        "no private configuration creation",
        "no PATH mutation",
        "no ADB installation or invocation"
    )
}

if (-not $Execute) {
    $plan | ConvertTo-Json -Depth 20
    return
}

[System.IO.Directory]::CreateDirectory((Join-Path $installPath "releases")) | Out-Null
[System.IO.Directory]::CreateDirectory((Join-Path $installPath "state")) | Out-Null

if ($effectiveAction -eq "Install") {
    $releaseRoot = Join-Path $installPath "releases\$targetVersion"
    if (Test-Path -LiteralPath $releaseRoot) {
        & $validatorPath -BundleRoot $releaseRoot -ExpectedVersion $targetVersion | Out-Null
        $installedManifest = Join-Path $releaseRoot "metadata\release-manifest.json"
        if ((Get-RustyFleetSha256 -LiteralPath $installedManifest) -cne $targetManifestSha256) {
            throw "an installed release with this version has different bytes"
        }
    }
    else {
        $stagingRoot = Join-Path $installPath (
            ".staging-$targetVersion-$([Guid]::NewGuid().ToString('N'))"
        )
        try {
            [System.IO.Directory]::CreateDirectory($stagingRoot) | Out-Null
            Get-ChildItem -LiteralPath $bundlePath -Force |
                Copy-Item -Destination $stagingRoot -Recurse -Force
            & $validatorPath -BundleRoot $stagingRoot -ExpectedVersion $targetVersion | Out-Null
            Move-Item -LiteralPath $stagingRoot -Destination $releaseRoot
        }
        finally {
            if (Test-Path -LiteralPath $stagingRoot) {
                Remove-Item -LiteralPath $stagingRoot -Recurse -Force
            }
        }
    }

    $newHistory = @()
    if ($current -and
        ($current.version -ne $targetVersion -or
            $current.manifest_sha256 -ne $targetManifestSha256)) {
        $newHistory += $current
    }
    $newHistory += @($history | Where-Object {
        $_.version -ne $targetVersion -and
        -not ($current -and $_.version -eq $current.version)
    })
    $newCurrent = [ordered]@{
        version = $targetVersion
        manifest_sha256 = $targetManifestSha256
        release_path = "releases/$targetVersion"
    }
}
else {
    $newCurrent = [ordered]@{
        version = $history[0].version
        manifest_sha256 = $history[0].manifest_sha256
        release_path = $history[0].release_path
    }
    $newHistory = @()
    if ($current) {
        $newHistory += $current
    }
    if ($history.Count -gt 1) {
        $newHistory += $history[1..($history.Count - 1)]
    }
}

$newState = [ordered]@{
    schema = "rusty.fleet.windows_install_state.v1"
    current = $newCurrent
    history = @($newHistory)
    policy = [ordered]@{
        update = "side_by_side_manifest"
        rollback = "pointer_only_previous_verified_release"
        automatic_delete = $false
    }
}
$stateText = ConvertTo-RustyFleetJson -InputObject $newState
$stateTempPath = "$statePath.$([Guid]::NewGuid().ToString('N')).tmp"
Write-RustyFleetUtf8 -LiteralPath $stateTempPath -Content $stateText
Move-Item -LiteralPath $stateTempPath -Destination $statePath -Force

[ordered]@{
    schema = "rusty.fleet.windows_install_result.v1"
    result = "pass"
    action = $effectiveAction.ToLowerInvariant()
    current = $newCurrent
    rollback_available = @($newHistory).Count -gt 0
    processes_started = 0
    services_registered = 0
} | ConvertTo-Json -Depth 20
