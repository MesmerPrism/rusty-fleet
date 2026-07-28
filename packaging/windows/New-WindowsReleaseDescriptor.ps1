# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9]+\.[0-9]+\.[0-9]+$")]
    [string] $Version,

    [Parameter(Mandatory)]
    [ValidateSet("dev", "preview", "stable")]
    [string] $Channel,

    [Parameter(Mandatory)]
    [string] $SetupPath,

    [Parameter(Mandatory)]
    [string] $SetupBuildReceiptPath,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9A-Fa-f]{40}$")]
    [string] $ExpectedSetupSignerThumbprint,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9a-f]{40}$")]
    [string] $ExpectedSourceRevision,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9a-f]{40}$")]
    [string] $ExpectedSourceTree,

    [Parameter(Mandatory)]
    [string] $DescriptorPrivateKeyPemPath,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9a-f]{64}$")]
    [string] $ExpectedDescriptorSignerSpkiSha256,

    [Parameter(Mandatory)]
    [string] $OutputDirectory,

    [ValidatePattern("^[A-Za-z0-9._-]{1,128}$")]
    [string] $DescriptorId,

    [DateTimeOffset] $IssuedAtUtc = [DateTimeOffset]::UtcNow,

    [ValidateRange(1, 1440)]
    [int] $LifetimeMinutes = 1380
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
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

function Get-RetainedSha256 {
    param([Parameter(Mandatory)][System.IO.FileStream] $Stream)

    $position = $Stream.Position
    try {
        $Stream.Position = 0
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            return [Convert]::ToHexString($sha.ComputeHash($Stream)).
                ToLowerInvariant()
        }
        finally {
            $sha.Dispose()
        }
    }
    finally {
        $Stream.Position = $position
    }
}

