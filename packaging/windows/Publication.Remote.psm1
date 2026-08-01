# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not ("RustyFleet.Publication.RemoteBoundedProcessRunner" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading.Tasks;

namespace RustyFleet.Publication
{
    public sealed class RemoteProcessResult
    {
        public int ExitCode { get; init; }
        public string StandardOutput { get; init; } = "";
        public bool StandardErrorPresent { get; init; }
    }

    public static class RemoteBoundedProcessRunner
    {
        public static RemoteProcessResult Run(
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
                throw new InvalidOperationException(
                    "remote release authority process did not start");
            }
            var output = ReadBounded(process.StandardOutput, maximumBytes);
            var error = ReadBounded(process.StandardError, maximumBytes);
            if (!process.WaitForExit(timeoutMilliseconds))
            {
                process.Kill(true);
                process.WaitForExit();
                throw new TimeoutException(
                    "remote release authority process exceeded its limit");
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
            return new RemoteProcessResult
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
                    throw new InvalidDataException(
                        "remote release authority output exceeded its limit");
                }
                result.Append(buffer, 0, read);
            }
        }
    }
}
"@
}

function Test-RustyFleetRemoteProperty {
    param(
        [AllowNull()][object] $InputObject,
        [Parameter(Mandatory)][string] $Name
    )

    return (
        $null -ne $InputObject -and
        $null -ne $InputObject.PSObject.Properties[$Name]
    )
}

function Assert-RustyFleetRemoteHexSha {
    param(
        [AllowNull()][object] $Value,
        [Parameter(Mandatory)][string] $Context
    )

    if ($Value -isnot [string] -or
        [string] $Value -cnotmatch "^[0-9a-f]{40}$") {
        throw "$Context is malformed"
    }
}

function Test-RustyFleetRemoteInteger {
    param([AllowNull()][object] $Value)

    return (
        $Value -is [byte] -or
        $Value -is [sbyte] -or
        $Value -is [short] -or
        $Value -is [ushort] -or
        $Value -is [int] -or
        $Value -is [uint] -or
        $Value -is [long] -or
        $Value -is [ulong]
    )
}

function Get-RustyFleetGhInvocation {
    param(
        [Parameter(Mandatory)][string] $GhExecutable,
        [Parameter(Mandatory)][string[]] $Arguments
    )

    if ([IO.Path]::GetExtension($GhExecutable) -ieq ".ps1") {
        $resolvedScript = (Resolve-Path -LiteralPath $GhExecutable).Path
        $pwsh = (Get-Command pwsh -ErrorAction Stop).Source
        return [pscustomobject]@{
            executable = $pwsh
            arguments = @(
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-File",
                $resolvedScript
            ) + $Arguments
        }
    }
    return [pscustomobject]@{
        executable = $GhExecutable
        arguments = $Arguments
    }
}

function Invoke-RustyFleetGh {
    param(
        [Parameter(Mandatory)][string] $GhExecutable,
        [Parameter(Mandatory)][string[]] $Arguments,
        [Parameter(Mandatory)][string] $Context,
        [ValidateRange(1, 120)][int] $TimeoutSeconds = 30,
        [ValidateRange(1024, 16777216)][int] $MaximumBytes = 4194304
    )

    $invocation = Get-RustyFleetGhInvocation `
        -GhExecutable $GhExecutable `
        -Arguments $Arguments
    try {
        $result = [RustyFleet.Publication.RemoteBoundedProcessRunner]::Run(
            $invocation.executable,
            [string[]] $invocation.arguments,
            $TimeoutSeconds * 1000,
            $MaximumBytes
        )
    }
    catch {
        throw "$Context failed closed"
    }
    if ($result.ExitCode -ne 0) {
        throw "$Context failed closed"
    }
    return $result.StandardOutput
}

