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
    "New-RustyFleetDeterministicZip"
)
