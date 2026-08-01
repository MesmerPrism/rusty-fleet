# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RustyFleetSha256 {
    param([Parameter(Mandatory)][string] $LiteralPath)

    (Get-FileHash -LiteralPath $LiteralPath -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Write-RustyFleetUtf8 {
    param(
        [Parameter(Mandatory)][string] $LiteralPath,
        [Parameter(Mandatory)][string] $Content
    )

    $parent = Split-Path -Parent $LiteralPath
    if ($parent) {
        [System.IO.Directory]::CreateDirectory($parent) | Out-Null
    }
    [System.IO.File]::WriteAllText(
        $LiteralPath,
        $Content,
        [System.Text.UTF8Encoding]::new($false)
    )
}

function ConvertTo-RustyFleetJson {
    param([Parameter(Mandatory)][object] $InputObject)

    ($InputObject | ConvertTo-Json -Depth 30) + "`n"
}

function Get-RustyFleetRelativePath {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][string] $LiteralPath
    )

    [System.IO.Path]::GetRelativePath(
        [System.IO.Path]::GetFullPath($Root),
        [System.IO.Path]::GetFullPath($LiteralPath)
    ).Replace("\", "/")
}

function Assert-RustyFleetHttpsUrl {
    param(
        [Parameter(Mandatory)][string] $Value,
        [Parameter(Mandatory)][string] $Name
    )

    $uri = $null
    if (-not [Uri]::TryCreate($Value, [UriKind]::Absolute, [ref] $uri) -or
        $uri.Scheme -ne "https") {
        throw "$Name must be an absolute HTTPS URL"
    }
}

function Assert-RustyFleetSha256 {
    param(
        [Parameter(Mandatory)][string] $Value,
        [Parameter(Mandatory)][string] $Name
    )

    if ($Value -cnotmatch "^[0-9a-f]{64}$") {
        throw "$Name must be exactly 64 lowercase hexadecimal characters"
    }
}

function Assert-RustyFleetPayloadPath {
    param([Parameter(Mandatory)][string] $RelativePath)

    if ($RelativePath -match "(^|/)\.\.?(/|$)" -or
        $RelativePath -match "(?i)(^|/)(adb|fastboot)(\.exe)?$" -or
        $RelativePath -match "(?i)(credential|password|passphrase|pairing|private[-_]?config|secret|token|keystore)") {
        throw "prohibited distribution payload path: $RelativePath"
    }
}

function Assert-RustyFleetExactProperties {
    param(
        [Parameter(Mandatory)][object] $InputObject,
        [Parameter(Mandatory)][string[]] $Expected,
        [Parameter(Mandatory)][string] $Context
    )

    $actual = @($InputObject.PSObject.Properties.Name | Sort-Object)
    if (@(Compare-Object ($Expected | Sort-Object) $actual).Count -ne 0) {
        throw "$Context has missing or unknown fields"
    }
}

