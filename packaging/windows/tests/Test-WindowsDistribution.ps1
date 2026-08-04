# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [string] $FleetAgentKeyRecordOwnerCapsuleRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$packagingRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Import-Module (Join-Path $packagingRoot "Distribution.Common.psm1") -Force
$distributionModule = Get-Module Distribution.Common

function Assert-Distribution {
    param(
        [Parameter(Mandatory)][bool] $Condition,
        [Parameter(Mandatory)][string] $Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

$provenanceTimestampJson = (
    '{"verified_at_utc":"2026-07-27T00:00:00Z"}'
)
Assert-Distribution (
    $provenanceTimestampJson -cmatch
        '"verified_at_utc":"2026-07-27T00:00:00Z"'
) "serialized provenance timestamp fixture is not canonical UTC"
$priorDistributionCulture = [Globalization.CultureInfo]::CurrentCulture
$priorDistributionUiCulture = [Globalization.CultureInfo]::CurrentUICulture
try {
    foreach ($cultureName in @("de-DE", "en-US", "")) {
        $culture = if ($cultureName) {
            [Globalization.CultureInfo]::GetCultureInfo($cultureName)
        }
        else {
            [Globalization.CultureInfo]::InvariantCulture
        }
        [Globalization.CultureInfo]::CurrentCulture = $culture
        [Globalization.CultureInfo]::CurrentUICulture = $culture
        $typedTimestamp = (
            $provenanceTimestampJson | ConvertFrom-Json
        ).verified_at_utc
        $parsedTimestamp = & $distributionModule {
            param($Value)
            ConvertTo-RustyFleetUtcDateTimeOffset `
                -Value $Value `
                -Context "test provenance timestamp"
        } $typedTimestamp
        Assert-Distribution (
            $typedTimestamp -is [DateTime] -and
            $parsedTimestamp.ToUnixTimeMilliseconds() -eq 1785110400000
        ) "typed provenance timestamp was culture-dependent: $cultureName"
    }
}
finally {
    [Globalization.CultureInfo]::CurrentCulture =
        $priorDistributionCulture
    [Globalization.CultureInfo]::CurrentUICulture =
        $priorDistributionUiCulture
}

function Write-TestArtifact {
    param(
        [Parameter(Mandatory)][string] $LiteralPath,
        [Parameter(Mandatory)][string] $Content
    )

    Write-RustyFleetUtf8 -LiteralPath $LiteralPath -Content $Content
}

function Write-TestProviderArtifact {
    param(
        [Parameter(Mandatory)][string] $LiteralPath,
        [Parameter(Mandatory)][string] $ProductVersion
    )

    $projectRoot = Join-Path (Split-Path -Parent $LiteralPath) (
        "provider-fixture-project"
    )
    $outputRoot = Join-Path $projectRoot "output"
    [System.IO.Directory]::CreateDirectory($projectRoot) | Out-Null
    $projectPath = Join-Path $projectRoot "ProviderFixture.csproj"
    Write-RustyFleetUtf8 -LiteralPath $projectPath -Content @"
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net9.0</TargetFramework>
    <AssemblyName>RustyFleet.HostessProviderFixture</AssemblyName>
    <Version>0.0.0-test.1</Version>
    <AssemblyVersion>0.0.0.0</AssemblyVersion>
    <FileVersion>0.0.0.0</FileVersion>
    <InformationalVersion>$ProductVersion</InformationalVersion>
    <Deterministic>true</Deterministic>
    <RestoreIgnoreFailedSources>true</RestoreIgnoreFailedSources>
  </PropertyGroup>
</Project>
"@
    Write-RustyFleetUtf8 `
        -LiteralPath (Join-Path $projectRoot "ProviderFixture.cs") `
        -Content @"
namespace RustyFleetDistributionTests;
public static class ProviderFixture { }
"@
    & dotnet build $projectPath `
        --nologo `
        --configuration Release `
        --output $outputRoot
    if ($LASTEXITCODE -ne 0) {
        throw "could not build the synthetic versioned provider fixture"
    }
    Copy-Item `
        -LiteralPath (Join-Path $outputRoot "RustyFleet.HostessProviderFixture.dll") `
        -Destination $LiteralPath
    $observed = (
        [System.Diagnostics.FileVersionInfo]::GetVersionInfo(
            $LiteralPath
        ).ProductVersion
    )
    if ($observed -cne $ProductVersion) {
        throw "synthetic provider fixture has the wrong ProductVersion"
    }
}

function Invoke-TestBundle {
    param(
        [Parameter(Mandatory)][string] $Version,
        [Parameter(Mandatory)][string] $OutputDirectory,
        [Parameter(Mandatory)][string] $ConsoleDirectory,
        [Parameter(Mandatory)][string] $HubPath,
        [Parameter(Mandatory)][string] $FleetctlPath,
        [Parameter(Mandatory)][string] $FleetOnboardPath,
        [Parameter(Mandatory)][string] $ProviderPath,
        [Parameter(Mandatory)][string] $ProviderSha256,
        [Parameter(Mandatory)][string] $ProviderMetadataDirectory,
        [string] $OwnerCapsuleRoot = "",
        [ValidateSet("dev", "labs", "stable")]
        [string] $Channel = "dev",
        [ValidateSet("alpha", "beta", "rc", "released")]
        [string] $Maturity = "released",
        [string] $SourceRevision = ("1" * 40),
        [string] $SourceTree = ("2" * 40)
    )

    $arguments = @{
        Version = $Version
        Channel = $Channel
        Maturity = $Maturity
        BuildKind = "unsigned-dev"
        HostessProviderPath = $ProviderPath
        HostessProviderSha256 = $ProviderSha256
        HostessProviderMetadataDirectory = $ProviderMetadataDirectory
        OutputDirectory = $OutputDirectory
        SourceRevision = $SourceRevision
        SourceTree = $SourceTree
        SourceDateEpoch = 1785110400
        SkipBuild = $true
        ConsoleArtifactDirectory = $ConsoleDirectory
        HubArtifactPath = $HubPath
        FleetctlArtifactPath = $FleetctlPath
        FleetOnboardArtifactPath = $FleetOnboardPath
    }
    if ($OwnerCapsuleRoot) {
        $arguments.FleetAgentKeyRecordOwnerCapsuleRoot = $OwnerCapsuleRoot
    }
    & (Join-Path $packagingRoot "New-WindowsBundle.ps1") @arguments
}

function Start-TestSetup {
    param(
        [Parameter(Mandatory)][string] $LiteralPath,
        [Parameter(Mandatory)][ValidateSet("i", "r")][string] $Answer
    )

    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $LiteralPath
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $start
    if (-not $process.Start()) {
        throw "could not start guided Setup test process"
    }
    $process.StandardInput.WriteLine($Answer)
    $process.StandardInput.Close()
    return $process
}

function Complete-TestSetup {
    param(
        [Parameter(Mandatory)][System.Diagnostics.Process] $Process,
        [int] $TimeoutMilliseconds = 30000
    )

    if (-not $Process.WaitForExit($TimeoutMilliseconds)) {
        $Process.Kill($true)
        $Process.WaitForExit()
        throw "guided Setup test process timed out"
    }
    $stdout = $Process.StandardOutput.ReadToEnd()
    $stderr = $Process.StandardError.ReadToEnd()
    [pscustomobject]@{
        exit_code = $Process.ExitCode
        stdout = $stdout
        stderr = $stderr
    }
}

function Wait-TestCandidate {
    param(
        [Parameter(Mandatory)][string] $InstallRoot,
        [int] $TimeoutMilliseconds = 10000
    )

    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMilliseconds)
    do {
        $candidate = Get-ChildItem `
            -LiteralPath (Join-Path $InstallRoot "releases") `
            -Directory `
            -Filter ".candidate-*" `
            -ErrorAction SilentlyContinue |
            Where-Object {
                Test-Path -LiteralPath (
                    Join-Path $_.FullName "components\fleetctl\fleetctl.exe"
                ) -PathType Leaf
            } |
            Sort-Object LastWriteTimeUtc -Descending |
            Select-Object -First 1
        if ($candidate) {
            return $candidate.FullName
        }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "timed out waiting for retained Setup candidate"
}

$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) (
    "rusty-fleet-windows-dist-$([Guid]::NewGuid().ToString('N'))"
)
[System.IO.Directory]::CreateDirectory($testRoot) | Out-Null

try {
    $inputs = Join-Path $testRoot "inputs"
    $console = Join-Path $inputs "console"
    [System.IO.Directory]::CreateDirectory($console) | Out-Null
    $consoleExe = Join-Path $console "RustyFleet.FleetConsole.exe"
    $consoleRuntime = Join-Path $console "RustyFleet.FleetConsole.runtimeconfig.json"
    $hub = Join-Path $inputs "fleet-hub-local.exe"
    $fleetctl = Join-Path $inputs "fleetctl.exe"
    $fleetOnboard = Join-Path $inputs "fleet-onboard.exe"
    $provider = Join-Path $inputs "rusty-hostess-hotspot-provider.exe"
    $providerRevision = "2" * 40
    $providerVersion = "0.0.0-test.1"
    $providerProductVersion = "$providerVersion+$providerRevision"
    Write-TestArtifact -LiteralPath $consoleExe -Content "console-test-artifact`n"
    Write-TestArtifact -LiteralPath $consoleRuntime -Content "{`"runtimeOptions`":{}}`n"
    Write-TestArtifact -LiteralPath $hub -Content "hub-test-artifact`n"
    Write-TestArtifact -LiteralPath $fleetctl -Content "fleetctl-test-artifact`n"
    Write-TestArtifact -LiteralPath $fleetOnboard -Content "fleet-onboard-test-artifact`n"
    Write-TestProviderArtifact `
        -LiteralPath $provider `
        -ProductVersion $providerProductVersion
    $providerSha256 = Get-RustyFleetSha256 -LiteralPath $provider
    [byte[]] $providerBytes = [System.IO.File]::ReadAllBytes($provider)
    $providerPayloadSize = [long] $providerBytes.LongLength
    $validCanonicalPe = Get-RustyFleetPeCanonicalPayload `
        -LiteralPath $provider `
        -ExpectedPayloadSize $providerPayloadSize
    Assert-Distribution (
        $validCanonicalPe.sha256 -ceq $providerSha256 -and
        $validCanonicalPe.size_bytes -eq $providerPayloadSize
    ) "valid unsigned canonical PE fixture was not accepted"
    $providerPeOffset = [int] [BitConverter]::ToUInt32($providerBytes, 0x3c)
    $providerOptionalHeader = $providerPeOffset + 24
    $providerMagic = [BitConverter]::ToUInt16(
        $providerBytes,
        $providerOptionalHeader
    )
    $providerDirectoryCountOffset = switch ($providerMagic) {
        0x10b { $providerOptionalHeader + 92 }
        0x20b { $providerOptionalHeader + 108 }
        default { throw "synthetic provider fixture is not a supported PE" }
    }
    $providerDataDirectories = switch ($providerMagic) {
        0x10b { $providerOptionalHeader + 96 }
        0x20b { $providerOptionalHeader + 112 }
    }
    $providerCertificateDirectory = $providerDataDirectories + (4 * 8)
    $malformedPeFixtures = [System.Collections.Generic.List[string]]::new()

    [byte[]] $badOptionalSize = $providerBytes.Clone()
    $observedOptionalSize = [BitConverter]::ToUInt16(
        $badOptionalSize,
        $providerPeOffset + 20
    )
    [BitConverter]::GetBytes([uint16] ($observedOptionalSize + 8)).CopyTo(
        $badOptionalSize,
        $providerPeOffset + 20
    )
    $badOptionalSizePath = Join-Path $inputs "bad-optional-size.exe"
    [System.IO.File]::WriteAllBytes($badOptionalSizePath, $badOptionalSize)
    $malformedPeFixtures.Add($badOptionalSizePath)

    [byte[]] $badDirectoryCount = $providerBytes.Clone()
    [BitConverter]::GetBytes([uint32] 17).CopyTo(
        $badDirectoryCount,
        $providerDirectoryCountOffset
    )
    $badDirectoryCountPath = Join-Path $inputs "bad-directory-count.exe"
    [System.IO.File]::WriteAllBytes(
        $badDirectoryCountPath,
        $badDirectoryCount
    )
    $malformedPeFixtures.Add($badDirectoryCountPath)

    $alignedProviderSize = [long] (
        ($providerPayloadSize + 7) -band (-bnot 7)
    )
    $wrongCertificateOffset = $alignedProviderSize + 8
    [byte[]] $badCertificateGap = [byte[]]::new(
        $wrongCertificateOffset + 8
    )
    [Array]::Copy(
        $providerBytes,
        0,
        $badCertificateGap,
        0,
        $providerBytes.Length
    )
    [BitConverter]::GetBytes([uint32] $wrongCertificateOffset).CopyTo(
        $badCertificateGap,
        $providerCertificateDirectory
    )
    [BitConverter]::GetBytes([uint32] 8).CopyTo(
        $badCertificateGap,
        $providerCertificateDirectory + 4
    )
    [BitConverter]::GetBytes([uint32] 8).CopyTo(
        $badCertificateGap,
        [int] $wrongCertificateOffset
    )
    [BitConverter]::GetBytes([uint16] 0x0200).CopyTo(
        $badCertificateGap,
        [int] $wrongCertificateOffset + 4
    )
    [BitConverter]::GetBytes([uint16] 0x0002).CopyTo(
        $badCertificateGap,
        [int] $wrongCertificateOffset + 6
    )
    $badCertificateGapPath = Join-Path $inputs "bad-certificate-gap.exe"
    [System.IO.File]::WriteAllBytes(
        $badCertificateGapPath,
        $badCertificateGap
    )
    $malformedPeFixtures.Add($badCertificateGapPath)

    foreach ($malformedPePath in $malformedPeFixtures) {
        $malformedPeRejected = $false
        try {
            Get-RustyFleetPeCanonicalPayload `
                -LiteralPath $malformedPePath `
                -ExpectedPayloadSize $providerPayloadSize | Out-Null
        }
        catch {
            $malformedPeRejected = $true
        }
        Assert-Distribution `
            $malformedPeRejected `
            "ambiguous or malformed PE canonicalization input was accepted"
    }
    $providerMetadata = Join-Path $inputs "hostess-provider-metadata"
    [System.IO.Directory]::CreateDirectory($providerMetadata) | Out-Null
    $providerLicense = Join-Path $providerMetadata "LICENSE"
    $providerNotices = Join-Path $providerMetadata "THIRD-PARTY-NOTICES.txt"
    Write-TestArtifact `
        -LiteralPath $providerLicense `
        -Content "Synthetic AGPL-3.0-or-later owner license fixture.`n"
    Write-TestArtifact `
        -LiteralPath $providerNotices `
        -Content "Synthetic third-party notices fixture; not release provenance.`n"
    $providerPolicyPath = Join-Path $providerMetadata (
        "rusty-hostess-hotspot-provider.release-policy.json"
    )
    $providerPolicy = [ordered]@{
        schema = "rusty.hostess.windows_hotspot.release_policy.v1"
        product_id = "rusty-hostess-windows-hotspot-provider"
        signer = [ordered]@{
            subject = "CN=Synthetic Test Signer"
            issuer = "CN=Synthetic Test Signer"
            thumbprint_sha1 = "6" * 40
            certificate_sha256 = "6" * 64
            code_signing_eku_oid = "1.3.6.1.5.5.7.3.3"
            self_issued = $true
            timestamp_required = $true
            public_trust_claim = $false
        }
        accepted_validation_boundaries = @(
            [ordered]@{
                authenticode_status = "valid"
                chain_trusted = $true
                chain_element_count = 1
                chain_status_flags = @()
            },
            [ordered]@{
                authenticode_status = "unknown_error"
                chain_trusted = $false
                chain_element_count = 1
                chain_status_flags = @("UntrustedRoot")
            }
        )
        distribution = [ordered]@{
            allowed_channels = @("labs")
            stable_eligible = $false
        }
        status = "active"
    }
    Write-TestArtifact `
        -LiteralPath $providerPolicyPath `
        -Content (ConvertTo-RustyFleetJson -InputObject $providerPolicy)
    $providerProvenance = [ordered]@{
        schema = "rusty.hostess.windows_hotspot.release_provenance.v2"
        product_id = "rusty-hostess-windows-hotspot-provider"
        provider_version = $providerVersion
        artifact = [ordered]@{
            name = "rusty-hostess-hotspot-provider.exe"
            sha256 = $providerSha256
            size_bytes = (Get-Item -LiteralPath $provider).Length
            product_version = $providerProductVersion
        }
        source = [ordered]@{
            repository = "https://github.com/MesmerPrism/rusty-hostess"
            revision = $providerRevision
            tree = "4" * 40
            availability_url = (
                "https://github.com/MesmerPrism/rusty-hostess/tree/" +
                $providerRevision
            )
            availability_state = "unverified_development"
            verified_at_utc = $null
            tree_clean = $true
        }
        build = [ordered]@{
            kind = "unsigned-dev"
            framework = "net9.0-windows10.0.19041.0"
            runtime_identifier = "win-x64"
            source_date_epoch = 1785110400
            unsigned_artifact_sha256 = $providerSha256
            unsigned_artifact_size_bytes = (
                Get-Item -LiteralPath $provider
            ).Length
            canonical_payload_sha256 = $providerSha256
            canonical_payload_size_bytes = (
                Get-Item -LiteralPath $provider
            ).Length
        }
        dependencies = @(
            [ordered]@{
                name = "Synthetic.Dependency"
                version = "1.0.0"
                license = "MIT"
                license_url = "https://example.invalid/licenses/mit"
                project_url = "https://example.invalid/synthetic-dependency"
            }
        )
        bundled_native_libraries = @(
            [ordered]@{
                name = "Synthetic.Native.dll"
                sha256 = "5" * 64
                size_bytes = 4096
            }
        )
        signing = [ordered]@{
            state = "unsigned"
            authenticode_status = "not_signed"
            subject = $null
            issuer = $null
            thumbprint_sha1 = $null
            certificate_sha256 = $null
            code_signing_eku_present = $false
            self_issued = $null
            timestamp_present = $false
            chain_trusted = $false
            chain_element_count = 0
            chain_status_flags = @()
            public_trust_claim = $false
            trust_boundary = "unsigned-development"
        }
        release_policy = [ordered]@{
            asset_name = "rusty-hostess-hotspot-provider.release-policy.json"
            schema = $providerPolicy.schema
            sha256 = Get-RustyFleetSha256 -LiteralPath $providerPolicyPath
            size_bytes = (Get-Item -LiteralPath $providerPolicyPath).Length
        }
        companion_documents = @(
            [ordered]@{
                name = "LICENSE"
                sha256 = Get-RustyFleetSha256 -LiteralPath $providerLicense
                size_bytes = (Get-Item -LiteralPath $providerLicense).Length
            },
            [ordered]@{
                name = "THIRD-PARTY-NOTICES.txt"
                sha256 = Get-RustyFleetSha256 -LiteralPath $providerNotices
                size_bytes = (Get-Item -LiteralPath $providerNotices).Length
            }
        )
        distribution = [ordered]@{
            eligibility = "development_only"
            binary_authority = "rusty-hostess-github-releases"
            allowed_channels = @()
            stable_eligible = $false
        }
    }
    Write-TestArtifact `
        -LiteralPath (
            Join-Path $providerMetadata "rusty-hostess-hotspot-provider.provenance.json"
        ) `
        -Content (ConvertTo-RustyFleetJson -InputObject $providerProvenance)

    $unboundMetadata = Join-Path $inputs "unbound-hostess-provider-metadata"
    [System.IO.Directory]::CreateDirectory($unboundMetadata) | Out-Null
    Copy-Item -LiteralPath $providerLicense -Destination $unboundMetadata
    Copy-Item -LiteralPath $providerNotices -Destination $unboundMetadata
    Copy-Item -LiteralPath $providerPolicyPath -Destination $unboundMetadata
    $unboundProvenance = (
        ConvertTo-RustyFleetJson -InputObject $providerProvenance |
        ConvertFrom-Json -Depth 30
    )
    $unboundProvenance.build.unsigned_artifact_sha256 = "7" * 64
    Write-TestArtifact `
        -LiteralPath (
            Join-Path $unboundMetadata (
                "rusty-hostess-hotspot-provider.provenance.json"
            )
        ) `
        -Content (ConvertTo-RustyFleetJson -InputObject $unboundProvenance)
    $unboundUnsignedRejected = $false
    try {
        Read-RustyFleetHostessProvenance `
            -MetadataDirectory $unboundMetadata `
            -ProviderPath $provider `
            -ProviderSha256 $providerSha256 `
            -BuildKind unsigned-dev `
            -Channel dev | Out-Null
    }
    catch {
        $unboundUnsignedRejected = $true
    }
    Assert-Distribution `
        $unboundUnsignedRejected `
        "unsigned rebuild evidence was not bound to the canonical PE payload"

    $signedMetadata = Join-Path $inputs "signed-hostess-provider-metadata"
    [System.IO.Directory]::CreateDirectory($signedMetadata) | Out-Null
    Copy-Item -LiteralPath $providerLicense -Destination $signedMetadata
    Copy-Item -LiteralPath $providerNotices -Destination $signedMetadata
    Copy-Item -LiteralPath $providerPolicyPath -Destination $signedMetadata
    $signedProvenance = (
        ConvertTo-RustyFleetJson -InputObject $providerProvenance |
        ConvertFrom-Json -Depth 30
    )
    $signedProvenance.build.kind = "signed-release"
    $signedProvenance.source.availability_state = "verified_public"
    $signedProvenance.source.verified_at_utc = "2026-07-27T00:00:00Z"
    $signedProvenance.signing.state = "accepted_exact_owner_signature"
    $signedProvenance.signing.authenticode_status = "unknown_error"
    $signedProvenance.signing.subject = "CN=Synthetic Test Signer"
    $signedProvenance.signing.issuer = "CN=Synthetic Test Signer"
    $signedProvenance.signing.thumbprint_sha1 = "6" * 40
    $signedProvenance.signing.certificate_sha256 = "6" * 64
    $signedProvenance.signing.code_signing_eku_present = $true
    $signedProvenance.signing.self_issued = $true
    $signedProvenance.signing.timestamp_present = $true
    $signedProvenance.signing.chain_trusted = $false
    $signedProvenance.signing.chain_element_count = 1
    $signedProvenance.signing.chain_status_flags = @("UntrustedRoot")
    $signedProvenance.signing.trust_boundary =
        "exact-pinned-self-issued-untrusted-root-only"
    $signedProvenance.distribution.eligibility = "labs_signed_release"
    $signedProvenance.distribution.allowed_channels = @("labs")
    Write-TestArtifact `
        -LiteralPath (
            Join-Path $signedMetadata (
                "rusty-hostess-hotspot-provider.provenance.json"
            )
        ) `
        -Content (ConvertTo-RustyFleetJson -InputObject $signedProvenance)
    $syntheticAuthPolicy = [pscustomobject][ordered]@{
        subject = "CN=Synthetic Test Signer"
        thumbprint = "6" * 40
        certificate_sha256 = "6" * 64
        self_issued = $true
        public_trust_claim = $false
        trust_mode = "exact-pinned-self-issued-untrusted-root-only"
        timestamp_required = $true
        allowed_chain_status_flags = @("UntrustedRoot")
    }
    $physicallyUnsignedReleaseRejected = $false
    try {
        Read-RustyFleetHostessProvenance `
            -MetadataDirectory $signedMetadata `
            -ProviderPath $provider `
            -ProviderSha256 $providerSha256 `
            -BuildKind signed-release `
            -Channel labs `
            -AuthenticodePolicy $syntheticAuthPolicy | Out-Null
    }
    catch {
        $physicallyUnsignedReleaseRejected = $true
    }
    Assert-Distribution `
        $physicallyUnsignedReleaseRejected `
        "signed metadata authorized a physically unsigned provider"

    $badProviderHashRejected = $false
    try {
        Invoke-TestBundle `
            -Version "0.0.0-test.0" `
            -OutputDirectory (Join-Path $testRoot "bad-provider") `
            -ConsoleDirectory $console `
            -HubPath $hub `
            -FleetctlPath $fleetctl `
            -FleetOnboardPath $fleetOnboard `
            -ProviderPath $provider `
            -ProviderSha256 ("0" * 64) `
            -ProviderMetadataDirectory $providerMetadata | Out-Null
    }
    catch {
        $badProviderHashRejected = $true
    }
    Assert-Distribution $badProviderHashRejected "provider digest mismatch was accepted"

    $outputOne = Join-Path $testRoot "one"
    $outputTwo = Join-Path $testRoot "two"
    Invoke-TestBundle `
        -Version "0.0.0-test.1" `
        -OutputDirectory $outputOne `
        -ConsoleDirectory $console `
        -HubPath $hub `
        -FleetctlPath $fleetctl `
        -FleetOnboardPath $fleetOnboard `
        -ProviderPath $provider `
        -ProviderSha256 $providerSha256 `
        -ProviderMetadataDirectory $providerMetadata | Out-Null
    Invoke-TestBundle `
        -Version "0.0.0-test.1" `
        -OutputDirectory $outputTwo `
        -ConsoleDirectory $console `
        -HubPath $hub `
        -FleetctlPath $fleetctl `
        -FleetOnboardPath $fleetOnboard `
        -ProviderPath $provider `
        -ProviderSha256 $providerSha256 `
        -ProviderMetadataDirectory $providerMetadata | Out-Null

    $bundleNameOne = "RustyFleet-0.0.0-test.1-win-x64"
    $bundleOne = Join-Path $outputOne $bundleNameOne
    $bundleTwo = Join-Path $outputTwo $bundleNameOne
    $validation = & (Join-Path $packagingRoot "Test-WindowsBundle.ps1") `
        -BundleRoot $bundleOne |
        ConvertFrom-Json
    Assert-Distribution ($validation.result -eq "pass") "valid bundle did not pass"
    Assert-Distribution ($validation.runtime_components -eq 5) "runtime composition is not exact"

    if ($FleetAgentKeyRecordOwnerCapsuleRoot) {
        $readyOutput = Join-Path $testRoot "owner-capsule-ready"
        Invoke-TestBundle `
            -Version "0.0.0-test.1" `
            -OutputDirectory $readyOutput `
            -ConsoleDirectory $console `
            -HubPath $hub `
            -FleetctlPath $fleetctl `
            -FleetOnboardPath $fleetOnboard `
            -ProviderPath $provider `
            -ProviderSha256 $providerSha256 `
            -ProviderMetadataDirectory $providerMetadata `
            -OwnerCapsuleRoot $FleetAgentKeyRecordOwnerCapsuleRoot | Out-Null
        $readyBundle = Join-Path $readyOutput $bundleNameOne
        $readyValidation = & (Join-Path $packagingRoot "Test-WindowsBundle.ps1") `
            -BundleRoot $readyBundle | ConvertFrom-Json
        $readyManifest = Get-Content -Raw -LiteralPath (
            Join-Path $readyBundle "metadata\release-manifest.json") |
            ConvertFrom-Json -Depth 30
        Assert-Distribution (
            $readyValidation.runtime_components -eq 6 -and
            $readyManifest.distribution.onboarding_ready -eq $true -and
            $readyManifest.distribution.onboarding_blocker -ceq "none" -and
            @($readyManifest.components | Where-Object {
                $_.component_id -ceq "rusty-quest-key-record-helper"
            }).Count -eq 1
        ) "complete owner capsule did not make the package onboarding-ready"

        $readyArchive = Join-Path $readyOutput "$bundleNameOne.zip"
        $readySetupOutput = Join-Path $testRoot "owner-capsule-ready-setup"
        $readySetupReceipt = & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
            -Version "0.0.0-test.1" -Channel dev -BuildKind unsigned-dev `
            -BundleArchivePath $readyArchive `
            -DevelopmentInstallRoot (Join-Path $testRoot "owner-capsule-ready-install") `
            -OutputDirectory $readySetupOutput | ConvertFrom-Json
        $readySetupPath = Join-Path $readySetupOutput "RustyFleet-Setup.exe"
        $readySetupPlan = & $readySetupPath --plan --json | ConvertFrom-Json
        Assert-Distribution (
            $readySetupReceipt.version -ceq "0.0.0-test.1" -and
            $readySetupPlan.ready -eq $true
        ) "Setup did not admit the exact onboarding-ready six-component bundle"

        $fiveMismatchRoot = Join-Path $testRoot "setup-five-components-falsely-ready"
        $fiveMismatchBundle = Join-Path $fiveMismatchRoot $bundleNameOne
        Copy-Item -LiteralPath $bundleOne -Destination $fiveMismatchBundle -Recurse
        $fiveMismatchManifestPath = Join-Path $fiveMismatchBundle `
            "metadata\release-manifest.json"
        $fiveMismatchManifest = Get-Content -LiteralPath $fiveMismatchManifestPath -Raw |
            ConvertFrom-Json -Depth 30
        $fiveMismatchManifest.distribution.onboarding_ready = $true
        $fiveMismatchManifest.distribution.onboarding_blocker = "none"
        Write-RustyFleetUtf8 -LiteralPath $fiveMismatchManifestPath -Content (
            $fiveMismatchManifest | ConvertTo-Json -Depth 30)
        $fiveMismatchArchive = Join-Path $fiveMismatchRoot "$bundleNameOne.zip"
        New-RustyFleetDeterministicZip -SourceDirectory $fiveMismatchBundle `
            -DestinationPath $fiveMismatchArchive -SourceDateEpoch 0
        $fiveMismatchAccepted = $false
        $fiveMismatchFailure = ""
        try {
            & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
                -Version "0.0.0-test.1" -Channel dev -BuildKind unsigned-dev `
                -BundleArchivePath $fiveMismatchArchive `
                -DevelopmentInstallRoot (Join-Path $testRoot "five-mismatch-install") `
                -OutputDirectory (Join-Path $testRoot "five-mismatch-setup") | Out-Null
            $fiveMismatchAccepted = $true
        }
        catch {
            $fiveMismatchFailure = $_.Exception.Message
        }
        Assert-Distribution (
            -not $fiveMismatchAccepted -and
            $fiveMismatchFailure -match
                "bundle runtime component set does not match onboarding readiness"
        ) `
            "Setup accepted five components that falsely claimed onboarding readiness"

        $sixMismatchRoot = Join-Path $testRoot "setup-six-components-falsely-blocked"
        $sixMismatchBundle = Join-Path $sixMismatchRoot $bundleNameOne
        Copy-Item -LiteralPath $readyBundle -Destination $sixMismatchBundle -Recurse
        $sixMismatchManifestPath = Join-Path $sixMismatchBundle `
            "metadata\release-manifest.json"
        $sixMismatchManifest = Get-Content -LiteralPath $sixMismatchManifestPath -Raw |
            ConvertFrom-Json -Depth 30
        $sixMismatchManifest.distribution.onboarding_ready = $false
        $sixMismatchManifest.distribution.onboarding_blocker =
            "pinned_rusty_quest_owner_key_record_release_not_bundled"
        Write-RustyFleetUtf8 -LiteralPath $sixMismatchManifestPath -Content (
            $sixMismatchManifest | ConvertTo-Json -Depth 30)
        $sixMismatchArchive = Join-Path $sixMismatchRoot "$bundleNameOne.zip"
        New-RustyFleetDeterministicZip -SourceDirectory $sixMismatchBundle `
            -DestinationPath $sixMismatchArchive -SourceDateEpoch 0
        $sixMismatchAccepted = $false
        $sixMismatchFailure = ""
        try {
            & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
                -Version "0.0.0-test.1" -Channel dev -BuildKind unsigned-dev `
                -BundleArchivePath $sixMismatchArchive `
                -DevelopmentInstallRoot (Join-Path $testRoot "six-mismatch-install") `
                -OutputDirectory (Join-Path $testRoot "six-mismatch-setup") | Out-Null
            $sixMismatchAccepted = $true
        }
        catch {
            $sixMismatchFailure = $_.Exception.Message
        }
        Assert-Distribution (
            -not $sixMismatchAccepted -and
            $sixMismatchFailure -match
                "bundle runtime component set does not match onboarding readiness"
        ) `
            "Setup accepted six components that falsely claimed onboarding was blocked"

        foreach ($ownerFile in @(
            "fleet-agent-key-record.exe", "LICENSE", "SOURCE-NOTICE.md")) {
            $caseName = $ownerFile.Replace(".", "-")
            $damagedReadyBundle = Join-Path $testRoot "owner-capsule-damaged-$caseName"
            Copy-Item -LiteralPath $readyBundle -Destination $damagedReadyBundle -Recurse
            [IO.File]::AppendAllText((
                Join-Path $damagedReadyBundle (
                    "components\rusty-quest-key-record-helper\$ownerFile")),
                "damage")
            $damagedOwnerRejected = $false
            try {
                & (Join-Path $packagingRoot "Test-WindowsBundle.ps1") `
                    -BundleRoot $damagedReadyBundle | Out-Null
            }
            catch {
                $damagedOwnerRejected = $true
            }
            Assert-Distribution $damagedOwnerRejected `
                "bundle validation accepted substituted owner capsule file: $ownerFile"
        }
    }

    foreach ($relative in @(
        "metadata\release-manifest.json",
        "metadata\checksums.sha256",
        "metadata\validation-receipt.json"
    )) {
        Assert-Distribution (
            (Get-RustyFleetSha256 -LiteralPath (Join-Path $bundleOne $relative)) -ceq
            (Get-RustyFleetSha256 -LiteralPath (Join-Path $bundleTwo $relative))
        ) "deterministic metadata differs: $relative"
    }
    $archiveOne = Join-Path $outputOne "$bundleNameOne.zip"
    $archiveTwo = Join-Path $outputTwo "$bundleNameOne.zip"
    Assert-Distribution (
        (Get-RustyFleetSha256 -LiteralPath $archiveOne) -ceq
        (Get-RustyFleetSha256 -LiteralPath $archiveTwo)
    ) "deterministic archives differ"

    $labsOne = Join-Path $testRoot "labs-one"
    $labsTwo = Join-Path $testRoot "labs-two"
    foreach ($labsOutput in @($labsOne, $labsTwo)) {
        Invoke-TestBundle -Version "0.0.1" -Channel labs -Maturity alpha `
            -OutputDirectory $labsOutput -ConsoleDirectory $console `
            -HubPath $hub -FleetctlPath $fleetctl -FleetOnboardPath $fleetOnboard `
            -ProviderPath $provider -ProviderSha256 $providerSha256 `
            -ProviderMetadataDirectory $providerMetadata | Out-Null
    }
    $labsBundleName = "RustyFleet-Labs-0.0.1-win-x64"
    $labsArchiveOne = Join-Path $labsOne "$labsBundleName.zip"
    $labsArchiveTwo = Join-Path $labsTwo "$labsBundleName.zip"
    Assert-Distribution (
        (Get-RustyFleetSha256 -LiteralPath $labsArchiveOne) -ceq
        (Get-RustyFleetSha256 -LiteralPath $labsArchiveTwo)
    ) "Labs bundle is not deterministic"
    $labsManifest = Get-Content -LiteralPath (
        Join-Path $labsOne "$labsBundleName\metadata\release-manifest.json"
    ) -Raw | ConvertFrom-Json -Depth 30
    Assert-Distribution (
        $labsManifest.product_channel -eq "labs" -and
        $labsManifest.maturity -eq "alpha" -and
        $labsManifest.channel -eq "labs" -and
        $labsManifest.distribution_track -eq "github-prerelease" -and
        $labsManifest.install.default_root -ceq
            "%LOCALAPPDATA%/RustyFleetLabs" -and
        $labsManifest.install.authority -ceq
            "RustyFleet-Labs-Setup.exe"
    ) "Labs manifest does not bind the isolated install identity"
    $labsSetupOutput = Join-Path $testRoot "labs-setup"
    $labsSetupRoot = Join-Path $testRoot "labs-install-root"
    $labsSetupReceipt = & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
        -Version "0.0.1" -Channel labs -Maturity alpha -BuildKind unsigned-dev `
        -BundleArchivePath $labsArchiveOne -DevelopmentInstallRoot $labsSetupRoot `
        -OutputDirectory $labsSetupOutput | ConvertFrom-Json
    $labsSetupPath = Join-Path $labsSetupOutput "RustyFleet-Labs-Setup.exe"
    Assert-Distribution (
        $labsSetupReceipt.product_channel -eq "labs" -and
        $labsSetupReceipt.maturity -eq "alpha" -and
        $labsSetupReceipt.channel -eq "labs" -and
        $labsSetupReceipt.distribution_track -eq "github-prerelease" -and
        (Test-Path -LiteralPath $labsSetupPath -PathType Leaf)
    ) "Labs Setup identity was not built dynamically"
    $labsPlan = & $labsSetupPath --plan --json | ConvertFrom-Json
    Assert-Distribution (
        $labsPlan.product -eq "rusty-fleet-labs" -and
        $labsPlan.channel -eq "labs"
    ) "Labs Setup plan identity is not exact"
    $labsInstall = Complete-TestSetup -Process (
        Start-TestSetup -LiteralPath $labsSetupPath -Answer i
    )
    Assert-Distribution ($labsInstall.exit_code -eq 0) "Labs Setup install failed"

    $shellRoot = Join-Path $testRoot "isolated-shell"
    $shellInstallRoot = Join-Path $testRoot "labs-shell-install"
    $shellSetupOutput = Join-Path $testRoot "labs-shell-setup"
    & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
        -Version "0.0.1" -Channel labs -Maturity alpha -BuildKind unsigned-dev `
        -BundleArchivePath $labsArchiveOne `
        -DevelopmentInstallRoot $shellInstallRoot `
        -DevelopmentShellTestRoot $shellRoot `
        -OutputDirectory $shellSetupOutput | Out-Null
    $shellSetupPath = Join-Path $shellSetupOutput "RustyFleet-Labs-Setup.exe"
    $shellInstall = Complete-TestSetup -Process (
        Start-TestSetup -LiteralPath $shellSetupPath -Answer i
    )
    $labsRegistry = Join-Path $shellRoot "Registry\rusty-fleet-labs.json"
    $stableRegistry = Join-Path $shellRoot "Registry\rusty-fleet.json"
    $stableShortcut = Join-Path $shellRoot "Programs\Rusty Fleet\stable.lnk"
    [IO.Directory]::CreateDirectory((Split-Path -Parent $stableRegistry)) | Out-Null
    [IO.Directory]::CreateDirectory((Split-Path -Parent $stableShortcut)) | Out-Null
    Write-TestArtifact -LiteralPath $stableRegistry -Content '{"stable":true}'
    Write-TestArtifact -LiteralPath $stableShortcut -Content "stable"
    $registration = Get-Content -LiteralPath $labsRegistry -Raw |
        ConvertFrom-Json
    Assert-Distribution (
        $shellInstall.exit_code -eq 0 -and
        $registration.UninstallString -match ' --uninstall$' -and
        $registration.InstallLocation -ceq $shellInstallRoot -and
        (Test-Path -LiteralPath (
            Join-Path $shellRoot "Programs\Rusty Fleet Labs\Rusty Fleet Labs.lnk"
        ))
    ) "Labs shell identity or explicit uninstall registration is incomplete"
    $uninstall = & $shellSetupPath --uninstall | ConvertFrom-Json
    Assert-Distribution (
        $uninstall.result -eq "pass" -and
        $uninstall.product -eq "rusty-fleet-labs" -and
        -not (Test-Path -LiteralPath $labsRegistry) -and
        -not (Test-Path -LiteralPath (
            Join-Path $shellRoot "Programs\Rusty Fleet Labs"
        )) -and
        (Test-Path -LiteralPath $stableRegistry) -and
        (Test-Path -LiteralPath $stableShortcut)
    ) "Labs uninstall did not remain isolated from stable shell identity"

    foreach ($uninstallFailure in @(
        "uninstall_after_shortcuts",
        "uninstall_delete_root"
    )) {
        $failureShellRoot = Join-Path $testRoot "shell-$uninstallFailure"
        $failureInstallRoot = Join-Path $testRoot "install-$uninstallFailure"
        $failureSetupOutput = Join-Path $testRoot "setup-$uninstallFailure"
        & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
            -Version "0.0.1" -Channel labs -Maturity alpha -BuildKind unsigned-dev `
            -BundleArchivePath $labsArchiveOne `
            -DevelopmentInstallRoot $failureInstallRoot `
            -DevelopmentShellTestRoot $failureShellRoot `
            -DevelopmentShellFailurePoint $uninstallFailure `
            -OutputDirectory $failureSetupOutput | Out-Null
        $failureSetup = Join-Path (
            $failureSetupOutput
        ) "RustyFleet-Labs-Setup.exe"
        $failureInstall = Complete-TestSetup -Process (
            Start-TestSetup -LiteralPath $failureSetup -Answer i
        )
        Assert-Distribution (
            $failureInstall.exit_code -eq 0
        ) "uninstall recovery fixture did not install"
        $failureRegistry = Join-Path (
            $failureShellRoot
        ) "Registry\rusty-fleet-labs.json"
        $failureShortcut = Join-Path (
            $failureShellRoot
        ) "Programs\Rusty Fleet Labs\Rusty Fleet Labs.lnk"
        $registryBefore = Get-RustyFleetSha256 -LiteralPath $failureRegistry
        $shortcutBefore = Get-RustyFleetSha256 -LiteralPath $failureShortcut
        & $failureSetup --uninstall *> $null
        Assert-Distribution (
            $LASTEXITCODE -ne 0 -and
            (Test-Path -LiteralPath $failureInstallRoot -PathType Container) -and
            (Get-RustyFleetSha256 -LiteralPath $failureRegistry) -ceq
                $registryBefore -and
            (Get-RustyFleetSha256 -LiteralPath $failureShortcut) -ceq
                $shortcutBefore
        ) "$uninstallFailure did not restore registry and shortcut identity"
    }

    $partialShellRoot = Join-Path $testRoot "shell-uninstall-partial"
    $partialInstallRoot = Join-Path $testRoot "install-uninstall-partial"
    $partialSetupOutput = Join-Path $testRoot "setup-uninstall-partial"
    & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
        -Version "0.0.1" -Channel labs -Maturity alpha -BuildKind unsigned-dev `
        -BundleArchivePath $labsArchiveOne `
        -DevelopmentInstallRoot $partialInstallRoot `
        -DevelopmentShellTestRoot $partialShellRoot `
        -DevelopmentShellFailurePoint uninstall_partial_delete `
        -OutputDirectory $partialSetupOutput | Out-Null
    $partialSetup = Join-Path $partialSetupOutput "RustyFleet-Labs-Setup.exe"
    $partialInstall = Complete-TestSetup -Process (
        Start-TestSetup -LiteralPath $partialSetup -Answer i
    )
    Assert-Distribution (
        $partialInstall.exit_code -eq 0
    ) "partial-delete recovery fixture did not install"
    $partialResult = & $partialSetup --uninstall | ConvertFrom-Json
    $partialParent = Split-Path -Parent $partialInstallRoot
    $partialQuarantine = Join-Path $partialParent $partialResult.quarantine
    $partialRecovery = Join-Path $partialParent $partialResult.recovery_receipt
    $partialReceipt = Get-Content -LiteralPath $partialRecovery -Raw |
        ConvertFrom-Json
    Assert-Distribution (
        $partialResult.result -eq "recoverable_cleanup" -and
        $partialResult.quarantine -cmatch
            '^\.rusty-fleet-labs-uninstall-[0-9a-f]{32}$' -and
        $partialResult.recovery_receipt -ceq
            "$($partialResult.quarantine).recovery.json" -and
        -not (Test-Path -LiteralPath $partialInstallRoot) -and
        (Test-Path -LiteralPath $partialQuarantine -PathType Container) -and
        -not (Test-Path -LiteralPath (
            Join-Path $partialQuarantine "state\current.json"
        )) -and
        $partialReceipt.result -eq "recoverable_cleanup" -and
        $partialReceipt.quarantine -ceq $partialResult.quarantine -and
        $partialReceipt.shell_identity_present -eq $false -and
        -not (Test-Path -LiteralPath (
            Join-Path $partialShellRoot "Registry\rusty-fleet-labs.json"
        )) -and
        -not (Test-Path -LiteralPath (
            Join-Path $partialShellRoot "Programs\Rusty Fleet Labs"
        ))
    ) "partial recursive deletion restored shell identity or lost recovery state"

    $receiptFailureShellRoot = Join-Path $testRoot "shell-receipt-failure"
    $receiptFailureInstallRoot = Join-Path $testRoot "install-receipt-failure"
    $receiptFailureSetupOutput = Join-Path $testRoot "setup-receipt-failure"
    & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
        -Version "0.0.1" -Channel labs -Maturity alpha -BuildKind unsigned-dev `
        -BundleArchivePath $labsArchiveOne `
        -DevelopmentInstallRoot $receiptFailureInstallRoot `
        -DevelopmentShellTestRoot $receiptFailureShellRoot `
        -DevelopmentShellFailurePoint `
            uninstall_partial_delete_receipt_failure `
        -OutputDirectory $receiptFailureSetupOutput | Out-Null
    $receiptFailureSetup = Join-Path (
        $receiptFailureSetupOutput
    ) "RustyFleet-Labs-Setup.exe"
    $receiptFailureInstall = Complete-TestSetup -Process (
        Start-TestSetup -LiteralPath $receiptFailureSetup -Answer i
    )
    Assert-Distribution (
        $receiptFailureInstall.exit_code -eq 0
    ) "receipt-write recovery fixture did not install"
    $receiptFailureResult = & $receiptFailureSetup --uninstall |
        ConvertFrom-Json
    $receiptFailureParent = Split-Path -Parent $receiptFailureInstallRoot
    $receiptFailureQuarantine = Join-Path (
        $receiptFailureParent
    ) $receiptFailureResult.quarantine
    Assert-Distribution (
        $receiptFailureResult.result -eq "recoverable_cleanup" -and
        $receiptFailureResult.recovery_receipt_write_failed -eq $true -and
        $null -eq $receiptFailureResult.recovery_receipt -and
        -not (Test-Path -LiteralPath $receiptFailureInstallRoot) -and
        (Test-Path `
            -LiteralPath $receiptFailureQuarantine `
            -PathType Container) -and
        -not (Test-Path -LiteralPath (
            Join-Path $receiptFailureQuarantine "state\current.json"
        )) -and
        -not (Test-Path -LiteralPath (
            Join-Path $receiptFailureShellRoot (
                "Registry\rusty-fleet-labs.json"
            )
        )) -and
        -not (Test-Path -LiteralPath (
            Join-Path $receiptFailureShellRoot "Programs\Rusty Fleet Labs"
        ))
    ) (
        "recovery receipt failure restored damaged quarantine as canonical " +
        "or recreated shell identity"
    )

    $reparseShellRoot = Join-Path $testRoot "shell-uninstall-reparse"
    $reparseInstallRoot = Join-Path $testRoot "install-uninstall-reparse"
    $reparseSetupOutput = Join-Path $testRoot "setup-uninstall-reparse"
    $quarantinesBeforeReparse = @(
        Get-ChildItem `
            -LiteralPath (Split-Path -Parent $reparseInstallRoot) `
            -Directory |
            Where-Object Name -Like '.rusty-fleet-labs-uninstall-*'
    ).Count
    & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
        -Version "0.0.1" -Channel labs -Maturity alpha -BuildKind unsigned-dev `
        -BundleArchivePath $labsArchiveOne `
        -DevelopmentInstallRoot $reparseInstallRoot `
        -DevelopmentShellTestRoot $reparseShellRoot `
        -OutputDirectory $reparseSetupOutput | Out-Null
    $reparseSetup = Join-Path $reparseSetupOutput "RustyFleet-Labs-Setup.exe"
    $reparseInstall = Complete-TestSetup -Process (
        Start-TestSetup -LiteralPath $reparseSetup -Answer i
    )
    Assert-Distribution (
        $reparseInstall.exit_code -eq 0
    ) "reparse rejection fixture did not install"
    $reparseTarget = Join-Path $testRoot "reparse-target"
    [IO.Directory]::CreateDirectory($reparseTarget) | Out-Null
    $reparsePath = Join-Path $reparseInstallRoot "unowned-junction"
    New-Item `
        -ItemType Junction `
        -Path $reparsePath `
        -Target $reparseTarget | Out-Null
    & $reparseSetup --uninstall *> $null
    Assert-Distribution (
        $LASTEXITCODE -ne 0 -and
        (Test-Path -LiteralPath $reparseInstallRoot -PathType Container) -and
        (Test-Path -LiteralPath (
            Join-Path $reparseShellRoot "Registry\rusty-fleet-labs.json"
        )) -and
        @(
            Get-ChildItem `
                -LiteralPath (Split-Path -Parent $reparseInstallRoot) `
                -Directory |
                Where-Object Name -Like '.rusty-fleet-labs-uninstall-*'
        ).Count -eq $quarantinesBeforeReparse
    ) "reparse-bearing install root entered uninstall quarantine"

    $failedShellRoot = Join-Path $testRoot "failed-shell"
    $failedInstallRoot = Join-Path $testRoot "failed-shell-install"
    $failedSetupOutput = Join-Path $testRoot "failed-shell-setup"
    & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
        -Version "0.0.1" -Channel labs -Maturity alpha -BuildKind unsigned-dev `
        -BundleArchivePath $labsArchiveOne `
        -DevelopmentInstallRoot $failedInstallRoot `
        -DevelopmentShellTestRoot $failedShellRoot `
        -DevelopmentShellFailurePoint after_registry `
        -OutputDirectory $failedSetupOutput | Out-Null
    $failedShellInstall = Complete-TestSetup -Process (
        Start-TestSetup `
            -LiteralPath (Join-Path $failedSetupOutput "RustyFleet-Labs-Setup.exe") `
            -Answer i
    )
    Assert-Distribution (
        $failedShellInstall.exit_code -ne 0 -and
        -not (Test-Path -LiteralPath (
            Join-Path $failedInstallRoot "state\current.json"
        )) -and
        -not (Test-Path -LiteralPath (
            Join-Path $failedShellRoot "Registry\rusty-fleet-labs.json"
        )) -and
        -not (Test-Path -LiteralPath (
            Join-Path $failedShellRoot "Programs\Rusty Fleet Labs"
        ))
    ) "shell failure left committed state, shortcuts, or uninstall registration"

    $labsStatePath = Join-Path $labsSetupRoot "state\current.json"
    $labsState = Get-Content -LiteralPath $labsStatePath -Raw |
        ConvertFrom-Json -Depth 20
    $labsState.current.relative_path = "../RustyFleet/releases/stable"
    [IO.File]::WriteAllText(
        $labsStatePath,
        (($labsState | ConvertTo-Json -Depth 20) + "`n"),
        [Text.UTF8Encoding]::new($false)
    )
    $crossChannelPointer = Complete-TestSetup -Process (
        Start-TestSetup -LiteralPath $labsSetupPath -Answer i
    )
    Assert-Distribution (
        $crossChannelPointer.exit_code -ne 0
    ) "Labs Setup accepted a Stable/cross-root current pointer"
    $stableAcceptedLabs = $false
    try {
        & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
            -Version "0.0.1" -Channel stable -BuildKind unsigned-dev `
            -BundleArchivePath $labsArchiveOne `
            -DevelopmentInstallRoot (Join-Path $testRoot "wrong-stable-root") `
            -OutputDirectory (Join-Path $testRoot "wrong-stable-setup") | Out-Null
        $stableAcceptedLabs = $true
    } catch {}
    Assert-Distribution (-not $stableAcceptedLabs) "Stable Setup accepted Labs artifact"
    $labsAcceptedStable = $false
    try {
        & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
            -Version "0.0.0-test.1" -Channel labs -Maturity alpha -BuildKind unsigned-dev `
            -BundleArchivePath $archiveOne `
            -DevelopmentInstallRoot (Join-Path $testRoot "wrong-labs-root") `
            -OutputDirectory (Join-Path $testRoot "wrong-labs-setup") | Out-Null
        $labsAcceptedStable = $true
    } catch {}
    Assert-Distribution (-not $labsAcceptedStable) "Labs Setup accepted Stable artifact"

    $manifestText = Get-Content `
        -LiteralPath (Join-Path $bundleOne "metadata\release-manifest.json") `
        -Raw
    $manifest = $manifestText | ConvertFrom-Json -Depth 30
    Assert-Distribution (
        @($manifest.components).Count -eq 5
    ) "manifest has an unexpected runtime component count"
    $manifestOnboard = @($manifest.components | Where-Object {
        $_.component_id -eq "fleet-onboard"
    })
    Assert-Distribution (
        $manifestOnboard.Count -eq 1 -and
        $manifestOnboard[0].kind -eq "offline_onboarding_generator" -and
        $manifestOnboard[0].entrypoint -eq
            "components/fleet-onboard/fleet-onboard.exe" -and
        $manifestOnboard[0].network_access -eq "absent"
    ) "fleet-onboard component contract is not exact"
    $manifestProvider = @($manifest.components | Where-Object {
        $_.component_id -eq "hostess-hotspot-provider"
    })
    Assert-Distribution (
        $manifestProvider.Count -eq 1 -and
        $manifestProvider[0].entrypoint -eq
        "providers/hostess-hotspot-provider/rusty-hostess-hotspot-provider.exe"
    ) "provider filename is not exact"
    Assert-Distribution (
        $manifestProvider[0].provenance.artifact_sha256 -ceq $providerSha256
    ) "provider provenance did not bind the supplied artifact"
    Assert-Distribution (
        $manifestProvider[0].provenance.owner_document_schema -eq
            "rusty.hostess.windows_hotspot.release_provenance.v2" -and
        $manifestProvider[0].provenance.distribution_eligibility -eq
            "development_only" -and
        $manifest.distribution.publication_allowed -eq $false
    ) "unsigned owner provenance or development-only eligibility is not preserved"
    Assert-Distribution (
        ($manifestProvider[0].contract.arguments -join " ") -eq
        "integration windows-hotspot --json"
    ) "provider invocation is not exact"
    Assert-Distribution (
        $manifest.install.authority -eq "RustyFleet-Setup.exe" -and
        $manifest.install.default_root -ceq
            "%LOCALAPPDATA%/RustyFleet" -and
        $manifest.install.plan_protocol -eq
            "rusty.fleet.guided_installer_plan.v2" -and
        $manifest.update.strategy -eq
            "setup_owned_side_by_side_manifest" -and
        $manifest.update.rollback.supported -eq $true -and
        $manifest.update.rollback.mode -eq
            "setup_owned_pointer_to_previous_fully_verified_release" -and
        $manifest.update.rollback.automatic_delete -eq $false
    ) "update and rollback metadata is incomplete"
    Assert-Distribution (
        $manifest.source.revision -ceq ("1" * 40) -and
        $manifest.source.tree -ceq ("2" * 40)
    ) "inner manifest source commit and tree binding is not exact"
    Assert-Distribution (
        $manifestText -notmatch [Regex]::Escape($testRoot)
    ) "manifest leaked a local test path"
    Assert-Distribution (
        @($manifest.excluded_payload_classes) -contains "credentials" -and
        @($manifest.excluded_payload_classes) -contains "private_configuration" -and
        @($manifest.excluded_payload_classes) -contains "adb"
    ) "manifest private payload exclusions are incomplete"

    $setupOutput = Join-Path $testRoot "setup"
    $setupInstallRoot = Join-Path $testRoot "setup-installed"
    $rollbackShellRoot = Join-Path $testRoot "rollback-shell"
    $setupReceipt = & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
        -Version "0.0.0-test.1" `
        -Channel dev `
        -BuildKind unsigned-dev `
        -BundleArchivePath (Join-Path $outputOne "$bundleNameOne.zip") `
        -DevelopmentInstallRoot $setupInstallRoot `
        -DevelopmentShellTestRoot $rollbackShellRoot `
        -DevelopmentTestPauseAfterRetainMs 2500 `
        -OutputDirectory $setupOutput |
        ConvertFrom-Json
    $setupPath = Join-Path $setupOutput "RustyFleet-Setup.exe"
    Assert-Distribution (
        $setupReceipt.result -eq "pass" -and
        $setupReceipt.version -eq "0.0.0-test.1" -and
        $setupReceipt.product_channel -eq "stable" -and
        $setupReceipt.maturity -eq "released" -and
        $setupReceipt.channel -eq "dev" -and
        $setupReceipt.distribution_track -eq "local-development" -and
        $setupReceipt.build_kind -eq "unsigned-dev" -and
        $setupReceipt.bundle_sha256 -ceq (
            Get-RustyFleetSha256 -LiteralPath (
                Join-Path $outputOne "$bundleNameOne.zip"
            )
        ) -and
        $setupReceipt.manifest_sha256 -ceq (
            Get-RustyFleetSha256 -LiteralPath (
                Join-Path $bundleOne "metadata\release-manifest.json"
            )
        ) -and
        $setupReceipt.canonical_pe_payload_sha256 -cmatch "^[0-9a-f]{64}$" -and
        $setupReceipt.canonical_pe_payload_size_bytes -gt 0 -and
        @($setupReceipt.PSObject.Properties.Name) -notcontains "setup_path" -and
        $setupReceipt.source_revision -ceq ("1" * 40) -and
        $setupReceipt.source_tree -ceq ("2" * 40) -and
        $setupReceipt.source_tree_clean -eq $false -and
        $setupReceipt.distribution_eligibility -eq "development_only" -and
        (Test-Path -LiteralPath $setupPath -PathType Leaf)
    ) "self-contained Setup build did not produce an exact path-free development receipt"
    Assert-Distribution (
        (($setupReceipt | ConvertTo-Json -Depth 10) -notmatch
            [Regex]::Escape($testRoot))
    ) "Setup build receipt leaked a machine-local path"

    Assert-Distribution (
        -not (Test-Path -LiteralPath $setupInstallRoot)
    ) "Setup build mutated the install root"
    $setupPlan = & $setupPath --plan --json | ConvertFrom-Json
    $setupSha256 = Get-RustyFleetSha256 -LiteralPath $setupPath
    Assert-Distribution (
        $LASTEXITCODE -eq 0 -and
        $setupPlan.schema -eq "rusty.fleet.guided_installer_plan.v2" -and
        $setupPlan.product -eq "rusty-fleet" -and
        $setupPlan.version -eq "0.0.0-test.1" -and
        $setupPlan.channel -eq "dev" -and
        $setupPlan.asset_sha256 -ceq $setupSha256 -and
        $setupPlan.authenticode_trust_mode -ceq "unsigned-development" -and
        $null -eq $setupPlan.signer_certificate_sha256 -and
        $setupPlan.signer_self_issued -eq $false -and
        $setupPlan.public_trust_claim -eq $false -and
        $setupPlan.timestamp_required -eq $false -and
        $setupPlan.ready -eq $true -and
        @($setupPlan.PSObject.Properties).Count -eq 11
    ) "Setup planning contract is not exact"
    Assert-Distribution (
        -not (Test-Path -LiteralPath $setupInstallRoot)
    ) "Setup planning mutated the install root"
    & $setupPath --unknown *> $null
    Assert-Distribution (
        $LASTEXITCODE -ne 0
    ) "Setup accepted arguments outside its closed interface"
    [System.IO.Directory]::CreateDirectory(
        (Join-Path $setupInstallRoot "releases\.candidate-interrupted-fixture")
    ) | Out-Null
    Write-TestArtifact `
        -LiteralPath (
            Join-Path $setupInstallRoot (
                "releases\.candidate-interrupted-fixture\partial.txt"
            )
        ) `
        -Content "unreferenced interruption evidence`n"
    $interrupted = Start-TestSetup -LiteralPath $setupPath -Answer i
    $interruptedCandidate = Wait-TestCandidate -InstallRoot $setupInstallRoot
    $retainedLeaf = Join-Path $interruptedCandidate (
        "components\fleetctl\fleetctl.exe"
    )
    $renameDenied = $false
    try {
        Move-Item `
            -LiteralPath $retainedLeaf `
            -Destination "$retainedLeaf.moved" `
            -ErrorAction Stop
    }
    catch {
        $renameDenied = $true
    }
    Assert-Distribution $renameDenied "retained Setup payload leaf allowed rename substitution"
    $interrupted.Kill($true)
    $interrupted.WaitForExit()
    $interrupted.Dispose()
    Assert-Distribution (
        -not (Test-Path -LiteralPath (
            Join-Path $setupInstallRoot "state\current.json"
        )) -and
        (Test-Path -LiteralPath $interruptedCandidate -PathType Container)
    ) "interrupted candidate was activated or did not remain inert"

    $installOne = Start-TestSetup -LiteralPath $setupPath -Answer i
    $installOneResult = Complete-TestSetup -Process $installOne
    $installOne.Dispose()
    Assert-Distribution (
        $installOneResult.exit_code -eq 0 -and
        [string]::IsNullOrEmpty($installOneResult.stderr) -and
        (Test-Path -LiteralPath (
            Join-Path $setupInstallRoot "state\current.json"
        ) -PathType Leaf)
    ) (
        "guided Setup did not complete a controlled per-user-style install: " +
        "exit=$($installOneResult.exit_code); stdout=" +
        $installOneResult.stdout.Trim() + "; stderr=" +
        $installOneResult.stderr.Trim()
    )
    $guidedState = Get-Content -LiteralPath (
        Join-Path $setupInstallRoot "state\current.json"
    ) -Raw | ConvertFrom-Json
    $guidedReleaseRoot = Join-Path $setupInstallRoot (
        $guidedState.current.relative_path.Replace("/", "\")
    )
    Assert-Distribution (
        $guidedState.schema -eq "rusty.fleet.windows_setup_state.v2" -and
        $guidedState.current.version -eq "0.0.0-test.1" -and
        @($guidedState.history).Count -eq 0 -and
        $guidedState.policy.rollback -eq
            "previous_fully_verified_release" -and
        $guidedState.current.relative_path -ne
            "releases/.candidate-interrupted-fixture" -and
        (Test-Path -LiteralPath (
            Join-Path $guidedReleaseRoot "components\fleet-onboard\fleet-onboard.exe"
        ) -PathType Leaf)
    ) "guided Setup state or fleet-onboard installation is not exact"
    Assert-Distribution (
        -not (Test-Path -LiteralPath (
            Join-Path $bundleOne "distribution-tools\Install-RustyFleet.ps1"
        ))
    ) "legacy split-authority installer remained in the bundle"

    $tamperedRoot = Join-Path $testRoot "tampered"
    Copy-Item -LiteralPath $bundleOne -Destination $tamperedRoot -Recurse
    Add-Content `
        -LiteralPath (Join-Path $tamperedRoot "components\fleetctl\fleetctl.exe") `
        -Value "tamper"
    $tamperRejected = $false
    try {
        & (Join-Path $packagingRoot "Test-WindowsBundle.ps1") `
            -BundleRoot $tamperedRoot | Out-Null
    }
    catch {
        $tamperRejected = $true
    }
    Assert-Distribution $tamperRejected "tampered payload was accepted"

    $extraRoot = Join-Path $testRoot "extra"
    Copy-Item -LiteralPath $bundleOne -Destination $extraRoot -Recurse
    Write-TestArtifact `
        -LiteralPath (Join-Path $extraRoot "components\fleetctl\unexpected.txt") `
        -Content "unexpected`n"
    $extraRejected = $false
    try {
        & (Join-Path $packagingRoot "Test-WindowsBundle.ps1") `
            -BundleRoot $extraRoot | Out-Null
    }
    catch {
        $extraRejected = $true
    }
    Assert-Distribution $extraRejected "unmanifested payload was accepted"

    Write-TestArtifact -LiteralPath $fleetctl -Content "fleetctl-test-artifact-v2`n"
    $outputThree = Join-Path $testRoot "three"
    Invoke-TestBundle `
        -Version "0.0.0-test.2" `
        -OutputDirectory $outputThree `
        -ConsoleDirectory $console `
        -HubPath $hub `
        -FleetctlPath $fleetctl `
        -FleetOnboardPath $fleetOnboard `
        -ProviderPath $provider `
        -ProviderSha256 $providerSha256 `
        -ProviderMetadataDirectory $providerMetadata `
        -SourceRevision ("3" * 40) | Out-Null
    $bundleTwoVersion = Join-Path $outputThree "RustyFleet-0.0.0-test.2-win-x64"
    $setupTwoOutput = Join-Path $testRoot "setup-two"
    $setupTwoReceipt = & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
        -Version "0.0.0-test.2" `
        -Channel dev `
        -BuildKind unsigned-dev `
        -BundleArchivePath (
            Join-Path $outputThree "RustyFleet-0.0.0-test.2-win-x64.zip"
        ) `
        -DevelopmentInstallRoot $setupInstallRoot `
        -DevelopmentShellTestRoot $rollbackShellRoot `
        -DevelopmentTestPauseAfterRetainMs 2500 `
        -OutputDirectory $setupTwoOutput |
        ConvertFrom-Json
    $setupTwoPath = Join-Path $setupTwoOutput "RustyFleet-Setup.exe"
    Assert-Distribution (
        $setupTwoReceipt.result -eq "pass" -and
        (Test-Path -LiteralPath $setupTwoPath -PathType Leaf)
    ) "second Setup build did not produce an update authority"

    $update = Start-TestSetup -LiteralPath $setupTwoPath -Answer i
    $updateResult = Complete-TestSetup -Process $update
    $update.Dispose()
    $statePath = Join-Path $setupInstallRoot "state\current.json"
    $updatedState = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    Assert-Distribution (
        $updateResult.exit_code -eq 0 -and
        $updatedState.current.version -eq "0.0.0-test.2" -and
        $updatedState.history[0].version -eq "0.0.0-test.1"
    ) (
        "Setup-owned update did not preserve previous fully verified release: " +
        "exit=$($updateResult.exit_code); stdout=" +
        $updateResult.stdout.Trim() + "; stderr=" +
        $updateResult.stderr.Trim() + "; state=" +
        ($updatedState | ConvertTo-Json -Compress -Depth 10)
    )

    $oldReleaseRoot = Join-Path $setupInstallRoot (
        $updatedState.history[0].relative_path.Replace("/", "\")
    )
    $oldFleetctl = Join-Path $oldReleaseRoot "components\fleetctl\fleetctl.exe"
    Add-Content -LiteralPath $oldFleetctl -Value "corrupt-old-payload"
    $corruptRollback = Start-TestSetup -LiteralPath $setupTwoPath -Answer r
    $corruptRollbackResult = Complete-TestSetup -Process $corruptRollback
    $corruptRollback.Dispose()
    $stateAfterCorruptRollback = Get-Content -LiteralPath $statePath -Raw |
        ConvertFrom-Json
    Assert-Distribution (
        $corruptRollbackResult.exit_code -ne 0 -and
        $stateAfterCorruptRollback.current.version -eq "0.0.0-test.2"
    ) "corrupt historical payload was accepted for rollback or mutated state"
    Copy-Item `
        -LiteralPath (Join-Path $bundleOne "components\fleetctl\fleetctl.exe") `
        -Destination $oldFleetctl `
        -Force

    $hardLink = Join-Path $testRoot "old-fleetctl-hardlink.exe"
    New-Item -ItemType HardLink -Path $hardLink -Target $oldFleetctl | Out-Null
    $hardLinkRollback = Start-TestSetup -LiteralPath $setupTwoPath -Answer r
    $hardLinkRollbackResult = Complete-TestSetup -Process $hardLinkRollback
    $hardLinkRollback.Dispose()
    Assert-Distribution (
        $hardLinkRollbackResult.exit_code -ne 0
    ) "hard-linked historical payload was accepted for rollback"
    Remove-Item -LiteralPath $hardLink -Force

    $rollback = Start-TestSetup -LiteralPath $setupTwoPath -Answer r
    $rollbackResult = Complete-TestSetup -Process $rollback
    $rollback.Dispose()
    $rolledBackState = Get-Content -LiteralPath $statePath -Raw |
        ConvertFrom-Json
    $rolledBackRegistration = Get-Content -LiteralPath (
        Join-Path $rollbackShellRoot "Registry\rusty-fleet.json"
    ) -Raw | ConvertFrom-Json
    $rolledBackShortcut = Get-Content -LiteralPath (
        Join-Path $rollbackShellRoot "Programs\Rusty Fleet\Rusty Fleet.lnk"
    ) -Raw | ConvertFrom-Json
    $rolledBackReleaseRoot = Join-Path $setupInstallRoot (
        $rolledBackState.current.relative_path.Replace("/", "\")
    )
    Assert-Distribution (
        $rollbackResult.exit_code -eq 0 -and
        $rolledBackState.current.version -eq "0.0.0-test.1" -and
        $rolledBackState.history[0].version -eq "0.0.0-test.2" -and
        $rolledBackRegistration.DisplayVersion -eq "0.0.0-test.1" -and
        $rolledBackShortcut.working_directory -ceq $rolledBackReleaseRoot
    ) "Setup-owned fully verified pointer rollback failed"

    $concurrentOne = Start-TestSetup -LiteralPath $setupTwoPath -Answer i
    $concurrentTwo = Start-TestSetup -LiteralPath $setupTwoPath -Answer i
    $concurrentResultOne = Complete-TestSetup `
        -Process $concurrentOne `
        -TimeoutMilliseconds 45000
    $concurrentResultTwo = Complete-TestSetup `
        -Process $concurrentTwo `
        -TimeoutMilliseconds 45000
    $concurrentOne.Dispose()
    $concurrentTwo.Dispose()
    $concurrentState = Get-Content -LiteralPath $statePath -Raw |
        ConvertFrom-Json
    Assert-Distribution (
        $concurrentResultOne.exit_code -eq 0 -and
        $concurrentResultTwo.exit_code -eq 0 -and
        $concurrentState.current.version -eq "0.0.0-test.2" -and
        @($concurrentState.history).Count -eq 1 -and
        $concurrentState.history[0].version -eq "0.0.0-test.1"
    ) "exclusive Setup transaction did not serialize concurrent updates"

    [ordered]@{
        schema = "rusty.fleet.windows_distribution_test.v1"
        result = "pass"
        deterministic_archive = $true
        exact_composition = $true
        provider_hash_pinned = $true
        provider_contract_exact = $true
        owner_license_and_notices_bound = $true
        unsigned_rebuild_bound_to_canonical_payload = $true
        malformed_pe_layout_rejected = $true
        provenance_timestamp_culture_invariant = $true
        signer_authorization_logic = $true
        unauthorized_signed_provenance_rejected = $true
        physically_unsigned_release_rejected = $true
        unsigned_dev_non_distributable = $true
        tamper_rejected = $true
        extra_payload_rejected = $true
        plan_only_no_mutation = $true
        setup_embeds_exact_bundle = $true
        setup_plan_contract_exact = $true
        guided_setup_install_verified = $true
        interrupted_candidate_inert_and_recoverable = $true
        retained_leaf_rename_denied = $true
        historical_payload_tamper_rejected = $true
        historical_payload_hardlink_rejected = $true
        concurrent_setup_serialized = $true
        side_by_side_update = $true
        pointer_only_rollback = $true
        credentials_private_config_adb_absent = $true
    } | ConvertTo-Json -Depth 10
}
finally {
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
