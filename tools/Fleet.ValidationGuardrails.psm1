# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

Set-StrictMode -Version Latest
$script:FleetGuardrailSourceRoot = Split-Path -Parent $PSScriptRoot

function Invoke-FleetGit {
    param([string] $RepositoryRoot, [string[]] $Arguments)
    $output = @(& git -C $RepositoryRoot @Arguments)
    if ($LASTEXITCODE -ne 0) {
        throw "Git failed: git $($Arguments -join ' ')"
    }
    return $output
}

function Get-FleetSha256 {
    param([Parameter(Mandatory)][string] $Path)
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-FleetSha256Text {
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Value)
    $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
}

function Read-FleetJsonFileSnapshot {
    param([Parameter(Mandatory)][string] $Path)
    $bytes = [IO.File]::ReadAllBytes([IO.Path]::GetFullPath($Path))
    $content = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
    [ordered]@{
        sha256 = [Convert]::ToHexString(
            [Security.Cryptography.SHA256]::HashData($bytes)
        ).ToLowerInvariant()
        content = $content
        value = $content | ConvertFrom-Json -Depth 100
    }
}

function Invoke-FleetProcess {
    param(
        [Parameter(Mandatory)][string] $FilePath,
        [Parameter(Mandatory)][string[]] $Arguments,
        [AllowNull()][string] $WorkingDirectory
    )
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    if ($WorkingDirectory) { $startInfo.WorkingDirectory = $WorkingDirectory }
    foreach ($argument in $Arguments) {
        [void]$startInfo.ArgumentList.Add($argument)
    }
    $process = [Diagnostics.Process]::Start($startInfo)
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    [ordered]@{
        exit_code = $process.ExitCode
        stdout = $stdout.GetAwaiter().GetResult()
        stderr = $stderr.GetAwaiter().GetResult()
    }
}

function Invoke-FleetGitRaw {
    param([string] $RepositoryRoot, [string[]] $Arguments)
    $result = Invoke-FleetProcess "git" (@("-C", $RepositoryRoot) + $Arguments) $RepositoryRoot
    if ($result.exit_code -ne 0) {
        throw "Git failed: git $($Arguments -join ' '): $($result.stderr.Trim())"
    }
    [string]$result.stdout
}

function Assert-FleetValidationConfig {
    param([Parameter(Mandatory)][object] $Config)
    if ($Config.schema -ne "rusty.fleet.validation_risk_config.v1") {
        throw "Unsupported Fleet validation risk configuration schema."
    }
    $expectedProfiles = [ordered]@{ focused = 1; standard = 2; release = 3 }
    if (@($Config.profiles).Count -ne $expectedProfiles.Count) {
        throw "Fleet validation profiles must be exactly focused, standard, and release."
    }
    $seen = @{}
    foreach ($profile in $Config.profiles) {
        $id = [string]$profile.id
        if (-not ($expectedProfiles.Keys -ccontains $id) -or [int]$profile.rank -ne $expectedProfiles[$id]) {
            throw "Fleet validation profile registry is not canonical."
        }
        if ($seen.ContainsKey($id)) { throw "Duplicate Fleet validation profile '$id'." }
        $seen[$id] = $true
    }
    $expectedAliases = [ordered]@{ Quick = "focused"; Standard = "standard"; Deep = "release" }
    if (@($Config.aliases.PSObject.Properties).Count -ne $expectedAliases.Count) {
        throw "Fleet validation aliases must be exactly Quick, Standard, and Deep."
    }
    foreach ($alias in $expectedAliases.Keys) {
        if (
            -not ($Config.aliases.PSObject.Properties.Name -ccontains $alias) -or
            [string]$Config.aliases.$alias -cne $expectedAliases[$alias]
        ) {
            throw "Fleet validation alias '$alias' is not canonical."
        }
    }
    if ([string]$Config.default_profile -cne "focused") {
        throw "Fleet validation default profile must be focused."
    }
    if (@($Config.risk_rules).Count -eq 0) { throw "At least one Fleet risk rule is required." }
    $ruleIds = @{}
    foreach ($rule in $Config.risk_rules) {
        $ruleId = [string]$rule.id
        if ([string]::IsNullOrWhiteSpace($ruleId) -or $ruleIds.ContainsKey($ruleId)) {
            throw "Fleet risk rule IDs must be non-empty and unique."
        }
        $ruleIds[$ruleId] = $true
        if (-not ($expectedProfiles.Keys -ccontains [string]$rule.profile)) {
            throw "Fleet risk rule '$ruleId' names an unknown profile."
        }
        if (@($rule.patterns).Count -eq 0) { throw "Fleet risk rule '$ruleId' has no patterns." }
    }
    $checkProfiles = @{}
    foreach ($check in $Config.checks) {
        $minimum = [string]$check.minimum_profile
        if (-not ($expectedProfiles.Keys -ccontains $minimum) -or $checkProfiles.ContainsKey($minimum)) {
            throw "Fleet checks must register exactly one check per canonical profile."
        }
        $checkProfiles[$minimum] = $true
        if (
            [string]::IsNullOrWhiteSpace([string]$check.id) -or
            [string]::IsNullOrWhiteSpace([string]$check.file) -or
            [IO.Path]::IsPathRooted([string]$check.file) -or
            [string]$check.file -match "(^|[\\/])\.\.([\\/]|$)"
        ) {
            throw "Fleet validation check '$($check.id)' has an unsafe file path."
        }
        if ([int]$check.timeout_seconds -lt 1 -or [int]$check.timeout_seconds -gt 7200) {
            throw "Fleet validation check '$($check.id)' has an invalid timeout."
        }
    }
    if ($checkProfiles.Count -ne $expectedProfiles.Count) {
        throw "Fleet validation checks must cover every canonical profile."
    }
    if ([int]$Config.heartbeat_seconds -lt 1 -or [int]$Config.heartbeat_seconds -gt 300) {
        throw "Fleet validation heartbeat is outside the 1-to-300-second bound."
    }
    $receiptPath = [string]$Config.receipt_directory
    if (
        $receiptPath.Replace("\", "/") -cne "artifacts/validation" -or
        [IO.Path]::IsPathRooted($receiptPath) -or
        $receiptPath -match "(^|[\\/])\.\.([\\/]|$)"
    ) {
        throw "Fleet validation receipt directory must be exactly artifacts/validation."
    }
}

function ConvertTo-FleetGlobRegex {
    param([Parameter(Mandatory)][string] $Pattern)
    $escaped = [Regex]::Escape($Pattern.Replace("\", "/"))
    $escaped = $escaped.Replace("\*\*/", "(?:.*/)?")
    $escaped = $escaped.Replace("\*\*", ".*")
    $escaped = $escaped.Replace("\*", "[^/]*")
    "^$escaped$"
}

function Assert-FleetProductionValidationConfig {
    param([Parameter(Mandatory)][object] $Config)
    $expectedChecks = [ordered]@{
        focused = [ordered]@{
            id = "fleet-repository-focused"
            arguments = @("-Tier", "Quick", "-GuardrailChild")
        }
        standard = [ordered]@{
            id = "fleet-repository-standard"
            arguments = @("-Tier", "Standard", "-GuardrailChild")
        }
        release = [ordered]@{
            id = "fleet-repository-release"
            arguments = @("-Tier", "Deep", "-GuardrailChild")
        }
    }
    foreach ($profile in $expectedChecks.Keys) {
        $check = @($Config.checks | Where-Object {
            [string]$_.minimum_profile -ceq $profile
        })
        $expected = $expectedChecks[$profile]
        if (
            $check.Count -ne 1 -or
            [string]$check[0].id -cne [string]$expected.id -or
            [string]$check[0].file -cne "tools/Test-Repo.ps1" -or
            (@($check[0].arguments | ForEach-Object { [string]$_ }) -join "`0") -cne
                (@($expected.arguments) -join "`0")
        ) {
            throw "Fleet production check identity for '$profile' is not canonical."
        }
    }
    $requiredForbiddenPatterns = @(
        "(?i)(^|[\\/])adb(?:\.exe)?$",
        "(?i)\b(?:adb|fastboot)\b",
        "(?i)Invoke-FleetWifiAdbTwoQuestAcceptance\.ps1"
    )
    $actualForbiddenPatterns = @(
        $Config.forbidden_command_patterns | ForEach-Object { [string]$_ }
    )
    foreach ($pattern in $requiredForbiddenPatterns) {
        if (-not ($actualForbiddenPatterns -ccontains $pattern)) {
            throw "Fleet production validation removed required device-command rejection '$pattern'."
        }
    }
    $ranks = [ordered]@{ focused = 1; standard = 2; release = 3 }
    $authorityPaths = @(
        ".github/workflows/ci.yml",
        ".github/workflows/deep-validation.yml",
        ".gitignore",
        "AGENTS.md",
        "apps/fleet-setup/RustyFleet.Setup.csproj",
        "Directory.Build.props",
        "Directory.Packages.props",
        "config/fleet-validation-risk.v1.json",
        "global.json",
        "morphospace/feature.lock.json",
        "morphospace/project.spec.json",
        "packaging/windows/trust/release-policy.json",
        "packaging/windows/Publication.Remote.psm1",
        "packaging/windows/Publish-WindowsRelease.ps1",
        "packaging/windows/Sign-WindowsArtifacts.ps1",
        "schemas/rusty.fleet.change_risk_manifest.v1.schema.json",
        "schemas/rusty.fleet.validation_run_receipt.v1.schema.json",
        "tools/Fleet.ValidationGuardrails.psm1",
        "tools/FleetWifiAdbTwoQuestAcceptance.psm1",
        "tools/Invoke-FleetWifiAdbTwoQuestAcceptance.ps1",
        "tools/Invoke-FleetValidation.ps1",
        "tools/Test-FleetOnboardingSecurity.ps1",
        "tools/Test-FleetWifiAdbTwoQuestAcceptance.ps1",
        "tools/Test-FleetValidationGuardrails.ps1",
        "tools/Test-WindowsDistribution.ps1",
        "tools/Test-Repo.ps1"
    )
    foreach ($path in $authorityPaths) {
        $highest = 0
        foreach ($rule in $Config.risk_rules) {
            foreach ($pattern in $rule.patterns) {
                if ($path -cmatch (ConvertTo-FleetGlobRegex ([string]$pattern))) {
                    $highest = [Math]::Max($highest, [int]$ranks[[string]$rule.profile])
                }
            }
        }
        if ($highest -ne [int]$ranks.release) {
            throw "Fleet validation authority path '$path' must require release validation."
        }
    }
}

function Test-FleetProductionValidationConfig {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string] $ConfigPath)
    $configSnapshot = Read-FleetJsonFileSnapshot $ConfigPath
    $config = $configSnapshot.value
    Assert-FleetValidationConfig $config
    Assert-FleetProductionValidationConfig $config
    $true
}

function Resolve-FleetProfile {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][object] $Config,
        [AllowNull()][string] $RequestedProfile,
        [Parameter(Mandatory)][string] $RequiredProfile
    )
    Assert-FleetValidationConfig $Config
    $ranks = @{}
    foreach ($profile in $Config.profiles) {
        $ranks[[string]$profile.id] = [int]$profile.rank
    }
    $profileIds = @($Config.profiles | ForEach-Object { [string]$_.id })
    $canonical = if ([string]::IsNullOrWhiteSpace($RequestedProfile)) {
        [string]$RequiredProfile
    } elseif ($profileIds -ccontains $RequestedProfile) {
        $RequestedProfile
    } elseif ($Config.aliases.PSObject.Properties.Name -ccontains $RequestedProfile) {
        [string]$Config.aliases.$RequestedProfile
    } else {
        throw "Unknown validation profile '$RequestedProfile'."
    }
    if (-not $ranks.ContainsKey($canonical)) {
        throw "Profile mapping resolved outside the canonical profile registry."
    }
    if ($ranks[$canonical] -lt $ranks[$RequiredProfile]) {
        throw "Requested profile '$RequestedProfile' would downgrade required profile '$RequiredProfile'."
    }
    $canonical
}

