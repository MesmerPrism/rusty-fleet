# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9]+\.[0-9]+\.[0-9]+$")]
    [string] $Version,

    [Parameter(Mandatory)]
    [ValidateSet("dev", "alpha", "preview", "stable")]
    [string] $Channel,

    [ValidatePattern("^v[0-9]+\.[0-9]+\.[0-9]+(?:-alpha\.[1-9][0-9]*)?$")]
    [string] $ReleaseTag,

    [Parameter(Mandatory)]
    [string] $SiteDirectory,

    [Parameter(Mandatory)]
    [string] $MetadataDirectory,

    [Parameter(Mandatory)]
    [string] $PublicationPreflightReceiptPath,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9a-f]{40}$")]
    [string] $ExpectedSourceRevision,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9a-f]{40}$")]
    [string] $ExpectedSourceTree,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9a-f]{64}$")]
    [string] $ExpectedDescriptorSignerSpkiSha256,

    [Parameter(Mandatory)]
    [string] $OutputDirectory,

    [string] $ExistingDeploymentDirectory,

    [string] $PreviousHandoffPath,

    [DateTimeOffset] $NowUtc = [DateTimeOffset]::UtcNow,

    [ValidateRange(1, 720)]
    [int] $MinimumRemainingMinutes = 60,

    [switch] $Resume
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (-not $ReleaseTag) { $ReleaseTag = "v$Version" }
$expectedReleaseTagPattern = if ($Channel -eq "alpha") {
    "^v$([regex]::Escape($Version))-alpha\.[1-9][0-9]*$"
}
else {
    "^v$([regex]::Escape($Version))$"
}
if ($ReleaseTag -cnotmatch $expectedReleaseTagPattern) {
    throw "release tag does not bind the exact version and channel"
}
$productStem = if ($Channel -eq "alpha") { "RustyFleet-Alpha" } else { "RustyFleet" }
$setupName = if ($Channel -eq "alpha") { "RustyFleet-Alpha-Setup.exe" } else { "RustyFleet-Setup.exe" }
$setupReceiptName = if ($Channel -eq "alpha") { "RustyFleet-Alpha-Setup.build-receipt.json" } else { "RustyFleet-Setup.build-receipt.json" }
$installationIdentity = if ($Channel -eq "alpha") { "rusty-fleet-alpha" } else { "rusty-fleet" }
Import-Module (Join-Path $PSScriptRoot "Distribution.Common.psm1") -Force

