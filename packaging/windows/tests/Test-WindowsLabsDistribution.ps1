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
$signing = Read-Repo "packaging/windows/Sign-WindowsArtifacts.ps1"
$policy = Read-Repo "packaging/windows/trust/release-policy.json" |
    ConvertFrom-Json -Depth 20
$schema = Read-Repo "schemas/rusty.fleet.windows_release.v4.schema.json" | ConvertFrom-Json -Depth 20
Assert-Labs (@($schema.properties.distribution_track.enum) -ccontains "github-prerelease") "release schema does not admit the Labs transport"
Assert-Labs (@($schema.properties.channel.enum) -ccontains "labs") "release schema does not preserve Fleet channel"
Assert-Labs (@($schema.properties.product_channel.enum) -ccontains "labs") "release schema lacks the persistent Labs product channel"
Assert-Labs (@($schema.properties.maturity.enum) -ccontains "alpha") "release schema does not preserve alpha maturity"
foreach ($obsolete in @("dev", "labs", "stable")) {
    Assert-Labs (@($schema.properties.distribution_track.enum) -cnotcontains $obsolete) "obsolete distribution track '$obsolete' is still admitted"
}
$schemaText = Read-Repo "schemas/rusty.fleet.windows_release.v4.schema.json"
Assert-Labs (
    $schemaText -match '"channel": \{ "const": "labs" \}' -and
    $schemaText -match '"distribution_track": \{ "const": "github-prerelease" \}' -and
    $schemaText -match '"authenticode_trust_mode": \{ "const": "exact-pinned-self-issued-untrusted-root-only" \}' -and
    $schemaText -match '"public_trust_claim": \{ "const": false \}' -and
    $schemaText -match '"authenticode_trust_mode": \{ "const": "public-chain-only" \}' -and
    $schemaText -match '"public_trust_claim": \{ "const": true \}'
) "release schema does not reject cross-axis substitutions"
Assert-Labs ($bundle -match 'RustyFleet-Labs-\$Version-win-x64' -and $setup -match 'RustyFleet-Labs-Setup\.exe') "Labs artifacts are not independently named"
Assert-Labs (
    $bundle -match 'FleetAgentKeyRecordOwnerCapsuleRoot' -and
    $bundle -match 'signed release requires the exact pinned Rusty Quest key-record owner capsule' -and
    $bundle -match 'onboarding_ready = \$ownerCapsuleReady' -and
    $bundle -match 'rusty-quest-key-record-helper' -and
    $bundle -match 'packaging-and-tool-provenance-only'
) "Labs bundle does not fail closed on the exact Rusty Quest owner capsule"
Assert-Labs ($setup -match 'rusty-fleet-labs' -and $setup -match 'Rusty Fleet Labs' -and $setup -match 'RustyFleetLabs') "Labs installation identity is incomplete"
Assert-Labs (
    $descriptor -match '\$plan\.product -cne \$installationIdentity' -and
    $descriptor -match
        'schema = "rusty\.fleet\.windows_release_descriptor_receipt\.v5"' -and
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
Assert-Labs (
    $pages -match 'metadata/\$Channel/release\.json' -and
    $pages -match '\[ValidateSet\("labs", "stable"\)\]' -and
    $pages -notmatch 'local-development' -and
    $pages -match 'github-prerelease' -and
    $pages -match 'github-release' -and
    $descriptor -match '\[ValidateSet\("labs", "stable"\)\]' -and
    $publication -match '\[ValidateSet\("labs", "stable"\)\]'
) "public release metadata is not isolated to Labs and Stable"
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
Assert-Labs (
    $labsWorkflow -match 'environment: windows-labs-release' -and
    $labsWorkflow -match 'gh workflow run release-windows\.yml' -and
    $labsWorkflow -match 'HOSTESS_RELEASE_POLICY_URL' -and
    $labsWorkflow -match
        'hostess_release_policy_url=\$env:PROVIDER_RELEASE_POLICY' -and
    $labsWorkflow -notmatch 'intentional workflow stop' -and
    $publication -match '-Prerelease \(\$Channel -cne "stable"\)'
) "Labs workflow does not delegate the complete never-latest owner release"
Assert-Labs ($labsWorkflow -match 'vX\.Y\.Z-alpha\.N' -and $stableWorkflow -match "inputs\.channel == 'labs'") "alpha maturity tag or protected Labs routing is incomplete"
Assert-Labs ($stableWorkflow -match "inputs\.publish_release && inputs\.signing_mode == 'signed-release'") "stable publication gate changed"
Assert-Labs (
    $policy.schema -ceq "rusty.fleet.windows_release_trust_policy.v2" -and
    $policy.channels.labs.publication_enabled -eq $true -and
    $policy.channels.labs.authenticode.subject -ceq "CN=MesmerPrism" -and
    $policy.channels.labs.authenticode.thumbprint -ceq
        "08A5878AD6E652A94517D2C79144EB2655B0088C" -and
    $policy.channels.labs.authenticode.certificate_sha256 -ceq
        "baead63c37e32085c3af19b4c739a6a308d700529f107d40e14fec2c94fe7ddf" -and
    $policy.channels.labs.authenticode.self_issued -eq $true -and
    $policy.channels.labs.authenticode.public_trust_claim -eq $false -and
    $policy.channels.labs.authenticode.trust_mode -ceq
        "exact-pinned-self-issued-untrusted-root-only" -and
    $policy.channels.labs.authenticode.timestamp_required -eq $true -and
    @($policy.channels.labs.authenticode.allowed_chain_status_flags).Count -eq 1 -and
    @($policy.channels.labs.authenticode.allowed_chain_status_flags)[0] -ceq
        "UntrustedRoot" -and
    @($policy.channels.labs.authorized_descriptor_signer_spki_sha256).Count -eq 1 -and
    @($policy.channels.labs.authorized_descriptor_signer_spki_sha256)[0] -ceq
        "0b3ef04dc5481d5e0a0a243df298c31052501e014a6e27516c48b95846657d0c" -and
    $policy.channels.stable.publication_enabled -eq $false -and
    $policy.channels.stable.authenticode.trust_mode -ceq "public-chain-only" -and
    $policy.channels.stable.authenticode.public_trust_claim -eq $true -and
    @($policy.channels.stable.authorized_descriptor_signer_spki_sha256).Count -eq 0
) "production release policy does not preserve exact Labs-only trust"
Assert-Labs (
    $program -match "LABS SIGNATURE NOTICE" -and
    $program -match "Unknown publisher warning" -and
    $program -match "Setup never installs or changes a Windows root certificate"
) "Setup does not disclose the exact self-issued Labs trust boundary"
Assert-Labs (
    $signing -match 'Cert:\\CurrentUser\\My' -and
    $signing -notmatch 'Cert:\\(?:CurrentUser|LocalMachine)\\Root' -and
    $signing -notmatch 'StoreName\]::Root'
) "signing workflow may import or mutate a Windows Root store"
Assert-Labs (
    $signing -cmatch
        '\$timestampUrl = "http://timestamp\.digicert\.com"' -and
    $signing -cmatch '/tr \$timestampUrl' -and
    $signing -cmatch '/td SHA256' -and
    $signing -cnotmatch '\[string\] \$TimestampUrl' -and
    $signing -cnotmatch 'https://timestamp\.digicert\.com'
) "signing workflow does not use DigiCert's exact RFC 3161 SignTool endpoint"
[ordered]@{
    schema = "rusty.fleet.windows_labs_distribution_test.v1"
    result = "pass"
    complete_product_bundle = $true
    labs_identity_isolated = $true
    owner_release_metadata_exact = $true
    legacy_preview_not_mapped = $true
    stable_identity_preserved = $true
    prerelease_never_latest = $true
    labs_policy_enabled = $true
    stable_policy_enabled = $false
    self_issued_warning_visible = $true
} | ConvertTo-Json -Depth 5
