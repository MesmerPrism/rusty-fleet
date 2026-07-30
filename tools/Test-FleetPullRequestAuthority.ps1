param(
    [Parameter(Mandatory = $true)][string]$RepositoryRoot,
    [Parameter(Mandatory = $true)][string]$TrustedVerifierRoot,
    [Parameter(Mandatory = $true)][ValidateRange(1, 2147483647)][int]$PullRequestNumber,
    [Parameter(Mandatory = $true)][string]$Repository,
    [Parameter(Mandatory = $true)][string]$RepositoryId,
    [Parameter(Mandatory = $true)][string]$RepositoryNodeId,
    [Parameter(Mandatory = $true)][string]$EventName,
    [Parameter(Mandatory = $true)][string]$EventAction,
    [Parameter(Mandatory = $true)][string]$EventRef,
    [Parameter(Mandatory = $true)][string]$EventSha,
    [Parameter(Mandatory = $true)][string]$BaseRepository,
    [Parameter(Mandatory = $true)][string]$BaseRef,
    [Parameter(Mandatory = $true)][string]$BaseCommit,
    [Parameter(Mandatory = $true)][string]$CandidateCommit,
    [AllowEmptyString()][string]$MergeCommit = "",
    [Parameter(Mandatory = $true)][string]$WorkflowRef,
    [Parameter(Mandatory = $true)][string]$WorkflowSha,
    [Parameter(Mandatory = $true)][string]$RunId,
    [Parameter(Mandatory = $true)][string]$RunAttempt,
    [Parameter(Mandatory = $true)][string]$Job,
    [Parameter(Mandatory = $true)][string]$EventPayloadPath,
    [Parameter(Mandatory = $true)][string]$OutDirectory,
    [switch]$OfflineTopologyMode,
    [string]$OfflineHeadRef = "",
    [string]$OfflineMergeRef = "",
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ExpectedRepository = "MesmerPrism/rusty-fleet"
$ExpectedRepositoryId = "1309815859"
$ExpectedRepositoryNodeId = "R_kgDOThI0Mw"
$ExpectedBaseRef = "main"
$ExpectedEventRef = "refs/heads/main"
$ExpectedEventName = "pull_request_target"
$ExpectedWorkflowRef = (
    "MesmerPrism/rusty-fleet/.github/workflows/" +
    "validation-authority.yml@refs/heads/main"
)
$ExpectedJob = "validation_authority"
$ExpectedVerifierCommit = "354545a63e870c3d89254f8fb78f6ed4060a8dc3"
$ExpectedVerifierTree = "1cf79cd4478e5dd5b940729b917a8beea41dac40"
$PolicyPath = "config/fleet-pull-request-authority.v1.json"
$FleetAssessmentSchemaPath = (
    "schemas/rusty.fleet.pull_request_authority_assessment.v1.schema.json"
)
$AdapterPath = "tools/Test-FleetPullRequestAuthority.ps1"
$WorkflowPath = ".github/workflows/validation-authority.yml"
$MaximumEventPayloadBytes = [int64]4194304
$MaximumTrustedArtifactBytes = [int64]16777216
$MaximumProcessOutputChars = 1048576
$GitTimeoutSeconds = 45
$VerifierTimeoutSeconds = 240

$VerifierArtifacts = @(
    [pscustomobject]@{
        path = "scripts/Test-ExternalValidationAuthority.ps1"
        object_id = "277a3bbbabfdedc66d50263a37e06bb094acac5f"
        size_bytes = [int64]34411
        sha256 = "89c3875c426eaa30500108644c0e2a89802d44049827aec2fe452358a5416c0e"
    },
    [pscustomobject]@{
        path = "schemas/external-validation-authority-policy-v1.schema.json"
        object_id = "1a7ed651d6cbcfccbf792ca60d41cc16301c407a"
        size_bytes = [int64]4217
        sha256 = "a89050065ea95d4f2d6edbf85c1d4e05802cef8c92c71684fb0d84e7cc616826"
    },
    [pscustomobject]@{
        path = "schemas/external-validation-authority-assessment-v1.schema.json"
        object_id = "2ad62c6c6f034538a8abc530e2cfecb7a43f8614"
        size_bytes = [int64]2769
        sha256 = "88b8b8a8d70cc5af50c9e43428017f971d04476b9235ebc545b634175e011426"
    }
)

$ForbiddenGitEnvironmentVariables = @(
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_NOSYSTEM",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_SYSTEM",
    "GIT_DIFF_OPTS",
    "GIT_DIR",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_GLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
    "GIT_INDEX_FILE",
    "GIT_LITERAL_PATHSPECS",
    "GIT_NAMESPACE",
    "GIT_NOGLOB_PATHSPECS",
    "GIT_OBJECT_DIRECTORY",
    "GIT_QUARANTINE_PATH",
    "GIT_REPLACE_REF_BASE",
    "GIT_SHALLOW_FILE",
    "GIT_WORK_TREE"
)

function Assert-Equal {
    param(
        [AllowNull()][object]$Actual,
        [AllowNull()][object]$Expected,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if (-not [string]::Equals(
        [string]$Actual,
        [string]$Expected,
        [StringComparison]::Ordinal
    )) {
        throw "$Label does not equal its trusted value."
    }
}

function Assert-LowerObjectId {
    param(
        [Parameter(Mandatory = $true)][string]$Value,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if ($Value -cnotmatch "^[0-9a-f]{40}$") {
        throw "$Label must be one lowercase SHA-1 Git object ID."
    }
}

function Assert-Decimal {
    param(
        [Parameter(Mandatory = $true)][string]$Value,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if ($Value -cnotmatch "^[1-9][0-9]{0,19}$") {
        throw "$Label must be one positive canonical decimal integer."
    }
}

function Assert-CleanGitEnvironment {
    $present = @(
        Get-ChildItem Env: |
            Where-Object {
                $name = [string]$_.Name
                $ForbiddenGitEnvironmentVariables -icontains $name -or
                $name.StartsWith(
                    "GIT_CONFIG_KEY_",
                    [StringComparison]::OrdinalIgnoreCase
                ) -or
                $name.StartsWith(
                    "GIT_CONFIG_VALUE_",
                    [StringComparison]::OrdinalIgnoreCase
                )
            } |
            ForEach-Object { [string]$_.Name } |
            Sort-Object -CaseSensitive
    )
    if ($present.Count -ne 0) {
        throw (
            "Ambient Git repository/object-store environment is forbidden: " +
            ($present -join ", ")
        )
    }
}

function Get-Sha256Bytes {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)

    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return (
            [BitConverter]::ToString($sha.ComputeHash($Bytes))
        ).Replace("-", "").ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function Get-Sha256File {
    param([Parameter(Mandatory = $true)][string]$Path)

    $stream = [IO.File]::Open(
        $Path,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::Read
    )
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return (
            [BitConverter]::ToString($sha.ComputeHash($stream))
        ).Replace("-", "").ToLowerInvariant()
    } finally {
        $sha.Dispose()
        $stream.Dispose()
    }
}

function Assert-StrictJson {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)

    if (-not ("RustyFleet.AuthorityStrictJson" -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Text.Json;

namespace RustyFleet
{
    public static class AuthorityStrictJson
    {
        public static void AssertNoDuplicateProperties(byte[] bytes)
        {
            Utf8JsonReader reader = new Utf8JsonReader(
                bytes,
                new JsonReaderOptions
                {
                    AllowTrailingCommas = false,
                    CommentHandling = JsonCommentHandling.Disallow,
                    MaxDepth = 64
                });
            Stack<HashSet<string>> objects = new Stack<HashSet<string>>();
            while (reader.Read())
            {
                if (reader.TokenType == JsonTokenType.StartObject)
                {
                    objects.Push(new HashSet<string>(StringComparer.OrdinalIgnoreCase));
                }
                else if (reader.TokenType == JsonTokenType.EndObject)
                {
                    if (objects.Count == 0) throw new JsonException("Unbalanced JSON object.");
                    objects.Pop();
                }
                else if (reader.TokenType == JsonTokenType.PropertyName)
                {
                    if (objects.Count == 0) throw new JsonException("Property outside object.");
                    string name = reader.GetString();
                    if (!objects.Peek().Add(name))
                        throw new JsonException("Duplicate or case-colliding property: " + name);
                }
            }
            if (objects.Count != 0) throw new JsonException("Unbalanced JSON object.");
        }
    }
}
'@
    }
    [RustyFleet.AuthorityStrictJson]::AssertNoDuplicateProperties($Bytes)
}

function Invoke-BoundedProcess {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds,
        [switch]$AllowFailure
    )

    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $FilePath
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $Arguments) {
        [void]$start.ArgumentList.Add($argument)
    }
    if (-not ("RustyFleet.BoundedProcessRunner" -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace RustyFleet
{
    public sealed class BoundedProcessResult
    {
        public int ExitCode { get; set; }
        public string Stdout { get; set; }
        public string Stderr { get; set; }
        public bool TimedOut { get; set; }
        public bool OutputExceeded { get; set; }
        public bool CaptureIncomplete { get; set; }
    }

    public static class BoundedProcessRunner
    {
        private static async Task DrainAsync(
            StreamReader reader,
            StringBuilder sink,
            int maximumChars,
            Action overflow)
        {
            char[] buffer = new char[4096];
            while (true)
            {
                int read = await reader.ReadAsync(
                    buffer,
                    0,
                    buffer.Length
                ).ConfigureAwait(false);
                if (read == 0)
                {
                    return;
                }
                int remaining;
                lock (sink)
                {
                    remaining = maximumChars - sink.Length;
                    if (remaining > 0)
                    {
                        sink.Append(buffer, 0, Math.Min(remaining, read));
                    }
                }
                if (read > remaining)
                {
                    overflow();
                    return;
                }
            }
        }

        private static void KillOwnedTree(Process process)
        {
            try
            {
                if (!process.HasExited)
                {
                    process.Kill(true);
                }
            }
            catch
            {
                // The exact owned process may already have exited.
            }
        }

        public static BoundedProcessResult Run(
            ProcessStartInfo start,
            int timeoutMilliseconds,
            int maximumChars)
        {
            using (Process process = new Process())
            {
                process.StartInfo = start;
                if (!process.Start())
                {
                    throw new InvalidOperationException(
                        "Failed to start the bounded process."
                    );
                }

                StringBuilder stdout = new StringBuilder();
                StringBuilder stderr = new StringBuilder();
                int outputExceeded = 0;
                Action overflow = delegate
                {
                    Interlocked.Exchange(ref outputExceeded, 1);
                    KillOwnedTree(process);
                };
                Task stdoutTask = DrainAsync(
                    process.StandardOutput,
                    stdout,
                    maximumChars,
                    overflow
                );
                Task stderrTask = DrainAsync(
                    process.StandardError,
                    stderr,
                    maximumChars,
                    overflow
                );

                bool exited = process.WaitForExit(timeoutMilliseconds);
                bool timedOut = !exited;
                if (!exited)
                {
                    KillOwnedTree(process);
                    exited = process.WaitForExit(5000);
                }
                bool captureComplete = exited && Task.WaitAll(
                    new Task[] { stdoutTask, stderrTask },
                    5000
                );
                if (!captureComplete)
                {
                    KillOwnedTree(process);
                    if (!exited)
                    {
                        exited = process.WaitForExit(5000);
                    }
                    if (exited)
                    {
                        captureComplete = Task.WaitAll(
                            new Task[] { stdoutTask, stderrTask },
                            5000
                        );
                    }
                }

                return new BoundedProcessResult
                {
                    ExitCode = exited ? process.ExitCode : -1,
                    Stdout = stdout.ToString(),
                    Stderr = stderr.ToString(),
                    TimedOut = timedOut,
                    OutputExceeded = outputExceeded != 0,
                    CaptureIncomplete = !exited || !captureComplete
                };
            }
        }
    }
}
'@
    }
    $capture = [RustyFleet.BoundedProcessRunner]::Run(
        $start,
        ($TimeoutSeconds * 1000),
        $MaximumProcessOutputChars
    )
    if ($capture.TimedOut) {
        throw "Process timed out after $TimeoutSeconds seconds: $FilePath"
    }
    if ($capture.OutputExceeded) {
        throw "Process output exceeded the bounded capture limit: $FilePath"
    }
    if ($capture.CaptureIncomplete) {
        throw "Process capture could not be reaped: $FilePath"
    }
    $result = [pscustomobject]@{
        exit_code = [int]$capture.ExitCode
        stdout = [string]$capture.Stdout
        stderr = [string]$capture.Stderr
    }
    if (-not $AllowFailure -and $result.exit_code -ne 0) {
        $summary = $result.stderr.Trim()
        if ($summary.Length -gt 1000) {
            $summary = $summary.Substring(0, 1000)
        }
        throw "Process failed ($($result.exit_code)): $FilePath`n$summary"
    }
    return $result
}

function Invoke-Git {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [switch]$AllowFailure
    )

    $argumentsWithRoot = @(
        "-c", "core.hooksPath=NUL",
        "-c", "protocol.file.allow=never",
        "-c", "protocol.ext.allow=never",
        "-C", $Root
    ) + $Arguments
    return Invoke-BoundedProcess `
        -FilePath "git" `
        -Arguments $argumentsWithRoot `
        -TimeoutSeconds $GitTimeoutSeconds `
        -AllowFailure:$AllowFailure
}

function Resolve-ExistingDirectory {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $full = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $full -PathType Container)) {
        throw "$Label is not an existing directory."
    }
    return $full.TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
}