function Assert-ExactProperties {
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

function Read-StrictJson {
    param(
        [Parameter(Mandatory)][string] $LiteralPath,
        [Parameter(Mandatory)][int] $MaximumBytes,
        [Parameter(Mandatory)][string] $Context
    )

    $resolved = (Resolve-Path -LiteralPath $LiteralPath).Path
    [byte[]] $bytes = [IO.File]::ReadAllBytes($resolved)
    if ($bytes.Length -le 0 -or $bytes.Length -gt $MaximumBytes) {
        throw "$Context size is outside its bound"
    }
    try {
        $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
        $value = $text | ConvertFrom-Json -Depth 20
    }
    catch {
        throw "$Context is not strict UTF-8 JSON"
    }
    return [pscustomobject]@{
        bytes = $bytes
        value = $value
    }
}

function Get-BytesSha256 {
    param([Parameter(Mandatory)][byte[]] $Bytes)

    return [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData($Bytes)
    ).ToLowerInvariant()
}

function ConvertFrom-Base64Url {
    param(
        [Parameter(Mandatory)][object] $Value,
        [Parameter(Mandatory)][string] $Context
    )

    if ($Value -isnot [string] -or
        [string]::IsNullOrWhiteSpace($Value) -or
        $Value -cnotmatch "^[A-Za-z0-9_-]+$") {
        throw "$Context is not canonical base64url"
    }
    $padded = $Value.Replace("-", "+").Replace("_", "/")
    $padded += "=" * ((4 - ($padded.Length % 4)) % 4)
    try {
        return [Convert]::FromBase64String($padded)
    }
    catch {
        throw "$Context is malformed"
    }
}

function Assert-NoPagesBinary {
    param(
        [Parameter(Mandatory)][string] $Root,
        [switch] $AllowDescriptorSpki
    )

    $rootPath = (Resolve-Path -LiteralPath $Root).Path
    $files = @(Get-ChildItem -LiteralPath $rootPath -Recurse -File -Force)
    foreach ($entry in @(Get-ChildItem -LiteralPath $rootPath -Recurse -Force)) {
        if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Pages input contains a reparse point"
        }
    }
    foreach ($file in $files) {
        $relative = [IO.Path]::GetRelativePath(
            $rootPath,
            $file.FullName
        ).Replace("\", "/")
        $extension = $file.Extension.ToLowerInvariant()
        $isAllowedSpki = (
            $AllowDescriptorSpki -and
            $relative -cmatch (
                "^Rusty-Fleet/metadata/(?:dev|alpha|preview|stable)/" +
                "release-descriptor\.spki\.der$"
            )
        )
        if (
            $extension -cin @(
                ".exe", ".dll", ".msi", ".msix", ".appx", ".zip", ".7z",
                ".tar", ".gz", ".apk", ".aab", ".pfx", ".p12", ".pem",
                ".key", ".cer", ".crt", ".der"
            ) -and
            -not $isAllowedSpki
        ) {
            throw "Pages payload contains a prohibited binary or key asset"
        }
        if ($file.Length -gt 16777216) {
            throw "Pages payload contains an oversized file"
        }
    }
}

function Get-ReleaseAssetMap {
    param([Parameter(Mandatory)][object] $Preflight)

    $expectedNames = @(
        $setupName,
        "$productStem-$Version-win-x64.zip",
        "$productStem-$Version-win-x64.zip.sha256",
        "$productStem-$Version-win-x64.manifest.json",
        "$productStem-$Version-win-x64.checksums.sha256",
        "$productStem-$Version-win-x64.validation-receipt.json",
        $setupReceiptName,
        "release.json",
        "release-descriptor.receipt.json",
        "release-descriptor.spki.der"
    ) | Sort-Object
    $assetMap = [Collections.Generic.Dictionary[string, object]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($asset in @($Preflight.assets)) {
        Assert-ExactProperties -InputObject $asset -Expected @(
            "name", "sha256", "size_bytes"
        ) -Context "publication preflight asset"
        if ($asset.name -isnot [string] -or
            $asset.name -cnotmatch "^[A-Za-z0-9._-]+$" -or
            $asset.sha256 -isnot [string] -or
            $asset.sha256 -cnotmatch "^[0-9a-f]{64}$" -or
            [long] $asset.size_bytes -le 0 -or
            -not $assetMap.TryAdd([string] $asset.name, $asset)) {
            throw "publication preflight asset inventory is malformed"
        }
    }
    $actualNames = @($assetMap.Keys | Sort-Object)
    if (($actualNames -join "`n") -cne ($expectedNames -join "`n")) {
        throw "publication preflight asset inventory is not closed"
    }
    return $assetMap
}

function Assert-MetadataFile {
    param(
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][object] $ExpectedAsset,
        [Parameter(Mandatory)][string] $Root
    )

    $path = Join-Path $Root $Name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "release metadata file is missing: $Name"
    }
    $item = Get-Item -LiteralPath $path
    if ($item.Length -ne [long] $ExpectedAsset.size_bytes -or
        (Get-RustyFleetSha256 -LiteralPath $path) -cne
            [string] $ExpectedAsset.sha256) {
        throw "release metadata file does not match publication preflight: $Name"
    }
}

function Get-HandoffReleaseFile {
    param(
        [Parameter(Mandatory)][string] $Role,
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][string] $Authority,
        [Parameter(Mandatory)][object] $Asset
    )

    return [ordered]@{
        role = $Role
        name = $Name
        sha256 = [string] $Asset.sha256
        size_bytes = [long] $Asset.size_bytes
        authority = $Authority
    }
}

