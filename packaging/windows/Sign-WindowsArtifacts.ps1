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

    [string] $TimestampUrl = "https://timestamp.digicert.com"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$certificate = Import-PfxCertificate `
    -FilePath $PfxPath `
    -CertStoreLocation "Cert:\CurrentUser\My" `
    -Password $PfxPassword `
    -Exportable:$false
try {
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
            /tr $TimestampUrl `
            $resolved
        if ($LASTEXITCODE -ne 0) {
            throw "Authenticode signing failed: $(Split-Path -Leaf $resolved)"
        }
        $signature = Get-AuthenticodeSignature -LiteralPath $resolved
        if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
            throw "signed artifact did not verify: $(Split-Path -Leaf $resolved)"
        }
    }
}
finally {
    if ($certificate) {
        Remove-Item -LiteralPath "Cert:\CurrentUser\My\$($certificate.Thumbprint)" -Force
    }
}