function Test-IsWithin {
    param(
        [Parameter(Mandatory = $true)][string]$Child,
        [Parameter(Mandatory = $true)][string]$Parent
    )

    $parentFull = [IO.Path]::GetFullPath($Parent).TrimEnd("\", "/")
    $childFull = [IO.Path]::GetFullPath($Child).TrimEnd("\", "/")
    return (
        $childFull.Equals($parentFull, [StringComparison]::OrdinalIgnoreCase) -or
        $childFull.StartsWith(
            $parentFull + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase
        )
    )
}

function Assert-NoReparseAncestor {
    param([Parameter(Mandatory = $true)][string]$Path)

    $full = [IO.Path]::GetFullPath($Path)
    $current = Get-Item -LiteralPath $full -Force
    while ($null -ne $current) {
        if (
            ($current.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
        ) {
            throw "Output path has a reparse-point ancestor."
        }
        $parent = Split-Path -Parent $current.FullName
        if (-not $parent -or $parent -eq $current.FullName) {
            break
        }
        $current = Get-Item -LiteralPath $parent -Force
    }
}

function Assert-GitCheckout {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$ExpectedCommit,
        [Parameter(Mandatory = $true)][string]$ExpectedTree,
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)]
        [ValidateSet("fleet", "verifier", "any")]
        [string]$RemoteKind
    )

    $inside = (Invoke-Git $Root @("rev-parse", "--is-inside-work-tree")).stdout.Trim()
    Assert-Equal $inside "true" "$Label Git worktree state"
    $shallow = (
        Invoke-Git $Root @("rev-parse", "--is-shallow-repository")
    ).stdout.Trim()
    Assert-Equal $shallow "false" "$Label shallow state"
    $head = (Invoke-Git $Root @("rev-parse", "HEAD^{commit}")).stdout.Trim()
    Assert-Equal $head $ExpectedCommit "$Label HEAD"
    $tree = (Invoke-Git $Root @("rev-parse", "HEAD^{tree}")).stdout.Trim()
    Assert-Equal $tree $ExpectedTree "$Label tree"
    $status = (
        Invoke-Git $Root @(
            "status",
            "--porcelain=v1",
            "--untracked-files=all"
        )
    ).stdout
    if ($status.Length -ne 0) {
        throw "$Label checkout is dirty."
    }
    if ((Invoke-Git $Root @("replace", "-l")).stdout.Trim().Length -ne 0) {
        throw "$Label checkout contains replacement refs."
    }
    $gitDir = (Invoke-Git $Root @("rev-parse", "--absolute-git-dir")).stdout.Trim()
    foreach ($relative in @("info/grafts", "objects/info/alternates")) {
        $candidate = Join-Path $gitDir $relative
        if (
            (Test-Path -LiteralPath $candidate -PathType Leaf) -and
            (Get-Item -LiteralPath $candidate).Length -ne 0
        ) {
            throw "$Label checkout contains forbidden Git indirection."
        }
    }
    if ($RemoteKind -cne "any") {
        $remote = (Invoke-Git $Root @("remote", "get-url", "origin")).stdout.Trim()
        if ($RemoteKind -ceq "fleet") {
            $expected = "MesmerPrism/rusty-fleet"
        } else {
            $expected = "MesmerPrism/rusty-morphospace-work-environment"
        }
        if (
            $remote -cne "https://github.com/$expected" -and
            $remote -cne "https://github.com/$expected.git"
        ) {
            throw "$Label origin is not its expected HTTPS repository."
        }
    }
}

