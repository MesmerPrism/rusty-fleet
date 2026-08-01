# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Pages([bool] $Condition, [string] $Message) {
    if (-not $Condition) {
        throw $Message
    }
}

function Write-TestUtf8([string] $LiteralPath, [string] $Content) {
    [IO.Directory]::CreateDirectory(
        (Split-Path -Parent $LiteralPath)
    ) | Out-Null
    [IO.File]::WriteAllText(
        $LiteralPath,
        $Content,
        [Text.UTF8Encoding]::new($false)
    )
}

function Get-TestSha256([string] $LiteralPath) {
    return (Get-FileHash -Algorithm SHA256 -LiteralPath $LiteralPath).
        Hash.ToLowerInvariant()
}

function ConvertTo-TestBase64Url([byte[]] $Bytes) {
    return [Convert]::ToBase64String($Bytes).
        TrimEnd("=").
        Replace("+", "-").
        Replace("/", "_")
}

$packagingRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Import-Module (Join-Path $packagingRoot "Distribution.Common.psm1") -Force
foreach ($case in @(
    [pscustomobject]@{ channel = "stable"; maturity = "released"; tag = "v9.9.9" },
    [pscustomobject]@{ channel = "labs"; maturity = "alpha"; tag = "v9.9.9-alpha.1" }
)) {
    $message = ""
    try {
        & (Join-Path $packagingRoot "New-WindowsPagesDeployment.ps1") `
            -Version "1.2.3" `
            -Channel $case.channel `
            -Maturity $case.maturity `
            -ReleaseTag $case.tag `
            -SiteDirectory "missing-site" `
            -MetadataDirectory "missing-metadata" `
            -PublicationPreflightReceiptPath "missing-preflight.json" `
            -ExpectedSourceRevision ("1" * 40) `
            -ExpectedSourceTree ("2" * 40) `
            -ExpectedDescriptorSignerSpkiSha256 ("3" * 64) `
            -OutputDirectory "unused-pages-output" | Out-Null
    }
    catch {
        $message = $_.Exception.Message
    }
    Assert-Pages `
        ($message -ceq "release tag does not bind the exact version and channel") `
        "$($case.channel) Pages staging accepted a cross-version release tag"
}
$testRoot = Join-Path ([IO.Path]::GetTempPath()) (
    "rusty-fleet-pages-test-$([Guid]::NewGuid().ToString('N'))"
)
[IO.Directory]::CreateDirectory($testRoot) | Out-Null
$rsa = [Security.Cryptography.RSA]::Create(3072)
$spkiBytes = $rsa.ExportSubjectPublicKeyInfo()
$spkiSha256 = [Convert]::ToHexString(
    [Security.Cryptography.SHA256]::HashData($spkiBytes)
).ToLowerInvariant()
$sourceRevision = "1" * 40
$sourceTree = "2" * 40
$setupSha256 = "3" * 64
$setupSignerCertificateSha256 = "4" * 64
$now = [DateTimeOffset]::UtcNow

function New-PagesMetadataFixture {
    param(
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][DateTimeOffset] $IssuedAtUtc,
        [Parameter(Mandatory)][int] $LifetimeMinutes,
        [Parameter(Mandatory)][string] $DescriptorId
    )

    $root = Join-Path $testRoot $Name
    $metadataRoot = Join-Path $root "metadata"
    [IO.Directory]::CreateDirectory($metadataRoot) | Out-Null
    $issuedAtMs = $IssuedAtUtc.ToUniversalTime().ToUnixTimeMilliseconds()
    $expiresAtMs = $IssuedAtUtc.ToUniversalTime().
        AddMinutes($LifetimeMinutes).ToUnixTimeMilliseconds()
    $durationMs = $expiresAtMs - $issuedAtMs
    $assetUrl = (
        "https://github.com/MesmerPrism/rusty-fleet/releases/download/" +
        "v1.2.3-alpha.1/RustyFleet-Labs-Setup.exe"
    )
    $payloadText = (
        '{"asset":{' +
        '"authenticode_trust_mode":"exact-pinned-self-issued-untrusted-root-only",' +
        '"installer_protocol":"rusty.fleet.guided_setup.v2",' +
        '"media_type":"application/vnd.microsoft.portable-executable",' +
        '"name":"RustyFleet-Labs-Setup.exe",' +
        '"public_trust_claim":false,' +
        '"sha256":"' + $setupSha256 + '",' +
        '"signer_certificate_sha256":"' +
            $setupSignerCertificateSha256 + '",' +
        '"signer_self_issued":true,' +
        '"signer_subject":"CN=MesmerPrism",' +
        '"signer_thumbprint":"08A5878AD6E652A94517D2C79144EB2655B0088C",' +
        '"size_bytes":1234,' +
        '"timestamp_required":true,' +
        '"url":"' + $assetUrl + '"},' +
        '"channel":"labs",' +
        '"distribution_track":"github-prerelease",' +
        '"descriptor_id":"' + $DescriptorId + '",' +
        '"expires_at_ms":' + $expiresAtMs + ',' +
        '"issued_at_ms":' + $issuedAtMs + ',' +
        '"maturity":"alpha",' +
        '"product":"rusty-fleet-labs",' +
        '"product_channel":"labs",' +
        '"schema":"rusty.fleet.windows_release.v4",' +
        '"validity_duration_ms":' + $durationMs + ',' +
        '"version":"1.2.3"}'
    )
    [byte[]] $payloadBytes = [Text.Encoding]::UTF8.GetBytes($payloadText)
    [byte[]] $signatureBytes = $rsa.SignData(
        $payloadBytes,
        [Security.Cryptography.HashAlgorithmName]::SHA256,
        [Security.Cryptography.RSASignaturePadding]::Pss
    )
    $envelope = [ordered]@{
        schema = "rusty.fleet.release_descriptor_envelope.v4"
        payload_base64url = ConvertTo-TestBase64Url $payloadBytes
        signature_base64url = ConvertTo-TestBase64Url $signatureBytes
        signer_spki_sha256 = $spkiSha256
    }
    $descriptorPath = Join-Path $metadataRoot "release.json"
    Write-TestUtf8 `
        -LiteralPath $descriptorPath `
        -Content (ConvertTo-RustyFleetJson -InputObject $envelope)
    $spkiPath = Join-Path $metadataRoot "release-descriptor.spki.der"
    [IO.File]::WriteAllBytes($spkiPath, $spkiBytes)
    $receipt = [ordered]@{
        schema = "rusty.fleet.windows_release_descriptor_receipt.v5"
        result = "pass"
        descriptor_id = $DescriptorId
        version = "1.2.3"
        product_channel = "labs"
        maturity = "alpha"
        channel = "labs"
        distribution_track = "github-prerelease"
        release_tag = "v1.2.3-alpha.1"
        installation_identity = "rusty-fleet-labs"
        primary_artifact = [ordered]@{
            role = "complete-product"
            name = "RustyFleet-Labs-Setup.exe"
            sha256 = $setupSha256
            bytes = 1234
            url = $assetUrl
        }
        issued_at_ms = $issuedAtMs
        expires_at_ms = $expiresAtMs
        validity_duration_ms = $durationMs
        setup_sha256 = $setupSha256
        setup_size_bytes = 1234
        setup_signer_certificate_sha256 = $setupSignerCertificateSha256
        setup_signer_subject = "CN=MesmerPrism"
        setup_signer_thumbprint = "08A5878AD6E652A94517D2C79144EB2655B0088C"
        setup_signer_self_issued = $true
        authenticode_trust_mode = "exact-pinned-self-issued-untrusted-root-only"
        public_trust_claim = $false
        timestamp_required = $true
        setup_build_receipt_sha256 = "5" * 64
        source_revision = $sourceRevision
        source_tree = $sourceTree
        canonical_pe_payload_sha256 = "6" * 64
        canonical_pe_payload_size_bytes = 1200
        descriptor_signer_spki_sha256 = $spkiSha256
        descriptor_signer_spki_asset = "release-descriptor.spki.der"
        payload_sha256 = [Convert]::ToHexString(
            [Security.Cryptography.SHA256]::HashData($payloadBytes)
        ).ToLowerInvariant()
        descriptor_sha256 = Get-TestSha256 $descriptorPath
        canonical_payload = "rfc8785_jcs_closed_shape"
        signature = "rsa_pss_sha256"
        pages_path = "Rusty-Fleet/metadata/labs/release.json"
        asset_url = $assetUrl
    }
    $receiptPath = Join-Path $metadataRoot (
        "release-descriptor.receipt.json"
    )
    Write-TestUtf8 `
        -LiteralPath $receiptPath `
        -Content (ConvertTo-RustyFleetJson -InputObject $receipt)

    $bundle = "RustyFleet-Labs-1.2.3-win-x64"
    $assets = @(
        [ordered]@{
            name = "RustyFleet-Labs-Setup.exe"
            sha256 = $setupSha256
            size_bytes = 1234
        },
        [ordered]@{
            name = "$bundle.zip"
            sha256 = "7" * 64
            size_bytes = 2345
        },
        [ordered]@{
            name = "$bundle.zip.sha256"
            sha256 = "8" * 64
            size_bytes = 100
        },
        [ordered]@{
            name = "$bundle.manifest.json"
            sha256 = "9" * 64
            size_bytes = 345
        },
        [ordered]@{
            name = "$bundle.checksums.sha256"
            sha256 = "a" * 64
            size_bytes = 456
        },
        [ordered]@{
            name = "$bundle.validation-receipt.json"
            sha256 = "b" * 64
            size_bytes = 567
        },
        [ordered]@{
            name = "RustyFleet-Labs-Setup.build-receipt.json"
            sha256 = "5" * 64
            size_bytes = 678
        },
        [ordered]@{
            name = "release.json"
            sha256 = Get-TestSha256 $descriptorPath
            size_bytes = (Get-Item -LiteralPath $descriptorPath).Length
        },
        [ordered]@{
            name = "release-descriptor.receipt.json"
            sha256 = Get-TestSha256 $receiptPath
            size_bytes = (Get-Item -LiteralPath $receiptPath).Length
        },
        [ordered]@{
            name = "release-descriptor.spki.der"
            sha256 = Get-TestSha256 $spkiPath
            size_bytes = (Get-Item -LiteralPath $spkiPath).Length
        }
    ) | Sort-Object name
    $preflight = [ordered]@{
        schema = "rusty.fleet.windows_publication_receipt.v3"
        result = "pass"
        mode = "preflight"
        version = "1.2.3"
        product_channel = "labs"
        maturity = "alpha"
        channel = "labs"
        distribution_track = "github-prerelease"
        tag = "v1.2.3-alpha.1"
        source_revision = $sourceRevision
        source_tree = $sourceTree
        setup_sha256 = $setupSha256
        bundle_sha256 = "7" * 64
        descriptor_sha256 = Get-TestSha256 $descriptorPath
        descriptor_receipt_sha256 = Get-TestSha256 $receiptPath
        descriptor_signer_spki_sha256 = $spkiSha256
        authenticode_trust_mode = "exact-pinned-self-issued-untrusted-root-only"
        signer_certificate_sha256 = $setupSignerCertificateSha256
        signer_self_issued = $true
        public_trust_claim = $false
        timestamp_required = $true
        asset_count = 10
        assets = $assets
        token_used = $false
        gh_invoked = $false
        draft_verified = $false
        visible_verified = $false
        remote_tag_verified = $false
        remote_integrity_verified = $false
        resumed_draft = $false
        uploaded_asset_count = 0
    }
    $preflightPath = Join-Path $root "publication-preflight.json"
    Write-TestUtf8 `
        -LiteralPath $preflightPath `
        -Content (ConvertTo-RustyFleetJson -InputObject $preflight)
    return [pscustomobject]@{
        metadata_root = $metadataRoot
        preflight_path = $preflightPath
    }
}

function Invoke-PagesFixture {
    param(
        [Parameter(Mandatory)][object] $Fixture,
        [Parameter(Mandatory)][string] $OutputDirectory,
        [string] $PreviousHandoffPath,
        [string] $ExistingDeploymentDirectory,
        [string] $Version = "1.2.3",
        [string] $ExpectedRevision = $sourceRevision,
        [string] $ExpectedSpki = $spkiSha256,
        [DateTimeOffset] $NowUtc = $now,
        [switch] $Resume
    )

    $arguments = @{
        Version = $Version
        Channel = "labs"
        Maturity = "alpha"
        ReleaseTag = "v1.2.3-alpha.1"
        SiteDirectory = $siteRoot
        MetadataDirectory = $Fixture.metadata_root
        PublicationPreflightReceiptPath = $Fixture.preflight_path
        ExpectedSourceRevision = $ExpectedRevision
        ExpectedSourceTree = $sourceTree
        ExpectedDescriptorSignerSpkiSha256 = $ExpectedSpki
        OutputDirectory = $OutputDirectory
        NowUtc = $NowUtc
    }
    if ($PreviousHandoffPath) {
        $arguments.PreviousHandoffPath = $PreviousHandoffPath
    }
    if ($ExistingDeploymentDirectory) {
        $arguments.ExistingDeploymentDirectory = $ExistingDeploymentDirectory
    }
    if ($Resume) {
        $arguments.Resume = $true
    }
    return & (Join-Path $packagingRoot "New-WindowsPagesDeployment.ps1") `
        @arguments
}

try {
    $siteRoot = Join-Path $testRoot "site"
    Write-TestUtf8 `
        -LiteralPath (Join-Path $siteRoot "index.html") `
        -Content "<!doctype html><title>Rusty Fleet test</title>"
    Write-TestUtf8 `
        -LiteralPath (Join-Path $siteRoot "styles.css") `
        -Content "body { color: black; }"

    $first = New-PagesMetadataFixture `
        -Name "first" `
        -IssuedAtUtc $now.AddMinutes(-5) `
        -LifetimeMinutes 1380 `
        -DescriptorId "v1.2.3-labs-first"
    $firstOutput = Join-Path $testRoot "pages-first"
    $existingSite = Join-Path $testRoot "existing-complete-site"
    $stableSentinel = Join-Path $existingSite "Rusty-Fleet\metadata\stable\release.json"
    Write-TestUtf8 -LiteralPath $stableSentinel -Content '{"stable":"byte-exact"}'
    $stableSentinelHash = Get-RustyFleetSha256 -LiteralPath $stableSentinel
    $firstHandoff = Invoke-PagesFixture `
        -Fixture $first `
        -OutputDirectory $firstOutput `
        -ExistingDeploymentDirectory $existingSite |
        ConvertFrom-Json -Depth 20
    $firstHandoffPath = Join-Path $firstOutput (
        "Rusty-Fleet\metadata\labs\deployment-handoff.json"
    )
    Assert-Pages (
        $firstHandoff.result -eq "pass" -and
        $firstHandoff.deployment_sequence -eq 1 -and
        $firstHandoff.pages_binary_count -eq 0 -and
        @($firstHandoff.release_files).Count -eq 5 -and
        (Test-Path -LiteralPath $firstHandoffPath -PathType Leaf) -and
        (Get-RustyFleetSha256 -LiteralPath (
            Join-Path $firstOutput "Rusty-Fleet\metadata\stable\release.json"
        )) -ceq $stableSentinelHash -and
        (
            Get-Content -LiteralPath (
                Join-Path $firstOutput (
                    "Rusty-Fleet\metadata\labs\" +
                    "release-descriptor.receipt.json"
                )
            ) -Raw | ConvertFrom-Json
        ).primary_artifact.role -ceq "complete-product"
    ) "initial Pages handoff is not exact"

    $resumed = Invoke-PagesFixture `
        -Fixture $first `
        -OutputDirectory $firstOutput `
        -Resume |
        ConvertFrom-Json -Depth 20
    Assert-Pages (
        $resumed.deployment_id -ceq $firstHandoff.deployment_id -and
        $resumed.deployment_sequence -eq 1
    ) "completed deployment did not resume idempotently"

    $interruptedOutput = Join-Path $testRoot "pages-interrupted"
    $interruptedStage = (
        "$interruptedOutput.staging-$($firstHandoff.deployment_id)"
    )
    Write-TestUtf8 `
        -LiteralPath (Join-Path $interruptedStage "partial.txt") `
        -Content "interrupted"
    $recovered = Invoke-PagesFixture `
        -Fixture $first `
        -OutputDirectory $interruptedOutput `
        -Resume |
        ConvertFrom-Json -Depth 20
    Assert-Pages (
        $recovered.deployment_id -ceq $firstHandoff.deployment_id -and
        -not (Test-Path -LiteralPath $interruptedStage)
    ) "interrupted deployment did not resume"

    $binarySite = Join-Path $testRoot "site-binary"
    Copy-Item -LiteralPath $siteRoot -Destination $binarySite -Recurse
    Write-TestUtf8 `
        -LiteralPath (Join-Path $binarySite "RustyFleet-Setup.exe") `
        -Content "prohibited"
    $siteRoot = $binarySite
    $binaryRejected = $false
    try {
        Invoke-PagesFixture `
            -Fixture $first `
            -OutputDirectory (Join-Path $testRoot "pages-binary") |
            Out-Null
    }
    catch {
        $binaryRejected = $true
    }
    Assert-Pages $binaryRejected "Pages accepted a binary payload"
    $siteRoot = Join-Path $testRoot "site"

    foreach ($wrong in @(
        [pscustomobject]@{
            name = "source"
            version = "1.2.3"
            revision = "0" * 40
            spki = $spkiSha256
        },
        [pscustomobject]@{
            name = "tag"
            version = "1.2.4"
            revision = $sourceRevision
            spki = $spkiSha256
        },
        [pscustomobject]@{
            name = "signer"
            version = "1.2.3"
            revision = $sourceRevision
            spki = "0" * 64
        }
    )) {
        $rejected = $false
        try {
            Invoke-PagesFixture `
                -Fixture $first `
                -OutputDirectory (
                    Join-Path $testRoot "pages-wrong-$($wrong.name)"
                ) `
                -Version $wrong.version `
                -ExpectedRevision $wrong.revision `
                -ExpectedSpki $wrong.spki |
                Out-Null
        }
        catch {
            $rejected = $true
        }
        Assert-Pages $rejected "Pages accepted a wrong $($wrong.name) binding"
    }

    foreach ($ownerMutation in @(
        [pscustomobject]@{
            name = "release-tag"
            apply = {
                param($value)
                $value.release_tag = "v1.2.4"
            }
        },
        [pscustomobject]@{
            name = "installation-identity"
            apply = {
                param($value)
                $value.installation_identity = "rusty-fleet"
            }
        },
        [pscustomobject]@{
            name = "primary-artifact"
            apply = {
                param($value)
                $value.primary_artifact.bytes = 1235
            }
        }
    )) {
        $ownerRoot = Join-Path $testRoot (
            "owner-metadata-$($ownerMutation.name)"
        )
        Copy-Item `
            -LiteralPath $first.metadata_root `
            -Destination $ownerRoot `
            -Recurse
        $ownerReceiptPath = Join-Path $ownerRoot (
            "release-descriptor.receipt.json"
        )
        $ownerReceipt = Get-Content -LiteralPath $ownerReceiptPath -Raw |
            ConvertFrom-Json -Depth 20
        & $ownerMutation.apply $ownerReceipt
        Write-TestUtf8 `
            -LiteralPath $ownerReceiptPath `
            -Content (ConvertTo-RustyFleetJson -InputObject $ownerReceipt)
        $ownerPreflight = Get-Content -LiteralPath $first.preflight_path -Raw |
            ConvertFrom-Json -Depth 20
        $ownerReceiptSha256 = Get-TestSha256 $ownerReceiptPath
        $ownerReceiptSize = (Get-Item -LiteralPath $ownerReceiptPath).Length
        $ownerReceiptAsset = @(
            $ownerPreflight.assets |
                Where-Object name -CEQ "release-descriptor.receipt.json"
        )
        $ownerReceiptAsset[0].sha256 = $ownerReceiptSha256
        $ownerReceiptAsset[0].size_bytes = $ownerReceiptSize
        $ownerPreflight.descriptor_receipt_sha256 = $ownerReceiptSha256
        $ownerPreflightPath = Join-Path $testRoot (
            "owner-preflight-$($ownerMutation.name).json"
        )
        Write-TestUtf8 `
            -LiteralPath $ownerPreflightPath `
            -Content (ConvertTo-RustyFleetJson -InputObject $ownerPreflight)
        $ownerRejected = $false
        try {
            Invoke-PagesFixture `
                -Fixture ([pscustomobject]@{
                    metadata_root = $ownerRoot
                    preflight_path = $ownerPreflightPath
                }) `
                -OutputDirectory (
                    Join-Path $testRoot "pages-owner-$($ownerMutation.name)"
                ) |
                Out-Null
        }
        catch {
            $ownerRejected = $true
        }
        Assert-Pages `
            $ownerRejected `
            "Pages accepted wrong owner $($ownerMutation.name)"
    }

    $damaged = Join-Path $testRoot "damaged"
    Copy-Item `
        -LiteralPath $first.metadata_root `
        -Destination $damaged `
        -Recurse
    [IO.File]::AppendAllText(
        (Join-Path $damaged "release.json"),
        "damage",
        [Text.UTF8Encoding]::new($false)
    )
    $damagedFixture = [pscustomobject]@{
        metadata_root = $damaged
        preflight_path = $first.preflight_path
    }
    $assetRejected = $false
    try {
        Invoke-PagesFixture `
            -Fixture $damagedFixture `
            -OutputDirectory (Join-Path $testRoot "pages-damaged") |
            Out-Null
    }
    catch {
        $assetRejected = $true
    }
    Assert-Pages $assetRejected "Pages accepted a damaged descriptor asset"

    $stale = New-PagesMetadataFixture `
        -Name "stale" `
        -IssuedAtUtc $now.AddDays(-2) `
        -LifetimeMinutes 60 `
        -DescriptorId "v1.2.3-dev-stale"
    $staleRejected = $false
    try {
        Invoke-PagesFixture `
            -Fixture $stale `
            -OutputDirectory (Join-Path $testRoot "pages-stale") |
            Out-Null
    }
    catch {
        $staleRejected = $true
    }
    Assert-Pages $staleRejected "Pages accepted stale release metadata"

    $renewal = New-PagesMetadataFixture `
        -Name "renewal" `
        -IssuedAtUtc $now.AddMinutes(5) `
        -LifetimeMinutes 1380 `
        -DescriptorId "v1.2.3-dev-renewal"
    $renewed = Invoke-PagesFixture `
        -Fixture $renewal `
        -OutputDirectory (Join-Path $testRoot "pages-renewal") `
        -PreviousHandoffPath $firstHandoffPath `
        -NowUtc $now.AddMinutes(5) |
        ConvertFrom-Json -Depth 20
    Assert-Pages (
        $renewed.deployment_sequence -eq 2 -and
        $renewed.previous_handoff_sha256 -cmatch "^[0-9a-f]{64}$" -and
        $renewed.descriptor_id -ceq "v1.2.3-dev-renewal" -and
        $renewed.expires_at_ms -gt $firstHandoff.expires_at_ms
    ) "fresh release metadata did not renew"

    $replayRejected = $false
    try {
        Invoke-PagesFixture `
            -Fixture $first `
            -OutputDirectory (Join-Path $testRoot "pages-replay") `
            -PreviousHandoffPath $firstHandoffPath |
            Out-Null
    }
    catch {
        $replayRejected = $true
    }
    Assert-Pages $replayRejected "Pages accepted a descriptor replay"

    [ordered]@{
        schema = "rusty.fleet.windows_pages_deployment_test.v1"
        result = "pass"
        renewal_verified = $true
        stale_metadata_rejected = $true
        wrong_source_tag_asset_signer_rejected = $true
        owner_release_identity_verified = $true
        owner_release_identity_substitution_rejected = $true
        replay_rejected = $true
        pages_binary_count = 0
        completed_resume_verified = $true
        interrupted_stage_resume_verified = $true
        stable_subtree_byte_exact = $true
    } | ConvertTo-Json -Depth 5
}
finally {
    $rsa.Dispose()
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
