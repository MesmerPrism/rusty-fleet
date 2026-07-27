# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not ("RustyFleetAcceptanceNative" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

public static class RustyFleetAcceptanceNative
{
    private const uint MOVEFILE_REPLACE_EXISTING = 0x1;
    private const uint MOVEFILE_WRITE_THROUGH = 0x8;

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool MoveFileExW(
        string existingFileName,
        string newFileName,
        uint flags);

    public static void PublishWriteThrough(string source, string destination)
    {
        if (!MoveFileExW(
            source,
            destination,
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH))
        {
            throw new Win32Exception(
                Marshal.GetLastWin32Error(),
                "MoveFileExW write-through publication failed");
        }
    }
}
"@
}

$script:ConfigSchema = "rusty.fleet.wifi_adb_two_quest_run_config.v1"
$script:StateSchema = "rusty.fleet.wifi_adb_two_quest_acceptance_state.v2"
$script:QfmCommit = "a6d8e88c9d65f642d0cbf74fc8b92c8f1cd19ae5"
$script:HelperCommit = "d800e5c7c5f8c77ad2bae52450f32092f3c92ace"
$script:ArtifactIds = @(
    "adb",
    "fleet-onboard",
    "fleet-hub",
    "fleetctl",
    "questionable-file-manager",
    "helper-operator",
    "fleet-agent-apk",
    "wireless-adb-helper-apk",
    "termux-apk",
    "kiosk-setup-helper-apk"
)
$script:FleetAgentPackage = "io.github.mesmerprism.rustyquest.fleetagent"
$script:FleetAgentActivity =
    "io.github.mesmerprism.rustyquest.fleetagent/.FleetAgentActivity"
$script:HelperPackage = "org.questtermuxlab.wirelessadbrecovery"
$script:TermuxPackage = "com.termux"
$script:KioskPackage = "io.github.mesmerprism.rustykiosk"
$script:KioskHelperPackage = "io.github.mesmerprism.rustykiosk.setuphelper"
$script:ManagedPackages = @(
    $script:FleetAgentPackage,
    $script:HelperPackage,
    $script:TermuxPackage,
    $script:KioskPackage,
    $script:KioskHelperPackage
)
$script:RunInstallablePackages = @(
    $script:FleetAgentPackage,
    $script:HelperPackage,
    $script:TermuxPackage,
    $script:KioskHelperPackage
)
$script:WriteSecureSettingsPermission =
    "android.permission.WRITE_SECURE_SETTINGS"
$script:HelperPermissions = @(
    $script:WriteSecureSettingsPermission,
    "com.termux.permission.RUN_COMMAND",
    "android.permission.POST_NOTIFICATIONS"
)

function New-AcceptanceError {
    param(
        [Parameter(Mandatory)][string] $Code,
        [Parameter(Mandatory)][string] $Message
    )
    return [InvalidOperationException]::new("$Code`: $Message")
}

function Assert-Condition {
    param(
        [Parameter(Mandatory)][bool] $Condition,
        [Parameter(Mandatory)][string] $Code,
        [Parameter(Mandatory)][string] $Message
    )
    if (-not $Condition) {
        throw (New-AcceptanceError -Code $Code -Message $Message)
    }
}

function Assert-ExactProperties {
    param(
        [Parameter(Mandatory)][object] $Value,
        [Parameter(Mandatory)][string[]] $Required,
        [string[]] $Optional = @(),
        [Parameter(Mandatory)][string] $Context
    )
    Assert-Condition ($Value -is [Collections.IDictionary]) "config_shape_invalid" `
        "$Context must be a JSON object."
    $names = @($Value.Keys)
    foreach ($name in $names) {
        Assert-Condition ($name -in @($Required + $Optional)) "config_unknown_field" `
            "$Context contains an unknown field."
    }
    foreach ($name in $Required) {
        Assert-Condition $Value.Contains($name) "config_missing_field" `
            "$Context is missing a required field."
    }
}

function Assert-NoDuplicateJsonProperties {
    param([Parameter(Mandatory)][string] $Json)

    $document = $null
    try {
        $document = [Text.Json.JsonDocument]::Parse(
            $Json,
            [Text.Json.JsonDocumentOptions]@{
                AllowTrailingCommas = $false
                CommentHandling = [Text.Json.JsonCommentHandling]::Disallow
                MaxDepth = 64
            })
    } catch {
        throw (New-AcceptanceError "config_json_invalid" "The private run config is not strict JSON.")
    }

    function Visit-JsonElement {
        param([Parameter(Mandatory)][Text.Json.JsonElement] $Element)
        if ($Element.ValueKind -eq [Text.Json.JsonValueKind]::Object) {
            $names = [Collections.Generic.HashSet[string]]::new(
                [StringComparer]::Ordinal)
            foreach ($property in $Element.EnumerateObject()) {
                if (-not $names.Add($property.Name)) {
                    throw (New-AcceptanceError "config_duplicate_field" `
                        "The private run config contains a duplicate JSON property.")
                }
                Visit-JsonElement -Element $property.Value
            }
        } elseif ($Element.ValueKind -eq [Text.Json.JsonValueKind]::Array) {
            foreach ($item in $Element.EnumerateArray()) {
                Visit-JsonElement -Element $item
            }
        }
    }

    try {
        Visit-JsonElement -Element $document.RootElement
    } finally {
        $document.Dispose()
    }
}

function Get-Sha256 {
    param([Parameter(Mandatory)][string] $Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).
        Hash.ToLowerInvariant()
}

function Get-BytesSha256 {
    param([Parameter(Mandatory)][byte[]] $Bytes)
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        return [Convert]::ToHexString($hasher.ComputeHash($Bytes)).
            ToLowerInvariant()
    } finally {
        $hasher.Dispose()
    }
}

function Resolve-PrivateLeaf {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $Context,
        [switch] $AllowMissing
    )
    Assert-Condition ([IO.Path]::IsPathFullyQualified($Path)) "private_path_invalid" `
        "$Context must be an absolute path in the private run config."
    $full = [IO.Path]::GetFullPath($Path)
    Assert-Condition (-not $full.StartsWith("\\", [StringComparison]::Ordinal)) `
        "private_path_invalid" "$Context must not use a UNC path."
    $cursor = if (Test-Path -LiteralPath $full) {
        Get-Item -LiteralPath $full -Force
    } else {
        $parent = Split-Path -Parent $full
        while ($parent -and -not (Test-Path -LiteralPath $parent)) {
            $parent = Split-Path -Parent $parent
        }
        if ($parent) { Get-Item -LiteralPath $parent -Force } else { $null }
    }
    while ($null -ne $cursor) {
        Assert-Condition (-not (
                $cursor.Attributes -band [IO.FileAttributes]::ReparsePoint
            )) "private_input_reparse" `
            "$Context must not traverse a reparse point."
        $cursor = if ($cursor -is [IO.FileInfo]) {
            $cursor.Directory
        } else {
            $cursor.Parent
        }
    }
    if ($AllowMissing) {
        return $full
    }
    Assert-Condition (Test-Path -LiteralPath $full -PathType Leaf) `
        "private_input_missing" "$Context does not resolve to one file."
    $item = Get-Item -LiteralPath $full -Force
    Assert-Condition (-not ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) `
        "private_input_reparse" "$Context must not be a reparse point."
    return $full
}

function Read-StrictJsonFile {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][string] $Context,
        [ValidateRange(1, 1048576)][int] $MaximumBytes = 65536
    )
    $resolved = Resolve-PrivateLeaf -Path $Path -Context $Context
    $stream = [IO.File]::Open(
        $resolved,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::Read)
    try {
        Assert-Condition ($stream.Length -gt 0 -and $stream.Length -le $MaximumBytes) `
            "private_input_size_invalid" "$Context has an invalid byte length."
        $bytes = [byte[]]::new([int]$stream.Length)
        $read = 0
        while ($read -lt $bytes.Length) {
            $count = $stream.Read($bytes, $read, $bytes.Length - $read)
            Assert-Condition ($count -gt 0) "private_input_changed" `
                "$Context changed while it was read."
            $read += $count
        }
        Assert-Condition ($stream.Position -eq $stream.Length) "private_input_changed" `
            "$Context changed while it was read."
        $utf8 = [Text.UTF8Encoding]::new($false, $true)
        try {
            $json = $utf8.GetString($bytes)
        } catch {
            throw (New-AcceptanceError "private_input_encoding_invalid" `
                "$Context must be strict UTF-8.")
        }
        Assert-NoDuplicateJsonProperties -Json $json
        try {
            $value = $json | ConvertFrom-Json -AsHashtable -Depth 64
        } catch {
            throw (New-AcceptanceError "private_input_json_invalid" `
                "$Context must be strict JSON.")
        }
        return [pscustomobject]@{
            Path = $resolved
            Json = $json
            Value = $value
            Sha256 = Get-BytesSha256 -Bytes $bytes
        }
    } finally {
        $stream.Dispose()
        if ($null -ne $bytes) {
            [Array]::Clear($bytes, 0, $bytes.Length)
        }
    }
}

function Test-PortableId {
    param([AllowNull()][object] $Value)
    return $Value -is [string] -and
        $Value.Length -ge 2 -and $Value.Length -le 128 -and
        $Value -cmatch '^[a-z0-9]+(?:[._-][a-z0-9]+)*$'
}

function Get-ArtifactMap {
    param([Parameter(Mandatory)][Collections.IDictionary] $Config)
    $map = [ordered]@{}
    foreach ($pin in $Config.artifact_pins) {
        Assert-ExactProperties -Value $pin `
            -Required @("artifact_id", "path", "sha256") -Context "artifact pin"
        $id = [string]$pin.artifact_id
        Assert-Condition ($id -cin $script:ArtifactIds) "artifact_id_invalid" `
            "The private run config contains an unsupported artifact pin."
        Assert-Condition (-not $map.Contains($id)) "artifact_id_duplicate" `
            "Each required artifact must be pinned exactly once."
        Assert-Condition ([string]$pin.sha256 -cmatch '^[0-9a-f]{64}$') `
            "artifact_hash_invalid" "Every artifact pin needs a lowercase SHA-256."
        $path = Resolve-PrivateLeaf -Path ([string]$pin.path) `
            -Context "artifact pin $id"
        $actual = Get-Sha256 -Path $path
        Assert-Condition ($actual -ceq [string]$pin.sha256) "artifact_hash_mismatch" `
            "A pinned artifact does not match its configured SHA-256."
        $map[$id] = [pscustomobject]@{
            Id = $id
            Path = $path
            Sha256 = $actual
        }
    }
    Assert-Condition ($map.Count -eq $script:ArtifactIds.Count) `
        "artifact_set_invalid" "The exact required artifact set must be pinned."
    foreach ($id in $script:ArtifactIds) {
        Assert-Condition $map.Contains($id) "artifact_set_invalid" `
            "The exact required artifact set must be pinned."
    }
    return $map
}

function Read-ValidatedRunConfig {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string] $RunConfig)

    $read = Read-StrictJsonFile -Path $RunConfig -Context "private run config"
    $config = $read.Value
    Assert-ExactProperties -Value $config -Required @(
        "schema",
        "run_id",
        "private_state_root",
        "source_commits",
        "artifact_pins",
        "onboarding",
        "hub",
        "devices",
        "timing"
    ) -Context "private run config"
    Assert-Condition ([string]$config.schema -ceq $script:ConfigSchema) `
        "config_schema_invalid" "The private run config schema is unsupported."
    Assert-Condition (Test-PortableId $config.run_id) "run_id_invalid" `
        "The run ID must be a bounded lowercase portable identifier."

    Assert-ExactProperties -Value $config.source_commits -Required @(
        "questionable_file_manager",
        "wireless_adb_helper"
    ) -Context "source_commits"
    Assert-Condition (
        [string]$config.source_commits.questionable_file_manager -ceq $script:QfmCommit -and
        [string]$config.source_commits.wireless_adb_helper -ceq $script:HelperCommit
    ) "owner_source_mismatch" "The exact reviewed QFM and helper commits are required."

    Assert-Condition ([IO.Path]::IsPathFullyQualified(
            [string]$config.private_state_root)) "private_state_root_invalid" `
        "private_state_root must be absolute."
    $stateRoot = [IO.Path]::GetFullPath([string]$config.private_state_root)
    Assert-Condition (-not $stateRoot.StartsWith("\\", [StringComparison]::Ordinal)) `
        "private_state_root_invalid" "private_state_root must not be a UNC path."
    $config.private_state_root = $stateRoot

    Assert-ExactProperties -Value $config.onboarding `
        -Required @("request_path", "inventory_path") -Context "onboarding"
    $config.onboarding.request_path = Resolve-PrivateLeaf `
        -Path ([string]$config.onboarding.request_path) `
        -Context "onboarding request"
    $config.onboarding.inventory_path = Resolve-PrivateLeaf `
        -Path ([string]$config.onboarding.inventory_path) `
        -Context "onboarding inventory" -AllowMissing

    Assert-ExactProperties -Value $config.hub `
        -Required @("config_path", "operator_url", "manage_firewall") -Context "hub"
    $config.hub.config_path = Resolve-PrivateLeaf `
        -Path ([string]$config.hub.config_path) -Context "Hub config" -AllowMissing
    $operatorUri = $null
    $parsed = [Uri]::TryCreate(
        [string]$config.hub.operator_url,
        [UriKind]::Absolute,
        [ref]$operatorUri)
    Assert-Condition ($parsed -and $operatorUri.Scheme -ceq "http" -and
        $operatorUri.IsLoopback -and $operatorUri.Port -ge 1 -and
        $operatorUri.Port -le 65535 -and
        -not $operatorUri.AbsolutePath.Trim("/") -and
        -not $operatorUri.Query -and -not $operatorUri.Fragment) `
        "hub_operator_url_invalid" "The operator URL must be one loopback HTTP origin."
    Assert-Condition ($config.hub.manage_firewall -is [bool]) `
        "hub_firewall_flag_invalid" "manage_firewall must be boolean."

    Assert-Condition ($config.devices -is [Collections.IList] -and
        $config.devices.Count -eq 2) "device_count_invalid" `
        "Exactly two device records are required."
    $slots = @()
    $deviceIds = @()
    $serials = @()
    $seedHashes = @()
    $onboardingRequest = Read-StrictJsonFile `
        -Path $config.onboarding.request_path -Context "onboarding request"
    Assert-Condition (
        [string]$onboardingRequest.Value.schema -ceq
            "rusty.fleet.offline_onboarding_request.v1" -and
        $onboardingRequest.Value.devices -is [Collections.IList] -and
        $onboardingRequest.Value.devices.Count -eq 2
    ) "onboarding_request_invalid" `
        "The onboarding request must describe exactly two Fleet devices."
    $onboardingDeviceIds = @(
        $onboardingRequest.Value.devices | ForEach-Object { [string]$_.device_id }
    )
    foreach ($device in $config.devices) {
        Assert-ExactProperties -Value $device -Required @(
            "slot",
            "device_id",
            "identity_revision",
            "usb_serial",
            "qfm_enrollment_path",
            "fleet_agent_profile_path",
            "fleet_agent_seed_path"
        ) -Context "device"
        Assert-Condition ([string]$device.slot -cin @("device_a", "device_b")) `
            "device_slot_invalid" "Device slots must be device_a and device_b."
        Assert-Condition (Test-PortableId $device.device_id) "device_id_invalid" `
            "Fleet device IDs must be bounded lowercase portable identifiers."
        Assert-Condition ($device.identity_revision -is [long] -or
            $device.identity_revision -is [int]) "identity_revision_invalid" `
            "identity_revision must be an integer."
        Assert-Condition ([long]$device.identity_revision -ge 1) `
            "identity_revision_invalid" "identity_revision must be positive."
        Assert-Condition ([string]$device.usb_serial -cmatch '^[^\s:]{1,128}$') `
            "usb_serial_invalid" "Each device requires one exact USB-only serial."

        $device.qfm_enrollment_path = Resolve-PrivateLeaf `
            -Path ([string]$device.qfm_enrollment_path) `
            -Context "QFM enrollment"
        $device.fleet_agent_profile_path = Resolve-PrivateLeaf `
            -Path ([string]$device.fleet_agent_profile_path) `
            -Context "Fleet Agent profile" -AllowMissing
        $device.fleet_agent_seed_path = Resolve-PrivateLeaf `
            -Path ([string]$device.fleet_agent_seed_path) `
            -Context "Fleet Agent seed" -AllowMissing

        $enrollment = Read-StrictJsonFile -Path $device.qfm_enrollment_path `
            -Context "QFM enrollment" -MaximumBytes 4096
        try {
            Assert-Condition (
                [string]$enrollment.Value.schema -ceq
                    "questionable.file_manager.quest_connectivity_profile_enrollment.v1" -and
                [string]$enrollment.Value.device_id -ceq [string]$device.device_id -and
                [string]$enrollment.Value.usb_serial -ceq [string]$device.usb_serial
            ) "qfm_enrollment_mismatch" `
                "Each QFM enrollment must bind its exact configured device and USB serial."
        } finally {
            $enrollment = $null
        }
        $profileExists = Test-Path -LiteralPath $device.fleet_agent_profile_path -PathType Leaf
        $seedExists = Test-Path -LiteralPath $device.fleet_agent_seed_path -PathType Leaf
        Assert-Condition ($profileExists -eq $seedExists) `
            "onboarding_output_partial" `
            "Fleet Agent profile and seed must be both absent or both present."
        if ($profileExists) {
            $profile = Read-StrictJsonFile -Path $device.fleet_agent_profile_path `
                -Context "Fleet Agent profile"
            try {
                Assert-Condition (
                    [string]$profile.Value.schema -ceq
                        "rusty.quest.fleet_agent_profile.v1" -and
                    $profile.Value.enabled -eq $true -and
                    [string]$profile.Value.device_id -ceq [string]$device.device_id -and
                    [long]$profile.Value.identity_revision -eq
                        [long]$device.identity_revision
                ) "fleet_agent_profile_mismatch" `
                    "Each Fleet Agent profile must bind its exact configured device."
            } finally {
                $profile = $null
            }
            $seedInfo = Get-Item -LiteralPath $device.fleet_agent_seed_path
            Assert-Condition ($seedInfo.Length -eq 32) "fleet_agent_seed_invalid" `
                "Each Fleet Agent seed must contain exactly 32 bytes."
            $seedHashes += Get-Sha256 -Path $device.fleet_agent_seed_path
        }

        $slots += [string]$device.slot
        $deviceIds += [string]$device.device_id
        $serials += [string]$device.usb_serial
    }
    Assert-Condition (
        @($slots | Sort-Object -Unique).Count -eq 2 -and
        $slots -contains "device_a" -and $slots -contains "device_b"
    ) "device_slot_duplicate" "The two logical device slots must be distinct."
    Assert-Condition (@($deviceIds | Sort-Object -Unique).Count -eq 2) `
        "device_id_duplicate" "The two Fleet device IDs must be distinct."
    Assert-Condition (@($serials | Sort-Object -Unique).Count -eq 2) `
        "usb_serial_duplicate" "The two USB serials must be distinct."
    Assert-Condition (
        @($onboardingDeviceIds | Sort-Object -Unique).Count -eq 2 -and
        @($onboardingDeviceIds | Where-Object { $_ -notin $deviceIds }).Count -eq 0
    ) "onboarding_device_mismatch" `
        "The onboarding request must bind the exact two Fleet device IDs."
    if ($seedHashes.Count -gt 0) {
        Assert-Condition (@($seedHashes | Sort-Object -Unique).Count -eq 2) `
            "fleet_agent_seed_duplicate" "The two signing seeds must be distinct."
    }

    Assert-ExactProperties -Value $config.timing -Required @(
        "baseline_timeout_seconds",
        "proof_timeout_seconds",
        "expiry_timeout_seconds",
        "reboot_timeout_seconds"
    ) -Context "timing"
    foreach ($name in @(
        "baseline_timeout_seconds",
        "proof_timeout_seconds",
        "expiry_timeout_seconds",
        "reboot_timeout_seconds"
    )) {
        Assert-Condition (
            $config.timing[$name] -is [int] -or
            $config.timing[$name] -is [long]
        ) "timing_invalid" "Timing values must be integers."
    }
    Assert-Condition ([int]$config.timing.baseline_timeout_seconds -in 10..600) `
        "timing_invalid" "baseline_timeout_seconds is out of range."
    Assert-Condition ([int]$config.timing.proof_timeout_seconds -in 10..180) `
        "timing_invalid" "proof_timeout_seconds is out of range."
    Assert-Condition ([int]$config.timing.expiry_timeout_seconds -in 61..300) `
        "timing_invalid" "expiry_timeout_seconds is out of range."
    Assert-Condition ([int]$config.timing.reboot_timeout_seconds -in 30..600) `
        "timing_invalid" "reboot_timeout_seconds is out of range."

    $artifacts = Get-ArtifactMap -Config $config
    return [pscustomobject]@{
        Path = $read.Path
        Sha256 = $read.Sha256
        Config = $config
        Artifacts = $artifacts
    }
}