function Get-GitIdentity {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Revision,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $commit = (
        Invoke-Git $Root @("rev-parse", "$Revision^{commit}")
    ).stdout.Trim()
    Assert-LowerObjectId $commit "$Label commit"
    $tree = (
        Invoke-Git $Root @("rev-parse", "$commit^{tree}")
    ).stdout.Trim()
    Assert-LowerObjectId $tree "$Label tree"
    return [pscustomobject][ordered]@{
        commit = $commit
        tree = $tree
    }
}

function Get-GitBlobEntry {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Commit,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $result = Invoke-Git $Root @("ls-tree", "--full-name", $Commit, "--", $Path)
    $line = $result.stdout.TrimEnd("`r", "`n")
    if ($line -notmatch "^([0-9]{6}) blob ([0-9a-f]{40})`t(.+)$") {
        throw "Trusted artifact is absent or is not one regular Git blob: $Path"
    }
    Assert-Equal $Matches[1] "100644" "trusted artifact mode for $Path"
    Assert-Equal $Matches[3] $Path "trusted artifact path"
    $sizeText = (
        Invoke-Git $Root @("cat-file", "-s", $Matches[2])
    ).stdout.Trim()
    [int64]$size = 0
    if (-not [int64]::TryParse(
        $sizeText,
        [Globalization.NumberStyles]::None,
        [Globalization.CultureInfo]::InvariantCulture,
        [ref]$size
    )) {
        throw "Trusted artifact size is not canonical."
    }
    if ($size -lt 0 -or $size -gt $MaximumTrustedArtifactBytes) {
        throw "Trusted artifact exceeds its bounded size: $Path"
    }
    return [pscustomobject][ordered]@{
        path = $Path
        mode = "100644"
        object_id = $Matches[2]
        size_bytes = $size
    }
}

