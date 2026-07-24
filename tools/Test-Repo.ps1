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
            $relativeSegments = [IO.Path]::GetRelativePath($repoRoot, $candidate).
                Split([IO.Path]::DirectorySeparatorChar, [StringSplitOptions]::RemoveEmptyEntries)
            -not ($relativeSegments | Where-Object { $_ -in @("bin", "obj", "target") }) -and
            -not ($ignoredRoots | Where-Object {
                $candidate.StartsWith($_ + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
                $candidate -eq $_
            })
    }
}

function Get-TransitionBinding {
    param(
        [Parameter(Mandatory)]
        [object] $Unit,

        [Parameter(Mandatory)]
        [object] $State,

        [Parameter(Mandatory)]
        [string] $EventId,

        [Parameter(Mandatory)]
        [string] $ExpectedStatus,

        [AllowNull()]
        [object] $ExpectedCurrentUnit,

        [AllowNull()]
        [object] $ExpectedNextReadyUnit,

        [switch] $Historical
    )

    $eventsPath = Join-Path $repoRoot "morphospace/iteration-events.jsonl"
    $events = @(
        Get-Content -LiteralPath $eventsPath |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
            ForEach-Object { $_ | ConvertFrom-Json -Depth 100 }
    )
    $matchingEvents = @($events | Where-Object { $_.event_id -eq $EventId })
    Assert-True -Condition ($matchingEvents.Count -eq 1) `
        -Message "$ExpectedStatus workflow unit must have exactly one owned transition event."
    Assert-True -Condition (
        $matchingEvents[0].event_type -eq "state-transition" -and
        $matchingEvents[0].unit_id -eq $Unit.unit_id -and
        (
            $Historical -or
            (
                $State.last_event_id -eq $EventId -and
                $State.current_unit -eq $ExpectedCurrentUnit -and
                $State.next_ready_unit -eq $ExpectedNextReadyUnit
            )
        )
    ) -Message "$ExpectedStatus workflow unit is not bound to workspace state."

    $transactionId = "$EventId-transition"
    $intentPath = Join-Path $repoRoot "morphospace/receipts/transactions/$transactionId.intent.json"
    $completionPath = Join-Path $repoRoot "morphospace/receipts/transactions/$transactionId.completion.json"
    Assert-True -Condition (
        (Test-Path -LiteralPath $intentPath -PathType Leaf) -and
        (Test-Path -LiteralPath $completionPath -PathType Leaf)
    ) -Message "$ExpectedStatus workflow unit is missing its transition-ledger receipts."
    $intent = Get-Content -LiteralPath $intentPath -Raw | ConvertFrom-Json -Depth 100
    $completion = Get-Content -LiteralPath $completionPath -Raw | ConvertFrom-Json -Depth 100
    $intentSha256 = (Get-FileHash -LiteralPath $intentPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $unitProjection = $Unit | ConvertTo-Json -Depth 100 -Compress
    $targetUnitProjection = $intent.target.unit.document | ConvertTo-Json -Depth 100 -Compress
    $targetStateProjection = $intent.target.state.document | ConvertTo-Json -Depth 100 -Compress
    $stateMatches = if ($Historical) {
        $intent.target.state.document.last_event_id -eq $EventId -and
        $intent.target.state.document.current_unit -eq $ExpectedCurrentUnit -and
        $intent.target.state.document.next_ready_unit -eq $ExpectedNextReadyUnit
    } else {
        $stateProjection = $State | ConvertTo-Json -Depth 100 -Compress
        $stateProjection -ceq $targetStateProjection
    }
    Assert-True -Condition (
        $intent.transaction_id -eq $transactionId -and
        $intent.event.event_id -eq $EventId -and
        $intent.target.unit.document.status -eq $ExpectedStatus -and
        $completion.transaction_id -eq $transactionId -and
        $completion.event_id -eq $EventId -and
        $completion.status -eq "committed" -and
        $completion.unit_sha256 -eq $intent.target.unit.sha256 -and
        $completion.state_sha256 -eq $intent.target.state.sha256 -and
        $unitProjection -ceq $targetUnitProjection -and
        $stateMatches -and
        $completion.intent.sha256 -eq $intentSha256
    ) -Message "$ExpectedStatus workflow transition receipts are stale or damaged."

    return [pscustomobject]@{
        event = $matchingEvents[0]
        intent = $intent
        completion = $completion
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
        ".rs", ".cs", ".toml", ".xml", ".xaml", ".csproj", ".props"
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
        "Cargo.lock",
        "Cargo.toml",
        "global.json",
        "Directory.Build.props",
        "apps/fleet-hub-local/Cargo.toml",
        "apps/fleet-hub-local/src/lib.rs",
        "apps/fleet-hub-local/src/main.rs",
        "apps/fleetctl/Cargo.toml",
        "apps/fleetctl/src/lib.rs",
        "apps/fleetctl/src/main.rs",
        "apps/fleet-console-wpf/RustyFleet.FleetConsole.csproj",
        "apps/fleet-console-wpf/App.xaml",
        "apps/fleet-console-wpf/App.xaml.cs",
        "apps/fleet-console-wpf/MainWindow.xaml",
        "apps/fleet-console-wpf/MainWindow.xaml.cs",
        "apps/fleet-console-wpf/Contracts/FleetProjectionModels.cs",
        "apps/fleet-console-wpf/Contracts/FleetProjectionValidation.cs",
        "apps/fleet-console-wpf/Services/FleetApiClient.cs",
        "apps/fleet-console-wpf/ViewModels/Commands.cs",
        "apps/fleet-console-wpf/ViewModels/DeviceViewModels.cs",
        "apps/fleet-console-wpf/ViewModels/FleetWorkspaceViewModel.cs",
        "apps/fleet-console-wpf.tests/RustyFleet.FleetConsole.Tests.csproj",
        "apps/fleet-console-wpf.tests/Program.cs",
        "crates/fleet-contracts/Cargo.toml",
        "crates/fleet-contracts/src/checkin.rs",
        "crates/fleet-contracts/src/lib.rs",
        "crates/fleet-hub/Cargo.toml",
        "crates/fleet-hub/src/lib.rs",
        "crates/fleet-manifold-adapter/Cargo.toml",
        "crates/fleet-manifold-adapter/src/lib.rs",
        "crates/fleet-simulator/Cargo.toml",
        "crates/fleet-simulator/src/lib.rs",
        "fixtures/contracts/checkin-claims.valid.json",
        "fixtures/contracts/checkin-claims.damaged.json",
        "fixtures/contracts/checkin-signing-vector.valid.json",
        "fixtures/contracts/device-observation.valid.json",
        "fixtures/contracts/device-observation.damaged.json",
        "fixtures/contracts/query.valid.json",
        "fixtures/contracts/saved-view.valid.json",
        "fixtures/contracts/saved-view.damaged.json",
        "fixtures/contracts/stream-descriptor.valid.json",
        "fixtures/scenarios/scale-and-damage.v1.json",
        "schemas/rusty.fleet.checkin_claims.v1.schema.json",
        "schemas/rusty.fleet.checkin_signing_vector.v1.schema.json",
        "schemas/rusty.fleet.device_observation.v1.schema.json",
        "schemas/rusty.fleet.operation_ledger.v1.schema.json",
        "schemas/rusty.fleet.operator_projection.v1.schema.json",
        "schemas/rusty.fleet.query.v1.schema.json",
        "schemas/rusty.fleet.signed_checkin.v1.schema.json",
        "schemas/rusty.fleet.stream_descriptor.v1.schema.json",
        "docs/ARCHITECTURE.md",
        "docs/DATASTREAMS.md",
        "docs/IMPLEMENTATION_PLAN.md",
        "docs/M0_SOURCE_FOUNDATION.md",
        "docs/M0_GRAPH_AND_INSTRUCTION_REVIEW.md",
        "docs/M1_LOCAL_MONITORING.md",
        "docs/OPERATOR_UI.md",
        "docs/WORKFLOW.md",
        "docs/VALIDATION.md",
        "docs/PUBLIC_PRIVATE_BOUNDARY.md",
        "docs/decisions/0003-datastream-lifecycle-and-authority.md",
        "docs/decisions/0004-m0-source-boundary-and-threat-model.md",
        "docs/decisions/0005-m1-checkin-authority.md",
        "docs/decisions/0006-m1-local-ingress-threat-model.md",
        "docs/research/DATASTREAM_REFERENCE_LEDGER.md",
        "docs/research/FLEET_RESEARCH_INTEGRATION_REVIEW.md",
        "docs/research/MORPHOSPACE_DATASTREAM_MATRIX.md",
        "morphospace/.gitattributes",
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

function Invoke-Cargo {
    param([Parameter(Mandatory)][string[]] $Arguments)

    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Cargo failed: cargo $($Arguments -join ' ')"
    }
}

function Invoke-DotNet {
    param([Parameter(Mandatory)][string[]] $Arguments)

    & dotnet @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "dotnet failed: dotnet $($Arguments -join ' ')"
    }
}

function Test-WpfConsole {
    $globalJson = Get-Content -LiteralPath (Join-Path $repoRoot "global.json") -Raw |
        ConvertFrom-Json -Depth 20
    Assert-True -Condition (
        $globalJson.sdk.version -eq "10.0.201" -and
        $globalJson.sdk.rollForward -eq "disable" -and
        $globalJson.sdk.allowPrerelease -eq $false
    ) -Message "The WPF toolchain must remain pinned to the stable installed .NET SDK."

    $consoleProject = Get-Content -LiteralPath (
        Join-Path $repoRoot "apps/fleet-console-wpf/RustyFleet.FleetConsole.csproj"
    ) -Raw
    Assert-True -Condition (
        $consoleProject.Contains("<UseWPF>true</UseWPF>") -and
        -not $consoleProject.Contains("<PackageReference")
    ) -Message "The M1 Console must retain native WPF semantics without a theme package."

    $testProject = Join-Path (
        $repoRoot
    ) "apps/fleet-console-wpf.tests/RustyFleet.FleetConsole.Tests.csproj"
    Invoke-DotNet -Arguments @(
        "build",
        $testProject,
        "-c",
        "Release",
        "--nologo"
    )

    $receiptLines = @(
        & dotnet run `
            --project $testProject `
            -c Release `
            --no-build `
            -- `
            --repo-root $repoRoot
    )
    if ($LASTEXITCODE -ne 0) {
        throw "Native WPF validation executable failed."
    }
    $receipt = ($receiptLines -join [Environment]::NewLine) | ConvertFrom-Json -Depth 20
    Assert-True -Condition (
        $receipt.schema -eq "rusty.fleet.wpf_validation.v1" -and
        $receipt.result -eq "pass" -and
        $receipt.projection_rows -eq 1000 -and
        $receipt.grid_columns -eq 12 -and
        $receipt.realized_rows -lt 250 -and
        $receipt.native_datagrid -eq $true -and
        $receipt.recycling_virtualization -eq $true -and
        $receipt.native_automation_peer -eq $true -and
        $receipt.inspector_automation_peer -eq $true -and
        $receipt.pointer_batch_toggle -eq $true -and
        $receipt.accessible_batch_toggle -eq $true -and
        $receipt.loopback_hub_only -eq $true -and
        $receipt.bounded_hub_response -eq $true -and
        $receipt.canonical_watch_projection -eq $true -and
        $receipt.watch_cursor_bounded -eq $true -and
        $receipt.watch_sequence_reset_rebased -eq $true -and
        $receipt.rejected_watch_event_distinguished -eq $true -and
        $receipt.damaged_watch_fail_closed -eq $true -and
        $receipt.watch_unavailable_query_fallback -eq $true -and
        $receipt.watch_sync_accessible -eq $true -and
        $receipt.projection_identity_fail_closed -eq $true -and
        $receipt.mixed_freshness_fixture -eq $true -and
        $receipt.fresh_rows -eq 500 -and
        $receipt.stale_rows -eq 250 -and
        $receipt.offline_rows -eq 250 -and
        $receipt.capability_downgrade_rows -eq 125 -and
        $receipt.mixed_state_grammar -eq $true -and
        $receipt.canonical_scope -eq $true -and
        $receipt.saved_view_crud -eq $true -and
        $receipt.saved_view_exact_query_restored -eq $true -and
        $receipt.saved_view_navigation_restored -eq $true -and
        $receipt.empty_scope_preserved -eq $true -and
        $receipt.grouped_virtualization -eq $true -and
        $receipt.hidden_selection_preserved -eq $true -and
        $receipt.inspector_outside_scope_preserved -eq $true -and
        $receipt.theme_dependency -eq "none" -and
        $receipt.batch_selection_preserved -eq $true -and
        $receipt.inspector_capability_families -ge 3 -and
        $receipt.view_model_ms -lt 2000
    ) -Message "Native WPF scale, accessibility, or stable-context evidence is incomplete."
}