function Get-TermuxAdmissionLineageSha256 {
    param([Parameter(Mandatory)][object] $Admission)
    $stream = [IO.MemoryStream]::new()
    try {
        $utf8 = [Text.UTF8Encoding]::new($false)
        $stream.Write($utf8.GetBytes(
                "rusty.fleet.quest-wifi-adb.termux-admission-lineage.v1`0"))
        foreach ($value in @(
            $Admission.checkin_id,
            $Admission.operation_id,
            $Admission.device_id,
            $Admission.source_epoch,
            $Admission.proof_id,
            $Admission.receipt_request_id,
            $Admission.receipt_evidence_sha256,
            $Admission.key_id,
            $Admission.public_key_sha256,
            $Admission.claims_jcs_sha256,
            $Admission.signing_message_sha256,
            $Admission.signature_sha256
        )) {
            $stream.Write($utf8.GetBytes([string]$value))
            $stream.WriteByte(0)
        }
        foreach ($value in @(
            [uint64]$Admission.identity_revision,
            [uint64]$Admission.source_revision,
            [uint64]$Admission.evidence_revision,
            [uint64]$Admission.key_generation,
            [uint64]$Admission.fleet_accepted_revision,
            [uint64]$Admission.enrollment_authority_revision,
            [uint64]$Admission.manifold_authority_revision
        )) {
            $bytes = [BitConverter]::GetBytes($value)
            if (-not [BitConverter]::IsLittleEndian) { [Array]::Reverse($bytes) }
            $stream.Write($bytes)
        }
        foreach ($value in @(
            [int64]$Admission.accepted_at_ms,
            [int64]$Admission.expires_at_ms
        )) {
            $bytes = [BitConverter]::GetBytes($value)
            if (-not [BitConverter]::IsLittleEndian) { [Array]::Reverse($bytes) }
            $stream.Write($bytes)
        }
        return Get-BytesSha256 -Bytes $stream.ToArray()
    } finally {
        $stream.Dispose()
    }
}

function Test-HubTermuxAdmission {
    [CmdletBinding()]
    param(
        [AllowNull()][object] $Operation,
        [Parameter(Mandatory)][string] $ExpectedOperationId,
        [Parameter(Mandatory)][string] $ExpectedDeviceId,
        [Parameter(Mandatory)][long] $ExpectedIdentityRevision,
        [Parameter(Mandatory)][long] $NowMs,
        [long] $MinimumEvidenceRevision = 0
    )
    if ($null -eq $Operation -or @($Operation.targets).Count -ne 1) {
        return [pscustomobject]@{
            Valid = $false
            ReasonCode = "hub_operation_absent"
        }
    }
    $target = $Operation.targets[0]
    $proof = $target.termux_proof
    $admission = $target.termux_admission
    $receipt = $target.receipt
    if ($null -eq $proof -or $null -eq $admission -or $null -eq $receipt) {
        return [pscustomobject]@{
            Valid = $false
            ReasonCode = "signed_admission_absent"
        }
    }
    $hashPattern = '^[0-9a-f]{64}$'
    $valid = (
        [string]$Operation.operation_id -ceq $ExpectedOperationId -and
        [string]$target.device_id -ceq $ExpectedDeviceId -and
        [long]$target.identity_revision -eq $ExpectedIdentityRevision -and
        [string]$proof.schema -ceq "rusty.fleet.quest_wifi_adb_termux_proof.v1" -and
        [string]$proof.owner_id -ceq "quest-termux-lab" -and
        [string]$proof.device_id -ceq $ExpectedDeviceId -and
        [long]$proof.identity_revision -eq $ExpectedIdentityRevision -and
        [string]$proof.route_mode -ceq "modern_tls" -and
        [string]$proof.discovery_mode -cin @("tls_nsd", "tls_mdns") -and
        $proof.listener_discovered -eq $true -and
        [string]$proof.shell_identity -ceq "uid=2000(shell)" -and
        $proof.available -eq $true -and
        [long]$proof.evidence_revision -gt $MinimumEvidenceRevision -and
        [long]$proof.observed_at_ms -le $NowMs -and
        [long]$proof.fresh_until_ms -ge $NowMs -and
        [long]$proof.fresh_until_ms -gt [long]$proof.observed_at_ms -and
        ([long]$proof.fresh_until_ms - [long]$proof.observed_at_ms) -le 60000 -and
        [string]$proof.evidence_sha256 -cmatch $hashPattern -and
        [string]$admission.schema -ceq
            "rusty.fleet.quest_wifi_adb_termux_admission.v1" -and
        [string]$admission.operation_id -ceq $ExpectedOperationId -and
        [string]$admission.device_id -ceq $ExpectedDeviceId -and
        [long]$admission.identity_revision -eq $ExpectedIdentityRevision -and
        [string]$admission.proof_id -ceq [string]$proof.proof_id -and
        [string]$admission.source_epoch -ceq [string]$proof.source_epoch -and
        [long]$admission.source_revision -eq [long]$proof.source_revision -and
        [long]$admission.evidence_revision -eq [long]$proof.evidence_revision -and
        [string]$admission.receipt_request_id -ceq [string]$receipt.request_id -and
        [string]$admission.receipt_evidence_sha256 -ceq
            [string]$receipt.evidence_sha256 -and
        $admission.signature_verified -eq $true -and
        $admission.canonical_claims_verified -eq $true -and
        $admission.enrollment_active -eq $true -and
        [long]$admission.key_generation -gt 0 -and
        [long]$admission.fleet_accepted_revision -gt 0 -and
        [long]$admission.enrollment_authority_revision -gt 0 -and
        [long]$admission.manifold_authority_revision -gt 0 -and
        [long]$admission.accepted_at_ms -le $NowMs -and
        [long]$admission.expires_at_ms -ge $NowMs -and
        [long]$admission.expires_at_ms -gt [long]$admission.accepted_at_ms -and
        [string]$admission.public_key_sha256 -cmatch $hashPattern -and
        [string]$admission.claims_jcs_sha256 -cmatch $hashPattern -and
        [string]$admission.signing_message_sha256 -cmatch $hashPattern -and
        [string]$admission.signature_sha256 -cmatch $hashPattern -and
        [string]$admission.lineage_sha256 -ceq
            (Get-TermuxAdmissionLineageSha256 -Admission $admission)
    )
    if ($valid) {
        return [pscustomobject]@{
            Valid = $true
            ReasonCode = "signed_admission_valid"
        }
    }
    $reason = if ([string]$Operation.operation_id -cne $ExpectedOperationId) {
        "admission_operation_mismatch"
    } elseif ([string]$proof.device_id -cne $ExpectedDeviceId -or
        [string]$admission.device_id -cne $ExpectedDeviceId) {
        "admission_device_mismatch"
    } elseif ([string]$proof.shell_identity -cne "uid=2000(shell)") {
        "proof_shell_uid_invalid"
    } elseif ([long]$proof.fresh_until_ms -lt $NowMs -or
        [long]$admission.expires_at_ms -lt $NowMs) {
        "proof_stale"
    } elseif ([long]$proof.evidence_revision -le $MinimumEvidenceRevision) {
        "proof_revision_not_advanced"
    } else {
        "signed_admission_invalid"
    }
    return [pscustomobject]@{ Valid = $false; ReasonCode = $reason }
}

function Get-CleanupTruth {
    [CmdletBinding()]
    param([Parameter(Mandatory)][Collections.IDictionary] $Checks)
    $values = @($Checks.Values)
    $failed = @($values | Where-Object { $_ -ne $true }).Count
    return [pscustomobject]@{
        Status = if ($failed -eq 0) { "complete" } else { "partial_failure" }
        FailedCount = $failed
        TotalCount = $values.Count
    }
}

function Invoke-BoundedProcess {
    param(
        [Parameter(Mandatory)][string] $FilePath,
        [Parameter(Mandatory)][string[]] $Arguments,
        [AllowNull()][string] $StandardInput = $null,
        [ValidateRange(1, 600)][int] $TimeoutSeconds = 30,
        [Collections.IDictionary] $Environment = @{}
    )
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $FilePath
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.StandardOutputEncoding = [Text.Encoding]::UTF8
    $start.StandardErrorEncoding = [Text.Encoding]::UTF8
    if ($null -ne $StandardInput) {
        $start.RedirectStandardInput = $true
        $start.StandardInputEncoding = [Text.Encoding]::UTF8
    }
    foreach ($argument in $Arguments) {
        $start.ArgumentList.Add($argument)
    }
    foreach ($entry in $Environment.GetEnumerator()) {
        $start.Environment[[string]$entry.Key] = [string]$entry.Value
    }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    Assert-Condition $process.Start() "owner_process_start_failed" `
        "A pinned owner process could not be started."
    try {
        if ($null -ne $StandardInput) {
            $process.StandardInput.Write($StandardInput)
            $process.StandardInput.Close()
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            try {
                $process.Kill($true)
                [void]$process.WaitForExit(5000)
            } catch {
                # The caller receives a typed timeout regardless of platform detail.
            }
            throw (New-AcceptanceError "owner_process_timed_out" `
                "A pinned owner process exceeded its bounded deadline.")
        }
        [Threading.Tasks.Task]::WaitAll(
            [Threading.Tasks.Task[]]@($stdoutTask, $stderrTask),
            5000) | Out-Null
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        Assert-Condition ($stdout.Length -le 1MB -and $stderr.Length -le 256KB) `
            "owner_output_oversized" "A pinned owner process exceeded output bounds."
        return [pscustomobject]@{
            ExitCode = $process.ExitCode
            Stdout = $stdout
            Stderr = $stderr
        }
    } finally {
        $process.Dispose()
    }
}

function Invoke-JsonOwner {
    param(
        [Parameter(Mandatory)][string] $FilePath,
        [Parameter(Mandatory)][string[]] $Arguments,
        [AllowNull()][string] $StandardInput = $null,
        [ValidateRange(1, 600)][int] $TimeoutSeconds = 30,
        [Collections.IDictionary] $Environment = @{},
        [int[]] $AllowedExitCodes = @(0)
    )
    $result = Invoke-BoundedProcess -FilePath $FilePath -Arguments $Arguments `
        -StandardInput $StandardInput -TimeoutSeconds $TimeoutSeconds `
        -Environment $Environment
    Assert-Condition ($result.ExitCode -in $AllowedExitCodes) `
        "owner_process_rejected" "A pinned owner process rejected the closed request."
    try {
        return $result.Stdout | ConvertFrom-Json -Depth 64
    } catch {
        throw (New-AcceptanceError "owner_json_invalid" `
            "A pinned owner process returned invalid JSON.")
    } finally {
        $result = $null
    }
}

function Invoke-AdbExact {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device,
        [Parameter(Mandatory)][string[]] $Arguments,
        [AllowNull()][string] $StandardInput = $null,
        [ValidateRange(1, 600)][int] $TimeoutSeconds = 30,
        [int[]] $AllowedExitCodes = @(0)
    )
    $fullArguments = @("-s", [string]$Device.usb_serial) + $Arguments
    $result = Invoke-BoundedProcess `
        -FilePath $Context.Artifacts["adb"].Path `
        -Arguments $fullArguments -StandardInput $StandardInput `
        -TimeoutSeconds $TimeoutSeconds
    Assert-Condition ($result.ExitCode -in $AllowedExitCodes) `
        "serial_scoped_adb_failed" "A fixed serial-scoped ADB operation failed."
    return $result
}

function Invoke-QfmExact {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][string[]] $Arguments,
        [ValidateRange(1, 600)][int] $TimeoutSeconds = 60
    )
    return Invoke-JsonOwner `
        -FilePath $Context.Artifacts["questionable-file-manager"].Path `
        -Arguments $Arguments -TimeoutSeconds $TimeoutSeconds
}

function Invoke-HelperExact {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device,
        [Parameter(Mandatory)]
        [ValidateSet(
            "status",
            "restore-now",
            "enable-boot-attempt",
            "disable-boot-attempt",
            "disable-wireless",
            "prepare-termux-prerequisites")]
        [string] $HelperAction,
        [string] $RequestId = "",
        [switch] $Confirm,
        [switch] $ConfirmTermuxPackageInstall
    )
    if (-not $RequestId) {
        $RequestId = "fleet-" + [guid]::NewGuid().ToString("N")
    }
    $arguments = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", $Context.Artifacts["helper-operator"].Path,
        "-Serial", [string]$Device.usb_serial,
        "-Action", $HelperAction,
        "-RequestId", $RequestId,
        "-TimeoutSeconds", [string][int]$Context.Config.timing.proof_timeout_seconds
    )
    if ($Confirm) {
        $arguments += "-Confirm"
    }
    if ($ConfirmTermuxPackageInstall) {
        $arguments += "-ConfirmTermuxPackageInstall"
    }
    $adbDirectory = Split-Path -Parent $Context.Artifacts["adb"].Path
    $environment = @{
        PATH = $adbDirectory + [IO.Path]::PathSeparator + $env:PATH
    }
    return Invoke-JsonOwner -FilePath (Get-Command pwsh -ErrorAction Stop).Source `
        -Arguments $arguments -Environment $environment `
        -TimeoutSeconds ([int]$Context.Config.timing.proof_timeout_seconds + 15) `
        -AllowedExitCodes @(0, 2, 3)
}

function Invoke-FleetCtlExact {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][string[]] $Arguments
    )
    return Invoke-JsonOwner -FilePath $Context.Artifacts["fleetctl"].Path `
        -Arguments $Arguments -TimeoutSeconds 30 `
        -Environment @{
            RUSTY_FLEET_HUB_URL = [string]$Context.Config.hub.operator_url
        }
}

