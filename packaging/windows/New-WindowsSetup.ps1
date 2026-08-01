# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$")]
    [string] $Version,

    [Parameter(Mandatory)]
    [ValidateSet("dev", "labs", "stable")]
    [string] $Channel,

    [ValidateSet("alpha", "beta", "rc", "released")]
    [string] $Maturity = "released",

    [Parameter(Mandatory)]
    [ValidateSet("unsigned-dev", "signed-release")]
    [string] $BuildKind,

    [Parameter(Mandatory)]
    [string] $BundleArchivePath,

    [Parameter(Mandatory)]
    [string] $OutputDirectory,

    [string] $FleetSignerThumbprint,
    [string] $HostessSignerThumbprint,
    [string] $ReleasePolicyPath = (
        Join-Path $PSScriptRoot "trust\release-policy.json"
    ),
    [string] $DevelopmentInstallRoot,
    [ValidateRange(0, 10000)]
    [int] $DevelopmentTestPauseAfterRetainMs = 0,
    [string] $DevelopmentShellTestRoot,
    [ValidateSet(
        "",
        "after_setup",
        "after_shortcuts",
        "after_registry",
        "uninstall_after_shortcuts",
        "uninstall_after_registry",
        "uninstall_delete_root",
        "uninstall_partial_delete",
        "uninstall_partial_delete_receipt_failure"
    )]
    [string] $DevelopmentShellFailurePoint = "",
    [string] $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
Import-Module (Join-Path $PSScriptRoot "Distribution.Common.psm1") -Force
$channelPolicy = $null
if ($BuildKind -eq "signed-release") {
    if ($Channel -eq "dev") {
        throw "signed Setup requires Labs or Stable release identity"
    }
    $channelPolicy = Read-RustyFleetReleaseTrustPolicy `
        -LiteralPath (Resolve-Path -LiteralPath $ReleasePolicyPath).Path `
        -Channel $Channel
    if ($channelPolicy.publication_enabled -ne $true -or
        $FleetSignerThumbprint.ToUpperInvariant() -cne
            $channelPolicy.authenticode.thumbprint -or
        $HostessSignerThumbprint.ToUpperInvariant() -cne
            $channelPolicy.authenticode.thumbprint) {
        throw "signed Setup inputs are not authorized for the selected channel"
    }
}

$archive = (Resolve-Path -LiteralPath $BundleArchivePath).Path
$productChannel = if ($Channel -eq "labs") { "labs" } else { "stable" }
$distributionTrack = switch ($Channel) {
    "dev" { "local-development" }
    "labs" { "github-prerelease" }
    "stable" { "github-release" }
}
$productStem = if ($Channel -eq "labs") { "RustyFleet-Labs" } else { "RustyFleet" }
$setupFileName = if ($Channel -eq "labs") {
    "RustyFleet-Labs-Setup.exe"
}
else {
    "RustyFleet-Setup.exe"
}
$expectedArchiveName = "$productStem-$Version-win-x64.zip"
if ((Split-Path -Leaf $archive) -cne $expectedArchiveName) {
    throw "inner bundle filename must be exactly $expectedArchiveName"
}
$archiveSha256 = Get-RustyFleetSha256 -LiteralPath $archive

