// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.IO.Compression;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.Win32.SafeHandles;

namespace RustyFleet.Setup;

internal enum SetupAction
{
    Install,
    Rollback,
}

internal sealed record InstallResult(
    string Schema,
    string Result,
    string Action,
    string Version,
    string ReleaseId,
    string ManifestSha256,
    string BundleSha256,
    bool RollbackAvailable,
    int ProcessesStarted,
    int ServicesRegistered,
    bool ConfigurationCreated,
    bool OnboardingInvoked);

internal sealed record InventoryItem(string Sha256, long SizeBytes);

internal sealed class ReleaseDefinition
{
    private const long MaximumExpandedBytes = 2L * 1024 * 1024 * 1024;
    private static readonly HashSet<string> ExpectedComponents = new(
        [
            "fleet-console",
            "fleet-hub",
            "fleetctl",
            "fleet-onboard",
            "hostess-hotspot-provider",
        ],
        StringComparer.Ordinal);

    private ReleaseDefinition(
        string version,
        string channel,
        string buildKind,
        string manifestSha256,
        string checksumsSha256,
        string receiptSha256,
        Dictionary<string, InventoryItem> payload,
        Dictionary<string, InventoryItem> allFiles)
    {
        Version = version;
        Channel = channel;
        BuildKind = buildKind;
        ManifestSha256 = manifestSha256;
        ChecksumsSha256 = checksumsSha256;
        ReceiptSha256 = receiptSha256;
        Payload = payload;
        AllFiles = allFiles;
    }

    public string Version { get; }
    public string Channel { get; }
    public string BuildKind { get; }
    public string ManifestSha256 { get; }
    public string ChecksumsSha256 { get; }
    public string ReceiptSha256 { get; }
    public IReadOnlyDictionary<string, InventoryItem> Payload { get; }
    public IReadOnlyDictionary<string, InventoryItem> AllFiles { get; }

    public static ReleaseDefinition Parse(
        byte[] manifestBytes,
        byte[] checksumsBytes,
        byte[] receiptBytes)
    {
        var manifestSha256 = Hash(manifestBytes);
        var checksumsSha256 = Hash(checksumsBytes);
        var receiptSha256 = Hash(receiptBytes);
        using var document = ParseJson(manifestBytes, "release manifest");
        var root = document.RootElement;
        RequireString(root, "schema", "rusty.fleet.windows_release_manifest.v1");
        RequireString(root, "product_id", "rusty-fleet");
        RequireString(root, "platform", "windows");
        RequireString(root, "architecture", "x64");
        var version = RequiredString(root, "version");
        var channel = RequiredString(root, "channel");
        if (channel is not ("dev" or "alpha" or "preview" or "stable"))
        {
            throw new InvalidDataException("release manifest channel is invalid");
        }
        var buildKind = RequiredString(root.GetProperty("build"), "kind");
        if (buildKind is not ("unsigned-dev" or "signed-release"))
        {
            throw new InvalidDataException("release manifest build kind is invalid");
        }
        var distribution = root.GetProperty("distribution");
        if (buildKind == "signed-release")
        {
            RequireString(distribution, "eligibility", "signed_release");
            RequireBoolean(distribution, "publication_allowed", true);
        }
        else
        {
            RequireString(distribution, "eligibility", "development_only");
            RequireBoolean(distribution, "publication_allowed", false);
        }
        RequireString(
            root.GetProperty("install"),
            "authority",
            channel == "alpha"
                ? "RustyFleet-Alpha-Setup.exe"
                : "RustyFleet-Setup.exe");

        var componentIds = root.GetProperty("components")
            .EnumerateArray()
            .Select(item => RequiredString(item, "component_id"))
            .ToArray();
        if (componentIds.Length != ExpectedComponents.Count ||
            componentIds.Distinct(StringComparer.Ordinal).Count() != componentIds.Length ||
            !componentIds.ToHashSet(StringComparer.Ordinal).SetEquals(ExpectedComponents))
        {
            throw new InvalidDataException("release manifest component composition is not exact");
        }

        var payload = new Dictionary<string, InventoryItem>(StringComparer.Ordinal);
        long expandedBytes = 0;
        foreach (var item in root.GetProperty("payload").EnumerateArray())
        {
            var path = RequiredString(item, "path");
            PathPolicy.ValidateRelative(path);
            var digest = RequiredString(item, "sha256");
            ValidateSha256(digest);
            var size = item.GetProperty("size_bytes").GetInt64();
            if (size < 0 || checked(expandedBytes += size) > MaximumExpandedBytes)
            {
                throw new InvalidDataException("release payload size is outside the limit");
            }
            if (!payload.TryAdd(path, new InventoryItem(digest, size)))
            {
                throw new InvalidDataException($"duplicate release payload path: {path}");
            }
        }

        var checksumEntries = ParseChecksums(checksumsBytes);
        var expectedChecksums = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var (path, item) in payload)
        {
            expectedChecksums.Add(path, item.Sha256);
        }
        expectedChecksums.Add("metadata/release-manifest.json", manifestSha256);
        if (checksumEntries.Count != expectedChecksums.Count ||
            expectedChecksums.Any(pair =>
                !checksumEntries.TryGetValue(pair.Key, out var digest) ||
                !FixedEqualsHex(pair.Value, digest)))
        {
            throw new InvalidDataException("release checksums do not bind the exact payload");
        }

