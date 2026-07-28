# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Hosted Windows runners cannot silently add an ephemeral root to the protected
# CurrentUser Root store. Keep the real Authenticode operation, but project the
# exact untrusted-root-only result as trusted only after binding the unchanged
# signed bytes and signer thumbprint. Every other status or byte change remains
# the built-in fail-closed result.
$script:ApprovedSignatureHashes = @{}

function New-RustyFleetTestCodeSigningCertificate {
    param(
        [Parameter(Mandatory)]
        [string] $Subject
    )

    $key = [Security.Cryptography.RSA]::Create(3072)
    try {
        $request = [Security.Cryptography.X509Certificates.CertificateRequest]::new(
            $Subject,
            $key,
            [Security.Cryptography.HashAlgorithmName]::SHA256,
            [Security.Cryptography.RSASignaturePadding]::Pkcs1
        )
        [void] $request.CertificateExtensions.Add(
            [Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new(
                [Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature,
                $true
            )
        )
        $enhancedKeyUsages = (
            [Security.Cryptography.OidCollection]::new()
        )
        [void] $enhancedKeyUsages.Add(
            [Security.Cryptography.Oid]::new("1.3.6.1.5.5.7.3.3")
        )
        [void] $request.CertificateExtensions.Add(
            [Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new(
                $enhancedKeyUsages,
                $true
            )
        )
        [void] $request.CertificateExtensions.Add(
            [Security.Cryptography.X509Certificates.X509SubjectKeyIdentifierExtension]::new(
                $request.PublicKey,
                $false
            )
        )
        return $request.CreateSelfSigned(
            [DateTimeOffset]::UtcNow.AddMinutes(-5),
            [DateTimeOffset]::UtcNow.AddDays(1)
        )
    }
    finally {
        $key.Dispose()
    }
}

function Test-RustyFleetUntrustedRootSignature {
    param(
        [Parameter(Mandatory)]
        [Management.Automation.Signature] $Signature
    )

    $untrustedRootMessage = (
        [ComponentModel.Win32Exception]::new(-2146762487).Message
    )
    return (
        $Signature.Status -eq
            [Management.Automation.SignatureStatus]::UnknownError -and
        $Signature.StatusMessage -ceq $untrustedRootMessage
    )
}

function Get-RustyFleetTestSha256([string] $LiteralPath) {
    $stream = [IO.File]::Open(
        $LiteralPath,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::Read
    )
    try {
        return [Convert]::ToHexString(
            [Security.Cryptography.SHA256]::HashData($stream)
        ).ToLowerInvariant()
    }
    finally {
        $stream.Dispose()
    }
}

function Register-RustyFleetTestAuthenticodeSignature {
    param(
        [Parameter(Mandatory)]
        [string] $LiteralPath,

        [Parameter(Mandatory)]
        [Security.Cryptography.X509Certificates.X509Certificate2] $Certificate
    )

    $signature = (
        Microsoft.PowerShell.Security\Get-AuthenticodeSignature `
            -LiteralPath $LiteralPath
    )
    if (
        $signature.SignatureType -ne
            [Management.Automation.SignatureType]::Authenticode -or
        $null -eq $signature.SignerCertificate -or
        $signature.SignerCertificate.Thumbprint -cne $Certificate.Thumbprint -or
        (
            $signature.Status -ne
                [Management.Automation.SignatureStatus]::Valid -and
            -not (Test-RustyFleetUntrustedRootSignature -Signature $signature)
        )
    ) {
        throw "generated test Authenticode signature is not cryptographically bound"
    }
    $sha256 = Get-RustyFleetTestSha256 -LiteralPath $LiteralPath
    $script:ApprovedSignatureHashes[$sha256] = (
        $Certificate.Thumbprint.ToUpperInvariant()
    )
}

function Get-RustyFleetTestAuthenticodeSignature {
    param(
        [Parameter(Mandatory)]
        [string] $LiteralPath
    )

    $signature = (
        Microsoft.PowerShell.Security\Get-AuthenticodeSignature `
            -LiteralPath $LiteralPath
    )
    if (
        $signature.SignatureType -eq
            [Management.Automation.SignatureType]::Authenticode -and
        $null -ne $signature.SignerCertificate
    ) {
        $sha256 = Get-RustyFleetTestSha256 -LiteralPath $LiteralPath
        $expectedThumbprint = $script:ApprovedSignatureHashes[$sha256]
        if (
            -not [string]::IsNullOrWhiteSpace($expectedThumbprint) -and
            $signature.SignerCertificate.Thumbprint.ToUpperInvariant() -ceq
                $expectedThumbprint -and
            (
                $signature.Status -eq
                    [Management.Automation.SignatureStatus]::Valid -or
                (Test-RustyFleetUntrustedRootSignature -Signature $signature)
            )
        ) {
            return [pscustomobject]@{
                Status = [Management.Automation.SignatureStatus]::Valid
                StatusMessage = "Exact test fixture signature verified"
                Path = $signature.Path
                SignatureType = $signature.SignatureType
                IsOSBinary = $signature.IsOSBinary
                SignerCertificate = $signature.SignerCertificate
                TimeStamperCertificate = $signature.TimeStamperCertificate
            }
        }
    }
    return $signature
}

Export-ModuleMember -Function @(
    "New-RustyFleetTestCodeSigningCertificate",
    "Register-RustyFleetTestAuthenticodeSignature",
    "Get-RustyFleetTestAuthenticodeSignature"
)