function Get-DeviceBySlot {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot
    )
    $match = @($Context.Config.devices | Where-Object {
        [string]$_.slot -ceq $Slot
    })
    Assert-Condition ($match.Count -eq 1) "device_slot_missing" `
        "The requested logical device slot is unavailable."
    return $match[0]
}

function Get-StatePath {
    param([Parameter(Mandatory)][object] $Context)
    return Join-Path $Context.Config.private_state_root "acceptance-state.json"
}

function Write-SanitizedState {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    $root = [IO.Path]::GetFullPath([string]$Context.Config.private_state_root)
    Assert-Condition (Test-Path -LiteralPath $root -PathType Container) `
        "state_root_missing" "The private state root has not been created by Preflight."
    $json = $State | ConvertTo-Json -Depth 32
    $privateValues = @(
        [string]$Context.Path,
        [string]$Context.Config.private_state_root,
        [string]$Context.Config.hub.config_path,
        [string]$Context.Config.onboarding.request_path,
        [string]$Context.Config.onboarding.inventory_path
    )
    foreach ($artifact in $Context.Artifacts.Values) {
        $privateValues += [string]$artifact.Path
    }
    foreach ($device in $Context.Config.devices) {
        $privateValues += @(
            [string]$device.device_id,
            [string]$device.usb_serial,
            [string]$device.qfm_enrollment_path,
            [string]$device.fleet_agent_profile_path,
            [string]$device.fleet_agent_seed_path
        )
    }
    foreach ($value in $privateValues | Where-Object {
        -not [string]::IsNullOrWhiteSpace($_)
    }) {
        Assert-Condition (-not $json.Contains(
                $value, [StringComparison]::OrdinalIgnoreCase)) `
            "state_private_value_detected" `
            "Sanitized state contains a private run-config value."
    }

    $path = Get-StatePath -Context $Context
    $temporary = Join-Path $root (
        ".acceptance-state-" + [guid]::NewGuid().ToString("N") + ".pending")
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(
        $json + [Environment]::NewLine)
    $stream = [IO.FileStream]::new(
        $temporary,
        [IO.FileMode]::CreateNew,
        [IO.FileAccess]::Write,
        [IO.FileShare]::None,
        4096,
        [IO.FileOptions]::WriteThrough)
    try {
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally {
        $stream.Dispose()
        [Array]::Clear($bytes, 0, $bytes.Length)
    }
    $temporaryItem = Get-Item -LiteralPath $temporary -Force
    Assert-Condition (-not (
            $temporaryItem.Attributes -band [IO.FileAttributes]::ReparsePoint
        )) "state_temporary_reparse" `
        "The randomized state temp file became a reparse point."
    [RustyFleetAcceptanceNative]::PublishWriteThrough($temporary, $path)
}

function Assert-ValidSanitizedStateShape {
    param([Parameter(Mandatory)][Collections.IDictionary] $State)
    Assert-ExactProperties -Value $State -Required @(
        "schema", "run_id_hash", "config_sha256", "status", "phase",
        "sequence", "checkpoint", "devices", "hub", "onboarding", "cleanup",
        "claims", "events", "mutation", "mutation_history",
        "journal_head_sha256", "final_receipt_sha256"
    ) -Context "acceptance state"
    Assert-ExactProperties -Value $State.hub -Required @(
        "started_by_run", "process_id", "firewall_created",
        "two_fresh_baseline_checkins"
    ) -Context "acceptance state hub"
    Assert-ExactProperties -Value $State.onboarding -Required @(
        "apply_attempted_by_run", "applied_by_run", "distinct_profiles_verified"
    ) -Context "acceptance state onboarding"
    Assert-ExactProperties -Value $State.cleanup -Required @(
        "attempted", "status", "checks"
    ) -Context "acceptance state cleanup"
    Assert-ExactProperties -Value $State.claims -Required @(
        "planned_only", "installed", "reachable", "authorized", "effective"
    ) -Context "acceptance state claims"
    Assert-Condition ($State.devices -is [Collections.IList] -and
        $State.devices.Count -eq 2) "state_shape_invalid" `
        "Acceptance state must retain exactly two device slots."
    foreach ($device in $State.devices) {
        Assert-ExactProperties -Value $device -Required @(
            "slot", "snapshot", "run_owned", "acceptance"
        ) -Context "acceptance state device"
        Assert-ExactProperties -Value $device.snapshot -Required @(
            "usb_ready", "package_set_sha256", "packages", "helper_grants",
            "kiosk_helper_write_secure_settings_granted", "qfm_profile_state",
            "kiosk_direct_link_observation", "after_boot_enabled",
            "wifi_setting_enabled", "transport_usb_present",
            "signer_checks_complete", "agent_process_present",
            "agent_private_inputs_absent", "boot_id_sha256",
            "boot_elapsed_milliseconds", "termux_process_epoch_sha256"
        ) -Context "acceptance state device snapshot"
        Assert-ExactProperties -Value $device.run_owned -Required @(
            "qfm_profile_created", "agent_profile_staged", "agent_started",
            "termux_restart_confirmed", "added_packages"
        ) -Context "acceptance state run ownership"
        Assert-ExactProperties -Value $device.acceptance -Required @(
            "baseline_revision", "proof_revision", "renewed_proof_revision",
            "termux_usable", "expiry_observed", "disable_observed",
            "reboot_loss_observed", "recovery_observed",
            "boot_disable_confirmed", "wireless_disable_confirmed"
        ) -Optional @(
            "operation_id", "status_operation_id", "disable_operation_id",
            "pre_reboot_boot_id_sha256",
            "pre_reboot_elapsed_milliseconds", "pre_reboot_source_epoch_sha256",
            "pre_reboot_accepted_revision", "isolation_projection_sha256"
        ) -Context "acceptance state result"
    }
    if ($null -ne $State.checkpoint) {
        Assert-ExactProperties -Value $State.checkpoint -Required @(
            "kind", "slot", "reason_code", "entered_at_ms"
        ) -Optional @("process_epoch_sha256") `
            -Context "acceptance state checkpoint"
    }
    foreach ($event in @($State.events)) {
        Assert-ExactProperties -Value $event -Required @(
            "sequence", "phase", "status", "slot", "reason_code",
            "recorded_at_ms"
        ) -Context "acceptance state event"
    }
    if ($null -ne $State.mutation) {
        Assert-ExactProperties -Value $State.mutation -Required @(
            "mutation_id", "kind", "slot", "action_id", "stage", "owner_id",
            "prepared_at_ms", "sent_at_ms", "confirmed_at_ms",
            "reconciliation_code", "target_sha256", "boot_id_sha256",
            "proof_lineage_sha256", "artifact_pin_sha256", "request_id_sha256",
            "cleanup_owner", "previous_journal_sha256", "journal_sha256"
        ) -Optional @("package", "expected_sha256") `
            -Context "acceptance state mutation"
    }
    foreach ($mutation in @($State.mutation_history)) {
        Assert-ExactProperties -Value $mutation -Required @(
            "mutation_id", "kind", "slot", "action_id", "stage", "owner_id",
            "prepared_at_ms", "sent_at_ms", "confirmed_at_ms",
            "reconciliation_code", "target_sha256", "boot_id_sha256",
            "proof_lineage_sha256", "artifact_pin_sha256", "request_id_sha256",
            "cleanup_owner", "previous_journal_sha256", "journal_sha256"
        ) -Optional @("package", "expected_sha256") `
            -Context "acceptance state mutation history"
    }
}

function Read-SanitizedState {
    param([Parameter(Mandatory)][object] $Context)
    $path = Get-StatePath -Context $Context
    Assert-Condition (Test-Path -LiteralPath $path -PathType Leaf) `
        "state_missing" "Preflight has not created acceptance state."
    $state = (Read-StrictJsonFile -Path $path `
            -Context "acceptance state" -MaximumBytes 1048576).Value
    Assert-Condition (
        [string]$state.schema -ceq $script:StateSchema -and
        [string]$state.config_sha256 -ceq [string]$Context.Sha256 -and
        [string]$state.run_id_hash -ceq
            (Get-BytesSha256 -Bytes ([Text.Encoding]::UTF8.GetBytes(
                [string]$Context.Config.run_id)))
    ) "resume_config_mismatch" `
        "The private run config does not match this resumable state."
    Assert-ValidSanitizedStateShape -State $state
    Assert-MutationJournal -State $state
    return $state
}

function Get-MutationJournalSha256 {
    param([Parameter(Mandatory)][Collections.IDictionary] $Mutation)
    $parts = @(
        "rusty.fleet.wifi-adb-two-quest.mutation-journal.v1",
        [string]$Mutation.mutation_id,
        [string]$Mutation.kind,
        [string]$Mutation.slot,
        [string]$Mutation.action_id,
        [string]$Mutation.stage,
        [string]$Mutation.owner_id,
        [string]$Mutation.target_sha256,
        [string]$Mutation.boot_id_sha256,
        [string]$Mutation.proof_lineage_sha256,
        [string]$Mutation.artifact_pin_sha256,
        [string]$Mutation.request_id_sha256,
        [string]$Mutation.cleanup_owner,
        [string]$Mutation.reconciliation_code,
        [string]$Mutation.previous_journal_sha256,
        [string][long]$Mutation.prepared_at_ms,
        [string][long]$Mutation.sent_at_ms,
        [string][long]$Mutation.confirmed_at_ms,
        [string]$Mutation.package,
        [string]$Mutation.expected_sha256
    )
    return Get-BytesSha256 -Bytes (
        [Text.Encoding]::UTF8.GetBytes(($parts -join "`0")))
}

function Assert-MutationJournal {
    param([Parameter(Mandatory)][Collections.IDictionary] $State)
    $previous = "0" * 64
    foreach ($record in @($State.mutation_history)) {
        Assert-Condition (
            [string]$record.previous_journal_sha256 -ceq $previous -and
            [string]$record.journal_sha256 -ceq
                (Get-MutationJournalSha256 -Mutation $record) -and
            [string]$record.stage -cin @("confirmed", "terminal")
        ) "mutation_journal_invalid" `
            "The durable mutation digest chain is invalid."
        $previous = [string]$record.journal_sha256
    }
    Assert-Condition ([string]$State.journal_head_sha256 -ceq $previous) `
        "mutation_journal_head_invalid" `
        "The durable mutation journal head does not match its chain."
    if ($null -ne $State.mutation) {
        Assert-Condition (
            [string]$State.mutation.previous_journal_sha256 -ceq $previous -and
            [string]$State.mutation.journal_sha256 -ceq
                (Get-MutationJournalSha256 -Mutation $State.mutation) -and
            [string]$State.mutation.stage -cin @(
                "prepared_not_sent", "sent_outcome_unknown", "confirmed",
                "cleanup_required", "terminal")
        ) "mutation_journal_active_invalid" `
            "The active durable mutation record is invalid."
    }
}

function Start-DurableMutation {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][string] $Kind,
        [Parameter(Mandatory)][string] $ActionId,
        [string] $Slot = "none",
        [string] $OwnerId = "runner",
        [string] $ArtifactPinSha256 = "",
        [string] $BootIdSha256 = "",
        [string] $ProofLineageSha256 = "",
        [string] $CleanupOwner = "runner",
        [string] $Package = "",
        [string] $ExpectedSha256 = "",
        [string] $RequestId = ""
    )
    Assert-Condition ($null -eq $State.mutation) "mutation_already_active" `
        "A durable mutation must be reconciled before another can start."
    $ordinal = @($State.mutation_history).Count + 1
    $requestMaterial = "$($Context.Sha256)`0$Kind`0$Slot`0$ordinal"
    $mutationHash = Get-BytesSha256 -Bytes (
        [Text.Encoding]::UTF8.GetBytes($requestMaterial))
    if (-not $RequestId) {
        $RequestId = "mutation-" + $mutationHash.Substring(0, 32)
    }
    $requestHash = Get-BytesSha256 -Bytes (
        [Text.Encoding]::UTF8.GetBytes($RequestId))
    $targetHash = if ($Slot -cin @("device_a", "device_b")) {
        $device = Get-DeviceBySlot -Context $Context -Slot $Slot
        Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes([string]$device.device_id))
    } else {
        "0" * 64
    }
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $record = [ordered]@{
        mutation_id = "mutation-" + $mutationHash.Substring(0, 32)
        kind = $Kind
        slot = $Slot
        action_id = $ActionId
        stage = "prepared_not_sent"
        owner_id = $OwnerId
        prepared_at_ms = $now
        sent_at_ms = 0
        confirmed_at_ms = 0
        reconciliation_code = "prepared_durable"
        target_sha256 = $targetHash
        boot_id_sha256 = if ($BootIdSha256) {
            $BootIdSha256
        } else { "0" * 64 }
        proof_lineage_sha256 = if ($ProofLineageSha256) {
            $ProofLineageSha256
        } else { "0" * 64 }
        artifact_pin_sha256 = if ($ArtifactPinSha256) {
            $ArtifactPinSha256
        } else { "0" * 64 }
        request_id_sha256 = $requestHash
        cleanup_owner = $CleanupOwner
        previous_journal_sha256 = [string]$State.journal_head_sha256
        journal_sha256 = ""
        package = $Package
        expected_sha256 = $ExpectedSha256
    }
    $record.journal_sha256 = Get-MutationJournalSha256 -Mutation $record
    $State.mutation = $record
    Write-SanitizedState -Context $Context -State $State
    return [string]$record.mutation_id
}

function Set-DurableMutationSent {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    Assert-Condition (
        $null -ne $State.mutation -and
        [string]$State.mutation.stage -ceq "prepared_not_sent"
    ) "mutation_not_prepared" `
        "A mutation cannot be sent without a durable prepared record."
    $State.mutation.stage = "sent_outcome_unknown"
    $State.mutation.sent_at_ms =
        [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $State.mutation.reconciliation_code = "dispatch_committed_no_ack"
    $State.mutation.journal_sha256 =
        Get-MutationJournalSha256 -Mutation $State.mutation
    Write-SanitizedState -Context $Context -State $State
}

function Complete-DurableMutation {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][string] $ReconciliationCode
    )
    Assert-Condition (
        $null -ne $State.mutation -and
        [string]$State.mutation.stage -ceq "sent_outcome_unknown"
    ) "mutation_outcome_not_unknown" `
        "Only an exact owner readback can confirm a sent mutation."
    $State.mutation.stage = "confirmed"
    $State.mutation.confirmed_at_ms =
        [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $State.mutation.reconciliation_code = $ReconciliationCode
    $State.mutation.journal_sha256 =
        Get-MutationJournalSha256 -Mutation $State.mutation
    Write-SanitizedState -Context $Context -State $State
    $State.mutation_history = @($State.mutation_history) + $State.mutation
    $State.journal_head_sha256 = [string]$State.mutation.journal_sha256
    $State.mutation = $null
    Write-SanitizedState -Context $Context -State $State
}

function Set-DurableMutationCleanupRequired {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][string] $ReasonCode
    )
    Assert-Condition ($null -ne $State.mutation) "mutation_absent" `
        "No durable mutation is available for recovery."
    $State.mutation.stage = "cleanup_required"
    $State.mutation.reconciliation_code = $ReasonCode
    $State.mutation.journal_sha256 =
        Get-MutationJournalSha256 -Mutation $State.mutation
    $State.status = "cleanup_required"
    Write-SanitizedState -Context $Context -State $State
}

function Assert-NoAmbiguousMutation {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    Assert-MutationJournal -State $State
    if ($null -eq $State.mutation) {
        return
    }
    if ([string]$State.mutation.stage -ceq "confirmed") {
        $State.mutation_history = @($State.mutation_history) + $State.mutation
        $State.journal_head_sha256 = [string]$State.mutation.journal_sha256
        $State.mutation = $null
        Write-SanitizedState -Context $Context -State $State
        return
    }
    if ([string]$State.mutation.stage -cin @(
        "prepared_not_sent", "sent_outcome_unknown"
    )) {
        Set-DurableMutationCleanupRequired -Context $Context -State $State `
            -ReasonCode "ambiguous_outcome_no_redispatch"
    }
    throw (New-AcceptanceError "mutation_cleanup_required" `
        "An interrupted owner mutation will not be redispatched; run Cleanup.")
}

function Set-FinalReceiptDigest {
    param(
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][string] $Disposition
    )
    $deviceEvidence = @($State.devices | ForEach-Object {
        [ordered]@{
            slot = [string]$_.slot
            proof_revision = [long]$_.acceptance.proof_revision
            renewed_proof_revision =
                [long]$_.acceptance.renewed_proof_revision
            reboot_loss_observed =
                $_.acceptance.reboot_loss_observed -eq $true
            recovery_observed = $_.acceptance.recovery_observed -eq $true
        }
    }) | ConvertTo-Json -Depth 8 -Compress
    $material = @(
        "rusty.fleet.wifi-adb-two-quest.final-receipt.v1",
        $Disposition,
        [string]$State.journal_head_sha256,
        $deviceEvidence,
        ($State.cleanup.checks | ConvertTo-Json -Depth 12 -Compress)
    ) -join "`0"
    $State.final_receipt_sha256 = Get-BytesSha256 -Bytes (
        [Text.Encoding]::UTF8.GetBytes($material))
}

function Add-StateEvent {
    param(
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][string] $Phase,
        [Parameter(Mandatory)][string] $Status,
        [string] $Slot = "none",
        [string] $ReasonCode = "none"
    )
    $State.sequence = [int]$State.sequence + 1
    $State.events = @($State.events) + [ordered]@{
        sequence = $State.sequence
        phase = $Phase
        status = $Status
        slot = $Slot
        reason_code = $ReasonCode
        recorded_at_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    }
    if ($State.events.Count -gt 128) {
        $State.events = @($State.events | Select-Object -Last 128)
    }
}

function New-SanitizedState {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][object[]] $Snapshots
    )
    $devices = @()
    foreach ($snapshot in $Snapshots) {
        $devices += [ordered]@{
            slot = $snapshot.slot
            snapshot = [ordered]@{
                usb_ready = $snapshot.usb_ready
                package_set_sha256 = $snapshot.package_set_sha256
                packages = $snapshot.packages
                helper_grants = $snapshot.helper_grants
                kiosk_helper_write_secure_settings_granted =
                    $snapshot.kiosk_helper_write_secure_settings_granted
                qfm_profile_state = $snapshot.qfm_profile_state
                kiosk_direct_link_observation = $snapshot.kiosk_direct_link_observation
                after_boot_enabled = $snapshot.after_boot_enabled
                wifi_setting_enabled = $snapshot.wifi_setting_enabled
                transport_usb_present = $snapshot.transport_usb_present
                signer_checks_complete = $snapshot.signer_checks_complete
                agent_process_present = $snapshot.agent_process_present
                agent_private_inputs_absent =
                    $snapshot.agent_private_inputs_absent
                boot_id_sha256 = $snapshot.boot_id_sha256
                boot_elapsed_milliseconds =
                    $snapshot.boot_elapsed_milliseconds
                termux_process_epoch_sha256 =
                    $snapshot.termux_process_epoch_sha256
            }
            run_owned = [ordered]@{
                qfm_profile_created = $false
                agent_profile_staged = $false
                agent_started = $false
                termux_restart_confirmed = $false
                added_packages = @()
            }
            acceptance = [ordered]@{
                baseline_revision = 0
                proof_revision = 0
                renewed_proof_revision = 0
                termux_usable = $false
                expiry_observed = $false
                disable_observed = $false
                reboot_loss_observed = $false
                recovery_observed = $false
                boot_disable_confirmed = $false
                wireless_disable_confirmed = $false
            }
        }
    }
    $state = [ordered]@{
        schema = $script:StateSchema
        run_id_hash = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes([string]$Context.Config.run_id))
        config_sha256 = $Context.Sha256
        status = "preflighted"
        phase = "preflight"
        sequence = 0
        checkpoint = $null
        mutation = $null
        mutation_history = @()
        journal_head_sha256 = "0" * 64
        final_receipt_sha256 = "0" * 64
        devices = $devices
        hub = [ordered]@{
            started_by_run = $false
            process_id = 0
            firewall_created = $false
            two_fresh_baseline_checkins = $false
        }
        onboarding = [ordered]@{
            apply_attempted_by_run = $false
            applied_by_run = $false
            distinct_profiles_verified = $true
        }
        cleanup = [ordered]@{
            attempted = $false
            status = "not_started"
            checks = [ordered]@{}
        }
        claims = [ordered]@{
            planned_only = $false
            installed = $false
            reachable = $false
            authorized = $false
            effective = $false
        }
        events = @()
    }
    Add-StateEvent -State $state -Phase "preflight" -Status "passed"
    return $state
}

function Test-PackagePresent {
    param(
        [Parameter(Mandatory)][string] $PackageList,
        [Parameter(Mandatory)][string] $Package
    )
    return @($PackageList -split "`r?`n" | Where-Object {
        $_ -ceq "package:$Package"
    }).Count -eq 1
}

function Get-PermissionGrant {
    param(
        [Parameter(Mandatory)][string] $PackageDump,
        [Parameter(Mandatory)][string] $Permission
    )
    $pattern = "(?m)^\s*" + [regex]::Escape($Permission) +
        ":\s+granted=(true|false)\s*$"
    $match = [regex]::Match($PackageDump, $pattern)
    return $match.Success -and $match.Groups[1].Value -ceq "true"
}

function Get-DeviceBootIdentity {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device
    )
    $bootId = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "cat", "/proc/sys/kernel/random/boot_id")
    $boot = $bootId.Stdout.Trim().ToLowerInvariant()
    Assert-Condition ($boot -cmatch
        '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$') `
        "boot_identity_invalid" "Quest returned no bounded boot identity."
    $uptimeResult = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "cat", "/proc/uptime")
    $uptimeText = ($uptimeResult.Stdout.Trim() -split '\s+')[0]
    $uptime = 0.0
    Assert-Condition ([double]::TryParse(
            $uptimeText,
            [Globalization.NumberStyles]::AllowDecimalPoint,
            [Globalization.CultureInfo]::InvariantCulture,
            [ref]$uptime) -and $uptime -ge 0) `
        "boot_elapsed_invalid" "Quest returned no bounded boot elapsed time."
    return [pscustomobject]@{
        BootIdSha256 = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes($boot))
        ElapsedMilliseconds = [long][Math]::Floor($uptime * 1000)
    }
}