        using var receipt = ParseJson(receiptBytes, "validation receipt");
        var receiptRoot = receipt.RootElement;
        RequireString(
            receiptRoot,
            "schema",
            "rusty.fleet.windows_distribution_validation_receipt.v1");
        RequireString(receiptRoot, "result", "pass");
        RequireString(receiptRoot, "version", version);
        RequireString(receiptRoot, "manifest_sha256", manifestSha256);
        RequireString(receiptRoot, "checksums_sha256", checksumsSha256);
        if (receiptRoot.GetProperty("payload_files").GetInt32() != payload.Count ||
            receiptRoot.GetProperty("runtime_components").GetInt32() !=
                ExpectedComponents.Count)
        {
            throw new InvalidDataException("validation receipt counts are not exact");
        }
        RequireString(
            receiptRoot,
            "distribution_eligibility",
            buildKind == "signed-release" ? "signed_release" : "development_only");
        RequireBoolean(
            receiptRoot,
            "publication_allowed",
            buildKind == "signed-release");

        var allFiles = new Dictionary<string, InventoryItem>(payload, StringComparer.Ordinal)
        {
            ["metadata/release-manifest.json"] =
                new(manifestSha256, manifestBytes.LongLength),
            ["metadata/checksums.sha256"] =
                new(checksumsSha256, checksumsBytes.LongLength),
            ["metadata/validation-receipt.json"] =
                new(receiptSha256, receiptBytes.LongLength),
        };
        return new ReleaseDefinition(
            version,
            channel,
            buildKind,
            manifestSha256,
            checksumsSha256,
            receiptSha256,
            payload,
            allFiles);
    }

    private static Dictionary<string, string> ParseChecksums(byte[] bytes)
    {
        var text = new UTF8Encoding(false, true).GetString(bytes);
        if (!text.EndsWith('\n') || text.Contains('\r'))
        {
            throw new InvalidDataException("release checksums are not canonical LF UTF-8");
        }
        var result = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var line in text.Split('\n', StringSplitOptions.RemoveEmptyEntries))
        {
            if (line.Length < 67 || line[64] != ' ' || line[65] != '*')
            {
                throw new InvalidDataException("release checksum line is malformed");
            }
            var digest = line[..64];
            var path = line[66..];
            ValidateSha256(digest);
            PathPolicy.ValidateRelative(path);
            if (!result.TryAdd(path, digest))
            {
                throw new InvalidDataException($"duplicate checksum path: {path}");
            }
        }
        return result;
    }

    internal static JsonDocument ParseJson(byte[] bytes, string description)
    {
        try
        {
            return JsonDocument.Parse(bytes, new JsonDocumentOptions
            {
                AllowTrailingCommas = false,
                CommentHandling = JsonCommentHandling.Disallow,
                MaxDepth = 32,
            });
        }
        catch (JsonException exception)
        {
            throw new InvalidDataException($"{description} is invalid JSON", exception);
        }
    }

    internal static string RequiredString(JsonElement owner, string name)
    {
        if (!owner.TryGetProperty(name, out var value) ||
            value.ValueKind != JsonValueKind.String ||
            string.IsNullOrEmpty(value.GetString()))
        {
            throw new InvalidDataException($"release {name} is missing");
        }
        return value.GetString()!;
    }

    internal static void RequireString(JsonElement owner, string name, string expected)
    {
        if (!owner.TryGetProperty(name, out var value) ||
            value.ValueKind != JsonValueKind.String ||
            !string.Equals(value.GetString(), expected, StringComparison.Ordinal))
        {
            throw new InvalidDataException($"unexpected release {name}");
        }
    }

    private static void RequireBoolean(JsonElement owner, string name, bool expected)
    {
        if (!owner.TryGetProperty(name, out var value) ||
            value.ValueKind is not (JsonValueKind.True or JsonValueKind.False) ||
            value.GetBoolean() != expected)
        {
            throw new InvalidDataException($"unexpected release {name}");
        }
    }

    internal static string Hash(byte[] value) =>
        Convert.ToHexStringLower(SHA256.HashData(value));

    internal static void ValidateSha256(string value)
    {
        if (value.Length != 64 || value.Any(ch =>
            !(ch is >= '0' and <= '9' or >= 'a' and <= 'f')))
        {
            throw new InvalidDataException("SHA-256 is not lowercase hexadecimal");
        }
    }

    internal static bool FixedEqualsHex(string left, string right)
    {
        ValidateSha256(left);
        ValidateSha256(right);
        return CryptographicOperations.FixedTimeEquals(
            Convert.FromHexString(left),
            Convert.FromHexString(right));
    }
}

internal static class PathPolicy
{
    private static readonly HashSet<string> ReservedNames = new(
        ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5",
         "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4",
         "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"],
        StringComparer.OrdinalIgnoreCase);

    public static void ValidateRelative(string path)
    {
        if (string.IsNullOrWhiteSpace(path) ||
            path.Contains('\\', StringComparison.Ordinal) ||
            path.StartsWith("/", StringComparison.Ordinal) ||
            path.EndsWith("/", StringComparison.Ordinal) ||
            Path.IsPathFullyQualified(path))
        {
            throw new InvalidDataException($"unsafe release path: {path}");
        }
        foreach (var part in path.Split('/'))
        {
            var stem = part.Split('.')[0];
            if (part is "" or "." or ".." ||
                part.EndsWith(' ') ||
                part.EndsWith('.') ||
                part.Any(ch => ch < 32 || "<>:\"|?*".Contains(ch)) ||
                ReservedNames.Contains(stem))
            {
                throw new InvalidDataException($"unsafe release path: {path}");
            }
        }
    }
}

internal sealed class EmbeddedBundle
{
    private const string ResourceName = "RustyFleet.EmbeddedBundle.zip";
    private const long MaximumArchiveBytes = 512L * 1024 * 1024;
    private readonly byte[] bytes;

