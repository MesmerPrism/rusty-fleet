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
        [Parameter(Mandatory)][string] $ProviderPath,
        [Parameter(Mandatory)][string] $ProviderSha256,
        [Parameter(Mandatory)][string] $ProviderMetadataDirectory,
        [string] $SourceRevision = ("1" * 40)
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
        -SourceDateEpoch 1785110400 `
        -SkipBuild `
        -ConsoleArtifactDirectory $ConsoleDirectory `
        -HubArtifactPath $HubPath `
        -FleetctlArtifactPath $FleetctlPath
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
    $provider = Join-Path $inputs "rusty-hostess-hotspot-provider.exe"
    $providerRevision = "2" * 40
    $providerVersion = "0.0.0-test.1"
    $providerProductVersion = "$providerVersion+$providerRevision"
    Write-TestArtifact -LiteralPath $consoleExe -Content "console-test-artifact`n"
    Write-TestArtifact -LiteralPath $consoleRuntime -Content "{`"runtimeOptions`":{}}`n"
    Write-TestArtifact -LiteralPath $hub -Content "hub-test-artifact`n"
    Write-TestArtifact -LiteralPath $fleetctl -Content "fleetctl-test-artifact`n"
    Write-TestProviderArtifact `
        -LiteralPath $provider `
        -ProductVersion $providerProductVersion
    $providerSha256 = Get-RustyFleetSha256 -LiteralPath $provider
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
        -ProviderPath $provider `
        -ProviderSha256 $providerSha256 `
        -ProviderMetadataDirectory $providerMetadata | Out-Null
    Invoke-TestBundle `
        -Version "0.0.0-test.1" `
        -OutputDirectory $outputTwo `
        -ConsoleDirectory $console `
        -HubPath $hub `
        -FleetctlPath $fleetctl `
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
    Assert-Distribution ($validation.runtime_components -eq 4) "runtime composition is not exact"

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
        @($manifest.components).Count -eq 4
    ) "manifest has an unexpected runtime component count"
    Assert-Distribution (
        $manifest.components[3].entrypoint -eq
        "providers/hostess-hotspot-provider/rusty-hostess-hotspot-provider.exe"
    ) "provider filename is not exact"
    Assert-Distribution (
        $manifest.components[3].provenance.artifact_sha256 -ceq $providerSha256
    ) "provider provenance did not bind the supplied artifact"
    Assert-Distribution (
        $manifest.components[3].provenance.owner_document_schema -eq
            "rusty.hostess.windows_hotspot.release_provenance.v1" -and
        $manifest.components[3].provenance.distribution_eligibility -eq
            "development_only" -and
        $manifest.distribution.publication_allowed -eq $false
    ) "unsigned owner provenance or development-only eligibility is not preserved"
    Assert-Distribution (
        ($manifest.components[3].contract.arguments -join " ") -eq
        "integration windows-hotspot --json"
    ) "provider invocation is not exact"
    Assert-Distribution (
        $manifest.update.strategy -eq "side_by_side_manifest" -and
        $manifest.update.rollback.supported -eq $true -and
        $manifest.update.rollback.automatic_delete -eq $false
    ) "update and rollback metadata is incomplete"
    Assert-Distribution (
        $manifestText -notmatch [Regex]::Escape($testRoot)
    ) "manifest leaked a local test path"
    Assert-Distribution (
        @($manifest.excluded_payload_classes) -contains "credentials" -and
        @($manifest.excluded_payload_classes) -contains "private_configuration" -and
        @($manifest.excluded_payload_classes) -contains "adb"
    ) "manifest private payload exclusions are incomplete"

    $planInstallRoot = Join-Path $testRoot "plan-only-install"
    $plan = & (Join-Path $bundleOne "distribution-tools\Install-RustyFleet.ps1") `
        -Action Install `
        -BundleRoot $bundleOne `
        -InstallRoot $planInstallRoot |
        ConvertFrom-Json
    Assert-Distribution (
        $plan.schema -eq "rusty.fleet.windows_install_plan.v1" -and
        $plan.execute -eq $false -and
        -not (Test-Path -LiteralPath $planInstallRoot)
    ) "plan-only installer mutated the target"
    Assert-Distribution (
        @($plan.side_effects_excluded) -contains "no ADB installation or invocation"
    ) "installer plan does not explicitly exclude ADB"

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

    $installRoot = Join-Path $testRoot "installed"
    & (Join-Path $bundleOne "distribution-tools\Install-RustyFleet.ps1") `
        -Action Install `
        -Execute `
        -BundleRoot $bundleOne `
        -InstallRoot $installRoot | Out-Null
    $statePath = Join-Path $installRoot "state\current.json"
    $firstState = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    Assert-Distribution (
        $firstState.current.version -eq "0.0.0-test.1" -and
        @($firstState.history).Count -eq 0
    ) "first side-by-side install state is invalid"

    Write-TestArtifact -LiteralPath $fleetctl -Content "fleetctl-test-artifact-v2`n"
    $outputThree = Join-Path $testRoot "three"
    Invoke-TestBundle `
        -Version "0.0.0-test.2" `
        -OutputDirectory $outputThree `
        -ConsoleDirectory $console `
        -HubPath $hub `
        -FleetctlPath $fleetctl `
        -ProviderPath $provider `
        -ProviderSha256 $providerSha256 `
        -ProviderMetadataDirectory $providerMetadata `
        -SourceRevision ("3" * 40) | Out-Null
    $bundleTwoVersion = Join-Path $outputThree "RustyFleet-0.0.0-test.2-win-x64"
    & (Join-Path $bundleTwoVersion "distribution-tools\Install-RustyFleet.ps1") `
        -Action Install `
        -Execute `
        -BundleRoot $bundleTwoVersion `
        -InstallRoot $installRoot | Out-Null
    $updatedState = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    Assert-Distribution (
        $updatedState.current.version -eq "0.0.0-test.2" -and
        $updatedState.history[0].version -eq "0.0.0-test.1"
    ) "update did not preserve previous verified release metadata"

    $rollbackPlan = & (
        Join-Path $bundleTwoVersion "distribution-tools\Install-RustyFleet.ps1"
    ) `
        -Action Rollback `
        -BundleRoot $bundleTwoVersion `
        -InstallRoot $installRoot |
        ConvertFrom-Json
    $stateAfterRollbackPlan = Get-Content -LiteralPath $statePath -Raw |
        ConvertFrom-Json
    Assert-Distribution (
        $rollbackPlan.target_version -eq "0.0.0-test.1" -and
        $rollbackPlan.execute -eq $false -and
        $stateAfterRollbackPlan.current.version -eq "0.0.0-test.2"
    ) "rollback plan either targeted the wrong release or mutated state"

    & (Join-Path $bundleTwoVersion "distribution-tools\Install-RustyFleet.ps1") `
        -Action Rollback `
        -Execute `
        -BundleRoot $bundleTwoVersion `
        -InstallRoot $installRoot | Out-Null
    $rolledBackState = Get-Content -LiteralPath $statePath -Raw |
        ConvertFrom-Json
    Assert-Distribution (
        $rolledBackState.current.version -eq "0.0.0-test.1" -and
        $rolledBackState.history[0].version -eq "0.0.0-test.2"
    ) "verified pointer-only rollback failed"

    [ordered]@{
        schema = "rusty.fleet.windows_distribution_test.v1"
        result = "pass"
        deterministic_archive = $true
        exact_composition = $true
        provider_hash_pinned = $true
        provider_contract_exact = $true
        owner_license_and_notices_bound = $true
        unsigned_rebuild_bound_to_canonical_payload = $true
        signer_authorization_logic = $true
        unauthorized_signed_provenance_rejected = $true
        physically_unsigned_release_rejected = $true
        unsigned_dev_non_distributable = $true
        tamper_rejected = $true
        extra_payload_rejected = $true
        plan_only_no_mutation = $true
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