function Write-GitBlob {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$ObjectId,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    $parent = Split-Path -Parent $Destination
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        [void][IO.Directory]::CreateDirectory($parent)
    }
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = "git"
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in @(
        "-c", "core.hooksPath=NUL",
        "-c", "protocol.file.allow=never",
        "-c", "protocol.ext.allow=never",
        "-C", $Root,
        "cat-file", "blob", $ObjectId
    )) {
        [void]$start.ArgumentList.Add($argument)
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $stream = $null
    try {
        $stream = [IO.FileStream]::new(
            $Destination,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None
        )
        if (-not $process.Start()) {
            throw "Failed to start Git blob extraction."
        }
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $copyTask = $process.StandardOutput.BaseStream.CopyToAsync($stream)
        if (-not $process.WaitForExit($GitTimeoutSeconds * 1000)) {
            try {
                $process.Kill($true)
            } catch {
                # Best effort after the exact owned process timed out.
            }
            $process.WaitForExit()
            throw "Git blob extraction timed out."
        }
        [void]$copyTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        $stream.Flush($true)
        if ($process.ExitCode -ne 0) {
            throw "Git blob extraction failed: $($stderr.Trim())"
        }
    } finally {
        if ($null -ne $stream) {
            $stream.Dispose()
        }
        $process.Dispose()
    }
}

