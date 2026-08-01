# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string[]] $ExecutablePath,

    [Parameter(Mandatory)]
    [string] $PfxPath,

    [Parameter(Mandatory)]
    [securestring] $PfxPassword,

    [Parameter(Mandatory)]
    [ValidateSet("labs", "stable")]
    [string] $Channel,

    [string] $ReleasePolicyPath = (
        Join-Path $PSScriptRoot "trust\release-policy.json"
    )
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
Import-Module (Join-Path $PSScriptRoot "Distribution.Common.psm1") -Force

# DigiCert's RFC 3161 SignTool endpoint is intentionally HTTP. SignTool
# validates the signed RFC 3161 transaction, and Fleet rejects a missing
# embedded timestamp after signing.
$timestampUrl = "http://timestamp.digicert.com"

$channelPolicy = Read-RustyFleetReleaseTrustPolicy `
    -LiteralPath (Resolve-Path -LiteralPath $ReleasePolicyPath).Path `
    -Channel $Channel
if ($channelPolicy.publication_enabled -ne $true) {
    throw "$Channel release signing is disabled by the revisioned trust policy"
}

$importedCertificates = @()
$certificate = $null
try {
    $importedCertificates = @(
        Import-PfxCertificate `
            -FilePath $PfxPath `
            -CertStoreLocation "Cert:\CurrentUser\My" `
            -Password $PfxPassword `
            -Exportable:$false
    )
    if ($importedCertificates.Count -ne 1) {
        throw "signing PFX must import exactly one certificate"
    }
    $certificate = $importedCertificates[0]
    $certificateSha256 = [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData($certificate.RawData)
    ).ToLowerInvariant()
    if ($certificate.Subject -cne $channelPolicy.authenticode.subject -or
        $certificate.Thumbprint.ToUpperInvariant() -cne
            $channelPolicy.authenticode.thumbprint -or
        $certificateSha256 -cne
            $channelPolicy.authenticode.certificate_sha256) {
        throw "PFX does not contain the exact channel-authorized signer certificate"
    }
    $signTool = Get-ChildItem `
        -Path "${env:ProgramFiles(x86)}\Windows Kits\10\bin" `
        -Filter "signtool.exe" `
        -File `
        -Recurse |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if (-not $signTool) {
        throw "signtool.exe is not available"
    }

    foreach ($path in $ExecutablePath) {
        $resolved = (Resolve-Path -LiteralPath $path).Path
        & $signTool.FullName sign `
            /sha1 $certificate.Thumbprint `
            /fd SHA256 `
            /td SHA256 `
            /tr $timestampUrl `
            $resolved
        if ($LASTEXITCODE -ne 0) {
            throw "Authenticode signing failed: $(Split-Path -Leaf $resolved)"
        }
        Get-RustyFleetAuthenticodeAssessment `
            -LiteralPath $resolved `
            -AuthenticodePolicy $channelPolicy.authenticode `
            -Channel $Channel | Out-Null
    }
}
finally {
    foreach ($importedCertificate in $importedCertificates) {
        Remove-Item `
            -LiteralPath (
                "Cert:\CurrentUser\My\$($importedCertificate.Thumbprint)"
            ) `
            -Force `
            -ErrorAction SilentlyContinue
    }
}
