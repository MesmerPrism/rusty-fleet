# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
Import-Module (Join-Path $PSScriptRoot "..\Distribution.Common.psm1") -Force

function New-TestCertificate([string] $Subject, [bool] $IsCa = $false) {
    $key = [Security.Cryptography.RSA]::Create(2048)
    $request = [Security.Cryptography.X509Certificates.CertificateRequest]::new(
        $Subject,
        $key,
        [Security.Cryptography.HashAlgorithmName]::SHA256,
        [Security.Cryptography.RSASignaturePadding]::Pkcs1
    )
    [void] $request.CertificateExtensions.Add(
        [Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new(
            $IsCa,
            $false,
            0,
            $true
        )
    )
    $enhancedKeyUsages = [Security.Cryptography.OidCollection]::new()
    [void] $enhancedKeyUsages.Add(
        [Security.Cryptography.Oid]::new("1.3.6.1.5.5.7.3.3")
    )
    [void] $request.CertificateExtensions.Add(
        [Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new(
            $enhancedKeyUsages,
            $true
        )
    )
    return [pscustomobject]@{
        Key = $key
        Certificate = $request.CreateSelfSigned(
            [DateTimeOffset]::UtcNow.AddMinutes(-5),
            [DateTimeOffset]::UtcNow.AddDays(1)
        )
    }
}

function Get-CertificateSha256($Certificate) {
    [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData($Certificate.RawData)
    ).ToLowerInvariant()
}

function Get-RootStoreFingerprint {
    $entries = [Collections.Generic.List[string]]::new()
    foreach ($location in @(
        [Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser,
        [Security.Cryptography.X509Certificates.StoreLocation]::LocalMachine
    )) {
        $store = [Security.Cryptography.X509Certificates.X509Store]::new(
            [Security.Cryptography.X509Certificates.StoreName]::Root,
            $location
        )
        try {
            $store.Open(
                [Security.Cryptography.X509Certificates.OpenFlags]::ReadOnly
            )
            foreach ($certificate in $store.Certificates) {
                $entries.Add(
                    "$location|$($certificate.Thumbprint)|" +
                    (Get-CertificateSha256 $certificate)
                )
            }
        }
        finally {
            $store.Close()
            $store.Dispose()
        }
    }
    $canonical = ($entries | Sort-Object) -join "`n"
    return [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData(
            [Text.Encoding]::UTF8.GetBytes($canonical)
        )
    ).ToLowerInvariant()
}

function New-TestPolicy($Certificate) {
    [pscustomobject][ordered]@{
        subject = $Certificate.Subject
        thumbprint = $Certificate.Thumbprint.ToUpperInvariant()
        certificate_sha256 = Get-CertificateSha256 $Certificate
        self_issued = $true
        public_trust_claim = $false
        trust_mode = "exact-pinned-self-issued-untrusted-root-only"
        timestamp_required = $true
        allowed_chain_status_flags = @("UntrustedRoot")
    }
}

function New-TestSignature(
    $Certificate,
    $TimestampCertificate,
    [Management.Automation.SignatureStatus] $Status =
        [Management.Automation.SignatureStatus]::UnknownError
) {
    [pscustomobject]@{
        Status = $Status
        SignatureType = [Management.Automation.SignatureType]::Authenticode
        SignerCertificate = $Certificate
        TimeStamperCertificate = $TimestampCertificate
    }
}

function Assert-Rejected([scriptblock] $Action, [string] $Context) {
    $rejected = $false
    try { & $Action | Out-Null } catch { $rejected = $true }
    if (-not $rejected) { throw "$Context was accepted" }
}

$rootStoreBefore = Get-RootStoreFingerprint
$one = New-TestCertificate "CN=MesmerPrism"
$two = New-TestCertificate "CN=MesmerPrism"
$root = New-TestCertificate "CN=Synthetic Root" $true
$leafKey = [Security.Cryptography.RSA]::Create(2048)
$leafRequest = [Security.Cryptography.X509Certificates.CertificateRequest]::new(
    "CN=MesmerPrism",
    $leafKey,
    [Security.Cryptography.HashAlgorithmName]::SHA256,
    [Security.Cryptography.RSASignaturePadding]::Pkcs1
)
$serial = [byte[]](1..16)
$leaf = $leafRequest.Create(
    $root.Certificate,
    $root.Certificate.NotBefore,
    $root.Certificate.NotAfter.AddMinutes(-1),
    $serial
)
try {
    $policy = New-TestPolicy $one.Certificate
    $signature = New-TestSignature $one.Certificate $one.Certificate
    $assessment = Assert-RustyFleetAuthenticodeAssessment `
        -Signature $signature `
        -AuthenticodePolicy $policy `
        -Channel labs `
        -ObservedChainStatusFlags @("UntrustedRoot")
    if ($assessment.result -cne "pass" -or
        $assessment.authenticode_status -cne "unknown_error" -or
        $assessment.chain_trusted -ne $false -or
        $assessment.public_trust_claim -ne $false -or
        $assessment.trust_mode -cne
            "exact-pinned-self-issued-untrusted-root-only" -or
        $assessment.validation_boundary -cne
            "exact-pinned-self-issued-untrusted-root-only") {
        throw "exact pinned self-issued assessment did not preserve trust truth"
    }
    $locallyTrustedAssessment = Assert-RustyFleetAuthenticodeAssessment `
        -Signature (New-TestSignature `
            $one.Certificate `
            $one.Certificate `
            ([Management.Automation.SignatureStatus]::Valid)) `
        -AuthenticodePolicy $policy `
        -Channel labs `
        -ObservedChainStatusFlags @()
    if ($locallyTrustedAssessment.authenticode_status -cne "valid" -or
        $locallyTrustedAssessment.chain_trusted -ne $true -or
        @($locallyTrustedAssessment.chain_status_flags).Count -ne 0 -or
        $locallyTrustedAssessment.public_trust_claim -ne $false -or
        $locallyTrustedAssessment.validation_boundary -cne
            "host-chain-valid-no-public-trust-claim") {
        throw "locally trusted exact signer did not preserve non-public trust truth"
    }

    Assert-Rejected {
        Assert-RustyFleetAuthenticodeAssessment `
            -Signature (New-TestSignature $two.Certificate $one.Certificate) `
            -AuthenticodePolicy $policy `
            -Channel labs `
            -ObservedChainStatusFlags @("UntrustedRoot")
    } "wrong certificate bytes and thumbprint"

    $wrongThumb = New-TestPolicy $one.Certificate
    $wrongThumb.thumbprint = "0" * 40
    Assert-Rejected {
        Assert-RustyFleetAuthenticodeAssessment $signature $wrongThumb labs `
            @("UntrustedRoot")
    } "wrong policy thumbprint"

    $issuedPolicy = New-TestPolicy $leaf
    Assert-Rejected {
        Assert-RustyFleetAuthenticodeAssessment `
            -Signature (New-TestSignature $leaf $one.Certificate) `
            -AuthenticodePolicy $issuedPolicy `
            -Channel labs `
            -ObservedChainStatusFlags @("UntrustedRoot")
    } "non-self-issued signer"

    Assert-Rejected {
        Assert-RustyFleetAuthenticodeAssessment `
            -Signature (New-TestSignature $one.Certificate $null) `
            -AuthenticodePolicy $policy `
            -Channel labs `
            -ObservedChainStatusFlags @("UntrustedRoot")
    } "missing timestamp"

    $publicClaim = New-TestPolicy $one.Certificate
    $publicClaim.public_trust_claim = $true
    Assert-Rejected {
        Assert-RustyFleetAuthenticodeAssessment $signature $publicClaim labs `
            @("UntrustedRoot")
    } "Labs public-trust claim"

    Assert-Rejected {
        Assert-RustyFleetAuthenticodeAssessment $signature $policy labs @()
    } "wrong chain flag set"
    Assert-Rejected {
        Assert-RustyFleetAuthenticodeAssessment `
            (New-TestSignature `
                $one.Certificate `
                $one.Certificate `
                ([Management.Automation.SignatureStatus]::Valid)) `
            $policy `
            labs `
            @("UntrustedRoot")
    } "locally trusted status with an untrusted-root chain"
    Assert-Rejected {
        Assert-RustyFleetAuthenticodeAssessment $signature $policy labs `
            @("UntrustedRoot", "NotTimeValid")
    } "multiple chain flags"
    Assert-Rejected {
        Assert-RustyFleetAuthenticodeAssessment $signature $policy labs @() 2
    } "untrusted status with a clean multi-element chain"
    Assert-Rejected {
        Assert-RustyFleetAuthenticodeAssessment $signature $policy stable @()
    } "Labs policy substituted into Stable"

    $rootStoreAfter = Get-RootStoreFingerprint
    if ($rootStoreAfter -cne $rootStoreBefore) {
        throw "Windows Root certificate stores changed during trust validation"
    }
    [ordered]@{
        schema = "rusty.fleet.windows_authenticode_trust_test.v1"
        result = "pass"
        exact_pinned_self_issued_labs = $true
        local_valid_and_runner_untrusted_boundaries = $true
        public_trust_claim = $false
        timestamp_required = $true
        negative_cases = 10
        root_store_mutated = ($rootStoreAfter -cne $rootStoreBefore)
    } | ConvertTo-Json -Depth 5
}
finally {
    $leaf.Dispose()
    $leafKey.Dispose()
    $root.Certificate.Dispose()
    $root.Key.Dispose()
    $two.Certificate.Dispose()
    $two.Key.Dispose()
    $one.Certificate.Dispose()
    $one.Key.Dispose()
}