function Write-JsonCreateNew {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$JsonText
    )

    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($JsonText + "`n")
    $stream = [IO.FileStream]::new(
        $Path,
        [IO.FileMode]::CreateNew,
        [IO.FileAccess]::Write,
        [IO.FileShare]::None
    )
    try {
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally {
        $stream.Dispose()
    }
}

function Export-GitArtifactEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Commit,
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    $entry = Get-GitBlobEntry $Root $Commit $Path
    Write-GitBlob $Root $entry.object_id $Destination
    $file = Get-Item -LiteralPath $Destination
    Assert-Equal ([string]$file.Length) ([string]$entry.size_bytes) "artifact size"
    return [pscustomobject][ordered]@{
        path = $entry.path
        mode = $entry.mode
        object_id = $entry.object_id
        size_bytes = $entry.size_bytes
        sha256 = Get-Sha256File $Destination
    }
}

Assert-CleanGitEnvironment
Assert-Equal $Repository $ExpectedRepository "repository"
Assert-Equal $RepositoryId $ExpectedRepositoryId "repository REST ID"
Assert-Equal $RepositoryNodeId $ExpectedRepositoryNodeId "repository node ID"
Assert-Equal $BaseRepository $ExpectedRepository "base repository"
Assert-Equal $EventName $ExpectedEventName "event name"
if ($EventAction -cnotin @("opened", "synchronize", "reopened", "ready_for_review", "edited")) {
    throw "Event action is not admitted by the base-owned workflow."
}
Assert-Equal $EventRef $ExpectedEventRef "event ref"
Assert-Equal $BaseRef $ExpectedBaseRef "base ref"
Assert-Equal $WorkflowRef $ExpectedWorkflowRef "workflow ref"
Assert-Equal $Job $ExpectedJob "job ID"
Assert-LowerObjectId $EventSha "event SHA"
Assert-LowerObjectId $BaseCommit "base commit"
Assert-LowerObjectId $CandidateCommit "candidate commit"
if ($MergeCommit) {
    Assert-LowerObjectId $MergeCommit "merge commit"
}
Assert-LowerObjectId $WorkflowSha "workflow SHA"
Assert-Equal $EventSha $BaseCommit "event SHA"
Assert-Equal $WorkflowSha $BaseCommit "workflow SHA"
Assert-Decimal $RunId "run ID"
Assert-Decimal $RunAttempt "run attempt"

if ($OfflineTopologyMode) {
    if ($env:GITHUB_ACTIONS -ceq "true") {
        throw "Offline topology mode is forbidden in GitHub Actions."
    }
    if ($env:RUSTY_FLEET_AUTHORITY_SELF_TEST -cne "1") {
        throw "Offline topology mode requires the private self-test gate."
    }
    if (
        $OfflineHeadRef -cnotmatch "^refs/validation-authority-self-test/[a-z0-9-]+/head$" -or
        $OfflineMergeRef -cnotmatch "^refs/validation-authority-self-test/[a-z0-9-]+/merge$"
    ) {
        throw "Offline topology refs are outside the self-test namespace."
    }
} elseif ($OfflineHeadRef -or $OfflineMergeRef) {
    throw "Offline refs are forbidden outside offline topology mode."
}

$repositoryFull = Resolve-ExistingDirectory $RepositoryRoot "Fleet repository root"
$verifierFull = Resolve-ExistingDirectory $TrustedVerifierRoot "trusted verifier root"
$outputFull = Resolve-ExistingDirectory $OutDirectory "output directory"
if (
    (Test-IsWithin $outputFull $repositoryFull) -or
    (Test-IsWithin $outputFull $verifierFull)
) {
    throw "Output directory must remain outside both Git repositories."
}
Assert-NoReparseAncestor $outputFull
if (@(Get-ChildItem -LiteralPath $outputFull -Force).Count -ne 0) {
    throw "Output directory must start empty."
}

$eventFull = [IO.Path]::GetFullPath($EventPayloadPath)
if (-not (Test-Path -LiteralPath $eventFull -PathType Leaf)) {
    throw "GitHub event payload is absent."
}
if ((Get-Item -LiteralPath $eventFull).Length -gt $MaximumEventPayloadBytes) {
    throw "GitHub event payload exceeds the bounded size."
}
$eventBytes = [IO.File]::ReadAllBytes($eventFull)
Assert-StrictJson $eventBytes
$eventPayloadSha256 = Get-Sha256Bytes $eventBytes
$eventJson = [Text.UTF8Encoding]::new($false, $true).GetString($eventBytes)
$event = $eventJson | ConvertFrom-Json -Depth 40
Assert-Equal ([string]$event.action) $EventAction "payload action"
Assert-Equal ([string]$event.repository.full_name) $Repository "payload repository"
Assert-Equal ([string]$event.repository.id) $RepositoryId "payload repository REST ID"
Assert-Equal ([string]$event.repository.node_id) $RepositoryNodeId "payload repository node ID"
Assert-Equal ([string]$event.pull_request.number) ([string]$PullRequestNumber) "payload PR number"
Assert-Equal ([string]$event.pull_request.base.repo.full_name) $BaseRepository "payload base repository"
Assert-Equal ([string]$event.pull_request.base.ref) $BaseRef "payload base ref"
Assert-Equal ([string]$event.pull_request.base.sha) $BaseCommit "payload base commit"
$eventHeadCommit = [string]$event.pull_request.head.sha
Assert-Equal $eventHeadCommit $CandidateCommit "payload head commit"
$payloadMergeProperty = $event.pull_request.PSObject.Properties[
    "merge_commit_sha"
]
if ($null -eq $payloadMergeProperty) {
    throw "Payload merge commit field is absent."
}
$eventMergeCommit = $null
if ($null -ne $payloadMergeProperty.Value) {
    if ($payloadMergeProperty.Value -isnot [string]) {
        throw "Payload merge commit must be null or a canonical object ID."
    }
    $eventMergeCommit = [string]$payloadMergeProperty.Value
    Assert-LowerObjectId $eventMergeCommit "payload merge commit"
    if (-not $MergeCommit) {
        throw "Payload merge commit is present but the workflow argument is absent."
    }
    Assert-Equal $eventMergeCommit $MergeCommit "payload merge commit workflow argument"
} elseif ($MergeCommit) {
    throw "Payload merge commit is null but the workflow argument is present."
}