    private EmbeddedBundle(
        byte[] bytes,
        string archiveSha256,
        ReleaseDefinition definition)
    {
        this.bytes = bytes;
        ArchiveSha256 = archiveSha256;
        Definition = definition;
    }

    public string ArchiveSha256 { get; }
    public ReleaseDefinition Definition { get; }

    public static EmbeddedBundle Load()
    {
        using var resource = Assembly.GetExecutingAssembly()
            .GetManifestResourceStream(ResourceName)
            ?? throw new InvalidDataException("embedded Fleet bundle is missing");
        if (resource.Length <= 0 || resource.Length > MaximumArchiveBytes)
        {
            throw new InvalidDataException("embedded Fleet bundle size is outside the limit");
        }
        using var memory = new MemoryStream();
        resource.CopyTo(memory);
        var bytes = memory.ToArray();
        var archiveSha256 = ReleaseDefinition.Hash(bytes);
        if (!ReleaseDefinition.FixedEqualsHex(
                archiveSha256,
                ReleaseConfiguration.BundleSha256))
        {
            throw new InvalidDataException("embedded Fleet bundle hash is not release-bound");
        }

        using var zip = Open(bytes);
        var entries = ValidateEntries(zip);
        var manifest = ReadBounded(entries["metadata/release-manifest.json"], 8 * 1024 * 1024);
        var checksums = ReadBounded(entries["metadata/checksums.sha256"], 8 * 1024 * 1024);
        var receipt = ReadBounded(entries["metadata/validation-receipt.json"], 2 * 1024 * 1024);
        var definition = ReleaseDefinition.Parse(manifest, checksums, receipt);
        if (!ReleaseDefinition.FixedEqualsHex(
                definition.ManifestSha256,
                ReleaseConfiguration.ManifestSha256) ||
            !string.Equals(
                definition.Version,
                ReleaseConfiguration.Version,
                StringComparison.Ordinal) ||
            !string.Equals(
                definition.Channel,
                ReleaseConfiguration.Channel,
                StringComparison.Ordinal) ||
            !string.Equals(
                definition.BuildKind,
                ReleaseConfiguration.BuildKind,
                StringComparison.Ordinal) ||
            !definition.AllFiles.Keys.ToHashSet(StringComparer.Ordinal)
                .SetEquals(entries.Keys))
        {
            throw new InvalidDataException("embedded Fleet release identity is not exact");
        }
        foreach (var (path, item) in definition.AllFiles)
        {
            var entry = entries[path];
            if (entry.Length != item.SizeBytes)
            {
                throw new InvalidDataException($"embedded release size mismatch: {path}");
            }
            using var input = entry.Open();
            var hash = Convert.ToHexStringLower(SHA256.HashData(input));
            if (!ReleaseDefinition.FixedEqualsHex(hash, item.Sha256))
            {
                throw new InvalidDataException($"embedded release digest mismatch: {path}");
            }
        }
        return new EmbeddedBundle(bytes, archiveSha256, definition);
    }

    public void ExtractTo(string root, DirectoryGuard guard)
    {
        using var zip = Open(bytes);
        var entries = ValidateEntries(zip);
        foreach (var directory in entries.Keys
                     .Select(Path.GetDirectoryName)
                     .Where(path => !string.IsNullOrEmpty(path))
                     .Select(path => path!.Replace('\\', '/'))
                     .Distinct(StringComparer.Ordinal)
                     .OrderBy(path => path.Count(ch => ch == '/'))
                     .ThenBy(path => path, StringComparer.Ordinal))
        {
            guard.CreateNewRelativeDirectories(root, directory);
        }
        foreach (var path in Definition.AllFiles.Keys.Order(StringComparer.Ordinal))
        {
            using var input = entries[path].Open();
            guard.WriteNewRetainedFile(root, path, input);
        }
        InstalledRelease.ValidateExact(root, Definition, guard);
    }

    private static ZipArchive Open(byte[] value) =>
        new(new MemoryStream(value, writable: false), ZipArchiveMode.Read);

    private static Dictionary<string, ZipArchiveEntry> ValidateEntries(ZipArchive zip)
    {
        var entries = new Dictionary<string, ZipArchiveEntry>(StringComparer.Ordinal);
        var windowsNames = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var productStem = ReleaseConfiguration.Channel == "alpha"
            ? "RustyFleet-Alpha"
            : "RustyFleet";
        var prefix = $"{productStem}-{ReleaseConfiguration.Version}-win-x64/";
        foreach (var entry in zip.Entries)
        {
            var archivePath = entry.FullName;
            if (!archivePath.StartsWith(prefix, StringComparison.Ordinal))
            {
                throw new InvalidDataException($"ZIP entry is outside its exact root: {archivePath}");
            }
            var path = archivePath[prefix.Length..];
            if (string.IsNullOrEmpty(path) || path.EndsWith("/", StringComparison.Ordinal))
            {
                continue;
            }
            PathPolicy.ValidateRelative(path);
            if (!entries.TryAdd(path, entry) || !windowsNames.Add(path))
            {
                throw new InvalidDataException($"duplicate ZIP path: {path}");
            }
        }
        return entries;
    }

    private static byte[] ReadBounded(ZipArchiveEntry entry, int limit)
    {
        if (entry.Length < 0 || entry.Length > limit)
        {
            throw new InvalidDataException($"ZIP entry is outside its limit: {entry.FullName}");
        }
        using var input = entry.Open();
        using var output = new MemoryStream(checked((int)entry.Length));
        input.CopyTo(output);
        if (output.Length != entry.Length)
        {
            throw new InvalidDataException($"ZIP entry length changed: {entry.FullName}");
        }
        return output.ToArray();
    }
}

