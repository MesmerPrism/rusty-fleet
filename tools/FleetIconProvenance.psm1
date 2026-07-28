# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function ConvertTo-FleetCanonicalText {
    [CmdletBinding()]
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Text)

    return $Text.Replace(
        "`r`n", "`n", [StringComparison]::Ordinal).Replace(
        "`r", "`n", [StringComparison]::Ordinal)
}

function Get-FleetCanonicalTextSha256 {
    [CmdletBinding()]
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Text)

    $canonical = ConvertTo-FleetCanonicalText -Text $Text
    $bytes = [Text.UTF8Encoding]::new($false, $true).GetBytes($canonical)
    try {
        return [Convert]::ToHexString(
            [Security.Cryptography.SHA256]::HashData($bytes)
        ).ToLowerInvariant()
    } finally {
        [Array]::Clear($bytes, 0, $bytes.Length)
    }
}

Export-ModuleMember -Function @(
    "ConvertTo-FleetCanonicalText",
    "Get-FleetCanonicalTextSha256"
)