function Get-FleetStatusEntries {
    param([Parameter(Mandatory)][string] $RepositoryRoot)
    $raw = Invoke-FleetGitRaw $RepositoryRoot @(
        "-c", "core.quotepath=false", "status", "--porcelain=v1", "-z", "--untracked-files=all"
    )
    $tokens = @($raw.Split([char]0, [StringSplitOptions]::RemoveEmptyEntries))
    $entries = @()
    for ($index = 0; $index -lt $tokens.Count; $index++) {
        $token = [string]$tokens[$index]
        if ($token.Length -lt 4) { throw "Malformed NUL-delimited Git status evidence." }
        $status = $token.Substring(0, 2)
        $path = $token.Substring(3).Replace("\", "/")
        $originalPath = $null
        if ($status.Contains("R") -or $status.Contains("C")) {
            $index++
            if ($index -ge $tokens.Count) { throw "Incomplete Git rename/copy status evidence." }
            $originalPath = ([string]$tokens[$index]).Replace("\", "/")
        }
        $entries += [ordered]@{
            status = $status
            path = $path
            original_path = $originalPath
        }
    }
    @($entries | Sort-Object path, original_path -CaseSensitive)
}

function Get-FleetGitSnapshot {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string] $RepositoryRoot)
    $root = [IO.Path]::GetFullPath($RepositoryRoot)
    $commit = @(Invoke-FleetGit $root @("rev-parse", "HEAD"))[0]
    $tree = @(Invoke-FleetGit $root @("rev-parse", "HEAD^{tree}"))[0]
    $branchValue = @(Invoke-FleetGit $root @("rev-parse", "--abbrev-ref", "HEAD"))[0]
    $branch = if ($branchValue -ceq "HEAD") { $null } else { [string]$branchValue }
    $indexRaw = Invoke-FleetGitRaw $root @(
        "-c", "core.quotepath=false", "ls-files", "--stage", "-z"
    )
    $status = @(Get-FleetStatusEntries $root)
    $pathEvidence = @()
    foreach ($entry in $status) {
        $relative = [string]$entry.path
        $full = Join-Path $root $relative
        $pathEvidence += [ordered]@{
            status = [string]$entry.status
            path = $relative
            original_path = $entry.original_path
            sha256 = if (Test-Path -LiteralPath $full -PathType Leaf) {
                Get-FleetSha256 $full
            } else { $null }
        }
    }
    [ordered]@{
        commit = $commit
        tree = $tree
        branch = $branch
        detached = $null -eq $branch
        index_sha256 = Get-FleetSha256Text $indexRaw
        clean = $status.Count -eq 0
        paths = $pathEvidence
    }
}

function Get-FleetChangedPaths {
    param(
        [string] $RepositoryRoot,
        [AllowNull()][string] $BaseCommit
    )
    $status = @(Get-FleetStatusEntries $RepositoryRoot)
    $statusPaths = @(
        foreach ($entry in $status) {
            [string]$entry.path
            if ($entry.original_path) { [string]$entry.original_path }
        }
    )
    if ($BaseCommit) {
        $resolvedBase = @(Invoke-FleetGit $RepositoryRoot @("rev-parse", "$BaseCommit^{commit}"))[0]
        & git -C $RepositoryRoot merge-base --is-ancestor $resolvedBase HEAD
        if ($LASTEXITCODE -ne 0) {
            throw "Base commit '$resolvedBase' is not an ancestor of HEAD."
        }
        $raw = Invoke-FleetGitRaw $RepositoryRoot @(
            "-c", "core.quotepath=false", "diff", "--name-only", "--no-renames", "-z",
            "--diff-filter=ACDMRTUXB", "$resolvedBase...HEAD", "--"
        )
        $committedPaths = @(
            $raw.Split([char]0, [StringSplitOptions]::RemoveEmptyEntries) |
                ForEach-Object { ([string]$_).Replace("\", "/") }
        )
        return [ordered]@{
            base_commit = $resolvedBase
            paths = @(($committedPaths + $statusPaths) | Sort-Object -Unique -CaseSensitive)
        }
    }
    [ordered]@{ base_commit = $null; paths = @($statusPaths | Sort-Object -Unique -CaseSensitive) }
}

function Get-FleetChangeRiskManifest {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string] $RepositoryRoot,
        [Parameter(Mandatory)][string] $ConfigPath,
        [AllowNull()][string] $BaseCommit
    )
    $configSnapshot = Read-FleetJsonFileSnapshot $ConfigPath
    $config = $configSnapshot.value
    Assert-FleetValidationConfig $config
    $snapshot = Get-FleetGitSnapshot $RepositoryRoot
    $changed = Get-FleetChangedPaths $RepositoryRoot $BaseCommit
    $ranks = @{}
    foreach ($profile in $config.profiles) { $ranks[[string]$profile.id] = [int]$profile.rank }
    $highest = [string]$config.default_profile
    $classifications = @()
    foreach ($path in $changed.paths) {
        $ruleMatches = @()
        foreach ($rule in $config.risk_rules) {
            foreach ($pattern in $rule.patterns) {
                if ($path -cmatch (ConvertTo-FleetGlobRegex $pattern)) {
                    $ruleMatches += $rule
                    break
                }
            }
        }
        if ($ruleMatches.Count -eq 0) { throw "No risk rule classifies '$path'." }
        $winner = $ruleMatches | Sort-Object @{ Expression = { $ranks[[string]$_.profile] }; Descending = $true }, id | Select-Object -First 1
        if ($ranks[[string]$winner.profile] -gt $ranks[$highest]) { $highest = [string]$winner.profile }
        $classifications += [ordered]@{ path = $path; rule_id = [string]$winner.id; profile = [string]$winner.profile }
    }
    $manifest = [ordered]@{
        schema = "rusty.fleet.change_risk_manifest.v1"
        repository = "rusty-fleet"
        git = [ordered]@{
            commit = $snapshot.commit
            tree = $snapshot.tree
            branch = $snapshot.branch
            detached = $snapshot.detached
            index_sha256 = $snapshot.index_sha256
            snapshot_sha256 = Get-FleetSha256Text (
                $snapshot | ConvertTo-Json -Depth 100 -Compress
            )
            base_commit = $changed.base_commit
            clean = $snapshot.clean
        }
        paths = @($changed.paths)
        classifications = $classifications
        required_profile = $highest
        config_sha256 = $configSnapshot.sha256
    }
    [void](Test-FleetChangeRiskManifest $manifest)
    [void](Test-FleetSchemaInstance $manifest "rusty.fleet.change_risk_manifest.v1.schema.json")
    $manifest
}