function Invoke-RustyFleetGhJson {
    param(
        [Parameter(Mandatory)][string] $GhExecutable,
        [Parameter(Mandatory)][string[]] $Arguments,
        [Parameter(Mandatory)][string] $Context,
        [ValidateRange(1, 120)][int] $TimeoutSeconds = 30,
        [ValidateRange(1024, 16777216)][int] $MaximumBytes = 4194304
    )

    $text = Invoke-RustyFleetGh `
        -GhExecutable $GhExecutable `
        -Arguments $Arguments `
        -Context $Context `
        -TimeoutSeconds $TimeoutSeconds `
        -MaximumBytes $MaximumBytes
    if ([string]::IsNullOrWhiteSpace($text)) {
        throw "$Context returned no JSON"
    }
    try {
        $value = $text | ConvertFrom-Json -Depth 40 -NoEnumerate
    }
    catch {
        throw "$Context returned malformed JSON"
    }
    return [pscustomobject]@{
        value = $value
    }
}

function Get-RustyFleetLatestReleaseOrAbsent {
    param(
        [Parameter(Mandatory)][string] $GitHubRepository,
        [Parameter(Mandatory)][string] $GhExecutable
    )

    $arguments = @(
        "api",
        "--method",
        "GET",
        "repos/$GitHubRepository/releases/latest"
    )
    try {
        return (Invoke-RustyFleetGhJson `
            -GhExecutable $GhExecutable `
            -Arguments $arguments `
            -Context "latest release isolation lookup").value
    }
    catch {
        $invocation = Get-RustyFleetGhInvocation `
            -GhExecutable $GhExecutable `
            -Arguments ($arguments + "--include")
        try {
            $result = [RustyFleet.Publication.RemoteBoundedProcessRunner]::Run(
                $invocation.executable,
                [string[]] $invocation.arguments,
                30000,
                4194304
            )
        }
        catch {
            throw "latest release isolation lookup failed closed"
        }
        if ($result.ExitCode -eq 0 -or
            $result.StandardOutput -cnotmatch
                '(?s)^HTTP/\S+\s+404(?:\s|$).*\{\s*"message"\s*:\s*"Not Found"') {
            throw "latest release isolation lookup failed closed"
        }
        return $null
    }
}

function Resolve-RustyFleetRemoteTagCommit {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [ValidatePattern("^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")]
        [string] $GitHubRepository,

        [Parameter(Mandatory)]
        [ValidatePattern("^v[0-9]+\.[0-9]+\.[0-9]+(?:-alpha\.[1-9][0-9]*)?$")]
        [string] $Tag,

        [Parameter(Mandatory)]
        [ValidatePattern("^[0-9a-f]{40}$")]
        [string] $ExpectedSourceRevision,

        [Parameter(Mandatory)]
        [string] $GhExecutable,

        [ValidateRange(1, 16)]
        [int] $MaximumTagDepth = 8
    )

    $ref = (Invoke-RustyFleetGhJson `
        -GhExecutable $GhExecutable `
        -Arguments @(
            "api",
            "--method",
            "GET",
            "repos/$GitHubRepository/git/ref/tags/$Tag"
        ) `
        -Context "remote release tag lookup").value
    if (-not (Test-RustyFleetRemoteProperty $ref "ref") -or
        $ref.ref -isnot [string] -or
        $ref.ref -cne "refs/tags/$Tag" -or
        -not (Test-RustyFleetRemoteProperty $ref "object") -or
        -not (Test-RustyFleetRemoteProperty $ref.object "type") -or
        -not (Test-RustyFleetRemoteProperty $ref.object "sha")) {
        throw "remote release tag reference is malformed"
    }

    $currentType = [string] $ref.object.type
    $currentSha = [string] $ref.object.sha
    Assert-RustyFleetRemoteHexSha `
        -Value $currentSha `
        -Context "remote release tag target"
    $observedTags = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal
    )
    $depth = 0
    while ($currentType -ceq "tag") {
        if ($depth -ge $MaximumTagDepth -or
            -not $observedTags.Add($currentSha)) {
            throw "remote release tag chain is cyclic or exceeds its bound"
        }
        $tagObject = (Invoke-RustyFleetGhJson `
            -GhExecutable $GhExecutable `
            -Arguments @(
                "api",
                "--method",
                "GET",
                "repos/$GitHubRepository/git/tags/$currentSha"
            ) `
            -Context "remote annotated tag lookup").value
        if (-not (Test-RustyFleetRemoteProperty $tagObject "sha") -or
            $tagObject.sha -isnot [string] -or
            $tagObject.sha -cne $currentSha -or
            -not (Test-RustyFleetRemoteProperty $tagObject "object") -or
            -not (Test-RustyFleetRemoteProperty $tagObject.object "type") -or
            -not (Test-RustyFleetRemoteProperty $tagObject.object "sha")) {
            throw "remote annotated tag object is malformed"
        }
        if ($depth -eq 0 -and
            (
                -not (Test-RustyFleetRemoteProperty $tagObject "tag") -or
                $tagObject.tag -isnot [string] -or
                $tagObject.tag -cne $Tag
            )) {
            throw "remote annotated release tag name is not exact"
        }
        $currentType = [string] $tagObject.object.type
        $currentSha = [string] $tagObject.object.sha
        Assert-RustyFleetRemoteHexSha `
            -Value $currentSha `
            -Context "remote annotated tag target"
        $depth++
    }
    if ($currentType -cne "commit" -or
        $currentSha -cne $ExpectedSourceRevision) {
        throw "remote release tag does not resolve to the expected source revision"
    }
    return $currentSha
}

