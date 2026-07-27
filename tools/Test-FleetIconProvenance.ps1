# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
Import-Module (Join-Path $PSScriptRoot "FleetIconProvenance.psm1") -Force

$expected = "1dedfecaef954dda9bb6f4f133376535e4799908441e7832558a1f70f4ed6f79"
$source = [IO.File]::ReadAllText(
    (Join-Path $repoRoot "assets\branding\rusty-fleet.svg"))
$lf = ConvertTo-FleetCanonicalText -Text $source
$crlf = $lf.Replace("`n", "`r`n", [StringComparison]::Ordinal)
$cr = $lf.Replace("`n", "`r", [StringComparison]::Ordinal)

$hashes = @(
    Get-FleetCanonicalTextSha256 -Text $lf
    Get-FleetCanonicalTextSha256 -Text $crlf
    Get-FleetCanonicalTextSha256 -Text $cr
)
if (@($hashes | Sort-Object -Unique).Count -ne 1 -or
    $hashes[0] -cne $expected) {
    throw "Fleet SVG canonical hashing is not stable across line endings."
}

Write-Output "Rusty Fleet icon provenance line-ending tests passed."