function Get-TermuxProcessEpochSha256 {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device
    )
    $pidResult = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "pidof", $script:TermuxPackage) `
        -AllowedExitCodes @(0, 1)
    $pids = @($pidResult.Stdout.Trim() -split '\s+' |
        Where-Object { $_ -cmatch '^[1-9][0-9]*$' })
    if ($pids.Count -ne 1) {
        return ("0" * 64)
    }
    $stat = Invoke-AdbExact -Context $Context -Device $Device -Arguments @(
        "shell", "cat", "/proc/$($pids[0])/stat"
    ) -AllowedExitCodes @(0, 1)
    if ($stat.ExitCode -ne 0 -or [string]::IsNullOrWhiteSpace($stat.Stdout)) {
        return ("0" * 64)
    }
    return Get-BytesSha256 -Bytes (
        [Text.Encoding]::UTF8.GetBytes($stat.Stdout.Trim()))
}

function Get-DeviceSnapshot {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device
    )
    $state = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("get-state")
    Assert-Condition ($state.Stdout.Trim() -ceq "device") "usb_device_not_ready" `
        "One configured Quest is not ready on its exact USB serial."
    $packagesResult = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "pm", "list", "packages")
    $packagesText = $packagesResult.Stdout.Trim()
    $packagePresence = [ordered]@{}
    foreach ($package in $script:ManagedPackages) {
        $packagePresence[$package] = Test-PackagePresent `
            -PackageList $packagesText -Package $package
    }

    $helperDump = ""
    if ($packagePresence[$script:HelperPackage]) {
        $helperDumpResult = Invoke-AdbExact -Context $Context -Device $Device `
            -Arguments @("shell", "dumpsys", "package", $script:HelperPackage)
        $helperDump = $helperDumpResult.Stdout
    }
    $grants = [ordered]@{}
    foreach ($permission in $script:HelperPermissions) {
        $grants[$permission] = if ($helperDump) {
            Get-PermissionGrant -PackageDump $helperDump -Permission $permission
        } else {
            $false
        }
    }
    $kioskHelperWriteSecureSettingsGrant = $false
    if ($packagePresence[$script:KioskHelperPackage]) {
        $kioskHelperDump = Invoke-AdbExact -Context $Context -Device $Device `
            -Arguments @(
                "shell", "dumpsys", "package", $script:KioskHelperPackage)
        $kioskHelperWriteSecureSettingsGrant = Get-PermissionGrant `
            -PackageDump $kioskHelperDump.Stdout `
            -Permission $script:WriteSecureSettingsPermission
    }

    $wifi = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "settings", "get", "global", "adb_wifi_enabled")
    $wifiValue = $wifi.Stdout.Trim()
    Assert-Condition ($wifiValue -cin @("0", "1", "null", "")) `
        "wifi_setting_readback_invalid" `
        "Wireless Debugging setting readback was not bounded."

    $helperStatus = $null
    if ($packagePresence[$script:HelperPackage]) {
        $helperStatus = Invoke-HelperExact -Context $Context -Device $Device `
            -HelperAction status
        Assert-Condition (
            [string]$helperStatus.schema -ceq
                "quest-termux-lab.wireless-adb-operator-receipt.v1"
        ) "helper_status_schema_invalid" "Helper status returned an unexpected schema."
    }

    $qfmProfile = Invoke-QfmExact -Context $Context -Arguments @(
        "connectivity-profile",
        "status",
        "--device-id", [string]$Device.device_id,
        "--json"
    )
    $profileState = [string]$qfmProfile.state
    Assert-Condition ($profileState -cin @("enrolled", "absent", "invalid")) `
        "qfm_profile_status_invalid" "QFM returned an unsupported profile state."

    $kioskStatus = Invoke-QfmExact -Context $Context -Arguments @(
        "kiosk",
        "status",
        "--serial", [string]$Device.usb_serial,
        "--adb", $Context.Artifacts["adb"].Path,
        "--json"
    )
    Assert-Condition ($null -ne $kioskStatus.installation) `
        "kiosk_status_invalid" "QFM returned no Kiosk installation observation."
    $directObservation = "not_yet_observed"

    $agentProcess = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "pidof", $script:FleetAgentPackage) `
        -AllowedExitCodes @(0, 1)
    $agentPrivateInputsAbsent = $true
    if ($packagePresence[$script:FleetAgentPackage]) {
        $privateInputs = Invoke-AdbExact -Context $Context -Device $Device `
            -Arguments @(
                "shell", "run-as", $script:FleetAgentPackage,
                "sh", "-c",
                "test ! -e files/fleet-agent"
            ) -AllowedExitCodes @(0, 1)
        $agentPrivateInputsAbsent = $privateInputs.ExitCode -eq 0
    }
    $bootIdentity = Get-DeviceBootIdentity -Context $Context -Device $Device
    $termuxProcessEpochSha256 = Get-TermuxProcessEpochSha256 `
        -Context $Context -Device $Device

    # Signer values are compared only in memory by the APK owner. No signer
    # material enters the sanitized state.
    foreach ($artifactId in @(
        "fleet-agent-apk",
        "wireless-adb-helper-apk",
        "termux-apk",
        "kiosk-setup-helper-apk"
    )) {
        $inspection = Invoke-QfmExact -Context $Context -Arguments @(
            "apk",
            "inspect",
            "--file", $Context.Artifacts[$artifactId].Path,
            "--json"
        )
        Assert-Condition ($null -ne $inspection.identity -and
            [string]$inspection.sha256 -ceq $Context.Artifacts[$artifactId].Sha256) `
            "apk_inspection_mismatch" "QFM APK inspection did not match its exact pin."
    }

    return [pscustomobject]@{
        slot = [string]$Device.slot
        usb_ready = $true
        package_set_sha256 = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes(
                (($packagesText -split "`r?`n" | Sort-Object) -join "`n")))
        packages = $packagePresence
        helper_grants = $grants
        kiosk_helper_write_secure_settings_granted =
            $kioskHelperWriteSecureSettingsGrant
        qfm_profile_state = $profileState
        kiosk_direct_link_observation = $directObservation
        after_boot_enabled = if ($null -ne $helperStatus) {
            [bool]$helperStatus.boot_attempt_enabled
        } else {
            $false
        }
        wifi_setting_enabled = $wifiValue -ceq "1"
        transport_usb_present = $true
        signer_checks_complete = $true
        agent_process_present = -not [string]::IsNullOrWhiteSpace(
            $agentProcess.Stdout)
        agent_private_inputs_absent = $agentPrivateInputsAbsent
        boot_id_sha256 = $bootIdentity.BootIdSha256
        boot_elapsed_milliseconds = $bootIdentity.ElapsedMilliseconds
        termux_process_epoch_sha256 = $termuxProcessEpochSha256
    }
}

function Get-SanitizedPlan {
    param([Parameter(Mandatory)][object] $Context)
    return [ordered]@{
        schema = "rusty.fleet.wifi_adb_two_quest_plan.v1"
        status = "ready"
        config_sha256 = $Context.Sha256
        source_contracts = [ordered]@{
            questionable_file_manager = "exact_reviewed_commit"
            wireless_adb_helper = "exact_reviewed_commit"
        }
        artifact_pins = @($script:ArtifactIds | ForEach-Object {
            [ordered]@{
                artifact_id = $_
                sha256_verified = $true
            }
        })
        devices = @(
            [ordered]@{ slot = "device_a"; private_bindings_verified = $true },
            [ordered]@{ slot = "device_b"; private_bindings_verified = $true }
        )
        next_phase = "preflight"
        claims = [ordered]@{
            installed = $false
            reachable = $false
            authorized = $false
            effective = $false
        }
    }
}

function Get-OnboardingGeneratedPaths {
    param([Parameter(Mandatory)][object] $Context)
    $paths = @(
        [string]$Context.Config.onboarding.inventory_path,
        [string]$Context.Config.hub.config_path
    )
    foreach ($device in $Context.Config.devices) {
        $paths += @(
            [string]$device.fleet_agent_profile_path,
            [string]$device.fleet_agent_seed_path
        )
    }
    return $paths
}

function Assert-OnboardingOutputsAbsent {
    param([Parameter(Mandatory)][object] $Context)
    foreach ($generatedPath in Get-OnboardingGeneratedPaths -Context $Context) {
        Assert-Condition (-not (Test-Path -LiteralPath $generatedPath)) `
            "onboarding_output_preexists" `
            "The transaction requires fresh, absent offline-onboarding outputs."
    }
}

function Invoke-Preflight {
    param([Parameter(Mandatory)][object] $Context)
    $root = [IO.Path]::GetFullPath([string]$Context.Config.private_state_root)
    Assert-Condition (-not (Test-Path -LiteralPath $root)) `
        "private_state_root_exists" `
        "Preflight requires a new private state root and never overwrites a prior run."
    Assert-OnboardingOutputsAbsent -Context $Context

    $snapshots = @()
    foreach ($slot in @("device_a", "device_b")) {
        $device = Get-DeviceBySlot -Context $Context -Slot $slot
        $snapshots += Get-DeviceSnapshot -Context $Context -Device $device
    }
    foreach ($snapshot in $snapshots) {
        Assert-Condition (-not $snapshot.wifi_setting_enabled) `
            "unsafe_initial_wireless_state" `
            "Acceptance requires Wireless Debugging initially disabled for exact cleanup."
        Assert-Condition (-not $snapshot.after_boot_enabled) `
            "unsafe_initial_after_boot_state" `
            "Acceptance requires the after-boot request initially disabled."
        Assert-Condition ([string]$snapshot.qfm_profile_state -ceq "absent") `
            "preexisting_private_profile" `
            "Acceptance fails closed when a Fleet connectivity profile already exists."
        Assert-Condition (-not $snapshot.agent_process_present) `
            "preexisting_fleet_agent_process" `
            "Acceptance never adopts or stops a pre-existing Fleet Agent process."
        Assert-Condition ($snapshot.agent_private_inputs_absent) `
            "preexisting_fleet_agent_private_inputs" `
            "Acceptance never overwrites pre-existing Fleet Agent app-private inputs."
        Assert-Condition ([bool]$snapshot.packages[$script:KioskPackage]) `
            "kiosk_prerequisite_missing" `
            "Acceptance requires the separately distributed Kiosk app."
    }

    New-Item -ItemType Directory -Path $root | Out-Null
    try {
        $acl = Get-Acl -LiteralPath $root
        $acl.SetAccessRuleProtection($true, $false)
        $identity = [Security.Principal.WindowsIdentity]::GetCurrent().Name
        $rule = [Security.AccessControl.FileSystemAccessRule]::new(
            $identity,
            [Security.AccessControl.FileSystemRights]::FullControl,
            [Security.AccessControl.InheritanceFlags]"ContainerInherit, ObjectInherit",
            [Security.AccessControl.PropagationFlags]::None,
            [Security.AccessControl.AccessControlType]::Allow)
        [void]$acl.AddAccessRule($rule)
        Set-Acl -LiteralPath $root -AclObject $acl
        New-Item -ItemType Directory -Path (Join-Path $root "runtime") |
            Out-Null
        $state = New-SanitizedState -Context $Context -Snapshots $snapshots
        Write-SanitizedState -Context $Context -State $state
        return $state
    } catch {
        if (Test-Path -LiteralPath $root -PathType Container) {
            $resolvedRoot = [IO.Path]::GetFullPath($root)
            Assert-Condition ($resolvedRoot -ceq $root) `
                "preflight_cleanup_refused" "Preflight cleanup target drifted."
            Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
        }
        throw
    }
}

function Get-StateDevice {
    param(
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot
    )
    $match = @($State.devices | Where-Object { [string]$_.slot -ceq $Slot })
    Assert-Condition ($match.Count -eq 1) "state_device_missing" `
        "Sanitized state is missing a logical device slot."
    return $match[0]
}

function Set-Checkpoint {
    param(
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][ValidateSet(
            "awaiting_kiosk_direct_link",
            "awaiting_termux_bootstrap",
            "awaiting_termux_restart",
            "awaiting_wearer_approval",
            "awaiting_attended_reboot")]
        [string] $Kind,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot,
        [Parameter(Mandatory)][string] $ReasonCode,
        [string] $ProcessEpochSha256 = ""
    )
    $State.status = "blocked_attended"
    $State.checkpoint = [ordered]@{
        kind = $Kind
        slot = $Slot
        reason_code = $ReasonCode
        entered_at_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    }
    if ($ProcessEpochSha256) {
        $State.checkpoint.process_epoch_sha256 = $ProcessEpochSha256
    }
    Add-StateEvent -State $State -Phase $State.phase `
        -Status "blocked_attended" -Slot $Slot -ReasonCode $ReasonCode
}

function Clear-Checkpoint {
    param([Parameter(Mandatory)][Collections.IDictionary] $State)
    $State.checkpoint = $null
    $State.status = "running"
}

function Install-RunPackage {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $ConfigDevice,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][Collections.IDictionary] $StateDevice,
        [Parameter(Mandatory)][string] $Package,
        [Parameter(Mandatory)][string] $ArtifactId
    )
    if (
        [bool]$StateDevice.snapshot.packages[$Package] -or
        @($StateDevice.run_owned.added_packages) -contains $Package
    ) {
        return
    }
    $bootIdentity = Get-DeviceBootIdentity `
        -Context $Context -Device $ConfigDevice
    [void](Start-DurableMutation -Context $Context -State $State `
        -Kind "qfm-package-install" -ActionId "packages.install-release" `
        -Slot ([string]$ConfigDevice.slot) `
        -OwnerId "questionable-file-manager" `
        -ArtifactPinSha256 $Context.Artifacts[$ArtifactId].Sha256 `
        -BootIdSha256 $bootIdentity.BootIdSha256 `
        -CleanupOwner "android-package-manager" -Package $Package `
        -ExpectedSha256 $Context.Artifacts[$ArtifactId].Sha256)
    Set-DurableMutationSent -Context $Context -State $State
    $receipt = Invoke-QfmExact -Context $Context -Arguments @(
        "apk",
        "install",
        "--serial", [string]$ConfigDevice.usb_serial,
        "--file", $Context.Artifacts[$ArtifactId].Path,
        "--adb", $Context.Artifacts["adb"].Path,
        "--json"
    ) -TimeoutSeconds 180
    Assert-Condition ($null -ne $receipt.result -or $null -ne $receipt.mutation) `
        "package_install_receipt_invalid" `
        "QFM did not return an owner installation receipt."
    $installed = Invoke-AdbExact -Context $Context -Device $ConfigDevice `
        -Arguments @("shell", "pm", "list", "packages", $Package)
    Assert-Condition (
        Test-PackagePresent -PackageList $installed.Stdout -Package $Package
    ) "package_install_readback_missing" `
        "The exact package manager readback did not confirm the installed package."
    $StateDevice.run_owned.added_packages =
        @($StateDevice.run_owned.added_packages) + $Package
    Complete-DurableMutation -Context $Context -State $State `
        -ReconciliationCode "qfm_receipt_and_package_readback"
}

function Set-FixedPackagePermission {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Collections.IDictionary] $State,
        [Parameter(Mandatory)][Collections.IDictionary] $Device,
        [Parameter(Mandatory)][ValidateSet(
            "org.questtermuxlab.wirelessadbrecovery",
            "io.github.mesmerprism.rustykiosk.setuphelper")]
        [string] $Package,
        [Parameter(Mandatory)][string] $Permission,
        [Parameter(Mandatory)][bool] $Granted
    )
    $verb = if ($Granted) { "grant" } else { "revoke" }
    if ($null -ne $State) {
        $bootIdentity = Get-DeviceBootIdentity -Context $Context -Device $Device
        [void](Start-DurableMutation -Context $Context -State $State `
            -Kind "android-permission-$verb" `
            -ActionId "android.permission.$verb" `
            -Slot ([string]$Device.slot) -OwnerId "android-package-manager" `
            -ArtifactPinSha256 $Context.Artifacts["adb"].Sha256 `
            -BootIdSha256 $bootIdentity.BootIdSha256 `
            -CleanupOwner "android-package-manager" `
            -ExpectedSha256 (Get-BytesSha256 -Bytes (
                [Text.Encoding]::UTF8.GetBytes("$Package`0$Permission`0$Granted"))))
        Set-DurableMutationSent -Context $Context -State $State
    }
    [void](Invoke-AdbExact -Context $Context -Device $Device -Arguments @(
        "shell", "pm", $verb, $Package, $Permission
    ))
    $dump = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "dumpsys", "package", $Package)
    Assert-Condition (
        (Get-PermissionGrant -PackageDump $dump.Stdout `
            -Permission $Permission) -eq $Granted
    ) "permission_readback_mismatch" `
        "The exact package permission readback did not match the requested state."
    if ($null -ne $State) {
        Complete-DurableMutation -Context $Context -State $State `
            -ReconciliationCode "package_permission_exact_readback"
    }
}

