# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [switch] $BaselineOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Publication([bool] $Condition, [string] $Message) {
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

function Invoke-TestGit {
    param(
        [Parameter(Mandatory)][string] $Repository,
        [Parameter(Mandatory)][string[]] $Arguments
    )

    $output = @(& git -C $Repository @Arguments)
    if ($LASTEXITCODE -ne 0) {
        throw "test Git command failed: $($Arguments -join ' ')"
    }
    return ($output -join "`n").Trim()
}

function Invoke-PublicationAuthority {
    param(
        [Parameter(Mandatory)][string] $Mode,
        [Parameter(Mandatory)][string] $InputRoot,
        [Parameter(Mandatory)][string] $SourceRepository,
        [Parameter(Mandatory)][string] $SourceRevision,
        [Parameter(Mandatory)][string] $SourceTree,
        [Parameter(Mandatory)][string] $FleetSigner,
        [Parameter(Mandatory)][string] $HostessSigner,
        [Parameter(Mandatory)][string] $DescriptorSpki,
        [Parameter(Mandatory)][string] $GhExecutable
    )

    & (Join-Path $packagingRoot "Publish-WindowsRelease.ps1") `
        -Mode $Mode `
        -AssetDirectory $InputRoot `
        -Version "1.2.3" `
        -Channel preview `
        -ExpectedFleetSignerThumbprint $FleetSigner `
        -ExpectedHostessSignerThumbprint $HostessSigner `
        -ExpectedDescriptorSignerSpkiSha256 $DescriptorSpki `
        -ExpectedSourceRevision $SourceRevision `
        -ExpectedSourceTree $SourceTree `
        -RepositoryRoot $SourceRepository `
        -ExpectedRef "refs/tags/v1.2.3" `
        -GhExecutable $GhExecutable
}

$packagingRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Import-Module (Join-Path $packagingRoot "Distribution.Common.psm1") -Force
$distributionModule = Get-Module Distribution.Common
$testRoot = Join-Path ([IO.Path]::GetTempPath()) (
    "rusty-fleet-publication-test-$([Guid]::NewGuid().ToString('N'))"
)
[IO.Directory]::CreateDirectory($testRoot) | Out-Null
$descriptorRsa = [Security.Cryptography.RSA]::Create(3072)
$signingCertificate = $null
$certificateStoreNames = @("Root", "TrustedPublisher")
$priorToken = [Environment]::GetEnvironmentVariable(
    "GH_TOKEN",
    [EnvironmentVariableTarget]::Process
)