$inspectionRoot = Join-Path ([System.IO.Path]::GetTempPath()) (
    "rusty-fleet-setup-inspect-$([Guid]::NewGuid().ToString('N'))"
)
$buildRoot = Join-Path ([System.IO.Path]::GetTempPath()) (
    "rusty-fleet-setup-build-$([Guid]::NewGuid().ToString('N'))"
)
try {
    [System.IO.Compression.ZipFile]::ExtractToDirectory($archive, $inspectionRoot)
    $inspectionBundleRoot = Join-Path $inspectionRoot (
        "$productStem-$Version-win-x64"
    )
    $manifestPath = Join-Path $inspectionBundleRoot "metadata\release-manifest.json"
    $manifest = Get-Content -LiteralPath $manifestPath -Raw |
        ConvertFrom-Json -Depth 30
    if ($manifest.schema -ne "rusty.fleet.windows_release_manifest.v3" -or
        $manifest.version -cne $Version -or
        $manifest.product_channel -cne $productChannel -or
        $manifest.maturity -cne $Maturity -or
        $manifest.distribution_track -cne $distributionTrack -or
        $manifest.build.kind -cne $BuildKind -or
        $manifest.source.revision -cnotmatch "^[0-9a-f]{40}$" -or
        $manifest.source.tree -cnotmatch "^[0-9a-f]{40}$") {
        throw "inner bundle identity does not match Setup inputs"
    }
    if ($BuildKind -eq "signed-release" -and
        $manifest.build.source_tree_clean -ne $true) {
        throw "signed Setup requires the inner manifest's clean source assertion"
    }
    & (Join-Path $PSScriptRoot "Test-WindowsBundle.ps1") `
        -BundleRoot $inspectionBundleRoot `
        -ExpectedVersion $Version `
        -ExpectedFleetSignerThumbprint $FleetSignerThumbprint `
        -ExpectedHostessSignerThumbprint $HostessSignerThumbprint |
        Out-Null

    if ($BuildKind -eq "signed-release") {
        if ($FleetSignerThumbprint -cnotmatch "^[0-9A-Fa-f]{40}$" -or
            $HostessSignerThumbprint -cnotmatch "^[0-9A-Fa-f]{40,64}$") {
            throw "signed Setup requires independent Fleet and Hostess signer pins"
        }
    }
    elseif ($FleetSignerThumbprint -or $HostessSignerThumbprint) {
        throw "unsigned development Setup must not carry release signer pins"
    }
    if ($BuildKind -eq "signed-release" -and $DevelopmentInstallRoot) {
        throw "signed Setup must not carry a development install-root override"
    }
    if ($BuildKind -eq "signed-release" -and
        ($DevelopmentTestPauseAfterRetainMs -ne 0 -or
         $DevelopmentShellTestRoot -or
         $DevelopmentShellFailurePoint)) {
        throw "signed Setup must not carry development test controls"
    }
    if ($DevelopmentShellFailurePoint -and -not $DevelopmentShellTestRoot) {
        throw "shell failure injection requires an isolated development shell root"
    }

    [System.IO.Directory]::CreateDirectory($buildRoot) | Out-Null
    $configPath = Join-Path $buildRoot "GeneratedReleaseConfiguration.cs"
    $manifestSha256 = Get-RustyFleetSha256 -LiteralPath $manifestPath
    $fleetSigner = if ($FleetSignerThumbprint) {
        $FleetSignerThumbprint.ToUpperInvariant()
    }
    else {
        ""
    }
    $developmentShellRoot = if ($DevelopmentShellTestRoot) {
        [IO.Path]::GetFullPath($DevelopmentShellTestRoot).
            Replace("\", "\\").
            Replace('"', '\"')
    }
    else {
        ""
    }
    $hostessSigner = if ($HostessSignerThumbprint) {
        $HostessSignerThumbprint.ToUpperInvariant()
    }
    else {
        ""
    }
    $signerCertificateSha256 = if ($channelPolicy) {
        $channelPolicy.authenticode.certificate_sha256
    }
    else { "" }
    $authenticodeTrustMode = if ($channelPolicy) {
        $channelPolicy.authenticode.trust_mode
    }
    else { "unsigned-development" }
    $signerSelfIssued = if ($channelPolicy -and
        $channelPolicy.authenticode.self_issued) { "true" } else { "false" }
    $publicTrustClaim = if ($channelPolicy -and
        $channelPolicy.authenticode.public_trust_claim) { "true" } else { "false" }
    $timestampRequired = if ($channelPolicy -and
        $channelPolicy.authenticode.timestamp_required) { "true" } else { "false" }
    $developmentRoot = if ($DevelopmentInstallRoot) {
        [IO.Path]::GetFullPath($DevelopmentInstallRoot).
            Replace("\", "\\").
            Replace('"', '\"')
    }
    else {
        ""
    }
    $config = @"
// Generated only by New-WindowsSetup.ps1 from an already validated inner bundle.
namespace RustyFleet.Setup;
internal static class ReleaseConfiguration
{
    internal static readonly string Version = "$Version";
    internal static readonly string Channel = "$Channel";
    internal static readonly string BuildKind = "$BuildKind";
    internal static readonly string BundleSha256 = "$archiveSha256";
    internal static readonly string ManifestSha256 = "$manifestSha256";
    internal static readonly string FleetSignerThumbprint = "$fleetSigner";
    internal static readonly string HostessSignerThumbprint = "$hostessSigner";
    internal static readonly string SignerCertificateSha256 = "$signerCertificateSha256";
    internal static readonly string AuthenticodeTrustMode = "$authenticodeTrustMode";
    internal static readonly bool SignerSelfIssued = $signerSelfIssued;
    internal static readonly bool PublicTrustClaim = $publicTrustClaim;
    internal static readonly bool TimestampRequired = $timestampRequired;
    internal static readonly string ProductId = "$(if ($Channel -eq "labs") { "rusty-fleet-labs" } else { "rusty-fleet" })";
    internal static readonly string ProductChannel = "$productChannel";
    internal static readonly string Maturity = "$Maturity";
    internal static readonly string DistributionTrack = "$distributionTrack";
    internal static readonly string DisplayName = "$(if ($Channel -eq "labs") { "Rusty Fleet Labs" } else { "Rusty Fleet" })";
    internal static readonly string SetupFileName = "$setupFileName";
    internal static readonly string InstallDirectoryName = "$(if ($Channel -eq "labs") { "RustyFleetLabs" } else { "RustyFleet" })";
    internal static readonly string DevelopmentInstallRoot = "$developmentRoot";
    internal static readonly int DevelopmentTestPauseAfterRetainMs = $DevelopmentTestPauseAfterRetainMs;
    internal static readonly string DevelopmentShellTestRoot = "$developmentShellRoot";
    internal static readonly string DevelopmentShellFailurePoint = "$DevelopmentShellFailurePoint";
}
"@
    Write-RustyFleetUtf8 -LiteralPath $configPath -Content $config

    $publishRoot = Join-Path $buildRoot "publish"
    & dotnet publish `
        (Join-Path $RepoRoot "apps\fleet-setup\RustyFleet.Setup.csproj") `
        --nologo `
        --configuration Release `
        --runtime win-x64 `
        --self-contained true `
        --output $publishRoot `
        "-p:FleetEmbeddedBundlePath=$archive" `
        "-p:FleetSetupGeneratedConfigPath=$configPath" |
        Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "Rusty Fleet Setup publish failed"
    }
    $built = Join-Path $publishRoot "RustyFleet.Setup.exe"
    if (-not (Test-Path -LiteralPath $built -PathType Leaf)) {
        throw "Rusty Fleet Setup executable is missing"
    }
    [System.IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null
    $destination = Join-Path (
        [System.IO.Path]::GetFullPath($OutputDirectory)
    ) $setupFileName
    if (Test-Path -LiteralPath $destination) {
        throw "refusing to overwrite an existing $setupFileName"
    }
    Copy-Item -LiteralPath $built -Destination $destination
    $canonicalPayload = Get-RustyFleetPeCanonicalPayload `
        -LiteralPath $destination `
        -ExpectedPayloadSize (Get-Item -LiteralPath $destination).Length
    [ordered]@{
        schema = "rusty.fleet.windows_setup_build_receipt.v3"
        result = "pass"
        version = $Version
        channel = $Channel
        product_channel = $productChannel
        maturity = $Maturity
        distribution_track = $distributionTrack
        build_kind = $BuildKind
        setup_sha256 = Get-RustyFleetSha256 -LiteralPath $destination
        bundle_sha256 = $archiveSha256
        manifest_sha256 = $manifestSha256
        source_revision = [string] $manifest.source.revision
        source_tree = [string] $manifest.source.tree
        source_tree_clean = [bool] $manifest.build.source_tree_clean
        canonical_pe_payload_sha256 = $canonicalPayload.sha256
        canonical_pe_payload_size_bytes = [long] $canonicalPayload.size_bytes
        authenticode_trust_mode = if ($channelPolicy) {
            $channelPolicy.authenticode.trust_mode
        } else { "unsigned-development" }
        signer_certificate_sha256 = if ($channelPolicy) {
            $channelPolicy.authenticode.certificate_sha256
        } else { $null }
        signer_self_issued = if ($channelPolicy) {
            [bool] $channelPolicy.authenticode.self_issued
        } else { $false }
        public_trust_claim = if ($channelPolicy) {
            [bool] $channelPolicy.authenticode.public_trust_claim
        } else { $false }
        timestamp_required = if ($channelPolicy) {
            [bool] $channelPolicy.authenticode.timestamp_required
        } else { $false }
        distribution_eligibility = if ($BuildKind -eq "signed-release") {
            "requires_setup_authenticode_signing"
        }
        else {
            "development_only"
        }
    } | ConvertTo-Json -Depth 10
}
finally {
    foreach ($path in @($inspectionRoot, $buildRoot)) {
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path -Recurse -Force
        }
    }
}