function Write-FleetAgentPrivateInput {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device,
        [Parameter(Mandatory)][string] $SourcePath,
        [Parameter(Mandatory)][ValidateSet(
            "files/fleet-agent/profile.json",
            "files/fleet-agent/signing-seed.bin")]
        [string] $Destination
    )
    $bytes = [IO.File]::ReadAllBytes($SourcePath)
    $base64 = [Convert]::ToBase64String($bytes)
    try {
        $fixedCommand = "run-as $($script:FleetAgentPackage) sh -c " +
            "'base64 -d > $Destination'"
        [void](Invoke-AdbExact -Context $Context -Device $Device `
            -Arguments @("shell", "-T", $fixedCommand) `
            -StandardInput $base64)
        $hash = Invoke-AdbExact -Context $Context -Device $Device -Arguments @(
            "exec-out",
            "run-as", $script:FleetAgentPackage,
            "sha256sum", $Destination
        )
        $match = [regex]::Match($hash.Stdout.Trim(), '^([0-9a-fA-F]{64})\s+\S+$')
        Assert-Condition (
            $match.Success -and
            $match.Groups[1].Value.ToLowerInvariant() -ceq
                (Get-BytesSha256 -Bytes $bytes)
        ) "fleet_agent_stage_hash_mismatch" `
            "Fleet Agent app-private input did not match its private source."
    } finally {
        [Array]::Clear($bytes, 0, $bytes.Length)
        $base64 = $null
    }
}

function Test-FleetAgentPrivateInputsExact {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device
    )
    foreach ($binding in @(
        @{
            Source = [string]$Device.fleet_agent_profile_path
            Destination = "files/fleet-agent/profile.json"
        },
        @{
            Source = [string]$Device.fleet_agent_seed_path
            Destination = "files/fleet-agent/signing-seed.bin"
        }
    )) {
        $hash = Invoke-AdbExact -Context $Context -Device $Device -Arguments @(
            "exec-out", "run-as", $script:FleetAgentPackage,
            "sha256sum", [string]$binding.Destination
        ) -AllowedExitCodes @(0, 1)
        $match = [regex]::Match(
            $hash.Stdout.Trim(), '^([0-9a-fA-F]{64})\s+\S+$')
        if (-not $match.Success -or
            $match.Groups[1].Value.ToLowerInvariant() -cne
                (Get-Sha256 -Path ([string]$binding.Source))) {
            return $false
        }
    }
    $modes = Invoke-AdbExact -Context $Context -Device $Device -Arguments @(
        "shell", "run-as", $script:FleetAgentPackage,
        "stat", "-c", "%a:%n",
        "files/fleet-agent",
        "files/fleet-agent/profile.json",
        "files/fleet-agent/signing-seed.bin"
    ) -AllowedExitCodes @(0, 1)
    return $modes.ExitCode -eq 0 -and
        $modes.Stdout.Contains(
            "700:files/fleet-agent", [StringComparison]::Ordinal) -and
        $modes.Stdout.Contains(
            "600:files/fleet-agent/profile.json",
            [StringComparison]::Ordinal) -and
        $modes.Stdout.Contains(
            "600:files/fleet-agent/signing-seed.bin",
            [StringComparison]::Ordinal)
}

function Start-FleetAgent {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device
    )
    [void](Invoke-AdbExact -Context $Context -Device $Device -Arguments @(
        "shell", "am", "start",
        "-a", "io.github.mesmerprism.rustyquest.fleetagent.DEBUG_START",
        "-n", $script:FleetAgentActivity
    ))
}

function Stop-FleetAgent {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device
    )
    [void](Invoke-AdbExact -Context $Context -Device $Device -Arguments @(
        "shell", "am", "start",
        "-a", "io.github.mesmerprism.rustyquest.fleetagent.DEBUG_STOP",
        "-n", $script:FleetAgentActivity
    ) -AllowedExitCodes @(0, 1))
}

function Start-FleetHub {
    param([Parameter(Mandatory)][object] $Context)
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Context.Artifacts["fleet-hub"].Path
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.ArgumentList.Add("--config")
    $start.ArgumentList.Add([string]$Context.Config.hub.config_path)
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    Assert-Condition $process.Start() "hub_start_failed" `
        "The pinned Fleet Hub could not be started."
    Start-Sleep -Milliseconds 750
    Assert-Condition (-not $process.HasExited) "hub_start_failed" `
        "The pinned Fleet Hub exited before becoming ready."
    $pid = $process.Id
    $process.Dispose()
    return $pid
}

function New-RunFirewallRule {
    param([Parameter(Mandatory)][object] $Context)
    if (-not [bool]$Context.Config.hub.manage_firewall) {
        return $false
    }
    $hubConfig = Read-StrictJsonFile -Path $Context.Config.hub.config_path `
        -Context "Hub config"
    $checkinBind = [string]$hubConfig.Value.checkin_bind
    $match = [regex]::Match($checkinBind, '^[^:]+:([1-9][0-9]{0,4})$')
    Assert-Condition ($match.Success -and [int]$match.Groups[1].Value -le 65535) `
        "hub_checkin_bind_invalid" `
        "The private Hub config has no exact IPv4 check-in listener."
    $name = "RustyFleet-WifiAdb-" + $Context.Sha256.Substring(0, 12)
    Assert-Condition ($null -eq (Get-NetFirewallRule -DisplayName $name `
            -ErrorAction SilentlyContinue)) "firewall_rule_exists" `
        "The exact run-owned firewall rule already exists."
    New-NetFirewallRule -DisplayName $name -Direction Inbound -Action Allow `
        -Protocol TCP -LocalPort ([int]$match.Groups[1].Value) `
        -Profile Private | Out-Null
    return $true
}

function Remove-RunFirewallRule {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    if (-not [bool]$State.hub.firewall_created) {
        return $true
    }
    $name = "RustyFleet-WifiAdb-" + $Context.Sha256.Substring(0, 12)
    $rule = Get-NetFirewallRule -DisplayName $name -ErrorAction SilentlyContinue
    if ($null -ne $rule) {
        $rule | Remove-NetFirewallRule
    }
    return $null -eq (Get-NetFirewallRule -DisplayName $name `
        -ErrorAction SilentlyContinue)
}

function Stop-RunOwnedHub {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    if (-not [bool]$State.hub.started_by_run -or [int]$State.hub.process_id -le 0) {
        return $true
    }
    $process = Get-Process -Id ([int]$State.hub.process_id) -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        return $true
    }
    try {
        $observedPath = $process.Path
        Assert-Condition ($observedPath -and
            (Get-Sha256 -Path $observedPath) -ceq
                $Context.Artifacts["fleet-hub"].Sha256) `
            "hub_process_identity_mismatch" `
            "The recorded Hub PID no longer belongs to the pinned Fleet Hub."
        $process.Kill($true)
        return $process.WaitForExit(5000)
    } finally {
        $process.Dispose()
    }
}

function Wait-FleetInspect {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device,
        [Parameter(Mandatory)][scriptblock] $Predicate,
        [Parameter(Mandatory)][int] $TimeoutSeconds,
        [Parameter(Mandatory)][string] $FailureCode
    )
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        try {
            $projection = Invoke-FleetCtlExact -Context $Context -Arguments @(
                "inspect", [string]$Device.device_id
            )
            if (& $Predicate $projection) {
                return $projection
            }
        } catch {
            # A bounded not-yet-ready state is retried until the typed deadline.
        }
        Start-Sleep -Milliseconds 750
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw (New-AcceptanceError $FailureCode `
        "Fleet did not report the required signed state before the deadline.")
}

function Invoke-WifiOperation {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][Collections.IDictionary] $Device,
        [Parameter(Mandatory)][ValidateSet(
            "status",
            "request-wireless-adb",
            "disable-wireless-adb")]
        [string] $WifiAction
    )
    $preview = Invoke-FleetCtlExact -Context $Context -Arguments @(
        "wifi-adb-preview",
        $WifiAction,
        "$([string]$Device.device_id)@$([long]$Device.identity_revision)"
    )
    Assert-Condition (
        [string]$preview.schema -ceq "rusty.fleet.quest_wifi_adb_operation.v1" -and
        [string]$preview.lifecycle -ceq "proposed" -and
        $preview.targets.Count -eq 1 -and
        $preview.preview.targets.Count -eq 1 -and
        [string]$preview.targets[0].device_id -ceq [string]$Device.device_id -and
        [long]$preview.targets[0].identity_revision -eq
            [long]$Device.identity_revision -and
        [string]$preview.preview.targets[0].device_id -ceq
            [string]$Device.device_id -and
        [long]$preview.preview.targets[0].identity_revision -eq
            [long]$Device.identity_revision
    ) "wifi_preview_invalid" "Fleet did not return one exact proposed operation."
    $bootIdentity = Get-DeviceBootIdentity -Context $Context -Device $Device
    $proofLineage = "0" * 64
    $stateDevice = Get-StateDevice -State $State -Slot ([string]$Device.slot)
    if ($stateDevice.acceptance.Contains("operation_id")) {
        $prior = Get-WifiOperation -Context $Context `
            -OperationId ([string]$stateDevice.acceptance.operation_id)
        if ($null -ne $prior.targets[0].termux_admission) {
            $proofLineage =
                [string]$prior.targets[0].termux_admission.lineage_sha256
        }
    }
    $operationIdSha256 = Get-BytesSha256 -Bytes (
        [Text.Encoding]::UTF8.GetBytes([string]$preview.operation_id))
    [void](Start-DurableMutation -Context $Context -State $State `
        -Kind "fleet-wifi-adb-operation" `
        -ActionId "quest.wifi-adb-control.$WifiAction" `
        -Slot ([string]$Device.slot) -OwnerId "fleet-hub" `
        -ArtifactPinSha256 $Context.Artifacts["fleetctl"].Sha256 `
        -BootIdSha256 $bootIdentity.BootIdSha256 `
        -ProofLineageSha256 $proofLineage `
        -CleanupOwner "questionable-file-manager" `
        -ExpectedSha256 $operationIdSha256)
    Set-DurableMutationSent -Context $Context -State $State
    $executed = Invoke-FleetCtlExact -Context $Context -Arguments @(
        "wifi-adb-execute",
        [string]$preview.operation_id,
        [string]$preview.preview.preview_id
    )
    Assert-Condition (
        [string]$executed.operation_id -ceq [string]$preview.operation_id -and
        $executed.targets.Count -eq 1 -and
        [string]$executed.targets[0].device_id -ceq [string]$Device.device_id -and
        [long]$executed.targets[0].identity_revision -eq
            [long]$Device.identity_revision
    ) "wifi_execute_invalid" "Fleet execution did not bind the exact preview."
    Assert-Condition (
        $null -ne $executed.targets[0].receipt -and
        [string]$executed.targets[0].receipt.operation_id -ceq
            [string]$preview.operation_id -and
        [string]$executed.targets[0].receipt.device_id -ceq
            [string]$Device.device_id
    ) "wifi_owner_receipt_invalid" `
        "Fleet did not retain the exact provider receipt."
    if ($WifiAction -ceq "status") {
        $stateDevice.acceptance.status_operation_id =
            [string]$executed.operation_id
    } elseif ($WifiAction -ceq "request-wireless-adb") {
        $stateDevice.acceptance.operation_id =
            [string]$executed.operation_id
    } elseif ($WifiAction -ceq "disable-wireless-adb") {
        $stateDevice.acceptance.disable_operation_id =
            [string]$executed.operation_id
    }
    Complete-DurableMutation -Context $Context -State $State `
        -ReconciliationCode "fleet_owner_receipt_exact_readback"
    return $executed
}

function Get-WifiOperation {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][string] $OperationId
    )
    return Invoke-FleetCtlExact -Context $Context -Arguments @(
        "wifi-adb-get", $OperationId
    )
}

function Get-SignedIsolationProjection {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot
    )
    $device = Get-DeviceBySlot -Context $Context -Slot $Slot
    $inspect = Invoke-FleetCtlExact -Context $Context -Arguments @(
        "inspect", [string]$device.device_id
    )
    Assert-Condition (
        [string]$inspect.row.identity.device_id -ceq [string]$device.device_id -and
        [long]$inspect.row.identity.identity_revision -eq
            [long]$device.identity_revision -and
        [string]$inspect.row.freshness -ceq "fresh" -and
        [long]$inspect.row.accepted_revision -gt 0 -and
        -not [string]::IsNullOrWhiteSpace([string]$inspect.row.source_epoch)
    ) "isolation_signed_projection_invalid" `
        "Isolation checks require a fresh signed exact-device projection."
    $lineage = @()
    $operations = Invoke-FleetCtlExact -Context $Context -Arguments @(
        "wifi-adb-list"
    )
    foreach ($operation in @($operations)) {
        foreach ($target in @($operation.targets | Where-Object {
            [string]$_.device_id -ceq [string]$device.device_id
        })) {
            $lineage += [ordered]@{
                operation_id = [string]$operation.operation_id
                action = [string]$operation.preview.action
                lifecycle = [string]$target.lifecycle
                receipt_request_id = [string]$target.receipt.request_id
                receipt_evidence_sha256 = [string]$target.receipt.evidence_sha256
                receipt_effect_applied = $target.receipt.effect_applied -eq $true
                proof_id = [string]$target.termux_proof.proof_id
                proof_source_revision = [long]$target.termux_proof.source_revision
                proof_evidence_revision = [long]$target.termux_proof.evidence_revision
                admission_lineage_sha256 =
                    [string]$target.termux_admission.lineage_sha256
                termux_usable = $target.termux_usable -eq $true
            }
        }
    }
    $lineageJson = @($lineage | Sort-Object operation_id) |
        ConvertTo-Json -Depth 12 -Compress
    $packages = Invoke-AdbExact -Context $Context -Device $device `
        -Arguments @("shell", "pm", "list", "packages")
    $packageFacts = [ordered]@{}
    $processFacts = [ordered]@{}
    foreach ($package in $script:ManagedPackages) {
        $packageFacts[$package] = Test-PackagePresent `
            -PackageList $packages.Stdout -Package $package
        $pid = Invoke-AdbExact -Context $Context -Device $device `
            -Arguments @("shell", "pidof", $package) `
            -AllowedExitCodes @(0, 1)
        $processFacts[$package] =
            -not [string]::IsNullOrWhiteSpace($pid.Stdout)
    }
    $wireless = Invoke-AdbExact -Context $Context -Device $device `
        -Arguments @("shell", "settings", "get", "global", "adb_wifi_enabled")
    $transport = Invoke-AdbExact -Context $Context -Device $device `
        -Arguments @("get-state")
    $physicalJson = [ordered]@{
        packages = $packageFacts
        processes = $processFacts
        wireless_setting = $wireless.Stdout.Trim()
        usb_transport = $transport.Stdout.Trim()
    } | ConvertTo-Json -Depth 8 -Compress
    return [pscustomobject]@{
        AcceptedRevision = [long]$inspect.row.accepted_revision
        SourceEpochSha256 = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes([string]$inspect.row.source_epoch))
        OperationLineageSha256 = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes($lineageJson))
        OperationCount = $lineage.Count
        PhysicalLineageSha256 = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes($physicalJson))
    }
}

function Assert-NonTargetIsolation {
    param(
        [Parameter(Mandatory)][object] $Before,
        [Parameter(Mandatory)][object] $After
    )
    Assert-Condition (
        $After.AcceptedRevision -ge $Before.AcceptedRevision -and
        $After.SourceEpochSha256 -ceq $Before.SourceEpochSha256 -and
        $After.OperationCount -eq $Before.OperationCount -and
        $After.OperationLineageSha256 -ceq
            $Before.OperationLineageSha256 -and
        $After.PhysicalLineageSha256 -ceq
            $Before.PhysicalLineageSha256
    ) "cross_device_lineage_changed" `
        "A non-target device changed operation, effect, proof, revision, receipt, or source lineage."
}

function Wait-WifiProof {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device,
        [Parameter(Mandatory)][string] $OperationId,
        [long] $MinimumEvidenceRevision = 0
    )
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(
        [int]$Context.Config.timing.proof_timeout_seconds)
    do {
        $operation = Get-WifiOperation -Context $Context -OperationId $OperationId
        Assert-Condition ($operation.targets.Count -eq 1 -and
            [string]$operation.targets[0].device_id -ceq [string]$Device.device_id) `
            "wifi_operation_device_mismatch" `
            "Fleet returned an operation for the wrong device."
        $now = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $proofResult = Test-HubTermuxAdmission `
            -Operation $operation -ExpectedOperationId $OperationId `
            -ExpectedDeviceId ([string]$Device.device_id) `
            -ExpectedIdentityRevision ([long]$Device.identity_revision) `
            -NowMs $now -MinimumEvidenceRevision $MinimumEvidenceRevision
        if ($proofResult.Valid -and $operation.targets[0].termux_usable -eq $true) {
            return $operation
        }
        Start-Sleep -Milliseconds 750
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw (New-AcceptanceError "termux_proof_timeout" `
        "No fresh signed exact-shell owner proof reached Fleet before the deadline.")
}

