# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$")]
    [string] $Version,

    [Parameter(Mandatory)]
    [string] $HostessProviderPath,

    [Parameter(Mandatory)]
    [ValidatePattern("^[0-9a-f]{64}$")]
    [string] $HostessProviderSha256,

    [Parameter(Mandatory)]
    [string] $HostessProviderMetadataDirectory,

    [ValidateSet("dev", "alpha", "preview", "stable")]
    [string] $Channel = "dev",

    [ValidateSet("unsigned-dev", "signed-release")]
    [string] $BuildKind = "unsigned-dev",

    [string] $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path,
    [string] $OutputDirectory,
    [string] $SourceRevision,
    [string] $SourceTree,
    [long] $SourceDateEpoch = 0,
    [string] $SourceRepository = "https://github.com/MesmerPrism/rusty-fleet",
    [string] $ReleaseBaseUrl = "https://github.com/MesmerPrism/rusty-fleet/releases",
    [string] $PagesUrl = "https://mesmerprism.com/Rusty-Fleet/",

    [switch] $SkipBuild,
    [string] $ConsoleArtifactDirectory,
    [string] $HubArtifactPath,
    [string] $FleetctlArtifactPath,
    [string] $FleetOnboardArtifactPath,
    [string] $ReleasePolicyPath = (
        Join-Path $PSScriptRoot "trust\release-policy.json"
    ),
    [switch] $RequireCleanSource,
    [switch] $RequireAuthenticodeSignatures,
    [string] $ExpectedFleetSignerThumbprint,
    [string] $ExpectedHostessSignerThumbprint
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
Import-Module (Join-Path $PSScriptRoot "Distribution.Common.psm1") -Force

foreach ($pair in @(
    @($SourceRepository, "SourceRepository"),
    @($ReleaseBaseUrl, "ReleaseBaseUrl"),
    @($PagesUrl, "PagesUrl")
)) {
    Assert-RustyFleetHttpsUrl -Value $pair[0] -Name $pair[1]
}
Assert-RustyFleetSha256 `
    -Value $HostessProviderSha256 `
    -Name "HostessProviderSha256"

