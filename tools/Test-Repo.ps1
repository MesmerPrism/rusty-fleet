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
        "docs/IMPLEMENTATION_PLAN.md",
        "docs/WORKFLOW.md",
        "docs/VALIDATION.md",
        "docs/PUBLIC_PRIVATE_BOUNDARY.md",
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
    Assert-True -Condition ($unit.status -eq "proposed") `
        -Message "Milestone 0 must remain proposed until an owned workflow transition reviews it."
    Assert-True -Condition ($unit.unit_id -eq "fleet-m0-foundation-and-simulator") `
        -Message "Unexpected Milestone 0 unit ID."
    Assert-True -Condition (@($unit.acceptance).Count -ge 5) `
        -Message "Milestone 0 must remain a vertical stack with complete acceptance coverage."
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
