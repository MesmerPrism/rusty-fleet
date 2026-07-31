# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
function Read-Repo([string] $Path) { Get-Content -LiteralPath (Join-Path $repo $Path) -Raw }
function Assert-Labs([bool] $Condition, [string] $Message) { if (-not $Condition) { throw $Message } }
$bundle = Read-Repo "packaging/windows/New-WindowsBundle.ps1"
$setup = Read-Repo "packaging/windows/New-WindowsSetup.ps1"
$engine = Read-Repo "apps/fleet-setup/SetupEngine.cs"
$program = Read-Repo "apps/fleet-setup/Program.cs"
$pages = Read-Repo "packaging/windows/New-WindowsPagesDeployment.ps1"
$pagesWorkflow = Read-Repo ".github/workflows/pages.yml"
$stableWorkflow = Read-Repo ".github/workflows/release-windows.yml"
$labsWorkflow = Read-Repo ".github/workflows/release-windows-labs.yml"
$publication = Read-Repo "packaging/windows/Publish-WindowsRelease.ps1"
$descriptor = Read-Repo "packaging/windows/New-WindowsReleaseDescriptor.ps1"
$schema = Read-Repo "schemas/rusty.fleet.windows_release.v3.schema.json" | ConvertFrom-Json -Depth 20
Assert-Labs (@($schema.properties.distribution_track.enum) -ccontains "github-prerelease") "release schema does not admit the Labs transport"
Assert-Labs (@($schema.properties.channel.enum) -ccontains "labs") "release schema does not preserve Fleet channel"
Assert-Labs (@($schema.properties.product_channel.enum) -ccontains "labs") "release schema lacks the persistent Labs product channel"
Assert-Labs (@($schema.properties.maturity.enum) -ccontains "alpha") "release schema does not preserve alpha maturity"
foreach ($obsolete in @("dev", "labs", "stable")) {
    Assert-Labs (@($schema.properties.distribution_track.enum) -cnotcontains $obsolete) "obsolete distribution track '$obsolete' is still admitted"
}
$schemaText = Read-Repo "schemas/rusty.fleet.windows_release.v3.schema.json"
Assert-Labs (
    $schemaText -match '"channel": \{ "const": "dev" \}' -and
    $schemaText -match '"distribution_track": \{ "const": "local-development" \}' -and
    $schemaText -match '"channel": \{ "const": "labs" \}' -and
    $schemaText -match '"distribution_track": \{ "const": "github-prerelease" \}' -and
    $schemaText -match '"channel": \{ "const": "stable" \}' -and
    $schemaText -match '"distribution_track": \{ "const": "github-release" \}'
) "release schema does not reject cross-axis substitutions"
Assert-Labs ($bundle -match 'RustyFleet-Labs-\$Version-win-x64' -and $setup -match 'RustyFleet-Labs-Setup\.exe') "Labs artifacts are not independently named"
Assert-Labs ($setup -match 'rusty-fleet-labs' -and $setup -match 'Rusty Fleet Labs' -and $setup -match 'RustyFleetLabs') "Labs installation identity is incomplete"
Assert-Labs (
    $descriptor -match '\$plan\.product -cne \$installationIdentity' -and
    $descriptor -match
        'schema = "rusty\.fleet\.windows_release_descriptor_receipt\.v4"' -and
    $descriptor -match 'release_tag = \$ReleaseTag' -and
    $descriptor -match 'installation_identity = \$installationIdentity' -and
    $descriptor -match 'role = "complete-product"' -and
    $descriptor -match 'name = \$setupName' -and
    $descriptor -match 'bytes = \[long\] \$setupInfo\.Length' -and
    $publication -match
        '\$descriptorReceipt\.release_tag -cne \$tag' -and
    $pages -match
        '\$descriptorReceipt\.release_tag -cne \$ReleaseTag'
) "owner release metadata does not bind the exact Labs release identity"
Assert-Labs ($program -match 'ReleaseConfiguration\.InstallDirectoryName' -and $program -match 'ReleaseConfiguration\.ProductId' -and $engine -match 'channel is not \("dev" or "labs" or "stable"\)') "Setup does not bind Labs identity"
Assert-Labs ($engine -match 'SpecialFolder\.Programs' -and $engine -match 'CurrentVersion\\Uninstall' -and $engine -match 'ReleaseConfiguration\.DisplayName' -and $engine -match 'ReleaseConfiguration\.ProductId') "channel-specific shortcuts or uninstall registration are absent"
Assert-Labs ($pages -match 'metadata/\$Channel/release\.json' -and $pages -match 'local-development' -and $pages -match 'github-prerelease' -and $pages -match 'github-release') "Pages metadata is not channel isolated"
Assert-Labs (
    $pagesWorkflow -match '"RESOLVED_RELEASE_TAG=\$tag" >> \$env:GITHUB_ENV' -and
    $pagesWorkflow -match '\$tag = \$env:RESOLVED_RELEASE_TAG' -and
    $pagesWorkflow -match '-ReleaseTag \$env:RESOLVED_RELEASE_TAG' -and
    $pagesWorkflow -match
        '(?s)Publish-WindowsRelease\.ps1.*-Channel \$env:CHANNEL.*-ReleaseTag \$env:RESOLVED_RELEASE_TAG' -and
    $pagesWorkflow -match
        ([regex]::Escape('$productStem = if ($env:CHANNEL -ceq ''labs'')')) -and
    $pagesWorkflow -match "'RustyFleet-Labs'" -and
    $pagesWorkflow -match '\$setupName = "\$productStem-Setup\.exe"' -and
    $pagesWorkflow -match '\$bundle = "\$productStem-\$env:VERSION-win-x64"' -and
    $pagesWorkflow -notmatch "(?m)^\s+'RustyFleet-Setup\.exe',$" -and
    $pagesWorkflow -notmatch
        '(?m)^\s+\$bundle = "RustyFleet-\$env:VERSION-win-x64"$' -and
    $pagesWorkflow -match 'ReleaseTag = \$env:RESOLVED_RELEASE_TAG' -and
    $pagesWorkflow -notmatch 'ExpectedRef "refs/tags/v\$env:VERSION"' -and
    $pagesWorkflow -match
        'elseif \(\$response\.StatusCode -ne 404\) \{\s*throw ''prior Pages metadata handoff lookup failed closed'''
) "Pages does not retain the exact resolved maturity tag or contains a detached branch"
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
Assert-Labs ($workflowPowerShell.Count -gt 0) "Pages PowerShell steps were not found"
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
    Assert-Labs ($errors.Count -eq 0) (
        "Pages PowerShell step does not parse: " +
        ($errors | ForEach-Object Message | Select-Object -First 1)
    )
}
Assert-Labs ($labsWorkflow -match 'environment: windows-labs-release' -and $labsWorkflow -match 'gh workflow run release-windows\.yml' -and $labsWorkflow -notmatch 'intentional workflow stop' -and $publication -match '-Prerelease \(\$Channel -cne "stable"\)') "Labs workflow does not delegate the complete never-latest owner release"
Assert-Labs ($labsWorkflow -match 'vX\.Y\.Z-alpha\.N' -and $stableWorkflow -match "inputs\.channel == 'labs'") "alpha maturity tag or protected Labs routing is incomplete"
Assert-Labs ($stableWorkflow -match "inputs\.publish_release && inputs\.signing_mode == 'signed-release'") "stable publication gate changed"
[ordered]@{schema="rusty.fleet.windows_labs_distribution_test.v1";result="pass";complete_product_bundle=$true;labs_identity_isolated=$true;owner_release_metadata_exact=$true;legacy_preview_not_mapped=$true;stable_identity_preserved=$true;prerelease_never_latest=$true;production_policy_enabled=$false} | ConvertTo-Json -Depth 5