try {
    $timestampJson = (
        '{"verified_at_utc":"2026-07-27T00:00:00Z"}'
    )
    Assert-Publication (
        $timestampJson -cmatch
            '"verified_at_utc":"2026-07-27T00:00:00Z"'
    ) "serialized provenance timestamp fixture is not canonical UTC"
    $priorCulture = [Globalization.CultureInfo]::CurrentCulture
    $priorUiCulture = [Globalization.CultureInfo]::CurrentUICulture
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
                $timestampJson | ConvertFrom-Json
            ).verified_at_utc
            $parsedTimestamp = & $distributionModule {
                param($Value)
                ConvertTo-RustyFleetUtcDateTimeOffset `
                    -Value $Value `
                    -Context "test provenance timestamp"
            } $typedTimestamp
            Assert-Publication (
                $typedTimestamp -is [DateTime] -and
                $parsedTimestamp.ToUnixTimeMilliseconds() -eq 1785110400000
            ) "typed provenance timestamp was culture-dependent: $cultureName"
        }
    }
    finally {
        [Globalization.CultureInfo]::CurrentCulture = $priorCulture
        [Globalization.CultureInfo]::CurrentUICulture = $priorUiCulture
    }

    $signingCertificate = New-SelfSignedCertificate `
        -Type CodeSigningCert `
        -Subject "CN=Rusty Fleet publication test" `
        -CertStoreLocation "Cert:\CurrentUser\My" `
        -HashAlgorithm SHA256 `
        -NotAfter ([DateTime]::UtcNow.AddDays(1))
    foreach ($storeName in $certificateStoreNames) {
        $store = [Security.Cryptography.X509Certificates.X509Store]::new(
            $storeName,
            [Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser
        )
        try {
            $store.Open(
                [Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite
            )
            $store.Add($signingCertificate)
        }
        finally {
            $store.Dispose()
        }
    }
    $signerThumbprint = $signingCertificate.Thumbprint.ToUpperInvariant()
    $descriptorSpki = $descriptorRsa.ExportSubjectPublicKeyInfo()
    $descriptorSpkiSha256 = [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData($descriptorSpki)
    ).ToLowerInvariant()

    $sourceRepo = Join-Path $testRoot "source"
    $policyPath = Join-Path $sourceRepo (
        "packaging\windows\trust\release-policy.json"
    )
    $policy = [ordered]@{
        schema = "rusty.fleet.windows_release_trust_policy.v1"
        publication_enabled = $true
        authorized_fleet_signer_thumbprints = @($signerThumbprint)
        authorized_hostess_signer_thumbprints = @($signerThumbprint)
        authorized_descriptor_signer_spki_sha256 = @(
            $descriptorSpkiSha256
        )
        status = "test_authority_only"
    }
    Write-TestUtf8 `
        -LiteralPath $policyPath `
        -Content (ConvertTo-RustyFleetJson -InputObject $policy)
    & git init --initial-branch=main $sourceRepo | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "could not initialize publication source fixture"
    }
    Invoke-TestGit -Repository $sourceRepo -Arguments @(
        "config", "user.name", "Rusty Fleet tests"
    ) | Out-Null
    Invoke-TestGit -Repository $sourceRepo -Arguments @(
        "config", "user.email", "tests@invalid.example"
    ) | Out-Null
    Invoke-TestGit -Repository $sourceRepo -Arguments @(
        "config", "core.autocrlf", "false"
    ) | Out-Null
    Invoke-TestGit -Repository $sourceRepo -Arguments @("add", ".") | Out-Null
    Invoke-TestGit -Repository $sourceRepo -Arguments @(
        "commit", "-m", "test release authority"
    ) | Out-Null
    Invoke-TestGit -Repository $sourceRepo -Arguments @(
        "tag", "v1.2.3"
    ) | Out-Null
    $sourceRevision = Invoke-TestGit `
        -Repository $sourceRepo `
        -Arguments @("rev-parse", "HEAD")
    $sourceTree = Invoke-TestGit `
        -Repository $sourceRepo `
        -Arguments @("rev-parse", "HEAD^{tree}")

    $providerRevision = "2" * 40
    $providerVersion = "0.0.0-test.1"
    $providerProductVersion = "$providerVersion+$providerRevision"
    $providerProject = Join-Path $testRoot "provider-project"
    $providerOutput = Join-Path $providerProject "output"
    Write-TestUtf8 `
        -LiteralPath (Join-Path $providerProject "Provider.csproj") `
        -Content @"
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net10.0</TargetFramework>
    <DebugType>None</DebugType>
    <DebugSymbols>false</DebugSymbols>
    <Deterministic>true</Deterministic>
    <AssemblyName>rusty-hostess-hotspot-provider</AssemblyName>
    <Version>$providerVersion</Version>
    <InformationalVersion>$providerProductVersion</InformationalVersion>
  </PropertyGroup>
</Project>
"@
    Write-TestUtf8 `
        -LiteralPath (Join-Path $providerProject "Program.cs") `
        -Content @"
namespace RustyFleetPublicationTests;
public static class ProviderFixture { }
"@
    & dotnet build `
        (Join-Path $providerProject "Provider.csproj") `
        --nologo `
        --configuration Release `
        --output $providerOutput |
        Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "could not build publication provider fixture"
    }
    $providerPath = Join-Path $testRoot (
        "rusty-hostess-hotspot-provider.exe"
    )
    Copy-Item `
        -LiteralPath (
            Join-Path $providerOutput "rusty-hostess-hotspot-provider.dll"
        ) `
        -Destination $providerPath
    $providerUnsignedSha256 = Get-RustyFleetSha256 -LiteralPath $providerPath
    $providerUnsignedSize = (Get-Item -LiteralPath $providerPath).Length
    $providerCanonical = Get-RustyFleetPeCanonicalPayload `
        -LiteralPath $providerPath `
        -ExpectedPayloadSize $providerUnsignedSize
    $providerSigned = Set-AuthenticodeSignature `
        -FilePath $providerPath `
        -Certificate $signingCertificate `
        -HashAlgorithm SHA256
    Assert-Publication `
        ($providerSigned.Status -eq
            [Management.Automation.SignatureStatus]::Valid) `
        "provider fixture could not be signed"
    $providerSha256 = Get-RustyFleetSha256 -LiteralPath $providerPath
    $observedProductVersion = (
        [Diagnostics.FileVersionInfo]::GetVersionInfo($providerPath).
            ProductVersion
    )
    Assert-Publication `
        ($observedProductVersion -ceq $providerProductVersion) `
        "provider fixture product version is not exact"

    $consoleRoot = Join-Path $testRoot "fleet-console"
    [IO.Directory]::CreateDirectory($consoleRoot) | Out-Null
    $consolePath = Join-Path $consoleRoot "RustyFleet.FleetConsole.exe"
    $hubPath = Join-Path $testRoot "fleet-hub-local.exe"
    $fleetctlPath = Join-Path $testRoot "fleetctl.exe"
    $fleetOnboardPath = Join-Path $testRoot "fleet-onboard.exe"
    foreach ($destination in @(
        $consolePath,
        $hubPath,
        $fleetctlPath,
        $fleetOnboardPath
    )) {
        Copy-Item -LiteralPath $providerPath -Destination $destination
    }

    $metadataRoot = Join-Path $testRoot "hostess-metadata"
    $licensePath = Join-Path $metadataRoot "LICENSE"
    $noticesPath = Join-Path $metadataRoot "THIRD-PARTY-NOTICES.txt"
    Write-TestUtf8 `
        -LiteralPath $licensePath `
        -Content "Synthetic publication test owner license.`n"
    Write-TestUtf8 `
        -LiteralPath $noticesPath `
        -Content "Synthetic publication test third-party notices.`n"
    $providerSignature = Get-AuthenticodeSignature -LiteralPath $providerPath
    $providerProvenance = [ordered]@{
        schema = "rusty.hostess.windows_hotspot.release_provenance.v1"
        product_id = "rusty-hostess-windows-hotspot-provider"
        provider_version = $providerVersion
        artifact = [ordered]@{
            name = "rusty-hostess-hotspot-provider.exe"
            sha256 = $providerSha256
            size_bytes = (Get-Item -LiteralPath $providerPath).Length
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
            availability_state = "verified_public"
            verified_at_utc = "2026-07-27T00:00:00Z"
            tree_clean = $true
        }
        build = [ordered]@{
            kind = "signed-release"
            framework = "net10.0-windows10.0.19041.0"
            runtime_identifier = "win-x64"
            source_date_epoch = 1785110400
            unsigned_artifact_sha256 = $providerUnsignedSha256
            unsigned_artifact_size_bytes = $providerUnsignedSize
            canonical_payload_sha256 = $providerCanonical.sha256
            canonical_payload_size_bytes = $providerCanonical.size_bytes
        }
        dependencies = @(
            [ordered]@{
                name = "Synthetic.Dependency"
                version = "1.0.0"
                license = "MIT"
                license_url = "https://example.invalid/licenses/mit"
                project_url = "https://example.invalid/dependency"
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
            state = "verified"
            status = "Valid"
            subject = $providerSignature.SignerCertificate.Subject
            thumbprint = $signerThumbprint.ToLowerInvariant()
            authorized_thumbprint = $signerThumbprint.ToLowerInvariant()
        }
        companion_documents = @(
            [ordered]@{
                name = "LICENSE"
                sha256 = Get-RustyFleetSha256 -LiteralPath $licensePath
                size_bytes = (Get-Item -LiteralPath $licensePath).Length
            },
            [ordered]@{
                name = "THIRD-PARTY-NOTICES.txt"
                sha256 = Get-RustyFleetSha256 -LiteralPath $noticesPath
                size_bytes = (Get-Item -LiteralPath $noticesPath).Length
            }
        )
        distribution = [ordered]@{
            eligibility = "signed_release"
            binary_authority = "rusty-hostess-github-releases"
        }
    }
    Write-TestUtf8 `
        -LiteralPath (
            Join-Path $metadataRoot (
                "rusty-hostess-hotspot-provider.provenance.json"
            )
        ) `
        -Content (ConvertTo-RustyFleetJson -InputObject $providerProvenance)
    $verifiedAtFixture = [DateTimeOffset]::MinValue
    Assert-Publication (
        $providerSignature.Status -eq
            [Management.Automation.SignatureStatus]::Valid -and
        $null -ne $providerSignature.SignerCertificate -and
        [DateTimeOffset]::TryParse(
            [string] $providerProvenance.source.verified_at_utc,
            [ref] $verifiedAtFixture
        )
    ) (
        "provider signing fixture lost trust before bundle validation: " +
        "status=$($providerSignature.Status); " +
        "certificate=$($null -ne $providerSignature.SignerCertificate)"
    )
    $parsedProviderProvenance = Get-Content -LiteralPath (
        Join-Path $metadataRoot (
            "rusty-hostess-hotspot-provider.provenance.json"
        )
    ) -Raw | ConvertFrom-Json -Depth 30
    $recheckedProviderSignature = Get-AuthenticodeSignature `
        -LiteralPath $providerPath
    Assert-Publication (
        $parsedProviderProvenance.source.availability_state -ceq
            "verified_public" -and
        -not [string]::IsNullOrWhiteSpace(
            [string] $parsedProviderProvenance.source.verified_at_utc
        ) -and
        $parsedProviderProvenance.source.verified_at_utc -is [DateTime] -and
        $recheckedProviderSignature.Status -eq
            [Management.Automation.SignatureStatus]::Valid -and
        $null -ne $recheckedProviderSignature.SignerCertificate
    ) (
        "parsed provider fixture lost release trust: availability=" +
        $parsedProviderProvenance.source.availability_state +
        "; verified_at=" +
        [string] $parsedProviderProvenance.source.verified_at_utc +
        "; status=$($recheckedProviderSignature.Status); " +
        "certificate=" +
        ($null -ne $recheckedProviderSignature.SignerCertificate)
    )

    $distributionRoot = Join-Path $testRoot "distribution"
    & (Join-Path $packagingRoot "New-WindowsBundle.ps1") `
        -Version "1.2.3" `
        -Channel preview `
        -BuildKind signed-release `
        -HostessProviderPath $providerPath `
        -HostessProviderSha256 $providerSha256 `
        -HostessProviderMetadataDirectory $metadataRoot `
        -OutputDirectory $distributionRoot `
        -SourceRevision $sourceRevision `
        -SourceTree $sourceTree `
        -SourceDateEpoch 1785110400 `
        -SkipBuild `
        -ConsoleArtifactDirectory $consoleRoot `
        -HubArtifactPath $hubPath `
        -FleetctlArtifactPath $fleetctlPath `
        -FleetOnboardArtifactPath $fleetOnboardPath `
        -RequireCleanSource `
        -RequireAuthenticodeSignatures `
        -ExpectedFleetSignerThumbprint $signerThumbprint `
        -ExpectedHostessSignerThumbprint $signerThumbprint `
        -RepoRoot $sourceRepo `
        -ReleasePolicyPath $policyPath |
        Out-Null

    $setupReceiptJson = & (
        Join-Path $packagingRoot "New-WindowsSetup.ps1"
    ) `
        -Version "1.2.3" `
        -Channel preview `
        -BuildKind signed-release `
        -BundleArchivePath (
            Join-Path $distributionRoot "RustyFleet-1.2.3-win-x64.zip"
        ) `
        -OutputDirectory $distributionRoot `
        -FleetSignerThumbprint $signerThumbprint `
        -HostessSignerThumbprint $signerThumbprint
    $setupReceipt = $setupReceiptJson | ConvertFrom-Json -Depth 10
    $setupReceiptPath = Join-Path $distributionRoot (
        "RustyFleet-Setup.build-receipt.json"
    )
    Write-TestUtf8 `
        -LiteralPath $setupReceiptPath `
        -Content ($setupReceipt | ConvertTo-Json -Depth 10)
    $setupPath = Join-Path $distributionRoot "RustyFleet-Setup.exe"
    $signedSetup = Set-AuthenticodeSignature `
        -FilePath $setupPath `
        -Certificate $signingCertificate `
        -HashAlgorithm SHA256
    Assert-Publication `
        ($signedSetup.Status -eq
            [Management.Automation.SignatureStatus]::Valid) `
        "Setup fixture could not be signed"

    $descriptorKeyPath = Join-Path $testRoot "descriptor-key.pem"
    Write-TestUtf8 `
        -LiteralPath $descriptorKeyPath `
        -Content $descriptorRsa.ExportPkcs8PrivateKeyPem()
    $descriptorRoot = Join-Path $testRoot "descriptor"
    & (Join-Path $packagingRoot "New-WindowsReleaseDescriptor.ps1") `
        -Version "1.2.3" `
        -Channel preview `
        -SetupPath $setupPath `
        -SetupBuildReceiptPath $setupReceiptPath `
        -ExpectedSetupSignerThumbprint $signerThumbprint `
        -ExpectedSourceRevision $sourceRevision `
        -ExpectedSourceTree $sourceTree `
        -DescriptorPrivateKeyPemPath $descriptorKeyPath `
        -ExpectedDescriptorSignerSpkiSha256 $descriptorSpkiSha256 `
        -OutputDirectory $descriptorRoot `
        -DescriptorId "v1.2.3-preview-publication-test" `
        -IssuedAtUtc (
            [DateTimeOffset]::Parse("2026-07-27T12:00:00Z")
        ) `
        -LifetimeMinutes 1380 |
        Out-Null

    $stage = Join-Path $testRoot "publication-input"
    [IO.Directory]::CreateDirectory($stage) | Out-Null
    $bundleName = "RustyFleet-1.2.3-win-x64"
    foreach ($source in @(
        $setupPath,
        (Join-Path $distributionRoot "$bundleName.zip"),
        (Join-Path $distributionRoot "$bundleName.zip.sha256"),
        (Join-Path $distributionRoot "$bundleName.manifest.json"),
        (Join-Path $distributionRoot "$bundleName.checksums.sha256"),
        (Join-Path $distributionRoot "$bundleName.validation-receipt.json"),
        $setupReceiptPath,
        (Join-Path $descriptorRoot "release.json"),
        (Join-Path $descriptorRoot "release-descriptor.receipt.json"),
        (Join-Path $descriptorRoot "release-descriptor.spki.der"),
        $policyPath
    )) {
        Copy-Item -LiteralPath $source -Destination $stage
    }

    $fakeGhMarker = Join-Path $testRoot "fake-gh-invoked.txt"
    $fakeGh = Join-Path $testRoot "fake-gh.ps1"
    Write-TestUtf8 -LiteralPath $fakeGh -Content @"
param([Parameter(ValueFromRemainingArguments = `$true)][string[]] `$GhArgs)
[IO.File]::AppendAllText(
    '$fakeGhMarker',
    'invoked',
    [Text.UTF8Encoding]::new(`$false)
)
exit 99
"@
    [Environment]::SetEnvironmentVariable(
        "GH_TOKEN",
        "test-token-must-not-be-used-during-preflight",
        [EnvironmentVariableTarget]::Process
    )
    $preflight = Invoke-PublicationAuthority `
        -Mode Preflight `
        -InputRoot $stage `
        -SourceRepository $sourceRepo `
        -SourceRevision $sourceRevision `
        -SourceTree $sourceTree `
        -FleetSigner $signerThumbprint `
        -HostessSigner $signerThumbprint `
        -DescriptorSpki $descriptorSpkiSha256 `
        -GhExecutable $fakeGh |
        ConvertFrom-Json -Depth 20
    Assert-Publication (
        $preflight.result -eq "pass" -and
        $preflight.mode -eq "preflight" -and
        $preflight.asset_count -eq 10 -and
        @($preflight.assets).Count -eq 10 -and
        @($preflight.assets.name | Sort-Object -Unique).Count -eq 10 -and
        $preflight.token_used -eq $false -and
        $preflight.gh_invoked -eq $false -and
        -not (Test-Path -LiteralPath $fakeGhMarker)
    ) "valid publication preflight was not exact and token-free"

    $expectedInputNames = @(
        "RustyFleet-Setup.exe",
        "$bundleName.zip",
        "$bundleName.zip.sha256",
        "$bundleName.manifest.json",
        "$bundleName.checksums.sha256",
        "$bundleName.validation-receipt.json",
        "RustyFleet-Setup.build-receipt.json",
        "release.json",
        "release-descriptor.receipt.json",
        "release-descriptor.spki.der",
        "release-policy.json"
    ) | Sort-Object
    $retained = [RustyFleet.Publication.RetainedAssetSet]::Open(
        $stage,
        $expectedInputNames
    )
    try {
        $writeDenied = $false
        try {
            $writer = [IO.File]::Open(
                (Join-Path $stage "release.json"),
                [IO.FileMode]::Open,
                [IO.FileAccess]::Write,
                [IO.FileShare]::ReadWrite
            )
            $writer.Dispose()
        }
        catch {
            $writeDenied = $true
        }
        $leafRenameDenied = $false
        try {
            Move-Item `
                -LiteralPath (Join-Path $stage "release.json") `
                -Destination (Join-Path $stage "replacement.json") `
                -ErrorAction Stop
        }
        catch {
            $leafRenameDenied = $true
        }
        $retained.Verify()
        Assert-Publication (
            $writeDenied -and $leafRenameDenied
        ) "retained publication asset allowed write or rename substitution"
    }
    finally {
        $retained.Dispose()
    }

    if ($BaselineOnly) {
        [ordered]@{
            schema = "rusty.fleet.windows_publication_test.v1"
            result = "pass"
            baseline_only = $true
        } | ConvertTo-Json -Depth 5
        return
    }

    $mutations = [ordered]@{
        release_json = "release.json"
        zip = "$bundleName.zip"
        manifest = "$bundleName.manifest.json"
        checksums = "$bundleName.checksums.sha256"
        setup_build_receipt = "RustyFleet-Setup.build-receipt.json"
        validation_receipt = "$bundleName.validation-receipt.json"
    }
    $caseRoot = Join-Path $testRoot "mutated-input"
    Copy-Item -LiteralPath $stage -Destination $caseRoot -Recurse
    foreach ($case in $mutations.GetEnumerator()) {
        $casePath = Join-Path $caseRoot $case.Value
        [IO.File]::AppendAllText(
            $casePath,
            "substitution",
            [Text.UTF8Encoding]::new($false)
        )
        if (Test-Path -LiteralPath $fakeGhMarker) {
            Remove-Item -LiteralPath $fakeGhMarker -Force
        }
        $rejected = $false
        try {
            Invoke-PublicationAuthority `
                -Mode Publish `
                -InputRoot $caseRoot `
                -SourceRepository $sourceRepo `
                -SourceRevision $sourceRevision `
                -SourceTree $sourceTree `
                -FleetSigner $signerThumbprint `
                -HostessSigner $signerThumbprint `
                -DescriptorSpki $descriptorSpkiSha256 `
                -GhExecutable $fakeGh |
                Out-Null
        }
        catch {
            $rejected = $true
        }
        Assert-Publication (
            $rejected -and
            -not (Test-Path -LiteralPath $fakeGhMarker)
        ) "$($case.Key) substitution reached the release token or gh"
        Copy-Item `
            -LiteralPath (Join-Path $stage $case.Value) `
            -Destination $casePath `
            -Force
    }

    Write-TestUtf8 `
        -LiteralPath (Join-Path $caseRoot "unexpected.bin") `
        -Content "unexpected publication asset"
    foreach ($inventoryMutation in @("added", "removed")) {
        if ($inventoryMutation -eq "removed") {
            Remove-Item `
                -LiteralPath (Join-Path $caseRoot "unexpected.bin") `
                -Force
            Remove-Item `
                -LiteralPath (
                    Join-Path $caseRoot "release-descriptor.spki.der"
                ) `
                -Force
        }
        if (Test-Path -LiteralPath $fakeGhMarker) {
            Remove-Item -LiteralPath $fakeGhMarker -Force
        }
        $inventoryRejected = $false
        try {
            Invoke-PublicationAuthority `
                -Mode Publish `
                -InputRoot $caseRoot `
                -SourceRepository $sourceRepo `
                -SourceRevision $sourceRevision `
                -SourceTree $sourceTree `
                -FleetSigner $signerThumbprint `
                -HostessSigner $signerThumbprint `
                -DescriptorSpki $descriptorSpkiSha256 `
                -GhExecutable $fakeGh |
                Out-Null
        }
        catch {
            $inventoryRejected = $true
        }
        Assert-Publication (
            $inventoryRejected -and
            -not (Test-Path -LiteralPath $fakeGhMarker)
        ) "publication inventory addition or omission reached gh"
    }

    [ordered]@{
        schema = "rusty.fleet.windows_publication_test.v1"
        result = "pass"
        signed_setup_and_canonical_receipt = $true
        exact_zip_sidecar_and_full_bundle_validation = $true
        top_level_metadata_byte_equal = $true
        rsa_pss_jcs_descriptor_and_spki_verified = $true
        exact_filename_sha256_inventory = $true
        retained_write_and_rename_denied = $true
        release_json_substitution_rejected_before_gh = $true
        zip_substitution_rejected_before_gh = $true
        manifest_substitution_rejected_before_gh = $true
        checksums_substitution_rejected_before_gh = $true
        build_receipt_substitution_rejected_before_gh = $true
        validation_receipt_substitution_rejected_before_gh = $true
        asset_addition_and_omission_rejected_before_gh = $true
        preflight_token_free = $true
    } | ConvertTo-Json -Depth 10
}
finally {
    [Environment]::SetEnvironmentVariable(
        "GH_TOKEN",
        $priorToken,
        [EnvironmentVariableTarget]::Process
    )
    foreach ($storeName in $certificateStoreNames) {
        $store = [Security.Cryptography.X509Certificates.X509Store]::new(
            $storeName,
            [Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser
        )
        try {
            $store.Open(
                [Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite
            )
            if ($signingCertificate) {
                $store.Remove($signingCertificate)
            }
        }
        finally {
            $store.Dispose()
        }
    }
    if ($signingCertificate) {
        $myStore = [Security.Cryptography.X509Certificates.X509Store]::new(
            "My",
            [Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser
        )
        try {
            $myStore.Open(
                [Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite
            )
            $myStore.Remove($signingCertificate)
        }
        finally {
            $myStore.Dispose()
            $signingCertificate.Dispose()
        }
    }
    $descriptorRsa.Dispose()
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
