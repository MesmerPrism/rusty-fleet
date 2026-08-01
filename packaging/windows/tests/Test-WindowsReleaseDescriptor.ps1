# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Descriptor([bool] $Condition, [string] $Message) {
    if (-not $Condition) {
        throw $Message
    }
}

function ConvertFrom-Base64Url([string] $Value) {
    $padded = $Value.Replace("-", "+").Replace("_", "/")
    $padded += "=" * ((4 - ($padded.Length % 4)) % 4)
    return [Convert]::FromBase64String($padded)
}

$packagingRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Import-Module (Join-Path $packagingRoot "Distribution.Common.psm1") -Force
Import-Module (
    Join-Path $PSScriptRoot "WindowsCertificateFixture.psm1"
) -Force
function global:Get-AuthenticodeSignature {
    param(
        [Parameter(Mandatory)]
        [string] $LiteralPath
    )

    Get-RustyFleetTestAuthenticodeSignature -LiteralPath $LiteralPath
}
$repoRoot = (Resolve-Path (Join-Path $packagingRoot "..\..")).Path
foreach ($case in @(
    [pscustomobject]@{ channel = "stable"; maturity = "released"; tag = "v9.9.9" },
    [pscustomobject]@{ channel = "labs"; maturity = "alpha"; tag = "v9.9.9-alpha.1" }
)) {
    $message = ""
    try {
        & (Join-Path $packagingRoot "New-WindowsReleaseDescriptor.ps1") `
            -Version "1.2.3" `
            -Channel $case.channel `
            -Maturity $case.maturity `
            -ReleaseTag $case.tag `
            -SetupPath "missing-setup.exe" `
            -SetupBuildReceiptPath "missing-build-receipt.json" `
            -ExpectedSetupSignerThumbprint ("A" * 40) `
            -ExpectedSourceRevision ("1" * 40) `
            -ExpectedSourceTree ("2" * 40) `
            -DescriptorPrivateKeyPemPath "missing-key.pem" `
            -ExpectedDescriptorSignerSpkiSha256 ("3" * 64) `
            -OutputDirectory "unused-output" | Out-Null
    }
    catch {
        $message = $_.Exception.Message
    }
    Assert-Descriptor `
        ($message -ceq "release tag does not bind the exact version and channel") `
        "$($case.channel) descriptor accepted a cross-version release tag"
}
$testRoot = Join-Path ([IO.Path]::GetTempPath()) (
    "rusty-fleet-descriptor-$([Guid]::NewGuid().ToString('N'))"
)
[IO.Directory]::CreateDirectory($testRoot) | Out-Null
$rsa = [Security.Cryptography.RSA]::Create(3072)
$signingCertificate = $null
try {
    $signingCertificate = New-RustyFleetTestCodeSigningCertificate `
        -Subject "CN=Rusty Fleet descriptor test"
    $signerCertificateSha256 = [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData(
            $signingCertificate.RawData
        )
    ).ToLowerInvariant()
    $fixtureRoot = Join-Path $testRoot "setup-fixture"
    $fixtureOutput = Join-Path $fixtureRoot "output"
    [IO.Directory]::CreateDirectory($fixtureRoot) | Out-Null
    [IO.File]::WriteAllText(
        (Join-Path $fixtureRoot "SetupFixture.csproj"),
        @"
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net10.0-windows10.0.19041.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
    <RuntimeIdentifier>win-x64</RuntimeIdentifier>
    <SelfContained>true</SelfContained>
    <PublishSingleFile>true</PublishSingleFile>
    <PublishTrimmed>false</PublishTrimmed>
    <PublishReadyToRun>false</PublishReadyToRun>
    <DebugType>None</DebugType>
    <DebugSymbols>false</DebugSymbols>
    <Deterministic>true</Deterministic>
    <AssemblyName>RustyFleet-Setup</AssemblyName>
  </PropertyGroup>
</Project>
"@,
        [Text.UTF8Encoding]::new($false)
    )
    [IO.File]::WriteAllText(
        (Join-Path $fixtureRoot "Program.cs"),
        @"
using System.Security.Cryptography;
using System.Text.Json;
if (!args.SequenceEqual(["--plan", "--json"], StringComparer.Ordinal))
{
    return 2;
}
using var stream = new FileStream(
    Environment.ProcessPath!,
    FileMode.Open,
    FileAccess.Read,
    FileShare.Read);
var plan = new
{
    schema = "rusty.fleet.guided_installer_plan.v2",
    product = "rusty-fleet-labs",
    version = "1.2.3",
    channel = "labs",
    asset_sha256 = Convert.ToHexStringLower(SHA256.HashData(stream)),
    authenticode_trust_mode = "exact-pinned-self-issued-untrusted-root-only",
    signer_certificate_sha256 = "$signerCertificateSha256",
    signer_self_issued = true,
    public_trust_claim = false,
    timestamp_required = true,
    ready = true
};
Console.WriteLine(JsonSerializer.Serialize(plan));
return 0;
"@,
        [Text.UTF8Encoding]::new($false)
    )
    & dotnet publish `
        (Join-Path $fixtureRoot "SetupFixture.csproj") `
        --nologo `
        --configuration Release `
        --output $fixtureOutput |
        Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "could not build the guided Setup protocol fixture"
    }
    $setupPath = Join-Path $testRoot "RustyFleet-Labs-Setup.exe"
    Copy-Item `
        -LiteralPath (Join-Path $fixtureOutput "RustyFleet-Setup.exe") `
        -Destination $setupPath
    $unsignedSetupSha256 = Get-RustyFleetSha256 -LiteralPath $setupPath
    $unsignedSetupSize = (Get-Item -LiteralPath $setupPath).Length
    $canonicalPayload = Get-RustyFleetPeCanonicalPayload `
        -LiteralPath $setupPath `
        -ExpectedPayloadSize $unsignedSetupSize

    $signed = Set-AuthenticodeSignature `
        -FilePath $setupPath `
        -Certificate $signingCertificate `
        -HashAlgorithm SHA256
    Register-RustyFleetTestAuthenticodeSignature `
        -LiteralPath $setupPath `
        -Certificate $signingCertificate
    $signed = Get-AuthenticodeSignature -LiteralPath $setupPath
    Assert-Descriptor `
        ($signed.Status -eq [Management.Automation.SignatureStatus]::UnknownError) `
        (
            "the generated Setup fixture could not be signed: " +
            "$($signed.Status) ($($signed.StatusMessage))"
        )
    $setupSignature = Get-AuthenticodeSignature -LiteralPath $setupPath
    Assert-Descriptor `
        ($setupSignature.Status -eq [Management.Automation.SignatureStatus]::UnknownError) `
        "the generated signed Setup fixture does not preserve untrusted-root truth"
    $signedCanonicalPayload = Get-RustyFleetPeCanonicalPayload `
        -LiteralPath $setupPath `
        -ExpectedPayloadSize $unsignedSetupSize
    Assert-Descriptor `
        ($signedCanonicalPayload.sha256 -ceq $canonicalPayload.sha256) `
        "the signed Setup fixture changed its canonical PE payload"

    $buildReceiptPath = Join-Path $testRoot "setup-build-receipt.json"
    $buildReceipt = [ordered]@{
        schema = "rusty.fleet.windows_setup_build_receipt.v3"
        result = "pass"
        version = "1.2.3"
        product_channel = "labs"
        maturity = "alpha"
        channel = "labs"
        distribution_track = "github-prerelease"
        build_kind = "signed-release"
        setup_sha256 = $unsignedSetupSha256
        bundle_sha256 = "3" * 64
        manifest_sha256 = "4" * 64
        source_revision = "1" * 40
        source_tree = "2" * 40
        source_tree_clean = $true
        canonical_pe_payload_sha256 = $canonicalPayload.sha256
        canonical_pe_payload_size_bytes = $canonicalPayload.size_bytes
        authenticode_trust_mode = "exact-pinned-self-issued-untrusted-root-only"
        signer_certificate_sha256 = $signerCertificateSha256
        signer_self_issued = $true
        public_trust_claim = $false
        timestamp_required = $true
        distribution_eligibility = "requires_setup_authenticode_signing"
    }
    [IO.File]::WriteAllText(
        $buildReceiptPath,
        ($buildReceipt | ConvertTo-Json -Depth 10),
        [Text.UTF8Encoding]::new($false)
    )

    $keyPath = Join-Path $testRoot "descriptor-key.pem"
    [IO.File]::WriteAllText(
        $keyPath,
        $rsa.ExportPkcs8PrivateKeyPem(),
        [Text.UTF8Encoding]::new($false)
    )
    $spkiSha256 = [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData(
            $rsa.ExportSubjectPublicKeyInfo()
        )
    ).ToLowerInvariant()
    $testPolicyPath = Join-Path $testRoot "release-policy.json"
    $testPolicy = [ordered]@{
        schema = "rusty.fleet.windows_release_trust_policy.v2"
        channels = [ordered]@{
            labs = [ordered]@{
                publication_enabled = $true
                authenticode = [ordered]@{
                    subject = $signingCertificate.Subject
                    thumbprint = $signingCertificate.Thumbprint.ToUpperInvariant()
                    certificate_sha256 = $signerCertificateSha256
                    self_issued = $true
                    public_trust_claim = $false
                    trust_mode = "exact-pinned-self-issued-untrusted-root-only"
                    timestamp_required = $true
                    allowed_chain_status_flags = @("UntrustedRoot")
                }
                authorized_descriptor_signer_spki_sha256 = @($spkiSha256)
                status = "labs_exact_pinned_self_issued_signer_configured"
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
    [IO.File]::WriteAllText(
        $testPolicyPath,
        ($testPolicy | ConvertTo-Json -Depth 10),
        [Text.UTF8Encoding]::new($false)
    )
    $output = Join-Path $testRoot "output"
    $issued = [DateTimeOffset]::Parse(
        "2026-07-27T12:00:00Z",
        [Globalization.CultureInfo]::InvariantCulture
    )
    $receipt = & (Join-Path $packagingRoot "New-WindowsReleaseDescriptor.ps1") `
        -Version "1.2.3" `
        -Channel labs -Maturity alpha `
        -ReleaseTag "v1.2.3-alpha.1" `
        -SetupPath $setupPath `
        -SetupBuildReceiptPath $buildReceiptPath `
        -ExpectedSetupSignerThumbprint $setupSignature.SignerCertificate.Thumbprint `
        -ReleasePolicyPath $testPolicyPath `
        -ExpectedSourceRevision ("1" * 40) `
        -ExpectedSourceTree ("2" * 40) `
        -DescriptorPrivateKeyPemPath $keyPath `
        -ExpectedDescriptorSignerSpkiSha256 $spkiSha256 `
        -OutputDirectory $output `
        -DescriptorId "v1.2.3-alpha.1-test" `
        -IssuedAtUtc $issued `
        -LifetimeMinutes 1380 |
        ConvertFrom-Json
    Assert-Descriptor `
        ($receipt.result -eq "pass" -and
            $receipt.signature -eq "rsa_pss_sha256" -and
            $receipt.canonical_payload -eq "rfc8785_jcs_closed_shape" -and
            $receipt.schema -eq
                "rusty.fleet.windows_release_descriptor_receipt.v5" -and
            $receipt.release_tag -ceq "v1.2.3-alpha.1" -and
            $receipt.installation_identity -ceq "rusty-fleet-labs" -and
            $receipt.primary_artifact.role -ceq "complete-product" -and
            $receipt.primary_artifact.name -ceq "RustyFleet-Labs-Setup.exe" -and
            $receipt.primary_artifact.sha256 -ceq
                (Get-RustyFleetSha256 -LiteralPath $setupPath) -and
            [long] $receipt.primary_artifact.bytes -eq
                (Get-Item -LiteralPath $setupPath).Length -and
            $receipt.primary_artifact.url -ceq
                $receipt.asset_url) `
        "descriptor receipt is not exact"

    $descriptorPath = Join-Path $output "release.json"
    $publicKeyPath = Join-Path $output "release-descriptor.spki.der"
    $envelopeText = Get-Content -LiteralPath $descriptorPath -Raw
    $envelope = $envelopeText | ConvertFrom-Json
    $payloadBytes = ConvertFrom-Base64Url $envelope.payload_base64url
    $signatureBytes = ConvertFrom-Base64Url $envelope.signature_base64url
    Assert-Descriptor `
        ($envelope.schema -eq "rusty.fleet.release_descriptor_envelope.v4" -and
            $envelope.signer_spki_sha256 -ceq $spkiSha256 -and
            $receipt.descriptor_signer_spki_asset -ceq
                "release-descriptor.spki.der" -and
            [Security.Cryptography.CryptographicOperations]::FixedTimeEquals(
                [IO.File]::ReadAllBytes($publicKeyPath),
                $rsa.ExportSubjectPublicKeyInfo()
            ) -and
            $rsa.VerifyData(
                $payloadBytes,
                $signatureBytes,
                [Security.Cryptography.HashAlgorithmName]::SHA256,
                [Security.Cryptography.RSASignaturePadding]::Pss
            )) `
        "descriptor envelope signature or SPKI binding failed"

    $payloadText = [Text.Encoding]::UTF8.GetString($payloadBytes)
    $payload = $payloadText | ConvertFrom-Json
    Assert-Descriptor `
        ($payloadText.StartsWith(
            '{"asset":{"authenticode_trust_mode":"exact-pinned-self-issued-untrusted-root-only",' +
            '"installer_protocol":"rusty.fleet.guided_setup.v2",' +
            '"media_type":"application/vnd.microsoft.portable-executable",' +
            '"name":"RustyFleet-Labs-Setup.exe","public_trust_claim":false,"sha256":"',
            [StringComparison]::Ordinal
        ) -and
            $payloadText.EndsWith(
                ',"maturity":"alpha",' +
                '"product":"rusty-fleet-labs",' +
                '"product_channel":"labs",' +
                '"schema":"rusty.fleet.windows_release.v4",' +
                '"validity_duration_ms":82800000,' +
                '"version":"1.2.3"}',
                [StringComparison]::Ordinal
            ) -and
            $payload.asset.url -eq
                "https://github.com/MesmerPrism/rusty-fleet/releases/download/v1.2.3-alpha.1/RustyFleet-Labs-Setup.exe" -and
            $payload.asset.name -eq "RustyFleet-Labs-Setup.exe" -and
            $payload.product_channel -eq "labs" -and
            $payload.maturity -eq "alpha" -and
            $payload.channel -eq "labs" -and
            $payload.distribution_track -eq "github-prerelease" -and
            $payload.asset.installer_protocol -eq "rusty.fleet.guided_setup.v2" -and
            $payload.asset.public_trust_claim -eq $false -and
            $payload.asset.signer_self_issued -eq $true -and
            $payload.validity_duration_ms -eq 82800000 -and
            ($payload.expires_at_ms - $payload.issued_at_ms) -eq
                $payload.validity_duration_ms) `
        "signed payload is not the exact closed JCS release contract"

    $retainedRoot = Join-Path $testRoot "retained-path"
    $retainedParent = Join-Path $retainedRoot "parent"
    [IO.Directory]::CreateDirectory($retainedParent) | Out-Null
    $retainedFile = Join-Path $retainedParent "fixture.exe"
    [IO.File]::WriteAllText(
        $retainedFile,
        "retained-path-test",
        [Text.UTF8Encoding]::new($false)
    )
    $retainedChain = (
        [RustyFleet.Release.RetainedDirectoryChain]::Open($retainedFile)
    )
    $retainedStream = [IO.File]::Open(
        $retainedFile,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::Read
    )
    try {
        $retainedIdentity = (
            [RustyFleet.Release.RetainedDirectoryChain]::GetFileIdentity(
                $retainedStream.SafeFileHandle
            )
        )
        $writeDenied = $false
        try {
            $writer = [IO.File]::Open(
                $retainedFile,
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
                -LiteralPath $retainedFile `
                -Destination (Join-Path $retainedParent "replacement.exe")
        }
        catch {
            $leafRenameDenied = $true
        }
        $ancestorRenameDenied = $false
        try {
            Move-Item `
                -LiteralPath $retainedParent `
                -Destination (Join-Path $retainedRoot "replacement-parent")
        }
        catch {
            $ancestorRenameDenied = $true
        }
        $retainedChain.Verify()
        [RustyFleet.Release.RetainedDirectoryChain]::VerifyFile(
            $retainedFile,
            $retainedIdentity
        )
        Assert-Descriptor `
            ($writeDenied -and $leafRenameDenied -and $ancestorRenameDenied) `
            "retained Setup path accepted write, leaf, or ancestor substitution"
    }
    finally {
        $retainedStream.Dispose()
        $retainedChain.Dispose()
    }

    $wrongSetupSignerRejected = $false
    try {
        & (Join-Path $packagingRoot "New-WindowsReleaseDescriptor.ps1") `
            -Version "1.2.3" `
            -Channel labs -Maturity alpha -ReleaseTag "v1.2.3-alpha.1" `
            -SetupPath $setupPath `
            -SetupBuildReceiptPath $buildReceiptPath `
            -ExpectedSetupSignerThumbprint ("0" * 40) `
            -ReleasePolicyPath $testPolicyPath `
            -ExpectedSourceRevision ("1" * 40) `
            -ExpectedSourceTree ("2" * 40) `
            -DescriptorPrivateKeyPemPath $keyPath `
            -ExpectedDescriptorSignerSpkiSha256 $spkiSha256 `
            -OutputDirectory (Join-Path $testRoot "wrong-signer") | Out-Null
    }
    catch {
        $wrongSetupSignerRejected = $true
    }
    Assert-Descriptor $wrongSetupSignerRejected "wrong Setup signer pin was accepted"

    $wrongSpkiRejected = $false
    try {
        & (Join-Path $packagingRoot "New-WindowsReleaseDescriptor.ps1") `
            -Version "1.2.3" `
            -Channel labs -Maturity alpha -ReleaseTag "v1.2.3-alpha.1" `
            -SetupPath $setupPath `
            -SetupBuildReceiptPath $buildReceiptPath `
            -ExpectedSetupSignerThumbprint $setupSignature.SignerCertificate.Thumbprint `
            -ReleasePolicyPath $testPolicyPath `
            -ExpectedSourceRevision ("1" * 40) `
            -ExpectedSourceTree ("2" * 40) `
            -DescriptorPrivateKeyPemPath $keyPath `
            -ExpectedDescriptorSignerSpkiSha256 ("0" * 64) `
            -OutputDirectory (Join-Path $testRoot "wrong-spki") | Out-Null
    }
    catch {
        $wrongSpkiRejected = $true
    }
    Assert-Descriptor $wrongSpkiRejected "wrong descriptor SPKI pin was accepted"

    $overwriteRejected = $false
    try {
        & (Join-Path $packagingRoot "New-WindowsReleaseDescriptor.ps1") `
            -Version "1.2.3" `
            -Channel labs -Maturity alpha -ReleaseTag "v1.2.3-alpha.1" `
            -SetupPath $setupPath `
            -SetupBuildReceiptPath $buildReceiptPath `
            -ExpectedSetupSignerThumbprint $setupSignature.SignerCertificate.Thumbprint `
            -ReleasePolicyPath $testPolicyPath `
            -ExpectedSourceRevision ("1" * 40) `
            -ExpectedSourceTree ("2" * 40) `
            -DescriptorPrivateKeyPemPath $keyPath `
            -ExpectedDescriptorSignerSpkiSha256 $spkiSha256 `
            -OutputDirectory $output | Out-Null
    }
    catch {
        $overwriteRejected = $true
    }
    Assert-Descriptor $overwriteRejected "existing release descriptor was overwritten"
    Assert-Descriptor `
        ($envelopeText -notmatch [Regex]::Escape($testRoot)) `
        "release descriptor leaked a machine-local path"

    [ordered]@{
        schema = "rusty.fleet.windows_release_descriptor_test.v2"
        result = "pass"
        rsa_pss_sha256 = $true
        spki_pin_exact = $true
        setup_signer_pin_exact = $true
        canonical_closed_payload = $true
        immutable_asset_url = $true
        release_tag_exact = $true
        installation_identity_exact = $true
        primary_artifact_exact = $true
        labs_descriptor_end_to_end = $true
        lifetime_23_hours = $true
        retained_path_substitution_rejected = $true
        overwrite_rejected = $true
        machine_paths_absent = $true
    } | ConvertTo-Json -Depth 5
}
finally {
    Remove-Item `
        -LiteralPath "Function:\global:Get-AuthenticodeSignature" `
        -Force `
        -ErrorAction SilentlyContinue
    if ($signingCertificate) {
        $signingCertificate.Dispose()
    }
    $rsa.Dispose()
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