function Test-SourceImplementation {
    $workspace = Get-Content -LiteralPath (Join-Path $repoRoot "Cargo.toml") -Raw
    foreach ($member in @(
        "apps/fleet-hub-local",
        "apps/fleetctl",
        "crates/fleet-contracts",
        "crates/fleet-hub",
        "crates/fleet-manifold-adapter",
        "crates/fleet-simulator"
    )) {
        Assert-True -Condition ($workspace.Contains("`"$member`"")) `
            -Message "Cargo workspace is missing required member: $member"
    }

    $lock = Get-Content -LiteralPath (Join-Path $repoRoot "Cargo.lock") -Raw
    foreach ($forbiddenPackage in @(
        'name = "windows"',
        'name = "ffmpeg"',
        'name = "liblsl"'
    )) {
        Assert-True -Condition (-not $lock.Contains($forbiddenPackage)) `
            -Message "Current M0/M1 owner boundary does not permit this package: $forbiddenPackage"
    }

    foreach ($schemaFile in Get-ChildItem -LiteralPath (Join-Path $repoRoot "schemas") -Filter "*.schema.json") {
        $schema = Get-Content -LiteralPath $schemaFile.FullName -Raw | ConvertFrom-Json -Depth 100
        Assert-True -Condition (
            $schema.'$schema' -eq "https://json-schema.org/draft/2020-12/schema" -and
            -not [string]::IsNullOrWhiteSpace($schema.'$id') -and
            -not [string]::IsNullOrWhiteSpace($schema.title)
        ) -Message "Versioned schema metadata is incomplete: $($schemaFile.Name)"
    }

    $scenarioManifest = Get-Content -LiteralPath (
        Join-Path $repoRoot "fixtures/scenarios/scale-and-damage.v1.json"
    ) -Raw | ConvertFrom-Json -Depth 100
    Assert-True -Condition (
        $scenarioManifest.schema -eq "rusty.fleet.fixture_manifest.v1" -and
        $scenarioManifest.seed -eq 5932739705870634068 -and
        (@($scenarioManifest.sizes) -join ",") -eq "4,50,250,1000,5000" -and
        @($scenarioManifest.mutations).Count -ge 7 -and
        @($scenarioManifest.datastream_conditions).Count -eq 18
    ) -Message "Deterministic simulator fixture manifest is stale or damaged."

    Invoke-Cargo -Arguments @("fmt", "--all", "--", "--check")
    Invoke-Cargo -Arguments @("test", "--workspace", "--locked")
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
    Assert-True -Condition ($unit.status -in @("proposed", "ready", "active", "validating", "accepted")) `
        -Message "Milestone 0 has an unsupported workflow status."
    if ($unit.status -eq "ready") {
        $readyEventId = "$($unit.unit_id)-ready-0001"
        Get-TransitionBinding -Unit $unit -State $state -EventId $readyEventId `
            -ExpectedStatus "ready" -ExpectedCurrentUnit $null `
            -ExpectedNextReadyUnit $unit.unit_id | Out-Null
    }
    if ($unit.status -eq "active") {
        $claimedEventId = "$($unit.unit_id)-claimed-0002"
        Get-TransitionBinding -Unit $unit -State $state -EventId $claimedEventId `
            -ExpectedStatus "active" -ExpectedCurrentUnit $unit.unit_id `
            -ExpectedNextReadyUnit $null | Out-Null

        $claimPath = Join-Path $repoRoot "morphospace/receipts/$($unit.unit_id)-claim.json"
        Assert-True -Condition (Test-Path -LiteralPath $claimPath -PathType Leaf) `
            -Message "Active Milestone 0 is missing its claim receipt."
        $claim = Get-Content -LiteralPath $claimPath -Raw | ConvertFrom-Json -Depth 100
        $repositoryStates = @($claim.preservation.repository_states)
        $repositoryHeads = @($state.repository_heads)
        Assert-True -Condition (
            $claim.action -eq "Claim" -and
            $claim.executed -eq $true -and
            $claim.transition -eq "ready-to-active" -and
            $claim.status_before -eq "ready" -and
            $claim.status_after -eq "active" -and
            $claim.current_unit_after -eq $unit.unit_id -and
            $claim.event_id -eq $claimedEventId -and
            $claim.preservation.git_mutation_performed -eq $false -and
            $claim.preservation.device_mutation_performed -eq $false -and
            $repositoryStates.Count -eq 1 -and
            $repositoryHeads.Count -eq 1 -and
            $repositoryStates[0].repo_id -eq "rusty-fleet" -and
            $repositoryStates[0].head -eq $repositoryHeads[0].head -and
            $repositoryStates[0].branch -eq $repositoryHeads[0].branch -and
            $repositoryStates[0].dirty -eq $false -and
            $repositoryStates[0].relation -eq "synchronized" -and
            $repositoryHeads[0].head -eq "1f9a4ba833bae9b0684bb91758fe304c526699da" -and
            $repositoryHeads[0].branch -eq "main"
        ) -Message "Active Milestone 0 claim baseline is stale or damaged."
    }
    if ($unit.status -eq "validating") {
        Assert-True -Condition (
            $state.current_unit -eq $unit.unit_id -and
            $state.next_ready_unit -eq $null
        ) -Message "Validating Milestone 0 is not the current workflow authority."
    }
    if ($unit.status -eq "accepted") {
        $acceptedEventId = "$($unit.unit_id)-accepted-0005"
        Get-TransitionBinding -Unit $unit -State $state -EventId $acceptedEventId `
            -ExpectedStatus "accepted" -ExpectedCurrentUnit $null `
            -ExpectedNextReadyUnit $null -Historical | Out-Null
        Assert-True -Condition (
            $state.validation_checkpoint.result -eq "pass" -and
            $state.last_accepted_receipt -eq $state.validation_checkpoint.receipt -and
            @($unit.instruction_surfaces | Where-Object { $_.status -ne "complete" }).Count -eq 0
        ) -Message "Accepted Milestone 0 is missing passing validation or instruction completion."
    }
    Assert-True -Condition (@($unit.acceptance).Count -ge 5) `
        -Message "Milestone 0 must remain a vertical stack with complete acceptance coverage."
    Assert-True -Condition (@($unit.acceptance.acceptance_id) -contains "canonical-datastream-projections") `
        -Message "Milestone 0 must include source-only datastream contract acceptance."
    $threatModel = Get-Content -LiteralPath (
        Join-Path $repoRoot "docs/decisions/0004-m0-source-boundary-and-threat-model.md"
    ) -Raw
    foreach ($phrase in @(
        "source epoch",
        "previously seen",
        "finite contract limits",
        "pre-deserialization",
        "accepted for Milestone 0"
    )) {
        Assert-True -Condition ($threatModel.Contains($phrase, [StringComparison]::OrdinalIgnoreCase)) `
            -Message "Milestone 0 threat-model guardrail is missing: $phrase"
    }

    $m1UnitPath = Join-Path $repoRoot "morphospace/iteration-units/fleet-m1-local-no-adb-monitoring.json"
    Assert-True -Condition (Test-Path -LiteralPath $m1UnitPath -PathType Leaf) `
        -Message "The Milestone 1 local-monitoring stack is missing."
    $m1Unit = Get-Content -LiteralPath $m1UnitPath -Raw | ConvertFrom-Json -Depth 100
    Assert-True -Condition (
        $m1Unit.unit_id -eq "fleet-m1-local-no-adb-monitoring" -and
        $m1Unit.status -in @("active", "validating", "accepted") -and
        @($m1Unit.allowed_repositories.repo_id) -contains "rusty-fleet" -and
        @($m1Unit.allowed_repositories.repo_id) -contains "rusty-quest" -and
        @($m1Unit.acceptance.acceptance_id) -contains "authenticated-no-adb-checkin" -and
        @($m1Unit.acceptance.acceptance_id) -contains "cli-api-ui-parity" -and
        @($m1Unit.acceptance.acceptance_id) -contains "stacked-validation-and-cleanup"
    ) -Message "Milestone 1 is not one complete local-monitoring stack."
    if ($m1Unit.status -eq "active") {
        Get-TransitionBinding -Unit $m1Unit -State $state `
            -EventId "$($m1Unit.unit_id)-claimed-0007" `
            -ExpectedStatus "active" -ExpectedCurrentUnit $m1Unit.unit_id `
            -ExpectedNextReadyUnit $null | Out-Null
    }

    $m1Text = $m1Unit | ConvertTo-Json -Depth 100 -Compress
    foreach ($requiredBoundary in @(
        "5c3679b5b7faaacfe65daecfaf48d442af279870",
        "d0782ea5a79bf88ef16b5d0adb8792803d5705ea",
        "without ADB",
        "unpublished Rusty LSL P68 candidate",
        "platform-limited foreground"
    )) {
        Assert-True -Condition (
            $m1Text.Contains($requiredBoundary, [StringComparison]::OrdinalIgnoreCase)
        ) -Message "Milestone 1 owner or support boundary is missing: $requiredBoundary"
    }

    foreach ($decision in @(
        "docs/decisions/0005-m1-checkin-authority.md",
        "docs/decisions/0006-m1-local-ingress-threat-model.md"
    )) {
        $decisionText = Get-Content -LiteralPath (Join-Path $repoRoot $decision) -Raw
        foreach ($requiredDecisionPhrase in @(
            "Manifold",
            "no ADB",
            "bounded",
            "replay",
            "received time"
        )) {
            Assert-True -Condition (
                $decisionText.Contains($requiredDecisionPhrase, [StringComparison]::OrdinalIgnoreCase)
            ) -Message "$decision is missing M1 guardrail: $requiredDecisionPhrase"
        }
    }
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
    Test-SourceImplementation
    Test-WpfConsole
    Test-DatastreamPlanning
    Invoke-Git diff --check

    if ($Tier -in @("Standard", "Deep")) {
        Test-MarkdownLinks
        Invoke-Cargo -Arguments @(
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings"
        )

        $cliList = & cargo run --quiet --locked -p fleetctl -- list 4
        if ($LASTEXITCODE -ne 0) {
            throw "fleetctl list smoke test failed."
        }
        $cliProjection = $cliList | ConvertFrom-Json -Depth 100
        Assert-True -Condition (
            $cliProjection.schema -eq "rusty.fleet.query_result.v1" -and
            $cliProjection.total_count -eq 4 -and
            $cliProjection.window_count -eq 4
        ) -Message "fleetctl list did not return the canonical four-device projection."

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
