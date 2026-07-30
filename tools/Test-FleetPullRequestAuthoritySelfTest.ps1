param(
    [Parameter(Mandatory = $true)][string]$TrustedVerifierRoot,
    [Parameter(Mandatory = $true)][string]$GuardrailCandidateRoot,
    [switch]$KeepTemporaryFiles,
    [switch]$Json
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ExpectedVerifierCommit = "354545a63e870c3d89254f8fb78f6ed4060a8dc3"
$ExpectedGuardrailCandidate = "bee088f24277a5ee3537f04c729639ef204d4827"
$AdapterPath = Join-Path $PSScriptRoot "Test-FleetPullRequestAuthority.ps1"
$SourceRoot = Split-Path -Parent $PSScriptRoot
$SourceSchema = Join-Path `
    $SourceRoot `
    "schemas/rusty.fleet.pull_request_authority_assessment.v1.schema.json"
$MaximumCapturedChars = 1048576
$ProcessTimeoutSeconds = 300

function Invoke-Process {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string[]]$Arguments,
        [switch]$AllowFailure,
        [int]$TimeoutSeconds = 60
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
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) {
            throw "Failed to start self-test process: $FilePath"
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            try {
                $process.Kill($true)
            } catch {
                # Best effort after the exact owned process timed out.
            }
            $process.WaitForExit()
            throw "Self-test process timed out: $FilePath"
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if (
            $stdout.Length -gt $MaximumCapturedChars -or
            $stderr.Length -gt $MaximumCapturedChars
        ) {
            throw "Self-test process output exceeded its capture bound."
        }
        $result = [pscustomobject]@{
            exit_code = $process.ExitCode
            stdout = $stdout
            stderr = $stderr
        }
        if (-not $AllowFailure -and $result.exit_code -ne 0) {
            throw "Self-test process failed: $FilePath`n$($stderr.Trim())"
        }
        return $result
    } finally {
        $process.Dispose()
    }
}

function Invoke-Git {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [switch]$AllowFailure
    )

    return Invoke-Process `
        -FilePath "git" `
        -Arguments (@("-C", $Root) + $Arguments) `
        -AllowFailure:$AllowFailure
}

function Write-Utf8 {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Text
    )

    $parent = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        [void][IO.Directory]::CreateDirectory($parent)
    }
    [IO.File]::WriteAllText(
        $Path,
        $Text,
        [Text.UTF8Encoding]::new($false)
    )
}

function Copy-ExactSourceFile {
    param(
        [Parameter(Mandatory = $true)][string]$SourceRoot,
        [Parameter(Mandatory = $true)][string]$DestinationRoot,
        [Parameter(Mandatory = $true)][string]$RelativePath
    )

    $source = Join-Path $SourceRoot $RelativePath
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Required source bootstrap file is absent: $RelativePath"
    }
    $destination = Join-Path $DestinationRoot $RelativePath
    $parent = Split-Path -Parent $destination
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        [void][IO.Directory]::CreateDirectory($parent)
    }
    [IO.File]::Copy($source, $destination, $true)
}

function New-EventPayload {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Action,
        [Parameter(Mandatory = $true)][int]$Number,
        [Parameter(Mandatory = $true)][string]$BaseCommit,
        [Parameter(Mandatory = $true)][string]$HeadCommit,
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$MergeCommit,
        [string]$RepositoryId = "1309815859"
    )

    $payload = [pscustomobject][ordered]@{
        action = $Action
        repository = [pscustomobject][ordered]@{
            full_name = "MesmerPrism/rusty-fleet"
            id = [int64]$RepositoryId
            node_id = "R_kgDOThI0Mw"
        }
        pull_request = [pscustomobject][ordered]@{
            number = $Number
            base = [pscustomobject][ordered]@{
                ref = "main"
                sha = $BaseCommit
                repo = [pscustomobject][ordered]@{
                    full_name = "MesmerPrism/rusty-fleet"
                }
            }
            head = [pscustomobject][ordered]@{
                sha = $HeadCommit
            }
            merge_commit_sha = $MergeCommit
        }
    }
    Write-Utf8 $Path (($payload | ConvertTo-Json -Depth 20) + "`n")
}

function New-CommitTree {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Tree,
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [string[]]$Parents,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $arguments = @("commit-tree", $Tree)
    foreach ($parent in $Parents) {
        $arguments += @("-p", $parent)
    }
    $arguments += @("-m", $Message)
    return (Invoke-Git $Root $arguments).stdout.Trim()
}

