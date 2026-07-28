# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$policyPath = Join-Path $PSScriptRoot "..\trust\release-policy.json"
$policyText = Get-Content -LiteralPath $policyPath -Raw
$policy = $policyText | ConvertFrom-Json -Depth 10
$expectedProperties = @(
    "authorized_descriptor_signer_spki_sha256",
    "authorized_fleet_signer_thumbprints",
    "authorized_hostess_signer_thumbprints",
    "publication_enabled",
    "schema",
    "status"
) | Sort-Object
$actualProperties = @($policy.PSObject.Properties.Name | Sort-Object)
if (($expectedProperties -join "`n") -cne ($actualProperties -join "`n") -or
    $policy.schema -cne "rusty.fleet.windows_release_trust_policy.v1" -or
    $policy.publication_enabled -ne $false -or
    @($policy.authorized_fleet_signer_thumbprints).Count -ne 0 -or
    @($policy.authorized_hostess_signer_thumbprints).Count -ne 0 -or
    @($policy.authorized_descriptor_signer_spki_sha256).Count -ne 0 -or
    $policy.status -cne "no_production_signing_authority_configured") {
    throw "the checked-in release policy must fail publication closed until reviewed pins exist"
}
if ($policyText -match "[A-Za-z]:\\" -or
    $policyText -match "BEGIN .*PRIVATE KEY" -or
    $policyText -match "\b(?:10|127|192)\.") {
    throw "release policy contains a private or machine-local value"
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$workflowPath = Join-Path $repoRoot ".github\workflows\release-windows.yml"
$workflowText = Get-Content -LiteralPath $workflowPath -Raw
$publicationAuthorityPath = Join-Path $PSScriptRoot (
    "..\Publish-WindowsRelease.ps1"
)
$publicationAuthorityText = Get-Content `
    -LiteralPath $publicationAuthorityPath `
    -Raw
$remoteAuthorityPath = Join-Path $PSScriptRoot (
    "..\Publication.Remote.psm1"
)
$remoteAuthorityText = Get-Content `
    -LiteralPath $remoteAuthorityPath `
    -Raw
if ($workflowText -notmatch "(?m)^permissions:\r?\n  contents: read$" -or
    $workflowText -notmatch "(?m)^          persist-credentials: false$" -or
    $workflowText -match [regex]::Escape('${{ github.token }}') -or
    $workflowText -notmatch
        [regex]::Escape('${{ secrets.FLEET_RELEASE_PUBLISH_TOKEN }}') -or
    $workflowText -notmatch
        [regex]::Escape('-ExpectedRef $env:GITHUB_REF') -or
    $workflowText -notmatch [regex]::Escape(
        "if: inputs.publish_release && inputs.signing_mode == 'signed-release'"
    ) -or
    $workflowText -notmatch [regex]::Escape(
        '-Mode Preflight'
    ) -or
    $workflowText -notmatch [regex]::Escape('-Mode Publish') -or
    $workflowText -match "\bgh release (?:create|edit|view)\b") {
    throw "release workflow does not retain its read-only exact-tag token boundary"
}
$requiredAuthorityEvidence = @(
    "RetainedAssetSet",
    "publication inputs must be on a fixed local volume",
    "publication input inventory has additions, omissions, or casing changes",
    "staged release policy is not byte-exact to the tagged policy",
    "Setup Authenticode signer is not independently authorized",
    "ZIP SHA-256 sidecar",
    "RFC 8785 closed release payload",
    "release descriptor RSA-PSS signature is invalid",
    "Publish-RustyFleetGitHubRelease",
    "[Environment]::GetEnvironmentVariable"
)
foreach ($evidence in $requiredAuthorityEvidence) {
    if ($publicationAuthorityText -notmatch [regex]::Escape($evidence)) {
        throw "publication authority is missing required evidence: $evidence"
    }
}
$requiredRemoteAuthorityEvidence = @(
    "RemoteBoundedProcessRunner",
    "remote release tag does not resolve to the expected source revision",
    "remote release tag chain is cyclic or exceeds its bound",
    "remote release inventory exceeded its bounded search",
    "remote release asset digest, size, or state is not exact",
    "remote release contains an extra, duplicate, or malformed asset",
    "draft release asset upload",
    "draft release visibility transition"
)
foreach ($evidence in $requiredRemoteAuthorityEvidence) {
    if ($remoteAuthorityText -notmatch [regex]::Escape($evidence)) {
        throw "remote publication authority is missing required evidence: $evidence"
    }
}
$preflightStart = $workflowText.IndexOf(
    "      - name: Stage and preflight exact publication inputs",
    [StringComparison]::Ordinal
)
$evidenceStart = $workflowText.IndexOf(
    "      - name: Upload release evidence",
    [StringComparison]::Ordinal
)
$publicationStart = $workflowText.IndexOf(
    "      - name: Publish GitHub Release assets",
    [StringComparison]::Ordinal
)
if ($preflightStart -lt 0 -or
    $evidenceStart -le $preflightStart -or
    $publicationStart -le $evidenceStart) {
    throw "release preflight, evidence, and publication steps are not ordered exactly"
}
$preflightStep = $workflowText.Substring(
    $preflightStart,
    $evidenceStart - $preflightStart
)
if ($preflightStep -match
        [regex]::Escape('${{ secrets.FLEET_RELEASE_PUBLISH_TOKEN }}') -or
    $preflightStep -match "\bGH_TOKEN\b" -or
    $preflightStep -match "\bgh\b") {
    throw "publication preflight can access a token or gh"
}
$evidenceStep = $workflowText.Substring(
    $evidenceStart,
    $publicationStart - $evidenceStart
)
if ($evidenceStep -match "(?m)/\*\.zip\r?$" -or
    $evidenceStep -match "RustyFleet-Setup\.exe") {
    throw "Actions release evidence must not contain Fleet binaries"
}

[ordered]@{
    schema = "rusty.fleet.windows_release_policy_test.v1"
    result = "pass"
    publication_enabled = $false
    authorized_fleet_signers = 0
    authorized_hostess_signers = 0
    authorized_descriptor_signers = 0
    release_blocker = "production_trust_pins_not_configured"
    workflow_contents_permission = "read"
    checkout_credentials_persisted = $false
    actions_binary_artifacts = 0
    isolated_publication_authority = $true
    token_free_preflight = $true
} | ConvertTo-Json -Depth 5