$siteRoot = (Resolve-Path -LiteralPath $SiteDirectory).Path
$metadataRoot = (Resolve-Path -LiteralPath $MetadataDirectory).Path
$metadataEntries = @(Get-ChildItem -LiteralPath $metadataRoot -Force)
$expectedMetadataNames = @(
    "release-descriptor.receipt.json",
    "release-descriptor.spki.der",
    "release.json"
) | Sort-Object
$actualMetadataNames = @($metadataEntries.Name | Sort-Object)
if ($metadataEntries.Count -ne 3 -or
    @($metadataEntries | Where-Object { -not $_.PSIsContainer }).Count -ne 3 -or
    ($actualMetadataNames -join "`n") -cne
        ($expectedMetadataNames -join "`n")) {
    throw "release metadata directory is not exact"
}
Assert-NoPagesBinary -Root $siteRoot

$preflightJson = Read-StrictJson `
    -LiteralPath $PublicationPreflightReceiptPath `
    -MaximumBytes 1048576 `
    -Context "publication preflight receipt"
$preflight = $preflightJson.value
Assert-ExactProperties -InputObject $preflight -Expected @(
    "schema",
    "result",
    "mode",
    "version",
    "channel",
    "tag",
    "source_revision",
    "source_tree",
    "setup_sha256",
    "bundle_sha256",
    "descriptor_sha256",
    "descriptor_receipt_sha256",
    "descriptor_signer_spki_sha256",
    "asset_count",
    "assets",
    "token_used",
    "gh_invoked",
    "draft_verified",
    "visible_verified",
    "remote_tag_verified",
    "remote_integrity_verified",
    "resumed_draft",
    "uploaded_asset_count"
) -Context "publication preflight receipt"
if ($preflight.schema -cne
        "rusty.fleet.windows_publication_receipt.v1" -or
    $preflight.result -cne "pass" -or
    $preflight.mode -cne "preflight" -or
    $preflight.version -cne $Version -or
    $preflight.channel -cne $Channel -or
    $preflight.tag -cne $ReleaseTag -or
    $preflight.source_revision -cne $ExpectedSourceRevision -or
    $preflight.source_tree -cne $ExpectedSourceTree -or
    $preflight.descriptor_signer_spki_sha256 -cne
        $ExpectedDescriptorSignerSpkiSha256 -or
    [long] $preflight.asset_count -ne 10 -or
    $preflight.token_used -ne $false -or
    $preflight.gh_invoked -ne $false) {
    throw "publication preflight receipt is not an exact token-free pass"
}
$assetMap = Get-ReleaseAssetMap -Preflight $preflight
foreach ($name in $expectedMetadataNames) {
    Assert-MetadataFile `
        -Name $name `
        -ExpectedAsset $assetMap[$name] `
        -Root $metadataRoot
}