internal static class InstalledRelease
{
    public static ReleaseDefinition LoadAndValidate(
        string root,
        DirectoryGuard guard,
        string expectedManifestSha256)
    {
        guard.RetainExistingTree(root);
        var manifest = guard.ReadAllRetained(
            Path.Combine(root, "metadata", "release-manifest.json"),
            8 * 1024 * 1024);
        if (!ReleaseDefinition.FixedEqualsHex(
                ReleaseDefinition.Hash(manifest),
                expectedManifestSha256))
        {
            throw new InvalidDataException("installed release manifest is not exact");
        }
        var checksums = guard.ReadAllRetained(
            Path.Combine(root, "metadata", "checksums.sha256"),
            8 * 1024 * 1024);
        var receipt = guard.ReadAllRetained(
            Path.Combine(root, "metadata", "validation-receipt.json"),
            2 * 1024 * 1024);
        var definition = ReleaseDefinition.Parse(manifest, checksums, receipt);
        ValidateExact(root, definition, guard);
        return definition;
    }

    public static void ValidateExact(
        string root,
        ReleaseDefinition definition,
        DirectoryGuard guard)
    {
        guard.RetainExistingTree(root);
        var actual = guard.RetainedFilesUnder(root);
        if (!actual.SetEquals(definition.AllFiles.Keys))
        {
            throw new InvalidDataException("installed release composition is not exact");
        }
        foreach (var (relative, expected) in definition.AllFiles)
        {
            var path = Path.Combine(root, relative.Replace('/', Path.DirectorySeparatorChar));
            guard.ValidateRetainedFile(path, expected);
        }
        guard.RecheckRetainedTree(root);
    }
}