if (-not (Test-Path -LiteralPath $AdapterPath -PathType Leaf)) {
    throw "Authority adapter is absent."
}
if (-not (Test-Path -LiteralPath $SourceSchema -PathType Leaf)) {
    throw "Fleet authority assessment schema is absent."
}
$verifierSource = [IO.Path]::GetFullPath($TrustedVerifierRoot)
if (-not (Test-Path -LiteralPath $verifierSource -PathType Container)) {
    throw "Trusted verifier source is absent."
}

$testRoot = Join-Path `
    ([IO.Path]::GetTempPath()) `
    ("rusty-fleet-authority-selftest-" + [Guid]::NewGuid().ToString("N"))
[void][IO.Directory]::CreateDirectory($testRoot)
$fleetRoot = Join-Path $testRoot "fleet"
$verifierRoot = Join-Path $testRoot "verifier"
$sentinel = Join-Path $testRoot "candidate-executed.sentinel"
$caseCount = 0
$passedCases = [Collections.Generic.List[string]]::new()
$pwsh = (Get-Process -Id $PID).Path
$previousSelfTestGate = $env:RUSTY_FLEET_AUTHORITY_SELF_TEST

try {
    [void](Invoke-Process "git" @(
        "clone", "--no-local", "--no-checkout", $verifierSource, $verifierRoot
    ))
    $checkoutVerifier = Invoke-Git `
        $verifierRoot `
        @("checkout", "--detach", $ExpectedVerifierCommit) `
        -AllowFailure
    if ($checkoutVerifier.exit_code -ne 0) {
        [void](Invoke-Git $verifierRoot @(
            "fetch",
            "--no-tags",
            $verifierSource,
            "refs/remotes/origin/main:refs/heads/trusted-main"
        ))
        [void](Invoke-Git $verifierRoot @(
            "checkout", "--detach", $ExpectedVerifierCommit
        ))
    }

    [void](Invoke-Process "git" @("init", $fleetRoot))
    [void](Invoke-Git $fleetRoot @("config", "user.name", "Fleet authority self-test"))
    [void](Invoke-Git $fleetRoot @("config", "user.email", "selftest@example.invalid"))
    [void](Invoke-Git $fleetRoot @("config", "core.autocrlf", "false"))
    [void](Invoke-Git $fleetRoot @("remote", "add", "origin", $fleetRoot))

    $adapterDestination = Join-Path `
        $fleetRoot `
        "tools/Test-FleetPullRequestAuthority.ps1"
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $adapterDestination))
    [IO.File]::Copy($AdapterPath, $adapterDestination, $false)
    $schemaDestination = Join-Path `
        $fleetRoot `
        "schemas/rusty.fleet.pull_request_authority_assessment.v1.schema.json"
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $schemaDestination))
    [IO.File]::Copy($SourceSchema, $schemaDestination, $false)
    Write-Utf8 `
        (Join-Path $fleetRoot ".github/workflows/validation-authority.yml") `
        "name: validation-authority-self-test`n"
    $policy = [pscustomobject][ordered]@{
        schema = "rusty.morphospace.workflow.external_validation_authority_policy.v1"
        policy_id = "rusty-fleet-pull-request-authority-v1"
        repository = "MesmerPrism/rusty-fleet"
        mandatory_protected_paths = @(
            "config/fleet-pull-request-authority.v1.json"
        )
        protected_rules = @(
            [pscustomobject][ordered]@{
                rule_id = "p01-github"
                match = "prefix"
                path = ".github/"
            }
        )
        approved_change_sets = @()
        status = "active"
    }
    Write-Utf8 `
        (Join-Path $fleetRoot "config/fleet-pull-request-authority.v1.json") `
        (($policy | ConvertTo-Json -Depth 20) + "`n")
    [void](Invoke-Git $fleetRoot @("add", "--all"))
    [void](Invoke-Git $fleetRoot @("commit", "-m", "Bootstrap trusted authority"))
    $base = (Invoke-Git $fleetRoot @("rev-parse", "HEAD")).stdout.Trim()
    $baseTree = (Invoke-Git $fleetRoot @("rev-parse", "HEAD^{tree}")).stdout.Trim()

    [void](Invoke-Git $fleetRoot @("checkout", "-b", "candidate"))
    $trapText = @"
if (`$env:RUSTY_FLEET_CANDIDATE_SENTINEL) {
    [IO.File]::WriteAllText(
        `$env:RUSTY_FLEET_CANDIDATE_SENTINEL,
        "candidate code executed"
    )
}
throw "Candidate trap must never execute."
"@
    Write-Utf8 (Join-Path $fleetRoot "tools/candidate-trap.ps1") $trapText
    [void](Invoke-Git $fleetRoot @("add", "--all"))
    [void](Invoke-Git $fleetRoot @("commit", "-m", "Add candidate trap"))
    $head = (Invoke-Git $fleetRoot @("rev-parse", "HEAD")).stdout.Trim()
    $headTree = (Invoke-Git $fleetRoot @("rev-parse", "HEAD^{tree}")).stdout.Trim()

    [void](Invoke-Git $fleetRoot @("checkout", "-b", "integration", $base))
    [void](Invoke-Git $fleetRoot @("merge", "--no-ff", "--no-commit", $head))
    [void](Invoke-Git $fleetRoot @("commit", "-m", "Synthetic server merge"))
    $merge = (Invoke-Git $fleetRoot @("rev-parse", "HEAD")).stdout.Trim()
    [void](Invoke-Git $fleetRoot @(
        "update-ref",
        "refs/validation-authority-self-test/baseline/head",
        $head
    ))
    [void](Invoke-Git $fleetRoot @(
        "update-ref",
        "refs/validation-authority-self-test/baseline/merge",
        $merge
    ))
    [void](Invoke-Git $fleetRoot @("checkout", "--detach", $base))

    $reversedMerge = New-CommitTree `
        $fleetRoot `
        $headTree `
        @($head, $base) `
        "Reversed parents"
    $extraMerge = New-CommitTree `
        $fleetRoot `
        $headTree `
        @($base, $head, $merge) `
        "Extra parent"
    $wrongTreeMerge = New-CommitTree `
        $fleetRoot `
        $baseTree `
        @($base, $head) `
        "Wrong merge tree"
    $unrelatedHead = New-CommitTree `
        $fleetRoot `
        $headTree `
        @() `
        "Unrelated head"
    $unrelatedMerge = New-CommitTree `
        $fleetRoot `
        $headTree `
        @($base, $unrelatedHead) `
        "Unrelated merge"
    foreach ($row in @(
        @("reversed", $head, $reversedMerge),
        @("extra", $head, $extraMerge),
        @("wrong-tree", $head, $wrongTreeMerge),
        @("unrelated", $unrelatedHead, $unrelatedMerge),
        @("wrong-head-ref", $base, $merge)
    )) {
        [void](Invoke-Git $fleetRoot @(
            "update-ref",
            "refs/validation-authority-self-test/$($row[0])/head",
            $row[1]
        ))
        [void](Invoke-Git $fleetRoot @(
            "update-ref",
            "refs/validation-authority-self-test/$($row[0])/merge",
            $row[2]
        ))
    }

    function Invoke-Case {
        param(
            [Parameter(Mandatory = $true)][string]$Name,
            [switch]$ExpectSuccess,
            [string]$CaseEventName = "pull_request_target",
            [string]$CaseAction = "opened",
            [string]$CaseRepositoryId = "1309815859",
            [int]$CasePullRequestNumber = 7,
            [int]$PayloadPullRequestNumber = 7,
            [string]$CaseBase = $base,
            [string]$CaseHead = $head,
            [AllowEmptyString()][string]$CaseMerge = $merge,
            [string]$RefCase = "baseline",
            [string]$PayloadHead = $head,
            [AllowEmptyString()][string]$PayloadMerge = $merge,
            [string]$CaseVerifierRoot = $verifierRoot,
            [string]$ExistingOutput = ""
        )

        $script:caseCount++
        $eventPath = Join-Path $testRoot "$Name-event.json"
        New-EventPayload `
            -Path $eventPath `
            -Action $CaseAction `
            -Number $PayloadPullRequestNumber `
            -BaseCommit $CaseBase `
            -HeadCommit $PayloadHead `
            -MergeCommit $PayloadMerge `
            -RepositoryId $CaseRepositoryId
        if ($ExistingOutput) {
            $out = $ExistingOutput
        } else {
            $out = Join-Path $testRoot "$Name-output"
            [void][IO.Directory]::CreateDirectory($out)
        }
        $arguments = @(
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy", "Bypass",
            "-File", $AdapterPath,
            "-RepositoryRoot", $fleetRoot,
            "-TrustedVerifierRoot", $CaseVerifierRoot,
            "-PullRequestNumber", [string]$CasePullRequestNumber,
            "-Repository", "MesmerPrism/rusty-fleet",
            "-RepositoryId", $CaseRepositoryId,
            "-RepositoryNodeId", "R_kgDOThI0Mw",
            "-EventName", $CaseEventName,
            "-EventAction", $CaseAction,
            "-EventRef", "refs/heads/main",
            "-EventSha", $CaseBase,
            "-BaseRepository", "MesmerPrism/rusty-fleet",
            "-BaseRef", "main",
            "-BaseCommit", $CaseBase,
            "-CandidateCommit", $CaseHead,
            "-MergeCommit", $CaseMerge,
            "-WorkflowRef",
            "MesmerPrism/rusty-fleet/.github/workflows/validation-authority.yml@refs/heads/main",
            "-WorkflowSha", $CaseBase,
            "-RunId", "12345",
            "-RunAttempt", "1",
            "-Job", "validation_authority",
            "-EventPayloadPath", $eventPath,
            "-OutDirectory", $out,
            "-OfflineTopologyMode",
            "-OfflineHeadRef",
            "refs/validation-authority-self-test/$RefCase/head",
            "-OfflineMergeRef",
            "refs/validation-authority-self-test/$RefCase/merge",
            "-Json"
        )
        $result = Invoke-Process `
            -FilePath $pwsh `
            -Arguments $arguments `
            -AllowFailure `
            -TimeoutSeconds $ProcessTimeoutSeconds
        if ($ExpectSuccess) {
            if ($result.exit_code -ne 0) {
                throw "Case '$Name' unexpectedly failed: $($result.stderr.Trim())"
            }
            $summary = $result.stdout | ConvertFrom-Json -Depth 20
            if (
                [string]$summary.decision -cne "unprotected" -or
                [bool]$summary.candidate_code_executed
            ) {
                throw "Case '$Name' returned a damaged summary."
            }
        } elseif ($result.exit_code -eq 0) {
            throw "Damaged case '$Name' was accepted."
        }
        $script:passedCases.Add($Name)
        return [pscustomobject]@{
            output = $out
            result = $result
        }
    }

    $env:RUSTY_FLEET_AUTHORITY_SELF_TEST = "1"
    $env:RUSTY_FLEET_CANDIDATE_SENTINEL = $sentinel
    $baseline = Invoke-Case -Name "baseline" -ExpectSuccess
    if (Test-Path -LiteralPath $sentinel) {
        throw "Candidate trap executed during static admission."
    }
    [void](Invoke-Case `
        -Name "hosted-null-event-merge" `
        -ExpectSuccess `
        -CaseMerge "" `
        -PayloadMerge "")
    [void](Invoke-Case -Name "wrong-event" -CaseEventName "pull_request")
    [void](Invoke-Case -Name "wrong-action" -CaseAction "closed")
    [void](Invoke-Case -Name "wrong-repository-id" -CaseRepositoryId "1309815858")
    [void](Invoke-Case `
        -Name "pr-number-mismatch" `
        -CasePullRequestNumber 8 `
        -PayloadPullRequestNumber 7)
    [void](Invoke-Case `
        -Name "head-ref-mismatch" `
        -RefCase "wrong-head-ref")
    [void](Invoke-Case `
        -Name "reversed-merge-parents" `
        -RefCase "reversed" `
        -CaseMerge $reversedMerge `
        -PayloadMerge $reversedMerge)
    [void](Invoke-Case `
        -Name "extra-merge-parent" `
        -RefCase "extra" `
        -CaseMerge $extraMerge `
        -PayloadMerge $extraMerge)
    [void](Invoke-Case `
        -Name "wrong-merge-tree" `
        -RefCase "wrong-tree" `
        -CaseMerge $wrongTreeMerge `
        -PayloadMerge $wrongTreeMerge)
    [void](Invoke-Case `
        -Name "base-not-ancestor" `
        -RefCase "unrelated" `
        -CaseHead $unrelatedHead `
        -PayloadHead $unrelatedHead `
        -CaseMerge $unrelatedMerge `
        -PayloadMerge $unrelatedMerge)
    [void](Invoke-Case `
        -Name "receipt-overwrite" `
        -ExistingOutput $baseline.output)

    $verifierScript = Join-Path `
        $verifierRoot `
        "scripts/Test-ExternalValidationAuthority.ps1"
    $verifierOriginal = [IO.File]::ReadAllBytes($verifierScript)
    try {
        [IO.File]::AppendAllText($verifierScript, "`n# substitution`n")
        [void](Invoke-Case -Name "dirty-verifier" -CaseVerifierRoot $verifierRoot)
    } finally {
        [IO.File]::WriteAllBytes($verifierScript, $verifierOriginal)
    }

    [void](Invoke-Git $fleetRoot @("replace", $base, $head))
    try {
        [void](Invoke-Case -Name "replacement-ref")
    } finally {
        [void](Invoke-Git $fleetRoot @("replace", "-d", $base))
    }

    $externalPath = Join-Path `
        $baseline.output `
        "external-validation-authority-assessment.json"
    $fleetPath = Join-Path `
        $baseline.output `
        "fleet-pull-request-authority-assessment.json"
    $fleetReceipt = Get-Content -Raw -LiteralPath $fleetPath |
        ConvertFrom-Json -Depth 40
    $externalSha = (Get-FileHash -LiteralPath $externalPath -Algorithm SHA256).Hash.
        ToLowerInvariant()
    if (
        [string]$fleetReceipt.external_assessment.sha256 -cne $externalSha -or
        [bool]$fleetReceipt.candidate_code_executed -or
        [bool]$fleetReceipt.execution_attested -or
        [bool]$fleetReceipt.publication_authority
    ) {
        throw "Baseline receipt did not preserve external digest and false claims."
    }
    $passedCases.Add("external-digest-and-false-claims")
    $caseCount++
    if (Test-Path -LiteralPath $sentinel) {
        throw "Candidate trap executed during a damaged-case check."
    }

    $productionPolicyExercised = $false
    $guardrailSource = [IO.Path]::GetFullPath($GuardrailCandidateRoot)
    if (-not (Test-Path -LiteralPath $guardrailSource -PathType Container)) {
        throw "Guardrail candidate root is absent."
    }
    $guardrailCommit = (
        Invoke-Git $guardrailSource @(
            "rev-parse",
            "$ExpectedGuardrailCandidate^{commit}"
        )
    ).stdout.Trim()
    if ($guardrailCommit -cne $ExpectedGuardrailCandidate) {
        throw "Guardrail repository does not contain the exact reviewed commit."
    }
    if ((
        Invoke-Git $guardrailSource @(
            "status", "--porcelain=v1", "--untracked-files=all"
        )
    ).stdout.Length -ne 0) {
        throw "Guardrail candidate root is dirty."
    }

    $productionRoot = Join-Path $testRoot "production-fleet"
        [void](Invoke-Process "git" @("init", $productionRoot))
        [void](Invoke-Git $productionRoot @(
            "config", "user.name", "Fleet authority production self-test"
        ))
        [void](Invoke-Git $productionRoot @(
            "config", "user.email", "selftest@example.invalid"
        ))
        [void](Invoke-Git $productionRoot @("config", "core.autocrlf", "false"))
        [void](Invoke-Git $productionRoot @(
            "remote", "add", "origin", $productionRoot
        ))
        [void](Invoke-Git $productionRoot @(
            "fetch",
            "--no-tags",
            $SourceRoot,
            "HEAD:refs/heads/source-base"
        ))
        [void](Invoke-Git $productionRoot @(
            "checkout", "--detach", "refs/heads/source-base"
        ))

        [string[]]$bootstrapPaths = @(
            ".github/workflows/validation-authority.yml",
            "config/fleet-pull-request-authority.v1.json",
            "config/validation-authority/fleet-validation-guardrails-20260730.approval-token.json",
            "docs/VALIDATION_AUTHORITY.md",
            "schemas/rusty.fleet.pull_request_authority_assessment.v1.schema.json",
            "tools/Test-FleetPullRequestAuthority.ps1",
            "tools/Test-FleetPullRequestAuthoritySelfTest.ps1"
        )
        foreach ($relative in $bootstrapPaths) {
            Copy-ExactSourceFile $SourceRoot $productionRoot $relative
        }
        [void](Invoke-Git $productionRoot @("add", "--all"))
        [void](Invoke-Git $productionRoot @(
            "commit", "--allow-empty", "-m", "Bootstrap exact production authority"
        ))
        $productionBase = (
            Invoke-Git $productionRoot @("rev-parse", "HEAD^{commit}")
        ).stdout.Trim()

        [void](Invoke-Git $productionRoot @(
            "fetch",
            "--no-tags",
            $guardrailSource,
            "$ExpectedGuardrailCandidate`:refs/heads/guardrail-candidate"
        ))
        $fetchedGuardrail = (
            Invoke-Git $productionRoot @(
                "rev-parse", "refs/heads/guardrail-candidate^{commit}"
            )
        ).stdout.Trim()
        if ($fetchedGuardrail -cne $ExpectedGuardrailCandidate) {
            throw "Fetched guardrail candidate changed identity."
        }

        [void](Invoke-Git $productionRoot @(
            "checkout",
            "-b",
            "production-authority-head",
            $ExpectedGuardrailCandidate
        ))
        [void](Invoke-Git $productionRoot @(
            "merge", "--no-ff", "--no-commit", $productionBase
        ))
        [void](Invoke-Git $productionRoot @(
            "commit", "-m", "Compose bootstrap with reviewed guardrails"
        ))
        $headWithToken = (
            Invoke-Git $productionRoot @("rev-parse", "HEAD^{commit}")
        ).stdout.Trim()
        $tokenRelative = (
            "config/validation-authority/" +
            "fleet-validation-guardrails-20260730.approval-token.json"
        )
        $tokenPath = Join-Path $productionRoot $tokenRelative
        if (-not (Test-Path -LiteralPath $tokenPath -PathType Leaf)) {
            throw "Production approval token was absent before consumption."
        }
        [IO.File]::Delete($tokenPath)
        [void](Invoke-Git $productionRoot @("add", "--update", "--", $tokenRelative))
        [void](Invoke-Git $productionRoot @(
            "commit", "-m", "Consume one-time guardrail approval"
        ))
        $acceptedHead = (
            Invoke-Git $productionRoot @("rev-parse", "HEAD^{commit}")
        ).stdout.Trim()
        $acceptedTree = (
            Invoke-Git $productionRoot @("rev-parse", "HEAD^{tree}")
        ).stdout.Trim()

        $productionPolicy = Get-Content -Raw -LiteralPath (
            Join-Path $SourceRoot "config/fleet-pull-request-authority.v1.json"
        ) | ConvertFrom-Json -Depth 50
        $productionApproval = @(
            $productionPolicy.approved_change_sets |
                Where-Object {
                    [string]$_.approval_id -ceq "fleet-validation-guardrails-20260730"
                }
        )
        if ($productionApproval.Count -ne 1) {
            throw "Production policy did not contain one exact guardrail approval."
        }
        [string[]]$expectedProductionPaths = @(
            $productionApproval[0].changed_paths |
                ForEach-Object { [string]$_ }
        )
        [string[]]$acceptedPaths = @(
            (
                Invoke-Git $productionRoot @(
                    "diff",
                    "--name-only",
                    "--no-renames",
                    $productionBase,
                    $acceptedHead
                )
            ).stdout -split "\r?\n" |
                Where-Object { $_ }
        )
        if (
            ($acceptedPaths -join "`n") -cne
            ($expectedProductionPaths -join "`n")
        ) {
            throw "Composed production head did not have the exact approved path set."
        }

        function Invoke-ProductionCase {
            param(
                [Parameter(Mandatory = $true)][string]$Name,
                [Parameter(Mandatory = $true)][string]$CaseBase,
                [Parameter(Mandatory = $true)][string]$CaseHead,
                [Parameter(Mandatory = $true)][string]$CaseMerge,
                [switch]$ExpectSuccess
            )

            $script:caseCount++
            [void](Invoke-Git $productionRoot @(
                "update-ref",
                "refs/validation-authority-self-test/$Name/head",
                $CaseHead
            ))
            [void](Invoke-Git $productionRoot @(
                "update-ref",
                "refs/validation-authority-self-test/$Name/merge",
                $CaseMerge
            ))
            [void](Invoke-Git $productionRoot @(
                "checkout", "--detach", $CaseBase
            ))
            $eventPath = Join-Path $testRoot "$Name-event.json"
            New-EventPayload `
                -Path $eventPath `
                -Action "opened" `
                -Number 91 `
                -BaseCommit $CaseBase `
                -HeadCommit $CaseHead `
                -MergeCommit $CaseMerge
            $out = Join-Path $testRoot "$Name-output"
            [void][IO.Directory]::CreateDirectory($out)
            $arguments = @(
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy", "Bypass",
                "-File", $AdapterPath,
                "-RepositoryRoot", $productionRoot,
                "-TrustedVerifierRoot", $verifierRoot,
                "-PullRequestNumber", "91",
                "-Repository", "MesmerPrism/rusty-fleet",
                "-RepositoryId", "1309815859",
                "-RepositoryNodeId", "R_kgDOThI0Mw",
                "-EventName", "pull_request_target",
                "-EventAction", "opened",
                "-EventRef", "refs/heads/main",
                "-EventSha", $CaseBase,
                "-BaseRepository", "MesmerPrism/rusty-fleet",
                "-BaseRef", "main",
                "-BaseCommit", $CaseBase,
                "-CandidateCommit", $CaseHead,
                "-MergeCommit", $CaseMerge,
                "-WorkflowRef",
                "MesmerPrism/rusty-fleet/.github/workflows/validation-authority.yml@refs/heads/main",
                "-WorkflowSha", $CaseBase,
                "-RunId", "98765",
                "-RunAttempt", "1",
                "-Job", "validation_authority",
                "-EventPayloadPath", $eventPath,
                "-OutDirectory", $out,
                "-OfflineTopologyMode",
                "-OfflineHeadRef",
                "refs/validation-authority-self-test/$Name/head",
                "-OfflineMergeRef",
                "refs/validation-authority-self-test/$Name/merge",
                "-Json"
            )
            $caseResult = Invoke-Process `
                -FilePath $pwsh `
                -Arguments $arguments `
                -AllowFailure `
                -TimeoutSeconds $ProcessTimeoutSeconds
            if ($ExpectSuccess) {
                if ($caseResult.exit_code -ne 0) {
                    throw (
                        "Production case '$Name' unexpectedly failed: " +
                        $caseResult.stderr.Trim()
                    )
                }
                $summary = $caseResult.stdout | ConvertFrom-Json -Depth 20
                if (
                    [string]$summary.decision -cne "approved-change-set" -or
                    [string]$summary.approval_id -cne
                    "fleet-validation-guardrails-20260730"
                ) {
                    throw "Production approval returned the wrong decision."
                }
                $receipt = Get-Content -Raw -LiteralPath (
                    Join-Path $out "fleet-pull-request-authority-assessment.json"
                ) | ConvertFrom-Json -Depth 50
                $externalReceiptPath = Join-Path `
                    $out `
                    "external-validation-authority-assessment.json"
                $externalReceipt = Get-Content -Raw -LiteralPath (
                    $externalReceiptPath
                ) | ConvertFrom-Json -Depth 50
                $externalReceiptSha256 = (
                    Get-FileHash `
                        -LiteralPath $externalReceiptPath `
                        -Algorithm SHA256
                ).Hash.ToLowerInvariant()
                if (
                    [string]$receipt.decision -cne "approved-change-set" -or
                    [string]$receipt.approval_id -cne
                    "fleet-validation-guardrails-20260730" -or
                    [string]$receipt.external_assessment.sha256 -cne
                    $externalReceiptSha256 -or
                    ($externalReceipt.changed_paths -join "`n") -cne
                    ($expectedProductionPaths -join "`n") -or
                    [string]$externalReceipt.decision -cne
                    "approved-change-set" -or
                    [string]$externalReceipt.approval_id -cne
                    "fleet-validation-guardrails-20260730"
                ) {
                    throw "Production approval receipt did not bind the exact policy."
                }
            } elseif ($caseResult.exit_code -eq 0) {
                throw "Damaged production case '$Name' was accepted."
            }
            $script:passedCases.Add($Name)
        }

        $acceptedMerge = New-CommitTree `
            $productionRoot `
            $acceptedTree `
            @($productionBase, $acceptedHead) `
            "Synthetic accepted PR merge"
        Invoke-ProductionCase `
            -Name "production-approved-change-set" `
            -CaseBase $productionBase `
            -CaseHead $acceptedHead `
            -CaseMerge $acceptedMerge `
            -ExpectSuccess

        $withTokenTree = (
            Invoke-Git $productionRoot @("rev-parse", "$headWithToken^{tree}")
        ).stdout.Trim()
        $withTokenMerge = New-CommitTree `
            $productionRoot `
            $withTokenTree `
            @($productionBase, $headWithToken) `
            "Synthetic token-retained PR merge"
        Invoke-ProductionCase `
            -Name "production-token-not-deleted" `
            -CaseBase $productionBase `
            -CaseHead $headWithToken `
            -CaseMerge $withTokenMerge

        [void](Invoke-Git $productionRoot @(
            "checkout", "-b", "production-artifact-tamper", $acceptedHead
        ))
        [IO.File]::AppendAllText(
            (Join-Path $productionRoot "README.md"),
            "`nproduction authority tamper`n"
        )
        [void](Invoke-Git $productionRoot @("add", "--", "README.md"))
        [void](Invoke-Git $productionRoot @(
            "commit", "-m", "Tamper approved artifact"
        ))
        $tamperedHead = (
            Invoke-Git $productionRoot @("rev-parse", "HEAD^{commit}")
        ).stdout.Trim()
        $tamperedTree = (
            Invoke-Git $productionRoot @("rev-parse", "HEAD^{tree}")
        ).stdout.Trim()
        $tamperedMerge = New-CommitTree `
            $productionRoot `
            $tamperedTree `
            @($productionBase, $tamperedHead) `
            "Synthetic tampered PR merge"
        Invoke-ProductionCase `
            -Name "production-artifact-tamper" `
            -CaseBase $productionBase `
            -CaseHead $tamperedHead `
            -CaseMerge $tamperedMerge

        [void](Invoke-Git $productionRoot @(
            "checkout", "-b", "production-extra-path", $acceptedHead
        ))
        Write-Utf8 `
            (Join-Path $productionRoot "unexpected-authority-path.txt") `
            "unexpected path`n"
        [void](Invoke-Git $productionRoot @(
            "add", "--", "unexpected-authority-path.txt"
        ))
        [void](Invoke-Git $productionRoot @(
            "commit", "-m", "Add unapproved extra path"
        ))
        $extraPathHead = (
            Invoke-Git $productionRoot @("rev-parse", "HEAD^{commit}")
        ).stdout.Trim()
        $extraPathTree = (
            Invoke-Git $productionRoot @("rev-parse", "HEAD^{tree}")
        ).stdout.Trim()
        $extraPathMerge = New-CommitTree `
            $productionRoot `
            $extraPathTree `
            @($productionBase, $extraPathHead) `
            "Synthetic extra-path PR merge"
        Invoke-ProductionCase `
            -Name "production-extra-path" `
            -CaseBase $productionBase `
            -CaseHead $extraPathHead `
            -CaseMerge $extraPathMerge

        [void](Invoke-Git $productionRoot @(
            "checkout", "-b", "production-consumed-drift", $acceptedHead
        ))
        foreach ($artifact in @($productionApproval[0].artifacts)) {
            if ([string]$artifact.state -ceq "present") {
                [IO.File]::AppendAllText(
                    (Join-Path $productionRoot ([string]$artifact.path)),
                    " "
                )
            }
        }
        Copy-ExactSourceFile $SourceRoot $productionRoot $tokenRelative
        [void](Invoke-Git $productionRoot @("add", "--all"))
        [void](Invoke-Git $productionRoot @(
            "commit", "-m", "Create consumed-approval drifted base"
        ))
        $driftedBase = (
            Invoke-Git $productionRoot @("rev-parse", "HEAD^{commit}")
        ).stdout.Trim()
        $ancestorResult = Invoke-Git $productionRoot @(
            "merge-base",
            "--is-ancestor",
            $ExpectedGuardrailCandidate,
            $driftedBase
        ) -AllowFailure
        if ($ancestorResult.exit_code -ne 0) {
            throw "Consumed replay base did not contain the accepted ancestor."
        }
        $replayHead = New-CommitTree `
            $productionRoot `
            $acceptedTree `
            @($driftedBase) `
            "Replay exact old approved artifact tree"
        [string[]]$replayPaths = @(
            (
                Invoke-Git $productionRoot @(
                    "diff",
                    "--name-only",
                    "--no-renames",
                    $driftedBase,
                    $replayHead
                )
            ).stdout -split "\r?\n" |
                Where-Object { $_ }
        )
        if (
            ($replayPaths -join "`n") -cne
            ($expectedProductionPaths -join "`n")
        ) {
            throw "Consumed replay fixture did not recreate the exact old approval."
        }
        $replayMerge = New-CommitTree `
            $productionRoot `
            $acceptedTree `
            @($driftedBase, $replayHead) `
            "Synthetic consumed replay PR merge"
        Invoke-ProductionCase `
            -Name "production-consumed-replay" `
            -CaseBase $driftedBase `
            -CaseHead $replayHead `
            -CaseMerge $replayMerge

    $productionPolicyExercised = $true

    $result = [pscustomobject][ordered]@{
        schema = "rusty.fleet.pull_request_authority_self_test.v1"
        passed = $true
        cases = $caseCount
        candidate_trap_executed = $false
        production_policy_exercised = $productionPolicyExercised
        case_names = @($passedCases)
    }
    if ($Json) {
        Write-Output ($result | ConvertTo-Json -Depth 10)
    } else {
        Write-Output $result
    }
} finally {
    $env:RUSTY_FLEET_AUTHORITY_SELF_TEST = $previousSelfTestGate
    Remove-Item Env:RUSTY_FLEET_CANDIDATE_SENTINEL -ErrorAction SilentlyContinue
    if (-not $KeepTemporaryFiles) {
        $resolvedTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd("\", "/")
        $resolvedTest = [IO.Path]::GetFullPath($testRoot)
        if (-not $resolvedTest.StartsWith(
            $resolvedTemp + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase
        )) {
            throw "Refusing to clean a self-test directory outside the system temp root."
        }
        Remove-Item -LiteralPath $resolvedTest -Recurse -Force
    }
}