Assert-GitCheckout `
    -Root $repositoryFull `
    -ExpectedCommit $BaseCommit `
    -ExpectedTree ((Get-GitIdentity $repositoryFull $BaseCommit "base").tree) `
    -Label "Fleet base" `
    -RemoteKind $(if ($OfflineTopologyMode) { "any" } else { "fleet" })
Assert-GitCheckout `
    -Root $verifierFull `
    -ExpectedCommit $ExpectedVerifierCommit `
    -ExpectedTree $ExpectedVerifierTree `
    -Label "trusted verifier" `
    -RemoteKind $(if ($OfflineTopologyMode) { "any" } else { "verifier" })

$baseIdentity = Get-GitIdentity $repositoryFull $BaseCommit "base"
$verifierIdentity = Get-GitIdentity $verifierFull $ExpectedVerifierCommit "verifier"
Assert-Equal $verifierIdentity.tree $ExpectedVerifierTree "verifier tree"

if ($OfflineTopologyMode) {
    $headRef = $OfflineHeadRef
    $mergeRef = $OfflineMergeRef
} else {
    $namespace = (
        "refs/validation-authority/run-$RunId-attempt-$RunAttempt/" +
        "pr-$PullRequestNumber"
    )
    $headRef = "$namespace/head"
    $mergeRef = "$namespace/merge"
    $remote = "https://github.com/MesmerPrism/rusty-fleet.git"
    [void](Invoke-Git $repositoryFull @(
        "-c", "fetch.fsckObjects=true",
        "fetch",
        "--force",
        "--quiet",
        "--no-tags",
        "--no-recurse-submodules",
        $remote,
        "+refs/pull/$PullRequestNumber/head`:$headRef",
        "+refs/pull/$PullRequestNumber/merge`:$mergeRef"
    ))
}