internal static class Installer
{
    private const int MaximumHistory = 16;
    private static readonly JsonSerializerOptions StateJson = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        WriteIndented = true,
        PropertyNameCaseInsensitive = false,
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow,
        RespectRequiredConstructorParameters = true,
    };

    public static InstallResult Execute(
        SetupAction action,
        EmbeddedBundle bundle,
        string installRoot)
    {
        using var guard = DirectoryGuard.OpenInstallChain(installRoot);
        var releasesRoot = guard.CreateOrRetainRelative(installRoot, "releases");
        var stateRoot = guard.CreateOrRetainRelative(installRoot, "state");
        guard.AcquireTransactionLock(Path.Combine(stateRoot, "setup.lock"));

        var statePath = Path.Combine(stateRoot, "current.json");
        var previous = ReadAndValidateState(statePath, installRoot, guard);
        StateRelease current;
        StateRelease[] history;

        if (action == SetupAction.Rollback)
        {
            if (previous is null || previous.History.Length == 0)
            {
                throw new InvalidOperationException("there is no verified previous release to roll back to");
            }
            current = previous.History[0];
            history = new[] { previous.Current }
                .Concat(previous.History.Skip(1))
                .DistinctBy(item => item.ReleaseId, StringComparer.Ordinal)
                .Take(MaximumHistory)
                .ToArray();
        }
        else
        {
            var releaseId =
                $"{bundle.Definition.Version}-{bundle.Definition.ManifestSha256[..16]}";
            var reusable = previous is null
                ? null
                : new[] { previous.Current }
                    .Concat(previous.History)
                    .FirstOrDefault(item =>
                        string.Equals(item.ReleaseId, releaseId, StringComparison.Ordinal) &&
                        ReleaseDefinition.FixedEqualsHex(
                            item.ManifestSha256,
                            bundle.Definition.ManifestSha256) &&
                        ReleaseDefinition.FixedEqualsHex(
                            item.BundleSha256,
                            bundle.ArchiveSha256));
            if (reusable is null)
            {
                var candidateName =
                    $".candidate-{bundle.Definition.ManifestSha256[..16]}-{Guid.NewGuid():N}";
                var candidateRoot = Path.Combine(releasesRoot, candidateName);
                guard.CreateNewDirectory(candidateRoot);
                bundle.ExtractTo(candidateRoot, guard);
                current = new StateRelease(
                    Version: bundle.Definition.Version,
                    ReleaseId: releaseId,
                    ManifestSha256: bundle.Definition.ManifestSha256,
                    BundleSha256: bundle.ArchiveSha256,
                    RelativePath: $"releases/{candidateName}");
            }
            else
            {
                current = reusable;
            }
            history = previous is null
                ? []
                : new[] { previous.Current }
                    .Concat(previous.History)
                    .Where(item =>
                        !string.Equals(item.ReleaseId, current.ReleaseId, StringComparison.Ordinal))
                    .DistinctBy(item => item.ReleaseId, StringComparer.Ordinal)
                    .Take(MaximumHistory)
                    .ToArray();
        }

        var state = new InstallState(
            Schema: "rusty.fleet.windows_setup_state.v2",
            Current: current,
            History: history,
            Policy: new StatePolicy(
                Update: "side_by_side_exact_manifest",
                Rollback: "previous_fully_verified_release",
                AutomaticDelete: false));
        if (ReleaseConfiguration.BuildKind == "unsigned-dev" &&
            ReleaseConfiguration.DevelopmentTestPauseAfterRetainMs > 0)
        {
            Thread.Sleep(ReleaseConfiguration.DevelopmentTestPauseAfterRetainMs);
        }
        ValidateStateRelease(current, installRoot, guard);
        foreach (var item in history)
        {
            ValidateStateRelease(item, installRoot, guard);
        }
        CommitState(statePath, stateRoot, state, guard);
        ValidateStateRelease(current, installRoot, guard);
        foreach (var item in history)
        {
            ValidateStateRelease(item, installRoot, guard);
        }
        guard.RecheckRetainedTree(
            Path.Combine(
                installRoot,
                current.RelativePath.Replace('/', Path.DirectorySeparatorChar)));

        return new InstallResult(
            Schema: "rusty.fleet.windows_setup_result.v2",
            Result: "pass",
            Action: action == SetupAction.Install ? "install" : "rollback",
            Version: current.Version,
            ReleaseId: current.ReleaseId,
            ManifestSha256: current.ManifestSha256,
            BundleSha256: current.BundleSha256,
            RollbackAvailable: history.Length > 0,
            ProcessesStarted: 0,
            ServicesRegistered: 0,
            ConfigurationCreated: false,
            OnboardingInvoked: false);
    }

    private static InstallState? ReadAndValidateState(
        string statePath,
        string installRoot,
        DirectoryGuard guard)
    {
        if (!File.Exists(statePath))
        {
            return null;
        }
        guard.RetainExistingFile(statePath);
        var bytes = guard.ReadAllRetained(statePath, 1024 * 1024);
        InstallState state;
        try
        {
            state = JsonSerializer.Deserialize<InstallState>(bytes, StateJson)
                ?? throw new JsonException();
        }
        catch (JsonException exception)
        {
            throw new InvalidDataException("existing Fleet install state is invalid", exception);
        }
        if (!string.Equals(
                state.Schema,
                "rusty.fleet.windows_setup_state.v2",
                StringComparison.Ordinal) ||
            state.History.Length > MaximumHistory ||
            state.Policy.Update != "side_by_side_exact_manifest" ||
            state.Policy.Rollback != "previous_fully_verified_release" ||
            state.Policy.AutomaticDelete)
        {
            throw new InvalidDataException("existing Fleet install state is unsupported");
        }
        var ids = new HashSet<string>(StringComparer.Ordinal);
        foreach (var item in new[] { state.Current }.Concat(state.History))
        {
            if (!ids.Add(item.ReleaseId))
            {
                throw new InvalidDataException("existing Fleet install state has duplicate releases");
            }
            ValidateStateRelease(item, installRoot, guard);
        }
        return state;
    }

    private static ReleaseDefinition ValidateStateRelease(
        StateRelease item,
        string installRoot,
        DirectoryGuard guard)
    {
        ReleaseDefinition.ValidateSha256(item.ManifestSha256);
        ReleaseDefinition.ValidateSha256(item.BundleSha256);
        if (string.IsNullOrWhiteSpace(item.ReleaseId) ||
            string.IsNullOrWhiteSpace(item.Version))
        {
            throw new InvalidDataException("installed release identity is invalid");
        }
        PathPolicy.ValidateRelative(item.RelativePath);
        var segments = item.RelativePath.Split('/');
        if (segments.Length != 2 ||
            segments[0] != "releases" ||
            !segments[1].StartsWith(".candidate-", StringComparison.Ordinal))
        {
            throw new InvalidDataException("installed release state path is not exact");
        }
        var releaseRoot = Path.GetFullPath(Path.Combine(
            installRoot,
            item.RelativePath.Replace('/', Path.DirectorySeparatorChar)));
        var prefix = Path.TrimEndingDirectorySeparator(installRoot) +
            Path.DirectorySeparatorChar;
        if (!releaseRoot.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
        {
            throw new InvalidDataException("installed release escaped its root");
        }
        var definition = InstalledRelease.LoadAndValidate(
            releaseRoot,
            guard,
            item.ManifestSha256);
        if (!string.Equals(definition.Version, item.Version, StringComparison.Ordinal) ||
            !string.Equals(
                item.ReleaseId,
                $"{definition.Version}-{definition.ManifestSha256[..16]}",
                StringComparison.Ordinal))
        {
            throw new InvalidDataException("installed release state identity is not exact");
        }
        return definition;
    }

    private static void CommitState(
        string statePath,
        string stateRoot,
        InstallState state,
        DirectoryGuard guard)
    {
        var stateBytes = Encoding.UTF8.GetBytes(
            JsonSerializer.Serialize(state, StateJson) + "\n");
        var tempPath = Path.Combine(stateRoot, $".current-{Guid.NewGuid():N}.tmp");
        guard.WriteNewRetainedStateFile(tempPath, stateBytes);
        guard.ValidateRetainedBytes(tempPath, stateBytes);
        guard.ReleaseRetainedFile(statePath);
        guard.RenameRetainedFile(
            tempPath,
            stateRoot,
            "current.json",
            statePath);
        guard.ValidateRetainedBytes(statePath, stateBytes);
    }

    private sealed record InstallState(
        string Schema,
        StateRelease Current,
        StateRelease[] History,
        StatePolicy Policy);

    private sealed record StateRelease(
        string Version,
        string ReleaseId,
        string ManifestSha256,
        string BundleSha256,
        string RelativePath);

    private sealed record StatePolicy(
        string Update,
        string Rollback,
        bool AutomaticDelete);
}

internal sealed class DirectoryGuard : IDisposable
{
    private const uint FileReadAttributes = 0x80;
    private const uint GenericRead = 0x80000000;
    private const uint GenericWrite = 0x40000000;
    private const uint DeleteAccess = 0x00010000;
    private const uint FileShareRead = 0x1;
    private const uint FileShareWrite = 0x2;
    private const uint CreateNew = 1;
    private const uint OpenExisting = 3;
    private const uint OpenAlways = 4;
    private const uint FileAttributeNormal = 0x80;
    private const uint FileFlagWriteThrough = 0x80000000;
    private const uint FileFlagBackupSemantics = 0x02000000;
    private const uint FileFlagOpenReparsePoint = 0x00200000;
    private const uint FileAttributeReparsePoint = 0x400;
    private const int ErrorAlreadyExists = 183;
    private const int ErrorSharingViolation = 32;
    private const int FileRenameInfo = 3;
    private readonly Dictionary<string, SafeFileHandle> directories =
        new(StringComparer.OrdinalIgnoreCase);
    private readonly Dictionary<string, FileStream> files =
        new(StringComparer.OrdinalIgnoreCase);
    private FileStream? transactionLock;