function Get-RustyFleetRemoteReleaseByTag {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [ValidatePattern("^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")]
        [string] $GitHubRepository,

        [Parameter(Mandatory)]
        [ValidatePattern("^v[0-9]+\.[0-9]+\.[0-9]+(?:-alpha\.[1-9][0-9]*)?$")]
        [string] $Tag,

        [Parameter(Mandatory)]
        [string] $GhExecutable,

        [ValidateRange(1, 20)]
        [int] $MaximumPages = 10
    )

    $matches = [Collections.Generic.List[object]]::new()
    $complete = $false
    for ($page = 1; $page -le $MaximumPages; $page++) {
        $releases = (Invoke-RustyFleetGhJson `
            -GhExecutable $GhExecutable `
            -Arguments @(
                "api",
                "--method",
                "GET",
                "repos/$GitHubRepository/releases",
                "-f",
                "per_page=100",
                "-f",
                "page=$page"
            ) `
            -Context "remote release inventory lookup").value
        if ($releases -isnot [Collections.IList]) {
            throw "remote release inventory is malformed"
        }
        foreach ($release in $releases) {
            if (-not (Test-RustyFleetRemoteProperty $release "tag_name") -or
                $release.tag_name -isnot [string]) {
                throw "remote release inventory entry is malformed"
            }
            if ($release.tag_name -ceq $Tag) {
                $matches.Add($release)
            }
        }
        if ($releases.Count -lt 100) {
            $complete = $true
            break
        }
    }
    if (-not $complete) {
        throw "remote release inventory exceeded its bounded search"
    }
    if ($matches.Count -gt 1) {
        throw "remote release inventory contains a duplicate exact tag"
    }
    if ($matches.Count -eq 0) {
        return $null
    }
    return $matches[0]
}

function Wait-RustyFleetRemoteReleaseByTag {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [ValidatePattern("^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")]
        [string] $GitHubRepository,

        [Parameter(Mandatory)]
        [ValidatePattern("^v[0-9]+\.[0-9]+\.[0-9]+(?:-alpha\.[1-9][0-9]*)?$")]
        [string] $Tag,

        [Parameter(Mandatory)]
        [string] $GhExecutable,

        [ValidateRange(1, 120)]
        [int] $MaximumAttempts = 60,

        [ValidateRange(100, 5000)]
        [int] $DelayMilliseconds = 1000
    )

    for ($attempt = 1; $attempt -le $MaximumAttempts; $attempt++) {
        $release = Get-RustyFleetRemoteReleaseByTag `
            -GitHubRepository $GitHubRepository `
            -Tag $Tag `
            -GhExecutable $GhExecutable
        if ($null -ne $release) {
            return $release
        }
        if ($attempt -lt $MaximumAttempts) {
            Start-Sleep -Milliseconds $DelayMilliseconds
        }
    }
    return $null
}

