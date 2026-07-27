# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RustyFleetSha256 {
    param([Parameter(Mandatory)][string] $LiteralPath)

    (Get-FileHash -LiteralPath $LiteralPath -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Write-RustyFleetUtf8 {
    param(
        [Parameter(Mandatory)][string] $LiteralPath,
        [Parameter(Mandatory)][string] $Content
    )

    $parent = Split-Path -Parent $LiteralPath
    if ($parent) {
        [System.IO.Directory]::CreateDirectory($parent) | Out-Null
    }
    [System.IO.File]::WriteAllText(
        $LiteralPath,
        $Content,
        [System.Text.UTF8Encoding]::new($false)
    )
}

function ConvertTo-RustyFleetJson {
    param([Parameter(Mandatory)][object] $InputObject)

    ($InputObject | ConvertTo-Json -Depth 30) + "`n"
}

function Get-RustyFleetRelativePath {
    param(
        [Parameter(Mandatory)][string] $Root,
        [Parameter(Mandatory)][string] $LiteralPath
    )

    [System.IO.Path]::GetRelativePath(
        [System.IO.Path]::GetFullPath($Root),
        [System.IO.Path]::GetFullPath($LiteralPath)
    ).Replace("\", "/")
}

function Assert-RustyFleetHttpsUrl {
    param(
        [Parameter(Mandatory)][string] $Value,
        [Parameter(Mandatory)][string] $Name
    )

    $uri = $null
    if (-not [Uri]::TryCreate($Value, [UriKind]::Absolute, [ref] $uri) -or
        $uri.Scheme -ne "https") {
        throw "$Name must be an absolute HTTPS URL"
    }
}

function Assert-RustyFleetSha256 {
    param(
        [Parameter(Mandatory)][string] $Value,
        [Parameter(Mandatory)][string] $Name
    )

    if ($Value -cnotmatch "^[0-9a-f]{64}$") {
        throw "$Name must be exactly 64 lowercase hexadecimal characters"
    }
}

function Assert-RustyFleetPayloadPath {
    param([Parameter(Mandatory)][string] $RelativePath)

    if ($RelativePath -match "(^|/)\.\.?(/|$)" -or
        $RelativePath -match "(?i)(^|/)(adb|fastboot)(\.exe)?$" -or
        $RelativePath -match "(?i)(credential|password|passphrase|pairing|private[-_]?config|secret|token|keystore)") {
        throw "prohibited distribution payload path: $RelativePath"
    }
}

function Assert-RustyFleetExactProperties {
    param(
        [Parameter(Mandatory)][object] $InputObject,
        [Parameter(Mandatory)][string[]] $Expected,
        [Parameter(Mandatory)][string] $Context
    )

    $actual = @($InputObject.PSObject.Properties.Name | Sort-Object)
    if (@(Compare-Object ($Expected | Sort-Object) $actual).Count -ne 0) {
        throw "$Context has missing or unknown fields"
    }
}

function Read-RustyFleetHostessProvenance {
    param(
        [Parameter(Mandatory)][string] $MetadataDirectory,
        [Parameter(Mandatory)][string] $ProviderPath,
        [Parameter(Mandatory)][string] $ProviderSha256,
        [Parameter(Mandatory)][ValidateSet("unsigned-dev", "signed-release")]
        [string] $BuildKind
    )

    $metadataPath = (Resolve-Path -LiteralPath $MetadataDirectory).Path
    $providerFullPath = (Resolve-Path -LiteralPath $ProviderPath).Path
    $documents = [ordered]@{
        provenance = "rusty-hostess-hotspot-provider.provenance.json"
        license = "LICENSE"
        notices = "THIRD-PARTY-NOTICES.txt"
    }
    foreach ($name in $documents.Values) {
        if (-not (Test-Path -LiteralPath (Join-Path $metadataPath $name) -PathType Leaf)) {
            throw "Hostess owner metadata is missing required document: $name"
        }
    }
    $metadataFiles = @(
        Get-ChildItem -LiteralPath $metadataPath -File |
            Select-Object -ExpandProperty Name |
            Sort-Object
    )
    if (@(Compare-Object ($documents.Values | Sort-Object) $metadataFiles).Count -ne 0) {
        throw "Hostess owner metadata must contain exactly its three issued documents"
    }

    $provenancePath = Join-Path $metadataPath $documents.provenance
    $provenance = Get-Content -LiteralPath $provenancePath -Raw |
        ConvertFrom-Json -Depth 30
    Assert-RustyFleetExactProperties -InputObject $provenance -Expected @(
        "schema",
        "product_id",
        "provider_version",
        "artifact",
        "source",
        "build",
        "dependencies",
        "bundled_native_libraries",
        "signing",
        "companion_documents",
        "distribution"
    ) -Context "Hostess provenance"
    Assert-RustyFleetExactProperties -InputObject $provenance.artifact -Expected @(
        "name", "sha256", "size_bytes"
    ) -Context "Hostess artifact provenance"
    Assert-RustyFleetExactProperties -InputObject $provenance.source -Expected @(
        "repository", "revision", "tree", "availability_url", "tree_clean"
    ) -Context "Hostess source provenance"
    Assert-RustyFleetExactProperties -InputObject $provenance.build -Expected @(
        "kind", "framework", "runtime_identifier", "source_date_epoch"
    ) -Context "Hostess build provenance"
    Assert-RustyFleetExactProperties -InputObject $provenance.signing -Expected @(
        "state", "status", "subject", "thumbprint"
    ) -Context "Hostess signing provenance"
    Assert-RustyFleetExactProperties -InputObject $provenance.distribution -Expected @(
        "eligibility", "binary_authority"
    ) -Context "Hostess distribution provenance"
    if ($provenance.schema -ne "rusty.hostess.windows_hotspot.release_provenance.v1" -or
        $provenance.product_id -ne "rusty-hostess-windows-hotspot-provider" -or
        $provenance.provider_version -cnotmatch
            "^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$") {
        throw "Hostess provenance schema is not supported"
    }

    $providerItem = Get-Item -LiteralPath $providerFullPath
    if ($provenance.artifact.name -cne "rusty-hostess-hotspot-provider.exe" -or
        $provenance.artifact.sha256 -cne $ProviderSha256 -or
        $provenance.artifact.sha256 -cne (
            Get-RustyFleetSha256 -LiteralPath $providerFullPath
        ) -or
        [long] $provenance.artifact.size_bytes -ne $providerItem.Length) {
        throw "Hostess provenance does not bind the exact provider artifact"
    }
    Assert-RustyFleetSha256 `
        -Value $provenance.artifact.sha256 `
        -Name "Hostess provenance artifact digest"

    if ($provenance.source.repository -cne
            "https://github.com/MesmerPrism/rusty-hostess" -or
        $provenance.source.revision -cnotmatch "^[0-9a-f]{40}$" -or
        $provenance.source.tree -cnotmatch "^[0-9a-f]{40}$" -or
        $provenance.source.tree_clean -ne $true) {
        throw "Hostess provenance does not bind a clean full source commit and tree"
    }
    $availabilityUri = [Uri] $provenance.source.availability_url
    if ($availabilityUri.Scheme -cne "https" -or
        $availabilityUri.Host -cne "github.com" -or
        $availabilityUri.AbsolutePath.TrimEnd("/") -cne
            "/MesmerPrism/rusty-hostess/tree/$($provenance.source.revision)") {
        throw "Hostess source availability does not bind the exact revision"
    }
    foreach ($buildField in @(
        $provenance.build.kind,
        $provenance.build.framework,
        $provenance.build.runtime_identifier
    )) {
        if ([string]::IsNullOrWhiteSpace([string] $buildField)) {
            throw "Hostess provenance build metadata is incomplete"
        }
    }
    if ($provenance.build.kind -cne $BuildKind -or
        [long] $provenance.build.source_date_epoch -le 0) {
        throw "Hostess provenance source-date epoch is invalid"
    }

    $dependencies = @($provenance.dependencies)
    if ($dependencies.Count -eq 0) {
        throw "Hostess provenance must carry its owner-generated dependency report"
    }
    foreach ($dependency in $dependencies) {
        Assert-RustyFleetExactProperties -InputObject $dependency -Expected @(
            "name", "version", "license", "license_url", "project_url"
        ) -Context "Hostess dependency provenance"
        foreach ($field in @(
            $dependency.name,
            $dependency.version,
            $dependency.license
        )) {
            if ([string]::IsNullOrWhiteSpace([string] $field)) {
                throw "Hostess dependency provenance is incomplete"
            }
        }
    }

    $nativeLibraries = @($provenance.bundled_native_libraries)
    if ($nativeLibraries.Count -eq 0) {
        throw "Hostess provenance must carry its bundled native library inventory"
    }
    foreach ($library in $nativeLibraries) {
        Assert-RustyFleetExactProperties -InputObject $library -Expected @(
            "name", "sha256", "size_bytes"
        ) -Context "Hostess bundled native library provenance"
        if ($library.name -cnotmatch "^[A-Za-z0-9_.-]+\.dll$" -or
            [long] $library.size_bytes -le 0) {
            throw "Hostess bundled-native-library inventory is incomplete"
        }
        Assert-RustyFleetSha256 `
            -Value $library.sha256 `
            -Name "Hostess bundled native library digest"
    }
    if (@($nativeLibraries.name | Sort-Object -Unique).Count -ne
        $nativeLibraries.Count) {
        throw "Hostess bundled native library names are not unique"
    }

    $companions = @($provenance.companion_documents)
    $expectedCompanions = @("LICENSE", "THIRD-PARTY-NOTICES.txt")
    $companionNames = @($companions | ForEach-Object { $_.name })
    if (@(Compare-Object $expectedCompanions $companionNames).Count -ne 0 -or
        $companions.Count -ne 2) {
        throw "Hostess provenance must bind exactly LICENSE and THIRD-PARTY-NOTICES.txt"
    }
    foreach ($companion in $companions) {
        Assert-RustyFleetExactProperties -InputObject $companion -Expected @(
            "name", "sha256", "size_bytes"
        ) -Context "Hostess companion document provenance"
        Assert-RustyFleetSha256 `
            -Value $companion.sha256 `
            -Name "Hostess companion document digest"
        $documentPath = Join-Path $metadataPath $companion.name
        if ((Get-RustyFleetSha256 -LiteralPath $documentPath) -cne $companion.sha256 -or
            (Get-Item -LiteralPath $documentPath).Length -ne [long] $companion.size_bytes) {
            throw "Hostess companion document does not match owner provenance: $($companion.name)"
        }
    }

    if ($BuildKind -eq "signed-release") {
        if ($provenance.distribution.eligibility -ne "signed_release" -or
            $provenance.signing.state -ne "verified" -or
            $provenance.signing.status -ine "valid" -or
            [string]::IsNullOrWhiteSpace([string] $provenance.signing.subject) -or
            [string]::IsNullOrWhiteSpace([string] $provenance.signing.thumbprint)) {
            throw "Hostess provenance does not authorize signed release distribution"
        }
    }
    elseif ($provenance.distribution.eligibility -ne "development_only") {
        throw "unsigned Hostess provenance must be development_only"
    }
    if ($provenance.distribution.binary_authority -ne
        "rusty-hostess-github-releases") {
        throw "Hostess binary authority is not exact"
    }
    $provenanceText = Get-Content -LiteralPath $provenancePath -Raw
    if ($provenanceText -match "(?i)[a-z]:\\\\|\\\\\\\\") {
        throw "Hostess provenance contains a machine-private path"
    }

    [ordered]@{
        root = $metadataPath
        documents = $documents
        provenance = $provenance
        provenance_sha256 = Get-RustyFleetSha256 -LiteralPath $provenancePath
    }
}

function New-RustyFleetDeterministicZip {
    param(
        [Parameter(Mandatory)][string] $SourceDirectory,
        [Parameter(Mandatory)][string] $DestinationPath,
        [Parameter(Mandatory)][long] $SourceDateEpoch
    )

    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem

    $destinationParent = Split-Path -Parent $DestinationPath
    [System.IO.Directory]::CreateDirectory($destinationParent) | Out-Null
    if (Test-Path -LiteralPath $DestinationPath) {
        Remove-Item -LiteralPath $DestinationPath -Force
    }

    $minimumZipEpoch = [DateTimeOffset]::new(
        1980, 1, 1, 0, 0, 0, [TimeSpan]::Zero
    ).ToUnixTimeSeconds()
    $effectiveEpoch = [Math]::Max($minimumZipEpoch, $SourceDateEpoch)
    $entryTimestamp = [DateTimeOffset]::FromUnixTimeSeconds($effectiveEpoch)
    $sourceName = Split-Path -Leaf $SourceDirectory

    $archive = [System.IO.Compression.ZipFile]::Open(
        $DestinationPath,
        [System.IO.Compression.ZipArchiveMode]::Create
    )
    try {
        $files = Get-ChildItem -LiteralPath $SourceDirectory -File -Recurse |
            Sort-Object {
                Get-RustyFleetRelativePath -Root $SourceDirectory -LiteralPath $_.FullName
            }
        foreach ($file in $files) {
            $relative = Get-RustyFleetRelativePath `
                -Root $SourceDirectory `
                -LiteralPath $file.FullName
            $entry = $archive.CreateEntry(
                "$sourceName/$relative",
                [System.IO.Compression.CompressionLevel]::Optimal
            )
            $entry.LastWriteTime = $entryTimestamp
            $input = [System.IO.File]::OpenRead($file.FullName)
            $output = $entry.Open()
            try {
                $input.CopyTo($output)
            }
            finally {
                $output.Dispose()
                $input.Dispose()
            }
        }
    }
    finally {
        $archive.Dispose()
    }
}

Export-ModuleMember -Function @(
    "Get-RustyFleetSha256",
    "Write-RustyFleetUtf8",
    "ConvertTo-RustyFleetJson",
    "Get-RustyFleetRelativePath",
    "Assert-RustyFleetHttpsUrl",
    "Assert-RustyFleetSha256",
    "Assert-RustyFleetPayloadPath",
    "Read-RustyFleetHostessProvenance",
    "New-RustyFleetDeterministicZip"
)