function Read-RustyFleetReleaseTrustPolicy {
    param(
        [Parameter(Mandatory)][string] $LiteralPath,
        [Parameter(Mandatory)][ValidateSet("labs", "stable")]
        [string] $Channel
    )

    $policy = Get-Content -LiteralPath $LiteralPath -Raw |
        ConvertFrom-Json -Depth 20
    Assert-RustyFleetExactProperties -InputObject $policy -Expected @(
        "schema", "channels"
    ) -Context "Fleet release trust policy"
    Assert-RustyFleetExactProperties -InputObject $policy.channels -Expected @(
        "labs", "stable"
    ) -Context "Fleet release trust channels"
    foreach ($channelName in @("labs", "stable")) {
        $channelPolicy = $policy.channels.$channelName
        Assert-RustyFleetExactProperties -InputObject $channelPolicy -Expected @(
            "publication_enabled",
            "authenticode",
            "authorized_descriptor_signer_spki_sha256",
            "status"
        ) -Context "Fleet $channelName release trust policy"
        Assert-RustyFleetExactProperties `
            -InputObject $channelPolicy.authenticode `
            -Expected @(
                "subject",
                "thumbprint",
                "certificate_sha256",
                "self_issued",
                "public_trust_claim",
                "trust_mode",
                "timestamp_required",
                "allowed_chain_status_flags"
            ) `
            -Context "Fleet $channelName Authenticode policy"
    }
    if ($policy.schema -cne
            "rusty.fleet.windows_release_trust_policy.v2") {
        throw "Fleet release trust policy schema is not supported"
    }

    $labs = $policy.channels.labs
    $labsAuth = $labs.authenticode
    if ($labs.publication_enabled -ne $true -or
        [string]::IsNullOrWhiteSpace([string] $labsAuth.subject) -or
        $labsAuth.thumbprint -cnotmatch "^[0-9A-F]{40}$" -or
        $labsAuth.certificate_sha256 -cnotmatch "^[0-9a-f]{64}$" -or
        $labsAuth.self_issued -ne $true -or
        $labsAuth.public_trust_claim -ne $false -or
        $labsAuth.trust_mode -cne
            "exact-pinned-self-issued-untrusted-root-only" -or
        $labsAuth.timestamp_required -ne $true -or
        @($labsAuth.allowed_chain_status_flags).Count -ne 1 -or
        @($labsAuth.allowed_chain_status_flags)[0] -cne "UntrustedRoot" -or
        @($labs.authorized_descriptor_signer_spki_sha256).Count -ne 1 -or
        @($labs.authorized_descriptor_signer_spki_sha256)[0] -cnotmatch
            "^[0-9a-f]{64}$" -or
        $labs.status -cne
            "labs_exact_pinned_self_issued_signer_configured") {
        throw "Labs release trust policy is not the exact reviewed authorization"
    }

    $stable = $policy.channels.stable
    $stableAuth = $stable.authenticode
    if ($stable.publication_enabled -ne $false -or
        $null -ne $stableAuth.subject -or
        $null -ne $stableAuth.thumbprint -or
        $null -ne $stableAuth.certificate_sha256 -or
        $stableAuth.self_issued -ne $false -or
        $stableAuth.public_trust_claim -ne $true -or
        $stableAuth.trust_mode -cne "public-chain-only" -or
        $stableAuth.timestamp_required -ne $true -or
        @($stableAuth.allowed_chain_status_flags).Count -ne 0 -or
        @($stable.authorized_descriptor_signer_spki_sha256).Count -ne 0 -or
        $stable.status -cne "stable_public_chain_signer_not_configured") {
        throw "Stable release trust policy must remain disabled and public-chain-only"
    }

    return $policy.channels.$Channel
}

function Get-RustyFleetCertificateChainAssessment {
    param(
        [Parameter(Mandatory)]
        [Security.Cryptography.X509Certificates.X509Certificate2]
        $Certificate
    )

    $chain = [Security.Cryptography.X509Certificates.X509Chain]::new()
    try {
        $chain.ChainPolicy.RevocationMode =
            [Security.Cryptography.X509Certificates.X509RevocationMode]::NoCheck
        $chain.ChainPolicy.VerificationFlags =
            [Security.Cryptography.X509Certificates.X509VerificationFlags]::NoFlag
        [void] $chain.Build($Certificate)
        return [pscustomobject][ordered]@{
            chain_status_flags = @(
                $chain.ChainStatus |
                    ForEach-Object { [string] $_.Status } |
                    Sort-Object -Unique
            )
            chain_element_count = $chain.ChainElements.Count
        }
    }
    finally {
        $chain.Dispose()
    }
}

function Assert-RustyFleetValidationBoundary {
    param(
        [Parameter(Mandatory)][string] $AuthenticodeStatus,
        [Parameter(Mandatory)][bool] $ChainTrusted,
        [Parameter(Mandatory)][int] $ChainElementCount,
        [Parameter(Mandatory)][AllowEmptyCollection()]
        [string[]] $ChainStatusFlags,
        [Parameter(Mandatory)][object[]] $AcceptedBoundaries,
        [Parameter(Mandatory)][string] $Context
    )

    $matches = @($AcceptedBoundaries | Where-Object {
        $_.authenticode_status -ceq $AuthenticodeStatus -and
        $_.chain_trusted -eq $ChainTrusted -and
        [int] $_.chain_element_count -eq $ChainElementCount -and
        @(Compare-Object `
            @($_.chain_status_flags) `
            @($ChainStatusFlags)).Count -eq 0 -and
        @($_.chain_status_flags).Count -eq @($ChainStatusFlags).Count
    })
    if ($matches.Count -ne 1) {
        throw "$Context is not one exact accepted validation boundary"
    }
    return $matches[0]
}

function Get-RustyFleetLabsTrustBoundaryLabel {
    param(
        [Parameter(Mandatory)][string] $AuthenticodeStatus,
        [Parameter(Mandatory)][bool] $ChainTrusted,
        [Parameter(Mandatory)][int] $ChainElementCount,
        [Parameter(Mandatory)][AllowEmptyCollection()]
        [string[]] $ChainStatusFlags,
        [Parameter(Mandatory)][object[]] $AcceptedBoundaries,
        [Parameter(Mandatory)][string] $Context
    )

    $boundary = Assert-RustyFleetValidationBoundary `
        -AuthenticodeStatus $AuthenticodeStatus `
        -ChainTrusted $ChainTrusted `
        -ChainElementCount $ChainElementCount `
        -ChainStatusFlags $ChainStatusFlags `
        -AcceptedBoundaries $AcceptedBoundaries `
        -Context $Context
    if ($boundary.authenticode_status -ceq "valid" -and
        $boundary.chain_trusted -eq $true -and
        [int] $boundary.chain_element_count -eq 1 -and
        @($boundary.chain_status_flags).Count -eq 0) {
        return "host-chain-valid-no-public-trust-claim"
    }
    if ($boundary.authenticode_status -ceq "unknown_error" -and
        $boundary.chain_trusted -eq $false -and
        [int] $boundary.chain_element_count -eq 1 -and
        @($boundary.chain_status_flags).Count -eq 1 -and
        @($boundary.chain_status_flags)[0] -ceq "UntrustedRoot") {
        return "exact-pinned-self-issued-untrusted-root-only"
    }
    throw "$Context does not map to one exact Labs trust-boundary label"
}

function Assert-RustyFleetAuthenticodeAssessment {
    param(
        [Parameter(Mandatory)][object] $Signature,
        [Parameter(Mandatory)][object] $AuthenticodePolicy,
        [Parameter(Mandatory)][ValidateSet("labs", "stable")]
        [string] $Channel,
        [Parameter(Mandatory)][AllowEmptyCollection()]
        [string[]] $ObservedChainStatusFlags,
        [int] $ObservedChainElementCount = 1
    )

    if ($null -eq $Signature.SignerCertificate -or
        $Signature.SignatureType -ne
            [Management.Automation.SignatureType]::Authenticode) {
        throw "artifact does not carry an Authenticode signer certificate"
    }
    $certificate = $Signature.SignerCertificate
    $certificateSha256 = [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData($certificate.RawData)
    ).ToLowerInvariant()
    $subject = [string] $certificate.Subject
    $thumbprint = $certificate.Thumbprint.Replace(
        " ", ""
    ).ToUpperInvariant()
    $selfIssued = [Convert]::ToBase64String(
        $certificate.SubjectName.RawData
    ) -ceq [Convert]::ToBase64String($certificate.IssuerName.RawData)
    $codeSigningEkuPresent = @(
        $certificate.Extensions |
            Where-Object {
                $_ -is [Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]
            } |
            ForEach-Object { $_.EnhancedKeyUsages } |
            ForEach-Object { $_ } |
            Where-Object { $_.Value -ceq "1.3.6.1.5.5.7.3.3" }
    ).Count -gt 0
    $chainFlags = @($ObservedChainStatusFlags | Sort-Object -Unique)
    if ($chainFlags.Count -ne @($ObservedChainStatusFlags).Count -or
        $ObservedChainElementCount -lt 1) {
        throw "Authenticode certificate chain assessment is not canonical"
    }
    if ($AuthenticodePolicy.timestamp_required -eq $true -and
        $null -eq $Signature.TimeStamperCertificate) {
        throw "Authenticode signature does not carry the required timestamp"
    }

    if ($Channel -eq "labs") {
        $authenticodeStatus = switch ($Signature.Status) {
            ([Management.Automation.SignatureStatus]::Valid) { "valid" }
            ([Management.Automation.SignatureStatus]::UnknownError) {
                "unknown_error"
            }
            default { "rejected" }
        }
        $acceptedBoundaries = @(
            [pscustomobject][ordered]@{
                authenticode_status = "valid"
                chain_trusted = $true
                chain_element_count = 1
                chain_status_flags = @()
            },
            [pscustomobject][ordered]@{
                authenticode_status = "unknown_error"
                chain_trusted = $false
                chain_element_count = 1
                chain_status_flags = @(
                    $AuthenticodePolicy.allowed_chain_status_flags
                )
            }
        )
        $chainTrusted = $authenticodeStatus -ceq "valid"
        $validationBoundary = Get-RustyFleetLabsTrustBoundaryLabel `
            -AuthenticodeStatus $authenticodeStatus `
            -ChainTrusted $chainTrusted `
            -ChainElementCount $ObservedChainElementCount `
            -ChainStatusFlags $chainFlags `
            -AcceptedBoundaries $acceptedBoundaries `
            -Context "Labs current-host Authenticode assessment"
        if ($AuthenticodePolicy.trust_mode -cne
                "exact-pinned-self-issued-untrusted-root-only" -or
            $AuthenticodePolicy.public_trust_claim -ne $false -or
            $AuthenticodePolicy.self_issued -ne $true -or
            $subject -cne $AuthenticodePolicy.subject -or
            $thumbprint -cne $AuthenticodePolicy.thumbprint -or
            $certificateSha256 -cne
                $AuthenticodePolicy.certificate_sha256 -or
            -not $selfIssued -or
            -not $codeSigningEkuPresent) {
            throw "Labs Authenticode signature is not the exact pinned self-issued identity"
        }
    }
    elseif ($AuthenticodePolicy.trust_mode -cne "public-chain-only" -or
        $AuthenticodePolicy.public_trust_claim -ne $true -or
        $AuthenticodePolicy.self_issued -ne $false -or
        $Signature.Status -ne
            [Management.Automation.SignatureStatus]::Valid -or
        $selfIssued -or
        [string]::IsNullOrWhiteSpace([string] $AuthenticodePolicy.subject) -or
        [string]::IsNullOrWhiteSpace([string] $AuthenticodePolicy.thumbprint) -or
        [string]::IsNullOrWhiteSpace(
            [string] $AuthenticodePolicy.certificate_sha256
        ) -or
        $subject -cne $AuthenticodePolicy.subject -or
        $thumbprint -cne $AuthenticodePolicy.thumbprint -or
        $certificateSha256 -cne $AuthenticodePolicy.certificate_sha256 -or
        -not $codeSigningEkuPresent -or
        $ObservedChainElementCount -lt 2 -or
        $chainFlags.Count -ne 0) {
        throw "Stable Authenticode signature is not valid under a public trust chain"
    }
    else {
        $validationBoundary = "public-chain-valid"
    }

    $normalizedStatus = switch ($Signature.Status) {
        ([Management.Automation.SignatureStatus]::Valid) { "valid" }
        ([Management.Automation.SignatureStatus]::UnknownError) {
            "unknown_error"
        }
        default { "rejected" }
    }
    $chainTrusted = $normalizedStatus -ceq "valid"

    [ordered]@{
        schema = "rusty.fleet.authenticode_assessment.v1"
        result = "pass"
        channel = $Channel
        subject = $subject
        thumbprint = $thumbprint
        certificate_sha256 = $certificateSha256
        code_signing_eku_present = $codeSigningEkuPresent
        self_issued = $selfIssued
        public_trust_claim = [bool] $AuthenticodePolicy.public_trust_claim
        trust_mode = [string] $AuthenticodePolicy.trust_mode
        validation_boundary = $validationBoundary
        timestamp_present = $null -ne $Signature.TimeStamperCertificate
        authenticode_status = $normalizedStatus
        chain_trusted = $chainTrusted
        chain_element_count = $ObservedChainElementCount
        chain_status_flags = $chainFlags
    }
}

function Get-RustyFleetAuthenticodeAssessment {
    param(
        [Parameter(Mandatory)][string] $LiteralPath,
        [Parameter(Mandatory)][object] $AuthenticodePolicy,
        [Parameter(Mandatory)][ValidateSet("labs", "stable")]
        [string] $Channel
    )

    $signature = Get-AuthenticodeSignature -LiteralPath $LiteralPath
    if ($null -eq $signature.SignerCertificate) {
        throw "artifact does not carry an Authenticode signer certificate"
    }
    $chainAssessment = Get-RustyFleetCertificateChainAssessment `
        -Certificate $signature.SignerCertificate
    Assert-RustyFleetAuthenticodeAssessment `
        -Signature $signature `
        -AuthenticodePolicy $AuthenticodePolicy `
        -Channel $Channel `
        -ObservedChainStatusFlags $chainAssessment.chain_status_flags `
        -ObservedChainElementCount $chainAssessment.chain_element_count
}

function Get-RustyFleetPeCanonicalPayload {
    param(
        [Parameter(Mandatory)][string] $LiteralPath,
        [Parameter(Mandatory)][long] $ExpectedPayloadSize
    )

    [byte[]] $bytes = [System.IO.File]::ReadAllBytes($LiteralPath)
    if ($bytes.Length -lt 512) {
        throw "PE artifact is unexpectedly small"
    }
    if ($bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) {
        throw "PE artifact does not have a valid DOS header"
    }
    $peOffset = [int] [BitConverter]::ToUInt32($bytes, 0x3c)
    if ($peOffset -lt 64 -or $peOffset + 24 -gt $bytes.Length -or
        $bytes[$peOffset] -ne 0x50 -or
        $bytes[$peOffset + 1] -ne 0x45 -or
        $bytes[$peOffset + 2] -ne 0 -or
        $bytes[$peOffset + 3] -ne 0) {
        throw "PE artifact does not have a valid PE header"
    }
    $optionalHeader = $peOffset + 24
    $optionalHeaderSize = [int] [BitConverter]::ToUInt16(
        $bytes,
        $peOffset + 20
    )
    if ($optionalHeaderSize -le 0 -or
        $optionalHeader + $optionalHeaderSize -gt $bytes.Length) {
        throw "PE optional header is truncated"
    }
    $optionalHeaderMagic = [BitConverter]::ToUInt16($bytes, $optionalHeader)
    $expectedOptionalHeaderSize = switch ($optionalHeaderMagic) {
        0x10b { 224 }
        0x20b { 240 }
        default { throw "PE artifact has an unsupported optional header" }
    }
    if ($optionalHeaderSize -ne $expectedOptionalHeaderSize) {
        throw "PE artifact has a non-canonical optional-header size"
    }
    $dataDirectories = switch ($optionalHeaderMagic) {
        0x10b { $optionalHeader + 96 }
        0x20b { $optionalHeader + 112 }
    }
    $directoryCountOffset = switch ($optionalHeaderMagic) {
        0x10b { $optionalHeader + 92 }
        0x20b { $optionalHeader + 108 }
    }
    $checksumOffset = $optionalHeader + 64
    $certificateDirectory = $dataDirectories + (4 * 8)
    $directoryCount = [long] [BitConverter]::ToUInt32(
        $bytes,
        $directoryCountOffset
    )
    if ($directoryCountOffset + 4 -gt
            $optionalHeader + $optionalHeaderSize -or
        $directoryCount -ne 16 -or
        $dataDirectories + ($directoryCount * 8) -ne
            $optionalHeader + $optionalHeaderSize -or
        $checksumOffset + 4 -gt $optionalHeader + $optionalHeaderSize -or
        $certificateDirectory + 8 -gt
            $optionalHeader + $optionalHeaderSize) {
        throw "PE optional header does not have the canonical directory layout"
    }
    $certificateOffset = [long] [BitConverter]::ToUInt32(
        $bytes,
        $certificateDirectory
    )
    $certificateSize = [long] [BitConverter]::ToUInt32(
        $bytes,
        $certificateDirectory + 4
    )
    if (($certificateOffset -eq 0) -xor ($certificateSize -eq 0)) {
        throw "PE certificate directory is inconsistent"
    }
    $physicalPayloadSize = [long] $bytes.Length
    if ($certificateOffset -ne 0) {
        if ($certificateOffset % 8 -ne 0 -or
            $certificateSize -lt 8 -or
            $certificateOffset + $certificateSize -ne $bytes.Length) {
            throw "PE signature table is malformed or has an overlay"
        }
        $certificateLength = [long] [BitConverter]::ToUInt32(
            $bytes,
            [int] $certificateOffset
        )
        $certificateRevision = [BitConverter]::ToUInt16(
            $bytes,
            [int] $certificateOffset + 4
        )
        $certificateType = [BitConverter]::ToUInt16(
            $bytes,
            [int] $certificateOffset + 6
        )
        $alignedCertificateLength = (
            [long] (($certificateLength + 7) -band (-bnot 7))
        )
        if ($certificateLength -lt 8 -or
            $certificateLength -gt $certificateSize -or
            $certificateRevision -ne 0x0200 -or
            $certificateType -ne 0x0002 -or
            $alignedCertificateLength -ne $certificateSize) {
            throw "PE signature table is ambiguous or contains multiple entries"
        }
        for (
            $index = $certificateOffset + $certificateLength
            $index -lt $certificateOffset + $certificateSize
            $index++
        ) {
            if ($bytes[$index] -ne 0) {
                throw "PE signature-table alignment padding is not zero"
            }
        }
        $physicalPayloadSize = $certificateOffset
    }
    if ($ExpectedPayloadSize -le $certificateDirectory + 8 -or
        $ExpectedPayloadSize -gt $physicalPayloadSize) {
        throw "canonical PE payload size is invalid"
    }
    if (($certificateOffset -eq 0 -and
            $ExpectedPayloadSize -ne $physicalPayloadSize) -or
        ($certificateOffset -ne 0 -and
            $physicalPayloadSize -ne
                (($ExpectedPayloadSize + 7) -band (-bnot 7)))) {
        throw "canonical PE payload has an invalid signature-alignment gap"
    }
    for (
        $index = $ExpectedPayloadSize
        $index -lt $physicalPayloadSize
        $index++
    ) {
        if ($bytes[$index] -ne 0) {
            throw "PE artifact has data between its payload and signature"
        }
    }
    [byte[]] $payload = [byte[]]::new($ExpectedPayloadSize)
    [Array]::Copy($bytes, 0, $payload, 0, $ExpectedPayloadSize)
    [Array]::Clear($payload, $checksumOffset, 4)
    [Array]::Clear($payload, $certificateDirectory, 8)
    [ordered]@{
        sha256 = [Convert]::ToHexString(
            [System.Security.Cryptography.SHA256]::HashData($payload)
        ).ToLowerInvariant()
        size_bytes = $ExpectedPayloadSize
    }
}

function ConvertTo-RustyFleetUtcDateTimeOffset {
    param(
        [Parameter(Mandatory)][object] $Value,
        [Parameter(Mandatory)][string] $Context
    )

    if ($Value -is [DateTime]) {
        return [DateTimeOffset]::new($Value.ToUniversalTime())
    }
    if ($Value -is [DateTimeOffset]) {
        return $Value.ToUniversalTime()
    }
    $parsed = [DateTimeOffset]::MinValue
    if (-not [DateTimeOffset]::TryParse(
        [string] $Value,
        [Globalization.CultureInfo]::InvariantCulture,
        [Globalization.DateTimeStyles]::AssumeUniversal -bor
            [Globalization.DateTimeStyles]::AdjustToUniversal,
        [ref] $parsed
    )) {
        throw "$Context is not an invariant UTC timestamp"
    }
    return $parsed.ToUniversalTime()
}

function Read-RustyFleetHostessProvenance {
    param(
        [Parameter(Mandatory)][string] $MetadataDirectory,
        [Parameter(Mandatory)][string] $ProviderPath,
        [Parameter(Mandatory)][string] $ProviderSha256,
        [Parameter(Mandatory)][ValidateSet("unsigned-dev", "signed-release")]
        [string] $BuildKind,
        [Parameter(Mandatory)][ValidateSet("dev", "labs", "stable")]
        [string] $Channel,
        [object] $AuthenticodePolicy
    )

    $metadataPath = (Resolve-Path -LiteralPath $MetadataDirectory).Path
    $providerFullPath = (Resolve-Path -LiteralPath $ProviderPath).Path
    $documents = [ordered]@{
        provenance = "rusty-hostess-hotspot-provider.provenance.json"
        license = "LICENSE"
        notices = "THIRD-PARTY-NOTICES.txt"
        policy = "rusty-hostess-hotspot-provider.release-policy.json"
    }
    foreach ($name in $documents.Values) {
        if (-not (Test-Path -LiteralPath (Join-Path $metadataPath $name) -PathType Leaf)) {
            throw "Hostess owner metadata is missing required document: $name"
        }
    }
    $metadataFiles = @(
        Get-ChildItem -LiteralPath $metadataPath -File |
            Select-Object -ExpandProperty Name |
            Sort-Object
    )
    if (@(Compare-Object ($documents.Values | Sort-Object) $metadataFiles).Count -ne 0) {
        throw "Hostess owner metadata must contain exactly its four issued documents"
    }

    $provenancePath = Join-Path $metadataPath $documents.provenance
    $provenance = Get-Content -LiteralPath $provenancePath -Raw |
        ConvertFrom-Json -Depth 30
    Assert-RustyFleetExactProperties -InputObject $provenance -Expected @(
        "schema",
        "product_id",
        "provider_version",
        "artifact",
        "source",
        "build",
        "dependencies",
        "bundled_native_libraries",
        "signing",
        "release_policy",
        "companion_documents",
        "distribution"
    ) -Context "Hostess provenance"
    Assert-RustyFleetExactProperties -InputObject $provenance.artifact -Expected @(
        "name", "sha256", "size_bytes", "product_version"
    ) -Context "Hostess artifact provenance"
    Assert-RustyFleetExactProperties -InputObject $provenance.source -Expected @(
        "repository",
        "revision",
        "tree",
        "availability_url",
        "availability_state",
        "verified_at_utc",
        "tree_clean"
    ) -Context "Hostess source provenance"
    Assert-RustyFleetExactProperties -InputObject $provenance.build -Expected @(
        "kind",
        "framework",
        "runtime_identifier",
        "source_date_epoch",
        "unsigned_artifact_sha256",
        "unsigned_artifact_size_bytes",
        "canonical_payload_sha256",
        "canonical_payload_size_bytes"
    ) -Context "Hostess build provenance"
    Assert-RustyFleetExactProperties -InputObject $provenance.signing -Expected @(
        "state",
        "authenticode_status",
        "subject",
        "issuer",
        "thumbprint_sha1",
        "certificate_sha256",
        "code_signing_eku_present",
        "self_issued",
        "timestamp_present",
        "chain_trusted",
        "chain_element_count",
        "chain_status_flags",
        "public_trust_claim",
        "trust_boundary"
    ) -Context "Hostess signing provenance"
    Assert-RustyFleetExactProperties -InputObject $provenance.release_policy -Expected @(
        "asset_name", "schema", "sha256", "size_bytes"
    ) -Context "Hostess release policy evidence"
    Assert-RustyFleetExactProperties -InputObject $provenance.distribution -Expected @(
        "eligibility", "binary_authority", "allowed_channels", "stable_eligible"
    ) -Context "Hostess distribution provenance"
    if ($provenance.schema -ne "rusty.hostess.windows_hotspot.release_provenance.v2" -or
        $provenance.product_id -ne "rusty-hostess-windows-hotspot-provider" -or
        $provenance.provider_version -cnotmatch
            "^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$") {
        throw "Hostess provenance schema is not supported"
    }

    $providerItem = Get-Item -LiteralPath $providerFullPath
    $expectedProductVersion = (
        "$($provenance.provider_version)+$($provenance.source.revision)"
    )
    $observedProductVersion = (
        [System.Diagnostics.FileVersionInfo]::GetVersionInfo(
            $providerFullPath
        ).ProductVersion
    )
    if ($provenance.artifact.name -cne "rusty-hostess-hotspot-provider.exe" -or
        $provenance.artifact.sha256 -cne $ProviderSha256 -or
        $provenance.artifact.sha256 -cne (
            Get-RustyFleetSha256 -LiteralPath $providerFullPath
        ) -or
        [long] $provenance.artifact.size_bytes -ne $providerItem.Length -or
        $provenance.artifact.product_version -cne $expectedProductVersion -or
        $observedProductVersion -cne $expectedProductVersion) {
        throw "Hostess provenance does not bind the exact provider artifact"
    }
    Assert-RustyFleetSha256 `
        -Value $provenance.artifact.sha256 `
        -Name "Hostess provenance artifact digest"
    $providerPolicyPath = Join-Path $metadataPath $documents.policy
    if ($provenance.release_policy.asset_name -cne $documents.policy -or
        $provenance.release_policy.schema -cne
            "rusty.hostess.windows_hotspot.release_policy.v1" -or
        $provenance.release_policy.sha256 -cne
            (Get-RustyFleetSha256 -LiteralPath $providerPolicyPath) -or
        [long] $provenance.release_policy.size_bytes -ne
            (Get-Item -LiteralPath $providerPolicyPath).Length) {
        throw "Hostess release policy evidence is not exact"
    }
    $providerPolicy = Get-Content -LiteralPath $providerPolicyPath -Raw |
        ConvertFrom-Json -Depth 20
    Assert-RustyFleetExactProperties -InputObject $providerPolicy -Expected @(
        "schema", "product_id", "signer", "accepted_validation_boundaries",
        "distribution", "status"
    ) -Context "Hostess release policy"
    Assert-RustyFleetExactProperties -InputObject $providerPolicy.signer -Expected @(
        "subject", "issuer", "thumbprint_sha1", "certificate_sha256",
        "code_signing_eku_oid", "self_issued", "timestamp_required",
        "public_trust_claim"
    ) -Context "Hostess release signer policy"
    Assert-RustyFleetExactProperties -InputObject $providerPolicy.distribution -Expected @(
        "allowed_channels", "stable_eligible"
    ) -Context "Hostess release distribution policy"
    if ($providerPolicy.schema -cne
            "rusty.hostess.windows_hotspot.release_policy.v1" -or
        $providerPolicy.product_id -cne
            "rusty-hostess-windows-hotspot-provider" -or
        [string]::IsNullOrWhiteSpace([string] $providerPolicy.signer.subject) -or
        $providerPolicy.signer.issuer -cne $providerPolicy.signer.subject -or
        $providerPolicy.signer.thumbprint_sha1 -cnotmatch "^[0-9A-F]{40}$" -or
        $providerPolicy.signer.certificate_sha256 -cnotmatch "^[0-9a-f]{64}$" -or
        $providerPolicy.signer.code_signing_eku_oid -cne
            "1.3.6.1.5.5.7.3.3" -or
        $providerPolicy.signer.self_issued -ne $true -or
        $providerPolicy.signer.timestamp_required -ne $true -or
        $providerPolicy.signer.public_trust_claim -ne $false -or
        @($providerPolicy.distribution.allowed_channels).Count -ne 1 -or
        @($providerPolicy.distribution.allowed_channels)[0] -cne "labs" -or
        $providerPolicy.distribution.stable_eligible -ne $false -or
        $providerPolicy.status -cne "active") {
        throw "Hostess release policy does not preserve the exact Labs trust boundary"
    }
    $boundaries = @($providerPolicy.accepted_validation_boundaries)
    if ($boundaries.Count -ne 2) {
        throw "Hostess release policy validation-boundary set is not exact"
    }
    foreach ($boundary in $boundaries) {
        Assert-RustyFleetExactProperties -InputObject $boundary -Expected @(
            "authenticode_status", "chain_trusted", "chain_element_count",
            "chain_status_flags"
        ) -Context "Hostess validation boundary"
    }
    $trustedBoundary = @($boundaries | Where-Object {
        $_.authenticode_status -ceq "valid"
    })
    $untrustedBoundary = @($boundaries | Where-Object {
        $_.authenticode_status -ceq "unknown_error"
    })
    if ($trustedBoundary.Count -ne 1 -or
        $trustedBoundary[0].chain_trusted -ne $true -or
        [int] $trustedBoundary[0].chain_element_count -ne 1 -or
        @($trustedBoundary[0].chain_status_flags).Count -ne 0 -or
        $untrustedBoundary.Count -ne 1 -or
        $untrustedBoundary[0].chain_trusted -ne $false -or
        [int] $untrustedBoundary[0].chain_element_count -ne 1 -or
        @($untrustedBoundary[0].chain_status_flags).Count -ne 1 -or
        @($untrustedBoundary[0].chain_status_flags)[0] -cne "UntrustedRoot") {
        throw "Hostess release policy chain-boundary truth is not exact"
    }

    if ($provenance.source.repository -cne
            "https://github.com/MesmerPrism/rusty-hostess" -or
        $provenance.source.revision -cnotmatch "^[0-9a-f]{40}$" -or
        $provenance.source.tree -cnotmatch "^[0-9a-f]{40}$" -or
        $provenance.source.tree_clean -ne $true) {
        throw "Hostess provenance does not bind a clean full source commit and tree"
    }
    $availabilityUri = [Uri] $provenance.source.availability_url
    if ($availabilityUri.Scheme -cne "https" -or
        $availabilityUri.Host -cne "github.com" -or
        $availabilityUri.AbsolutePath.TrimEnd("/") -cne
            "/MesmerPrism/rusty-hostess/tree/$($provenance.source.revision)") {
        throw "Hostess source availability does not bind the exact revision"
    }
    foreach ($buildField in @(
        $provenance.build.kind,
        $provenance.build.framework,
        $provenance.build.runtime_identifier
    )) {
        if ([string]::IsNullOrWhiteSpace([string] $buildField)) {
            throw "Hostess provenance build metadata is incomplete"
        }
    }
    if ($provenance.build.kind -cne $BuildKind -or
        [long] $provenance.build.source_date_epoch -le 0 -or
        [long] $provenance.build.unsigned_artifact_size_bytes -le 0) {
        throw "Hostess provenance source-date epoch is invalid"
    }
    Assert-RustyFleetSha256 `
        -Value $provenance.build.unsigned_artifact_sha256 `
        -Name "Hostess unsigned artifact digest"
    Assert-RustyFleetSha256 `
        -Value $provenance.build.canonical_payload_sha256 `
        -Name "Hostess canonical PE payload digest"
    $canonicalPayload = Get-RustyFleetPeCanonicalPayload `
        -LiteralPath $providerFullPath `
        -ExpectedPayloadSize $provenance.build.canonical_payload_size_bytes
    if ($canonicalPayload.sha256 -cne
            $provenance.build.canonical_payload_sha256 -or
        $canonicalPayload.size_bytes -ne
            [long] $provenance.build.canonical_payload_size_bytes -or
        $provenance.build.unsigned_artifact_sha256 -cne
            $provenance.build.canonical_payload_sha256 -or
        [long] $provenance.build.unsigned_artifact_size_bytes -ne
            [long] $provenance.build.canonical_payload_size_bytes) {
        throw "Hostess provider canonical PE payload does not match owner provenance"
    }

    $dependencies = @($provenance.dependencies)
    if ($dependencies.Count -eq 0) {
        throw "Hostess provenance must carry its owner-generated dependency report"
    }
    foreach ($dependency in $dependencies) {
        Assert-RustyFleetExactProperties -InputObject $dependency -Expected @(
            "name", "version", "license", "license_url", "project_url"
        ) -Context "Hostess dependency provenance"
        foreach ($field in @(
            $dependency.name,
            $dependency.version,
            $dependency.license
        )) {
            if ([string]::IsNullOrWhiteSpace([string] $field)) {
                throw "Hostess dependency provenance is incomplete"
            }
        }
    }

    $nativeLibraries = @($provenance.bundled_native_libraries)
    if ($nativeLibraries.Count -eq 0) {
        throw "Hostess provenance must carry its bundled native library inventory"
    }
    foreach ($library in $nativeLibraries) {
        Assert-RustyFleetExactProperties -InputObject $library -Expected @(
            "name", "sha256", "size_bytes"
        ) -Context "Hostess bundled native library provenance"
        if ($library.name -cnotmatch "^[A-Za-z0-9_.-]+\.dll$" -or
            [long] $library.size_bytes -le 0) {
            throw "Hostess bundled-native-library inventory is incomplete"
        }
        Assert-RustyFleetSha256 `
            -Value $library.sha256 `
            -Name "Hostess bundled native library digest"
    }
    if (@($nativeLibraries.name | Sort-Object -Unique).Count -ne
        $nativeLibraries.Count) {
        throw "Hostess bundled native library names are not unique"
    }

    $companions = @($provenance.companion_documents)
    $expectedCompanions = @("LICENSE", "THIRD-PARTY-NOTICES.txt")
    $companionNames = @($companions | ForEach-Object { $_.name })
    if (@(Compare-Object $expectedCompanions $companionNames).Count -ne 0 -or
        $companions.Count -ne 2) {
        throw "Hostess provenance must bind exactly LICENSE and THIRD-PARTY-NOTICES.txt"
    }
    foreach ($companion in $companions) {
        Assert-RustyFleetExactProperties -InputObject $companion -Expected @(
            "name", "sha256", "size_bytes"
        ) -Context "Hostess companion document provenance"
        Assert-RustyFleetSha256 `
            -Value $companion.sha256 `
            -Name "Hostess companion document digest"
        $documentPath = Join-Path $metadataPath $companion.name
        if ((Get-RustyFleetSha256 -LiteralPath $documentPath) -cne $companion.sha256 -or
            (Get-Item -LiteralPath $documentPath).Length -ne [long] $companion.size_bytes) {
            throw "Hostess companion document does not match owner provenance: $($companion.name)"
        }
    }

    if ($BuildKind -eq "signed-release") {
        if ($Channel -eq "dev" -or $null -eq $AuthenticodePolicy) {
            throw "signed Hostess provenance requires an exact release-channel trust policy"
        }
        $verifiedAtValue = $provenance.source.verified_at_utc
        $verifiedAtValid = try {
            $verifiedAt = ConvertTo-RustyFleetUtcDateTimeOffset `
                -Value $verifiedAtValue `
                -Context "Hostess source verified_at_utc"
            $true
        }
        catch {
            $false
        }
        if ($provenance.source.availability_state -cne "verified_public" -or
            [string]::IsNullOrWhiteSpace(
                [string] $verifiedAtValue
            ) -or
            -not $verifiedAtValid) {
            throw "Hostess signed release provenance is not independently verified"
        }
        if ($providerPolicy.signer.subject -cne $AuthenticodePolicy.subject -or
            $providerPolicy.signer.issuer -cne $AuthenticodePolicy.subject -or
            $providerPolicy.signer.thumbprint_sha1 -cne
                $AuthenticodePolicy.thumbprint -or
            $providerPolicy.signer.certificate_sha256 -cne
                $AuthenticodePolicy.certificate_sha256) {
            throw "Hostess owner policy signer does not match Fleet channel authorization"
        }
        $assessment = Get-RustyFleetAuthenticodeAssessment `
            -LiteralPath $providerFullPath `
            -AuthenticodePolicy $AuthenticodePolicy `
            -Channel $Channel
        $recordedTrustBoundary = Get-RustyFleetLabsTrustBoundaryLabel `
            -AuthenticodeStatus $provenance.signing.authenticode_status `
            -ChainTrusted ([bool] $provenance.signing.chain_trusted) `
            -ChainElementCount (
                [int] $provenance.signing.chain_element_count
            ) `
            -ChainStatusFlags @(
                $provenance.signing.chain_status_flags
            ) `
            -AcceptedBoundaries $boundaries `
            -Context "Hostess recorded Authenticode assessment"
        $currentHostTrustBoundary = Get-RustyFleetLabsTrustBoundaryLabel `
            -AuthenticodeStatus $assessment.authenticode_status `
            -ChainTrusted ([bool] $assessment.chain_trusted) `
            -ChainElementCount ([int] $assessment.chain_element_count) `
            -ChainStatusFlags @($assessment.chain_status_flags) `
            -AcceptedBoundaries $boundaries `
            -Context "Hostess current-host Authenticode assessment"
        if ($currentHostTrustBoundary -cne $assessment.validation_boundary) {
            throw "Hostess current-host Authenticode boundary is internally inconsistent"
        }
        if ($provenance.distribution.eligibility -ne "labs_signed_release" -or
            @($provenance.distribution.allowed_channels).Count -ne 1 -or
            @($provenance.distribution.allowed_channels)[0] -cne "labs" -or
            $provenance.distribution.stable_eligible -ne $false -or
            $provenance.signing.state -ne "accepted_exact_owner_signature" -or
            $provenance.signing.subject -cne $assessment.subject -or
            $provenance.signing.subject -cne $providerPolicy.signer.subject -or
            $provenance.signing.issuer -cne $providerPolicy.signer.issuer -or
            $provenance.signing.thumbprint_sha1 -cne
                $assessment.thumbprint.ToLowerInvariant() -or
            $provenance.signing.thumbprint_sha1 -cne
                $providerPolicy.signer.thumbprint_sha1.ToLowerInvariant() -or
            $provenance.signing.certificate_sha256 -cne
                $assessment.certificate_sha256 -or
            $provenance.signing.certificate_sha256 -cne
                $providerPolicy.signer.certificate_sha256 -or
            $provenance.signing.code_signing_eku_present -ne
                $assessment.code_signing_eku_present -or
            $provenance.signing.code_signing_eku_present -ne $true -or
            $provenance.signing.self_issued -ne $assessment.self_issued -or
            $provenance.signing.timestamp_present -ne $true -or
            $provenance.signing.public_trust_claim -ne
                $assessment.public_trust_claim -or
            $provenance.signing.public_trust_claim -ne $false -or
            $provenance.signing.trust_boundary -cne $recordedTrustBoundary) {
            throw "Hostess provenance does not authorize signed release distribution"
        }
    }
    else {
        $observedSignature = Get-AuthenticodeSignature -LiteralPath $providerFullPath
        if ($provenance.distribution.eligibility -ne "development_only" -or
        $provenance.source.availability_state -cne "unverified_development" -or
        $null -ne $provenance.source.verified_at_utc -or
        $provenance.signing.state -cne "unsigned" -or
        $provenance.signing.authenticode_status -cne "not_signed" -or
        $null -ne $provenance.signing.subject -or
        $null -ne $provenance.signing.issuer -or
        $null -ne $provenance.signing.thumbprint_sha1 -or
        $null -ne $provenance.signing.certificate_sha256 -or
        $provenance.signing.code_signing_eku_present -ne $false -or
        $null -ne $provenance.signing.self_issued -or
        $provenance.signing.timestamp_present -ne $false -or
        $provenance.signing.chain_trusted -ne $false -or
        [int] $provenance.signing.chain_element_count -ne 0 -or
        $provenance.signing.public_trust_claim -ne $false -or
        $provenance.signing.trust_boundary -cne "unsigned-development" -or
        @($provenance.signing.chain_status_flags).Count -ne 0 -or
        @($provenance.distribution.allowed_channels).Count -ne 0 -or
        $provenance.distribution.stable_eligible -ne $false -or
        $observedSignature.Status -ne
            [System.Management.Automation.SignatureStatus]::NotSigned -or
        $provenance.build.unsigned_artifact_sha256 -cne
            $provenance.artifact.sha256 -or
        [long] $provenance.build.unsigned_artifact_size_bytes -ne
            [long] $provenance.artifact.size_bytes) {
            throw "unsigned Hostess provenance must be an exact development-only artifact"
        }
    }
    if ($provenance.distribution.binary_authority -ne
        "rusty-hostess-github-releases") {
        throw "Hostess binary authority is not exact"
    }
    $provenanceText = Get-Content -LiteralPath $provenancePath -Raw
    if ($provenanceText -match "(?i)[a-z]:\\\\|\\\\\\\\") {
        throw "Hostess provenance contains a machine-private path"
    }

    [ordered]@{
        root = $metadataPath
        documents = $documents
        provenance = $provenance
        provenance_sha256 = Get-RustyFleetSha256 -LiteralPath $provenancePath
    }
}