function Get-RustyFleetRemoteReleaseId {
    param(
        [Parameter(Mandatory)][object] $Release,
        [Parameter(Mandatory)][string] $Tag,
        [Parameter(Mandatory)][bool] $ExpectedDraft,
        [Parameter(Mandatory)][bool] $ExpectedPrerelease
    )

    if (-not (Test-RustyFleetRemoteProperty $Release "id") -or
        -not (Test-RustyFleetRemoteProperty $Release "tag_name") -or
        -not (Test-RustyFleetRemoteProperty $Release "draft") -or
        -not (Test-RustyFleetRemoteProperty $Release "prerelease") -or
        -not (Test-RustyFleetRemoteProperty $Release "assets") -or
        $Release.tag_name -isnot [string] -or
        $Release.tag_name -cne $Tag -or
        $Release.draft -isnot [bool] -or
        $Release.draft -ne $ExpectedDraft -or
        $Release.prerelease -isnot [bool] -or
        $Release.prerelease -ne $ExpectedPrerelease -or
        $Release.assets -isnot [Collections.IList] -or
        -not (Test-RustyFleetRemoteInteger $Release.id)) {
        throw "remote release state is malformed or not resumable"
    }
    $releaseId = [long] $Release.id
    if ($releaseId -le 0) {
        throw "remote release id is malformed"
    }
    return $releaseId
}

function Assert-RustyFleetRemoteAssetInventory {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][object[]] $ExpectedAssets,
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]] $RemoteAssets,
        [switch] $AllowMissing
    )

    $expectedByName = [Collections.Generic.Dictionary[string, object]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($expected in $ExpectedAssets) {
        if (-not (Test-RustyFleetRemoteProperty $expected "name") -or
            -not (Test-RustyFleetRemoteProperty $expected "sha256") -or
            -not (Test-RustyFleetRemoteProperty $expected "size_bytes") -or
            $expected.name -isnot [string] -or
            $expected.name -cnotmatch "^[A-Za-z0-9._-]+$" -or
            $expected.name -cne [IO.Path]::GetFileName($expected.name) -or
            $expected.sha256 -isnot [string] -or
            $expected.sha256 -cnotmatch "^[0-9a-f]{64}$" -or
            -not (Test-RustyFleetRemoteInteger $expected.size_bytes) -or
            [long] $expected.size_bytes -le 0 -or
            -not $expectedByName.TryAdd([string] $expected.name, $expected)) {
            throw "closed local publication inventory is malformed"
        }
    }
    if ($expectedByName.Count -eq 0) {
        throw "closed local publication inventory is empty"
    }

    $observedNames = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($remote in $RemoteAssets) {
        if (-not (Test-RustyFleetRemoteProperty $remote "name") -or
            -not (Test-RustyFleetRemoteProperty $remote "size") -or
            -not (Test-RustyFleetRemoteProperty $remote "digest") -or
            -not (Test-RustyFleetRemoteProperty $remote "state") -or
            $remote.name -isnot [string] -or
            -not $observedNames.Add([string] $remote.name) -or
            -not $expectedByName.ContainsKey([string] $remote.name)) {
            throw "remote release contains an extra, duplicate, or malformed asset"
        }
        $expected = $expectedByName[[string] $remote.name]
        if ($remote.digest -isnot [string] -or
            $remote.digest -cnotmatch "^sha256:[0-9a-f]{64}$" -or
            $remote.digest.Substring(7) -cne $expected.sha256 -or
            -not (Test-RustyFleetRemoteInteger $remote.size) -or
            [long] $remote.size -ne [long] $expected.size_bytes -or
            $remote.state -isnot [string] -or
            $remote.state -cne "uploaded") {
            throw "remote release asset digest, size, or state is not exact"
        }
    }

    $missing = [Collections.Generic.List[string]]::new()
    foreach ($expectedName in $expectedByName.Keys) {
        if (-not $observedNames.Contains($expectedName)) {
            $missing.Add($expectedName)
        }
    }
    if (-not $AllowMissing -and $missing.Count -ne 0) {
        throw "remote release asset inventory is incomplete"
    }
    return @($missing | Sort-Object)
}