function Test-FleetSnapshotEqual {
    param([object] $Before, [object] $After)
    ($Before | ConvertTo-Json -Depth 100 -Compress) -ceq
        ($After | ConvertTo-Json -Depth 100 -Compress)
}

function Test-FleetChangeRiskManifest {
    [CmdletBinding()]
    param([Parameter(Mandatory)][object] $Manifest)
    if ($Manifest.schema -cne "rusty.fleet.change_risk_manifest.v1") {
        throw "Invalid Fleet change-risk manifest schema."
    }
    if ($Manifest.repository -cne "rusty-fleet") {
        throw "Invalid Fleet change-risk repository identity."
    }
    foreach ($sha in @(
        [string]$Manifest.git.commit,
        [string]$Manifest.git.tree
    )) {
        if ($sha -cnotmatch "^[0-9a-f]{40}$") { throw "Invalid Fleet Git object identity." }
    }
    foreach ($sha in @(
        [string]$Manifest.git.index_sha256,
        [string]$Manifest.git.snapshot_sha256,
        [string]$Manifest.config_sha256
    )) {
        if ($sha -cnotmatch "^[0-9a-f]{64}$") { throw "Invalid Fleet SHA-256 identity." }
    }
    if ($Manifest.git.base_commit -and [string]$Manifest.git.base_commit -cnotmatch "^[0-9a-f]{40}$") {
        throw "Invalid Fleet base-commit identity."
    }
    if ([bool]$Manifest.git.detached -eq [bool]$Manifest.git.branch) {
        throw "Fleet branch and detached evidence are inconsistent."
    }
    $paths = @($Manifest.paths | ForEach-Object { [string]$_ })
    if (($paths -join "`0") -cne (($paths | Sort-Object -Unique -CaseSensitive) -join "`0")) {
        throw "Fleet changed paths must be unique and case-sensitively sorted."
    }
    $profiles = [ordered]@{ focused = 1; standard = 2; release = 3 }
    $highest = 1
    $classified = @{}
    foreach ($classification in @($Manifest.classifications)) {
        $path = [string]$classification.path
        $profile = [string]$classification.profile
        if (
            -not ($paths -ccontains $path) -or
            $classified.ContainsKey($path) -or
            -not ($profiles.Keys -ccontains $profile) -or
            [string]::IsNullOrWhiteSpace([string]$classification.rule_id)
        ) {
            throw "Invalid Fleet path classification."
        }
        $classified[$path] = $true
        $highest = [Math]::Max($highest, [int]$profiles[$profile])
    }
    if ($classified.Count -ne $paths.Count) {
        throw "Every Fleet changed path must have exactly one classification."
    }
    $required = [string]$Manifest.required_profile
    if (-not ($profiles.Keys -ccontains $required) -or [int]$profiles[$required] -ne $highest) {
        throw "Fleet required profile does not equal the highest classified risk."
    }
    $true
}

function ConvertFrom-FleetReceiptTimestamp {
    param(
        [AllowNull()][object] $Value,
        [Parameter(Mandatory)][string] $FieldName
    )
    if ($null -eq $Value) {
        throw "Fleet receipt timestamp '$FieldName' is required."
    }
    if ($Value -is [DateTimeOffset]) { return [DateTimeOffset]$Value }
    if ($Value -is [DateTime]) { return [DateTimeOffset]$Value }
    try {
        [DateTimeOffset]::ParseExact(
            [string]$Value,
            "o",
            [Globalization.CultureInfo]::InvariantCulture
        )
    } catch {
        throw "Fleet receipt contains an invalid timestamp '$FieldName': '$Value'."
    }
}

function Test-FleetNullableStringEqual {
    param(
        [AllowNull()][object] $Left,
        [AllowNull()][object] $Right
    )
    if ($null -eq $Left -or $null -eq $Right) {
        return $null -eq $Left -and $null -eq $Right
    }
    ([string]$Left) -ceq ([string]$Right)
}

function Assert-FleetCommandReceipt {
    param(
        [Parameter(Mandatory)][object] $Command,
        [Parameter(Mandatory)][DateTimeOffset] $ReceiptStarted,
        [Parameter(Mandatory)][DateTimeOffset] $ReceiptFinished
    )
    $status = [string]$Command.status
    if ($status -cnotin @("planned", "passed", "failed", "timed_out", "not_run")) {
        throw "Fleet command receipt has an invalid status."
    }
    $survivors = @($Command.cleanup.survivors)
    $attempted = [bool]$Command.cleanup.attempted
    $completed = [bool]$Command.cleanup.completed
    if ((-not $attempted -and (-not $completed -or $survivors.Count -ne 0)) -or
        ($completed -and $survivors.Count -ne 0)) {
        throw "Fleet command receipt has incoherent cleanup evidence."
    }

    if ($status -cin @("planned", "not_run")) {
        if (
            $null -ne $Command.exit_code -or
            $null -ne $Command.started_at -or
            $null -ne $Command.finished_at -or
            $null -ne $Command.duration_ms -or
            [bool]$Command.timed_out -or
            $null -ne $Command.failure_reason -or
            $attempted -or
            -not $completed
        ) {
            throw "Fleet non-executed command receipt contains execution evidence."
        }
        return
    }

    $started = ConvertFrom-FleetReceiptTimestamp $Command.started_at "command.started_at"
    $finished = ConvertFrom-FleetReceiptTimestamp $Command.finished_at "command.finished_at"
    if ($finished -lt $started -or $started -lt $ReceiptStarted -or $finished -gt $ReceiptFinished) {
        throw "Fleet command receipt timestamps are outside the aggregate run interval."
    }
    $expectedDuration = [int64]($finished - $started).TotalMilliseconds
    if ($null -eq $Command.duration_ms -or [int64]$Command.duration_ms -ne $expectedDuration) {
        throw "Fleet command receipt duration does not match its timestamps."
    }

    if ($status -ceq "passed") {
        if (
            $null -eq $Command.exit_code -or
            [int]$Command.exit_code -ne 0 -or
            [bool]$Command.timed_out -or
            $null -ne $Command.failure_reason -or
            $attempted -or
            -not $completed
        ) {
            throw "Fleet passing command receipt has incoherent execution evidence."
        }
        return
    }

    if ($status -ceq "failed") {
        $reason = [string]$Command.failure_reason
        if (
            $null -eq $Command.exit_code -or
            [bool]$Command.timed_out -or
            $reason -cnotin @("nonzero-exit", "owned-child-leak", "cleanup-unverified") -or
            -not $attempted -or
            ($reason -ceq "nonzero-exit" -and [int]$Command.exit_code -eq 0) -or
            ($reason -ceq "cleanup-unverified" -and $completed) -or
            ($reason -cne "cleanup-unverified" -and -not $completed)
        ) {
            throw "Fleet failed command receipt has incoherent execution evidence."
        }
        return
    }

    $timeoutReason = [string]$Command.failure_reason
    if (
        $null -ne $Command.exit_code -or
        -not [bool]$Command.timed_out -or
        $timeoutReason -cnotin @("timeout", "cleanup-unverified") -or
        -not $attempted -or
        ($timeoutReason -ceq "cleanup-unverified" -and $completed) -or
        ($timeoutReason -ceq "timeout" -and -not $completed)
    ) {
        throw "Fleet timed-out command receipt has incoherent execution evidence."
    }
}

