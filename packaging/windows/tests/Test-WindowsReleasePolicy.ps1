# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function ConvertTo-PolicyNewlines([string] $Text) {
    return $Text.Replace("`r`n", "`n").Replace("`r", "`n")
}

function Get-ReleaseWorkflowBoundaryFailures([string] $Text) {
    $portableText = ConvertTo-PolicyNewlines $Text
    $checks = [ordered]@{
        contents_read = (
            $portableText -match "(?m)^permissions:\n  contents: read$"
        )
        checkout_credentials_disabled = (
            $portableText -match
                "(?m)^          persist-credentials: false$"
        )
        ambient_github_token_absent = (
            $portableText -notmatch [regex]::Escape('${{ github.token }}')
        )
        protected_publish_token_present = (
            $portableText -match [regex]::Escape(
                '${{ secrets.FLEET_RELEASE_PUBLISH_TOKEN }}'
            )
        )
        exact_tag_ref_present = (
            $portableText -match
                [regex]::Escape('-ExpectedRef $env:GITHUB_REF')
        )
        signed_release_publish_condition_present = (
            $portableText -match [regex]::Escape(
                "if: inputs.publish_release && " +
                "inputs.signing_mode == 'signed-release'"
            )
        )
        isolated_preflight_present = (
            $portableText -match [regex]::Escape('-Mode Preflight')
        )
        isolated_publish_present = (
            $portableText -match [regex]::Escape('-Mode Publish')
        )
        direct_gh_release_command_absent = (
            $portableText -notmatch "\bgh release (?:create|edit|view)\b"
        )
    }
    return @(
        foreach ($check in $checks.GetEnumerator()) {
            if (-not [bool] $check.Value) {
                [string] $check.Key
            }
        }
    )
}

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
$workflowText = ConvertTo-PolicyNewlines (
    Get-Content -LiteralPath $workflowPath -Raw
)
$pagesWorkflowPath = Join-Path $repoRoot ".github\workflows\pages.yml"
$pagesWorkflowText = ConvertTo-PolicyNewlines (
    Get-Content -LiteralPath $pagesWorkflowPath -Raw
)
$pagesAuthorityPath = Join-Path $PSScriptRoot (
    "..\New-WindowsPagesDeployment.ps1"
)
$pagesAuthorityText = Get-Content -LiteralPath $pagesAuthorityPath -Raw
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
$releaseBoundaryFailures = @(
    Get-ReleaseWorkflowBoundaryFailures $workflowText
)
$releaseBoundaryCrLfFailures = @(
    Get-ReleaseWorkflowBoundaryFailures (
        $workflowText.Replace("`n", "`r`n")
    )
)
if ($releaseBoundaryFailures.Count -ne 0 -or
    $releaseBoundaryCrLfFailures.Count -ne 0) {
    throw (
        "release workflow does not retain its read-only exact-tag token " +
        "boundary: actual=[$($releaseBoundaryFailures -join ',')]; " +
        "crlf=[$($releaseBoundaryCrLfFailures -join ',')]"
    )
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

$requiredPagesVariables = @(
    "FLEET_METADATA_DEPLOYMENT_ENABLED",
    "FLEET_METADATA_VERSION",
    "FLEET_METADATA_CHANNEL",
    "FLEET_METADATA_SOURCE_REVISION",
    "FLEET_METADATA_SOURCE_TREE",
    "FLEET_SIGNER_THUMBPRINT",
    "HOSTESS_SIGNER_THUMBPRINT",
    "FLEET_DESCRIPTOR_SIGNER_SPKI_SHA256"
)
foreach ($variable in $requiredPagesVariables) {
    if ($pagesWorkflowText -notmatch [regex]::Escape($variable)) {
        throw "Pages renewal workflow is missing public trust input: $variable"
    }
}
if ($pagesWorkflowText -notmatch "(?m)^permissions:\n  contents: read$" -or
    $pagesWorkflowText -notmatch "(?m)^  cancel-in-progress: false$" -or
    $pagesWorkflowText -notmatch
        "(?m)^    environment: windows-release-metadata$" -or
    $pagesWorkflowText -notmatch [regex]::Escape(
        '${{ secrets.FLEET_DESCRIPTOR_SIGNING_KEY_PEM_BASE64 }}'
    ) -or
    $pagesWorkflowText -match
        "secrets\.(?:FLEET_SIGNING_PFX|FLEET_RELEASE_PUBLISH_TOKEN)" -or
    $pagesWorkflowText -notmatch [regex]::Escape("-Mode Preflight") -or
    $pagesWorkflowText -notmatch [regex]::Escape(
        "New-WindowsPagesDeployment.ps1"
    ) -or
    $pagesWorkflowText -match
        "\bgh release (?:create|upload|edit|delete)\b") {
    throw "Pages workflow does not preserve the protected renewal boundary"
}
$requiredPagesAuthorityEvidence = @(
    "rusty.fleet.windows_release_metadata_handoff.v2",
    "publication preflight asset inventory is not closed",
    "release descriptor RSA-PSS signature is invalid",
    "release metadata renewal is stale, downgraded, or replayed",
    "Pages payload contains a prohibited binary or key asset",
    "interrupted Pages deployment stage requires explicit resume",
    "existing Pages deployment output is not an exact resumable handoff",
    "binary_authority = ""github_releases""",
    "pages_binary_count = 0"
)
foreach ($evidence in $requiredPagesAuthorityEvidence) {
    if ($pagesAuthorityText -notmatch [regex]::Escape($evidence)) {
        throw "Pages deployment authority is missing evidence: $evidence"
    }
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
    renewable_pages_metadata = $true
    pages_binary_count = 0
    metadata_renewal_secret_count = 1
    workflow_newline_forms = @("lf", "crlf")
} | ConvertTo-Json -Depth 5
