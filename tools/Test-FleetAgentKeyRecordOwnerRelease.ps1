# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param(
    [string] $CapsuleRoot = "",
    [string] $PinPath = (Join-Path $PSScriptRoot "..\config\fleet-agent-key-record-owner-release.v1.json"),
    [switch] $PolicySelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-Sha256([string] $LiteralPath) {
    return (Get-FileHash -LiteralPath $LiteralPath -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-ExactProperties($Value, [string[]] $Names, [string] $Label) {
    $actual = @($Value.PSObject.Properties.Name | Sort-Object)
    $expected = @($Names | Sort-Object)
    if (@(Compare-Object $actual $expected -SyncWindow 0).Count -ne 0 -or
        $actual.Count -ne $expected.Count) {
        throw "$Label contains an unknown or missing field"
    }
}

function Test-MachineLocalPathText([string] $Text) {
    $serverComponent = '[A-Za-z0-9][A-Za-z0-9._$-]*'
    $shareComponent = '[\x20-\x7e-[\\/:<>:"|?*]]+'
    $patterns = @(
        '(?i)[A-Z]:[\\/](?![\\/])',
        '(?i)\\\\\?\\[A-Z]:[\\/](?![\\/])',
        ('(?i)\\\\(?!\?\\UNC(?:\\|$))' + $serverComponent +
            '[\\/]' + $shareComponent + '(?=[\\/\x00]|$)'),
        ('(?i)\\\\\?\\UNC\\' + $serverComponent +
            '\\' + $shareComponent + '(?=[\\\x00]|$)')
    )
    return @($patterns | Where-Object { [regex]::IsMatch($Text, $_) }).Count -gt 0
}

function Assert-NoMachineLocalPathByteSequence([string] $LiteralPath) {
    $bytes = [IO.File]::ReadAllBytes($LiteralPath)
    if (Test-MachineLocalPathText ([Text.Encoding]::Latin1.GetString($bytes))) {
        throw "owner capsule executable contains a machine-local ASCII path"
    }
    foreach ($encoding in @([Text.Encoding]::Unicode, [Text.Encoding]::BigEndianUnicode)) {
        foreach ($offset in @(0, 1)) {
            $count = $bytes.Length - $offset
            if ($count -lt 4) { continue }
            if (($count % 2) -ne 0) { $count-- }
            if (Test-MachineLocalPathText ($encoding.GetString($bytes, $offset, $count))) {
                throw "owner capsule executable contains a machine-local UTF-16 path"
            }
        }
    }
}

function Assert-X64WindowsExecutable([string] $LiteralPath) {
    $bytes = [IO.File]::ReadAllBytes($LiteralPath)
    if ($bytes.Length -lt 64 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) {
        throw "owner capsule executable is not a Windows PE"
    }
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
    if ($peOffset -lt 0 -or $peOffset + 6 -gt $bytes.Length -or
        $bytes[$peOffset] -ne 0x50 -or $bytes[$peOffset + 1] -ne 0x45 -or
        $bytes[$peOffset + 2] -ne 0 -or $bytes[$peOffset + 3] -ne 0 -or
        [BitConverter]::ToUInt16($bytes, $peOffset + 4) -ne 0x8664) {
        throw "owner capsule executable is not x64 Windows PE"
    }
}

function Get-PeDebugDirectoryTypeOffsetList([byte[]] $Bytes) {
    $peOffset = [BitConverter]::ToInt32($Bytes, 0x3c)
    $sectionCount = [BitConverter]::ToUInt16($Bytes, $peOffset + 6)
    $optionalHeaderSize = [BitConverter]::ToUInt16($Bytes, $peOffset + 20)
    $optionalHeaderOffset = $peOffset + 24
    if ($optionalHeaderSize -lt 168 -or
        $optionalHeaderOffset + $optionalHeaderSize -gt $Bytes.Length -or
        [BitConverter]::ToUInt16($Bytes, $optionalHeaderOffset) -ne 0x20b -or
        [BitConverter]::ToUInt32($Bytes, $optionalHeaderOffset + 108) -le 6) {
        throw "owner capsule PE optional header is invalid"
    }
    $debugDirectoryRva = [BitConverter]::ToUInt32($Bytes, $optionalHeaderOffset + 160)
    $debugDirectorySize = [BitConverter]::ToUInt32($Bytes, $optionalHeaderOffset + 164)
    if ($debugDirectoryRva -eq 0 -or $debugDirectorySize -lt 28 -or
        ($debugDirectorySize % 28) -ne 0 -or $debugDirectorySize -gt 1792) {
        throw "owner capsule PE debug directory is invalid"
    }
    $sectionTableOffset = $optionalHeaderOffset + $optionalHeaderSize
    if ($sectionCount -lt 1 -or $sectionCount -gt 96 -or
        $sectionTableOffset + (40 * $sectionCount) -gt $Bytes.Length) {
        throw "owner capsule PE section table is invalid"
    }
    $debugDirectoryOffset = $null
    for ($index = 0; $index -lt $sectionCount; $index++) {
        $sectionOffset = $sectionTableOffset + (40 * $index)
        $virtualSize = [uint64][BitConverter]::ToUInt32($Bytes, $sectionOffset + 8)
        $virtualAddress = [uint64][BitConverter]::ToUInt32($Bytes, $sectionOffset + 12)
        $rawSize = [uint64][BitConverter]::ToUInt32($Bytes, $sectionOffset + 16)
        $rawOffset = [uint64][BitConverter]::ToUInt32($Bytes, $sectionOffset + 20)
        $sectionSpan = [Math]::Max($virtualSize, $rawSize)
        if ([uint64]$debugDirectoryRva -ge $virtualAddress -and
            [uint64]$debugDirectoryRva -lt $virtualAddress + $sectionSpan) {
            $relativeOffset = [uint64]$debugDirectoryRva - $virtualAddress
            if ($relativeOffset + [uint64]$debugDirectorySize -gt $rawSize -or
                $rawOffset + $relativeOffset + [uint64]$debugDirectorySize -gt
                    [uint64]$Bytes.Length) {
                throw "owner capsule PE debug directory escaped its section"
            }
            $debugDirectoryOffset = [int]($rawOffset + $relativeOffset)
            break
        }
    }
    if ($null -eq $debugDirectoryOffset) {
        throw "owner capsule PE debug directory is not file-backed"
    }
    return @(0..([int]($debugDirectorySize / 28) - 1) | ForEach-Object {
        $debugDirectoryOffset + ($_ * 28) + 12
    })
}

function Assert-PeReproducible([string] $LiteralPath) {
    $bytes = [IO.File]::ReadAllBytes($LiteralPath)
    $reproEntries = @(Get-PeDebugDirectoryTypeOffsetList -Bytes $bytes | Where-Object {
        [BitConverter]::ToUInt32($bytes, $_) -eq 16
    })
    if ($reproEntries.Count -ne 1) {
        throw "owner capsule executable lacks one IMAGE_DEBUG_TYPE_REPRO marker"
    }
}

function Get-PePolicyProbe([uint32] $DebugType) {
    $bytes = [byte[]]::new(1024)
    $bytes[0] = 0x4d; $bytes[1] = 0x5a
    [Array]::Copy([BitConverter]::GetBytes([uint32]0x80), 0, $bytes, 0x3c, 4)
    [Array]::Copy([byte[]](0x50, 0x45, 0, 0), 0, $bytes, 0x80, 4)
    [Array]::Copy([BitConverter]::GetBytes([uint16]0x8664), 0, $bytes, 0x84, 2)
    [Array]::Copy([BitConverter]::GetBytes([uint16]1), 0, $bytes, 0x86, 2)
    [Array]::Copy([BitConverter]::GetBytes([uint16]240), 0, $bytes, 0x94, 2)
    $optionalHeaderOffset = 0x98
    [Array]::Copy([BitConverter]::GetBytes([uint16]0x20b), 0, $bytes, $optionalHeaderOffset, 2)
    [Array]::Copy([BitConverter]::GetBytes([uint32]16), 0, $bytes, $optionalHeaderOffset + 108, 4)
    [Array]::Copy([BitConverter]::GetBytes([uint32]0x1000), 0, $bytes, $optionalHeaderOffset + 160, 4)
    [Array]::Copy([BitConverter]::GetBytes([uint32]28), 0, $bytes, $optionalHeaderOffset + 164, 4)
    $sectionOffset = $optionalHeaderOffset + 240
    [Array]::Copy([Text.Encoding]::ASCII.GetBytes(".rdata"), 0, $bytes, $sectionOffset, 6)
    [Array]::Copy([BitConverter]::GetBytes([uint32]0x200), 0, $bytes, $sectionOffset + 8, 4)
    [Array]::Copy([BitConverter]::GetBytes([uint32]0x1000), 0, $bytes, $sectionOffset + 12, 4)
    [Array]::Copy([BitConverter]::GetBytes([uint32]0x200), 0, $bytes, $sectionOffset + 16, 4)
    [Array]::Copy([BitConverter]::GetBytes([uint32]0x200), 0, $bytes, $sectionOffset + 20, 4)
    [Array]::Copy([BitConverter]::GetBytes($DebugType), 0, $bytes, 0x20c, 4)
    return $bytes
}

function Invoke-PolicySelfTest {
    $root = Join-Path ([IO.Path]::GetTempPath()) (
        "rusty-fleet-owner-release-policy-" + [guid]::NewGuid().ToString("N"))
    try {
        [void][IO.Directory]::CreateDirectory($root)
        $probe = Join-Path $root "probe.exe"
        [IO.File]::WriteAllBytes($probe, (Get-PePolicyProbe -DebugType 16))
        Assert-X64WindowsExecutable -LiteralPath $probe
        Assert-PeReproducible -LiteralPath $probe
        [IO.File]::WriteAllBytes($probe, (Get-PePolicyProbe -DebugType 2))
        $rejected = $false
        try { Assert-PeReproducible -LiteralPath $probe }
        catch { if ($_.Exception.Message -notmatch "IMAGE_DEBUG_TYPE_REPRO") { throw }; $rejected = $true }
        if (-not $rejected) { throw "PE reproducibility policy accepted missing marker" }
        [IO.File]::WriteAllBytes($probe, [Text.Encoding]::ASCII.GetBytes('Q:\synthetic\artifact.obj'))
        $rejected = $false
        try { Assert-NoMachineLocalPathByteSequence -LiteralPath $probe }
        catch { if ($_.Exception.Message -notmatch "machine-local") { throw }; $rejected = $true }
        if (-not $rejected) { throw "machine-path policy accepted ASCII path" }
        [IO.File]::WriteAllBytes($probe, [Text.Encoding]::Unicode.GetBytes('Q:\synthetic\artifact.obj'))
        $rejected = $false
        try { Assert-NoMachineLocalPathByteSequence -LiteralPath $probe }
        catch { if ($_.Exception.Message -notmatch "machine-local") { throw }; $rejected = $true }
        if (-not $rejected) { throw "machine-path policy accepted UTF-16 path" }
    }
    finally {
        if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
    }
}

if ($PolicySelfTest) {
    Invoke-PolicySelfTest
    Write-Output "Rusty Fleet owner release executable policy self-test passed"
    return
}
if ([string]::IsNullOrWhiteSpace($CapsuleRoot)) { throw "CapsuleRoot is required" }

$root = (Resolve-Path -LiteralPath $CapsuleRoot).Path
$pin = Get-Content -Raw -LiteralPath (Resolve-Path -LiteralPath $PinPath).Path |
    ConvertFrom-Json -Depth 30
Assert-ExactProperties $pin @(
    "schema", "owner_id", "owner_repository", "consumer_id", "capsule_schema",
    "capsule_version", "manifest_sha256", "manifest_size_bytes", "checksums_sha256",
    "checksums_size_bytes", "executable_sha256", "executable_size_bytes",
    "provenance_sha256", "provenance_size_bytes", "source_commit", "source_tree",
    "target", "payload", "owner_signature", "claims") "Fleet owner release pin"
Assert-ExactProperties $pin.owner_signature @("present", "authority", "reason") "owner signature boundary"
Assert-ExactProperties $pin.claims @(
    "capsule_validity", "onboarding_accepted", "device_activated", "device_reachable",
    "lease_issued", "peer_accepted", "live_authority") "capsule non-claims"
if ($pin.schema -cne "rusty.fleet.fleet_agent_key_record_owner_release_pin.v1" -or
    $pin.owner_id -cne "rusty-quest" -or
    $pin.owner_repository -cne "https://github.com/MesmerPrism/rusty-quest" -or
    $pin.consumer_id -cne "rusty-fleet/fleet-onboard" -or
    $pin.capsule_schema -cne "rusty.quest.fleet_agent_key_record_release_capsule.v1" -or
    $pin.capsule_version -cne "1.0.0" -or
    $pin.target -cne "x86_64-pc-windows-msvc" -or
    $pin.owner_signature.present -ne $false -or
    $pin.owner_signature.authority -cne "absent" -or
    $pin.owner_signature.reason -cne
        "rusty-quest-has-no-key-record-capsule-signing-or-revocation-authority" -or
    $pin.claims.capsule_validity -cne "packaging-and-tool-provenance-only" -or
    $pin.claims.onboarding_accepted -ne $false -or
    $pin.claims.device_activated -ne $false -or
    $pin.claims.device_reachable -ne $false -or
    $pin.claims.lease_issued -ne $false -or
    $pin.claims.peer_accepted -ne $false -or
    $pin.claims.live_authority -cne "rusty-manifold") {
    throw "Fleet owner release pin crosses the reviewed authority boundary"
}

$expectedFiles = @(
    "LICENSE", "SOURCE-NOTICE.md", "checksums.sha256", "fleet-agent-key-record.exe",
    "provenance.json", "release-manifest.json") | Sort-Object
$actualFiles = @(Get-ChildItem -LiteralPath $root -File -Recurse | ForEach-Object {
    [IO.Path]::GetRelativePath($root, $_.FullName).Replace("\", "/")
} | Sort-Object)
if (@(Compare-Object $actualFiles $expectedFiles -SyncWindow 0).Count -ne 0 -or
    $actualFiles.Count -ne $expectedFiles.Count) {
    throw "owner capsule contains an extra or missing file"
}

$manifestPath = Join-Path $root "release-manifest.json"
$manifestText = Get-Content -Raw -LiteralPath $manifestPath
if ((Get-Sha256 $manifestPath) -cne $pin.manifest_sha256) {
    throw "owner capsule manifest hash does not match the Fleet pin"
}
if ([long](Get-Item -LiteralPath $manifestPath).Length -ne [long]$pin.manifest_size_bytes) {
    throw "owner capsule manifest size does not match the Fleet pin"
}
$manifest = $manifestText | ConvertFrom-Json -Depth 30
Assert-ExactProperties $manifest @(
    "schema", "capsule_version", "tool_contract", "source", "artifact",
    "distribution", "payload") "owner release manifest"
Assert-ExactProperties $manifest.source @(
    "repository_url", "commit", "tree", "provenance_path", "provenance_sha256") "owner source"
Assert-ExactProperties $manifest.artifact @(
    "path", "sha256", "size_bytes", "target", "profile") "owner artifact"
Assert-ExactProperties $manifest.distribution @(
    "portable", "supported", "inert_until_invoked", "install_contract",
    "private_material_included", "live_onboarding_claim") "owner distribution"
if ($manifest.schema -cne $pin.capsule_schema -or
    $manifest.capsule_version -cne $pin.capsule_version -or
    $manifest.source.repository_url -cne $pin.owner_repository -or
    $manifest.source.commit -cne $pin.source_commit -or
    $manifest.source.tree -cne $pin.source_tree -or
    $manifest.source.provenance_path -cne "provenance.json" -or
    $manifest.source.provenance_sha256 -cne $pin.provenance_sha256 -or
    $manifest.artifact.path -cne "fleet-agent-key-record.exe" -or
    $manifest.artifact.sha256 -cne $pin.executable_sha256 -or
    [long]$manifest.artifact.size_bytes -ne [long]$pin.executable_size_bytes -or
    $manifest.artifact.target -cne $pin.target -or
    $manifest.artifact.profile -cne "release" -or
    $manifest.distribution.portable -ne $true -or
    $manifest.distribution.supported -ne $true -or
    $manifest.distribution.inert_until_invoked -ne $true -or
    $manifest.distribution.install_contract -cne "copy_capsule_byte_for_byte" -or
    $manifest.distribution.private_material_included -ne $false -or
    $manifest.distribution.live_onboarding_claim -ne $false) {
    throw "owner capsule manifest does not match the supported Fleet pin"
}

$pinnedPayload = @($pin.payload)
$ownerPayload = @($manifest.payload)
if ($pinnedPayload.Count -ne 4 -or $ownerPayload.Count -ne 4) {
    throw "owner capsule payload is not closed"
}
for ($index = 0; $index -lt $pinnedPayload.Count; $index++) {
    $expected = $pinnedPayload[$index]
    $actual = $ownerPayload[$index]
    Assert-ExactProperties $expected @("path", "sha256", "size_bytes") "pinned payload"
    Assert-ExactProperties $actual @("path", "sha256", "size_bytes") "owner payload"
    $path = Join-Path $root ([string]$expected.path)
    if ($actual.path -cne $expected.path -or $actual.sha256 -cne $expected.sha256 -or
        [long]$actual.size_bytes -ne [long]$expected.size_bytes -or
        (Get-Sha256 $path) -cne $expected.sha256 -or
        [long](Get-Item -LiteralPath $path).Length -ne [long]$expected.size_bytes) {
        throw "owner capsule payload bytes do not match the Fleet pin"
    }
}

$provenancePath = Join-Path $root "provenance.json"
$provenanceText = Get-Content -Raw -LiteralPath $provenancePath
if ((Get-Sha256 $provenancePath) -cne $pin.provenance_sha256) {
    throw "owner capsule provenance hash does not match the Fleet pin"
}
if ([long](Get-Item -LiteralPath $provenancePath).Length -ne
        [long]$pin.provenance_size_bytes) {
    throw "owner capsule provenance size does not match the Fleet pin"
}
$provenance = $provenanceText | ConvertFrom-Json -Depth 30
Assert-ExactProperties $provenance @("schema", "capsule_version", "source", "build", "claims") "owner provenance"
Assert-ExactProperties $provenance.source @(
    "repository_url", "commit", "tree", "package", "composition_fingerprint",
    "repositories", "workspace_parse_only_repositories", "files") "owner provenance source"
Assert-ExactProperties $provenance.build @(
    "target", "profile", "rustc", "cargo", "locked_dependencies",
    "isolated_git_materializations", "post_build_identity_verified", "path_remap_root",
    "symbols_stripped", "linker_reproducibility_argument", "pe_reproducibility_marker",
    "cargo_config_sha256") "owner provenance build"
Assert-ExactProperties $provenance.claims @(
    "owner", "helper_only", "runtime_activation", "enrollment_authority",
    "device_authority", "private_seed_included", "profile_included",
    "hub_configuration_included") "owner provenance claims"
$expectedRepositories = @(
    @("rusty-fleet", "contract-dependency", "https://github.com/MesmerPrism/rusty-fleet", "8181683be4a3abbc5daa0c4497c7aeb9e76316a8", "195565629a53dfaaeacb1a7260fda06062324ad9"),
    @("rusty-manifold", "contract-dependency", "https://github.com/MesmerPrism/rusty-manifold", "947421a928889889e485006bcc0200e05c2394f9", "836f1f21c5c8856bfc6dcdba8ed3721c090c76ba"),
    @("rusty-quest", "release-owner", "https://github.com/MesmerPrism/rusty-quest", [string]$pin.source_commit, [string]$pin.source_tree)
)
$expectedParseOnly = @(
    @("rusty-lattice", "workspace-parse-only", "https://github.com/MesmerPrism/rusty-lattice", "0aee7faa52fc965ff2255381781dd082ab639f4b", "4f60d4a01a3ca4dc217c4f82c16c952ab6733eb4"),
    @("rusty-matter", "workspace-parse-only", "https://github.com/MesmerPrism/rusty-matter", "eec8cddd9830f7ef0f90574ddcbde2daac0ec804", "cd4e1ce39a8c91263774ea3e69fb859f503ffde8"),
    @("rusty-optics", "workspace-parse-only", "https://github.com/MesmerPrism/rusty-optics", "fd01d84acffa1b0a3a192fe978af337d9fedd18a", "f527b761043e4e1e3a6bfa5969611dcf419e55fa")
)
$repositories = @($provenance.source.repositories)
$parseOnly = @($provenance.source.workspace_parse_only_repositories)
if ($provenance.schema -cne "rusty.quest.fleet_agent_key_record_release_provenance.v1" -or
    $provenance.capsule_version -cne $pin.capsule_version -or
    $provenance.source.repository_url -cne $pin.owner_repository -or
    $provenance.source.commit -cne $pin.source_commit -or
    $provenance.source.tree -cne $pin.source_tree -or
    $provenance.source.package -cne "rusty-quest-fleet-agent" -or
    $provenance.source.composition_fingerprint -cne "690b3f6192c27f2de7da621f1ffe4b136868701ce9161ca1fd961ee70b196609" -or
    $repositories.Count -ne $expectedRepositories.Count -or
    $parseOnly.Count -ne $expectedParseOnly.Count -or
    $provenance.build.target -cne $pin.target -or
    $provenance.build.profile -cne "release" -or
    $provenance.build.locked_dependencies -ne $true -or
    $provenance.build.isolated_git_materializations -ne $true -or
    $provenance.build.post_build_identity_verified -ne $true -or
    $provenance.build.path_remap_root -cne "/rusty-build" -or
    $provenance.build.symbols_stripped -ne $true -or
    $provenance.build.linker_reproducibility_argument -cne "/Brepro" -or
    $provenance.build.pe_reproducibility_marker -cne "IMAGE_DEBUG_TYPE_REPRO" -or
    $provenance.build.cargo_config_sha256 -cne "b25e4a2d0a7562470f166e8082f1ff2ee01bcd01cc222935e4fcffb15febfc4f" -or
    $provenance.claims.owner -cne $pin.owner_id -or
    $provenance.claims.helper_only -ne $true -or
    $provenance.claims.runtime_activation -cne "explicit_fleet_onboard_invocation" -or
    $provenance.claims.enrollment_authority -ne $false -or
    $provenance.claims.device_authority -ne $false -or
    $provenance.claims.private_seed_included -ne $false -or
    $provenance.claims.profile_included -ne $false -or
    $provenance.claims.hub_configuration_included -ne $false) {
    throw "owner capsule provenance does not match the supported Fleet pin"
}
for ($index = 0; $index -lt $expectedRepositories.Count; $index++) {
    $actual = $repositories[$index]; $expected = $expectedRepositories[$index]
    Assert-ExactProperties $actual @("repository_id", "role", "repository_url", "commit", "tree") "owner provenance repository"
    if ($actual.repository_id -cne $expected[0] -or $actual.role -cne $expected[1] -or
        $actual.repository_url -cne $expected[2] -or $actual.commit -cne $expected[3] -or
        $actual.tree -cne $expected[4]) { throw "owner capsule provenance repository set drifted" }
}
for ($index = 0; $index -lt $expectedParseOnly.Count; $index++) {
    $actual = $parseOnly[$index]; $expected = $expectedParseOnly[$index]
    Assert-ExactProperties $actual @("repository_id", "role", "repository_url", "commit", "tree") "owner parse-only repository"
    if ($actual.repository_id -cne $expected[0] -or $actual.role -cne $expected[1] -or
        $actual.repository_url -cne $expected[2] -or $actual.commit -cne $expected[3] -or
        $actual.tree -cne $expected[4]) { throw "owner capsule parse-only repository set drifted" }
}
$expectedSourceFiles = @(
    @("Cargo.lock", "0d95468b7838ea175e654baa0974781416effbaec9376b5879368ab84106330d"),
    @("Cargo.toml", "e40ca7015177e9a5e7d9546855613a01b81e855690ed8971ea1efedc7b93e6c1"),
    @("crates/rusty-quest-fleet-agent/Cargo.toml", "b2d670017388582aa7297f717c7a916901d8efd5ca325a5547bb237c38df8225"),
    @("crates/rusty-quest-fleet-agent/src/bin/fleet-agent-key-record.rs", "4f39342fdb72a6d3be94f99f949227d1ec2e2cfc13bc45a6d2c992e5f4016212"),
    @("crates/rusty-quest-fleet-agent/src/lib.rs", "af16db769ca0271438c0b84b5c0ce3fb1cfeea48416930e828d39cc43d7da11e"),
    @("tools/Build-FleetAgentKeyRecordRelease.ps1", "4525c43a52b87031cff47e79c60f73adaebacf7ebc99e2bcd7086283d864ff9b"),
    @("tools/Test-FleetAgentKeyRecordRelease.ps1", "8194506d56cbc5712c11623eabb3ba4b2f5a56f25414767f304adad7dedad486"),
    @("tools/lib/SourceComposition.psm1", "7e3a231b0703b9e0d1ab0b687a473f1a03366885a9eb3108d55839757d30c3df")
)
$sourceFiles = @($provenance.source.files)
if ($sourceFiles.Count -ne $expectedSourceFiles.Count) { throw "owner capsule source file set drifted" }
for ($index = 0; $index -lt $expectedSourceFiles.Count; $index++) {
    $actual = $sourceFiles[$index]; $expected = $expectedSourceFiles[$index]
    Assert-ExactProperties $actual @("path", "sha256") "owner provenance source file"
    if ($actual.path -cne $expected[0] -or $actual.sha256 -cne $expected[1]) {
        throw "owner capsule source file set drifted"
    }
}

$checksumsPath = Join-Path $root "checksums.sha256"
if ((Get-Sha256 $checksumsPath) -cne $pin.checksums_sha256 -or
    [long](Get-Item -LiteralPath $checksumsPath).Length -ne [long]$pin.checksums_size_bytes) {
    throw "owner capsule checksums do not match the Fleet pin"
}
$executablePath = Join-Path $root "fleet-agent-key-record.exe"
Assert-X64WindowsExecutable -LiteralPath $executablePath
Assert-PeReproducible -LiteralPath $executablePath
Assert-NoMachineLocalPathByteSequence -LiteralPath $executablePath

$publicText = $manifestText + "`n" + $provenanceText
foreach ($pattern in @(
    '[A-Za-z]:\\', '\\\\[^\\]', 'BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY',
    'signing-seed\.bin', 'hub_endpoint', 'state_directory', 'device_serial',
    'pairing_code')) {
    if ($publicText -match $pattern) {
        throw "owner capsule contains prohibited private or machine-local material"
    }
}

[pscustomobject][ordered]@{
    schema = "rusty.fleet.fleet_agent_key_record_owner_release_validation.v1"
    status = "pass"
    owner_id = [string]$pin.owner_id
    consumer_id = [string]$pin.consumer_id
    capsule_version = [string]$pin.capsule_version
    manifest_sha256 = [string]$pin.manifest_sha256
    executable_sha256 = [string]$pin.executable_sha256
    provenance_sha256 = [string]$pin.provenance_sha256
    checksums_sha256 = [string]$pin.checksums_sha256
    source_commit = [string]$pin.source_commit
    source_tree = [string]$pin.source_tree
    owner_signature_present = $false
    capsule_validity = "packaging-and-tool-provenance-only"
    onboarding_accepted = $false
    executable_reproducible = $true
    executable_machine_path_free = $true
    live_authority = "rusty-manifold"
} | ConvertTo-Json -Depth 5