function Wait-WifiProofAbsent {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][string] $OperationId
    )
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(
        [int]$Context.Config.timing.expiry_timeout_seconds)
    do {
        $operation = Get-WifiOperation -Context $Context -OperationId $OperationId
        if ($operation.targets.Count -eq 1 -and
            $operation.targets[0].termux_usable -eq $false -and
            $null -eq $operation.targets[0].termux_proof) {
            return $operation
        }
        Start-Sleep -Milliseconds 1000
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw (New-AcceptanceError "termux_proof_did_not_expire" `
        "Fleet did not clear the short-lived owner proof before the deadline.")
}

function Assert-GeneratedFleetInputs {
    param([Parameter(Mandatory)][object] $Context)
    Assert-Condition (
        Test-Path -LiteralPath $Context.Config.onboarding.inventory_path -PathType Leaf
    ) "onboarding_inventory_missing" `
        "Offline onboarding did not produce its private inventory commit marker."
    $seedHashes = @()
    foreach ($device in $Context.Config.devices) {
        $profile = Read-StrictJsonFile -Path $device.fleet_agent_profile_path `
            -Context "generated Fleet Agent profile"
        Assert-Condition (
            [string]$profile.Value.schema -ceq
                "rusty.quest.fleet_agent_profile.v1" -and
            $profile.Value.enabled -eq $true -and
            [string]$profile.Value.device_id -ceq [string]$device.device_id -and
            [long]$profile.Value.identity_revision -eq [long]$device.identity_revision
        ) "generated_profile_mismatch" `
            "Generated Fleet Agent profile does not bind its exact device."
        $seed = Resolve-PrivateLeaf -Path $device.fleet_agent_seed_path `
            -Context "generated Fleet Agent seed"
        Assert-Condition ((Get-Item -LiteralPath $seed).Length -eq 32) `
            "generated_seed_invalid" "Generated Fleet Agent seed is not exactly 32 bytes."
        $seedHashes += Get-Sha256 -Path $seed
    }
    Assert-Condition (@($seedHashes | Sort-Object -Unique).Count -eq 2) `
        "generated_seed_duplicate" "Offline onboarding did not create two distinct seeds."
}

function Invoke-OnboardingApply {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    $tool = $Context.Artifacts["fleet-onboard"].Path
    $request = [string]$Context.Config.onboarding.request_path
    $validation = Invoke-JsonOwner -FilePath $tool `
        -Arguments @("validate-tool", "--request", $request) -TimeoutSeconds 30
    Assert-Condition (
        [string]$validation.schema -ceq
            "rusty.fleet.offline_onboarding_tool_validation.v1" -and
        [string]$validation.status -ceq "valid"
    ) "onboarding_tool_invalid" "fleet-onboard rejected its exact pinned tool."
    $plan = Invoke-JsonOwner -FilePath $tool `
        -Arguments @("plan", "--request", $request) -TimeoutSeconds 30
    Assert-Condition ([string]$plan.plan_sha256 -cmatch '^[0-9a-f]{64}$') `
        "onboarding_plan_invalid" "fleet-onboard did not return a canonical plan digest."
    [void](Start-DurableMutation -Context $Context -State $State `
        -Kind "offline-onboarding-apply" `
        -ActionId "fleet.onboarding.apply" -OwnerId "fleet-onboard" `
        -ArtifactPinSha256 $Context.Artifacts["fleet-onboard"].Sha256 `
        -ExpectedSha256 ([string]$plan.plan_sha256) `
        -CleanupOwner "fleet-onboard")
    Set-DurableMutationSent -Context $Context -State $State
    $receipt = Invoke-JsonOwner -FilePath $tool -Arguments @(
        "apply",
        "--request", $request,
        "--confirm-plan-sha256", [string]$plan.plan_sha256,
        "--non-interactive"
    ) -TimeoutSeconds 120
    Assert-Condition (
        [string]$receipt.schema -ceq
            "rusty.fleet.offline_onboarding_apply_receipt.v1" -and
        [string]$receipt.status -ceq "generated" -and
        [string]$receipt.plan_sha256 -ceq [string]$plan.plan_sha256
    ) "onboarding_apply_invalid" `
        "fleet-onboard did not return an exact generated-only receipt."
    Assert-GeneratedFleetInputs -Context $Context
    $State.onboarding.applied_by_run = $true
    Complete-DurableMutation -Context $Context -State $State `
        -ReconciliationCode "generated_outputs_exact"
}

function Invoke-ProfileAndPackageProvision {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    foreach ($slot in @("device_a", "device_b")) {
        $device = Get-DeviceBySlot -Context $Context -Slot $slot
        $stateDevice = Get-StateDevice -State $State -Slot $slot
        if ([string]$stateDevice.snapshot.qfm_profile_state -ceq "invalid") {
            throw (New-AcceptanceError "qfm_profile_initially_invalid" `
                "QFM has an invalid pre-existing profile; the runner will not replace it.")
        }
        if (
            [string]$stateDevice.snapshot.qfm_profile_state -ceq "absent" -and
            -not [bool]$stateDevice.run_owned.qfm_profile_created
        ) {
            $bootIdentity = Get-DeviceBootIdentity `
                -Context $Context -Device $device
            [void](Start-DurableMutation -Context $Context -State $State `
                -Kind "qfm-profile-import" `
                -ActionId "qfm.connectivity-profile.import" -Slot $slot `
                -OwnerId "questionable-file-manager" `
                -ArtifactPinSha256 (
                    $Context.Artifacts["questionable-file-manager"].Sha256) `
                -BootIdSha256 $bootIdentity.BootIdSha256 `
                -CleanupOwner "questionable-file-manager")
            Set-DurableMutationSent -Context $Context -State $State
            $import = Invoke-QfmExact -Context $Context -Arguments @(
                "connectivity-profile",
                "import",
                "--file", [string]$device.qfm_enrollment_path,
                "--confirm-profile-write",
                "--json"
            )
            Assert-Condition ([string]$import.state -ceq "created") `
                "qfm_profile_create_failed" `
                "QFM did not create the exact private connectivity profile."
            $readback = Invoke-QfmExact -Context $Context -Arguments @(
                "connectivity-profile", "status",
                "--device-id", [string]$device.device_id,
                "--json"
            )
            Assert-Condition ([string]$readback.state -ceq "enrolled") `
                "qfm_profile_readback_failed" `
                "QFM did not read back the exact enrolled profile."
            $stateDevice.run_owned.qfm_profile_created = $true
            Complete-DurableMutation -Context $Context -State $State `
                -ReconciliationCode "profile_enrolled_exact_readback"
        }

        Install-RunPackage -Context $Context -ConfigDevice $device `
            -State $State -StateDevice $stateDevice `
            -Package $script:TermuxPackage `
            -ArtifactId "termux-apk"
        Install-RunPackage -Context $Context -ConfigDevice $device `
            -State $State -StateDevice $stateDevice `
            -Package $script:HelperPackage `
            -ArtifactId "wireless-adb-helper-apk"
        Install-RunPackage -Context $Context -ConfigDevice $device `
            -State $State -StateDevice $stateDevice `
            -Package $script:KioskHelperPackage `
            -ArtifactId "kiosk-setup-helper-apk"
        Install-RunPackage -Context $Context -ConfigDevice $device `
            -State $State -StateDevice $stateDevice `
            -Package $script:FleetAgentPackage `
            -ArtifactId "fleet-agent-apk"

        foreach ($permission in $script:HelperPermissions) {
            Set-FixedPackagePermission -Context $Context -Device $device `
                -State $State -Package $script:HelperPackage `
                -Permission $permission -Granted $true
        }
        Set-FixedPackagePermission -Context $Context -Device $device `
            -State $State -Package $script:KioskHelperPackage `
            -Permission $script:WriteSecureSettingsPermission -Granted $true
        $termux = if ([bool]$stateDevice.run_owned.termux_restart_confirmed) {
            Invoke-HelperExact -Context $Context -Device $device `
                -HelperAction "status"
        } else {
            $helperRequestId =
                "fleet-" + [guid]::NewGuid().ToString("N")
            $bootIdentity = Get-DeviceBootIdentity `
                -Context $Context -Device $device
            [void](Start-DurableMutation -Context $Context -State $State `
                -Kind "termux-prerequisites" `
                -ActionId "termux.prepare-prerequisites" -Slot $slot `
                -OwnerId "wireless-adb-helper" `
                -ArtifactPinSha256 (
                    $Context.Artifacts["helper-operator"].Sha256) `
                -BootIdSha256 $bootIdentity.BootIdSha256 `
                -CleanupOwner "wireless-adb-helper" `
                -RequestId $helperRequestId)
            Set-DurableMutationSent -Context $Context -State $State
            $result = Invoke-HelperExact -Context $Context -Device $device `
                -HelperAction "prepare-termux-prerequisites" `
                -Confirm -ConfirmTermuxPackageInstall `
                -RequestId $helperRequestId
            Complete-DurableMutation -Context $Context -State $State `
                -ReconciliationCode "termux_prerequisite_status_readback"
            $result
        }
        if (
            [string]$termux.state -cne "termux_prerequisites_ready" -or
            $termux.termux_allow_external_apps_configured -ne $true -or
            $termux.termux_python_ready -ne $true -or
            $termux.termux_android_tools_ready -ne $true
        ) {
            Set-Checkpoint -State $State -Kind "awaiting_termux_bootstrap" `
                -Slot $slot -ReasonCode "termux_prerequisites_incomplete"
            return $false
        }
        if (-not [bool]$stateDevice.run_owned.termux_restart_confirmed) {
            Assert-Condition ($termux.termux_restart_required -eq $true) `
                "termux_restart_contract_invalid" `
                "Successful Termux preparation did not require its documented restart."
            $processEpoch = Get-TermuxProcessEpochSha256 `
                -Context $Context -Device $device
            if ($processEpoch -ceq ("0" * 64)) {
                Set-Checkpoint -State $State -Kind "awaiting_termux_bootstrap" `
                    -Slot $slot -ReasonCode "termux_process_epoch_unavailable"
                return $false
            }
            Set-Checkpoint -State $State -Kind "awaiting_termux_restart" `
                -Slot $slot -ReasonCode "termux_restart_required" `
                -ProcessEpochSha256 $processEpoch
            return $false
        }
    }
    return $true
}

function Invoke-AgentStagingAndBaseline {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    foreach ($slot in @("device_a", "device_b")) {
        $device = Get-DeviceBySlot -Context $Context -Slot $slot
        $stateDevice = Get-StateDevice -State $State -Slot $slot
        $bootIdentity = Get-DeviceBootIdentity `
            -Context $Context -Device $device
        if (-not [bool]$stateDevice.run_owned.agent_profile_staged) {
            [void](Start-DurableMutation -Context $Context -State $State `
                -Kind "fleet-agent-private-stage" `
                -ActionId "fleet.agent.stage-private-inputs" -Slot $slot `
                -OwnerId "fleet-agent-app-private-storage" `
                -ArtifactPinSha256 (
                    $Context.Artifacts["fleet-agent-apk"].Sha256) `
                -BootIdSha256 $bootIdentity.BootIdSha256 `
                -CleanupOwner "fleet-agent-app-private-storage")
            Set-DurableMutationSent -Context $Context -State $State
            [void](Invoke-AdbExact -Context $Context -Device $device `
                -Arguments @(
                    "shell", "run-as", $script:FleetAgentPackage,
                    "mkdir", "-p", "files/fleet-agent"
                ))
            [void](Invoke-AdbExact -Context $Context -Device $device `
                -Arguments @(
                    "shell", "run-as", $script:FleetAgentPackage,
                    "chmod", "700", "files/fleet-agent"
                ))
            Write-FleetAgentPrivateInput -Context $Context -Device $device `
                -SourcePath $device.fleet_agent_profile_path `
                -Destination "files/fleet-agent/profile.json"
            Write-FleetAgentPrivateInput -Context $Context -Device $device `
                -SourcePath $device.fleet_agent_seed_path `
                -Destination "files/fleet-agent/signing-seed.bin"
            [void](Invoke-AdbExact -Context $Context -Device $device `
                -Arguments @(
                    "shell", "run-as", $script:FleetAgentPackage,
                    "chmod", "600",
                    "files/fleet-agent/profile.json",
                    "files/fleet-agent/signing-seed.bin"
                ))
            Assert-Condition (
                Test-FleetAgentPrivateInputsExact `
                    -Context $Context -Device $device
            ) "fleet_agent_private_mode_mismatch" `
                "Fleet Agent private inputs did not read back with exact hashes and modes."
            $stateDevice.run_owned.agent_profile_staged = $true
            Complete-DurableMutation -Context $Context -State $State `
                -ReconciliationCode "agent_private_hashes_and_modes_exact"
        } else {
            Assert-Condition (
                Test-FleetAgentPrivateInputsExact `
                    -Context $Context -Device $device
            ) "fleet_agent_private_inputs_drifted" `
                "Previously confirmed Fleet Agent inputs drifted; cleanup is required."
        }

        if (-not [bool]$stateDevice.run_owned.agent_started) {
            [void](Start-DurableMutation -Context $Context -State $State `
                -Kind "fleet-agent-start" `
                -ActionId "fleet.agent.debug-start" `
                -Slot $slot -OwnerId "fleet-agent" `
                -ArtifactPinSha256 (
                    $Context.Artifacts["fleet-agent-apk"].Sha256) `
                -BootIdSha256 $bootIdentity.BootIdSha256 `
                -CleanupOwner "fleet-agent")
            Set-DurableMutationSent -Context $Context -State $State
            Start-FleetAgent -Context $Context -Device $device
            $stateDevice.run_owned.agent_started = $true
        } else {
            $pid = Invoke-AdbExact -Context $Context -Device $device `
                -Arguments @("shell", "pidof", $script:FleetAgentPackage) `
                -AllowedExitCodes @(0, 1)
            Assert-Condition (-not [string]::IsNullOrWhiteSpace($pid.Stdout)) `
                "confirmed_agent_process_missing" `
                "A previously confirmed Agent process is absent; it will not be redispatched."
        }
        $minimumRevision = [long]$stateDevice.acceptance.baseline_revision
        $projection = Wait-FleetInspect -Context $Context -Device $device `
            -TimeoutSeconds ([int]$Context.Config.timing.baseline_timeout_seconds) `
            -FailureCode "baseline_checkin_timeout" `
            -Predicate {
                param($value)
                [string]$value.row.identity.device_id -ceq
                    [string]$device.device_id -and
                [long]$value.row.identity.identity_revision -eq
                    [long]$device.identity_revision -and
                [string]$value.row.freshness -ceq "fresh" -and
                [long]$value.row.accepted_revision -ge
                    [Math]::Max(1, $minimumRevision)
            }
        $stateDevice.acceptance.baseline_revision =
            [long]$projection.row.accepted_revision
        if ($null -ne $State.mutation -and
            [string]$State.mutation.kind -ceq "fleet-agent-start") {
            Complete-DurableMutation -Context $Context -State $State `
                -ReconciliationCode "fresh_signed_agent_checkin"
        }

        $status = if (
            $stateDevice.acceptance.Contains("status_operation_id")
        ) {
            Get-WifiOperation -Context $Context `
                -OperationId ([string]$stateDevice.acceptance.status_operation_id)
        } else {
            Invoke-WifiOperation -Context $Context -State $State `
                -Device $device -WifiAction "status"
        }
        Assert-Condition (
            [string]$status.targets[0].receipt.action -ceq "status" -and
            $status.targets[0].receipt.request_delivered -eq $true -and
            $status.targets[0].receipt.effect_applied -eq $true -and
            [string]$status.targets[0].receipt.evidence_sha256 -cmatch
                '^[0-9a-f]{64}$'
        ) "kiosk_direct_status_readback_invalid" `
            "Fleet did not receive a fresh direct Kiosk owner status readback."
        $stateDevice.snapshot.kiosk_direct_link_observation =
            "fresh_owner_status_readback"
    }
    $State.hub.two_fresh_baseline_checkins = $true
}

function Invoke-RequestCheckpoint {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot
    )
    $device = Get-DeviceBySlot -Context $Context -Slot $Slot
    $otherSlot = if ($Slot -ceq "device_a") { "device_b" } else { "device_a" }
    [void](Get-SignedIsolationProjection -Context $Context -State $State `
        -Slot $Slot)
    $otherBefore = Get-SignedIsolationProjection `
        -Context $Context -State $State -Slot $otherSlot
    $stateDevice = Get-StateDevice -State $State -Slot $Slot
    $operation = if ($stateDevice.acceptance.Contains("operation_id")) {
        Get-WifiOperation -Context $Context `
            -OperationId ([string]$stateDevice.acceptance.operation_id)
    } else {
        Invoke-WifiOperation -Context $Context -State $State `
            -Device $device -WifiAction "request-wireless-adb"
    }
    $target = $operation.targets[0]
    Assert-Condition (
        $target.receipt.request_delivered -eq $true -and
        $target.receipt.kiosk_setting_applied -eq $true -and
        [string]$target.receipt.wearer_approval -ceq "pending" -and
        $target.termux_usable -eq $false -and
        $null -eq $target.termux_proof
    ) "request_effect_boundary_invalid" `
        "A Wi-Fi request must remain incomplete pending wearer approval and proof."

    $stateDevice.acceptance.operation_id = [string]$operation.operation_id
    [void](Get-SignedIsolationProjection -Context $Context -State $State `
        -Slot $Slot)
    $otherAfter = Get-SignedIsolationProjection `
        -Context $Context -State $State -Slot $otherSlot
    Assert-NonTargetIsolation -Before $otherBefore -After $otherAfter
    Set-Checkpoint -State $State -Kind "awaiting_wearer_approval" `
        -Slot $Slot -ReasonCode "meta_protected_prompt_requires_wearer"
}

function Invoke-ProofAfterApproval {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot
    )
    $device = Get-DeviceBySlot -Context $Context -Slot $Slot
    $stateDevice = Get-StateDevice -State $State -Slot $Slot
    $otherSlot = if ($Slot -ceq "device_a") { "device_b" } else { "device_a" }
    [void](Get-SignedIsolationProjection -Context $Context -State $State `
        -Slot $Slot)
    $otherBefore = Get-SignedIsolationProjection `
        -Context $Context -State $State -Slot $otherSlot
    $helperRequestId = "fleet-" + [guid]::NewGuid().ToString("N")
    $bootIdentity = Get-DeviceBootIdentity -Context $Context -Device $device
    [void](Start-DurableMutation -Context $Context -State $State `
        -Kind "termux-proof-restore" `
        -ActionId "termux.loopback-adb.restore-now" -Slot $Slot `
        -OwnerId "wireless-adb-helper" `
        -ArtifactPinSha256 $Context.Artifacts["helper-operator"].Sha256 `
        -BootIdSha256 $bootIdentity.BootIdSha256 `
        -CleanupOwner "wireless-adb-helper" -RequestId $helperRequestId)
    Set-DurableMutationSent -Context $Context -State $State
    $helper = Invoke-HelperExact -Context $Context -Device $device `
        -HelperAction "restore-now" -Confirm -RequestId $helperRequestId
    Assert-Condition ($helper.accepted -eq $true) "helper_restore_rejected" `
        "The proof owner rejected its fixed restore request."
    $operation = Wait-WifiProof -Context $Context -Device $device `
        -OperationId ([string]$stateDevice.acceptance.operation_id)
    $proof = $operation.targets[0].termux_proof
    $State.mutation.proof_lineage_sha256 =
        [string]$operation.targets[0].termux_admission.lineage_sha256
    $State.mutation.journal_sha256 =
        Get-MutationJournalSha256 -Mutation $State.mutation
    $stateDevice.acceptance.proof_revision = [long]$proof.evidence_revision
    $stateDevice.acceptance.termux_usable = $true
    Complete-DurableMutation -Context $Context -State $State `
        -ReconciliationCode "fresh_signed_termux_admission"
    [void](Get-SignedIsolationProjection -Context $Context -State $State `
        -Slot $Slot)
    Assert-NonTargetIsolation -Before $otherBefore -After (
        Get-SignedIsolationProjection `
            -Context $Context -State $State -Slot $otherSlot)
}