function Publish-RustyFleetGitHubRelease {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [ValidatePattern("^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")]
        [string] $GitHubRepository,

        [Parameter(Mandatory)]
        [ValidatePattern("^v[0-9]+\.[0-9]+\.[0-9]+(?:-alpha\.[1-9][0-9]*)?$")]
        [string] $Tag,

        [Parameter(Mandatory)]
        [ValidatePattern("^[0-9a-f]{40}$")]
        [string] $ExpectedSourceRevision,

        [Parameter(Mandatory)]
        [object[]] $AssetInventory,

        [Parameter(Mandatory)]
        [bool] $Prerelease,

        [Parameter(Mandatory)]
        [string] $GhExecutable,

        [Parameter(Mandatory)]
        [scriptblock] $AssertLocalState
    )

    $assetsByName = [Collections.Generic.Dictionary[string, object]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($asset in $AssetInventory) {
        if (-not (Test-RustyFleetRemoteProperty $asset "path") -or
            $asset.path -isnot [string] -or
            -not [IO.Path]::IsPathFullyQualified([string] $asset.path) -or
            -not $assetsByName.TryAdd([string] $asset.name, $asset)) {
            throw "publication upload inventory is malformed"
        }
    }
    [void] (Assert-RustyFleetRemoteAssetInventory `
        -ExpectedAssets $AssetInventory `
        -RemoteAssets @() `
        -AllowMissing)

    & $AssertLocalState
    [void] (Resolve-RustyFleetRemoteTagCommit `
        -GitHubRepository $GitHubRepository `
        -Tag $Tag `
        -ExpectedSourceRevision $ExpectedSourceRevision `
        -GhExecutable $GhExecutable)
    & $AssertLocalState

    $release = Get-RustyFleetRemoteReleaseByTag `
        -GitHubRepository $GitHubRepository `
        -Tag $Tag `
        -GhExecutable $GhExecutable
    $resumed = $null -ne $release
    if ($null -eq $release) {
        $createArguments = @(
            "release",
            "create",
            $Tag,
            "--repo",
            $GitHubRepository,
            "--draft",
            "--verify-tag",
            "--generate-notes"
        )
        if ($Prerelease) {
            $createArguments += "--prerelease"
        }
        [void] (Invoke-RustyFleetGh `
            -GhExecutable $GhExecutable `
            -Arguments $createArguments `
            -Context "draft release creation")
        $release = Wait-RustyFleetRemoteReleaseByTag `
            -GitHubRepository $GitHubRepository `
            -Tag $Tag `
            -GhExecutable $GhExecutable
        if ($null -eq $release) {
            throw "created draft release is absent from exact remote inventory"
        }
    }

    $releaseId = Get-RustyFleetRemoteReleaseId `
        -Release $release `
        -Tag $Tag `
        -ExpectedDraft $true `
        -ExpectedPrerelease $Prerelease
    $missing = @(Assert-RustyFleetRemoteAssetInventory `
        -ExpectedAssets $AssetInventory `
        -RemoteAssets @($release.assets) `
        -AllowMissing)
    $uploadedCount = 0
    if ($missing.Count -ne 0) {
        & $AssertLocalState
        [void] (Resolve-RustyFleetRemoteTagCommit `
            -GitHubRepository $GitHubRepository `
            -Tag $Tag `
            -ExpectedSourceRevision $ExpectedSourceRevision `
            -GhExecutable $GhExecutable)
        & $AssertLocalState
        $uploadArguments = @(
            "release",
            "upload",
            $Tag
        )
        foreach ($name in $missing) {
            $uploadArguments += [string] $assetsByName[$name].path
        }
        $uploadArguments += @("--repo", $GitHubRepository)
        [void] (Invoke-RustyFleetGh `
            -GhExecutable $GhExecutable `
            -Arguments $uploadArguments `
            -Context "draft release asset upload" `
            -TimeoutSeconds 120)
        $uploadedCount = $missing.Count
    }

    & $AssertLocalState
    $release = Get-RustyFleetRemoteReleaseByTag `
        -GitHubRepository $GitHubRepository `
        -Tag $Tag `
        -GhExecutable $GhExecutable
    if ($null -eq $release) {
        throw "draft release disappeared during verification"
    }
    $observedReleaseId = Get-RustyFleetRemoteReleaseId `
        -Release $release `
        -Tag $Tag `
        -ExpectedDraft $true `
        -ExpectedPrerelease $Prerelease
    if ($observedReleaseId -ne $releaseId) {
        throw "remote draft release identity changed"
    }
    [void] (Assert-RustyFleetRemoteAssetInventory `
        -ExpectedAssets $AssetInventory `
        -RemoteAssets @($release.assets))

    & $AssertLocalState
    [void] (Resolve-RustyFleetRemoteTagCommit `
        -GitHubRepository $GitHubRepository `
        -Tag $Tag `
        -ExpectedSourceRevision $ExpectedSourceRevision `
        -GhExecutable $GhExecutable)
    & $AssertLocalState
    $published = (Invoke-RustyFleetGhJson `
        -GhExecutable $GhExecutable `
        -Arguments @(
            "api",
            "--method",
            "PATCH",
            "repos/$GitHubRepository/releases/$releaseId",
            "-F",
            "draft=false"
        ) `
        -Context "draft release visibility transition").value
    [void] (Get-RustyFleetRemoteReleaseId `
        -Release $published `
        -Tag $Tag `
        -ExpectedDraft $false `
        -ExpectedPrerelease $Prerelease)

    $visible = Get-RustyFleetRemoteReleaseByTag `
        -GitHubRepository $GitHubRepository `
        -Tag $Tag `
        -GhExecutable $GhExecutable
    if ($null -eq $visible) {
        throw "visible release disappeared during verification"
    }
    $visibleId = Get-RustyFleetRemoteReleaseId `
        -Release $visible `
        -Tag $Tag `
        -ExpectedDraft $false `
        -ExpectedPrerelease $Prerelease
    if ($visibleId -ne $releaseId) {
        throw "visible release identity changed"
    }
    [void] (Assert-RustyFleetRemoteAssetInventory `
        -ExpectedAssets $AssetInventory `
        -RemoteAssets @($visible.assets))
    if ($Prerelease) {
        $latest = Get-RustyFleetLatestReleaseOrAbsent `
            -GitHubRepository $GitHubRepository `
            -GhExecutable $GhExecutable
        if ($null -ne $latest -and (
            -not (Test-RustyFleetRemoteProperty $latest "tag_name") -or
            $latest.tag_name -isnot [string] -or
            $latest.tag_name -cnotmatch "^v[0-9]+\.[0-9]+\.[0-9]+$" -or
            $latest.tag_name -ceq $Tag
        )) {
            throw "prerelease became the repository latest release"
        }
    }
    & $AssertLocalState
    [void] (Resolve-RustyFleetRemoteTagCommit `
        -GitHubRepository $GitHubRepository `
        -Tag $Tag `
        -ExpectedSourceRevision $ExpectedSourceRevision `
        -GhExecutable $GhExecutable)

    return [pscustomobject]@{
        draft_verified = $true
        visible_verified = $true
        remote_tag_verified = $true
        remote_integrity_verified = $true
        resumed_draft = $resumed
        uploaded_asset_count = $uploadedCount
    }
}

Export-ModuleMember -Function @(
    "Resolve-RustyFleetRemoteTagCommit",
    "Get-RustyFleetRemoteReleaseByTag",
    "Assert-RustyFleetRemoteAssetInventory",
    "Publish-RustyFleetGitHubRelease"
)
