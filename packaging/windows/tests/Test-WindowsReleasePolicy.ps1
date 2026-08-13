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
        hostess_release_policy_input_present = (
            $portableText -match "(?m)^      hostess_release_policy_url:$"
        )
        hostess_release_policy_download_present = (
            $portableText -match
                [regex]::Escape('$env:PROVIDER_RELEASE_POLICY_URL') -and
            $portableText -match
                "rusty-hostess-hotspot-provider\.release-policy\.json"
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

function Assert-ExactUploadArtifactStep {
    param(
        [Parameter(Mandatory = $true)][string] $WorkflowText,
        [Parameter(Mandatory = $true)][string] $StepName,
        [Parameter(Mandatory = $true)][string] $ArtifactName,
        [Parameter(Mandatory = $true)][string[]] $Paths
    )

    $startMarker = "      - name: $StepName"
    $start = $WorkflowText.IndexOf($startMarker, [StringComparison]::Ordinal)
    if ($start -lt 0) {
        throw "upload-artifact step is absent: $StepName"
    }
    $next = $WorkflowText.IndexOf(
        "      - name:",
        $start + $startMarker.Length,
        [StringComparison]::Ordinal
    )
    if ($next -lt 0) {
        $next = $WorkflowText.Length
    }
    $actual = $WorkflowText.Substring($start, $next - $start).TrimEnd()

    $expectedLines = @(
        $startMarker,
        "        uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1",
        "        with:",
        "          name: $ArtifactName",
        "          path: |"
    )
    $expectedLines += @($Paths | ForEach-Object { "            $_" })
    $expectedLines += @(
        "          if-no-files-found: error",
        "          retention-days: 0",
        "          compression-level: 6",
        "          overwrite: false",
        "          include-hidden-files: false",
        "          archive: true"
    )
    $expected = $expectedLines -join "`n"
    if ($actual -cne $expected) {
        throw "upload-artifact contract drifted: $StepName"
    }
}

$policyPath = Join-Path $PSScriptRoot "..\trust\release-policy.json"
$policyText = Get-Content -LiteralPath $policyPath -Raw
Import-Module (Join-Path $PSScriptRoot "..\Distribution.Common.psm1") -Force
$labsPolicy = Read-RustyFleetReleaseTrustPolicy `
    -LiteralPath $policyPath `
    -Channel labs
$stablePolicy = Read-RustyFleetReleaseTrustPolicy `
    -LiteralPath $policyPath `
    -Channel stable
if ($labsPolicy.publication_enabled -ne $true -or
    $labsPolicy.authenticode.subject -cne "CN=MesmerPrism" -or
    $labsPolicy.authenticode.thumbprint -cne
        "08A5878AD6E652A94517D2C79144EB2655B0088C" -or
    $labsPolicy.authenticode.certificate_sha256 -cne
        "baead63c37e32085c3af19b4c739a6a308d700529f107d40e14fec2c94fe7ddf" -or
    $labsPolicy.authenticode.public_trust_claim -ne $false -or
    $labsPolicy.authenticode.trust_mode -cne
        "exact-pinned-self-issued-untrusted-root-only" -or
    $stablePolicy.publication_enabled -ne $false -or
    $stablePolicy.authenticode.public_trust_claim -ne $true -or
    $stablePolicy.authenticode.trust_mode -cne "public-chain-only" -or
    @($labsPolicy.authorized_descriptor_signer_spki_sha256)[0] -cne
        "0b3ef04dc5481d5e0a0a243df298c31052501e014a6e27516c48b95846657d0c") {
    throw "checked-in channel trust policy is not exact"
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
Assert-ExactUploadArtifactStep `
    -WorkflowText $workflowText `
    -StepName "Upload release evidence" `
    -ArtifactName 'RustyFleet-${{ inputs.version }}-release-evidence' `
    -Paths @(
        "artifacts/windows-distribution/*.zip.sha256",
        "artifacts/windows-distribution/*.manifest.json",
        "artifacts/windows-distribution/*.checksums.sha256",
        "artifacts/windows-distribution/*.validation-receipt.json",
        "artifacts/windows-distribution/*.build-receipt.json",
        "artifacts/windows-distribution/pages-metadata/release.json",
        "artifacts/windows-distribution/pages-metadata/release-descriptor.receipt.json",
        "artifacts/windows-distribution/pages-metadata/release-descriptor.spki.der",
        "artifacts/windows-distribution/publication-preflight.receipt.json"
    )
Assert-ExactUploadArtifactStep `
    -WorkflowText $pagesWorkflowText `
    -StepName "Upload renewal evidence only" `
    -ArtifactName 'RustyFleet-${{ vars.FLEET_METADATA_CHANNEL }}-metadata-renewal-evidence' `
    -Paths @(
        '${{ runner.temp }}/metadata-preflight.json',
        '${{ runner.temp }}/metadata-deployment-handoff.json',
        '${{ runner.temp }}/fleet-renewed-metadata/release.json',
        '${{ runner.temp }}/fleet-renewed-metadata/release-descriptor.receipt.json'
    )
$pagesAuthorityPath = Join-Path $PSScriptRoot (
    "..\New-WindowsPagesDeployment.ps1"
)
$pagesAuthorityText = Get-Content -LiteralPath $pagesAuthorityPath -Raw
$descriptorAuthorityPath = Join-Path $PSScriptRoot (
    "..\New-WindowsReleaseDescriptor.ps1"
)
$descriptorAuthorityText = Get-Content `
    -LiteralPath $descriptorAuthorityPath `
    -Raw
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
    "FLEET_METADATA_TAG",
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
    $pagesWorkflowText -notmatch [regex]::Escape(
        '${{ secrets.FLEET_RELEASE_PUBLISH_TOKEN }}'
    ) -or
    $pagesWorkflowText -match "secrets\.FLEET_SIGNING_PFX" -or
    $pagesWorkflowText -notmatch [regex]::Escape("-Mode Preflight") -or
    $pagesWorkflowText -notmatch [regex]::Escape(
        "New-WindowsPagesDeployment.ps1"
    ) -or
    $pagesWorkflowText -notmatch [regex]::Escape(
        "`$env:CHANNEL -cnotmatch '^(?:labs|stable)`$'"
    ) -or
    $pagesAuthorityText -notmatch
        '\[ValidateSet\("labs", "stable"\)\]' -or
    $descriptorAuthorityText -notmatch
        '\[ValidateSet\("labs", "stable"\)\]' -or
    $publicationAuthorityText -notmatch
        '\[ValidateSet\("labs", "stable"\)\]' -or
    $pagesAuthorityText -match 'local-development' -or
    $descriptorAuthorityText -match 'local-development' -or
    $pagesWorkflowText -match
        "\bgh release (?:create|upload|edit|delete)\b" -or
    $pagesWorkflowText -match 'actions/(?:configure|deploy)-pages@' -or
    $pagesWorkflowText -match 'actions/upload-pages-artifact@' -or
    $pagesWorkflowText -match '(?m)^\s+pages: write$' -or
    $pagesWorkflowText -match '(?m)^\s+id-token: write$' -or
    $pagesWorkflowText -notmatch 'rusty\.fleet\.pages_projection_request\.v1' -or
    $pagesWorkflowText -notmatch 'rusty\.fleet\.pages_projection_dispatch\.v1' -or
    $pagesWorkflowText -notmatch
        'repos/MesmerPrism/MesmerPrism\.github\.io/dispatches' -or
    $pagesWorkflowText -notmatch '\$requestBytes\.Length -gt 45000' -or
    $pagesWorkflowText -notmatch
        '\[Text\.Encoding\]::UTF8\.GetByteCount\(\$dispatchText\) -gt 65535' -or
    $pagesWorkflowText -notmatch
        'https://mesmerprism\.com/Rusty-Fleet/metadata/' -or
    $pagesWorkflowText -notmatch
        'central Pages metadata did not become exactly readable in time') {
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
    schema = "rusty.fleet.windows_release_policy_test.v2"
    result = "pass"
    labs_publication_enabled = $true
    stable_publication_enabled = $false
    labs_public_trust_claim = $false
    labs_trust_mode = $labsPolicy.authenticode.trust_mode
    stable_trust_mode = $stablePolicy.authenticode.trust_mode
    workflow_contents_permission = "read"
    checkout_credentials_persisted = $false
    actions_binary_artifacts = 0
    isolated_publication_authority = $true
    token_free_preflight = $true
    renewable_pages_metadata = $true
    renewable_pages_channels = @("labs", "stable")
    pages_binary_count = 0
    metadata_renewal_secret_count = 2
    workflow_newline_forms = @("lf", "crlf")
} | ConvertTo-Json -Depth 5