function Invoke-ProofExpiry {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot
    )
    $stateDevice = Get-StateDevice -State $State -Slot $Slot
    $otherSlot = if ($Slot -ceq "device_a") { "device_b" } else { "device_a" }
    [void](Get-SignedIsolationProjection -Context $Context -State $State `
        -Slot $Slot)
    $otherBefore = Get-SignedIsolationProjection `
        -Context $Context -State $State -Slot $otherSlot
    [void](Wait-WifiProofAbsent -Context $Context `
        -OperationId ([string]$stateDevice.acceptance.operation_id))
    $stateDevice.acceptance.termux_usable = $false
    $stateDevice.acceptance.expiry_observed = $true
    [void](Get-SignedIsolationProjection -Context $Context -State $State `
        -Slot $Slot)
    Assert-NonTargetIsolation -Before $otherBefore -After (
        Get-SignedIsolationProjection `
            -Context $Context -State $State -Slot $otherSlot)
}

function Invoke-ProofRenewal {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot
    )
    $device = Get-DeviceBySlot -Context $Context -Slot $Slot
    $stateDevice = Get-StateDevice -State $State -Slot $Slot
    $otherSlot = if ($Slot -ceq "device_a") { "device_b" } else { "device_a" }
    [void](Get-SignedIsolationProjection -Context $Context -State $State `
        -Slot $Slot)
    $otherBefore = Get-SignedIsolationProjection `
        -Context $Context -State $State -Slot $otherSlot
    $priorOperation = Get-WifiOperation -Context $Context `
        -OperationId ([string]$stateDevice.acceptance.operation_id)
    $priorLineage =
        [string]$priorOperation.targets[0].termux_admission.lineage_sha256
    $helperRequestId = "fleet-" + [guid]::NewGuid().ToString("N")
    $bootIdentity = Get-DeviceBootIdentity -Context $Context -Device $device
    [void](Start-DurableMutation -Context $Context -State $State `
        -Kind "termux-proof-renew" `
        -ActionId "termux.loopback-adb.restore-now" -Slot $Slot `
        -OwnerId "wireless-adb-helper" `
        -ArtifactPinSha256 $Context.Artifacts["helper-operator"].Sha256 `
        -BootIdSha256 $bootIdentity.BootIdSha256 `
        -ProofLineageSha256 $priorLineage `
        -CleanupOwner "wireless-adb-helper" -RequestId $helperRequestId)
    Set-DurableMutationSent -Context $Context -State $State
    $helper = Invoke-HelperExact -Context $Context -Device $device `
        -HelperAction "restore-now" -Confirm -RequestId $helperRequestId
    Assert-Condition ($helper.accepted -eq $true) "helper_renewal_rejected" `
        "The proof owner rejected its fixed renewal request."
    $operation = Wait-WifiProof -Context $Context -Device $device `
        -OperationId ([string]$stateDevice.acceptance.operation_id) `
        -MinimumEvidenceRevision ([long]$stateDevice.acceptance.proof_revision)
    $stateDevice.acceptance.renewed_proof_revision =
        [long]$operation.targets[0].termux_proof.evidence_revision
    $stateDevice.acceptance.termux_usable = $true
    $State.mutation.proof_lineage_sha256 =
        [string]$operation.targets[0].termux_admission.lineage_sha256
    $State.mutation.journal_sha256 =
        Get-MutationJournalSha256 -Mutation $State.mutation
    Complete-DurableMutation -Context $Context -State $State `
        -ReconciliationCode "higher_revision_signed_termux_admission"
    [void](Get-SignedIsolationProjection -Context $Context -State $State `
        -Slot $Slot)
    Assert-NonTargetIsolation -Before $otherBefore -After (
        Get-SignedIsolationProjection `
            -Context $Context -State $State -Slot $otherSlot)
}

function Invoke-DisableAndCheckpointReboot {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot
    )
    $device = Get-DeviceBySlot -Context $Context -Slot $Slot
    $stateDevice = Get-StateDevice -State $State -Slot $Slot
    $otherSlot = if ($Slot -ceq "device_a") { "device_b" } else { "device_a" }
    [void](Get-SignedIsolationProjection -Context $Context -State $State `
        -Slot $Slot)
    $otherBefore = Get-SignedIsolationProjection `
        -Context $Context -State $State -Slot $otherSlot
    $disabled = if (
        $stateDevice.acceptance.Contains("disable_operation_id")
    ) {
        Get-WifiOperation -Context $Context `
            -OperationId ([string]$stateDevice.acceptance.disable_operation_id)
    } else {
        Invoke-WifiOperation -Context $Context -State $State `
            -Device $device -WifiAction "disable-wireless-adb"
    }
    Assert-Condition (
        $disabled.targets.Count -eq 1 -and
        $disabled.targets[0].receipt.effect_applied -eq $true
    ) "wifi_disable_not_applied" "Fleet did not receive an owner-applied disable receipt."
    $priorOperation = Get-WifiOperation -Context $Context `
        -OperationId ([string]$stateDevice.acceptance.operation_id)
    $priorLineage =
        [string]$priorOperation.targets[0].termux_admission.lineage_sha256
    $bootIdentity = Get-DeviceBootIdentity -Context $Context -Device $device
    if (-not [bool]$stateDevice.acceptance.boot_disable_confirmed) {
        $helperRequestId = "fleet-" + [guid]::NewGuid().ToString("N")
        [void](Start-DurableMutation -Context $Context -State $State `
            -Kind "disable-boot-attempt" `
            -ActionId "termux.loopback-adb.disable-boot-attempt" -Slot $Slot `
            -OwnerId "wireless-adb-helper" `
            -ArtifactPinSha256 $Context.Artifacts["helper-operator"].Sha256 `
            -BootIdSha256 $bootIdentity.BootIdSha256 `
            -ProofLineageSha256 $priorLineage `
            -CleanupOwner "wireless-adb-helper" -RequestId $helperRequestId)
        Set-DurableMutationSent -Context $Context -State $State
        [void](Invoke-HelperExact -Context $Context -Device $device `
            -HelperAction "disable-boot-attempt" -Confirm `
            -RequestId $helperRequestId)
    }
    $bootStatus = Invoke-HelperExact -Context $Context -Device $device `
        -HelperAction "status"
    Assert-Condition ($bootStatus.boot_attempt_enabled -eq $false) `
        "boot_attempt_disable_readback_failed" `
        "The helper did not read back its boot attempt as disabled."
    if ($null -ne $State.mutation -and
        [string]$State.mutation.kind -ceq "disable-boot-attempt") {
        $stateDevice.acceptance.boot_disable_confirmed = $true
        Complete-DurableMutation -Context $Context -State $State `
            -ReconciliationCode "boot_attempt_disabled_exact_readback"
    }

    if (-not [bool]$stateDevice.acceptance.wireless_disable_confirmed) {
        $helperRequestId = "fleet-" + [guid]::NewGuid().ToString("N")
        [void](Start-DurableMutation -Context $Context -State $State `
            -Kind "disable-wireless-listener" `
            -ActionId "termux.loopback-adb.disable-wireless" -Slot $Slot `
            -OwnerId "wireless-adb-helper" `
            -ArtifactPinSha256 $Context.Artifacts["helper-operator"].Sha256 `
            -BootIdSha256 $bootIdentity.BootIdSha256 `
            -ProofLineageSha256 $priorLineage `
            -CleanupOwner "wireless-adb-helper" -RequestId $helperRequestId)
        Set-DurableMutationSent -Context $Context -State $State
        [void](Invoke-HelperExact -Context $Context -Device $device `
            -HelperAction "disable-wireless" -Confirm `
            -RequestId $helperRequestId)
    }
    $wirelessReadback = Invoke-AdbExact -Context $Context -Device $device `
        -Arguments @("shell", "settings", "get", "global", "adb_wifi_enabled")
    Assert-Condition ($wirelessReadback.Stdout.Trim() -cin @("0", "null", "")) `
        "wireless_disable_readback_failed" `
        "The exact Android setting did not read back as disabled."
    if ($null -ne $State.mutation -and
        [string]$State.mutation.kind -ceq "disable-wireless-listener") {
        $stateDevice.acceptance.wireless_disable_confirmed = $true
        Complete-DurableMutation -Context $Context -State $State `
            -ReconciliationCode "wireless_setting_disabled_exact_readback"
    }
    [void](Wait-WifiProofAbsent -Context $Context `
        -OperationId ([string]$stateDevice.acceptance.operation_id))
    $bootIdentity = Get-DeviceBootIdentity -Context $Context -Device $device
    $preRebootProjection = Invoke-FleetCtlExact -Context $Context -Arguments @(
        "inspect", [string]$device.device_id
    )
    Assert-Condition (
        [string]$preRebootProjection.row.freshness -ceq "fresh" -and
        [long]$preRebootProjection.row.accepted_revision -gt 0 -and
        -not [string]::IsNullOrWhiteSpace(
            [string]$preRebootProjection.row.source_epoch)
    ) "pre_reboot_signed_projection_invalid" `
        "The reboot checkpoint requires one fresh signed pre-reboot projection."
    $stateDevice.acceptance.pre_reboot_boot_id_sha256 =
        $bootIdentity.BootIdSha256
    $stateDevice.acceptance.pre_reboot_elapsed_milliseconds =
        $bootIdentity.ElapsedMilliseconds
    $stateDevice.acceptance.pre_reboot_source_epoch_sha256 =
        Get-BytesSha256 -Bytes ([Text.Encoding]::UTF8.GetBytes(
            [string]$preRebootProjection.row.source_epoch))
    $stateDevice.acceptance.pre_reboot_accepted_revision =
        [long]$preRebootProjection.row.accepted_revision
    $stateDevice.acceptance.termux_usable = $false
    $stateDevice.acceptance.disable_observed = $true
    [void](Get-SignedIsolationProjection -Context $Context -State $State `
        -Slot $Slot)
    Assert-NonTargetIsolation -Before $otherBefore -After (
        Get-SignedIsolationProjection `
            -Context $Context -State $State -Slot $otherSlot)
    Set-Checkpoint -State $State -Kind "awaiting_attended_reboot" `
        -Slot $Slot -ReasonCode "operator_must_reboot_headset"
}

function Invoke-RebootRecovery {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot
    )
    $device = Get-DeviceBySlot -Context $Context -Slot $Slot
    $stateDevice = Get-StateDevice -State $State -Slot $Slot
    $otherSlot = if ($Slot -ceq "device_a") { "device_b" } else { "device_a" }
    $otherBefore = Get-SignedIsolationProjection `
        -Context $Context -State $State -Slot $otherSlot
    $ready = Invoke-AdbExact -Context $Context -Device $device `
        -Arguments @("get-state")
    Assert-Condition ($ready.Stdout.Trim() -ceq "device") `
        "reboot_usb_not_ready" "The attended reboot has not returned to exact USB readiness."
    $postBootIdentity = Get-DeviceBootIdentity -Context $Context -Device $device
    Assert-Condition (
        [string]$postBootIdentity.BootIdSha256 -cne
            [string]$stateDevice.acceptance.pre_reboot_boot_id_sha256 -and
        [long]$postBootIdentity.ElapsedMilliseconds -lt
            [long]$stateDevice.acceptance.pre_reboot_elapsed_milliseconds
    ) "reboot_boot_identity_unchanged" `
        "A new boot-bound identity and reset elapsed clock are required; confirmation alone is insufficient."
    $pid = Invoke-AdbExact -Context $Context -Device $device `
        -Arguments @("shell", "pidof", $script:FleetAgentPackage) `
        -AllowedExitCodes @(0, 1)
    Assert-Condition ([string]::IsNullOrWhiteSpace($pid.Stdout)) `
        "agent_unexpectedly_sticky" `
        "The non-sticky Fleet Agent was active before explicit relaunch."
    $stateDevice.acceptance.reboot_loss_observed = $true
    [void](Start-DurableMutation -Context $Context -State $State `
        -Kind "fleet-agent-postboot-start" `
        -ActionId "fleet.agent.debug-start" -Slot $Slot `
        -OwnerId "fleet-agent" `
        -ArtifactPinSha256 $Context.Artifacts["fleet-agent-apk"].Sha256 `
        -BootIdSha256 $postBootIdentity.BootIdSha256 `
        -CleanupOwner "fleet-agent")
    Set-DurableMutationSent -Context $Context -State $State
    Start-FleetAgent -Context $Context -Device $device
    $projection = Wait-FleetInspect -Context $Context -Device $device `
        -TimeoutSeconds ([int]$Context.Config.timing.reboot_timeout_seconds) `
        -FailureCode "agent_relaunch_recovery_timeout" `
        -Predicate {
            param($value)
            [string]$value.row.freshness -ceq "fresh" -and
            [long]$value.row.accepted_revision -gt
                [long]$stateDevice.acceptance.pre_reboot_accepted_revision -and
            (Get-BytesSha256 -Bytes ([Text.Encoding]::UTF8.GetBytes(
                [string]$value.row.source_epoch))) -cne
                [string]$stateDevice.acceptance.pre_reboot_source_epoch_sha256
        }
    $postOperation = Get-WifiOperation -Context $Context `
        -OperationId ([string]$stateDevice.acceptance.operation_id)
    Assert-Condition (
        $postOperation.targets[0].termux_usable -eq $false -and
        $null -eq $postOperation.targets[0].termux_proof -and
        $null -eq $postOperation.targets[0].termux_admission
    ) "post_reboot_proof_not_cleared" `
        "The fresh post-boot signed source epoch must not inherit pre-boot proof lineage."
    $stateDevice.acceptance.recovery_observed = $true
    $stateDevice.acceptance.baseline_revision =
        [long]$projection.row.accepted_revision
    [void](Get-SignedIsolationProjection -Context $Context -State $State `
        -Slot $Slot)
    Assert-NonTargetIsolation -Before $otherBefore -After (
        Get-SignedIsolationProjection `
            -Context $Context -State $State -Slot $otherSlot)
    Complete-DurableMutation -Context $Context -State $State `
        -ReconciliationCode "fresh_postboot_signed_agent_checkin"
}