Assert-GitCheckout `
    -Root $repositoryFull `
    -ExpectedCommit $BaseCommit `
    -ExpectedTree $baseIdentity.tree `
    -Label "Fleet base after PR fetch" `
    -RemoteKind $(if ($OfflineTopologyMode) { "any" } else { "fleet" })

$resolvedHead = (Invoke-Git $repositoryFull @("rev-parse", "$headRef^{commit}")).stdout.Trim()
$resolvedMerge = (Invoke-Git $repositoryFull @("rev-parse", "$mergeRef^{commit}")).stdout.Trim()
Assert-LowerObjectId $resolvedHead "server-owned PR head ref"
Assert-LowerObjectId $resolvedMerge "server-owned PR merge ref"
Assert-Equal $resolvedHead $eventHeadCommit "server-owned PR head ref to event head"

$ancestry = Invoke-Git $repositoryFull @(
    "merge-base", "--is-ancestor", $BaseCommit, $CandidateCommit
) -AllowFailure
if ($ancestry.exit_code -ne 0) {
    throw "Fleet base is not an ancestor of the PR head."
}
$parentLine = (
    Invoke-Git $repositoryFull @(
        "show", "-s", "--format=%P", $resolvedMerge
    )
).stdout.Trim()
[string[]]$mergeParents = @($parentLine -split " ")
if (
    $mergeParents.Count -ne 2 -or
    $mergeParents[0] -cne $BaseCommit -or
    $mergeParents[1] -cne $CandidateCommit
) {
    throw "Server-owned PR merge must have exact parents [base, head]."
}
$candidateIdentity = Get-GitIdentity $repositoryFull $CandidateCommit "candidate"
$mergeIdentity = Get-GitIdentity $repositoryFull $resolvedMerge "merge"
Assert-Equal $mergeIdentity.tree $candidateIdentity.tree "PR merge tree"

$trustedRoot = Join-Path $outputFull "_trusted"
[void][IO.Directory]::CreateDirectory($trustedRoot)
foreach ($artifact in $VerifierArtifacts) {
    $entry = Get-GitBlobEntry `
        -Root $verifierFull `
        -Commit $ExpectedVerifierCommit `
        -Path ([string]$artifact.path)
    Assert-Equal $entry.mode "100644" "verifier artifact mode"
    Assert-Equal $entry.object_id ([string]$artifact.object_id) "verifier artifact object"
    Assert-Equal (
        [string]$entry.size_bytes
    ) ([string]$artifact.size_bytes) "verifier artifact size"
    $destination = Join-Path $trustedRoot ([string]$artifact.path)
    Write-GitBlob `
        -Root $verifierFull `
        -ObjectId $entry.object_id `
        -Destination $destination
    $file = Get-Item -LiteralPath $destination
    Assert-Equal ([string]$file.Length) ([string]$artifact.size_bytes) "verifier artifact size"
    Assert-Equal (
        Get-Sha256File $destination
    ) ([string]$artifact.sha256) "verifier artifact SHA-256"
}

$adapterRawPath = Join-Path $trustedRoot "fleet-adapter.ps1"
$adapterEvidence = Export-GitArtifactEvidence `
    $repositoryFull `
    $BaseCommit `
    $AdapterPath `
    $adapterRawPath
$adapterRecords = @($adapterEvidence)
if ($adapterRecords.Count -ne 1) {
    $types = @($adapterRecords | ForEach-Object {
        $_.GetType().FullName
    })
    throw (
        "Fleet adapter evidence did not have exactly one record; types: " +
        ($types -join ", ")
    )
}
$adapterEvidence = $adapterRecords[0]
if ($adapterEvidence.PSObject.Properties.Name -cnotcontains "sha256") {
    throw (
        "Fleet adapter evidence omitted SHA-256; record type was " +
        $adapterEvidence.GetType().FullName
    )
}
Assert-Equal (
    Get-Sha256File $PSCommandPath
) $adapterEvidence.sha256 "executed adapter SHA-256"

$workflowRawPath = Join-Path $trustedRoot "validation-authority.yml"
$workflowEvidence = Export-GitArtifactEvidence `
    $repositoryFull `
    $BaseCommit `
    $WorkflowPath `
    $workflowRawPath

$policyRawPath = Join-Path $trustedRoot "fleet-policy.json"
$policyEvidence = Export-GitArtifactEvidence `
    $repositoryFull `
    $BaseCommit `
    $PolicyPath `
    $policyRawPath

$fleetSchemaPath = Join-Path $trustedRoot "fleet-assessment.schema.json"
$fleetSchemaEvidence = Export-GitArtifactEvidence `
    $repositoryFull `
    $BaseCommit `
    $FleetAssessmentSchemaPath `
    $fleetSchemaPath

$externalPath = Join-Path $outputFull "external-validation-authority-assessment.json"
$fleetPath = Join-Path $outputFull "fleet-pull-request-authority-assessment.json"
$pwsh = (Get-Process -Id $PID).Path
$verifierScript = Join-Path `
    $trustedRoot `
    "scripts/Test-ExternalValidationAuthority.ps1"
$verifierResult = Invoke-BoundedProcess `
    -FilePath $pwsh `
    -Arguments @(
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy", "Bypass",
        "-File", $verifierScript,
        "-RepositoryRoot", $repositoryFull,
        "-PolicyPath", $PolicyPath,
        "-Repository", $ExpectedRepository,
        "-BaseCommit", $BaseCommit,
        "-CandidateCommit", $CandidateCommit,
        "-OutPath", $externalPath,
        "-Json"
    ) `
    -TimeoutSeconds $VerifierTimeoutSeconds `
    -AllowFailure
if ($verifierResult.exit_code -ne 0) {
    throw "Trusted static admission failed."
}
if (-not (Test-Path -LiteralPath $externalPath -PathType Leaf)) {
    throw "Trusted verifier did not create its external assessment."
}
if ((Get-Item -LiteralPath $externalPath).Length -gt 1048576) {
    throw "External authority assessment exceeded its bounded size."
}
$externalBytes = [IO.File]::ReadAllBytes($externalPath)
Assert-StrictJson $externalBytes
$externalJson = [Text.UTF8Encoding]::new($false, $true).GetString($externalBytes)
$externalSchema = Join-Path `
    $trustedRoot `
    "schemas/external-validation-authority-assessment-v1.schema.json"
if (-not (Test-Json -Json $externalJson -SchemaFile $externalSchema -ErrorAction Stop)) {
    throw "External authority assessment failed its pinned schema."
}
$external = $externalJson | ConvertFrom-Json -Depth 30
if ($verifierResult.stdout.Trim().Length -eq 0) {
    throw "Trusted verifier returned no structured stdout."
}
$stdoutExternal = $verifierResult.stdout | ConvertFrom-Json -Depth 30
Assert-Equal (
    $stdoutExternal.candidate.commit
) $external.candidate.commit "verifier stdout candidate"
Assert-Equal $external.repository $ExpectedRepository "external repository"
Assert-Equal $external.base.commit $BaseCommit "external base commit"
Assert-Equal $external.base.tree $baseIdentity.tree "external base tree"
Assert-Equal $external.candidate.commit $CandidateCommit "external candidate commit"
Assert-Equal $external.candidate.tree $candidateIdentity.tree "external candidate tree"
Assert-Equal ([string]$external.candidate_code_executed) "False" "external candidate execution claim"
Assert-Equal ([string]$external.execution_attested) "False" "external execution claim"
Assert-Equal ([string]$external.publication_authority) "False" "external publication claim"
$externalSha256 = Get-Sha256Bytes $externalBytes

$assessment = [pscustomobject][ordered]@{
    schema = "rusty.fleet.pull_request_authority_assessment.v1"
    repository = [pscustomobject][ordered]@{
        full_name = $Repository
        repository_id = [int64]$RepositoryId
        repository_node_id = $RepositoryNodeId
    }
    pull_request = [pscustomobject][ordered]@{
        authority_mode = $(if ($OfflineTopologyMode) {
            "offline-self-test"
        } else {
            "hosted"
        })
        number = $PullRequestNumber
        event_name = $EventName
        event_action = $EventAction
        event_ref = $EventRef
        event_sha = $EventSha
        event_payload_sha256 = $eventPayloadSha256
        base_repository = $BaseRepository
        base_ref = $BaseRef
        base_commit = $baseIdentity.commit
        base_tree = $baseIdentity.tree
        candidate_commit = $candidateIdentity.commit
        candidate_tree = $candidateIdentity.tree
        event_merge_commit = $eventMergeCommit
        merge_commit = $mergeIdentity.commit
        merge_commit_source = "server-owned-pr-ref"
        merge_tree = $mergeIdentity.tree
        merge_parent_commits = @($mergeParents)
        head_private_ref = $headRef
        merge_private_ref = $mergeRef
    }
    run = [pscustomobject][ordered]@{
        run_id = [int64]$RunId
        run_attempt = [int64]$RunAttempt
        job = $Job
        workflow_ref = $WorkflowRef
        workflow_sha = $WorkflowSha
    }
    trusted_authority = [pscustomobject][ordered]@{
        fleet_workflow = $workflowEvidence
        fleet_policy = $policyEvidence
        fleet_adapter = $adapterEvidence
        fleet_assessment_schema = $fleetSchemaEvidence
        verifier = [pscustomobject][ordered]@{
            repository = "MesmerPrism/rusty-morphospace-work-environment"
            commit = $ExpectedVerifierCommit
            tree = $ExpectedVerifierTree
            script = [pscustomobject][ordered]@{
                path = [string]$VerifierArtifacts[0].path
                mode = "100644"
                object_id = [string]$VerifierArtifacts[0].object_id
                size_bytes = [int64]$VerifierArtifacts[0].size_bytes
                sha256 = [string]$VerifierArtifacts[0].sha256
            }
            policy_schema = [pscustomobject][ordered]@{
                path = [string]$VerifierArtifacts[1].path
                mode = "100644"
                object_id = [string]$VerifierArtifacts[1].object_id
                size_bytes = [int64]$VerifierArtifacts[1].size_bytes
                sha256 = [string]$VerifierArtifacts[1].sha256
            }
            assessment_schema = [pscustomobject][ordered]@{
                path = [string]$VerifierArtifacts[2].path
                mode = "100644"
                object_id = [string]$VerifierArtifacts[2].object_id
                size_bytes = [int64]$VerifierArtifacts[2].size_bytes
                sha256 = [string]$VerifierArtifacts[2].sha256
            }
        }
    }
    external_assessment = [pscustomobject][ordered]@{
        schema = [string]$external.schema
        file_name = "external-validation-authority-assessment.json"
        sha256 = $externalSha256
        policy_id = [string]$external.policy_id
        policy_sha256 = [string]$external.policy_sha256
    }
    decision = [string]$external.decision
    approval_id = $external.approval_id
    changed_paths = @($external.changed_paths)
    protected_paths = @($external.protected_paths)
    candidate_code_executed = $false
    execution_attested = $false
    publication_authority = $false
    limitations = @(
        "Static base-owned admission only; no candidate code was executed.",
        "This run-bound receipt is evidence, not a signature.",
        "Candidate paths are intentionally omitted from hosted summaries.",
        "Execution, owner effects, and publication require separate authority."
    )
}
$assessmentJson = $assessment | ConvertTo-Json -Depth 30
if (-not (
    Test-Json -Json $assessmentJson -SchemaFile $fleetSchemaPath -ErrorAction Stop
)) {
    throw "Fleet authority assessment failed its base-owned schema."
}
Write-JsonCreateNew $fleetPath $assessmentJson
$fleetSha256 = Get-Sha256File $fleetPath

$summary = [pscustomobject][ordered]@{
    schema = "rusty.fleet.pull_request_authority_summary.v1"
    decision = [string]$external.decision
    approval_id = $external.approval_id
    changed_path_count = @($external.changed_paths).Count
    protected_path_count = @($external.protected_paths).Count
    external_assessment_sha256 = $externalSha256
    fleet_assessment_sha256 = $fleetSha256
    candidate_code_executed = $false
    publication_authority = $false
}
if ($Json) {
    Write-Output ($summary | ConvertTo-Json -Depth 10 -Compress)
} else {
    Write-Output $summary
}