    private DirectoryGuard() { }

    public static DirectoryGuard OpenInstallChain(string installRoot)
    {
        if (!Path.IsPathFullyQualified(installRoot) ||
            installRoot.StartsWith(@"\\", StringComparison.Ordinal) ||
            installRoot.StartsWith(@"\\?\", StringComparison.Ordinal) ||
            installRoot.StartsWith(@"\??\", StringComparison.Ordinal))
        {
            throw new IOException("install root must be a normal local absolute path");
        }
        var root = Path.GetPathRoot(installRoot)
            ?? throw new IOException("install root has no volume root");
        var guard = new DirectoryGuard();
        try
        {
            guard.RetainExistingDirectory(root);
            var current = root;
            var relative = Path.GetRelativePath(root, installRoot);
            foreach (var component in relative.Split(
                         Path.DirectorySeparatorChar,
                         StringSplitOptions.RemoveEmptyEntries))
            {
                current = Path.Combine(current, component);
                if (!Directory.Exists(current))
                {
                    Directory.CreateDirectory(current);
                }
                guard.RetainExistingDirectory(current);
            }
            return guard;
        }
        catch
        {
            guard.Dispose();
            throw;
        }
    }

    public string CreateOrRetainRelative(string root, string relative)
    {
        var current = root;
        foreach (var component in relative.Replace('\\', '/').Split(
                     '/',
                     StringSplitOptions.RemoveEmptyEntries))
        {
            if (component is "." or ".." || component.Contains(':'))
            {
                throw new IOException("unsafe install directory component");
            }
            current = Path.Combine(current, component);
            if (!Directory.Exists(current))
            {
                Directory.CreateDirectory(current);
            }
            RetainExistingDirectory(current);
        }
        return current;
    }

    public void CreateNewDirectory(string path)
    {
        var fullPath = Path.GetFullPath(path);
        if (!CreateDirectoryW(ToNativePath(fullPath), IntPtr.Zero))
        {
            ThrowLastIo($"could not create exact install directory: {fullPath}");
        }
        try
        {
            RetainExistingDirectory(fullPath);
        }
        catch
        {
            throw;
        }
    }

    public void CreateNewRelativeDirectories(string root, string relative)
    {
        var current = root;
        foreach (var component in relative.Split('/'))
        {
            current = Path.Combine(current, component);
            var full = Path.GetFullPath(current);
            if (directories.ContainsKey(full))
            {
                continue;
            }
            CreateNewDirectory(full);
        }
    }

    public void AcquireTransactionLock(string path)
    {
        var deadline = DateTime.UtcNow.AddSeconds(30);
        while (true)
        {
            var handle = CreateFileW(
                ToNativePath(path),
                GenericRead | GenericWrite,
                0,
                IntPtr.Zero,
                OpenAlways,
                FileAttributeNormal | FileFlagOpenReparsePoint,
                IntPtr.Zero);
            if (!handle.IsInvalid)
            {
                ThrowIfUnsafe(handle, path, requireSingleLink: true);
                transactionLock = new FileStream(handle, FileAccess.ReadWrite);
                return;
            }
            var error = Marshal.GetLastWin32Error();
            handle.Dispose();
            if (error != ErrorSharingViolation || DateTime.UtcNow >= deadline)
            {
                throw new IOException(
                    $"could not acquire the exclusive Fleet Setup transaction lock: {path}",
                    new System.ComponentModel.Win32Exception(error));
            }
            Thread.Sleep(50);
        }
    }

    public void WriteNewRetainedFile(string root, string relative, Stream input)
    {
        PathPolicy.ValidateRelative(relative);
        var path = Path.GetFullPath(Path.Combine(
            root,
            relative.Replace('/', Path.DirectorySeparatorChar)));
        var prefix = Path.TrimEndingDirectorySeparator(root) +
            Path.DirectorySeparatorChar;
        if (!path.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
        {
            throw new IOException("release payload escaped its retained root");
        }
        var stream = new FileStream(
            path,
            FileMode.CreateNew,
            FileAccess.ReadWrite,
            FileShare.Read,
            1024 * 1024,
            FileOptions.WriteThrough);
        try
        {
            input.CopyTo(stream);
            stream.Flush(flushToDisk: true);
            ThrowIfUnsafe(stream.SafeFileHandle, path, requireSingleLink: true);
            files.Add(path, stream);
        }
        catch
        {
            stream.Dispose();
            throw;
        }
    }

    public void WriteNewRetainedStateFile(string path, byte[] bytes)
    {
        var handle = CreateFileW(
            ToNativePath(path),
            GenericRead | GenericWrite | DeleteAccess,
            FileShareRead,
            IntPtr.Zero,
            CreateNew,
            FileAttributeNormal | FileFlagWriteThrough | FileFlagOpenReparsePoint,
            IntPtr.Zero);
        if (handle.IsInvalid)
        {
            ThrowLastIo($"could not create exact Fleet state file: {path}");
        }
        var stream = new FileStream(handle, FileAccess.ReadWrite);
        try
        {
            stream.Write(bytes);
            stream.Flush(flushToDisk: true);
            ThrowIfUnsafe(stream.SafeFileHandle, path, requireSingleLink: true);
            files.Add(Path.GetFullPath(path), stream);
        }
        catch
        {
            stream.Dispose();
            throw;
        }
    }

    public void RetainExistingTree(string root)
    {
        var fullRoot = Path.GetFullPath(root);
        RetainExistingDirectory(fullRoot);
        var pending = new Queue<string>();
        pending.Enqueue(fullRoot);
        while (pending.TryDequeue(out var current))
        {
            foreach (var directory in Directory.EnumerateDirectories(current)
                         .Order(StringComparer.OrdinalIgnoreCase))
            {
                RetainExistingDirectory(directory);
                pending.Enqueue(directory);
            }
        }
        foreach (var directory in directories.Keys
                     .Where(path => IsUnder(fullRoot, path))
                     .Order(StringComparer.OrdinalIgnoreCase))
        {
            foreach (var file in Directory.EnumerateFiles(directory)
                         .Order(StringComparer.OrdinalIgnoreCase))
            {
                RetainExistingFile(file);
            }
        }
    }

    public void RecheckRetainedTree(string root)
    {
        var fullRoot = Path.GetFullPath(root);
        foreach (var directory in directories
                     .Where(pair => IsUnder(fullRoot, pair.Key)))
        {
            ThrowIfUnsafe(directory.Value, directory.Key, requireSingleLink: false);
        }
        foreach (var file in files
                     .Where(pair => IsUnder(fullRoot, pair.Key)))
        {
            ThrowIfUnsafe(file.Value.SafeFileHandle, file.Key, requireSingleLink: true);
        }
        var observed = EnumerateRelativeFiles(fullRoot);
        var retained = RetainedFilesUnder(fullRoot);
        if (!observed.SetEquals(retained))
        {
            throw new IOException("retained release tree changed during validation");
        }
    }

    public HashSet<string> RetainedFilesUnder(string root)
    {
        var fullRoot = Path.GetFullPath(root);
        return files.Keys
            .Where(path => IsUnder(fullRoot, path))
            .Select(path => Path.GetRelativePath(fullRoot, path).Replace('\\', '/'))
            .ToHashSet(StringComparer.Ordinal);
    }

    public void RetainExistingFile(string path)
    {
        var fullPath = Path.GetFullPath(path);
        if (files.ContainsKey(fullPath))
        {
            return;
        }
        var handle = CreateFileW(
            ToNativePath(fullPath),
            GenericRead,
            FileShareRead,
            IntPtr.Zero,
            OpenExisting,
            FileAttributeNormal | FileFlagOpenReparsePoint,
            IntPtr.Zero);
        if (handle.IsInvalid)
        {
            ThrowLastIo($"could not retain install file: {fullPath}");
        }
        var stream = new FileStream(handle, FileAccess.Read);
        try
        {
            ThrowIfUnsafe(stream.SafeFileHandle, fullPath, requireSingleLink: true);
            files.Add(fullPath, stream);
        }
        catch
        {
            stream.Dispose();
            throw;
        }
    }

    public byte[] ReadAllRetained(string path, int maximumBytes)
    {
        var stream = RequiredFile(path);
        if (stream.Length < 0 || stream.Length > maximumBytes)
        {
            throw new InvalidDataException($"retained file is outside its limit: {path}");
        }
        var bytes = new byte[checked((int)stream.Length)];
        stream.Position = 0;
        stream.ReadExactly(bytes);
        stream.Position = 0;
        return bytes;
    }

    public void ValidateRetainedFile(string path, InventoryItem expected)
    {
        var stream = RequiredFile(path);
        ThrowIfUnsafe(stream.SafeFileHandle, path, requireSingleLink: true);
        if (stream.Length != expected.SizeBytes)
        {
            throw new InvalidDataException($"installed payload size mismatch: {path}");
        }
        stream.Position = 0;
        var digest = Convert.ToHexStringLower(SHA256.HashData(stream));
        stream.Position = 0;
        if (!ReleaseDefinition.FixedEqualsHex(digest, expected.Sha256))
        {
            throw new InvalidDataException($"installed payload digest mismatch: {path}");
        }
    }

    public void ValidateRetainedBytes(string path, byte[] expected)
    {
        var stream = RequiredFile(path);
        ThrowIfUnsafe(stream.SafeFileHandle, path, requireSingleLink: true);
        if (stream.Length != expected.LongLength)
        {
            throw new IOException("retained state length changed");
        }
        stream.Position = 0;
        var observed = new byte[expected.Length];
        stream.ReadExactly(observed);
        stream.Position = 0;
        if (!CryptographicOperations.FixedTimeEquals(
                SHA256.HashData(observed),
                SHA256.HashData(expected)))
        {
            throw new IOException("retained state bytes changed");
        }
    }

    public void ReleaseRetainedFile(string path)
    {
        var fullPath = Path.GetFullPath(path);
        if (files.Remove(fullPath, out var stream))
        {
            stream.Dispose();
        }
    }

    public void RenameRetainedFile(
        string sourcePath,
        string targetDirectory,
        string targetName,
        string targetPath)
    {
        if (IntPtr.Size != 8)
        {
            throw new PlatformNotSupportedException("Fleet Setup requires x64 Windows");
        }
        var sourceFull = Path.GetFullPath(sourcePath);
        var targetFull = Path.GetFullPath(targetPath);
        var source = RequiredFile(sourceFull);
        var directory = RequiredDirectory(targetDirectory);
        if (!string.Equals(
                Path.GetDirectoryName(targetFull),
                Path.GetFullPath(targetDirectory),
                StringComparison.OrdinalIgnoreCase) ||
            !string.Equals(
                Path.GetFileName(targetFull),
                targetName,
                StringComparison.Ordinal))
        {
            throw new IOException("Fleet state rename target is not exact");
        }
        ThrowIfUnsafe(directory, targetDirectory, requireSingleLink: false);
        var nameBytes = Encoding.Unicode.GetBytes(targetFull);
        // FILE_RENAME_INFO is 24 bytes on x64 because its variable WCHAR
        // member carries trailing structure alignment. FileNameLength excludes
        // that member's zero terminator/padding.
        var size = checked(24 + nameBytes.Length);
        var buffer = Marshal.AllocHGlobal(size);
        try
        {
            for (var index = 0; index < size; index++)
            {
                Marshal.WriteByte(buffer, index, 0);
            }
            Marshal.WriteByte(buffer, 0, 1);
            // SetFileInformationByHandle's Win32 conversion path requires a
            // null RootDirectory and an absolute name. The containing state
            // directory remains retained without delete sharing above, so the
            // absolute target cannot be substituted while this runs.
            Marshal.WriteInt64(buffer, 8, 0);
            Marshal.WriteInt32(buffer, 16, nameBytes.Length);
            Marshal.Copy(nameBytes, 0, IntPtr.Add(buffer, 20), nameBytes.Length);
            if (!SetFileInformationByHandle(
                    source.SafeFileHandle,
                    FileRenameInfo,
                    buffer,
                    (uint)size))
            {
                ThrowLastIo("could not atomically commit Fleet state");
            }
        }
        finally
        {
            Marshal.FreeHGlobal(buffer);
        }
        files.Remove(sourceFull);
        files.Add(targetFull, source);
    }

    private void RetainExistingDirectory(string path)
    {
        var fullPath = Path.GetFullPath(path);
        if (directories.ContainsKey(fullPath))
        {
            return;
        }
        var handle = CreateFileW(
            ToNativePath(fullPath),
            FileReadAttributes,
            FileShareRead,
            IntPtr.Zero,
            OpenExisting,
            FileFlagBackupSemantics | FileFlagOpenReparsePoint,
            IntPtr.Zero);
        if (handle.IsInvalid)
        {
            ThrowLastIo($"could not retain install directory: {fullPath}");
        }
        try
        {
            ThrowIfUnsafe(handle, fullPath, requireSingleLink: false);
            directories.Add(fullPath, handle);
        }
        catch
        {
            handle.Dispose();
            throw;
        }
    }

    private FileStream RequiredFile(string path)
    {
        var fullPath = Path.GetFullPath(path);
        if (!files.TryGetValue(fullPath, out var stream))
        {
            throw new IOException($"install file is not retained: {fullPath}");
        }
        return stream;
    }

    private SafeFileHandle RequiredDirectory(string path)
    {
        var fullPath = Path.GetFullPath(path);
        if (!directories.TryGetValue(fullPath, out var handle))
        {
            throw new IOException($"install directory is not retained: {fullPath}");
        }
        return handle;
    }

    private HashSet<string> EnumerateRelativeFiles(string root)
    {
        var result = new HashSet<string>(StringComparer.Ordinal);
        foreach (var path in Directory.EnumerateFiles(root, "*", SearchOption.AllDirectories))
        {
            result.Add(Path.GetRelativePath(root, path).Replace('\\', '/'));
        }
        return result;
    }

    private static string ToNativePath(string path)
    {
        var fullPath = Path.GetFullPath(path);
        return fullPath.StartsWith(@"\\?\", StringComparison.Ordinal)
            ? fullPath
            : @"\\?\" + fullPath;
    }

    private static bool IsUnder(string root, string path)
    {
        if (string.Equals(root, path, StringComparison.OrdinalIgnoreCase))
        {
            return true;
        }
        var prefix = Path.TrimEndingDirectorySeparator(root) +
            Path.DirectorySeparatorChar;
        return path.StartsWith(prefix, StringComparison.OrdinalIgnoreCase);
    }

    private static void ThrowIfUnsafe(
        SafeFileHandle handle,
        string path,
        bool requireSingleLink)
    {
        if (!GetFileInformationByHandle(handle, out var info))
        {
            ThrowLastIo($"could not inspect retained install path: {path}");
        }
        if ((info.FileAttributes & FileAttributeReparsePoint) != 0)
        {
            throw new IOException($"reparse points are forbidden in install paths: {path}");
        }
        if (requireSingleLink && info.NumberOfLinks != 1)
        {
            throw new IOException($"hard-linked install files are forbidden: {path}");
        }
    }

    private static void ThrowLastIo(string message)
    {
        var error = Marshal.GetLastWin32Error();
        var native = new System.ComponentModel.Win32Exception(error);
        throw new IOException($"{message}: Win32 {error}: {native.Message}", native);
    }

    public void Dispose()
    {
        transactionLock?.Dispose();
        transactionLock = null;
        foreach (var stream in files.Values)
        {
            stream.Dispose();
        }
        files.Clear();
        foreach (var handle in directories.Values.Reverse())
        {
            handle.Dispose();
        }
        directories.Clear();
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
        string lpFileName,
        uint dwDesiredAccess,
        uint dwShareMode,
        IntPtr lpSecurityAttributes,
        uint dwCreationDisposition,
        uint dwFlagsAndAttributes,
        IntPtr hTemplateFile);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CreateDirectoryW(
        string lpPathName,
        IntPtr lpSecurityAttributes);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetFileInformationByHandle(
        SafeFileHandle hFile,
        out ByHandleFileInformation lpFileInformation);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetFileInformationByHandle(
        SafeFileHandle hFile,
        int fileInformationClass,
        IntPtr lpFileInformation,
        uint dwBufferSize);
}
