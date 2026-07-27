# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$distributionTest = Join-Path $PSScriptRoot (
    "..\packaging\windows\tests\Test-WindowsDistribution.ps1"
)
if (-not (Test-Path -LiteralPath $distributionTest -PathType Leaf)) {
    throw "Windows distribution validation entrypoint is missing"
}

& $distributionTest