function Complete-Transition {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][string] $NextPhase,
        [string] $Slot = "none",
        [string] $ReasonCode = "transition_complete"
    )
    $State.phase = $NextPhase
    $State.status = "running"
    $State.checkpoint = $null
    Add-StateEvent -State $State -Phase $NextPhase -Status "passed" `
        -Slot $Slot -ReasonCode $ReasonCode
    Write-SanitizedState -Context $Context -State $State
    return $State
}

function Invoke-ResumeTransition {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][bool] $ConfirmCurrentCheckpoint
    )
    if ($null -ne $State.checkpoint) {
        Assert-Condition $ConfirmCurrentCheckpoint "attended_confirmation_required" `
            "The current typed attended checkpoint requires explicit confirmation."
        $kind = [string]$State.checkpoint.kind
        $slot = [string]$State.checkpoint.slot
        switch ($kind) {
            "awaiting_termux_bootstrap" {
                Clear-Checkpoint -State $State
                $ready = Invoke-ProfileAndPackageProvision `
                    -Context $Context -State $State
                if (-not $ready) {
                    Write-SanitizedState -Context $Context -State $State
                    return $State
                }
                return Complete-Transition -Context $Context -State $State `
                    -NextPhase "provisioned" -ReasonCode "termux_bootstrap_completed"
            }
            "awaiting_termux_restart" {
                $stateDevice = Get-StateDevice -State $State -Slot $slot
                $device = Get-DeviceBySlot -Context $Context -Slot $slot
                $newProcessEpoch = Get-TermuxProcessEpochSha256 `
                    -Context $Context -Device $device
                Assert-Condition (
                    $newProcessEpoch -cne ("0" * 64) -and
                    $newProcessEpoch -cne
                        [string]$State.checkpoint.process_epoch_sha256
                ) "termux_process_epoch_unchanged" `
                    "Termux must have a new observed process epoch; confirmation alone is insufficient."
                $stateDevice.run_owned.termux_restart_confirmed = $true
                Clear-Checkpoint -State $State
                $ready = Invoke-ProfileAndPackageProvision `
                    -Context $Context -State $State
                if (-not $ready) {
                    Write-SanitizedState -Context $Context -State $State
                    return $State
                }
                return Complete-Transition -Context $Context -State $State `
                    -NextPhase "provisioned" `
                    -ReasonCode "termux_restart_confirmed"
            }
            "awaiting_wearer_approval" {
                Clear-Checkpoint -State $State
                Invoke-ProofAfterApproval -Context $Context -State $State -Slot $slot
                return Complete-Transition -Context $Context -State $State `
                    -NextPhase "$slot-proof-current" -Slot $slot `
                    -ReasonCode "fresh_signed_shell_proof"
            }
            "awaiting_attended_reboot" {
                Clear-Checkpoint -State $State
                Invoke-RebootRecovery -Context $Context -State $State -Slot $slot
                $next = if ($slot -ceq "device_a") {
                    "device_a-recovered"
                } else {
                    "device_b-recovered"
                }
                return Complete-Transition -Context $Context -State $State `
                    -NextPhase $next -Slot $slot `
                    -ReasonCode "non_sticky_agent_explicitly_relaunched"
            }
            "awaiting_kiosk_direct_link" {
                Clear-Checkpoint -State $State
                $device = Get-DeviceBySlot -Context $Context -Slot $slot
                $profile = Invoke-QfmExact -Context $Context -Arguments @(
                    "connectivity-profile", "status",
                    "--device-id", [string]$device.device_id,
                    "--json"
                )
                Assert-Condition ([string]$profile.state -ceq "enrolled") `
                    "kiosk_direct_link_still_missing" `
                    "QFM still has no enrolled private direct-link profile."
                return Complete-Transition -Context $Context -State $State `
                    -NextPhase "profiles-ready" -Slot $slot `
                    -ReasonCode "kiosk_direct_link_enrolled"
            }
            default {
                throw (New-AcceptanceError "checkpoint_kind_invalid" `
                    "Sanitized state contains an unsupported checkpoint.")
            }
        }
    }

    switch ([string]$State.phase) {
        "preflight" {
            if ([bool]$State.onboarding.applied_by_run) {
                Assert-GeneratedFleetInputs -Context $Context
            } else {
                Assert-OnboardingOutputsAbsent -Context $Context
                $State.onboarding.apply_attempted_by_run = $true
                Write-SanitizedState -Context $Context -State $State
                Invoke-OnboardingApply -Context $Context -State $State
            }
            return Complete-Transition -Context $Context -State $State `
                -NextPhase "onboarding-applied"
        }
        "onboarding-applied" {
            if ([bool]$Context.Config.hub.manage_firewall -and
                -not [bool]$State.hub.firewall_created) {
                [void](Start-DurableMutation -Context $Context -State $State `
                    -Kind "windows-firewall-create" `
                    -ActionId "windows.firewall.create" `
                    -OwnerId "windows-firewall" `
                    -ArtifactPinSha256 (
                        $Context.Artifacts["fleet-hub"].Sha256) `
                    -CleanupOwner "windows-firewall")
                Set-DurableMutationSent -Context $Context -State $State
                $State.hub.firewall_created =
                    New-RunFirewallRule -Context $Context
                $ruleName =
                    "RustyFleet-WifiAdb-" + $Context.Sha256.Substring(0, 12)
                Assert-Condition ($null -ne (
                        Get-NetFirewallRule -DisplayName $ruleName `
                            -ErrorAction SilentlyContinue
                    )) "firewall_rule_readback_missing" `
                    "The exact run-owned firewall rule was not readable."
                Complete-DurableMutation -Context $Context -State $State `
                    -ReconciliationCode "firewall_rule_exact_readback"
            } elseif ([bool]$State.hub.firewall_created) {
                $ruleName =
                    "RustyFleet-WifiAdb-" + $Context.Sha256.Substring(0, 12)
                Assert-Condition ($null -ne (
                        Get-NetFirewallRule -DisplayName $ruleName `
                            -ErrorAction SilentlyContinue
                    )) "confirmed_firewall_rule_missing" `
                    "A previously confirmed firewall rule is absent."
            }
            if (-not [bool]$State.hub.started_by_run) {
                [void](Start-DurableMutation -Context $Context -State $State `
                    -Kind "fleet-hub-start" -ActionId "fleet.hub.start" `
                    -OwnerId "fleet-hub" `
                    -ArtifactPinSha256 $Context.Artifacts["fleet-hub"].Sha256 `
                    -CleanupOwner "runner")
                Set-DurableMutationSent -Context $Context -State $State
                $State.hub.process_id = Start-FleetHub -Context $Context
                $State.hub.started_by_run = $true
            }
            $hubProcess = Get-Process -Id ([int]$State.hub.process_id) `
                -ErrorAction Stop
            try {
                Assert-Condition (
                    $hubProcess.Path -and
                    (Get-Sha256 -Path $hubProcess.Path) -ceq
                        $Context.Artifacts["fleet-hub"].Sha256
                ) "hub_process_readback_mismatch" `
                    "The running Hub process does not match the pinned artifact."
            } finally {
                $hubProcess.Dispose()
            }
            if ($null -ne $State.mutation -and
                [string]$State.mutation.kind -ceq "fleet-hub-start") {
                Complete-DurableMutation -Context $Context -State $State `
                    -ReconciliationCode "pinned_hub_process_exact_readback"
            }
            return Complete-Transition -Context $Context -State $State `
                -NextPhase "hub-started"
        }
        "hub-started" {
            $ready = Invoke-ProfileAndPackageProvision `
                -Context $Context -State $State
            if (-not $ready) {
                Write-SanitizedState -Context $Context -State $State
                return $State
            }
            return Complete-Transition -Context $Context -State $State `
                -NextPhase "provisioned"
        }
        "provisioned" {
            Invoke-AgentStagingAndBaseline -Context $Context -State $State
            return Complete-Transition -Context $Context -State $State `
                -NextPhase "baseline-fresh" `
                -ReasonCode "two_fresh_signed_baseline_checkins"
        }
        "baseline-fresh" {
            $deviceA = Get-StateDevice -State $State -Slot "device_a"
            if ([long]$deviceA.acceptance.proof_revision -gt 0) {
                return Complete-Transition -Context $Context -State $State `
                    -NextPhase "device_a-proof-current" -Slot "device_a" `
                    -ReasonCode "recovered_confirmed_signed_proof"
            }
            Invoke-RequestCheckpoint -Context $Context -State $State `
                -Slot "device_a"
            Write-SanitizedState -Context $Context -State $State
            return $State
        }
        "device_a-proof-current" {
            Invoke-ProofExpiry -Context $Context -State $State -Slot "device_a"
            return Complete-Transition -Context $Context -State $State `
                -NextPhase "device_a-proof-expired" -Slot "device_a"
        }
        "device_a-proof-expired" {
            $deviceA = Get-StateDevice -State $State -Slot "device_a"
            if ([long]$deviceA.acceptance.renewed_proof_revision -gt
                [long]$deviceA.acceptance.proof_revision) {
                return Complete-Transition -Context $Context -State $State `
                    -NextPhase "device_a-proof-renewed" -Slot "device_a" `
                    -ReasonCode "recovered_confirmed_proof_renewal"
            }
            Invoke-ProofRenewal -Context $Context -State $State -Slot "device_a"
            return Complete-Transition -Context $Context -State $State `
                -NextPhase "device_a-proof-renewed" -Slot "device_a" `
                -ReasonCode "higher_evidence_revision"
        }
        "device_a-proof-renewed" {
            $deviceA = Get-StateDevice -State $State -Slot "device_a"
            if ([bool]$deviceA.acceptance.recovery_observed) {
                return Complete-Transition -Context $Context -State $State `
                    -NextPhase "device_a-recovered" -Slot "device_a" `
                    -ReasonCode "recovered_confirmed_postboot_checkin"
            }
            Invoke-DisableAndCheckpointReboot `
                -Context $Context -State $State -Slot "device_a"
            Write-SanitizedState -Context $Context -State $State
            return $State
        }
        "device_a-recovered" {
            $deviceB = Get-StateDevice -State $State -Slot "device_b"
            if ([long]$deviceB.acceptance.proof_revision -gt 0) {
                return Complete-Transition -Context $Context -State $State `
                    -NextPhase "device_b-proof-current" -Slot "device_b" `
                    -ReasonCode "recovered_confirmed_signed_proof"
            }
            Invoke-RequestCheckpoint -Context $Context -State $State `
                -Slot "device_b"
            Write-SanitizedState -Context $Context -State $State
            return $State
        }
        "device_b-proof-current" {
            Invoke-ProofExpiry -Context $Context -State $State -Slot "device_b"
            return Complete-Transition -Context $Context -State $State `
                -NextPhase "device_b-proof-expired" -Slot "device_b"
        }
        "device_b-proof-expired" {
            $deviceB = Get-StateDevice -State $State -Slot "device_b"
            if ([long]$deviceB.acceptance.renewed_proof_revision -gt
                [long]$deviceB.acceptance.proof_revision) {
                return Complete-Transition -Context $Context -State $State `
                    -NextPhase "device_b-proof-renewed" -Slot "device_b" `
                    -ReasonCode "recovered_confirmed_proof_renewal"
            }
            Invoke-ProofRenewal -Context $Context -State $State -Slot "device_b"
            return Complete-Transition -Context $Context -State $State `
                -NextPhase "device_b-proof-renewed" -Slot "device_b" `
                -ReasonCode "higher_evidence_revision"
        }
        "device_b-proof-renewed" {
            $deviceB = Get-StateDevice -State $State -Slot "device_b"
            if ([bool]$deviceB.acceptance.recovery_observed) {
                return Complete-Transition -Context $Context -State $State `
                    -NextPhase "device_b-recovered" -Slot "device_b" `
                    -ReasonCode "recovered_confirmed_postboot_checkin"
            }
            Invoke-DisableAndCheckpointReboot `
                -Context $Context -State $State -Slot "device_b"
            Write-SanitizedState -Context $Context -State $State
            return $State
        }
        "device_b-recovered" {
            $a = Get-StateDevice -State $State -Slot "device_a"
            $b = Get-StateDevice -State $State -Slot "device_b"
            Assert-Condition (
                $a.acceptance.expiry_observed -eq $true -and
                $a.acceptance.disable_observed -eq $true -and
                $a.acceptance.reboot_loss_observed -eq $true -and
                $a.acceptance.recovery_observed -eq $true -and
                $b.acceptance.expiry_observed -eq $true -and
                $b.acceptance.disable_observed -eq $true -and
                $b.acceptance.reboot_loss_observed -eq $true -and
                $b.acceptance.recovery_observed -eq $true -and
                [long]$a.acceptance.renewed_proof_revision -gt
                    [long]$a.acceptance.proof_revision -and
                [long]$b.acceptance.renewed_proof_revision -gt
                    [long]$b.acceptance.proof_revision
            ) "acceptance_matrix_incomplete" `
                "The complete two-device isolation and lifecycle matrix did not pass."
            $State.status = "acceptance_passed_cleanup_required"
            $State.phase = "acceptance-passed"
            $State.claims.installed = $true
            $State.claims.reachable = $true
            $State.claims.authorized = $true
            $State.claims.effective = $true
            Add-StateEvent -State $State -Phase "acceptance-passed" `
                -Status "passed" -ReasonCode "two_device_isolation_complete"
            Set-FinalReceiptDigest -State $State `
                -Disposition "acceptance-passed-cleanup-required"
            Write-SanitizedState -Context $Context -State $State
            return $State
        }
        "acceptance-passed" {
            return $State
        }
        default {
            throw (New-AcceptanceError "resume_phase_invalid" `
                "Sanitized state cannot resume from its current phase.")
        }
    }
}

function Invoke-CleanupStep {
    param(
        [Parameter(Mandatory)][Collections.IDictionary] $Checks,
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][scriptblock] $Operation
    )
    try {
        $Checks[$Name] = (& $Operation) -eq $true
    } catch {
        $Checks[$Name] = $false
    }
}

function Remove-RunAddedPackage {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device,
        [Parameter(Mandatory)][string] $Package
    )
    Assert-Condition ($Package -cin $script:RunInstallablePackages) `
        "cleanup_package_invalid" "Cleanup refused an unowned package."
    $result = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("uninstall", $Package) -AllowedExitCodes @(0, 1) `
        -TimeoutSeconds 120
    return $result.ExitCode -eq 0 -or
        $result.Stdout.Contains("Unknown package", [StringComparison]::OrdinalIgnoreCase)
}

function Remove-AgentPrivateInputs {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device
    )
    $result = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @(
            "shell", "run-as", $script:FleetAgentPackage,
            "rm", "-f",
            "files/fleet-agent/profile.json",
            "files/fleet-agent/signing-seed.bin"
        ) -AllowedExitCodes @(0, 1)
    return $result.ExitCode -in @(0, 1)
}

function Invoke-OnboardingCleanup {
    param([Parameter(Mandatory)][object] $Context)
    if (-not (Test-Path -LiteralPath $Context.Config.onboarding.inventory_path -PathType Leaf)) {
        return $true
    }
    $tool = $Context.Artifacts["fleet-onboard"].Path
    $inventory = [string]$Context.Config.onboarding.inventory_path
    $plan = Invoke-JsonOwner -FilePath $tool -Arguments @(
        "cleanup-plan", "--inventory", $inventory
    ) -TimeoutSeconds 30
    Assert-Condition ([string]$plan.cleanup_sha256 -cmatch '^[0-9a-f]{64}$') `
        "onboarding_cleanup_plan_invalid" `
        "fleet-onboard did not return a canonical cleanup digest."
    $receipt = Invoke-JsonOwner -FilePath $tool -Arguments @(
        "cleanup-apply",
        "--inventory", $inventory,
        "--confirm-cleanup-sha256", [string]$plan.cleanup_sha256
    ) -TimeoutSeconds 60
    return (
        [string]$receipt.schema -ceq
            "rusty.fleet.offline_onboarding_cleanup_receipt.v1" -and
        [string]$receipt.status -ceq "seed-material-deleted"
    )
}

function Invoke-AcceptanceCleanup {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    $State.cleanup.attempted = $true
    $State.cleanup.status = "running"
    $State.status = "cleanup_running"
    Write-SanitizedState -Context $Context -State $State
    $checks = [ordered]@{}

    foreach ($slot in @("device_a", "device_b")) {
        $device = Get-DeviceBySlot -Context $Context -Slot $slot
        $stateDevice = Get-StateDevice -State $State -Slot $slot
        $helperExpectedPresent =
            [bool]$stateDevice.snapshot.packages[$script:HelperPackage] -or
            @($stateDevice.run_owned.added_packages) -contains
                $script:HelperPackage

        Invoke-CleanupStep -Checks $checks -Name "$slot-disable-boot" -Operation {
            if (-not $helperExpectedPresent) {
                return $true
            }
            $receipt = Invoke-HelperExact -Context $Context -Device $device `
                -HelperAction "disable-boot-attempt" -Confirm
            return $receipt.accepted -eq $true
        }
        Invoke-CleanupStep -Checks $checks -Name "$slot-disable-wireless" -Operation {
            if (-not $helperExpectedPresent) {
                return $true
            }
            $receipt = Invoke-HelperExact -Context $Context -Device $device `
                -HelperAction "disable-wireless" -Confirm
            return $receipt.accepted -eq $true
        }
        Invoke-CleanupStep -Checks $checks -Name "$slot-proof-absent" -Operation {
            if ($stateDevice.acceptance.Contains("operation_id") -and
                [string]$stateDevice.acceptance.operation_id) {
                [void](Wait-WifiProofAbsent -Context $Context `
                    -OperationId ([string]$stateDevice.acceptance.operation_id))
            }
            return $true
        }
        Invoke-CleanupStep -Checks $checks -Name "$slot-agent-stopped" -Operation {
            if ([bool]$stateDevice.run_owned.agent_started) {
                Stop-FleetAgent -Context $Context -Device $device
            }
            return $true
        }
        Invoke-CleanupStep -Checks $checks -Name "$slot-agent-inputs-removed" -Operation {
            if ([bool]$stateDevice.run_owned.agent_profile_staged -and
                [bool]$stateDevice.snapshot.packages[$script:FleetAgentPackage]) {
                return Remove-AgentPrivateInputs -Context $Context -Device $device
            }
            return $true
        }
        Invoke-CleanupStep -Checks $checks -Name "$slot-qfm-profile-restored" -Operation {
            if ([bool]$stateDevice.run_owned.qfm_profile_created) {
                $receipt = Invoke-QfmExact -Context $Context -Arguments @(
                    "connectivity-profile",
                    "revoke",
                    "--device-id", [string]$device.device_id,
                    "--confirm-profile-revoke",
                    "--json"
                )
                return [string]$receipt.state -ceq "absent"
            }
            return $true
        }

        foreach ($permission in $script:HelperPermissions) {
            $permissionKey = ($permission -replace '[^A-Za-z0-9]+', '-').ToLowerInvariant()
            Invoke-CleanupStep -Checks $checks `
                -Name "$slot-grant-$permissionKey-restored" -Operation {
                    if ([bool]$stateDevice.snapshot.packages[$script:HelperPackage]) {
                        Set-FixedPackagePermission `
                            -Context $Context -Device $device `
                            -Package $script:HelperPackage `
                            -Permission $permission `
                            -Granted ([bool]$stateDevice.snapshot.helper_grants[$permission])
                    }
                    return $true
                }
        }
        Invoke-CleanupStep -Checks $checks `
            -Name "$slot-kiosk-helper-grant-restored" -Operation {
                if ([bool]$stateDevice.snapshot.packages[$script:KioskHelperPackage]) {
                    Set-FixedPackagePermission `
                        -Context $Context -Device $device `
                        -Package $script:KioskHelperPackage `
                        -Permission $script:WriteSecureSettingsPermission `
                        -Granted ([bool]$stateDevice.snapshot["kiosk_helper_write_secure_settings_granted"])
                }
                return $true
            }

        $addedPackages = @($stateDevice.run_owned.added_packages)
        [Array]::Reverse($addedPackages)
        foreach ($package in $addedPackages) {
            $packageKey = ($package -replace '[^A-Za-z0-9]+', '-').ToLowerInvariant()
            Invoke-CleanupStep -Checks $checks `
                -Name "$slot-package-$packageKey-removed" -Operation {
                    return Remove-RunAddedPackage -Context $Context `
                        -Device $device -Package $package
                }
        }
        Invoke-CleanupStep -Checks $checks `
            -Name "$slot-kiosk-direct-link-unchanged" -Operation {
                # The runner never changes Kiosk direct-link settings. QFM profile
                # creation/revocation is tracked independently above.
                return $true
            }
    }

    Invoke-CleanupStep -Checks $checks -Name "hub-stopped" -Operation {
        return Stop-RunOwnedHub -Context $Context -State $State
    }
    Invoke-CleanupStep -Checks $checks -Name "firewall-removed" -Operation {
        return Remove-RunFirewallRule -Context $Context -State $State
    }
    Invoke-CleanupStep -Checks $checks -Name "onboarding-seeds-removed" -Operation {
        if ([bool]$State.onboarding.apply_attempted_by_run -and
            (Test-Path -LiteralPath $Context.Config.onboarding.inventory_path `
                -PathType Leaf)) {
            return Invoke-OnboardingCleanup -Context $Context
        }
        if ([bool]$State.onboarding.apply_attempted_by_run) {
            return @(
                Get-OnboardingGeneratedPaths -Context $Context |
                    Where-Object { Test-Path -LiteralPath $_ }
            ).Count -eq 0
        }
        return $true
    }
    Invoke-CleanupStep -Checks $checks -Name "runtime-stage-empty" -Operation {
        $root = [IO.Path]::GetFullPath([string]$Context.Config.private_state_root)
        $runtime = [IO.Path]::GetFullPath((Join-Path $root "runtime"))
        Assert-Condition ($runtime.StartsWith(
                $root + [IO.Path]::DirectorySeparatorChar,
                [StringComparison]::OrdinalIgnoreCase)) `
            "runtime_cleanup_refused" "Runtime cleanup escaped its private root."
        if (Test-Path -LiteralPath $runtime -PathType Container) {
            Remove-Item -LiteralPath $runtime -Recurse -Force
        }
        return -not (Test-Path -LiteralPath $runtime)
    }

    $truth = Get-CleanupTruth -Checks $checks
    $State.cleanup.checks = $checks
    $State.cleanup.status = $truth.Status
    $State.status = if ($truth.Status -ceq "complete") {
        "complete"
    } else {
        "cleanup_partial_failure"
    }
    $State.phase = "cleanup"
    $State.claims.effective = $false
    if ($null -ne $State.mutation) {
        $State.mutation.stage = "terminal"
        $State.mutation.confirmed_at_ms =
            [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $State.mutation.reconciliation_code =
            "cleanup-$($truth.Status)"
        $State.mutation.journal_sha256 =
            Get-MutationJournalSha256 -Mutation $State.mutation
        $State.mutation_history = @($State.mutation_history) + $State.mutation
        $State.journal_head_sha256 =
            [string]$State.mutation.journal_sha256
        $State.mutation = $null
    }
    Add-StateEvent -State $State -Phase "cleanup" -Status $truth.Status `
        -ReasonCode (
            if ($truth.Status -ceq "complete") {
                "cleanup_truth_complete"
            } else {
                "cleanup_partial_failure"
            })
    Set-FinalReceiptDigest -State $State `
        -Disposition "cleanup-$($truth.Status)"
    Write-SanitizedState -Context $Context -State $State
    return $State
}

function Get-SanitizedStatus {
    param(
        [Parameter(Mandatory)][object] $Context,
        [AllowNull()][Collections.IDictionary] $State
    )
    if ($null -eq $State) {
        return [ordered]@{
            schema = "rusty.fleet.wifi_adb_two_quest_status.v1"
            status = "not_started"
            phase = "plan"
            checkpoint = $null
            devices = @(
                [ordered]@{ slot = "device_a"; state = "private" },
                [ordered]@{ slot = "device_b"; state = "private" }
            )
            claims = [ordered]@{
                installed = $false
                reachable = $false
                authorized = $false
                effective = $false
            }
        }
    }
    return $State
}

function Invoke-FleetWifiAdbTwoQuestAcceptance {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [ValidateSet("Plan", "Preflight", "Execute", "Resume", "Cleanup", "Status")]
        [string] $Action,
        [Parameter(Mandatory)][string] $RunConfig,
        [switch] $ConfirmMutation,
        [switch] $ConfirmCurrentCheckpoint
    )
    $context = Read-ValidatedRunConfig -RunConfig $RunConfig
    switch ($Action) {
        "Plan" {
            Assert-Condition (-not $ConfirmMutation -and -not $ConfirmCurrentCheckpoint) `
                "plan_confirmation_invalid" "Plan accepts no mutation confirmation."
            return Get-SanitizedPlan -Context $context
        }
        "Preflight" {
            Assert-Condition (-not $ConfirmMutation -and -not $ConfirmCurrentCheckpoint) `
                "preflight_confirmation_invalid" `
                "Preflight is read-only with respect to devices and accepts no mutation confirmation."
            return Invoke-Preflight -Context $context
        }
        "Status" {
            Assert-Condition (-not $ConfirmMutation -and -not $ConfirmCurrentCheckpoint) `
                "status_confirmation_invalid" "Status accepts no mutation confirmation."
            $path = Get-StatePath -Context $context
            $state = if (Test-Path -LiteralPath $path -PathType Leaf) {
                Read-SanitizedState -Context $context
            } else {
                $null
            }
            return Get-SanitizedStatus -Context $context -State $state
        }
        "Execute" {
            Assert-Condition $ConfirmMutation "mutation_confirmation_required" `
                "Execute requires -ConfirmMutation."
            $state = Read-SanitizedState -Context $context
            Assert-NoAmbiguousMutation -Context $context -State $state
            Assert-Condition ([string]$state.phase -ceq "preflight") `
                "execute_requires_preflight" "Execute starts only from completed Preflight."
            return Invoke-ResumeTransition -Context $context -State $state `
                -ConfirmCurrentCheckpoint:$false
        }
        "Resume" {
            Assert-Condition $ConfirmMutation "mutation_confirmation_required" `
                "Resume requires -ConfirmMutation."
            $state = Read-SanitizedState -Context $context
            Assert-NoAmbiguousMutation -Context $context -State $state
            return Invoke-ResumeTransition -Context $context -State $state `
                -ConfirmCurrentCheckpoint:$ConfirmCurrentCheckpoint
        }
        "Cleanup" {
            Assert-Condition $ConfirmMutation "cleanup_confirmation_required" `
                "Cleanup requires -ConfirmMutation."
            $state = Read-SanitizedState -Context $context
            return Invoke-AcceptanceCleanup -Context $context -State $state
        }
    }
}

Export-ModuleMember -Function @(
    "Invoke-FleetWifiAdbTwoQuestAcceptance",
    "Read-ValidatedRunConfig",
    "Test-HubTermuxAdmission",
    "Get-TermuxAdmissionLineageSha256",
    "Get-CleanupTruth",
    "New-SanitizedState",
    "Write-SanitizedState",
    "Read-SanitizedState",
    "Start-DurableMutation",
    "Set-DurableMutationSent",
    "Complete-DurableMutation",
    "Assert-NoAmbiguousMutation"
)
