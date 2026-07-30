# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet("Preflight", "Publish")]
    [string] $Mode,

    [Parameter(Mandatory)]
    [string] $AssetDirectory,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9]+\.[0-9]+\.[0-9]+$")]
    [string] $Version,

    [Parameter(Mandatory)]
    [ValidateSet("dev", "alpha", "preview", "stable")]
    [string] $Channel,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9A-Fa-f]{40}$")]
    [string] $ExpectedFleetSignerThumbprint,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9A-Fa-f]{40,64}$")]
    [string] $ExpectedHostessSignerThumbprint,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9a-f]{64}$")]
    [string] $ExpectedDescriptorSignerSpkiSha256,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9a-f]{40}$")]
    [string] $ExpectedSourceRevision,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9a-f]{40}$")]
    [string] $ExpectedSourceTree,

    [Parameter(Mandatory)]
    [string] $RepositoryRoot,

    [ValidatePattern("^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")]
    [string] $GitHubRepository = "MesmerPrism/rusty-fleet",

    [string] $ExpectedRef,

    [string] $GhExecutable = "gh"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
Import-Module (Join-Path $PSScriptRoot "Distribution.Common.psm1") -Force
Import-Module (Join-Path $PSScriptRoot "Publication.Remote.psm1") -Force

function Assert-ExactProperties {
    param(
        [Parameter(Mandatory)][object] $InputObject,
        [Parameter(Mandatory)][string[]] $Expected,
        [Parameter(Mandatory)][string] $Context
    )

    if ($null -eq $InputObject) {
        throw "$Context is missing"
    }
    $actual = @($InputObject.PSObject.Properties.Name | Sort-Object)
    if (@(Compare-Object ($Expected | Sort-Object) $actual).Count -ne 0) {
        throw "$Context has missing or unknown fields"
    }
}

function ConvertFrom-StrictUtf8Json {
    param(
        [Parameter(Mandatory)][byte[]] $Bytes,
        [Parameter(Mandatory)][int] $MaximumBytes,
        [Parameter(Mandatory)][string] $Context
    )

    if ($Bytes.Length -le 0 -or $Bytes.Length -gt $MaximumBytes) {
        throw "$Context size is outside its contract"
    }
    try {
        $text = [Text.UTF8Encoding]::new($false, $true).GetString($Bytes)
        if ($text.Length -gt 0 -and $text[0] -eq [char] 0xfeff) {
            throw "bom"
        }
        return $text | ConvertFrom-Json -Depth 40
    }
    catch {
        throw "$Context is not strict UTF-8 JSON"
    }
}

function ConvertFrom-Base64Url {
    param(
        [Parameter(Mandatory)][string] $Value,
        [Parameter(Mandatory)][string] $Context
    )

    if ($Value -cnotmatch "^[A-Za-z0-9_-]+$") {
        throw "$Context is not canonical base64url"
    }
    $padded = $Value.Replace("-", "+").Replace("_", "/")
    $padded += "=" * ((4 - ($padded.Length % 4)) % 4)
    try {
        [byte[]] $bytes = [Convert]::FromBase64String($padded)
    }
    catch {
        throw "$Context is not valid base64url"
    }
    $roundTrip = [Convert]::ToBase64String($bytes).
        TrimEnd("=").
        Replace("+", "-").
        Replace("/", "_")
    if ($roundTrip -cne $Value) {
        throw "$Context is not canonical base64url"
    }
    return $bytes
}

function Get-BytesSha256 {
    param([Parameter(Mandatory)][byte[]] $Bytes)

    [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData($Bytes)
    ).ToLowerInvariant()
}

function Assert-FixedTimeBytes {
    param(
        [Parameter(Mandatory)][byte[]] $Expected,
        [Parameter(Mandatory)][byte[]] $Observed,
        [Parameter(Mandatory)][string] $Context
    )

    if ($Expected.Length -ne $Observed.Length -or
        -not [Security.Cryptography.CryptographicOperations]::FixedTimeEquals(
            $Expected,
            $Observed
        )) {
        throw "$Context is not byte-exact"
    }
}

function Invoke-GitText {
    param(
        [Parameter(Mandatory)][string[]] $Arguments,
        [Parameter(Mandatory)][string] $Context,
        [switch] $AllowFailure
    )

    $output = @(& git -C $script:repoPath @Arguments 2>$null)
    $exitCode = $LASTEXITCODE
    if (-not $AllowFailure -and $exitCode -ne 0) {
        throw "$Context failed"
    }
    [pscustomobject]@{
        exit_code = $exitCode
        lines = $output
        text = ($output -join "`n").Trim()
    }
}

