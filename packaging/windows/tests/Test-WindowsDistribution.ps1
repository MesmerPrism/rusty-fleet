# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

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
        [string] $SourceRevision = ("1" * 40),
        [string] $SourceTree = ("2" * 40)
    )

    & (Join-Path $packagingRoot "New-WindowsBundle.ps1") `
        -Version $Version `
        -Channel dev `
        -BuildKind unsigned-dev `
        -HostessProviderPath $ProviderPath `
        -HostessProviderSha256 $ProviderSha256 `
        -HostessProviderMetadataDirectory $ProviderMetadataDirectory `
        -OutputDirectory $OutputDirectory `
        -SourceRevision $SourceRevision `
        -SourceTree $SourceTree `
        -SourceDateEpoch 1785110400 `
        -SkipBuild `
        -ConsoleArtifactDirectory $ConsoleDirectory `
        -HubArtifactPath $HubPath `
        -FleetctlArtifactPath $FleetctlPath `
        -FleetOnboardArtifactPath $FleetOnboardPath
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
    $providerProvenance = [ordered]@{
        schema = "rusty.hostess.windows_hotspot.release_provenance.v1"
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
            status = "NotSigned"
            subject = $null
            thumbprint = $null
            authorized_thumbprint = $null
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
            -BuildKind unsigned-dev | Out-Null
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
    $signedProvenance = (
        ConvertTo-RustyFleetJson -InputObject $providerProvenance |
        ConvertFrom-Json -Depth 30
    )
    $signedProvenance.build.kind = "signed-release"
    $signedProvenance.source.availability_state = "verified_public"
    $signedProvenance.source.verified_at_utc = "2026-07-27T00:00:00Z"
    $signedProvenance.signing.state = "verified"
    $signedProvenance.signing.status = "Valid"
    $signedProvenance.signing.subject = "CN=Synthetic Test Signer"
    $signedProvenance.signing.thumbprint = "6" * 40
    $signedProvenance.signing.authorized_thumbprint = "6" * 40
    $signedProvenance.distribution.eligibility = "signed_release"
    Write-TestArtifact `
        -LiteralPath (
            Join-Path $signedMetadata (
                "rusty-hostess-hotspot-provider.provenance.json"
            )
        ) `
        -Content (ConvertTo-RustyFleetJson -InputObject $signedProvenance)
    & $distributionModule {
        param($Provenance, $ExpectedSigner)
        Assert-RustyFleetHostessSignerAuthorization `
            -Provenance $Provenance `
            -ObservedSubject "CN=Synthetic Test Signer" `
            -ObservedThumbprint ("6" * 40) `
            -ExpectedSignerThumbprint $ExpectedSigner
    } $signedProvenance ("6" * 40)
    foreach ($expectedSigner in @($null, ("8" * 40))) {
        $unauthorizedSignedRejected = $false
        try {
            & $distributionModule {
                param($Provenance, $ExpectedSigner)
                Assert-RustyFleetHostessSignerAuthorization `
                    -Provenance $Provenance `
                    -ObservedSubject "CN=Synthetic Test Signer" `
                    -ObservedThumbprint ("6" * 40) `
                    -ExpectedSignerThumbprint $ExpectedSigner
            } $signedProvenance $expectedSigner
        }
        catch {
            $unauthorizedSignedRejected = $true
        }
        Assert-Distribution `
            $unauthorizedSignedRejected `
            "signed provenance without the independently authorized signer was accepted"
    }
    $physicallyUnsignedReleaseRejected = $false
    try {
        Read-RustyFleetHostessProvenance `
            -MetadataDirectory $signedMetadata `
            -ProviderPath $provider `
            -ProviderSha256 $providerSha256 `
            -BuildKind signed-release `
            -ExpectedSignerThumbprint ("6" * 40) | Out-Null
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
            "rusty.hostess.windows_hotspot.release_provenance.v1" -and
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
        $manifest.install.plan_protocol -eq
            "rusty.fleet.guided_installer_plan.v1" -and
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
    $setupReceipt = & (Join-Path $packagingRoot "New-WindowsSetup.ps1") `
        -Version "0.0.0-test.1" `
        -Channel dev `
        -BuildKind unsigned-dev `
        -BundleArchivePath (Join-Path $outputOne "$bundleNameOne.zip") `
        -DevelopmentInstallRoot $setupInstallRoot `
        -DevelopmentTestPauseAfterRetainMs 2500 `
        -OutputDirectory $setupOutput |
        ConvertFrom-Json
    $setupPath = Join-Path $setupOutput "RustyFleet-Setup.exe"
    Assert-Distribution (
        $setupReceipt.result -eq "pass" -and
        $setupReceipt.version -eq "0.0.0-test.1" -and
        $setupReceipt.channel -eq "dev" -and
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
        $setupPlan.schema -eq "rusty.fleet.guided_installer_plan.v1" -and
        $setupPlan.product -eq "rusty-fleet" -and
        $setupPlan.version -eq "0.0.0-test.1" -and
        $setupPlan.channel -eq "dev" -and
        $setupPlan.asset_sha256 -ceq $setupSha256 -and
        $setupPlan.ready -eq $true -and
        @($setupPlan.PSObject.Properties).Count -eq 6
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
    Assert-Distribution (
        $rollbackResult.exit_code -eq 0 -and
        $rolledBackState.current.version -eq "0.0.0-test.1" -and
        $rolledBackState.history[0].version -eq "0.0.0-test.2"
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
