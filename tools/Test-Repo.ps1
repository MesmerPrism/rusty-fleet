# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [ValidateSet("Quick", "Standard", "Deep")]
    [string] $Tier = "Quick",

    [string] $WorkEnvironmentRoot = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = Split-Path -Parent $PSScriptRoot

function Assert-True {
    param(
        [Parameter(Mandatory)]
        [bool] $Condition,

        [Parameter(Mandatory)]
        [string] $Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Invoke-Git {
    param([Parameter(ValueFromRemainingArguments)][string[]] $Arguments)

    & git -C $repoRoot @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Git failed: git $($Arguments -join ' ')"
    }
}

function Get-PublicFiles {
    $ignoredRoots = @(
        [IO.Path]::GetFullPath((Join-Path $repoRoot ".git")),
        [IO.Path]::GetFullPath((Join-Path $repoRoot "local")),
        [IO.Path]::GetFullPath((Join-Path $repoRoot "artifacts")),
        [IO.Path]::GetFullPath((Join-Path $repoRoot "target")),
        [IO.Path]::GetFullPath((Join-Path $repoRoot "bin")),
        [IO.Path]::GetFullPath((Join-Path $repoRoot "obj"))
    )

    Get-ChildItem -LiteralPath $repoRoot -Recurse -File -Force |
        Where-Object {
            $candidate = [IO.Path]::GetFullPath($_.FullName)
            -not ($ignoredRoots | Where-Object {
                $candidate.StartsWith($_ + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
                $candidate -eq $_
            })
        }
}

function Test-JsonDocuments {
    $jsonFiles = @(Get-PublicFiles | Where-Object Extension -EQ ".json")
    foreach ($file in $jsonFiles) {
        try {
            Get-Content -LiteralPath $file.FullName -Raw | ConvertFrom-Json -Depth 100 | Out-Null
        } catch {
            throw "Invalid JSON: $($file.FullName): $($_.Exception.Message)"
        }
    }

    $jsonlFiles = @(Get-PublicFiles | Where-Object Extension -EQ ".jsonl")
    foreach ($file in $jsonlFiles) {
        $lineNumber = 0
        foreach ($line in Get-Content -LiteralPath $file.FullName) {
            $lineNumber++
            if ([string]::IsNullOrWhiteSpace($line)) {
                continue
            }
            try {
                $line | ConvertFrom-Json -Depth 100 | Out-Null
            } catch {
                throw "Invalid JSONL: $($file.FullName):$lineNumber"
            }
        }
    }
}

function Test-PublicBoundary {
    $textExtensions = @(
        ".md", ".txt", ".json", ".jsonl", ".yml", ".yaml", ".ps1",
        ".rs", ".cs", ".toml", ".xml"
    )
    $patterns = [ordered]@{
        "local Windows user/work path" = "(?i)\b[A-Z]:\\(?:Users|Work)\\"
        "GitHub token" = "\bgh[opusr]_[A-Za-z0-9_]{20,}\b"
        "private key" = "-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"
        "AWS access key" = "\bAKIA[0-9A-Z]{16}\b"
    }

    foreach ($file in Get-PublicFiles | Where-Object Extension -In $textExtensions) {
        $content = Get-Content -LiteralPath $file.FullName -Raw
        foreach ($entry in $patterns.GetEnumerator()) {
            if ($content -match $entry.Value) {
                throw "Public-boundary scan found $($entry.Key) in $($file.FullName)."
            }
        }
    }
}

function Test-RequiredFiles {
    $required = @(
        "README.md",
        "AGENTS.md",
        "LICENSE",
        "CHANGELOG.md",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "docs/ARCHITECTURE.md",
        "docs/DATASTREAMS.md",
        "docs/IMPLEMENTATION_PLAN.md",
        "docs/OPERATOR_UI.md",
        "docs/WORKFLOW.md",
        "docs/VALIDATION.md",
        "docs/PUBLIC_PRIVATE_BOUNDARY.md",
        "docs/decisions/0003-datastream-lifecycle-and-authority.md",
        "docs/research/DATASTREAM_REFERENCE_LEDGER.md",
        "docs/research/FLEET_RESEARCH_INTEGRATION_REVIEW.md",
        "docs/research/MORPHOSPACE_DATASTREAM_MATRIX.md",
        "morphospace/project.spec.json",
        "morphospace/feature.lock.json",
        "morphospace/workspace.state.json",
        "morphospace/iteration-events.jsonl",
        "tools/Test-Repo.ps1",
        ".github/workflows/ci.yml",
        ".github/workflows/deep-validation.yml"
    )

    foreach ($relative in $required) {
        Assert-True -Condition (Test-Path -LiteralPath (Join-Path $repoRoot $relative) -PathType Leaf) `
            -Message "Required file is missing: $relative"
    }
}

function Test-PlanningInvariants {
    $spec = Get-Content -LiteralPath (Join-Path $repoRoot "morphospace/project.spec.json") -Raw |
        ConvertFrom-Json -Depth 100
    $lock = Get-Content -LiteralPath (Join-Path $repoRoot "morphospace/feature.lock.json") -Raw |
        ConvertFrom-Json -Depth 100
    $state = Get-Content -LiteralPath (Join-Path $repoRoot "morphospace/workspace.state.json") -Raw |
        ConvertFrom-Json -Depth 100

    Assert-True -Condition ($spec.project_id -eq "rusty-fleet") -Message "Unexpected project ID."
    Assert-True -Condition ($lock.project_id -eq $spec.project_id) -Message "Feature lock project mismatch."
    Assert-True -Condition ($state.project_id -eq $spec.project_id) -Message "Workspace state project mismatch."
    Assert-True -Condition ($lock.default_activation -eq "disabled") -Message "Default activation must remain disabled."
    Assert-True -Condition (@($lock.selected_features).Count -eq 0) -Message "Planning baseline must select no features."
    Assert-True -Condition (@($lock.features).Count -eq 0) -Message "Planning baseline must resolve no features."

    foreach ($property in $lock.effect_union.PSObject.Properties) {
        Assert-True -Condition (@($property.Value).Count -eq 0) `
            -Message "Planning baseline effect union must remain empty: $($property.Name)"
    }

    $unitPath = Join-Path $repoRoot "morphospace/iteration-units/fleet-m0-foundation-and-simulator.json"
    Assert-True -Condition (Test-Path -LiteralPath $unitPath -PathType Leaf) `
        -Message "The proposed Milestone 0 stack is missing."
    $unit = Get-Content -LiteralPath $unitPath -Raw | ConvertFrom-Json -Depth 100
    Assert-True -Condition ($unit.unit_id -eq "fleet-m0-foundation-and-simulator") `
        -Message "Unexpected Milestone 0 unit ID."
    Assert-True -Condition ($unit.status -in @("proposed", "ready")) `
        -Message "Milestone 0 may be proposed or ready only at the planning checkpoint."
    if ($unit.status -eq "ready") {
        $readyEventId = "$($unit.unit_id)-ready-0001"
        $eventsPath = Join-Path $repoRoot "morphospace/iteration-events.jsonl"
        $events = @(
            Get-Content -LiteralPath $eventsPath |
                Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
                ForEach-Object { $_ | ConvertFrom-Json -Depth 100 }
        )
        $readyEvents = @($events | Where-Object { $_.event_id -eq $readyEventId })
        Assert-True -Condition ($readyEvents.Count -eq 1) `
            -Message "Ready Milestone 0 must have exactly one owned ready transition event."
        Assert-True -Condition (
            $readyEvents[0].event_type -eq "state-transition" -and
            $readyEvents[0].unit_id -eq $unit.unit_id -and
            $state.last_event_id -eq $readyEventId -and
            $state.next_ready_unit -eq $unit.unit_id -and
            $null -eq $state.current_unit
        ) -Message "Ready Milestone 0 is not bound to workspace state."

        $transactionId = "$readyEventId-transition"
        $intentPath = Join-Path $repoRoot "morphospace/receipts/transactions/$transactionId.intent.json"
        $completionPath = Join-Path $repoRoot "morphospace/receipts/transactions/$transactionId.completion.json"
        Assert-True -Condition (
            (Test-Path -LiteralPath $intentPath -PathType Leaf) -and
            (Test-Path -LiteralPath $completionPath -PathType Leaf)
        ) -Message "Ready Milestone 0 is missing its transition-ledger receipts."
        $intent = Get-Content -LiteralPath $intentPath -Raw | ConvertFrom-Json -Depth 100
        $completion = Get-Content -LiteralPath $completionPath -Raw | ConvertFrom-Json -Depth 100
        $intentSha256 = (Get-FileHash -LiteralPath $intentPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $unitProjection = $unit | ConvertTo-Json -Depth 100 -Compress
        $targetUnitProjection = $intent.target.unit.document | ConvertTo-Json -Depth 100 -Compress
        $stateProjection = $state | ConvertTo-Json -Depth 100 -Compress
        $targetStateProjection = $intent.target.state.document | ConvertTo-Json -Depth 100 -Compress
        Assert-True -Condition (
            $intent.transaction_id -eq $transactionId -and
            $intent.event.event_id -eq $readyEventId -and
            $intent.target.unit.document.status -eq "ready" -and
            $completion.transaction_id -eq $transactionId -and
            $completion.event_id -eq $readyEventId -and
            $completion.status -eq "committed" -and
            $completion.unit_sha256 -eq $intent.target.unit.sha256 -and
            $completion.state_sha256 -eq $intent.target.state.sha256 -and
            $unitProjection -ceq $targetUnitProjection -and
            $stateProjection -ceq $targetStateProjection -and
            $completion.intent.sha256 -eq $intentSha256
        ) -Message "Ready Milestone 0 transition receipts are stale or damaged."
    }
    Assert-True -Condition (@($unit.acceptance).Count -ge 5) `
        -Message "Milestone 0 must remain a vertical stack with complete acceptance coverage."
    Assert-True -Condition (@($unit.acceptance.acceptance_id) -contains "canonical-datastream-projections") `
        -Message "Milestone 0 must include source-only datastream contract acceptance."
}

function Test-DatastreamPlanning {
    $datastreams = Get-Content -LiteralPath (Join-Path $repoRoot "docs/DATASTREAMS.md") -Raw
    $architecture = Get-Content -LiteralPath (Join-Path $repoRoot "docs/ARCHITECTURE.md") -Raw
    $implementation = Get-Content -LiteralPath (Join-Path $repoRoot "docs/IMPLEMENTATION_PLAN.md") -Raw
    $operatorUi = Get-Content -LiteralPath (Join-Path $repoRoot "docs/OPERATOR_UI.md") -Raw
    $ledger = Get-Content -LiteralPath (Join-Path $repoRoot "docs/research/DATASTREAM_REFERENCE_LEDGER.md") -Raw

    foreach ($phrase in @(
        "native descriptor",
        "source selection",
        "component epochs",
        "timestamp domains",
        "per-edge",
        "Scientific session, recording, and replay",
        "no data",
        "stalled",
        "frozen",
        "admission",
        "FFmpeg",
        "LSL adapter contract"
    )) {
        Assert-True -Condition ($datastreams.Contains($phrase, [StringComparison]::OrdinalIgnoreCase)) `
            -Message "Datastream planning guardrail is missing: $phrase"
    }

    Assert-True -Condition ($architecture.Contains("Datastream Management")) `
        -Message "Architecture must route to the normative datastream contract."
    Assert-True -Condition ($implementation.Contains("Selected datastream and media operations")) `
        -Message "Implementation plan must retain the selected-stream milestone stack."
    Assert-True -Condition ($operatorUi.Contains("Selected-stream detail")) `
        -Message "Operator UI must retain layered selected-stream detail."
    foreach ($source in @(
        "labstreaminglayer.readthedocs.io",
        "ffmpeg.org",
        "developer.android.com",
        "prometheus.io"
    )) {
        Assert-True -Condition ($ledger.Contains($source)) `
            -Message "Datastream primary-source ledger is missing: $source"
    }
}

function Test-MarkdownLinks {
    $markdownFiles = @(Get-PublicFiles | Where-Object Extension -EQ ".md")
    $linkPattern = "\[[^\]]+\]\(([^)]+)\)"

    foreach ($file in $markdownFiles) {
        $content = Get-Content -LiteralPath $file.FullName -Raw
        foreach ($match in [regex]::Matches($content, $linkPattern)) {
            $target = $match.Groups[1].Value.Trim()
            if (
                $target.StartsWith("http://", [StringComparison]::OrdinalIgnoreCase) -or
                $target.StartsWith("https://", [StringComparison]::OrdinalIgnoreCase) -or
                $target.StartsWith("mailto:", [StringComparison]::OrdinalIgnoreCase) -or
                $target.StartsWith("#") -or
                $target.StartsWith("<")
            ) {
                continue
            }

            $pathPart = ($target -split "#", 2)[0]
            if ([string]::IsNullOrWhiteSpace($pathPart)) {
                continue
            }

            $decoded = [Uri]::UnescapeDataString($pathPart).Replace("/", [IO.Path]::DirectorySeparatorChar)
            $resolved = [IO.Path]::GetFullPath((Join-Path $file.DirectoryName $decoded))
            if (-not (Test-Path -LiteralPath $resolved)) {
                throw "Broken Markdown link in $($file.FullName): $target"
            }
        }
    }
}

function Test-TrackedTree {
    $inside = (& git -C $repoRoot rev-parse --is-inside-work-tree 2>$null)
    if ($LASTEXITCODE -ne 0 -or $inside -ne "true") {
        throw "Deep validation requires a Git worktree."
    }

    $tracked = @(& git -C $repoRoot ls-files)
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to enumerate tracked files."
    }
    Assert-True -Condition ($tracked.Count -gt 0) -Message "Deep validation requires tracked files."

    $forbiddenExtensions = @(".apk", ".aab", ".idsig", ".keystore", ".jks", ".pfx", ".p12")
    foreach ($relative in $tracked) {
        $full = Join-Path $repoRoot $relative
        if (-not (Test-Path -LiteralPath $full -PathType Leaf)) {
            continue
        }
        $item = Get-Item -LiteralPath $full
        Assert-True -Condition ($item.Extension -notin $forbiddenExtensions) `
            -Message "Generated or sensitive artifact is tracked: $relative"
        Assert-True -Condition ($item.Length -le 5MB) `
            -Message "Tracked file exceeds the initial 5 MiB public-repo budget: $relative"
    }
}

Push-Location -LiteralPath $repoRoot
try {
    Test-RequiredFiles
    Test-JsonDocuments
    Test-PublicBoundary
    Test-PlanningInvariants
    Test-DatastreamPlanning
    Invoke-Git diff --check

    if ($Tier -in @("Standard", "Deep")) {
        Test-MarkdownLinks

        if ($WorkEnvironmentRoot) {
            $validator = Join-Path $WorkEnvironmentRoot "scripts/Test-WorkflowContracts.ps1"
            Assert-True -Condition (Test-Path -LiteralPath $validator -PathType Leaf) `
                -Message "Work-environment workflow validator not found: $validator"
            & pwsh -NoProfile -ExecutionPolicy Bypass -File $validator `
                -WorkspaceRoot (Join-Path $repoRoot "morphospace")
            if ($LASTEXITCODE -ne 0) {
                throw "Morphospace workflow validation failed."
            }
        }
    }

    if ($Tier -eq "Deep") {
        Test-TrackedTree
    }

    Write-Host "Rusty Fleet $Tier validation passed."
}
finally {
    Pop-Location
}