if (-not ("RustyFleet.Release.RetainedDirectoryChain" -as [type])) {
    Add-Type -TypeDefinition @"
using Microsoft.Win32.SafeHandles;
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;

namespace RustyFleet.Release
{
    public readonly struct PathIdentity : IEquatable<PathIdentity>
    {
        public PathIdentity(uint volume, ulong file)
        {
            Volume = volume;
            File = file;
        }

        public uint Volume { get; }
        public ulong File { get; }

        public bool Equals(PathIdentity other) =>
            Volume == other.Volume && File == other.File;
    }

    public sealed class RetainedDirectoryChain : IDisposable
    {
        private const uint FileReadAttributes = 0x0080;
        private const uint GenericRead = 0x80000000;
        private const uint ShareRead = 0x00000001;
        private const uint ShareWrite = 0x00000002;
        private const uint ShareDelete = 0x00000004;
        private const uint OpenExisting = 3;
        private const uint BackupSemantics = 0x02000000;
        private const uint OpenReparsePoint = 0x00200000;
        private const uint ReparsePoint = 0x00000400;

        private readonly List<string> paths = new();
        private readonly List<PathIdentity> identities = new();
        private readonly List<SafeFileHandle> handles = new();

        private RetainedDirectoryChain() { }

        public static RetainedDirectoryChain Open(string filePath)
        {
            var fullPath = Path.GetFullPath(filePath);
            var root = Path.GetPathRoot(fullPath)
                ?? throw new InvalidDataException("local volume root is unavailable");
            if (new DriveInfo(root).DriveType != DriveType.Fixed)
            {
                throw new InvalidDataException("release input is not on a fixed local volume");
            }

            var parent = Directory.GetParent(fullPath)
                ?? throw new InvalidDataException("release input parent is unavailable");
            var ordered = new Stack<string>();
            for (var current = parent; current != null; current = current.Parent)
            {
                ordered.Push(current.FullName);
            }

            var chain = new RetainedDirectoryChain();
            try
            {
                while (ordered.Count > 0)
                {
                    var path = ordered.Pop();
                    var handle = OpenDirectory(path, ShareRead | ShareWrite);
                    var information = ReadInformation(handle);
                    if ((information.dwFileAttributes & ReparsePoint) != 0)
                    {
                        handle.Dispose();
                        throw new InvalidDataException("reparse directory is not allowed");
                    }
                    chain.paths.Add(path);
                    chain.identities.Add(ToIdentity(information));
                    chain.handles.Add(handle);
                }
                return chain;
            }
            catch
            {
                chain.Dispose();
                throw;
            }
        }

        public void Verify()
        {
            for (var index = 0; index < paths.Count; index++)
            {
                using var handle = OpenDirectory(
                    paths[index],
                    ShareRead | ShareWrite | ShareDelete);
                var information = ReadInformation(handle);
                if ((information.dwFileAttributes & ReparsePoint) != 0 ||
                    !identities[index].Equals(ToIdentity(information)))
                {
                    throw new InvalidDataException("retained directory identity changed");
                }
            }
        }

        public static PathIdentity GetFileIdentity(SafeFileHandle handle) =>
            ToIdentity(ReadInformation(handle));

        public static void VerifyFile(string path, PathIdentity expected)
        {
            using var handle = CreateFileW(
                Path.GetFullPath(path),
                GenericRead,
                ShareRead | ShareWrite | ShareDelete,
                IntPtr.Zero,
                OpenExisting,
                OpenReparsePoint,
                IntPtr.Zero);
            if (handle.IsInvalid)
            {
                throw new IOException("could not reopen retained release file");
            }
            var information = ReadInformation(handle);
            if ((information.dwFileAttributes & ReparsePoint) != 0 ||
                !expected.Equals(ToIdentity(information)))
            {
                throw new InvalidDataException("retained release file identity changed");
            }
        }

        public void Dispose()
        {
            for (var index = handles.Count - 1; index >= 0; index--)
            {
                handles[index].Dispose();
            }
            handles.Clear();
        }

        private static SafeFileHandle OpenDirectory(string path, uint share)
        {
            var handle = CreateFileW(
                path,
                FileReadAttributes,
                share,
                IntPtr.Zero,
                OpenExisting,
                BackupSemantics | OpenReparsePoint,
                IntPtr.Zero);
            if (handle.IsInvalid)
            {
                throw new IOException("could not retain release directory");
            }
            return handle;
        }

        private static PathIdentity ToIdentity(BY_HANDLE_FILE_INFORMATION value) =>
            new(
                value.dwVolumeSerialNumber,
                ((ulong)value.nFileIndexHigh << 32) | value.nFileIndexLow);

        private static BY_HANDLE_FILE_INFORMATION ReadInformation(
            SafeFileHandle handle)
        {
            if (!GetFileInformationByHandle(handle, out var information))
            {
                throw new IOException("could not inspect retained release identity");
            }
            return information;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct BY_HANDLE_FILE_INFORMATION
        {
            public uint dwFileAttributes;
            public System.Runtime.InteropServices.ComTypes.FILETIME ftCreationTime;
            public System.Runtime.InteropServices.ComTypes.FILETIME ftLastAccessTime;
            public System.Runtime.InteropServices.ComTypes.FILETIME ftLastWriteTime;
            public uint dwVolumeSerialNumber;
            public uint nFileSizeHigh;
            public uint nFileSizeLow;
            public uint nNumberOfLinks;
            public uint nFileIndexHigh;
            public uint nFileIndexLow;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern SafeFileHandle CreateFileW(
            string fileName,
            uint desiredAccess,
            uint shareMode,
            IntPtr securityAttributes,
            uint creationDisposition,
            uint flagsAndAttributes,
            IntPtr templateFile);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool GetFileInformationByHandle(
            SafeFileHandle file,
            out BY_HANDLE_FILE_INFORMATION information);
    }

    public sealed class BoundedProcessResult
    {
        public int ExitCode { get; init; }
        public string StandardOutput { get; init; } = "";
        public bool StandardErrorPresent { get; init; }
    }

    public static class BoundedProcessRunner
    {
        public static BoundedProcessResult Run(
            string fileName,
            string[] arguments,
            int timeoutMilliseconds,
            int maximumBytes)
        {
            var start = new ProcessStartInfo
            {
                FileName = fileName,
                UseShellExecute = false,
                CreateNoWindow = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true
            };
            foreach (var argument in arguments)
            {
                start.ArgumentList.Add(argument);
            }

            using var process = new Process { StartInfo = start };
            if (!process.Start())
            {
                throw new InvalidOperationException("release plan process did not start");
            }
            var output = ReadBounded(process.StandardOutput, maximumBytes);
            var error = ReadBounded(process.StandardError, maximumBytes);
            if (!process.WaitForExit(timeoutMilliseconds))
            {
                process.Kill(true);
                process.WaitForExit();
                throw new TimeoutException("release plan process exceeded its limit");
            }
            try
            {
                Task.WaitAll(new Task[] { output, error });
            }
            catch
            {
                if (!process.HasExited)
                {
                    process.Kill(true);
                }
                throw;
            }
            return new BoundedProcessResult
            {
                ExitCode = process.ExitCode,
                StandardOutput = output.GetAwaiter().GetResult(),
                StandardErrorPresent =
                    error.GetAwaiter().GetResult().Length != 0
            };
        }

        private static async Task<string> ReadBounded(
            StreamReader reader,
            int maximumBytes)
        {
            var result = new StringBuilder();
            var buffer = new char[2048];
            var observedBytes = 0;
            while (true)
            {
                var read = await reader.ReadAsync(buffer, 0, buffer.Length)
                    .ConfigureAwait(false);
                if (read == 0)
                {
                    return result.ToString();
                }
                observedBytes += Encoding.UTF8.GetByteCount(buffer, 0, read);
                if (observedBytes > maximumBytes)
                {
                    throw new InvalidDataException("release plan output exceeded its limit");
                }
                result.Append(buffer, 0, read);
            }
        }
    }
}
"@
}

$setup = (Resolve-Path -LiteralPath $SetupPath).Path
if ((Split-Path -Leaf $setup) -cne "RustyFleet-Setup.exe") {
    throw "Setup filename must be exactly RustyFleet-Setup.exe"
}
$buildReceiptPath = (
    Resolve-Path -LiteralPath $SetupBuildReceiptPath
).Path
[byte[]] $buildReceiptBytes = [IO.File]::ReadAllBytes($buildReceiptPath)
if ($buildReceiptBytes.Length -le 0 -or $buildReceiptBytes.Length -gt 65536) {
    throw "Setup build receipt size is outside the release contract"
}
$buildReceiptSha256 = [Convert]::ToHexString(
    [Security.Cryptography.SHA256]::HashData($buildReceiptBytes)
).ToLowerInvariant()
$buildReceipt = [Text.UTF8Encoding]::new(
    $false,
    $true
).GetString($buildReceiptBytes) | ConvertFrom-Json -Depth 10
Assert-ExactProperties -InputObject $buildReceipt -Expected @(
    "schema",
    "result",
    "version",
    "channel",
    "build_kind",
    "setup_sha256",
    "bundle_sha256",
    "manifest_sha256",
    "source_revision",
    "source_tree",
    "source_tree_clean",
    "canonical_pe_payload_sha256",
    "canonical_pe_payload_size_bytes",
    "distribution_eligibility"
) -Context "Setup build receipt"
if ($buildReceipt.schema -cne "rusty.fleet.windows_setup_build_receipt.v1" -or
    $buildReceipt.result -cne "pass" -or
    $buildReceipt.version -cne $Version -or
    $buildReceipt.channel -cne $Channel -or
    $buildReceipt.build_kind -cne "signed-release" -or
    $buildReceipt.source_revision -cne $ExpectedSourceRevision -or
    $buildReceipt.source_tree -cne $ExpectedSourceTree -or
    $buildReceipt.source_tree_clean -ne $true -or
    $buildReceipt.distribution_eligibility -cne
        "requires_setup_authenticode_signing" -or
    $buildReceipt.setup_sha256 -cnotmatch "^[0-9a-f]{64}$" -or
    $buildReceipt.bundle_sha256 -cnotmatch "^[0-9a-f]{64}$" -or
    $buildReceipt.manifest_sha256 -cnotmatch "^[0-9a-f]{64}$" -or
    $buildReceipt.canonical_pe_payload_sha256 -cnotmatch
        "^[0-9a-f]{64}$" -or
    [long] $buildReceipt.canonical_pe_payload_size_bytes -le 0) {
    throw "Setup build receipt is not exact signed-release provenance"
}

$setupChain = $null
$setupHandle = $null
try {
    $setupChain = [RustyFleet.Release.RetainedDirectoryChain]::Open($setup)
    $setupHandle = [IO.File]::Open(
        $setup,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::Read
    )
    $setupIdentity = (
        [RustyFleet.Release.RetainedDirectoryChain]::GetFileIdentity(
            $setupHandle.SafeFileHandle
        )
    )
}
catch {
    if ($setupHandle) {
        $setupHandle.Dispose()
    }
    if ($setupChain) {
        $setupChain.Dispose()
    }
    throw "Setup retained-path validation failed closed"
}

function Assert-RetainedSetupPath {
    try {
        $setupChain.Verify()
        [RustyFleet.Release.RetainedDirectoryChain]::VerifyFile(
            $setup,
            $setupIdentity
        )
    }
    catch {
        throw "Setup retained-path identity changed"
    }
}

try {
Assert-RetainedSetupPath
$setupInfo = Get-Item -LiteralPath $setup
if ($setupInfo.Length -lt 1 -or $setupInfo.Length -gt 536870912) {
    throw "Setup size is outside the release contract"
}
$canonicalPayload = Get-RustyFleetPeCanonicalPayload `
    -LiteralPath $setup `
    -ExpectedPayloadSize (
        [long] $buildReceipt.canonical_pe_payload_size_bytes
    )
if ($canonicalPayload.sha256 -cne
        $buildReceipt.canonical_pe_payload_sha256 -or
    $canonicalPayload.size_bytes -ne
        [long] $buildReceipt.canonical_pe_payload_size_bytes) {
    throw "signed Setup does not match the canonical pre-sign build receipt"
}
Assert-RetainedSetupPath
$signature = Get-AuthenticodeSignature -LiteralPath $setup
if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
    $null -eq $signature.SignerCertificate -or
    $signature.SignerCertificate.Thumbprint -cne
        $ExpectedSetupSignerThumbprint.ToUpperInvariant()) {
    throw "Setup must have a valid Authenticode signature before descriptor signing"
}
Assert-RetainedSetupPath
$setupCertificateSha256 = [Convert]::ToHexString(
    [Security.Cryptography.SHA256]::HashData(
        $signature.SignerCertificate.RawData
    )
).ToLowerInvariant()
$setupSha256 = Get-RetainedSha256 -Stream $setupHandle

Assert-RetainedSetupPath
try {
    $planResult = [RustyFleet.Release.BoundedProcessRunner]::Run(
        $setup,
        @("--plan", "--json"),
        15000,
        65536
    )
    if ($planResult.ExitCode -ne 0 -or $planResult.StandardErrorPresent) {
        throw "plan_failed"
    }
    $plan = $planResult.StandardOutput | ConvertFrom-Json -Depth 10
    if ($null -eq $plan -or
        @($plan.PSObject.Properties).Count -ne 6 -or
        $plan.schema -cne "rusty.fleet.guided_installer_plan.v1" -or
        $plan.product -cne "rusty-fleet" -or
        $plan.version -cne $Version -or
        $plan.channel -cne $Channel -or
        $plan.asset_sha256 -cne $setupSha256 -or
        $plan.ready -ne $true) {
        throw "Setup no-change plan is not bound to the release descriptor inputs"
    }
}
catch {
    throw "Setup planning route failed closed"
}
Assert-RetainedSetupPath
if ((Get-RetainedSha256 -Stream $setupHandle) -cne $setupSha256) {
    throw "Setup changed while its exact planning route was running"
}

$privateKeyPath = (Resolve-Path -LiteralPath $DescriptorPrivateKeyPemPath).Path
$privateKeyText = [IO.File]::ReadAllText($privateKeyPath, [Text.Encoding]::UTF8)
$rsa = [Security.Cryptography.RSA]::Create()
try {
    $rsa.ImportFromPem($privateKeyText)
    if ($rsa.KeySize -lt 3072) {
        throw "descriptor RSA key must be at least 3072 bits"
    }
    $spki = $rsa.ExportSubjectPublicKeyInfo()
    $spkiSha256 = [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData($spki)
    ).ToLowerInvariant()
    if ($spkiSha256 -cne $ExpectedDescriptorSignerSpkiSha256) {
        throw "descriptor private key does not match the independently supplied SPKI pin"
    }
    Assert-RetainedSetupPath

    $issuedAtMs = $IssuedAtUtc.ToUniversalTime().ToUnixTimeMilliseconds()
    $expiresAtMs = $IssuedAtUtc.ToUniversalTime().
        AddMinutes($LifetimeMinutes).ToUnixTimeMilliseconds()
    $validityDurationMs = $expiresAtMs - $issuedAtMs
    if ($issuedAtMs -le 0 -or
        $expiresAtMs -le $issuedAtMs -or
        $validityDurationMs -gt 86400000) {
        throw "descriptor freshness interval is outside the v2 contract"
    }
    if (-not $DescriptorId) {
        $DescriptorId = "v$Version-$Channel-$($setupSha256.Substring(0, 16))"
    }
    $assetUrl = (
        "https://github.com/MesmerPrism/rusty-fleet/releases/download/" +
        "v$Version/RustyFleet-Setup.exe"
    )

    # Every variable interpolated below is constrained to ASCII identifiers,
    # exact constant text, lowercase hex, an exact derived URL, or an integer.
    # The fixed lexicographic property order and compact UTF-8 spelling are the
    # RFC 8785 JCS representation for this deliberately closed payload shape.
    $payloadText = (
        '{"asset":{' +
        '"installer_protocol":"rusty.fleet.guided_setup.v1",' +
        '"media_type":"application/vnd.microsoft.portable-executable",' +
        '"name":"RustyFleet-Setup.exe",' +
        '"sha256":"' + $setupSha256 + '",' +
        '"signer_certificate_sha256":"' + $setupCertificateSha256 + '",' +
        '"size_bytes":' + $setupInfo.Length + ',' +
        '"url":"' + $assetUrl + '"},' +
        '"channel":"' + $Channel + '",' +
        '"descriptor_id":"' + $DescriptorId + '",' +
        '"expires_at_ms":' + $expiresAtMs + ',' +
        '"issued_at_ms":' + $issuedAtMs + ',' +
        '"product":"rusty-fleet",' +
        '"schema":"rusty.fleet.windows_release.v2",' +
        '"validity_duration_ms":' + $validityDurationMs + ',' +
        '"version":"' + $Version + '"}'
    )
    $payloadBytes = [Text.Encoding]::UTF8.GetBytes($payloadText)
    $signatureBytes = $rsa.SignData(
        $payloadBytes,
        [Security.Cryptography.HashAlgorithmName]::SHA256,
        [Security.Cryptography.RSASignaturePadding]::Pss
    )
    if (-not $rsa.VerifyData(
        $payloadBytes,
        $signatureBytes,
        [Security.Cryptography.HashAlgorithmName]::SHA256,
        [Security.Cryptography.RSASignaturePadding]::Pss
    )) {
        throw "descriptor signature self-verification failed"
    }
    function ConvertTo-Base64Url([byte[]] $Bytes) {
        return [Convert]::ToBase64String($Bytes).
            TrimEnd("=").
            Replace("+", "-").
            Replace("/", "_")
    }
    $envelope = [ordered]@{
        schema = "rusty.fleet.release_descriptor_envelope.v2"
        payload_base64url = ConvertTo-Base64Url $payloadBytes
        signature_base64url = ConvertTo-Base64Url $signatureBytes
        signer_spki_sha256 = $spkiSha256
    }
    $envelopeText = ConvertTo-RustyFleetJson -InputObject $envelope
    if ([Text.Encoding]::UTF8.GetByteCount($envelopeText) -gt 65536) {
        throw "release descriptor exceeds the v2 size limit"
    }

    $output = [IO.Path]::GetFullPath($OutputDirectory)
    [IO.Directory]::CreateDirectory($output) | Out-Null
    $descriptorPath = Join-Path $output "release.json"
    $receiptPath = Join-Path $output "release-descriptor.receipt.json"
    $publicKeyPath = Join-Path $output "release-descriptor.spki.der"
    foreach ($path in @($descriptorPath, $receiptPath, $publicKeyPath)) {
        if (Test-Path -LiteralPath $path) {
            throw "refusing to overwrite existing descriptor output"
        }
    }
    Write-RustyFleetUtf8 -LiteralPath $descriptorPath -Content $envelopeText
    [IO.File]::WriteAllBytes($publicKeyPath, $spki)
    $receipt = [ordered]@{
        schema = "rusty.fleet.windows_release_descriptor_receipt.v2"
        result = "pass"
        descriptor_id = $DescriptorId
        version = $Version
        channel = $Channel
        issued_at_ms = $issuedAtMs
        expires_at_ms = $expiresAtMs
        validity_duration_ms = $validityDurationMs
        setup_sha256 = $setupSha256
        setup_size_bytes = $setupInfo.Length
        setup_signer_certificate_sha256 = $setupCertificateSha256
        setup_build_receipt_sha256 = $buildReceiptSha256
        source_revision = $ExpectedSourceRevision
        source_tree = $ExpectedSourceTree
        canonical_pe_payload_sha256 = $canonicalPayload.sha256
        canonical_pe_payload_size_bytes = $canonicalPayload.size_bytes
        descriptor_signer_spki_sha256 = $spkiSha256
        descriptor_signer_spki_asset = "release-descriptor.spki.der"
        payload_sha256 = [Convert]::ToHexString(
            [Security.Cryptography.SHA256]::HashData($payloadBytes)
        ).ToLowerInvariant()
        descriptor_sha256 = Get-RustyFleetSha256 -LiteralPath $descriptorPath
        canonical_payload = "rfc8785_jcs_closed_shape"
        signature = "rsa_pss_sha256"
        pages_path = "Rusty-Fleet/metadata/$Channel/release.json"
        asset_url = $assetUrl
    }
    Write-RustyFleetUtf8 `
        -LiteralPath $receiptPath `
        -Content (ConvertTo-RustyFleetJson -InputObject $receipt)
    $receipt | ConvertTo-Json -Depth 10
}
finally {
    $rsa.Dispose()
    if ($privateKeyText) {
        $privateKeyText = $null
    }
}
}
finally {
    if ($setupHandle) {
        $setupHandle.Dispose()
    }
    if ($setupChain) {
        $setupChain.Dispose()
    }
}
