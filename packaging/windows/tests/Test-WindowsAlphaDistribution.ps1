# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
function Read-Repo([string] $Path) { Get-Content -LiteralPath (Join-Path $repo $Path) -Raw }
function Assert-Alpha([bool] $Condition, [string] $Message) { if (-not $Condition) { throw $Message } }
$bundle = Read-Repo "packaging/windows/New-WindowsBundle.ps1"
$setup = Read-Repo "packaging/windows/New-WindowsSetup.ps1"
$engine = Read-Repo "apps/fleet-setup/SetupEngine.cs"
$program = Read-Repo "apps/fleet-setup/Program.cs"
$pages = Read-Repo "packaging/windows/New-WindowsPagesDeployment.ps1"
$pagesWorkflow = Read-Repo ".github/workflows/pages.yml"
$stableWorkflow = Read-Repo ".github/workflows/release-windows.yml"
$alphaWorkflow = Read-Repo ".github/workflows/release-windows-alpha.yml"
$publication = Read-Repo "packaging/windows/Publish-WindowsRelease.ps1"
$schema = Read-Repo "schemas/rusty.fleet.windows_release.v2.schema.json" | ConvertFrom-Json -Depth 20
Assert-Alpha (@($schema.properties.channel.enum) -ccontains "alpha") "release schema does not admit alpha"
Assert-Alpha ($bundle -match 'RustyFleet-Alpha-\$Version-win-x64' -and $setup -match 'RustyFleet-Alpha-Setup\.exe') "alpha artifacts are not independently named"
Assert-Alpha ($setup -match 'rusty-fleet-alpha' -and $setup -match 'Rusty Fleet Alpha' -and $setup -match 'RustyFleetAlpha') "alpha installation identity is incomplete"
Assert-Alpha ($program -match 'ReleaseConfiguration\.InstallDirectoryName' -and $program -match 'ReleaseConfiguration\.ProductId' -and $engine -match 'channel is not \("dev" or "alpha" or "preview" or "stable"\)') "Setup does not bind alpha identity or preview compatibility"
Assert-Alpha ($engine -match 'SpecialFolder\.Programs' -and $engine -match 'CurrentVersion\\Uninstall' -and $engine -match 'ReleaseConfiguration\.DisplayName' -and $engine -match 'ReleaseConfiguration\.ProductId') "channel-specific shortcuts or uninstall registration are absent"
Assert-Alpha ($pages -match 'metadata/\$Channel/release\.json' -and $pages -match 'dev\|alpha\|preview\|stable') "Pages metadata is not channel isolated"
Assert-Alpha (
    $pagesWorkflow -match '"RESOLVED_RELEASE_TAG=\$tag" >> \$env:GITHUB_ENV' -and
    $pagesWorkflow -match '\$tag = \$env:RESOLVED_RELEASE_TAG' -and
    $pagesWorkflow -match '-ReleaseTag \$env:RESOLVED_RELEASE_TAG' -and
    $pagesWorkflow -match
        ([regex]::Escape('$productStem = if ($env:CHANNEL -ceq ''alpha'')')) -and
    $pagesWorkflow -match "'RustyFleet-Alpha'" -and
    $pagesWorkflow -match '\$setupName = "\$productStem-Setup\.exe"' -and
    $pagesWorkflow -match '\$bundle = "\$productStem-\$env:VERSION-win-x64"' -and
    $pagesWorkflow -notmatch "(?m)^\s+'RustyFleet-Setup\.exe',$" -and
    $pagesWorkflow -notmatch
        '(?m)^\s+\$bundle = "RustyFleet-\$env:VERSION-win-x64"$' -and
    $pagesWorkflow -match 'ReleaseTag = \$env:RESOLVED_RELEASE_TAG' -and
    $pagesWorkflow -notmatch 'ExpectedRef "refs/tags/v\$env:VERSION"' -and
    $pagesWorkflow -notmatch '(?m)^\s+elseif \(\$response\.StatusCode'
) "Pages does not retain the exact resolved alpha tag or contains a detached branch"
$workflowLines = $pagesWorkflow -split '\r?\n'
$workflowPowerShell = [Collections.Generic.List[string]]::new()
for ($lineIndex = 0; $lineIndex -lt $workflowLines.Count; $lineIndex++) {
    if ($workflowLines[$lineIndex] -cnotmatch '^\s{8}run:\s*\|$') {
        continue
    }
    $body = [Collections.Generic.List[string]]::new()
    for ($bodyIndex = $lineIndex + 1; $bodyIndex -lt $workflowLines.Count; $bodyIndex++) {
        $line = $workflowLines[$bodyIndex]
        if ($line.Length -ne 0 -and $line -cnotmatch '^\s{10}') {
            break
        }
        $body.Add(($line -replace '^\s{10}', ''))
    }
    $workflowPowerShell.Add(($body -join "`n"))
}
Assert-Alpha ($workflowPowerShell.Count -gt 0) "Pages PowerShell steps were not found"
foreach ($scriptTextValue in $workflowPowerShell) {
    $scriptText = $scriptTextValue
    $scriptText = [regex]::Replace(
        $scriptText,
        '\$\{\{[^}]+\}\}',
        "'workflow-expression'"
    )
    $tokens = $null
    $errors = $null
    [void] [Management.Automation.Language.Parser]::ParseInput(
        $scriptText,
        [ref] $tokens,
        [ref] $errors
    )
    Assert-Alpha ($errors.Count -eq 0) (
        "Pages PowerShell step does not parse: " +
        ($errors | ForEach-Object Message | Select-Object -First 1)
    )
}
Assert-Alpha ($alphaWorkflow -match 'environment: windows-alpha-release' -and $alphaWorkflow -match 'gh workflow run release-windows\.yml' -and $alphaWorkflow -notmatch 'intentional workflow stop' -and $publication -match '-Prerelease \(\$Channel -cne "stable"\)') "alpha workflow does not delegate the complete never-latest owner release"
Assert-Alpha ($stableWorkflow -match 'vX\.Y\.Z-alpha\.N' -and $stableWorkflow -match 'inputs\.channel == ''alpha''') "alpha tag or protected environment routing is incomplete"
Assert-Alpha ($stableWorkflow -match "inputs\.publish_release && inputs\.signing_mode == 'signed-release'") "stable publication gate changed"
[ordered]@{schema="rusty.fleet.windows_alpha_distribution_test.v1";result="pass";complete_product_bundle=$true;alpha_identity_isolated=$true;preview_compatibility="deprecated_input_only";stable_identity_preserved=$true;prerelease_never_latest=$true;production_policy_enabled=$false} | ConvertTo-Json -Depth 5