if (-not ("RustyFleet.Publication.RetainedAssetSet" -as [type])) {
    Add-Type -TypeDefinition @"
using Microsoft.Win32.SafeHandles;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Security.Cryptography;

namespace RustyFleet.Publication
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

        public override int GetHashCode() => HashCode.Combine(Volume, File);
    }

    public sealed class RetainedAssetSet : IDisposable
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
        private const long MaximumAssetBytes = 2L * 1024 * 1024 * 1024;

        private readonly string root;
        private readonly string[] expectedNames;
        private readonly List<string> directoryPaths = new();
        private readonly List<PathIdentity> directoryIdentities = new();
        private readonly List<SafeFileHandle> directoryHandles = new();
        private readonly Dictionary<string, RetainedFile> files =
            new(StringComparer.Ordinal);

        private RetainedAssetSet(string root, string[] expectedNames)
        {
            this.root = root;
            this.expectedNames = expectedNames;
        }

        public static RetainedAssetSet Open(string rootPath, string[] names)
        {
            var fullRoot = Path.GetFullPath(rootPath);
            if (!Path.IsPathFullyQualified(fullRoot) ||
                fullRoot.StartsWith(@"\\", StringComparison.Ordinal) ||
                fullRoot.StartsWith(@"\\?\", StringComparison.Ordinal) ||
                fullRoot.StartsWith(@"\??\", StringComparison.Ordinal))
            {
                throw new InvalidDataException(
                    "publication root must be a normal local absolute path");
            }
            var volumeRoot = Path.GetPathRoot(fullRoot)
                ?? throw new InvalidDataException("publication volume is unavailable");
            if (new DriveInfo(volumeRoot).DriveType != DriveType.Fixed)
            {
                throw new InvalidDataException(
                    "publication inputs must be on a fixed local volume");
            }
            var expected = names.OrderBy(
                name => name,
                StringComparer.Ordinal).ToArray();
            if (expected.Length == 0 ||
                expected.Distinct(StringComparer.Ordinal).Count() != expected.Length ||
                expected.Any(name =>
                    string.IsNullOrWhiteSpace(name) ||
                    name != Path.GetFileName(name) ||
                    name.Contains('/') ||
                    name.Contains('\\')))
            {
                throw new InvalidDataException(
                    "publication expected-name inventory is invalid");
            }
            if (!Directory.Exists(fullRoot))
            {
                throw new DirectoryNotFoundException(
                    "publication input directory is missing");
            }

            var result = new RetainedAssetSet(fullRoot, expected);
            try
            {
                result.RetainDirectoryChain();
                result.AssertClosedInventory();
                foreach (var name in expected)
                {
                    result.RetainFile(name);
                }
                result.AssertClosedInventory();
                result.Verify();
                return result;
            }
            catch
            {
                result.Dispose();
                throw;
            }
        }

        public string[] Names => expectedNames.ToArray();

        public string PathFor(string name) => Required(name).Path;

        public long Length(string name)
        {
            var file = Required(name);
            Inspect(file.Stream.SafeFileHandle, file.Path, true, file.Identity);
            return file.Stream.Length;
        }

        public byte[] ReadAll(string name, int maximumBytes)
        {
            var file = Required(name);
            Inspect(file.Stream.SafeFileHandle, file.Path, true, file.Identity);
            if (file.Stream.Length <= 0 || file.Stream.Length > maximumBytes)
            {
                throw new InvalidDataException(
                    "retained publication input size is outside its contract");
            }
            file.Stream.Position = 0;
            var bytes = new byte[checked((int)file.Stream.Length)];
            file.Stream.ReadExactly(bytes);
            file.Stream.Position = 0;
            return bytes;
        }

        public string Sha256(string name)
        {
            var file = Required(name);
            Inspect(file.Stream.SafeFileHandle, file.Path, true, file.Identity);
            file.Stream.Position = 0;
            var hash = SHA256.HashData(file.Stream);
            file.Stream.Position = 0;
            return Convert.ToHexString(hash).ToLowerInvariant();
        }

        public void CopyTo(string name, string destination)
        {
            var file = Required(name);
            Inspect(file.Stream.SafeFileHandle, file.Path, true, file.Identity);
            using var output = new FileStream(
                destination,
                FileMode.CreateNew,
                FileAccess.Write,
                FileShare.None,
                1024 * 1024,
                FileOptions.WriteThrough);
            file.Stream.Position = 0;
            file.Stream.CopyTo(output);
            output.Flush(flushToDisk: true);
            file.Stream.Position = 0;
        }

        public void Verify()
        {
            AssertClosedInventory();
            for (var index = 0; index < directoryPaths.Count; index++)
            {
                Inspect(
                    directoryHandles[index],
                    directoryPaths[index],
                    false,
                    directoryIdentities[index]);
                using var reopened = OpenDirectory(
                    directoryPaths[index],
                    ShareRead | ShareWrite | ShareDelete);
                Inspect(
                    reopened,
                    directoryPaths[index],
                    false,
                    directoryIdentities[index]);
            }
            foreach (var file in files.Values)
            {
                Inspect(
                    file.Stream.SafeFileHandle,
                    file.Path,
                    true,
                    file.Identity);
                using var reopened = CreateFileW(
                    ToNativePath(file.Path),
                    GenericRead,
                    ShareRead | ShareWrite | ShareDelete,
                    IntPtr.Zero,
                    OpenExisting,
                    OpenReparsePoint,
                    IntPtr.Zero);
                if (reopened.IsInvalid)
                {
                    ThrowLastIo("could not reopen retained publication input");
                }
                Inspect(reopened, file.Path, true, file.Identity);
            }
        }

        public void Dispose()
        {
            foreach (var file in files.Values)
            {
                file.Stream.Dispose();
            }
            files.Clear();
            for (var index = directoryHandles.Count - 1; index >= 0; index--)
            {
                directoryHandles[index].Dispose();
            }
            directoryHandles.Clear();
        }

        private void RetainDirectoryChain()
        {
            var ordered = new Stack<string>();
            for (var current = new DirectoryInfo(root);
                 current != null;
                 current = current.Parent)
            {
                ordered.Push(current.FullName);
            }
            while (ordered.Count > 0)
            {
                var path = Path.GetFullPath(ordered.Pop());
                var handle = OpenDirectory(path, ShareRead);
                try
                {
                    var identity = Inspect(handle, path, false, null);
                    directoryPaths.Add(path);
                    directoryIdentities.Add(identity);
                    directoryHandles.Add(handle);
                }
                catch
                {
                    handle.Dispose();
                    throw;
                }
            }
        }

        private void RetainFile(string name)
        {
            var path = Path.GetFullPath(Path.Combine(root, name));
            var handle = CreateFileW(
                ToNativePath(path),
                GenericRead,
                ShareRead,
                IntPtr.Zero,
                OpenExisting,
                OpenReparsePoint,
                IntPtr.Zero);
            if (handle.IsInvalid)
            {
                ThrowLastIo("could not retain exact publication input");
            }
            var stream = new FileStream(handle, FileAccess.Read);
            try
            {
                var identity = Inspect(handle, path, true, null);
                if (stream.Length <= 0 || stream.Length > MaximumAssetBytes ||
                    files.Values.Any(existing =>
                        existing.Identity.Equals(identity)))
                {
                    throw new InvalidDataException(
                        "publication input size or identity is invalid");
                }
                files.Add(name, new RetainedFile(path, identity, stream));
            }
            catch
            {
                stream.Dispose();
                throw;
            }
        }

        private void AssertClosedInventory()
        {
            var directories = Directory.EnumerateDirectories(root).ToArray();
            if (directories.Length != 0)
            {
                throw new InvalidDataException(
                    "publication input directory contains an unexpected directory");
            }
            var observed = Directory.EnumerateFiles(root)
                .Select(Path.GetFileName)
                .OrderBy(name => name, StringComparer.Ordinal)
                .ToArray();
            if (!observed.SequenceEqual(expectedNames, StringComparer.Ordinal))
            {
                throw new InvalidDataException(
                    "publication input inventory has additions, omissions, or casing changes");
            }
        }

        private RetainedFile Required(string name)
        {
            if (!files.TryGetValue(name, out var file))
            {
                throw new InvalidDataException(
                    "publication input name is outside the closed inventory");
            }
            return file;
        }

        private static SafeFileHandle OpenDirectory(string path, uint share)
        {
            var handle = CreateFileW(
                ToNativePath(path),
                FileReadAttributes,
                share,
                IntPtr.Zero,
                OpenExisting,
                BackupSemantics | OpenReparsePoint,
                IntPtr.Zero);
            if (handle.IsInvalid)
            {
                ThrowLastIo("could not retain publication directory");
            }
            return handle;
        }

        private static PathIdentity Inspect(
            SafeFileHandle handle,
            string path,
            bool requireSingleLink,
            PathIdentity? expected)
        {
            if (!GetFileInformationByHandle(handle, out var information))
            {
                ThrowLastIo("could not inspect retained publication path");
            }
            if ((information.FileAttributes & ReparsePoint) != 0 ||
                (requireSingleLink && information.NumberOfLinks != 1))
            {
                throw new InvalidDataException(
                    "publication reparse points and hard links are forbidden");
            }
            var identity = new PathIdentity(
                information.VolumeSerialNumber,
                ((ulong)information.FileIndexHigh << 32) |
                    information.FileIndexLow);
            if (expected.HasValue && !expected.Value.Equals(identity))
            {
                throw new InvalidDataException(
                    "retained publication path identity changed");
            }
            return identity;
        }

        private static string ToNativePath(string path)
        {
            var full = Path.GetFullPath(path);
            return full.StartsWith(@"\\?\", StringComparison.Ordinal)
                ? full
                : @"\\?\" + full;
        }

        private static void ThrowLastIo(string message)
        {
            var error = Marshal.GetLastWin32Error();
            var native = new System.ComponentModel.Win32Exception(error);
            throw new IOException(
                $"{message}: Win32 {error}: {native.Message}",
                native);
        }

        private sealed class RetainedFile
        {
            public RetainedFile(
                string path,
                PathIdentity identity,
                FileStream stream)
            {
                Path = path;
                Identity = identity;
                Stream = stream;
            }

            public string Path { get; }
            public PathIdentity Identity { get; }
            public FileStream Stream { get; }
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct ByHandleFileInformation
        {
            public uint FileAttributes;
            public System.Runtime.InteropServices.ComTypes.FILETIME CreationTime;
            public System.Runtime.InteropServices.ComTypes.FILETIME LastAccessTime;
            public System.Runtime.InteropServices.ComTypes.FILETIME LastWriteTime;
            public uint VolumeSerialNumber;
            public uint FileSizeHigh;
            public uint FileSizeLow;
            public uint NumberOfLinks;
            public uint FileIndexHigh;
            public uint FileIndexLow;
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
            out ByHandleFileInformation information);
    }
}
"@
}

$tag = "v$Version"
if (-not $ExpectedRef) {
    $ExpectedRef = "refs/tags/$tag"
}
if ($ExpectedRef -cne "refs/tags/$tag") {
    throw "publication requires the exact version tag ref"
}
$repoPath = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$script:repoPath = $repoPath
$assetRoot = (Resolve-Path -LiteralPath $AssetDirectory).Path
$productStem = if ($Channel -eq "alpha") { "RustyFleet-Alpha" } else { "RustyFleet" }
$bundleName = "$productStem-$Version-win-x64"
$setupName = if ($Channel -eq "alpha") { "RustyFleet-Alpha-Setup.exe" } else { "RustyFleet-Setup.exe" }
$zipName = "$bundleName.zip"
$zipSidecarName = "$zipName.sha256"
$manifestName = "$bundleName.manifest.json"
$checksumsName = "$bundleName.checksums.sha256"
$validationReceiptName = "$bundleName.validation-receipt.json"
$setupBuildReceiptName = if ($Channel -eq "alpha") { "RustyFleet-Alpha-Setup.build-receipt.json" } else { "RustyFleet-Setup.build-receipt.json" }
$releaseJsonName = "release.json"
$descriptorReceiptName = "release-descriptor.receipt.json"
$spkiName = "release-descriptor.spki.der"
$policyName = "release-policy.json"
$publishedNames = @(
    $setupName,
    $zipName,
    $zipSidecarName,
    $manifestName,
    $checksumsName,
    $validationReceiptName,
    $setupBuildReceiptName,
    $releaseJsonName,
    $descriptorReceiptName,
    $spkiName
) | Sort-Object
$inputNames = @($publishedNames + $policyName) | Sort-Object
$assets = $null
$inspectionRoot = $null

function Assert-SourceAndPolicy {
    param(
        [Parameter(Mandatory)]
        [RustyFleet.Publication.RetainedAssetSet] $RetainedAssets
    )

    $RetainedAssets.Verify()
    $head = Invoke-GitText -Arguments @("rev-parse", "HEAD") `
        -Context "release HEAD resolution"
    $tagCommit = Invoke-GitText -Arguments @(
        "rev-parse",
        "$tag^{commit}"
    ) -Context "release tag resolution"
    $tree = Invoke-GitText -Arguments @(
        "rev-parse",
        "$ExpectedSourceRevision^{tree}"
    ) -Context "release tree resolution"
    $tagRef = Invoke-GitText -Arguments @(
        "show-ref",
        "--verify",
        "--hash",
        "refs/tags/$tag"
    ) -Context "release tag presence"
    $dirt = Invoke-GitText -Arguments @(
        "status",
        "--porcelain=v1",
        "--untracked-files=no"
    ) -Context "release worktree state"
    if ($head.text -cne $ExpectedSourceRevision -or
        $tagCommit.text -cne $ExpectedSourceRevision -or
        $tagRef.text -cnotmatch "^[0-9a-f]{40}$" -or
        $tree.text -cne $ExpectedSourceTree -or
        @($dirt.lines).Count -ne 0) {
        throw "release tag, revision, tree, or tracked worktree changed"
    }

    $tagPolicyBlob = Invoke-GitText -Arguments @(
        "rev-parse",
        "$ExpectedSourceRevision`:packaging/windows/trust/release-policy.json"
    ) -Context "tagged release policy resolution"
    $stagedPolicyPath = $RetainedAssets.PathFor($policyName)
    $stagedPolicyBlob = Invoke-GitText -Arguments @(
        "hash-object",
        "--no-filters",
        $stagedPolicyPath
    ) -Context "staged release policy identity"
    if ($tagPolicyBlob.text -cne $stagedPolicyBlob.text) {
        throw "staged release policy is not byte-exact to the tagged policy"
    }
    $policy = ConvertFrom-StrictUtf8Json `
        -Bytes ($RetainedAssets.ReadAll($policyName, 65536)) `
        -MaximumBytes 65536 `
        -Context "release trust policy"
    Assert-ExactProperties -InputObject $policy -Expected @(
        "schema",
        "publication_enabled",
        "authorized_fleet_signer_thumbprints",
        "authorized_hostess_signer_thumbprints",
        "authorized_descriptor_signer_spki_sha256",
        "status"
    ) -Context "release trust policy"
    if ($policy.schema -cne
            "rusty.fleet.windows_release_trust_policy.v1" -or
        $policy.publication_enabled -ne $true -or
        @($policy.authorized_fleet_signer_thumbprints) -cnotcontains
            $ExpectedFleetSignerThumbprint.ToUpperInvariant() -or
        @($policy.authorized_hostess_signer_thumbprints) -cnotcontains
            $ExpectedHostessSignerThumbprint.ToUpperInvariant() -or
        @($policy.authorized_descriptor_signer_spki_sha256) -cnotcontains
            $ExpectedDescriptorSignerSpkiSha256) {
        throw "tagged release policy does not authorize publication inputs"
    }
    $RetainedAssets.Verify()
}

try {
    $assets = [RustyFleet.Publication.RetainedAssetSet]::Open(
        $assetRoot,
        $inputNames
    )
    Assert-SourceAndPolicy -RetainedAssets $assets

    $buildReceiptBytes = $assets.ReadAll($setupBuildReceiptName, 65536)
    $buildReceipt = ConvertFrom-StrictUtf8Json `
        -Bytes $buildReceiptBytes `
        -MaximumBytes 65536 `
        -Context "Setup build receipt"
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
    if ($buildReceipt.schema -cne
            "rusty.fleet.windows_setup_build_receipt.v1" -or
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
        throw "Setup build receipt is not exact signed-release evidence"
    }

    $setupPath = $assets.PathFor($setupName)
    $setupSha256 = $assets.Sha256($setupName)
    $canonicalSetup = Get-RustyFleetPeCanonicalPayload `
        -LiteralPath $setupPath `
        -ExpectedPayloadSize (
            [long] $buildReceipt.canonical_pe_payload_size_bytes
        )
    $assets.Verify()
    if ($canonicalSetup.sha256 -cne
            $buildReceipt.canonical_pe_payload_sha256 -or
        $canonicalSetup.size_bytes -ne
            [long] $buildReceipt.canonical_pe_payload_size_bytes -or
        $buildReceipt.setup_sha256 -cne
            $buildReceipt.canonical_pe_payload_sha256) {
        throw "signed Setup does not match its canonical pre-sign receipt"
    }

    $zipSha256 = $assets.Sha256($zipName)
    $expectedSidecarBytes = [Text.Encoding]::UTF8.GetBytes(
        "$zipSha256 *$zipName`n"
    )
    Assert-FixedTimeBytes `
        -Expected $expectedSidecarBytes `
        -Observed ($assets.ReadAll($zipSidecarName, 1024)) `
        -Context "ZIP SHA-256 sidecar"
    if ($buildReceipt.bundle_sha256 -cne $zipSha256) {
        throw "Setup build receipt does not bind the exact ZIP"
    }

    $inspectionRoot = Join-Path ([IO.Path]::GetTempPath()) (
        "rusty-fleet-publication-$([Guid]::NewGuid().ToString('N'))"
    )
    [IO.Directory]::CreateDirectory($inspectionRoot) | Out-Null
    $zipCopy = Join-Path $inspectionRoot $zipName
    $assets.CopyTo($zipName, $zipCopy)
    if ((Get-RustyFleetSha256 -LiteralPath $zipCopy) -cne $zipSha256) {
        throw "retained ZIP copy changed before validation"
    }
    $extracted = Join-Path $inspectionRoot "extracted"
    [IO.Compression.ZipFile]::ExtractToDirectory($zipCopy, $extracted)
    $extractedEntries = @(Get-ChildItem -LiteralPath $extracted)
    $bundleRoot = Join-Path $extracted $bundleName
    if ($extractedEntries.Count -ne 1 -or
        -not (Test-Path -LiteralPath $bundleRoot -PathType Container)) {
        throw "ZIP top-level inventory is not exact"
    }

    $innerManifestPath = Join-Path $bundleRoot "metadata\release-manifest.json"
    $innerChecksumsPath = Join-Path $bundleRoot "metadata\checksums.sha256"
    $innerValidationPath = Join-Path $bundleRoot (
        "metadata\validation-receipt.json"
    )
    foreach ($pair in @(
        @($manifestName, $innerManifestPath, "release manifest"),
        @($checksumsName, $innerChecksumsPath, "payload checksums"),
        @($validationReceiptName, $innerValidationPath, "validation receipt")
    )) {
        Assert-FixedTimeBytes `
            -Expected ([IO.File]::ReadAllBytes($pair[1])) `
            -Observed ($assets.ReadAll($pair[0], 16 * 1024 * 1024)) `
            -Context $pair[2]
    }
    & (Join-Path $PSScriptRoot "Test-WindowsBundle.ps1") `
        -BundleRoot $bundleRoot `
        -ExpectedVersion $Version `
        -ExpectedFleetSignerThumbprint $ExpectedFleetSignerThumbprint `
        -ExpectedHostessSignerThumbprint $ExpectedHostessSignerThumbprint |
        Out-Null
    $assets.Verify()

    $manifestBytes = $assets.ReadAll($manifestName, 16 * 1024 * 1024)
    if ((Get-BytesSha256 $manifestBytes) -cne
        $buildReceipt.manifest_sha256) {
        throw "Setup build receipt does not bind the exact inner manifest"
    }
    $manifest = ConvertFrom-StrictUtf8Json `
        -Bytes $manifestBytes `
        -MaximumBytes (16 * 1024 * 1024) `
        -Context "release manifest"
    if ($manifest.version -cne $Version -or
        $manifest.channel -cne $Channel -or
        $manifest.build.kind -cne "signed-release" -or
        $manifest.build.source_tree_clean -ne $true -or
        $manifest.source.revision -cne $ExpectedSourceRevision -or
        $manifest.source.tree -cne $ExpectedSourceTree -or
        $manifest.distribution.publication_allowed -ne $true -or
        $manifest.distribution.eligibility -cne "signed_release") {
        throw "inner manifest is not exact signed-release provenance"
    }

    $validationReceipt = ConvertFrom-StrictUtf8Json `
        -Bytes ($assets.ReadAll($validationReceiptName, 1048576)) `
        -MaximumBytes 1048576 `
        -Context "distribution validation receipt"
    Assert-ExactProperties -InputObject $validationReceipt -Expected @(
        "schema",
        "result",
        "version",
        "source_revision",
        "manifest_sha256",
        "checksums_sha256",
        "payload_files",
        "runtime_components",
        "exact_external_provider",
        "hostess_owner_provenance_sha256",
        "distribution_eligibility",
        "publication_allowed",
        "credentials_absent",
        "private_configuration_absent",
        "adb_absent",
        "signatures"
    ) -Context "distribution validation receipt"
    if ($validationReceipt.result -cne "pass" -or
        $validationReceipt.version -cne $Version -or
        $validationReceipt.source_revision -cne $ExpectedSourceRevision -or
        $validationReceipt.manifest_sha256 -cne
            $buildReceipt.manifest_sha256 -or
        $validationReceipt.distribution_eligibility -cne "signed_release" -or
        $validationReceipt.publication_allowed -ne $true -or
        $validationReceipt.signatures -cne "verified") {
        throw "distribution validation receipt is not publication evidence"
    }

    $setupSignature = Get-AuthenticodeSignature -LiteralPath $setupPath
    $assets.Verify()
    if ($setupSignature.Status -ne
            [Management.Automation.SignatureStatus]::Valid -or
        $null -eq $setupSignature.SignerCertificate -or
        $setupSignature.SignerCertificate.Thumbprint -cne
            $ExpectedFleetSignerThumbprint.ToUpperInvariant()) {
        throw "Setup Authenticode signer is not independently authorized"
    }

    $spkiBytes = $assets.ReadAll($spkiName, 65536)
    if ((Get-BytesSha256 $spkiBytes) -cne
        $ExpectedDescriptorSignerSpkiSha256) {
        throw "descriptor SPKI companion does not match independent authorization"
    }
    $descriptorBytes = $assets.ReadAll($releaseJsonName, 65536)
    $envelope = ConvertFrom-StrictUtf8Json `
        -Bytes $descriptorBytes `
        -MaximumBytes 65536 `
        -Context "release descriptor"
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
        throw "release descriptor envelope identity is not exact"
    }
    $payloadBytes = ConvertFrom-Base64Url `
        -Value $envelope.payload_base64url `
        -Context "release descriptor payload"
    $signatureBytes = ConvertFrom-Base64Url `
        -Value $envelope.signature_base64url `
        -Context "release descriptor signature"
    $payload = ConvertFrom-StrictUtf8Json `
        -Bytes $payloadBytes `
        -MaximumBytes 32768 `
        -Context "release descriptor payload"
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
    $expectedAssetUrl = (
        "https://github.com/$GitHubRepository/releases/download/" +
        "$tag/$setupName"
    )
    if ($payload.schema -cne "rusty.fleet.windows_release.v2" -or
        $payload.product -cne "rusty-fleet" -or
        $payload.version -cne $Version -or
        $payload.channel -cne $Channel -or
        $payload.descriptor_id -cnotmatch "^[A-Za-z0-9._-]{1,128}$" -or
        [long] $payload.issued_at_ms -le 0 -or
        [long] $payload.expires_at_ms -le
            [long] $payload.issued_at_ms -or
        [long] $payload.validity_duration_ms -ne
            ([long] $payload.expires_at_ms -
                [long] $payload.issued_at_ms) -or
        [long] $payload.validity_duration_ms -gt 86400000 -or
        $payload.asset.installer_protocol -cne
            "rusty.fleet.guided_setup.v1" -or
        $payload.asset.media_type -cne
            "application/vnd.microsoft.portable-executable" -or
        $payload.asset.name -cne $setupName -or
        $payload.asset.sha256 -cne $setupSha256 -or
        [long] $payload.asset.size_bytes -ne $assets.Length($setupName) -or
        $payload.asset.url -cne $expectedAssetUrl) {
        throw "release descriptor payload is not bound to publication inputs"
    }
    $nowMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    if ([long] $payload.issued_at_ms -gt $nowMs + 300000 -or
        [long] $payload.expires_at_ms -le $nowMs) {
        throw "release descriptor is not currently fresh"
    }
    $setupCertificateSha256 = [Convert]::ToHexString(
        [Security.Cryptography.SHA256]::HashData(
            $setupSignature.SignerCertificate.RawData
        )
    ).ToLowerInvariant()
    if ($payload.asset.signer_certificate_sha256 -cne
        $setupCertificateSha256) {
        throw "descriptor does not bind the exact Setup signer certificate"
    }
    $jcsText = (
        '{"asset":{' +
        '"installer_protocol":"rusty.fleet.guided_setup.v1",' +
        '"media_type":"application/vnd.microsoft.portable-executable",' +
        '"name":"' + $setupName + '",' +
        '"sha256":"' + $setupSha256 + '",' +
        '"signer_certificate_sha256":"' + $setupCertificateSha256 + '",' +
        '"size_bytes":' + [long] $payload.asset.size_bytes + ',' +
        '"url":"' + $expectedAssetUrl + '"},' +
        '"channel":"' + $Channel + '",' +
        '"descriptor_id":"' + $payload.descriptor_id + '",' +
        '"expires_at_ms":' + [long] $payload.expires_at_ms + ',' +
        '"issued_at_ms":' + [long] $payload.issued_at_ms + ',' +
        '"product":"rusty-fleet",' +
        '"schema":"rusty.fleet.windows_release.v2",' +
        '"validity_duration_ms":' +
            [long] $payload.validity_duration_ms + ',' +
        '"version":"' + $Version + '"}'
    )
    Assert-FixedTimeBytes `
        -Expected ([Text.Encoding]::UTF8.GetBytes($jcsText)) `
        -Observed $payloadBytes `
        -Context "RFC 8785 closed release payload"
    $publicKey = [Security.Cryptography.RSA]::Create()
    try {
        $bytesRead = 0
        $publicKey.ImportSubjectPublicKeyInfo($spkiBytes, [ref] $bytesRead)
        if ($bytesRead -ne $spkiBytes.Length -or
            $publicKey.KeySize -lt 3072 -or
            -not $publicKey.VerifyData(
                $payloadBytes,
                $signatureBytes,
                [Security.Cryptography.HashAlgorithmName]::SHA256,
                [Security.Cryptography.RSASignaturePadding]::Pss
            )) {
            throw "release descriptor RSA-PSS signature is invalid"
        }
    }
    finally {
        $publicKey.Dispose()
    }

    $descriptorReceiptBytes = $assets.ReadAll(
        $descriptorReceiptName,
        65536
    )
    $descriptorReceipt = ConvertFrom-StrictUtf8Json `
        -Bytes $descriptorReceiptBytes `
        -MaximumBytes 65536 `
        -Context "release descriptor receipt"
    Assert-ExactProperties -InputObject $descriptorReceipt -Expected @(
        "schema",
        "result",
        "descriptor_id",
        "version",
        "channel",
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
    if ($descriptorReceipt.schema -cne
            "rusty.fleet.windows_release_descriptor_receipt.v2" -or
        $descriptorReceipt.result -cne "pass" -or
        $descriptorReceipt.descriptor_id -cne $payload.descriptor_id -or
        $descriptorReceipt.version -cne $Version -or
        $descriptorReceipt.channel -cne $Channel -or
        [long] $descriptorReceipt.issued_at_ms -ne
            [long] $payload.issued_at_ms -or
        [long] $descriptorReceipt.expires_at_ms -ne
            [long] $payload.expires_at_ms -or
        [long] $descriptorReceipt.validity_duration_ms -ne
            [long] $payload.validity_duration_ms -or
        $descriptorReceipt.setup_sha256 -cne $setupSha256 -or
        [long] $descriptorReceipt.setup_size_bytes -ne
            $assets.Length($setupName) -or
        $descriptorReceipt.setup_signer_certificate_sha256 -cne
            $setupCertificateSha256 -or
        $descriptorReceipt.setup_build_receipt_sha256 -cne
            (Get-BytesSha256 $buildReceiptBytes) -or
        $descriptorReceipt.source_revision -cne $ExpectedSourceRevision -or
        $descriptorReceipt.source_tree -cne $ExpectedSourceTree -or
        $descriptorReceipt.canonical_pe_payload_sha256 -cne
            $canonicalSetup.sha256 -or
        [long] $descriptorReceipt.canonical_pe_payload_size_bytes -ne
            $canonicalSetup.size_bytes -or
        $descriptorReceipt.descriptor_signer_spki_sha256 -cne
            $ExpectedDescriptorSignerSpkiSha256 -or
        $descriptorReceipt.descriptor_signer_spki_asset -cne $spkiName -or
        $descriptorReceipt.payload_sha256 -cne
            (Get-BytesSha256 $payloadBytes) -or
        $descriptorReceipt.descriptor_sha256 -cne
            (Get-BytesSha256 $descriptorBytes) -or
        $descriptorReceipt.canonical_payload -cne
            "rfc8785_jcs_closed_shape" -or
        $descriptorReceipt.signature -cne "rsa_pss_sha256" -or
        $descriptorReceipt.pages_path -cne
            "Rusty-Fleet/metadata/$Channel/release.json" -or
        $descriptorReceipt.asset_url -cne $expectedAssetUrl) {
        throw "release descriptor receipt is not exact hash-bound evidence"
    }

    $assets.Verify()
    Assert-SourceAndPolicy -RetainedAssets $assets
    $publicationAssets = @($publishedNames | ForEach-Object {
        [pscustomobject][ordered]@{
            name = $_
            sha256 = $assets.Sha256($_)
            size_bytes = $assets.Length($_)
            path = $assets.PathFor($_)
        }
    })
    $inventory = @($publicationAssets | ForEach-Object {
        [ordered]@{
            name = $_.name
            sha256 = $_.sha256
            size_bytes = $_.size_bytes
        }
    })
    $draftVerified = $false
    $visibleVerified = $false
    $remoteTagVerified = $false
    $remoteIntegrityVerified = $false
    $resumedDraft = $false
    $uploadedAssetCount = 0
    $tokenUsed = $false
    $ghInvoked = $false

    if ($Mode -eq "Publish") {
        Assert-SourceAndPolicy -RetainedAssets $assets
        if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable(
            "GH_TOKEN",
            [EnvironmentVariableTarget]::Process
        ))) {
            throw "publication requires the dedicated release token"
        }
        $tokenUsed = $true
        $ghInvoked = $true
        $localGuard = {
            $assets.Verify()
            Assert-SourceAndPolicy -RetainedAssets $assets
        }.GetNewClosure()
        $remoteResult = Publish-RustyFleetGitHubRelease `
            -GitHubRepository $GitHubRepository `
            -Tag $tag `
            -ExpectedSourceRevision $ExpectedSourceRevision `
            -AssetInventory $publicationAssets `
            -Prerelease ($Channel -cne "stable") `
            -GhExecutable $GhExecutable `
            -AssertLocalState $localGuard
        $draftVerified = $remoteResult.draft_verified
        $visibleVerified = $remoteResult.visible_verified
        $remoteTagVerified = $remoteResult.remote_tag_verified
        $remoteIntegrityVerified = $remoteResult.remote_integrity_verified
        $resumedDraft = $remoteResult.resumed_draft
        $uploadedAssetCount = $remoteResult.uploaded_asset_count
    }

    [ordered]@{
        schema = "rusty.fleet.windows_publication_receipt.v1"
        result = "pass"
        mode = $Mode.ToLowerInvariant()
        version = $Version
        channel = $Channel
        tag = $tag
        source_revision = $ExpectedSourceRevision
        source_tree = $ExpectedSourceTree
        setup_sha256 = $setupSha256
        bundle_sha256 = $zipSha256
        descriptor_sha256 = Get-BytesSha256 $descriptorBytes
        descriptor_receipt_sha256 = Get-BytesSha256 $descriptorReceiptBytes
        descriptor_signer_spki_sha256 =
            $ExpectedDescriptorSignerSpkiSha256
        asset_count = $inventory.Count
        assets = $inventory
        token_used = $tokenUsed
        gh_invoked = $ghInvoked
        draft_verified = $draftVerified
        visible_verified = $visibleVerified
        remote_tag_verified = $remoteTagVerified
        remote_integrity_verified = $remoteIntegrityVerified
        resumed_draft = $resumedDraft
        uploaded_asset_count = $uploadedAssetCount
    } | ConvertTo-Json -Depth 10
}
finally {
    if ($assets) {
        $assets.Dispose()
    }
    if ($inspectionRoot -and
        (Test-Path -LiteralPath $inspectionRoot)) {
        Remove-Item -LiteralPath $inspectionRoot -Recurse -Force
    }
}