$repoPath = (Resolve-Path -LiteralPath $RepoRoot).Path
if ($BuildKind -eq "signed-release") {
    $policyPath = (Resolve-Path -LiteralPath $ReleasePolicyPath).Path
    $policy = Get-Content -LiteralPath $policyPath -Raw |
        ConvertFrom-Json -Depth 10
    if ($policy.schema -cne "rusty.fleet.windows_release_trust_policy.v1" -or
        $policy.publication_enabled -ne $true -or
        @($policy.authorized_fleet_signer_thumbprints) -cnotcontains
            $ExpectedFleetSignerThumbprint.ToUpperInvariant() -or
        @($policy.authorized_hostess_signer_thumbprints) -cnotcontains
            $ExpectedHostessSignerThumbprint.ToUpperInvariant()) {
        throw "signed release inputs are not authorized by the revisioned Fleet release policy"
    }
}
$providerPath = (Resolve-Path -LiteralPath $HostessProviderPath).Path
if ((Split-Path -Leaf $providerPath) -cne "rusty-hostess-hotspot-provider.exe") {
    throw "the external provider filename must be exactly rusty-hostess-hotspot-provider.exe"
}
if ((Get-RustyFleetSha256 -LiteralPath $providerPath) -cne $HostessProviderSha256) {
    throw "the external Hostess provider does not match its supplied SHA-256"
}
$hostessProvenance = Read-RustyFleetHostessProvenance `
    -MetadataDirectory $HostessProviderMetadataDirectory `
    -ProviderPath $providerPath `
    -ProviderSha256 $HostessProviderSha256 `
    -BuildKind $BuildKind `
    -ExpectedSignerThumbprint $ExpectedHostessSignerThumbprint

if (-not $SourceRevision) {
    $SourceRevision = (& git -C $repoPath rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "could not resolve the Fleet source revision"
    }
}
if ($SourceRevision -cnotmatch "^[0-9a-f]{40}$") {
    throw "SourceRevision must be a full lowercase Git commit"
}
if (-not $SourceTree) {
    $SourceTree = (& git -C $repoPath rev-parse "$SourceRevision^{tree}" 2>$null).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "could not resolve the exact Fleet source tree"
    }
}
if ($SourceTree -cnotmatch "^[0-9a-f]{40}$") {
    throw "SourceTree must be a full lowercase Git tree"
}
if (-not $SkipBuild -or $RequireCleanSource -or $BuildKind -eq "signed-release") {
    $observedHead = (& git -C $repoPath rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $observedHead -cne $SourceRevision) {
        throw "the build must bind the current exact Fleet revision"
    }
    $sourceDirt = @(& git -C $repoPath status --porcelain=v1 --untracked-files=all)
    if ($LASTEXITCODE -ne 0 -or $sourceDirt.Count -ne 0) {
        throw "the build requires a clean Fleet source tree"
    }
}
if ($SourceDateEpoch -eq 0) {
    $SourceDateEpoch = [long] ((& git -C $repoPath show -s --format=%ct $SourceRevision).Trim())
    if ($LASTEXITCODE -ne 0) {
        throw "could not resolve SOURCE_DATE_EPOCH from the Fleet source revision"
    }
}

if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $repoPath "artifacts\windows-distribution"
}
$outputPath = [System.IO.Path]::GetFullPath($OutputDirectory)
[System.IO.Directory]::CreateDirectory($outputPath) | Out-Null

$buildRoot = Join-Path $outputPath ".build-$Version"
if (Test-Path -LiteralPath $buildRoot) {
    Remove-Item -LiteralPath $buildRoot -Recurse -Force
}
[System.IO.Directory]::CreateDirectory($buildRoot) | Out-Null

try {
    if (-not $SkipBuild) {
        $consolePublish = Join-Path $buildRoot "console"
        & dotnet publish `
            (Join-Path $repoPath "apps\fleet-console-wpf\RustyFleet.FleetConsole.csproj") `
            -c Release `
            -r win-x64 `
            --self-contained true `
            -o $consolePublish `
            /p:PublishSingleFile=true `
            /p:IncludeNativeLibrariesForSelfExtract=true `
            /p:DebugType=None `
            /p:DebugSymbols=false `
            /p:PublishTrimmed=false `
            /p:PublishReadyToRun=false `
            /p:Deterministic=true
        if ($LASTEXITCODE -ne 0) {
            throw "Fleet Console publish failed"
        }
        & cargo build `
            --manifest-path (Join-Path $repoPath "Cargo.toml") `
            --release `
            --locked `
            --bin fleet-hub-local `
            --bin fleetctl `
            --bin fleet-onboard
        if ($LASTEXITCODE -ne 0) {
            throw "Fleet Hub, fleetctl, or fleet-onboard release build failed"
        }
        $ConsoleArtifactDirectory = $consolePublish
        $HubArtifactPath = Join-Path $repoPath "target\release\fleet-hub-local.exe"
        $FleetctlArtifactPath = Join-Path $repoPath "target\release\fleetctl.exe"
        $FleetOnboardArtifactPath = Join-Path $repoPath "target\release\fleet-onboard.exe"
    }
    elseif (-not $ConsoleArtifactDirectory -or
        -not $HubArtifactPath -or
        -not $FleetctlArtifactPath -or
        -not $FleetOnboardArtifactPath) {
        throw "SkipBuild requires ConsoleArtifactDirectory, HubArtifactPath, FleetctlArtifactPath, and FleetOnboardArtifactPath"
    }

    $consolePath = (Resolve-Path -LiteralPath $ConsoleArtifactDirectory).Path
    $hubPath = (Resolve-Path -LiteralPath $HubArtifactPath).Path
    $fleetctlPath = (Resolve-Path -LiteralPath $FleetctlArtifactPath).Path
    $fleetOnboardPath = (Resolve-Path -LiteralPath $FleetOnboardArtifactPath).Path
    if (-not (Test-Path -LiteralPath (Join-Path $consolePath "RustyFleet.FleetConsole.exe") -PathType Leaf)) {
        throw "Fleet Console artifact directory has no RustyFleet.FleetConsole.exe"
    }
    if ((Split-Path -Leaf $hubPath) -cne "fleet-hub-local.exe" -or
        (Split-Path -Leaf $fleetctlPath) -cne "fleetctl.exe" -or
        (Split-Path -Leaf $fleetOnboardPath) -cne "fleet-onboard.exe") {
        throw "Fleet Hub, fleetctl, and fleet-onboard artifact names are not exact"
    }

    if ($RequireAuthenticodeSignatures) {
        if ($ExpectedFleetSignerThumbprint -cnotmatch "^[0-9A-Fa-f]{40}$") {
            throw "signed release requires an independently supplied Fleet signer thumbprint"
        }
        $fleetSigner = $ExpectedFleetSignerThumbprint.ToUpperInvariant()
        foreach ($executable in @(
            (Join-Path $consolePath "RustyFleet.FleetConsole.exe"),
            $hubPath,
            $fleetctlPath,
            $fleetOnboardPath
        )) {
            $signature = Get-AuthenticodeSignature -LiteralPath $executable
            $observedThumbprint = if ($signature.SignerCertificate) {
                $signature.SignerCertificate.Thumbprint
            }
            else {
                $null
            }
            if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
                $observedThumbprint -cne $fleetSigner) {
                throw "required Fleet Authenticode signer is not authorized: $(Split-Path -Leaf $executable)"
            }
        }
        $providerSignature = Get-AuthenticodeSignature -LiteralPath $providerPath
        if ($providerSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
            throw "required Hostess Authenticode signature is not valid"
        }
        if ($BuildKind -ne "signed-release") {
            throw "signature enforcement requires BuildKind signed-release"
        }
    }
    elseif ($BuildKind -eq "signed-release") {
        throw "signed-release requires RequireAuthenticodeSignatures"
    }

    $bundleName = if ($Channel -eq "alpha") {
        "RustyFleet-Alpha-$Version-win-x64"
    }
    else {
        "RustyFleet-$Version-win-x64"
    }
    $bundleRoot = Join-Path $outputPath $bundleName
    if (Test-Path -LiteralPath $bundleRoot) {
        Remove-Item -LiteralPath $bundleRoot -Recurse -Force
    }
    foreach ($relativeDirectory in @(
        "components\fleet-console",
        "components\fleet-hub",
        "components\fleetctl",
        "components\fleet-onboard",
        "providers\hostess-hotspot-provider",
        "providers\hostess-hotspot-provider\provenance",
        "distribution-tools",
        "metadata"
    )) {
        [System.IO.Directory]::CreateDirectory(
            (Join-Path $bundleRoot $relativeDirectory)
        ) | Out-Null
    }

    Get-ChildItem -LiteralPath $consolePath -Force |
        Copy-Item `
            -Destination (Join-Path $bundleRoot "components\fleet-console") `
            -Recurse
    Copy-Item -LiteralPath $hubPath `
        -Destination (Join-Path $bundleRoot "components\fleet-hub\fleet-hub-local.exe")
    Copy-Item -LiteralPath $fleetctlPath `
        -Destination (Join-Path $bundleRoot "components\fleetctl\fleetctl.exe")
    Copy-Item -LiteralPath $fleetOnboardPath `
        -Destination (Join-Path $bundleRoot "components\fleet-onboard\fleet-onboard.exe")
    Copy-Item -LiteralPath $providerPath `
        -Destination (Join-Path $bundleRoot "providers\hostess-hotspot-provider\rusty-hostess-hotspot-provider.exe")
    foreach ($documentName in $hostessProvenance.documents.Values) {
        Copy-Item `
            -LiteralPath (Join-Path $hostessProvenance.root $documentName) `
            -Destination (
                Join-Path $bundleRoot "providers\hostess-hotspot-provider\provenance\$documentName"
            )
    }
    foreach ($tool in @(
        "Distribution.Common.psm1",
        "Test-WindowsBundle.ps1"
    )) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $tool) `
            -Destination (Join-Path $bundleRoot "distribution-tools\$tool")
    }

    $componentRoots = [ordered]@{
        "fleet-console" = "components/fleet-console/"
        "fleet-hub" = "components/fleet-hub/"
        "fleetctl" = "components/fleetctl/"
        "fleet-onboard" = "components/fleet-onboard/"
        "hostess-hotspot-provider" = "providers/hostess-hotspot-provider/"
        "distribution-tools" = "distribution-tools/"
    }
    $payload = @(
        Get-ChildItem -LiteralPath $bundleRoot -File -Recurse |
            Where-Object { $_.FullName -notlike "$(Join-Path $bundleRoot 'metadata')*" } |
            ForEach-Object {
                $relative = Get-RustyFleetRelativePath `
                    -Root $bundleRoot `
                    -LiteralPath $_.FullName
                Assert-RustyFleetPayloadPath -RelativePath $relative
                $componentId = @($componentRoots.Keys |
                    Where-Object { $relative.StartsWith($componentRoots[$_], [StringComparison]::Ordinal) })
                if ($componentId.Count -ne 1) {
                    throw "payload does not map to exactly one component: $relative"
                }
                [ordered]@{
                    path = $relative
                    component_id = $componentId[0]
                    sha256 = Get-RustyFleetSha256 -LiteralPath $_.FullName
                    size_bytes = $_.Length
                }
            } |
            Sort-Object path
    )

    $providerEntrypoint = "providers/hostess-hotspot-provider/rusty-hostess-hotspot-provider.exe"
    $components = @(
        [ordered]@{
            component_id = "fleet-console"
            owner = "rusty-fleet"
            kind = "windows_gui"
            entrypoint = "components/fleet-console/RustyFleet.FleetConsole.exe"
        },
        [ordered]@{
            component_id = "fleet-hub"
            owner = "rusty-fleet"
            kind = "local_hub"
            entrypoint = "components/fleet-hub/fleet-hub-local.exe"
        },
        [ordered]@{
            component_id = "fleetctl"
            owner = "rusty-fleet"
            kind = "command_line"
            entrypoint = "components/fleetctl/fleetctl.exe"
        },
        [ordered]@{
            component_id = "fleet-onboard"
            owner = "rusty-fleet"
            kind = "offline_onboarding_generator"
            entrypoint = "components/fleet-onboard/fleet-onboard.exe"
            activation = "explicit_operator_invocation"
            network_access = "absent"
            output = "private_machine_bound_onboarding_only"
            operational_readiness = "requires_separately_configured_signed_owner_key_record_tool"
            bundled_owner_key_record_tool = $false
        },
        [ordered]@{
            component_id = "hostess-hotspot-provider"
            owner = "rusty-hostess"
            kind = "external_provider"
            entrypoint = $providerEntrypoint
            contract = [ordered]@{
                action_id = "host.windows-mobile-hotspot"
                arguments = @("integration", "windows-hotspot", "--json")
                stdin_schema = "rusty.hostess.windows_hotspot.provider_request.v1"
                stdout_schema = "rusty.hostess.windows_hotspot.provider_receipt.v1"
                process_results = [ordered]@{
                    verified = 0
                    failed = 1
                    rejected = 2
                    unavailable = 3
                }
                prohibited_public_fields = @(
                    "ssid",
                    "passphrase",
                    "profile",
                    "path",
                    "ip",
                    "private_endpoint"
                )
            }
            provenance = [ordered]@{
                supplied_externally = $true
                artifact_name = "rusty-hostess-hotspot-provider.exe"
                artifact_sha256 = $HostessProviderSha256
                product_version = $hostessProvenance.provenance.artifact.product_version
                provider_version = $hostessProvenance.provenance.provider_version
                owner_document_schema = $hostessProvenance.provenance.schema
                owner_document_path = "providers/hostess-hotspot-provider/provenance/rusty-hostess-hotspot-provider.provenance.json"
                owner_document_sha256 = $hostessProvenance.provenance_sha256
                license_path = "providers/hostess-hotspot-provider/provenance/LICENSE"
                third_party_notices_path = "providers/hostess-hotspot-provider/provenance/THIRD-PARTY-NOTICES.txt"
                source_repository = $hostessProvenance.provenance.source.repository
                source_revision = $hostessProvenance.provenance.source.revision
                source_tree = $hostessProvenance.provenance.source.tree
                source_availability_url = $hostessProvenance.provenance.source.availability_url
                source_availability_state = $hostessProvenance.provenance.source.availability_state
                source_verified_at_utc = $hostessProvenance.provenance.source.verified_at_utc
                unsigned_artifact_sha256 = $hostessProvenance.provenance.build.unsigned_artifact_sha256
                unsigned_artifact_size_bytes = [long] $hostessProvenance.provenance.build.unsigned_artifact_size_bytes
                canonical_payload_sha256 = $hostessProvenance.provenance.build.canonical_payload_sha256
                canonical_payload_size_bytes = [long] $hostessProvenance.provenance.build.canonical_payload_size_bytes
                dependency_count = @($hostessProvenance.provenance.dependencies).Count
                bundled_native_library_count = @(
                    $hostessProvenance.provenance.bundled_native_libraries
                ).Count
                signing_state = $hostessProvenance.provenance.signing.state
                authorized_signer_thumbprint = $hostessProvenance.provenance.signing.authorized_thumbprint
                distribution_eligibility = $hostessProvenance.provenance.distribution.eligibility
            }
        }
    )

    $archiveAsset = "$bundleName.zip"
    $manifest = [ordered]@{
        schema = "rusty.fleet.windows_release_manifest.v1"
        product_id = "rusty-fleet"
        version = $Version
        channel = $Channel
        platform = "windows"
        architecture = "x64"
        source = [ordered]@{
            repository = $SourceRepository
            revision = $SourceRevision
            tree = $SourceTree
        }
        build = [ordered]@{
            kind = $BuildKind
            reproducible_archive_for_identical_input_bytes = $true
            source_to_artifact_binding = "clean_worktree_prebuild_assertion"
            source_date_epoch = $SourceDateEpoch
            authenticode_required = [bool] $RequireAuthenticodeSignatures
            authorized_fleet_signer_thumbprint = if ($RequireAuthenticodeSignatures) {
                $ExpectedFleetSignerThumbprint.ToUpperInvariant()
            }
            else {
                $null
            }
            source_tree_clean = [bool] (
                -not $SkipBuild -or $RequireCleanSource -or
                $BuildKind -eq "signed-release"
            )
        }
        distribution = [ordered]@{
            binary_authority = "github_releases"
            release_base_url = $ReleaseBaseUrl
            archive_asset = $archiveAsset
            pages_url = $PagesUrl
            pages_role = "human_documentation_and_signed_metadata_only"
            eligibility = if ($BuildKind -eq "signed-release") {
                "signed_release"
            }
            else {
                "development_only"
            }
            publication_allowed = $BuildKind -eq "signed-release"
            onboarding_ready = $false
            onboarding_blocker = "signed_rusty_quest_owner_key_record_release_not_bundled"
        }
        components = $components
        payload = $payload
        install = [ordered]@{
            mode = "per_user_side_by_side"
            default_root = if ($Channel -eq "alpha") {
                "%LOCALAPPDATA%/RustyFleetAlpha"
            }
            else {
                "%LOCALAPPDATA%/RustyFleet"
            }
            activation = "explicit_operator_start"
            authority = if ($Channel -eq "alpha") {
                "RustyFleet-Alpha-Setup.exe"
            }
            else {
                "RustyFleet-Setup.exe"
            }
            plan_protocol = "rusty.fleet.guided_installer_plan.v1"
            service_registration = "absent"
            configuration = "external_after_install"
        }
        update = [ordered]@{
            strategy = "setup_owned_side_by_side_manifest"
            channel = $Channel
            manifest_asset = "$bundleName.manifest.json"
            checksums_asset = "$bundleName.checksums.sha256"
            validation_receipt_asset = "$bundleName.validation-receipt.json"
            archive_asset = $archiveAsset
            rollback = [ordered]@{
                supported = $true
                mode = "setup_owned_pointer_to_previous_fully_verified_release"
                preserves_releases = $true
                automatic_delete = $false
            }
        }
        excluded_payload_classes = @(
            "credentials",
            "private_configuration",
            "adb",
            "pairing_material",
            "device_serials",
            "generated_runtime_state"
        )
    }

    $manifestPath = Join-Path $bundleRoot "metadata\release-manifest.json"
    Write-RustyFleetUtf8 `
        -LiteralPath $manifestPath `
        -Content (ConvertTo-RustyFleetJson -InputObject $manifest)

    $checksumRecords = @($payload | ForEach-Object {
        [ordered]@{ path = $_.path; sha256 = $_.sha256 }
    })
    $checksumRecords += [ordered]@{
        path = "metadata/release-manifest.json"
        sha256 = Get-RustyFleetSha256 -LiteralPath $manifestPath
    }
    $checksumText = (($checksumRecords |
        Sort-Object path |
        ForEach-Object { "$($_.sha256) *$($_.path)" }) -join "`n") + "`n"
    $checksumsPath = Join-Path $bundleRoot "metadata\checksums.sha256"
    Write-RustyFleetUtf8 -LiteralPath $checksumsPath -Content $checksumText

    $receipt = [ordered]@{
        schema = "rusty.fleet.windows_distribution_validation_receipt.v1"
        result = "pass"
        version = $Version
        source_revision = $SourceRevision
        manifest_sha256 = Get-RustyFleetSha256 -LiteralPath $manifestPath
        checksums_sha256 = Get-RustyFleetSha256 -LiteralPath $checksumsPath
        payload_files = $payload.Count
        runtime_components = 5
        exact_external_provider = $true
        hostess_owner_provenance_sha256 = $hostessProvenance.provenance_sha256
        distribution_eligibility = $manifest.distribution.eligibility
        publication_allowed = $manifest.distribution.publication_allowed
        credentials_absent = $true
        private_configuration_absent = $true
        adb_absent = $true
        signatures = if ($RequireAuthenticodeSignatures) { "verified" } else { "not_required_unsigned_dev" }
    }
    $receiptPath = Join-Path $bundleRoot "metadata\validation-receipt.json"
    Write-RustyFleetUtf8 `
        -LiteralPath $receiptPath `
        -Content (ConvertTo-RustyFleetJson -InputObject $receipt)

    & (Join-Path $PSScriptRoot "Test-WindowsBundle.ps1") `
        -BundleRoot $bundleRoot `
        -ExpectedVersion $Version `
        -ExpectedFleetSignerThumbprint $ExpectedFleetSignerThumbprint `
        -ExpectedHostessSignerThumbprint $ExpectedHostessSignerThumbprint |
        Out-Null

    $archivePath = Join-Path $outputPath $archiveAsset
    New-RustyFleetDeterministicZip `
        -SourceDirectory $bundleRoot `
        -DestinationPath $archivePath `
        -SourceDateEpoch $SourceDateEpoch
    $archiveSha256 = Get-RustyFleetSha256 -LiteralPath $archivePath
    Write-RustyFleetUtf8 `
        -LiteralPath "$archivePath.sha256" `
        -Content "$archiveSha256 *$archiveAsset`n"

    foreach ($asset in @(
        @($manifestPath, "$bundleName.manifest.json"),
        @($checksumsPath, "$bundleName.checksums.sha256"),
        @($receiptPath, "$bundleName.validation-receipt.json")
    )) {
        Copy-Item -LiteralPath $asset[0] -Destination (Join-Path $outputPath $asset[1]) -Force
    }

    [ordered]@{
        schema = "rusty.fleet.windows_bundle_build_result.v1"
        result = "pass"
        version = $Version
        build_kind = $BuildKind
        bundle_directory = $bundleRoot
        archive = $archivePath
        archive_sha256 = $archiveSha256
        manifest_sha256 = $receipt.manifest_sha256
        payload_files = $payload.Count
        provider_sha256 = $HostessProviderSha256
        hostess_owner_provenance_sha256 = $hostessProvenance.provenance_sha256
        distribution_eligibility = $manifest.distribution.eligibility
        publication_allowed = $manifest.distribution.publication_allowed
    } | ConvertTo-Json -Depth 10
}
finally {
    if (Test-Path -LiteralPath $buildRoot) {
        Remove-Item -LiteralPath $buildRoot -Recurse -Force
    }
}
