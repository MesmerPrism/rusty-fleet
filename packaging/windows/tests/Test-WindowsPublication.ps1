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
        -Channel labs `
        -Maturity alpha `
        -ReleaseTag "v1.2.3-alpha.1" `
        -ExpectedFleetSignerThumbprint $FleetSigner `
        -ExpectedHostessSignerThumbprint $HostessSigner `
        -ExpectedDescriptorSignerSpkiSha256 $DescriptorSpki `
        -ExpectedSourceRevision $SourceRevision `
        -ExpectedSourceTree $SourceTree `
        -RepositoryRoot $SourceRepository `
        -ExpectedRef "refs/tags/v1.2.3-alpha.1" `
        -GhExecutable $GhExecutable
}

$packagingRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$repositoryRoot = (Resolve-Path (Join-Path $packagingRoot "..\..")).Path
Import-Module (Join-Path $packagingRoot "Distribution.Common.psm1") -Force
Import-Module (
    Join-Path $PSScriptRoot "WindowsCertificateFixture.psm1"
) -Force
foreach ($case in @(
    [pscustomobject]@{ channel = "stable"; maturity = "released"; tag = "v9.9.9" },
    [pscustomobject]@{ channel = "labs"; maturity = "alpha"; tag = "v9.9.9-alpha.1" }
)) {
    $message = ""
    try {
        & (Join-Path $packagingRoot "Publish-WindowsRelease.ps1") `
            -Mode Preflight `
            -AssetDirectory "missing-assets" `
            -Version "1.2.3" `
            -Channel $case.channel `
            -Maturity $case.maturity `
            -ReleaseTag $case.tag `
            -ExpectedFleetSignerThumbprint ("A" * 40) `
            -ExpectedHostessSignerThumbprint ("B" * 40) `
            -ExpectedDescriptorSignerSpkiSha256 ("3" * 64) `
            -ExpectedSourceRevision ("1" * 40) `
            -ExpectedSourceTree ("2" * 40) `
            -RepositoryRoot "missing-repository" | Out-Null
    }
    catch {
        $message = $_.Exception.Message
    }
    Assert-Publication `
        ($message -ceq "release tag does not bind the exact version and channel") `
        "$($case.channel) publication accepted a cross-version release tag"
}
function global:Get-AuthenticodeSignature {
    param(
        [Parameter(Mandatory)]
        [string] $LiteralPath
    )

    Get-RustyFleetTestAuthenticodeSignature -LiteralPath $LiteralPath
}
$distributionModule = Get-Module Distribution.Common
$testRoot = Join-Path ([IO.Path]::GetTempPath()) (
    "rusty-fleet-publication-test-$([Guid]::NewGuid().ToString('N'))"
)
[IO.Directory]::CreateDirectory($testRoot) | Out-Null
$descriptorRsa = [Security.Cryptography.RSA]::Create(3072)
$signingCertificate = $null
$testNow = [DateTimeOffset]::UtcNow
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

    $signingCertificate = New-RustyFleetTestCodeSigningCertificate `
        -Subject "CN=MesmerPrism"
    $signerThumbprint = $signingCertificate.Thumbprint.ToUpperInvariant()
    $signerCertificateSha256 = [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData(
            $signingCertificate.RawData
        )
    ).ToLowerInvariant()
    $descriptorSpki = $descriptorRsa.ExportSubjectPublicKeyInfo()
    $descriptorSpkiSha256 = [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData($descriptorSpki)
    ).ToLowerInvariant()

    $sourceRepo = Join-Path $testRoot "source"
    $policyPath = Join-Path $sourceRepo (
        "packaging\windows\trust\release-policy.json"
    )
    $policy = [ordered]@{
        schema = "rusty.fleet.windows_release_trust_policy.v2"
        channels = [ordered]@{
            labs = [ordered]@{
                publication_enabled = $true
                authenticode = [ordered]@{
                    subject = $signingCertificate.Subject
                    thumbprint = $signerThumbprint
                    certificate_sha256 = $signerCertificateSha256
                    self_issued = $true
                    public_trust_claim = $false
                    trust_mode = (
                        "exact-pinned-self-issued-untrusted-root-only"
                    )
                    timestamp_required = $true
                    allowed_chain_status_flags = @("UntrustedRoot")
                }
                authorized_descriptor_signer_spki_sha256 = @(
                    $descriptorSpkiSha256
                )
                status = (
                    "labs_exact_pinned_self_issued_signer_configured"
                )
            }
            stable = [ordered]@{
                publication_enabled = $false
                authenticode = [ordered]@{
                    subject = $null
                    thumbprint = $null
                    certificate_sha256 = $null
                    self_issued = $false
                    public_trust_claim = $true
                    trust_mode = "public-chain-only"
                    timestamp_required = $true
                    allowed_chain_status_flags = @()
                }
                authorized_descriptor_signer_spki_sha256 = @()
                status = "stable_public_chain_signer_not_configured"
            }
        }
    }
    Write-TestUtf8 `
        -LiteralPath $policyPath `
        -Content (
            [regex]::Replace(
                (ConvertTo-RustyFleetJson -InputObject $policy),
                "\r?\n",
                "`n"
            )
        )
    Copy-Item `
        -LiteralPath (Join-Path $repositoryRoot ".gitattributes") `
        -Destination (Join-Path $sourceRepo ".gitattributes")
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
    $policyEol = Invoke-TestGit -Repository $sourceRepo -Arguments @(
        "check-attr",
        "eol",
        "--",
        "packaging/windows/trust/release-policy.json"
    )
    Assert-Publication (
        $policyEol -ceq
            "packaging/windows/trust/release-policy.json: eol: lf"
    ) "production attributes do not force release-policy JSON to LF"
    Invoke-TestGit -Repository $sourceRepo -Arguments @("add", ".") | Out-Null
    Invoke-TestGit -Repository $sourceRepo -Arguments @(
        "commit", "-m", "test release authority"
    ) | Out-Null
    Invoke-TestGit -Repository $sourceRepo -Arguments @(
        "tag", "v1.2.3-alpha.1"
    ) | Out-Null
    $sourceRevision = Invoke-TestGit `
        -Repository $sourceRepo `
        -Arguments @("rev-parse", "HEAD")
    $sourceTree = Invoke-TestGit `
        -Repository $sourceRepo `
        -Arguments @("rev-parse", "HEAD^{tree}")
    $taggedPolicyBlob = Invoke-TestGit `
        -Repository $sourceRepo `
        -Arguments @(
            "rev-parse",
            "HEAD:packaging/windows/trust/release-policy.json"
        )
    $checkedOutPolicyBlob = Invoke-TestGit `
        -Repository $sourceRepo `
        -Arguments @("hash-object", "--no-filters", $policyPath)
    Assert-Publication (
        $checkedOutPolicyBlob -ceq $taggedPolicyBlob
    ) "Windows checkout changed the byte-exact tagged release policy"
    $sourceDirt = Invoke-TestGit `
        -Repository $sourceRepo `
        -Arguments @("status", "--porcelain=v1", "--untracked-files=no")
    Assert-Publication (
        -not $sourceDirt
    ) "publication source fixture is dirty after policy materialization: $sourceDirt"
    $windowsCheckout = Join-Path $testRoot "windows-checkout"
    & git init --initial-branch=main $windowsCheckout | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "could not initialize Windows checkout fixture"
    }
    Invoke-TestGit -Repository $windowsCheckout -Arguments @(
        "config", "core.autocrlf", "true"
    ) | Out-Null
    Invoke-TestGit -Repository $windowsCheckout -Arguments @(
        "remote", "add", "origin", $sourceRepo
    ) | Out-Null
    Invoke-TestGit -Repository $windowsCheckout -Arguments @(
        "fetch", "--no-tags", "origin", $sourceRevision
    ) | Out-Null
    Invoke-TestGit -Repository $windowsCheckout -Arguments @(
        "checkout", "--detach", "FETCH_HEAD"
    ) | Out-Null
    $windowsPolicyPath = Join-Path $windowsCheckout (
        "packaging\windows\trust\release-policy.json"
    )
    $windowsPolicyBlob = Invoke-TestGit `
        -Repository $windowsCheckout `
        -Arguments @("hash-object", "--no-filters", $windowsPolicyPath)
    $windowsDirt = Invoke-TestGit `
        -Repository $windowsCheckout `
        -Arguments @("status", "--porcelain=v1", "--untracked-files=no")
    Assert-Publication (
        $windowsPolicyBlob -ceq $taggedPolicyBlob -and -not $windowsDirt
    ) "fresh Windows checkout changed or dirtied the tagged release policy"

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
    Register-RustyFleetTestAuthenticodeSignature `
        -LiteralPath $providerPath `
        -Certificate $signingCertificate
    $providerSigned = Get-AuthenticodeSignature -LiteralPath $providerPath
    Assert-Publication `
        ($providerSigned.Status -eq
            [Management.Automation.SignatureStatus]::UnknownError) `
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
    $providerPolicyPath = Join-Path $metadataRoot (
        "rusty-hostess-hotspot-provider.release-policy.json"
    )
    $providerPolicy = [ordered]@{
        schema = "rusty.hostess.windows_hotspot.release_policy.v1"
        product_id = "rusty-hostess-windows-hotspot-provider"
        signer = [ordered]@{
            subject = $signingCertificate.Subject
            issuer = $signingCertificate.Issuer
            thumbprint_sha1 = $signerThumbprint
            certificate_sha256 = $signerCertificateSha256
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
    Write-TestUtf8 `
        -LiteralPath $providerPolicyPath `
        -Content (ConvertTo-RustyFleetJson -InputObject $providerPolicy)
    $providerProvenance = [ordered]@{
        schema = "rusty.hostess.windows_hotspot.release_provenance.v2"
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
            state = "accepted_exact_owner_signature"
            authenticode_status = "valid"
            subject = $providerSignature.SignerCertificate.Subject
            issuer = $providerSignature.SignerCertificate.Issuer
            thumbprint_sha1 = $signerThumbprint.ToLowerInvariant()
            certificate_sha256 = $signerCertificateSha256
            code_signing_eku_present = $true
            self_issued = $true
            timestamp_present = $true
            chain_trusted = $true
            chain_element_count = 1
            chain_status_flags = @()
            public_trust_claim = $false
            trust_boundary = (
                "host-chain-valid-no-public-trust-claim"
            )
        }
        release_policy = [ordered]@{
            asset_name = (
                "rusty-hostess-hotspot-provider.release-policy.json"
            )
            schema = $providerPolicy.schema
            sha256 = Get-RustyFleetSha256 `
                -LiteralPath $providerPolicyPath
            size_bytes = (Get-Item -LiteralPath $providerPolicyPath).Length
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
            eligibility = "labs_signed_release"
            binary_authority = "rusty-hostess-github-releases"
            allowed_channels = @("labs")
            stable_eligible = $false
        }
    }
    Write-TestUtf8 `
        -LiteralPath (
            Join-Path $metadataRoot (
                "rusty-hostess-hotspot-provider.provenance.json"
            )
        ) `
        -Content (ConvertTo-RustyFleetJson -InputObject $providerProvenance)
    foreach ($boundaryCase in @(
        [pscustomobject]@{
            name = "recorded-valid-mislabeled-untrusted"
            apply = {
                param($value)
                $value.signing.trust_boundary = (
                    "exact-pinned-self-issued-untrusted-root-only"
                )
            }
        },
        [pscustomobject]@{
            name = "recorded-untrusted-mislabeled-valid"
            apply = {
                param($value)
                $value.signing.authenticode_status = "unknown_error"
                $value.signing.chain_trusted = $false
                $value.signing.chain_status_flags = @("UntrustedRoot")
                $value.signing.trust_boundary = (
                    "host-chain-valid-no-public-trust-claim"
                )
            }
        }
    )) {
        $boundaryMetadata = Join-Path $testRoot $boundaryCase.name
        Copy-Item `
            -LiteralPath $metadataRoot `
            -Destination $boundaryMetadata `
            -Recurse
        $boundaryProvenancePath = Join-Path $boundaryMetadata (
            "rusty-hostess-hotspot-provider.provenance.json"
        )
        $boundaryProvenance = Get-Content `
            -LiteralPath $boundaryProvenancePath `
            -Raw | ConvertFrom-Json -Depth 30
        & $boundaryCase.apply $boundaryProvenance
        Write-TestUtf8 `
            -LiteralPath $boundaryProvenancePath `
            -Content (
                ConvertTo-RustyFleetJson -InputObject $boundaryProvenance
            )
        $boundaryMessage = ""
        try {
            Read-RustyFleetHostessProvenance `
                -MetadataDirectory $boundaryMetadata `
                -ProviderPath $providerPath `
                -ProviderSha256 $providerSha256 `
                -BuildKind signed-release `
                -Channel labs `
                -AuthenticodePolicy $policy.channels.labs.authenticode |
                    Out-Null
        }
        catch {
            $boundaryMessage = $_.Exception.Message
        }
        Assert-Publication (
            $boundaryMessage -ceq
                "Hostess provenance does not authorize signed release distribution"
        ) "$($boundaryCase.name) was not rejected at the recorded trust boundary"
    }
    $verifiedAtFixture = [DateTimeOffset]::MinValue
    Assert-Publication (
        $providerSignature.Status -eq
            [Management.Automation.SignatureStatus]::UnknownError -and
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
            [Management.Automation.SignatureStatus]::UnknownError -and
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
        -Channel labs `
        -Maturity alpha `
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
        -Channel labs `
        -Maturity alpha `
        -BuildKind signed-release `
        -BundleArchivePath (
            Join-Path $distributionRoot (
                "RustyFleet-Labs-1.2.3-win-x64.zip"
            )
        ) `
        -OutputDirectory $distributionRoot `
        -FleetSignerThumbprint $signerThumbprint `
        -HostessSignerThumbprint $signerThumbprint `
        -ReleasePolicyPath $policyPath
    $setupReceipt = $setupReceiptJson | ConvertFrom-Json -Depth 10
    $setupReceiptPath = Join-Path $distributionRoot (
        "RustyFleet-Labs-Setup.build-receipt.json"
    )
    Write-TestUtf8 `
        -LiteralPath $setupReceiptPath `
        -Content ($setupReceipt | ConvertTo-Json -Depth 10)
    $setupPath = Join-Path $distributionRoot "RustyFleet-Labs-Setup.exe"
    $signedSetup = Set-AuthenticodeSignature `
        -FilePath $setupPath `
        -Certificate $signingCertificate `
        -HashAlgorithm SHA256
    Register-RustyFleetTestAuthenticodeSignature `
        -LiteralPath $setupPath `
        -Certificate $signingCertificate
    $signedSetup = Get-AuthenticodeSignature -LiteralPath $setupPath
    Assert-Publication `
        ($signedSetup.Status -eq
            [Management.Automation.SignatureStatus]::UnknownError) `
        "Setup fixture could not be signed"

    $descriptorKeyPath = Join-Path $testRoot "descriptor-key.pem"
    Write-TestUtf8 `
        -LiteralPath $descriptorKeyPath `
        -Content $descriptorRsa.ExportPkcs8PrivateKeyPem()
    $descriptorRoot = Join-Path $testRoot "descriptor"
    & (Join-Path $packagingRoot "New-WindowsReleaseDescriptor.ps1") `
        -Version "1.2.3" `
        -Channel labs `
        -Maturity alpha `
        -ReleaseTag "v1.2.3-alpha.1" `
        -SetupPath $setupPath `
        -SetupBuildReceiptPath $setupReceiptPath `
        -ExpectedSetupSignerThumbprint $signerThumbprint `
        -ReleasePolicyPath $policyPath `
        -ExpectedSourceRevision $sourceRevision `
        -ExpectedSourceTree $sourceTree `
        -DescriptorPrivateKeyPemPath $descriptorKeyPath `
        -ExpectedDescriptorSignerSpkiSha256 $descriptorSpkiSha256 `
        -OutputDirectory $descriptorRoot `
        -DescriptorId "v1.2.3-alpha.1-labs-publication-test" `
        -IssuedAtUtc $testNow.AddMinutes(-5) `
        -LifetimeMinutes 1380 |
        Out-Null

    $stage = Join-Path $testRoot "publication-input"
    [IO.Directory]::CreateDirectory($stage) | Out-Null
    $bundleName = "RustyFleet-Labs-1.2.3-win-x64"
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
    $ownerReleaseMetadata = Get-Content -LiteralPath (
        Join-Path $stage "release-descriptor.receipt.json"
    ) -Raw | ConvertFrom-Json -Depth 20
    Assert-Publication (
        $preflight.result -eq "pass" -and
        $preflight.mode -eq "preflight" -and
        $preflight.asset_count -eq 10 -and
        @($preflight.assets).Count -eq 10 -and
        @($preflight.assets.name | Sort-Object -Unique).Count -eq 10 -and
        $ownerReleaseMetadata.schema -ceq
            "rusty.fleet.windows_release_descriptor_receipt.v5" -and
        $ownerReleaseMetadata.release_tag -ceq "v1.2.3-alpha.1" -and
        $ownerReleaseMetadata.installation_identity -ceq
            "rusty-fleet-labs" -and
        $ownerReleaseMetadata.primary_artifact.role -ceq
            "complete-product" -and
        $ownerReleaseMetadata.primary_artifact.name -ceq
            "RustyFleet-Labs-Setup.exe" -and
        $ownerReleaseMetadata.primary_artifact.sha256 -ceq
            $preflight.setup_sha256 -and
        [long] $ownerReleaseMetadata.primary_artifact.bytes -eq
            (Get-Item -LiteralPath $setupPath).Length -and
        $preflight.token_used -eq $false -and
        $preflight.gh_invoked -eq $false -and
        -not (Test-Path -LiteralPath $fakeGhMarker)
    ) "valid publication preflight was not exact and token-free"

    $publishBoundaryError = $null
    try {
        Invoke-PublicationAuthority `
            -Mode Publish `
            -InputRoot $stage `
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
        $publishBoundaryError = $_.Exception.Message
    }
    Assert-Publication (
        $publishBoundaryError -ceq
            "remote release tag lookup failed closed" -and
        (Test-Path -LiteralPath $fakeGhMarker)
    ) "valid publication did not cross the local-to-remote authority boundary"

    $crlfPolicyStage = Join-Path $testRoot "crlf-policy-input"
    Copy-Item -LiteralPath $stage -Destination $crlfPolicyStage -Recurse
    $crlfPolicyPath = Join-Path $crlfPolicyStage "release-policy.json"
    $crlfPolicy = [regex]::Replace(
        [IO.File]::ReadAllText($crlfPolicyPath),
        "\r?\n",
        "`r`n"
    )
    [IO.File]::WriteAllText(
        $crlfPolicyPath,
        $crlfPolicy,
        [Text.UTF8Encoding]::new($false)
    )
    if (Test-Path -LiteralPath $fakeGhMarker) {
        Remove-Item -LiteralPath $fakeGhMarker -Force
    }
    $crlfPolicyError = $null
    try {
        Invoke-PublicationAuthority `
            -Mode Publish `
            -InputRoot $crlfPolicyStage `
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
        $crlfPolicyError = $_.Exception.Message
    }
    Assert-Publication (
        $crlfPolicyError -ceq
            "staged release policy is not byte-exact to the tagged policy" -and
        -not (Test-Path -LiteralPath $fakeGhMarker)
    ) "CRLF release-policy substitution reached publication authority"

    $preflightPath = Join-Path $testRoot "publication-preflight.json"
    Write-TestUtf8 `
        -LiteralPath $preflightPath `
        -Content (ConvertTo-RustyFleetJson -InputObject $preflight)
    $siteRoot = Join-Path $testRoot "site"
    Write-TestUtf8 `
        -LiteralPath (Join-Path $siteRoot "index.html") `
        -Content "<!doctype html><title>Rusty Fleet test</title>"
    Write-TestUtf8 `
        -LiteralPath (Join-Path $siteRoot "styles.css") `
        -Content "body { color: black; }"
    $pagesOutput = Join-Path $testRoot "pages-output"
    $firstHandoff = & (
        Join-Path $packagingRoot "New-WindowsPagesDeployment.ps1"
    ) `
        -Version "1.2.3" `
        -Channel labs `
        -Maturity alpha `
        -ReleaseTag "v1.2.3-alpha.1" `
        -SiteDirectory $siteRoot `
        -MetadataDirectory $descriptorRoot `
        -PublicationPreflightReceiptPath $preflightPath `
        -ExpectedSourceRevision $sourceRevision `
        -ExpectedSourceTree $sourceTree `
        -ExpectedDescriptorSignerSpkiSha256 $descriptorSpkiSha256 `
        -OutputDirectory $pagesOutput `
        -NowUtc $testNow |
        ConvertFrom-Json -Depth 20
    $firstHandoffPath = Join-Path $pagesOutput (
        "Rusty-Fleet\metadata\labs\deployment-handoff.json"
    )
    Assert-Publication (
        $firstHandoff.schema -eq
            "rusty.fleet.windows_release_metadata_handoff.v2" -and
        $firstHandoff.product_channel -eq "labs" -and
        $firstHandoff.maturity -eq "alpha" -and
        $firstHandoff.channel -eq "labs" -and
        $firstHandoff.distribution_track -eq "github-prerelease" -and
        $firstHandoff.result -eq "pass" -and
        $firstHandoff.deployment_sequence -eq 1 -and
        $firstHandoff.pages_binary_count -eq 0 -and
        @($firstHandoff.release_files).Count -eq 5 -and
        (Test-Path -LiteralPath $firstHandoffPath -PathType Leaf) -and
        -not (Get-ChildItem -LiteralPath $pagesOutput -Recurse -File |
            Where-Object {
                $_.Name -ceq "RustyFleet-Labs-Setup.exe" -or
                $_.Extension -cin @(".zip", ".msi", ".dll")
            })
    ) "first Pages metadata deployment handoff is not exact or binary-free"

    $resumedHandoff = & (
        Join-Path $packagingRoot "New-WindowsPagesDeployment.ps1"
    ) `
        -Version "1.2.3" `
        -Channel labs `
        -Maturity alpha `
        -ReleaseTag "v1.2.3-alpha.1" `
        -SiteDirectory $siteRoot `
        -MetadataDirectory $descriptorRoot `
        -PublicationPreflightReceiptPath $preflightPath `
        -ExpectedSourceRevision $sourceRevision `
        -ExpectedSourceTree $sourceTree `
        -ExpectedDescriptorSignerSpkiSha256 $descriptorSpkiSha256 `
        -OutputDirectory $pagesOutput `
        -NowUtc $testNow `
        -Resume |
        ConvertFrom-Json -Depth 20
    Assert-Publication (
        $resumedHandoff.deployment_id -ceq $firstHandoff.deployment_id -and
        $resumedHandoff.deployment_sequence -eq 1
    ) "completed Pages deployment did not resume idempotently"

    $interruptedOutput = Join-Path $testRoot "pages-interrupted"
    $interruptedStage = (
        "$interruptedOutput.staging-$($firstHandoff.deployment_id)"
    )
    Write-TestUtf8 `
        -LiteralPath (Join-Path $interruptedStage "partial.txt") `
        -Content "interrupted"
    $interruptedResume = & (
        Join-Path $packagingRoot "New-WindowsPagesDeployment.ps1"
    ) `
        -Version "1.2.3" `
        -Channel labs `
        -Maturity alpha `
        -ReleaseTag "v1.2.3-alpha.1" `
        -SiteDirectory $siteRoot `
        -MetadataDirectory $descriptorRoot `
        -PublicationPreflightReceiptPath $preflightPath `
        -ExpectedSourceRevision $sourceRevision `
        -ExpectedSourceTree $sourceTree `
        -ExpectedDescriptorSignerSpkiSha256 $descriptorSpkiSha256 `
        -OutputDirectory $interruptedOutput `
        -NowUtc $testNow `
        -Resume |
        ConvertFrom-Json -Depth 20
    Assert-Publication (
        $interruptedResume.deployment_id -ceq $firstHandoff.deployment_id -and
        -not (Test-Path -LiteralPath $interruptedStage) -and
        (Test-Path -LiteralPath (
            Join-Path $interruptedOutput (
                "Rusty-Fleet\metadata\labs\release.json"
            )
        ))
    ) "interrupted Pages deployment did not rebuild and resume exactly"

    $badSite = Join-Path $testRoot "site-with-binary"
    Copy-Item -LiteralPath $siteRoot -Destination $badSite -Recurse
    Write-TestUtf8 `
        -LiteralPath (Join-Path $badSite "RustyFleet-Labs-Setup.exe") `
        -Content "binary must not enter Pages"
    $binaryRejected = $false
    try {
        & (Join-Path $packagingRoot "New-WindowsPagesDeployment.ps1") `
            -Version "1.2.3" `
            -Channel labs `
            -Maturity alpha `
            -ReleaseTag "v1.2.3-alpha.1" `
            -SiteDirectory $badSite `
            -MetadataDirectory $descriptorRoot `
            -PublicationPreflightReceiptPath $preflightPath `
            -ExpectedSourceRevision $sourceRevision `
            -ExpectedSourceTree $sourceTree `
            -ExpectedDescriptorSignerSpkiSha256 $descriptorSpkiSha256 `
            -OutputDirectory (Join-Path $testRoot "pages-binary") `
            -NowUtc $testNow |
            Out-Null
    }
    catch {
        $binaryRejected = $true
    }
    Assert-Publication $binaryRejected "Pages accepted a binary payload"

    foreach ($wrongBoundary in @(
        [pscustomobject]@{
            name = "source"
            version = "1.2.3"
            source_revision = "0" * 40
            descriptor_spki = $descriptorSpkiSha256
        },
        [pscustomobject]@{
            name = "tag"
            version = "1.2.4"
            source_revision = $sourceRevision
            descriptor_spki = $descriptorSpkiSha256
        },
        [pscustomobject]@{
            name = "signer"
            version = "1.2.3"
            source_revision = $sourceRevision
            descriptor_spki = "0" * 64
        }
    )) {
        $boundaryRejected = $false
        try {
            & (Join-Path $packagingRoot "New-WindowsPagesDeployment.ps1") `
                -Version $wrongBoundary.version `
                -Channel labs `
                -Maturity alpha `
                -ReleaseTag "v$($wrongBoundary.version)-alpha.1" `
                -SiteDirectory $siteRoot `
                -MetadataDirectory $descriptorRoot `
                -PublicationPreflightReceiptPath $preflightPath `
                -ExpectedSourceRevision $wrongBoundary.source_revision `
                -ExpectedSourceTree $sourceTree `
                -ExpectedDescriptorSignerSpkiSha256 (
                    $wrongBoundary.descriptor_spki
                ) `
                -OutputDirectory (
                    Join-Path $testRoot "pages-wrong-$($wrongBoundary.name)"
                ) `
                -NowUtc $testNow |
                Out-Null
        }
        catch {
            $boundaryRejected = $true
        }
        Assert-Publication `
            $boundaryRejected `
            "Pages accepted a wrong $($wrongBoundary.name) binding"
    }

    $damagedMetadata = Join-Path $testRoot "damaged-metadata"
    Copy-Item `
        -LiteralPath $descriptorRoot `
        -Destination $damagedMetadata `
        -Recurse
    [IO.File]::AppendAllText(
        (Join-Path $damagedMetadata "release.json"),
        "damage",
        [Text.UTF8Encoding]::new($false)
    )
    $assetRejected = $false
    try {
        & (Join-Path $packagingRoot "New-WindowsPagesDeployment.ps1") `
            -Version "1.2.3" `
            -Channel labs `
            -Maturity alpha `
            -ReleaseTag "v1.2.3-alpha.1" `
            -SiteDirectory $siteRoot `
            -MetadataDirectory $damagedMetadata `
            -PublicationPreflightReceiptPath $preflightPath `
            -ExpectedSourceRevision $sourceRevision `
            -ExpectedSourceTree $sourceTree `
            -ExpectedDescriptorSignerSpkiSha256 $descriptorSpkiSha256 `
            -OutputDirectory (Join-Path $testRoot "pages-damaged-asset") `
            -NowUtc $testNow |
            Out-Null
    }
    catch {
        $assetRejected = $true
    }
    Assert-Publication $assetRejected "Pages accepted damaged metadata"

    $staleMetadata = Join-Path $testRoot "stale-metadata"
    & (Join-Path $packagingRoot "New-WindowsReleaseDescriptor.ps1") `
        -Version "1.2.3" `
        -Channel labs `
        -Maturity alpha `
        -ReleaseTag "v1.2.3-alpha.1" `
        -SetupPath $setupPath `
        -SetupBuildReceiptPath $setupReceiptPath `
        -ExpectedSetupSignerThumbprint $signerThumbprint `
        -ReleasePolicyPath $policyPath `
        -ExpectedSourceRevision $sourceRevision `
        -ExpectedSourceTree $sourceTree `
        -DescriptorPrivateKeyPemPath $descriptorKeyPath `
        -ExpectedDescriptorSignerSpkiSha256 $descriptorSpkiSha256 `
        -OutputDirectory $staleMetadata `
        -DescriptorId "v1.2.3-alpha.1-labs-stale-test" `
        -IssuedAtUtc $testNow.AddDays(-2) `
        -LifetimeMinutes 60 |
        Out-Null
    $stalePreflight = $preflight |
        ConvertTo-Json -Depth 20 |
        ConvertFrom-Json -Depth 20
    foreach ($name in @(
        "release.json",
        "release-descriptor.receipt.json",
        "release-descriptor.spki.der"
    )) {
        $asset = @($stalePreflight.assets | Where-Object name -CEQ $name)
        $asset[0].sha256 = Get-RustyFleetSha256 `
            -LiteralPath (Join-Path $staleMetadata $name)
        $asset[0].size_bytes = (
            Get-Item -LiteralPath (Join-Path $staleMetadata $name)
        ).Length
    }
    $stalePreflight.descriptor_sha256 = (
        Get-RustyFleetSha256 `
            -LiteralPath (Join-Path $staleMetadata "release.json")
    )
    $stalePreflight.descriptor_receipt_sha256 = (
        Get-RustyFleetSha256 `
            -LiteralPath (
                Join-Path $staleMetadata "release-descriptor.receipt.json"
            )
    )
    $stalePreflightPath = Join-Path $testRoot "stale-preflight.json"
    Write-TestUtf8 `
        -LiteralPath $stalePreflightPath `
        -Content (ConvertTo-RustyFleetJson -InputObject $stalePreflight)
    $staleRejected = $false
    try {
        & (Join-Path $packagingRoot "New-WindowsPagesDeployment.ps1") `
            -Version "1.2.3" `
            -Channel labs `
            -Maturity alpha `
            -ReleaseTag "v1.2.3-alpha.1" `
            -SiteDirectory $siteRoot `
            -MetadataDirectory $staleMetadata `
            -PublicationPreflightReceiptPath $stalePreflightPath `
            -ExpectedSourceRevision $sourceRevision `
            -ExpectedSourceTree $sourceTree `
            -ExpectedDescriptorSignerSpkiSha256 $descriptorSpkiSha256 `
            -OutputDirectory (Join-Path $testRoot "pages-stale") `
            -NowUtc $testNow |
            Out-Null
    }
    catch {
        $staleRejected = $true
    }
    Assert-Publication $staleRejected "Pages accepted stale release metadata"

    $renewalMetadata = Join-Path $testRoot "renewal-metadata"
    & (Join-Path $packagingRoot "New-WindowsReleaseDescriptor.ps1") `
        -Version "1.2.3" `
        -Channel labs `
        -Maturity alpha `
        -ReleaseTag "v1.2.3-alpha.1" `
        -SetupPath $setupPath `
        -SetupBuildReceiptPath $setupReceiptPath `
        -ExpectedSetupSignerThumbprint $signerThumbprint `
        -ReleasePolicyPath $policyPath `
        -ExpectedSourceRevision $sourceRevision `
        -ExpectedSourceTree $sourceTree `
        -DescriptorPrivateKeyPemPath $descriptorKeyPath `
        -ExpectedDescriptorSignerSpkiSha256 $descriptorSpkiSha256 `
        -OutputDirectory $renewalMetadata `
        -DescriptorId "v1.2.3-alpha.1-labs-renewal-test" `
        -IssuedAtUtc $testNow.AddMinutes(5) `
        -LifetimeMinutes 1380 |
        Out-Null
    $renewalStage = Join-Path $testRoot "renewal-publication-input"
    Copy-Item -LiteralPath $stage -Destination $renewalStage -Recurse
    foreach ($name in @(
        "release.json",
        "release-descriptor.receipt.json",
        "release-descriptor.spki.der"
    )) {
        Copy-Item `
            -LiteralPath (Join-Path $renewalMetadata $name) `
            -Destination (Join-Path $renewalStage $name) `
            -Force
    }
    $renewalPreflight = Invoke-PublicationAuthority `
        -Mode Preflight `
        -InputRoot $renewalStage `
        -SourceRepository $sourceRepo `
        -SourceRevision $sourceRevision `
        -SourceTree $sourceTree `
        -FleetSigner $signerThumbprint `
        -HostessSigner $signerThumbprint `
        -DescriptorSpki $descriptorSpkiSha256 `
        -GhExecutable $fakeGh |
        ConvertFrom-Json -Depth 20
    $renewalPreflightPath = Join-Path $testRoot (
        "renewal-publication-preflight.json"
    )
    Write-TestUtf8 `
        -LiteralPath $renewalPreflightPath `
        -Content (ConvertTo-RustyFleetJson -InputObject $renewalPreflight)
    $renewalOutput = Join-Path $testRoot "pages-renewal"
    $renewalHandoff = & (
        Join-Path $packagingRoot "New-WindowsPagesDeployment.ps1"
    ) `
        -Version "1.2.3" `
        -Channel labs `
        -Maturity alpha `
        -ReleaseTag "v1.2.3-alpha.1" `
        -SiteDirectory $siteRoot `
        -MetadataDirectory $renewalMetadata `
        -PublicationPreflightReceiptPath $renewalPreflightPath `
        -ExpectedSourceRevision $sourceRevision `
        -ExpectedSourceTree $sourceTree `
        -ExpectedDescriptorSignerSpkiSha256 $descriptorSpkiSha256 `
        -OutputDirectory $renewalOutput `
        -PreviousHandoffPath $firstHandoffPath `
        -NowUtc $testNow.AddMinutes(5) |
        ConvertFrom-Json -Depth 20
    Assert-Publication (
        $renewalHandoff.deployment_sequence -eq 2 -and
        $renewalHandoff.previous_handoff_sha256 -cmatch
            "^[0-9a-f]{64}$" -and
        $renewalHandoff.descriptor_id -ceq
            "v1.2.3-alpha.1-labs-renewal-test" -and
        $renewalHandoff.expires_at_ms -gt $firstHandoff.expires_at_ms
    ) "fresh release metadata did not renew the Pages handoff"

    $replayRejected = $false
    try {
        & (Join-Path $packagingRoot "New-WindowsPagesDeployment.ps1") `
            -Version "1.2.3" `
            -Channel labs `
            -Maturity alpha `
            -ReleaseTag "v1.2.3-alpha.1" `
            -SiteDirectory $siteRoot `
            -MetadataDirectory $descriptorRoot `
            -PublicationPreflightReceiptPath $preflightPath `
            -ExpectedSourceRevision $sourceRevision `
            -ExpectedSourceTree $sourceTree `
            -ExpectedDescriptorSignerSpkiSha256 $descriptorSpkiSha256 `
            -OutputDirectory (Join-Path $testRoot "pages-replay") `
            -PreviousHandoffPath $firstHandoffPath `
            -NowUtc $testNow |
            Out-Null
    }
    catch {
        $replayRejected = $true
    }
    Assert-Publication $replayRejected "Pages accepted a descriptor replay"

    $expectedInputNames = @(
        "RustyFleet-Labs-Setup.exe",
        "$bundleName.zip",
        "$bundleName.zip.sha256",
        "$bundleName.manifest.json",
        "$bundleName.checksums.sha256",
        "$bundleName.validation-receipt.json",
        "RustyFleet-Labs-Setup.build-receipt.json",
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
        descriptor_receipt = "release-descriptor.receipt.json"
        zip = "$bundleName.zip"
        manifest = "$bundleName.manifest.json"
        checksums = "$bundleName.checksums.sha256"
        setup_build_receipt = "RustyFleet-Labs-Setup.build-receipt.json"
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

    foreach ($metadataMutation in @(
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
                $value.primary_artifact.url = (
                    "https://github.com/MesmerPrism/rusty-fleet/" +
                    "releases/download/v1.2.4/" +
                    "RustyFleet-Labs-Setup.exe"
                )
            }
        }
    )) {
        $metadataPath = Join-Path $caseRoot (
            "release-descriptor.receipt.json"
        )
        $metadata = Get-Content -LiteralPath (
            Join-Path $stage "release-descriptor.receipt.json"
        ) -Raw | ConvertFrom-Json -Depth 20
        & $metadataMutation.apply $metadata
        Write-TestUtf8 `
            -LiteralPath $metadataPath `
            -Content (ConvertTo-RustyFleetJson -InputObject $metadata)
        if (Test-Path -LiteralPath $fakeGhMarker) {
            Remove-Item -LiteralPath $fakeGhMarker -Force
        }
        $metadataRejected = $false
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
            $metadataRejected = $true
        }
        Assert-Publication (
            $metadataRejected -and
            -not (Test-Path -LiteralPath $fakeGhMarker)
        ) "wrong $($metadataMutation.name) reached the release token or gh"
        Copy-Item `
            -LiteralPath (
                Join-Path $stage "release-descriptor.receipt.json"
            ) `
            -Destination $metadataPath `
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
        recorded_and_local_authenticode_boundaries_independent = $true
        exact_zip_sidecar_and_full_bundle_validation = $true
        top_level_metadata_byte_equal = $true
        rsa_pss_jcs_descriptor_and_spki_verified = $true
        owner_release_metadata_verified = $true
        owner_release_metadata_substitution_rejected_before_gh = $true
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
        metadata_renewal_verified = $true
        stale_metadata_rejected = $true
        wrong_source_tag_asset_and_signer_rejected = $true
        metadata_replay_rejected = $true
        pages_binary_exclusion_verified = $true
        pages_interruption_and_resume_verified = $true
    } | ConvertTo-Json -Depth 10
}
finally {
    Remove-Item `
        -LiteralPath "Function:\global:Get-AuthenticodeSignature" `
        -Force `
        -ErrorAction SilentlyContinue
    [Environment]::SetEnvironmentVariable(
        "GH_TOKEN",
        $priorToken,
        [EnvironmentVariableTarget]::Process
    )
    if ($signingCertificate) {
        $signingCertificate.Dispose()
    }
    $descriptorRsa.Dispose()
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