$descriptorPath = Join-Path $metadataRoot "release.json"
$descriptorBytes = [IO.File]::ReadAllBytes($descriptorPath)
$descriptorSha256 = Get-BytesSha256 -Bytes $descriptorBytes
if ($descriptorSha256 -cne $preflight.descriptor_sha256) {
    throw "release descriptor hash does not match publication preflight"
}
$envelope = (
    Read-StrictJson `
        -LiteralPath $descriptorPath `
        -MaximumBytes 65536 `
        -Context "release descriptor"
).value
Assert-ExactProperties -InputObject $envelope -Expected @(
    "schema",
    "payload_base64url",
    "signature_base64url",
    "signer_spki_sha256"
) -Context "release descriptor envelope"
if ($envelope.schema -cne
        "rusty.fleet.release_descriptor_envelope.v2" -or
    $envelope.signer_spki_sha256 -cne
        $ExpectedDescriptorSignerSpkiSha256) {
    throw "release descriptor signer identity is not exact"
}

$payloadBytes = ConvertFrom-Base64Url `
    -Value $envelope.payload_base64url `
    -Context "release descriptor payload"
$signatureBytes = ConvertFrom-Base64Url `
    -Value $envelope.signature_base64url `
    -Context "release descriptor signature"
try {
    $payload = [Text.UTF8Encoding]::new(
        $false,
        $true
    ).GetString($payloadBytes) | ConvertFrom-Json -Depth 10
}
catch {
    throw "release descriptor payload is not strict UTF-8 JSON"
}
Assert-ExactProperties -InputObject $payload -Expected @(
    "asset",
    "channel",
    "descriptor_id",
    "expires_at_ms",
    "issued_at_ms",
    "product",
    "schema",
    "validity_duration_ms",
    "version"
) -Context "release descriptor payload"
Assert-ExactProperties -InputObject $payload.asset -Expected @(
    "installer_protocol",
    "media_type",
    "name",
    "sha256",
    "signer_certificate_sha256",
    "size_bytes",
    "url"
) -Context "release descriptor asset"
$expectedSetupUrl = (
    "https://github.com/MesmerPrism/rusty-fleet/releases/download/" +
    "$ReleaseTag/$setupName"
)
$nowMs = $NowUtc.ToUniversalTime().ToUnixTimeMilliseconds()
$minimumRemainingMs = [long] $MinimumRemainingMinutes * 60000
if ($payload.schema -cne "rusty.fleet.windows_release.v2" -or
    $payload.product -cne "rusty-fleet" -or
    $payload.version -cne $Version -or
    $payload.channel -cne $Channel -or
    $payload.descriptor_id -isnot [string] -or
    $payload.descriptor_id -cnotmatch "^[A-Za-z0-9._-]{1,128}$" -or
    [long] $payload.issued_at_ms -le 0 -or
    [long] $payload.issued_at_ms -gt $nowMs + 300000 -or
    [long] $payload.expires_at_ms -le [long] $payload.issued_at_ms -or
    [long] $payload.expires_at_ms -lt $nowMs + $minimumRemainingMs -or
    [long] $payload.validity_duration_ms -ne
        ([long] $payload.expires_at_ms - [long] $payload.issued_at_ms) -or
    [long] $payload.validity_duration_ms -gt 86400000 -or
    $payload.asset.installer_protocol -cne
        "rusty.fleet.guided_setup.v1" -or
    $payload.asset.media_type -cne
        "application/vnd.microsoft.portable-executable" -or
    $payload.asset.name -cne $setupName -or
    $payload.asset.sha256 -cne $preflight.setup_sha256 -or
    [long] $payload.asset.size_bytes -ne
        [long] $assetMap[$setupName].size_bytes -or
    $payload.asset.url -cne $expectedSetupUrl -or
    $payload.asset.signer_certificate_sha256 -isnot [string] -or
    $payload.asset.signer_certificate_sha256 -cnotmatch "^[0-9a-f]{64}$") {
    throw "release descriptor payload is stale or not exact"
}

[byte[]] $spkiBytes = [IO.File]::ReadAllBytes(
    (Join-Path $metadataRoot "release-descriptor.spki.der")
)
if ((Get-BytesSha256 -Bytes $spkiBytes) -cne
    $ExpectedDescriptorSignerSpkiSha256) {
    throw "release descriptor SPKI does not match independent authorization"
}
$rsa = [Security.Cryptography.RSA]::Create()
try {
    $read = 0
    $rsa.ImportSubjectPublicKeyInfo($spkiBytes, [ref] $read)
    if ($read -ne $spkiBytes.Length -or
        $rsa.KeySize -lt 3072 -or
        -not $rsa.VerifyData(
            $payloadBytes,
            $signatureBytes,
            [Security.Cryptography.HashAlgorithmName]::SHA256,
            [Security.Cryptography.RSASignaturePadding]::Pss
        )) {
        throw "release descriptor RSA-PSS signature is invalid"
    }
}
finally {
    $rsa.Dispose()
}

$descriptorReceiptPath = Join-Path $metadataRoot (
    "release-descriptor.receipt.json"
)
$descriptorReceiptJson = Read-StrictJson `
    -LiteralPath $descriptorReceiptPath `
    -MaximumBytes 65536 `
    -Context "release descriptor receipt"
$descriptorReceipt = $descriptorReceiptJson.value
Assert-ExactProperties -InputObject $descriptorReceipt -Expected @(
    "schema",
    "result",
    "descriptor_id",
    "version",
    "channel",
    "release_tag",
    "installation_identity",
    "primary_artifact",
    "issued_at_ms",
    "expires_at_ms",
    "validity_duration_ms",
    "setup_sha256",
    "setup_size_bytes",
    "setup_signer_certificate_sha256",
    "setup_build_receipt_sha256",
    "source_revision",
    "source_tree",
    "canonical_pe_payload_sha256",
    "canonical_pe_payload_size_bytes",
    "descriptor_signer_spki_sha256",
    "descriptor_signer_spki_asset",
    "payload_sha256",
    "descriptor_sha256",
    "canonical_payload",
    "signature",
    "pages_path",
    "asset_url"
) -Context "release descriptor receipt"
Assert-ExactProperties -InputObject $descriptorReceipt.primary_artifact -Expected @(
    "role",
    "name",
    "sha256",
    "bytes",
    "url"
) -Context "release descriptor primary artifact"
if ($descriptorReceipt.schema -cne
        "rusty.fleet.windows_release_descriptor_receipt.v3" -or
    $descriptorReceipt.result -cne "pass" -or
    $descriptorReceipt.version -cne $Version -or
    $descriptorReceipt.channel -cne $Channel -or
    $descriptorReceipt.release_tag -cne $ReleaseTag -or
    $descriptorReceipt.installation_identity -cne
        $installationIdentity -or
    $descriptorReceipt.primary_artifact.role -cne
        "complete-product" -or
    $descriptorReceipt.primary_artifact.name -cne $setupName -or
    $descriptorReceipt.primary_artifact.sha256 -cne
        $preflight.setup_sha256 -or
    [long] $descriptorReceipt.primary_artifact.bytes -ne
        [long] $assetMap[$setupName].size_bytes -or
    $descriptorReceipt.primary_artifact.url -cne $expectedSetupUrl -or
    $descriptorReceipt.descriptor_id -cne $payload.descriptor_id -or
    [long] $descriptorReceipt.issued_at_ms -ne
        [long] $payload.issued_at_ms -or
    [long] $descriptorReceipt.expires_at_ms -ne
        [long] $payload.expires_at_ms -or
    $descriptorReceipt.source_revision -cne $ExpectedSourceRevision -or
    $descriptorReceipt.source_tree -cne $ExpectedSourceTree -or
    $descriptorReceipt.setup_sha256 -cne $preflight.setup_sha256 -or
    $descriptorReceipt.descriptor_signer_spki_sha256 -cne
        $ExpectedDescriptorSignerSpkiSha256 -or
    $descriptorReceipt.descriptor_sha256 -cne $descriptorSha256 -or
    $descriptorReceipt.pages_path -cne
        "Rusty-Fleet/metadata/$Channel/release.json") {
    throw "release descriptor receipt is not exact"
}
if ((Get-BytesSha256 -Bytes $descriptorReceiptJson.bytes) -cne
    $preflight.descriptor_receipt_sha256) {
    throw "release descriptor receipt hash does not match publication preflight"
}

$deploymentSequence = 1L
$previousHandoffSha256 = $null
if ($PreviousHandoffPath) {
    $previousJson = Read-StrictJson `
        -LiteralPath $PreviousHandoffPath `
        -MaximumBytes 1048576 `
        -Context "previous Pages handoff"
    $previous = $previousJson.value
    Assert-ExactProperties -InputObject $previous -Expected @(
        "schema",
        "result",
        "deployment_id",
        "deployment_sequence",
        "previous_handoff_sha256",
        "product",
        "version",
        "channel",
        "tag",
        "source_revision",
        "source_tree",
        "descriptor_id",
        "issued_at_ms",
        "expires_at_ms",
        "validity_duration_ms",
        "descriptor_signer_spki_sha256",
        "setup_signer_certificate_sha256",
        "binary_authority",
        "pages_binary_count",
        "pages_path",
        "publication_preflight_receipt_sha256",
        "release_files"
    ) -Context "previous Pages handoff"
    if ($previous.schema -cne
            "rusty.fleet.windows_release_metadata_handoff.v1" -or
        $previous.result -cne "pass" -or
        $previous.product -cne "rusty-fleet" -or
        $previous.channel -cne $Channel -or
        $previous.version -isnot [string] -or
        $previous.version -cnotmatch "^[0-9]+\.[0-9]+\.[0-9]+$" -or
        [version] $Version -lt [version] $previous.version -or
        [long] $previous.deployment_sequence -lt 1 -or
        [long] $payload.issued_at_ms -le [long] $previous.issued_at_ms -or
        [long] $payload.expires_at_ms -le [long] $previous.expires_at_ms -or
        $payload.descriptor_id -ceq $previous.descriptor_id) {
        throw "release metadata renewal is stale, downgraded, or replayed"
    }
    $previousDescriptor = @(
        $previous.release_files |
            Where-Object { $_.role -ceq "descriptor" }
    )
    if ($previousDescriptor.Count -ne 1 -or
        $previousDescriptor[0].sha256 -ceq $descriptorSha256) {
        throw "release metadata renewal replays the prior descriptor"
    }
    $deploymentSequence = [long] $previous.deployment_sequence + 1
    $previousHandoffSha256 = Get-BytesSha256 -Bytes $previousJson.bytes
}

$releaseFiles = @(
    Get-HandoffReleaseFile `
        -Role "setup" `
        -Name $setupName `
        -Authority "github_releases" `
        -Asset $assetMap[$setupName]
    Get-HandoffReleaseFile `
        -Role "setup_build_receipt" `
        -Name $setupReceiptName `
        -Authority "github_releases" `
        -Asset $assetMap[$setupReceiptName]
    Get-HandoffReleaseFile `
        -Role "descriptor" `
        -Name "release.json" `
        -Authority "github_pages" `
        -Asset $assetMap["release.json"]
    Get-HandoffReleaseFile `
        -Role "descriptor_receipt" `
        -Name "release-descriptor.receipt.json" `
        -Authority "github_pages" `
        -Asset $assetMap["release-descriptor.receipt.json"]
    Get-HandoffReleaseFile `
        -Role "descriptor_spki" `
        -Name "release-descriptor.spki.der" `
        -Authority "github_pages" `
        -Asset $assetMap["release-descriptor.spki.der"]
)
$deploymentId = (
    "$Channel-$($descriptorSha256.Substring(0, 24))"
)
$handoff = [ordered]@{
    schema = "rusty.fleet.windows_release_metadata_handoff.v1"
    result = "pass"
    deployment_id = $deploymentId
    deployment_sequence = $deploymentSequence
    previous_handoff_sha256 = $previousHandoffSha256
    product = "rusty-fleet"
    version = $Version
    channel = $Channel
    tag = $ReleaseTag
    source_revision = $ExpectedSourceRevision
    source_tree = $ExpectedSourceTree
    descriptor_id = [string] $payload.descriptor_id
    issued_at_ms = [long] $payload.issued_at_ms
    expires_at_ms = [long] $payload.expires_at_ms
    validity_duration_ms = [long] $payload.validity_duration_ms
    descriptor_signer_spki_sha256 =
        $ExpectedDescriptorSignerSpkiSha256
    setup_signer_certificate_sha256 =
        [string] $payload.asset.signer_certificate_sha256
    binary_authority = "github_releases"
    pages_binary_count = 0
    pages_path = "Rusty-Fleet/metadata/$Channel"
    publication_preflight_receipt_sha256 =
        (Get-BytesSha256 -Bytes $preflightJson.bytes)
    release_files = $releaseFiles
}
$handoffText = ConvertTo-RustyFleetJson -InputObject $handoff
$outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
$outputParent = Split-Path -Parent $outputRoot
if ([string]::IsNullOrWhiteSpace($outputParent)) {
    throw "Pages output parent is unavailable"
}
[IO.Directory]::CreateDirectory($outputParent) | Out-Null
$stagingRoot = "$outputRoot.staging-$deploymentId"
if (Test-Path -LiteralPath $outputRoot) {
    if (-not $Resume) {
        throw "refusing to overwrite an existing Pages deployment output"
    }
    $existingHandoffPath = Join-Path $outputRoot (
        "Rusty-Fleet\metadata\$Channel\deployment-handoff.json"
    )
    if (-not (Test-Path -LiteralPath $existingHandoffPath -PathType Leaf) -or
        (Get-Content -LiteralPath $existingHandoffPath -Raw) -cne
            $handoffText) {
        throw "existing Pages deployment output is not an exact resumable handoff"
    }
    Assert-NoPagesBinary -Root $outputRoot -AllowDescriptorSpki
    $handoff | ConvertTo-Json -Depth 10
    return
}
if (Test-Path -LiteralPath $stagingRoot) {
    if (-not $Resume) {
        throw "an interrupted Pages deployment stage requires explicit resume"
    }
    $expectedParent = [IO.Path]::GetFullPath($outputParent)
    $observedParent = [IO.Path]::GetFullPath(
        (Split-Path -Parent $stagingRoot)
    )
    if ($observedParent -cne $expectedParent -or
        (Split-Path -Leaf $stagingRoot) -cne
            "$(Split-Path -Leaf $outputRoot).staging-$deploymentId") {
        throw "interrupted Pages stage ownership is not exact"
    }
    Assert-NoPagesBinary -Root $stagingRoot -AllowDescriptorSpki
    [IO.Directory]::Delete($stagingRoot, $true)
}

[IO.Directory]::CreateDirectory($stagingRoot) | Out-Null
try {
    $preserved = @{}
    if ($ExistingDeploymentDirectory) {
        $existingRoot = (Resolve-Path -LiteralPath $ExistingDeploymentDirectory).Path
        Assert-NoPagesBinary -Root $existingRoot -AllowDescriptorSpki
        foreach ($file in @(Get-ChildItem -LiteralPath $existingRoot -File -Recurse -Force)) {
            $relative = [IO.Path]::GetRelativePath($existingRoot, $file.FullName).Replace("\", "/")
            if ($relative.StartsWith(
                "Rusty-Fleet/metadata/",
                [StringComparison]::Ordinal
            ) -and -not $relative.StartsWith(
                "Rusty-Fleet/metadata/$Channel/",
                [StringComparison]::Ordinal
            )) {
                $preserved[$relative] = Get-RustyFleetSha256 -LiteralPath $file.FullName
            }
        }
        foreach ($entry in @(Get-ChildItem -LiteralPath $existingRoot -Force)) {
            Copy-Item -LiteralPath $entry.FullName -Destination $stagingRoot -Recurse -Force
        }
    }
    foreach ($directory in @(
        Get-ChildItem -LiteralPath $siteRoot -Directory -Recurse -Force
    )) {
        $relative = [IO.Path]::GetRelativePath(
            $siteRoot,
            $directory.FullName
        )
        [IO.Directory]::CreateDirectory(
            (Join-Path $stagingRoot $relative)
        ) | Out-Null
    }
    foreach ($file in @(
        Get-ChildItem -LiteralPath $siteRoot -File -Recurse -Force
    )) {
        $relative = [IO.Path]::GetRelativePath($siteRoot, $file.FullName)
        if ($relative.Replace("\", "/").StartsWith(
            "Rusty-Fleet/metadata/",
            [StringComparison]::Ordinal
        )) {
            throw "human site input must not own release metadata subtrees"
        }
        $destination = Join-Path $stagingRoot $relative
        [IO.Directory]::CreateDirectory(
            (Split-Path -Parent $destination)
        ) | Out-Null
        Copy-Item -LiteralPath $file.FullName -Destination $destination
    }
    $pagesMetadataRoot = Join-Path $stagingRoot (
        "Rusty-Fleet\metadata\$Channel"
    )
    [IO.Directory]::CreateDirectory($pagesMetadataRoot) | Out-Null
    foreach ($name in $expectedMetadataNames) {
        Copy-Item `
            -LiteralPath (Join-Path $metadataRoot $name) `
            -Destination (Join-Path $pagesMetadataRoot $name)
    }
    Write-RustyFleetUtf8 `
        -LiteralPath (Join-Path $pagesMetadataRoot "deployment-handoff.json") `
        -Content $handoffText
    foreach ($entry in $preserved.GetEnumerator()) {
        $path = Join-Path $stagingRoot $entry.Key
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or
            (Get-RustyFleetSha256 -LiteralPath $path) -cne $entry.Value) {
            throw "complete-site composition changed a non-target channel byte"
        }
    }
    Assert-NoPagesBinary -Root $stagingRoot -AllowDescriptorSpki
    [IO.Directory]::Move($stagingRoot, $outputRoot)
}
catch {
    throw
}

Assert-NoPagesBinary -Root $outputRoot -AllowDescriptorSpki
$handoff | ConvertTo-Json -Depth 10