function Assert-FleetManifestConfigBinding {
    param(
        [Parameter(Mandatory)][object] $Receipt,
        [Parameter(Mandatory)][string] $ConfigPath
    )
    $configSnapshot = Read-FleetJsonFileSnapshot $ConfigPath
    if ($configSnapshot.sha256 -cne [string]$Receipt.config_sha256) {
        throw "Fleet receipt does not bind the exact validation configuration bytes."
    }
    $config = $configSnapshot.value
    Assert-FleetValidationConfig $config
    $ranks = @{}
    foreach ($profile in $config.profiles) {
        $ranks[[string]$profile.id] = [int]$profile.rank
    }
    $highest = [string]$config.default_profile
    foreach ($path in @($Receipt.risk_manifest.paths | ForEach-Object { [string]$_ })) {
        $ruleMatches = @()
        foreach ($rule in $config.risk_rules) {
            foreach ($pattern in $rule.patterns) {
                if ($path -cmatch (ConvertTo-FleetGlobRegex ([string]$pattern))) {
                    $ruleMatches += $rule
                    break
                }
            }
        }
        if ($ruleMatches.Count -eq 0) {
            throw "Fleet receipt configuration does not classify '$path'."
        }
        $winner = $ruleMatches |
            Sort-Object @{ Expression = { $ranks[[string]$_.profile] }; Descending = $true }, id |
            Select-Object -First 1
        $actual = @($Receipt.risk_manifest.classifications | Where-Object {
            [string]$_.path -ceq $path
        })
        if (
            $actual.Count -ne 1 -or
            [string]$actual[0].rule_id -cne [string]$winner.id -or
            [string]$actual[0].profile -cne [string]$winner.profile
        ) {
            throw "Fleet receipt classification for '$path' differs from the bound configuration."
        }
        if ($ranks[[string]$winner.profile] -gt $ranks[$highest]) {
            $highest = [string]$winner.profile
        }
    }
    if ([string]$Receipt.required_profile -cne $highest) {
        throw "Fleet receipt required profile differs from the bound configuration decision."
    }
    $expectedChecks = @($config.checks | Where-Object {
        [string]$_.minimum_profile -ceq [string]$Receipt.effective_profile
    })
    $commands = @($Receipt.commands)
    if ($expectedChecks.Count -ne $commands.Count) {
        throw "Fleet receipt command count differs from the bound validation configuration."
    }
    for ($index = 0; $index -lt $expectedChecks.Count; $index++) {
        $expected = $expectedChecks[$index]
        $command = $commands[$index]
        if (
            [string]$command.id -cne [string]$expected.id -or
            [string]$command.file -cne ([string]$expected.file).Replace("\", "/") -or
            (@($command.arguments | ForEach-Object { [string]$_ }) -join "`0") -cne
                (@($expected.arguments | ForEach-Object { [string]$_ }) -join "`0") -or
            [int]$command.timeout_seconds -ne [int]$expected.timeout_seconds
        ) {
            throw "Fleet receipt command identity differs from the bound validation configuration."
        }
    }
    $config
}

function Assert-FleetManifestPathBinding {
    param(
        [Parameter(Mandatory)][object] $Receipt,
        [Parameter(Mandatory)][string] $RepositoryRoot
    )
    $commit = @(Invoke-FleetGit $RepositoryRoot @(
        "rev-parse", "$([string]$Receipt.risk_manifest.git.commit)^{commit}"
    ))[0]
    $tree = @(Invoke-FleetGit $RepositoryRoot @("rev-parse", "$commit^{tree}"))[0]
    if (
        $commit -cne [string]$Receipt.risk_manifest.git.commit -or
        $tree -cne [string]$Receipt.risk_manifest.git.tree
    ) {
        throw "Fleet receipt Git objects do not match the risk manifest."
    }
    $expectedPaths = @(
        foreach ($entry in @($Receipt.git.before.paths)) {
            [string]$entry.path
            if ($entry.original_path) { [string]$entry.original_path }
        }
    )
    $base = $Receipt.risk_manifest.git.base_commit
    if ($base) {
        & git -C $RepositoryRoot merge-base --is-ancestor ([string]$base) $commit
        if ($LASTEXITCODE -ne 0) {
            throw "Fleet receipt base commit is not an ancestor of its source commit."
        }
        $raw = Invoke-FleetGitRaw $RepositoryRoot @(
            "-c", "core.quotepath=false", "diff", "--name-only", "--no-renames", "-z",
            "--diff-filter=ACDMRTUXB", "$([string]$base)...$commit", "--"
        )
        $expectedPaths += @(
            $raw.Split([char]0, [StringSplitOptions]::RemoveEmptyEntries) |
                ForEach-Object { ([string]$_).Replace("\", "/") }
        )
    }
    $expectedPaths = @($expectedPaths | Sort-Object -Unique -CaseSensitive)
    $manifestPaths = @($Receipt.risk_manifest.paths | ForEach-Object { [string]$_ })
    if (($expectedPaths -join "`0") -cne ($manifestPaths -join "`0")) {
        throw "Fleet receipt changed paths do not match its Git and dirty-overlay evidence."
    }
}

function Test-FleetValidationRunReceipt {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][object] $Receipt,
        [Parameter(Mandatory)][string] $RepositoryRoot,
        [Parameter(Mandatory)][string] $ConfigPath
    )
    if ($Receipt.schema -cne "rusty.fleet.validation_run_receipt.v1") {
        throw "Invalid Fleet validation receipt schema."
    }
    $parsedRunId = [guid]::Empty
    if (-not [guid]::TryParseExact([string]$Receipt.run_id, "D", [ref]$parsedRunId)) {
        throw "Invalid Fleet validation run ID."
    }
    [void](Test-FleetChangeRiskManifest $Receipt.risk_manifest)
    if ([string]$Receipt.config_sha256 -cne [string]$Receipt.risk_manifest.config_sha256) {
        throw "Fleet receipt and risk-manifest configuration identities differ."
    }
    $beforeFingerprint = Get-FleetSha256Text (
        $Receipt.git.before | ConvertTo-Json -Depth 100 -Compress
    )
    if ($beforeFingerprint -cne [string]$Receipt.risk_manifest.git.snapshot_sha256) {
        throw "Fleet receipt before-snapshot does not match its risk manifest."
    }
    foreach ($field in @("commit", "tree", "index_sha256")) {
        if (
            [string]$Receipt.git.before.$field -cne
                [string]$Receipt.risk_manifest.git.$field
        ) {
            throw "Fleet receipt before-snapshot '$field' differs from its risk manifest."
        }
    }
    if (
        -not (Test-FleetNullableStringEqual $Receipt.git.before.branch $Receipt.risk_manifest.git.branch) -or
        [bool]$Receipt.git.before.detached -ne [bool]$Receipt.risk_manifest.git.detached -or
        [bool]$Receipt.git.before.clean -ne [bool]$Receipt.risk_manifest.git.clean
    ) {
        throw "Fleet receipt before-snapshot routing state differs from its risk manifest."
    }

    $profiles = [ordered]@{ focused = 1; standard = 2; release = 3 }
    $aliases = [ordered]@{ Quick = "focused"; Standard = "standard"; Deep = "release" }
    $effective = [string]$Receipt.effective_profile
    $required = [string]$Receipt.required_profile
    if (
        -not ($profiles.Keys -ccontains $effective) -or
        -not ($profiles.Keys -ccontains $required)
    ) {
        throw "Fleet receipt contains an invalid validation-profile decision."
    }
    $requestedValue = $Receipt.requested_profile
    $requestedCanonical = if ($null -eq $requestedValue) {
        $required
    } elseif ($profiles.Keys -ccontains [string]$requestedValue) {
        [string]$requestedValue
    } elseif ($aliases.Keys -ccontains [string]$requestedValue) {
        [string]$aliases[[string]$requestedValue]
    } else {
        throw "Fleet receipt contains an unknown requested validation profile."
    }
    if (
        [int]$profiles[$requestedCanonical] -lt [int]$profiles[$required] -or
        $effective -cne $requestedCanonical
    ) {
        throw "Fleet receipt effective profile does not equal its upward-only requested decision."
    }
    if ($required -cne [string]$Receipt.risk_manifest.required_profile) {
        throw "Fleet receipt required profile differs from its risk manifest."
    }
    [void](Assert-FleetManifestConfigBinding $Receipt $ConfigPath)
    Assert-FleetManifestPathBinding $Receipt $RepositoryRoot

    if ([string]$Receipt.mode -cnotin @("plan", "execute")) {
        throw "Invalid Fleet validation mode."
    }
    if ([string]$Receipt.result -cnotin @(
        "planned", "in_progress", "passed", "failed", "rejected"
    )) {
        throw "Invalid Fleet validation terminal result."
    }
    if (@($Receipt.limitations).Count -eq 0) { throw "Fleet receipt limitations are required." }
    $started = ConvertFrom-FleetReceiptTimestamp $Receipt.started_at "started_at"
    $finished = ConvertFrom-FleetReceiptTimestamp $Receipt.finished_at "finished_at"
    $expectedDuration = [int64]($finished - $started).TotalMilliseconds
    if (
        $finished -lt $started -or
        [int64]$Receipt.duration_ms -ne $expectedDuration
    ) {
        throw "Fleet receipt duration does not match its timestamps."
    }
    $commands = @($Receipt.commands)
    if ($commands.Count -eq 0) { throw "Fleet receipt requires at least one command decision." }
    foreach ($command in $commands) {
        Assert-FleetCommandReceipt $command $started $finished
    }

    $hasAfter = $null -ne $Receipt.git.after
    $snapshotsEqual = $hasAfter -and
        (Test-FleetSnapshotEqual $Receipt.git.before $Receipt.git.after)
    $recomputedDrift = $hasAfter -and -not $snapshotsEqual
    if ([bool]$Receipt.git.drift_detected -ne $recomputedDrift) {
        throw "Fleet receipt drift flag does not match its exact before/after snapshots."
    }
    if ($Receipt.mode -ceq "plan") {
        if (
            $Receipt.result -cne "planned" -or
            $hasAfter -or
            [bool]$Receipt.git.drift_detected -or
            @($commands | Where-Object status -CNE "planned").Count -ne 0
        ) {
            throw "Fleet plan receipt claims execution."
        }
    } else {
        if ($Receipt.result -ceq "planned") {
            throw "Fleet execute receipt cannot have a planned aggregate result."
        }
        if ($Receipt.result -ceq "in_progress") {
            if (
                $hasAfter -or
                [bool]$Receipt.git.drift_detected -or
                @($commands | Where-Object status -CNE "planned").Count -ne 0
            ) {
                throw "Fleet in-progress receipt contains terminal execution evidence."
            }
            return $true
        }
        if ($hasAfter -and -not [bool]$Receipt.git.before.clean -and
            -not [bool]$Receipt.allow_dirty_source) {
            throw "Fleet executed receipt lacks explicit dirty-source authority."
        }
        if ($Receipt.result -ceq "passed") {
            if (
                -not $hasAfter -or
                -not $snapshotsEqual -or
                @($commands | Where-Object status -CNE "passed").Count -ne 0
            ) {
                throw "Fleet passing receipt lacks complete passing evidence."
            }
        } elseif ($Receipt.result -ceq "failed") {
            if (
                -not $hasAfter -or
                -not $snapshotsEqual -or
                @($commands | Where-Object status -In @("failed", "timed_out")).Count -eq 0 -or
                @($commands | Where-Object status -EQ "planned").Count -ne 0
            ) {
                throw "Fleet failed receipt lacks executed failure evidence."
            }
        } elseif ($Receipt.result -ceq "rejected") {
            if (-not $hasAfter) {
                if (
                    [bool]$Receipt.git.drift_detected -or
                    @($commands | Where-Object status -CNE "planned").Count -ne 0 -or
                    [bool]$Receipt.git.before.clean -or
                    [bool]$Receipt.allow_dirty_source
                ) {
                    throw "Fleet pre-execution rejection has inconsistent command evidence."
                }
            } elseif (
                $snapshotsEqual -or
                -not [bool]$Receipt.git.drift_detected -or
                @($commands | Where-Object status -EQ "planned").Count -ne 0
            ) {
                throw "Fleet post-execution rejection lacks exact drift evidence."
            }
        }
    }
    if ([bool]$Receipt.git.drift_detected -and $Receipt.result -cne "rejected") {
        throw "Fleet drift evidence must reject the run."
    }
    $true
}

function Test-FleetSchemaInstance {
    param(
        [Parameter(Mandatory)][object] $Value,
        [Parameter(Mandatory)][string] $SchemaFileName
    )
    $schemaPath = Join-Path $script:FleetGuardrailSourceRoot "schemas/$SchemaFileName"
    if (-not (Test-Path -LiteralPath $schemaPath -PathType Leaf)) {
        throw "Fleet schema projection is missing: $SchemaFileName"
    }
    $json = $Value | ConvertTo-Json -Depth 100
    if (-not (Test-Json -Json $json -SchemaFile $schemaPath -ErrorAction Stop)) {
        throw "Fleet JSON instance failed schema validation: $SchemaFileName"
    }
    $true
}

function Initialize-FleetValidationJobHost {
    if (-not $IsWindows) {
        throw "Fleet validation process containment currently requires Windows Job Objects."
    }
    if ("RustyFleet.ValidationJob" -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace RustyFleet
{
    public sealed class ValidationJob : IDisposable
    {
        private const uint CREATE_SUSPENDED = 0x00000004;
        private const uint CREATE_NO_WINDOW = 0x08000000;
        private const uint EXTENDED_STARTUPINFO_PRESENT = 0x00080000;
        private const uint GENERIC_READ = 0x80000000;
        private const uint GENERIC_WRITE = 0x40000000;
        private const uint FILE_SHARE_READ = 0x00000001;
        private const uint FILE_SHARE_WRITE = 0x00000002;
        private const uint FILE_SHARE_DELETE = 0x00000004;
        private const uint CREATE_ALWAYS = 2;
        private const uint OPEN_EXISTING = 3;
        private const uint FILE_ATTRIBUTE_NORMAL = 0x00000080;
        private const uint STARTF_USESTDHANDLES = 0x00000100;
        private const int PROC_THREAD_ATTRIBUTE_HANDLE_LIST = 0x00020002;
        private const uint JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000;
        private const uint WAIT_OBJECT_0 = 0;
        private const uint WAIT_TIMEOUT = 258;
        private const int JobObjectBasicAccountingInformation = 1;
        private const int JobObjectBasicProcessIdList = 3;
        private const int JobObjectExtendedLimitInformation = 9;

        private IntPtr job;
        private IntPtr process;
        public int ProcessId { get; private set; }

        private ValidationJob(IntPtr jobHandle, IntPtr processHandle, int processId)
        {
            job = jobHandle;
            process = processHandle;
            ProcessId = processId;
        }

        private static string QuoteArgument(string argument)
        {
            if (argument == null) argument = String.Empty;
            if (argument.Length == 0) return "\"\"";
            bool needsQuotes = false;
            foreach (char value in argument)
            {
                if (Char.IsWhiteSpace(value) || value == '"')
                {
                    needsQuotes = true;
                    break;
                }
            }
            if (!needsQuotes) return argument;

            StringBuilder quoted = new StringBuilder();
            quoted.Append('"');
            int backslashes = 0;
            foreach (char value in argument)
            {
                if (value == '\\')
                {
                    backslashes++;
                    continue;
                }
                if (value == '"')
                {
                    quoted.Append('\\', backslashes * 2 + 1);
                    quoted.Append('"');
                    backslashes = 0;
                    continue;
                }
                quoted.Append('\\', backslashes);
                backslashes = 0;
                quoted.Append(value);
            }
            quoted.Append('\\', backslashes * 2);
            quoted.Append('"');
            return quoted.ToString();
        }

        public static ValidationJob Start(
            string application,
            string[] arguments,
            string workingDirectory,
            string standardOutputPath,
            string standardErrorPath)
        {
            IntPtr jobHandle = IntPtr.Zero;
            IntPtr standardInputHandle = new IntPtr(-1);
            IntPtr standardOutputHandle = new IntPtr(-1);
            IntPtr standardErrorHandle = new IntPtr(-1);
            IntPtr attributeList = IntPtr.Zero;
            IntPtr inheritedHandles = IntPtr.Zero;
            bool attributeListInitialized = false;
            PROCESS_INFORMATION processInfo = new PROCESS_INFORMATION();
            try
            {
                jobHandle = CreateJobObjectW(IntPtr.Zero, null);
                if (jobHandle == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());

                JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if (!SetInformationJobObject(
                    jobHandle,
                    JobObjectExtendedLimitInformation,
                    ref limits,
                    (uint)Marshal.SizeOf<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }

                STARTUPINFOEX startup = new STARTUPINFOEX();
                startup.StartupInfo.cb = (uint)Marshal.SizeOf<STARTUPINFOEX>();
                SECURITY_ATTRIBUTES security = new SECURITY_ATTRIBUTES();
                security.nLength = (uint)Marshal.SizeOf<SECURITY_ATTRIBUTES>();
                security.bInheritHandle = true;
                standardInputHandle = CreateFileW(
                    "NUL",
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    ref security,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    IntPtr.Zero);
                standardOutputHandle = CreateFileW(
                    standardOutputPath,
                    GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    ref security,
                    CREATE_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL,
                    IntPtr.Zero);
                standardErrorHandle = CreateFileW(
                    standardErrorPath,
                    GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    ref security,
                    CREATE_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL,
                    IntPtr.Zero);
                if (standardInputHandle == new IntPtr(-1) ||
                    standardOutputHandle == new IntPtr(-1) ||
                    standardErrorHandle == new IntPtr(-1))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
                startup.StartupInfo.hStdInput = standardInputHandle;
                startup.StartupInfo.hStdOutput = standardOutputHandle;
                startup.StartupInfo.hStdError = standardErrorHandle;
                IntPtr attributeListSize = IntPtr.Zero;
                InitializeProcThreadAttributeList(
                    IntPtr.Zero,
                    1,
                    0,
                    ref attributeListSize);
                if (attributeListSize == IntPtr.Zero)
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                attributeList = Marshal.AllocHGlobal(attributeListSize);
                if (!InitializeProcThreadAttributeList(
                    attributeList,
                    1,
                    0,
                    ref attributeListSize))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                attributeListInitialized = true;
                inheritedHandles = Marshal.AllocHGlobal(IntPtr.Size * 3);
                Marshal.WriteIntPtr(inheritedHandles, 0, standardInputHandle);
                Marshal.WriteIntPtr(inheritedHandles, IntPtr.Size, standardOutputHandle);
                Marshal.WriteIntPtr(inheritedHandles, IntPtr.Size * 2, standardErrorHandle);
                if (!UpdateProcThreadAttribute(
                    attributeList,
                    0,
                    new IntPtr(PROC_THREAD_ATTRIBUTE_HANDLE_LIST),
                    inheritedHandles,
                    new IntPtr(IntPtr.Size * 3),
                    IntPtr.Zero,
                    IntPtr.Zero))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                startup.lpAttributeList = attributeList;
                StringBuilder commandLine = new StringBuilder(QuoteArgument(application));
                foreach (string argument in arguments)
                {
                    commandLine.Append(' ');
                    commandLine.Append(QuoteArgument(argument));
                }
                if (!CreateProcessWithAttributesW(
                    application,
                    commandLine,
                    IntPtr.Zero,
                    IntPtr.Zero,
                    true,
                    CREATE_SUSPENDED | CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
                    IntPtr.Zero,
                    workingDirectory,
                    ref startup,
                    out processInfo))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                DeleteProcThreadAttributeList(attributeList);
                attributeListInitialized = false;
                Marshal.FreeHGlobal(attributeList);
                attributeList = IntPtr.Zero;
                Marshal.FreeHGlobal(inheritedHandles);
                inheritedHandles = IntPtr.Zero;

                if (!AssignProcessToJobObject(jobHandle, processInfo.hProcess))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                if (ResumeThread(processInfo.hThread) == UInt32.MaxValue)
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                CloseHandle(processInfo.hThread);
                processInfo.hThread = IntPtr.Zero;
                CloseHandle(standardInputHandle);
                standardInputHandle = new IntPtr(-1);
                CloseHandle(standardOutputHandle);
                standardOutputHandle = new IntPtr(-1);
                CloseHandle(standardErrorHandle);
                standardErrorHandle = new IntPtr(-1);
                return new ValidationJob(jobHandle, processInfo.hProcess, unchecked((int)processInfo.dwProcessId));
            }
            catch
            {
                if (processInfo.hProcess != IntPtr.Zero)
                {
                    TerminateProcess(processInfo.hProcess, 1);
                    CloseHandle(processInfo.hProcess);
                }
                if (processInfo.hThread != IntPtr.Zero) CloseHandle(processInfo.hThread);
                if (attributeListInitialized) DeleteProcThreadAttributeList(attributeList);
                if (attributeList != IntPtr.Zero) Marshal.FreeHGlobal(attributeList);
                if (inheritedHandles != IntPtr.Zero) Marshal.FreeHGlobal(inheritedHandles);
                if (standardInputHandle != new IntPtr(-1)) CloseHandle(standardInputHandle);
                if (standardOutputHandle != new IntPtr(-1)) CloseHandle(standardOutputHandle);
                if (standardErrorHandle != new IntPtr(-1)) CloseHandle(standardErrorHandle);
                if (jobHandle != IntPtr.Zero) CloseHandle(jobHandle);
                throw;
            }
        }

        public bool WaitForExit(int milliseconds)
        {
            uint result = WaitForSingleObject(process, unchecked((uint)milliseconds));
            if (result == WAIT_OBJECT_0) return true;
            if (result == WAIT_TIMEOUT) return false;
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }

        public int ExitCode
        {
            get
            {
                if (!GetExitCodeProcess(process, out uint code))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                return unchecked((int)code);
            }
        }

        public uint ActiveProcessCount
        {
            get
            {
                if (!QueryInformationJobObject(
                    job,
                    JobObjectBasicAccountingInformation,
                    out JOBOBJECT_BASIC_ACCOUNTING_INFORMATION accounting,
                    (uint)Marshal.SizeOf<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>(),
                    IntPtr.Zero))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                return accounting.ActiveProcesses;
            }
        }

        public int[] GetProcessIds()
        {
            const int capacity = 4096;
            int bytes = 8 + (IntPtr.Size * capacity);
            IntPtr buffer = Marshal.AllocHGlobal(bytes);
            try
            {
                if (!QueryInformationJobObject(
                    job,
                    JobObjectBasicProcessIdList,
                    buffer,
                    (uint)bytes,
                    IntPtr.Zero))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                uint assigned = unchecked((uint)Marshal.ReadInt32(buffer, 0));
                uint count = unchecked((uint)Marshal.ReadInt32(buffer, 4));
                if (assigned > capacity || count > capacity)
                    throw new InvalidOperationException("Fleet validation Job Object process-list capacity exceeded.");
                int[] ids = new int[count];
                for (int index = 0; index < count; index++)
                {
                    long value = IntPtr.Size == 8
                        ? Marshal.ReadInt64(buffer, 8 + index * IntPtr.Size)
                        : Marshal.ReadInt32(buffer, 8 + index * IntPtr.Size);
                    ids[index] = checked((int)value);
                }
                return ids;
            }
            finally
            {
                Marshal.FreeHGlobal(buffer);
            }
        }

        public void Terminate()
        {
            if (!TerminateJobObject(job, 1))
                throw new Win32Exception(Marshal.GetLastWin32Error());
        }

        public void Dispose()
        {
            if (process != IntPtr.Zero)
            {
                CloseHandle(process);
                process = IntPtr.Zero;
            }
            if (job != IntPtr.Zero)
            {
                CloseHandle(job);
                job = IntPtr.Zero;
            }
            GC.SuppressFinalize(this);
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct IO_COUNTERS
        {
            public UInt64 ReadOperationCount;
            public UInt64 WriteOperationCount;
            public UInt64 OtherOperationCount;
            public UInt64 ReadTransferCount;
            public UInt64 WriteTransferCount;
            public UInt64 OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_BASIC_LIMIT_INFORMATION
        {
            public Int64 PerProcessUserTimeLimit;
            public Int64 PerJobUserTimeLimit;
            public uint LimitFlags;
            public UIntPtr MinimumWorkingSetSize;
            public UIntPtr MaximumWorkingSetSize;
            public uint ActiveProcessLimit;
            public UIntPtr Affinity;
            public uint PriorityClass;
            public uint SchedulingClass;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION
        {
            public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
            public IO_COUNTERS IoInfo;
            public UIntPtr ProcessMemoryLimit;
            public UIntPtr JobMemoryLimit;
            public UIntPtr PeakProcessMemoryUsed;
            public UIntPtr PeakJobMemoryUsed;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_BASIC_ACCOUNTING_INFORMATION
        {
            public Int64 TotalUserTime;
            public Int64 TotalKernelTime;
            public Int64 ThisPeriodTotalUserTime;
            public Int64 ThisPeriodTotalKernelTime;
            public uint TotalPageFaultCount;
            public uint TotalProcesses;
            public uint ActiveProcesses;
            public uint TotalTerminatedProcesses;
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct STARTUPINFO
        {
            public uint cb;
            public string lpReserved;
            public string lpDesktop;
            public string lpTitle;
            public uint dwX;
            public uint dwY;
            public uint dwXSize;
            public uint dwYSize;
            public uint dwXCountChars;
            public uint dwYCountChars;
            public uint dwFillAttribute;
            public uint dwFlags;
            public ushort wShowWindow;
            public ushort cbReserved2;
            public IntPtr lpReserved2;
            public IntPtr hStdInput;
            public IntPtr hStdOutput;
            public IntPtr hStdError;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct STARTUPINFOEX
        {
            public STARTUPINFO StartupInfo;
            public IntPtr lpAttributeList;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct SECURITY_ATTRIBUTES
        {
            public uint nLength;
            public IntPtr lpSecurityDescriptor;
            [MarshalAs(UnmanagedType.Bool)]
            public bool bInheritHandle;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct PROCESS_INFORMATION
        {
            public IntPtr hProcess;
            public IntPtr hThread;
            public uint dwProcessId;
            public uint dwThreadId;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateJobObjectW(IntPtr attributes, string name);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool SetInformationJobObject(
            IntPtr job,
            int informationClass,
            ref JOBOBJECT_EXTENDED_LIMIT_INFORMATION information,
            uint informationLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateJobObject(IntPtr job, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool QueryInformationJobObject(
            IntPtr job,
            int informationClass,
            out JOBOBJECT_BASIC_ACCOUNTING_INFORMATION information,
            uint informationLength,
            IntPtr returnLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool QueryInformationJobObject(
            IntPtr job,
            int informationClass,
            IntPtr information,
            uint informationLength,
            IntPtr returnLength);

        [DllImport("kernel32.dll", EntryPoint = "CreateProcessW", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool CreateProcessWithAttributesW(
            string applicationName,
            StringBuilder commandLine,
            IntPtr processAttributes,
            IntPtr threadAttributes,
            bool inheritHandles,
            uint creationFlags,
            IntPtr environment,
            string currentDirectory,
            ref STARTUPINFOEX startupInfo,
            out PROCESS_INFORMATION processInformation);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool InitializeProcThreadAttributeList(
            IntPtr attributeList,
            int attributeCount,
            int flags,
            ref IntPtr size);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool UpdateProcThreadAttribute(
            IntPtr attributeList,
            uint flags,
            IntPtr attribute,
            IntPtr value,
            IntPtr size,
            IntPtr previousValue,
            IntPtr returnSize);

        [DllImport("kernel32.dll")]
        private static extern void DeleteProcThreadAttributeList(IntPtr attributeList);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateFileW(
            string fileName,
            uint desiredAccess,
            uint shareMode,
            ref SECURITY_ATTRIBUTES securityAttributes,
            uint creationDisposition,
            uint flagsAndAttributes,
            IntPtr templateFile);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint ResumeThread(IntPtr thread);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateProcess(IntPtr process, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool CloseHandle(IntPtr handle);
    }
}
'@
}

function Invoke-FleetGuardrailCheck {
    param(
        [string] $RepositoryRoot,
        [object] $Check,
        [int] $HeartbeatSeconds,
        [string[]] $ForbiddenPatterns
    )
    $file = [string]$Check.file
    $arguments = @($Check.arguments | ForEach-Object { [string]$_ })
    $identity = "$file $($arguments -join ' ')"
    foreach ($pattern in $ForbiddenPatterns) {
        if ($identity -match $pattern) { throw "Forbidden device command in validation registry: $identity" }
    }
    $fullFile = [IO.Path]::GetFullPath((Join-Path $RepositoryRoot $file))
    if (-not $fullFile.StartsWith([IO.Path]::GetFullPath($RepositoryRoot), [StringComparison]::OrdinalIgnoreCase)) {
        throw "Validation command escapes the repository."
    }
    $stdout = [IO.Path]::GetTempFileName()
    $stderr = [IO.Path]::GetTempFileName()
    $started = [DateTimeOffset]::UtcNow
    $job = $null
    $timedOut = $false
    $cleanupAttempted = $false
    $cleanupCompleted = $true
    $survivors = @()
    $ownedChildLeak = $false
    $failureReason = $null
    $exitCode = $null
    try {
        Initialize-FleetValidationJobHost
        $pwshPath = Join-Path $PSHOME "pwsh.exe"
        if (-not (Test-Path -LiteralPath $pwshPath -PathType Leaf)) {
            throw "The current PowerShell host executable is unavailable."
        }
        [string[]]$processArguments = @(
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            $fullFile
        ) + $arguments
        $job = [RustyFleet.ValidationJob]::Start(
            $pwshPath,
            $processArguments,
            [IO.Path]::GetFullPath($RepositoryRoot),
            $stdout,
            $stderr
        )
        $deadline = $started.AddSeconds([int]$Check.timeout_seconds)
        $nextHeartbeat = $started.AddSeconds($HeartbeatSeconds)
        $exited = $false
        while (-not $exited) {
            if ([DateTimeOffset]::UtcNow -ge $deadline) {
                $timedOut = $true
                $cleanupAttempted = $true
                $failureReason = "timeout"
                break
            }
            if ([DateTimeOffset]::UtcNow -ge $nextHeartbeat) {
                Write-Host ("heartbeat check={0} pid={1} elapsed_seconds={2}" -f
                    $Check.id, $job.ProcessId, [int]([DateTimeOffset]::UtcNow - $started).TotalSeconds)
                $nextHeartbeat = $nextHeartbeat.AddSeconds($HeartbeatSeconds)
            }
            $exited = $job.WaitForExit(250)
        }
        if (-not $timedOut) {
            $exitCode = $job.ExitCode
            $ownedChildLeak = $job.ActiveProcessCount -gt 0
        }
        if ($timedOut -or $exitCode -ne 0 -or $ownedChildLeak) {
            $cleanupAttempted = $true
            if (-not $failureReason) {
                $failureReason = if ($ownedChildLeak) { "owned-child-leak" } else { "nonzero-exit" }
            }
            try {
                $job.Terminate()
                $cleanupDeadline = [DateTimeOffset]::UtcNow.AddSeconds(5)
                while ($job.ActiveProcessCount -gt 0 -and [DateTimeOffset]::UtcNow -lt $cleanupDeadline) {
                    Start-Sleep -Milliseconds 100
                }
                $survivors = @($job.GetProcessIds() | ForEach-Object { [int]$_ })
                $cleanupCompleted = $survivors.Count -eq 0
            } catch {
                $cleanupCompleted = $false
                $failureReason = "cleanup-unverified"
                Write-Warning "Fleet validation Job Object cleanup could not be verified: $($_.Exception.Message)"
            }
        }
        $finished = [DateTimeOffset]::UtcNow
        $standardOutput = Get-Content -LiteralPath $stdout -Raw -ErrorAction SilentlyContinue
        $standardError = Get-Content -LiteralPath $stderr -Raw -ErrorAction SilentlyContinue
        if ($standardOutput) { Write-Host $standardOutput }
        if ($standardError) { Write-Warning $standardError }
        [ordered]@{
            id = [string]$Check.id
            file = $file.Replace("\", "/")
            arguments = $arguments
            timeout_seconds = [int]$Check.timeout_seconds
            status = if ($timedOut) {
                "timed_out"
            } elseif ($exitCode -eq 0 -and -not $ownedChildLeak -and $cleanupCompleted) {
                "passed"
            } else {
                "failed"
            }
            exit_code = if ($timedOut) { $null } else { $exitCode }
            failure_reason = if (-not $cleanupCompleted) { "cleanup-unverified" } else { $failureReason }
            started_at = $started.ToString("o")
            finished_at = $finished.ToString("o")
            duration_ms = [int64]($finished - $started).TotalMilliseconds
            timed_out = $timedOut
            cleanup = [ordered]@{
                attempted = $cleanupAttempted
                completed = (-not $cleanupAttempted) -or $cleanupCompleted
                survivors = @($survivors | ForEach-Object { [int]$_ })
            }
        }
    } finally {
        if ($job) { $job.Dispose() }
        Remove-Item -LiteralPath $stdout, $stderr -Force -ErrorAction SilentlyContinue
    }
}

function Assert-FleetReceiptPathNoReparse {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $RepositoryRoot
    )
    $root = [IO.Path]::GetFullPath($RepositoryRoot).
        TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $current = [IO.Path]::GetFullPath((Split-Path -Parent $Path))
    $prefix = $root + [IO.Path]::DirectorySeparatorChar
    while ($current.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -Force -LiteralPath $current
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Fleet validation receipt ancestry contains a reparse point: $current"
            }
        }
        $parent = Split-Path -Parent $current
        if ($parent -ceq $current) { break }
        $current = $parent
    }
}

function Write-FleetAtomicJson {
    param(
        [string] $Path,
        [object] $Value,
        [string] $RepositoryRoot,
        [string] $ConfigPath
    )
    if ($Value.schema -ceq "rusty.fleet.validation_run_receipt.v1") {
        [void](Test-FleetValidationRunReceipt -Receipt $Value `
            -RepositoryRoot $RepositoryRoot -ConfigPath $ConfigPath)
        [void](Test-FleetSchemaInstance $Value "rusty.fleet.validation_run_receipt.v1.schema.json")
    } else {
        throw "Atomic Fleet JSON output does not recognize schema '$($Value.schema)'."
    }
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    Assert-FleetReceiptPathNoReparse $Path $RepositoryRoot
    $temporary = Join-Path $parent (".{0}.{1}.tmp" -f [IO.Path]::GetFileName($Path), [guid]::NewGuid())
    try {
        $Value | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $temporary -Encoding utf8NoBOM
        [IO.File]::Move($temporary, [IO.Path]::GetFullPath($Path), $false)
    } finally {
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
    }
}

function Resolve-FleetReceiptPath {
    param(
        [Parameter(Mandatory)][string] $RepositoryRoot,
        [Parameter(Mandatory)][object] $Config,
        [Parameter(Mandatory)][string] $RunId
    )
    $root = [IO.Path]::GetFullPath(
        (Join-Path $RepositoryRoot ([string]$Config.receipt_directory))
    ).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $target = Join-Path $root "$RunId.json"
    $pending = Join-Path $root "$RunId.in-progress.json"
    $prefix = $root + [IO.Path]::DirectorySeparatorChar
    if (-not $target.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Fleet validation receipts must stay inside artifacts/validation."
    }
    foreach ($candidate in @($target, $pending)) {
        if (Test-Path -LiteralPath $candidate) {
            throw "Fleet validation receipt already exists and will not be overwritten: $candidate"
        }
        Assert-FleetReceiptPathNoReparse $candidate $RepositoryRoot
    }
    $relative = [IO.Path]::GetRelativePath(
        [IO.Path]::GetFullPath($RepositoryRoot),
        $target
    ).Replace("\", "/")
    $ignored = Invoke-FleetProcess "git" @(
        "-C", [IO.Path]::GetFullPath($RepositoryRoot),
        "check-ignore", "--quiet", "--no-index", "--", $relative
    ) ([IO.Path]::GetFullPath($RepositoryRoot))
    if ($ignored.exit_code -ne 0) {
        throw "Fleet validation receipt target is not covered by the repository ignore policy."
    }
    [ordered]@{ final = $target; pending = $pending }
}

function Invoke-FleetValidationGuardrail {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string] $RepositoryRoot,
        [Parameter(Mandatory)][string] $ConfigPath,
        [AllowNull()][string] $Profile,
        [AllowNull()][string] $BaseCommit,
        [switch] $Execute,
        [switch] $AllowDirtySource
    )
    $started = [DateTimeOffset]::UtcNow
    $runId = [guid]::NewGuid().ToString()
    $loadedConfig = Read-FleetJsonFileSnapshot $ConfigPath
    $loadedConfigSha256 = $loadedConfig.sha256
    $config = $loadedConfig.value
    Assert-FleetValidationConfig $config
    $canonicalRepositoryRoot = [IO.Path]::GetFullPath($script:FleetGuardrailSourceRoot)
    if (
        [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd("\") -ceq
            $canonicalRepositoryRoot.TrimEnd("\")
    ) {
        $canonicalConfigPath = [IO.Path]::GetFullPath(
            (Join-Path $canonicalRepositoryRoot "config/fleet-validation-risk.v1.json")
        )
        if ([IO.Path]::GetFullPath($ConfigPath) -cne $canonicalConfigPath) {
            throw "Fleet production validation requires its canonical risk configuration path."
        }
        Assert-FleetProductionValidationConfig $config
    }
    $manifest = Get-FleetChangeRiskManifest $RepositoryRoot $ConfigPath $BaseCommit
    $confirmManifest = Get-FleetChangeRiskManifest $RepositoryRoot $ConfigPath $BaseCommit
    if (
        ($manifest | ConvertTo-Json -Depth 100 -Compress) -cne
        ($confirmManifest | ConvertTo-Json -Depth 100 -Compress)
    ) {
        throw "Fleet risk evidence drifted before validation selection."
    }
    if ($manifest.config_sha256 -cne $loadedConfigSha256) {
        throw "Fleet validation configuration changed while it was being loaded."
    }
    if (-not $BaseCommit -and $manifest.git.clean -and [string]::IsNullOrWhiteSpace($Profile)) {
        throw "A clean source requires -BaseCommit or an explicit validation profile."
    }
    $effective = Resolve-FleetProfile $config $Profile $manifest.required_profile
    $selected = @($config.checks | Where-Object { [string]$_.minimum_profile -eq $effective })
    if ($selected.Count -eq 0) { throw "No validation check is registered for '$effective'." }
    foreach ($check in $selected) {
        $identity = "$($check.file) $(@($check.arguments) -join ' ')"
        foreach ($pattern in $config.forbidden_command_patterns) {
            if ($identity -match $pattern) {
                throw "Forbidden device command in validation registry: $identity"
            }
        }
    }
    $before = Get-FleetGitSnapshot $RepositoryRoot
    $beforeFingerprint = Get-FleetSha256Text (
        $before | ConvertTo-Json -Depth 100 -Compress
    )
    if ($beforeFingerprint -cne $manifest.git.snapshot_sha256) {
        throw "Fleet source snapshot drifted after risk classification."
    }
    $commands = @()
    foreach ($check in $selected) {
        $commands += [ordered]@{
                id = [string]$check.id; file = [string]$check.file
                arguments = @($check.arguments); timeout_seconds = [int]$check.timeout_seconds
                status = "planned"; exit_code = $null; started_at = $null; finished_at = $null
                duration_ms = $null; timed_out = $false; failure_reason = $null
                cleanup = [ordered]@{ attempted = $false; completed = $true; survivors = @() }
        }
    }
    $targets = if ($Execute) {
        Resolve-FleetReceiptPath $RepositoryRoot $config $runId
    } else { $null }
    $provisional = [ordered]@{
        schema = "rusty.fleet.validation_run_receipt.v1"
        run_id = $runId
        mode = "execute"
        requested_profile = if ($Profile) { $Profile } else { $null }
        effective_profile = $effective
        required_profile = $manifest.required_profile
        allow_dirty_source = [bool]$AllowDirtySource
        config_sha256 = $manifest.config_sha256
        risk_manifest = $manifest
        git = [ordered]@{ before = $before; after = $null; drift_detected = $false }
        commands = $commands
        started_at = $started.ToString("o")
        finished_at = $started.ToString("o")
        duration_ms = 0
        result = "in_progress"
        limitations = [string[]]@(
            "Execution is in progress or was interrupted; this provisional receipt makes no pass claim."
        )
    }
    if ($Execute) {
        Write-FleetAtomicJson $targets.pending $provisional $RepositoryRoot $ConfigPath
    }
    if ($Execute -and -not $before.clean -and -not $AllowDirtySource) {
        $finished = [DateTimeOffset]::UtcNow
        $provisional.finished_at = $finished.ToString("o")
        $provisional.duration_ms = [int64]($finished - $started).TotalMilliseconds
        $provisional.result = "rejected"
        $provisional.limitations = [string[]]@(
            "Execution rejected because the source was dirty and -AllowDirtySource was not explicit.",
            "No validation command was executed."
        )
        Write-FleetAtomicJson $targets.final $provisional $RepositoryRoot $ConfigPath
        Remove-Item -LiteralPath $targets.pending -Force
        return $provisional
    }
    if ($Execute) {
        $prelaunchManifest = Get-FleetChangeRiskManifest $RepositoryRoot $ConfigPath $BaseCommit
        $prelaunchSnapshot = Get-FleetGitSnapshot $RepositoryRoot
        if (
            (Get-FleetSha256 $ConfigPath) -cne $loadedConfigSha256 -or
            ($prelaunchManifest | ConvertTo-Json -Depth 100 -Compress) -cne
                ($manifest | ConvertTo-Json -Depth 100 -Compress) -or
            (Get-FleetSha256Text ($prelaunchSnapshot | ConvertTo-Json -Depth 100 -Compress)) -cne
                $manifest.git.snapshot_sha256
        ) {
            throw "Fleet validation inputs drifted immediately before command launch."
        }
        $commands = @()
        foreach ($check in $selected) {
            $commands += Invoke-FleetGuardrailCheck $RepositoryRoot $check `
                ([int]$config.heartbeat_seconds) @($config.forbidden_command_patterns)
            if ($commands[-1].status -ne "passed") { break }
        }
    }
    $after = if ($Execute) { Get-FleetGitSnapshot $RepositoryRoot } else { $null }
    $drift = $Execute -and -not (Test-FleetSnapshotEqual $before $after)
    $finished = [DateTimeOffset]::UtcNow
    $failed = @($commands | Where-Object { $_.status -notin @("planned", "passed") }).Count -gt 0
    $receipt = [ordered]@{
        schema = "rusty.fleet.validation_run_receipt.v1"
        run_id = $runId
        mode = if ($Execute) { "execute" } else { "plan" }
        requested_profile = if ($Profile) { $Profile } else { $null }
        effective_profile = $effective
        required_profile = $manifest.required_profile
        allow_dirty_source = [bool]$AllowDirtySource
        config_sha256 = $manifest.config_sha256
        risk_manifest = $manifest
        git = [ordered]@{ before = $before; after = $after; drift_detected = $drift }
        commands = $commands
        started_at = $started.ToString("o")
        finished_at = $finished.ToString("o")
        duration_ms = [int64]($finished - $started).TotalMilliseconds
        result = if (-not $Execute) { "planned" } elseif ($drift) { "rejected" } elseif ($failed) { "failed" } else { "passed" }
        limitations = [string[]]@(
            if ($Execute) {
                "Source-only validation; no device behavior, signing, release, or publication is claimed.",
                "Each registered check is contained by a kill-on-close Windows Job Object before it begins execution."
            } else {
                "Plan only; no validation command was executed and no receipt file was written."
            }
        )
    }
    [void](Test-FleetValidationRunReceipt -Receipt $receipt `
        -RepositoryRoot $RepositoryRoot -ConfigPath $ConfigPath)
    [void](Test-FleetSchemaInstance $receipt "rusty.fleet.validation_run_receipt.v1.schema.json")
    if ($Execute) {
        Write-FleetAtomicJson $targets.final $receipt $RepositoryRoot $ConfigPath
        Remove-Item -LiteralPath $targets.pending -Force
    }
    $receipt
}

Export-ModuleMember -Function @(
    "Get-FleetChangeRiskManifest",
    "Get-FleetGitSnapshot",
    "Invoke-FleetValidationGuardrail",
    "Resolve-FleetProfile",
    "Test-FleetChangeRiskManifest",
    "Test-FleetProductionValidationConfig",
    "Test-FleetValidationRunReceipt"
)
