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
$testRoot = Join-Path ([IO.Path]::GetTempPath()) (
    "rusty-fleet-descriptor-$([Guid]::NewGuid().ToString('N'))"
)
[IO.Directory]::CreateDirectory($testRoot) | Out-Null
$rsa = [Security.Cryptography.RSA]::Create(3072)
$signingCertificate = $null
try {
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
    schema = "rusty.fleet.guided_installer_plan.v1",
    product = "rusty-fleet",
    version = "1.2.3",
    channel = "preview",
    asset_sha256 = Convert.ToHexStringLower(SHA256.HashData(stream)),
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
    $setupPath = Join-Path $testRoot "RustyFleet-Setup.exe"
    Copy-Item `
        -LiteralPath (Join-Path $fixtureOutput "RustyFleet-Setup.exe") `
        -Destination $setupPath
    $unsignedSetupSha256 = Get-RustyFleetSha256 -LiteralPath $setupPath
    $unsignedSetupSize = (Get-Item -LiteralPath $setupPath).Length
    $canonicalPayload = Get-RustyFleetPeCanonicalPayload `
        -LiteralPath $setupPath `
        -ExpectedPayloadSize $unsignedSetupSize

    $signingCertificate = New-RustyFleetTestCodeSigningCertificate `
        -Subject "CN=Rusty Fleet descriptor test"
    $signed = Set-AuthenticodeSignature `
        -FilePath $setupPath `
        -Certificate $signingCertificate `
        -HashAlgorithm SHA256
    Register-RustyFleetTestAuthenticodeSignature `
        -LiteralPath $setupPath `
        -Certificate $signingCertificate
    $signed = Get-AuthenticodeSignature -LiteralPath $setupPath
    Assert-Descriptor `
        ($signed.Status -eq [Management.Automation.SignatureStatus]::Valid) `
        (
            "the generated Setup fixture could not be signed: " +
            "$($signed.Status) ($($signed.StatusMessage))"
        )
    $setupSignature = Get-AuthenticodeSignature -LiteralPath $setupPath
    Assert-Descriptor `
        ($setupSignature.Status -eq [Management.Automation.SignatureStatus]::Valid) `
        "the generated signed Setup fixture is not trusted"
    $signedCanonicalPayload = Get-RustyFleetPeCanonicalPayload `
        -LiteralPath $setupPath `
        -ExpectedPayloadSize $unsignedSetupSize
    Assert-Descriptor `
        ($signedCanonicalPayload.sha256 -ceq $canonicalPayload.sha256) `
        "the signed Setup fixture changed its canonical PE payload"

    $buildReceiptPath = Join-Path $testRoot "setup-build-receipt.json"
    $buildReceipt = [ordered]@{
        schema = "rusty.fleet.windows_setup_build_receipt.v1"
        result = "pass"
        version = "1.2.3"
        channel = "preview"
        build_kind = "signed-release"
        setup_sha256 = $unsignedSetupSha256
        bundle_sha256 = "3" * 64
        manifest_sha256 = "4" * 64
        source_revision = "1" * 40
        source_tree = "2" * 40
        source_tree_clean = $true
        canonical_pe_payload_sha256 = $canonicalPayload.sha256
        canonical_pe_payload_size_bytes = $canonicalPayload.size_bytes
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
    $output = Join-Path $testRoot "output"
    $issued = [DateTimeOffset]::Parse(
        "2026-07-27T12:00:00Z",
        [Globalization.CultureInfo]::InvariantCulture
    )
    $receipt = & (Join-Path $packagingRoot "New-WindowsReleaseDescriptor.ps1") `
        -Version "1.2.3" `
        -Channel preview `
        -SetupPath $setupPath `
        -SetupBuildReceiptPath $buildReceiptPath `
        -ExpectedSetupSignerThumbprint $setupSignature.SignerCertificate.Thumbprint `
        -ExpectedSourceRevision ("1" * 40) `
        -ExpectedSourceTree ("2" * 40) `
        -DescriptorPrivateKeyPemPath $keyPath `
        -ExpectedDescriptorSignerSpkiSha256 $spkiSha256 `
        -OutputDirectory $output `
        -DescriptorId "v1.2.3-preview-test" `
        -IssuedAtUtc $issued `
        -LifetimeMinutes 1380 |
        ConvertFrom-Json
    Assert-Descriptor `
        ($receipt.result -eq "pass" -and
            $receipt.signature -eq "rsa_pss_sha256" -and
            $receipt.canonical_payload -eq "rfc8785_jcs_closed_shape") `
        "descriptor receipt is not exact"

    $descriptorPath = Join-Path $output "release.json"
    $publicKeyPath = Join-Path $output "release-descriptor.spki.der"
    $envelopeText = Get-Content -LiteralPath $descriptorPath -Raw
    $envelope = $envelopeText | ConvertFrom-Json
    $payloadBytes = ConvertFrom-Base64Url $envelope.payload_base64url
    $signatureBytes = ConvertFrom-Base64Url $envelope.signature_base64url
    Assert-Descriptor `
        ($envelope.schema -eq "rusty.fleet.release_descriptor_envelope.v2" -and
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
            '{"asset":{"installer_protocol":"rusty.fleet.guided_setup.v1",' +
            '"media_type":"application/vnd.microsoft.portable-executable",' +
            '"name":"RustyFleet-Setup.exe","sha256":"',
            [StringComparison]::Ordinal
        ) -and
            $payloadText.EndsWith(
                ',"product":"rusty-fleet",' +
                '"schema":"rusty.fleet.windows_release.v2",' +
                '"validity_duration_ms":82800000,' +
                '"version":"1.2.3"}',
                [StringComparison]::Ordinal
            ) -and
            $payload.asset.url -eq
                "https://github.com/MesmerPrism/rusty-fleet/releases/download/v1.2.3/RustyFleet-Setup.exe" -and
            $payload.asset.name -eq "RustyFleet-Setup.exe" -and
            $payload.asset.installer_protocol -eq "rusty.fleet.guided_setup.v1" -and
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
            -Channel preview `
            -SetupPath $setupPath `
            -SetupBuildReceiptPath $buildReceiptPath `
            -ExpectedSetupSignerThumbprint ("0" * 40) `
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
            -Channel preview `
            -SetupPath $setupPath `
            -SetupBuildReceiptPath $buildReceiptPath `
            -ExpectedSetupSignerThumbprint $setupSignature.SignerCertificate.Thumbprint `
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
            -Channel preview `
            -SetupPath $setupPath `
            -SetupBuildReceiptPath $buildReceiptPath `
            -ExpectedSetupSignerThumbprint $setupSignature.SignerCertificate.Thumbprint `
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