function New-RustyFleetDeterministicZip {
    param(
        [Parameter(Mandatory)][string] $SourceDirectory,
        [Parameter(Mandatory)][string] $DestinationPath,
        [Parameter(Mandatory)][long] $SourceDateEpoch
    )

    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem

    $destinationParent = Split-Path -Parent $DestinationPath
    [System.IO.Directory]::CreateDirectory($destinationParent) | Out-Null
    if (Test-Path -LiteralPath $DestinationPath) {
        Remove-Item -LiteralPath $DestinationPath -Force
    }

    $minimumZipEpoch = [DateTimeOffset]::new(
        1980, 1, 1, 0, 0, 0, [TimeSpan]::Zero
    ).ToUnixTimeSeconds()
    $effectiveEpoch = [Math]::Max($minimumZipEpoch, $SourceDateEpoch)
    $entryTimestamp = [DateTimeOffset]::FromUnixTimeSeconds($effectiveEpoch)
    $sourceName = Split-Path -Leaf $SourceDirectory

    $archive = [System.IO.Compression.ZipFile]::Open(
        $DestinationPath,
        [System.IO.Compression.ZipArchiveMode]::Create
    )
    try {
        $files = Get-ChildItem -LiteralPath $SourceDirectory -File -Recurse |
            Sort-Object {
                Get-RustyFleetRelativePath -Root $SourceDirectory -LiteralPath $_.FullName
            }
        foreach ($file in $files) {
            $relative = Get-RustyFleetRelativePath `
                -Root $SourceDirectory `
                -LiteralPath $file.FullName
            $entry = $archive.CreateEntry(
                "$sourceName/$relative",
                [System.IO.Compression.CompressionLevel]::Optimal
            )
            $entry.LastWriteTime = $entryTimestamp
            $input = [System.IO.File]::OpenRead($file.FullName)
            $output = $entry.Open()
            try {
                $input.CopyTo($output)
            }
            finally {
                $output.Dispose()
                $input.Dispose()
            }
        }
    }
    finally {
        $archive.Dispose()
    }
}

Export-ModuleMember -Function @(
    "Get-RustyFleetSha256",
    "Write-RustyFleetUtf8",
    "ConvertTo-RustyFleetJson",
    "Get-RustyFleetRelativePath",
    "Assert-RustyFleetHttpsUrl",
    "Assert-RustyFleetSha256",
    "Assert-RustyFleetPayloadPath",
    "Get-RustyFleetPeCanonicalPayload",
    "Read-RustyFleetReleaseTrustPolicy",
    "Assert-RustyFleetAuthenticodeAssessment",
    "Get-RustyFleetAuthenticodeAssessment",
    "Read-RustyFleetHostessProvenance",
    "New-RustyFleetDeterministicZip"
)
