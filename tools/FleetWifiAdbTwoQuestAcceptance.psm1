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

$script:ConfigSchema = "rusty.fleet.wifi_adb_two_quest_run_config.v2"
$script:StateSchema = "rusty.fleet.wifi_adb_two_quest_acceptance_state.v5"
$script:AgentBoardReceiptSchema =
    "rusty.fleet.wifi_adb_two_quest_agent_board_receipt.v1"
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
            $value = $json |
                ConvertFrom-Json -AsHashtable -Depth 64 -DateKind String
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
        "agent_board",
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

    Assert-ExactProperties -Value $config.agent_board -Required @(
        "cli_path",
        "cli_sha256",
        "lease_duration_seconds"
    ) -Context "agent_board"
    Assert-Condition (
        [string]$config.agent_board.cli_sha256 -cmatch '^[0-9a-f]{64}$'
    ) "agent_board_cli_hash_invalid" `
        "The Agent Board wrapper needs one lowercase SHA-256 pin."
    $config.agent_board.cli_path = Resolve-PrivateLeaf `
        -Path ([string]$config.agent_board.cli_path) `
        -Context "Agent Board wrapper"
    Assert-Condition (
        [IO.Path]::GetExtension([string]$config.agent_board.cli_path) -ceq
            ".ps1"
    ) "agent_board_cli_type_invalid" `
        "The Agent Board coordination owner must be a pinned PowerShell wrapper."
    Assert-Condition (
        (Get-Sha256 -Path ([string]$config.agent_board.cli_path)) -ceq
            [string]$config.agent_board.cli_sha256
    ) "agent_board_cli_hash_mismatch" `
        "The Agent Board wrapper does not match its configured SHA-256."
    Assert-Condition (
        $config.agent_board.lease_duration_seconds -is [int] -or
        $config.agent_board.lease_duration_seconds -is [long]
    ) "agent_board_duration_invalid" `
        "The Agent Board lease duration must be an integer."
    Assert-Condition (
        [long]$config.agent_board.lease_duration_seconds -ge 600 -and
        [long]$config.agent_board.lease_duration_seconds -le 28800
    ) "agent_board_duration_invalid" `
        "The Agent Board lease duration must be between 10 minutes and 8 hours."

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
        [Collections.IDictionary] $Environment = @{},
        [ValidateRange(1, 600)][int] $TimeoutSeconds = 60
    )
    return Invoke-JsonOwner `
        -FilePath $Context.Artifacts["questionable-file-manager"].Path `
        -Arguments $Arguments -Environment $Environment `
        -TimeoutSeconds $TimeoutSeconds
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
        [string]$Context.Config.agent_board.cli_path,
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
        "journal_head_sha256", "final_receipt_sha256",
        "agent_board_reservation"
    ) -Context "acceptance state"
    Assert-Condition (
        $null -eq $State.agent_board_reservation -or
        @("bound", "expired", "released") -ccontains
            [string]$State.agent_board_reservation
    ) "agent_board_projection_invalid" `
        "The sanitized reservation projection must use its bounded vocabulary."
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
    foreach ($claimName in @(
        "planned_only", "installed", "reachable", "authorized", "effective"
    )) {
        Assert-Condition (
            [string]$State.claims[$claimName] -cin @(
                "not_evaluated", "confirmed", "not_claimed",
                "partial", "unknown"
            )
        ) "state_claim_invalid" `
            "Acceptance claims must use the bounded tri-state lifecycle vocabulary."
    }
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
            "adb_tcp_port_state", "adb_tls_port_state",
            "adb_listener_state", "wireless_session_state",
            "wireless_pending_state", "host_forward_count",
            "host_reverse_count", "adb_manager_format",
            "adb_retained_pairing_state",
            "adb_retained_pairing_sha256", "adb_manager_state_sha256",
            "helper_status_state", "helper_in_flight",
            "helper_proof_listener_discovered",
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
            "cleanup_owner", "isolation_scope",
            "isolation_before_sha256", "isolation_before_boot_sha256",
            "isolation_before_elapsed_a_ms", "isolation_before_elapsed_b_ms",
            "isolation_after_sha256", "isolation_after_boot_sha256",
            "isolation_after_elapsed_a_ms", "isolation_after_elapsed_b_ms",
            "previous_journal_sha256", "journal_sha256"
        ) -Optional @("package", "expected_sha256") `
            -Context "acceptance state mutation"
    }
    foreach ($mutation in @($State.mutation_history)) {
        Assert-ExactProperties -Value $mutation -Required @(
            "mutation_id", "kind", "slot", "action_id", "stage", "owner_id",
            "prepared_at_ms", "sent_at_ms", "confirmed_at_ms",
            "reconciliation_code", "target_sha256", "boot_id_sha256",
            "proof_lineage_sha256", "artifact_pin_sha256", "request_id_sha256",
            "cleanup_owner", "isolation_scope",
            "isolation_before_sha256", "isolation_before_boot_sha256",
            "isolation_before_elapsed_a_ms", "isolation_before_elapsed_b_ms",
            "isolation_after_sha256", "isolation_after_boot_sha256",
            "isolation_after_elapsed_a_ms", "isolation_after_elapsed_b_ms",
            "previous_journal_sha256", "journal_sha256"
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

function Get-AgentBoardReceiptPath {
    param([Parameter(Mandatory)][object] $Context)
    return Join-Path ([string]$Context.Config.private_state_root) `
        "agent-board-reservation.json"
}

function Write-AgentBoardReceipt {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Receipt
    )
    $root = [IO.Path]::GetFullPath(
        [string]$Context.Config.private_state_root)
    Assert-Condition (Test-Path -LiteralPath $root -PathType Container) `
        "state_root_missing" `
        "The private state root has not been created by Preflight."
    $path = Get-AgentBoardReceiptPath -Context $Context
    $temporary = Join-Path $root (
        ".agent-board-reservation-" +
        [guid]::NewGuid().ToString("N") + ".pending")
    $json = $Receipt | ConvertTo-Json -Depth 16
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
            $temporaryItem.Attributes -band
                [IO.FileAttributes]::ReparsePoint
        )) "agent_board_receipt_temporary_reparse" `
        "The randomized reservation receipt became a reparse point."
    [RustyFleetAcceptanceNative]::PublishWriteThrough($temporary, $path)
}

function Get-AgentBoardBinding {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)]
        [ValidateSet("device_a", "device_b")]
        [string] $Slot
    )
    $device = Get-DeviceBySlot -Context $Context -Slot $Slot
    return [pscustomobject]@{
        Slot = $Slot
        DeviceId = [string]$device.device_id
        UsbSerial = [string]$device.usb_serial
        Resource = "quest:" + [string]$device.usb_serial
        Owner = "rusty-fleet-wifi-adb-" +
            [string]$Context.Config.run_id
        Task = "two-quest acceptance $Slot"
        Reason = "run=$($Context.Config.run_id);slot=$Slot;" +
            "device=$($device.device_id)"
    }
}

function ConvertFrom-AgentBoardTimestamp {
    param([Parameter(Mandatory)][string] $Text)
    $parsed = [DateTimeOffset]::MinValue
    $valid = $Text -cmatch
        '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' -and
        [DateTimeOffset]::TryParse(
        $Text,
        [Globalization.CultureInfo]::InvariantCulture,
        (
            [Globalization.DateTimeStyles]::AssumeUniversal -bor
            [Globalization.DateTimeStyles]::AdjustToUniversal
        ),
        [ref]$parsed)
    Assert-Condition $valid "agent_board_timestamp_invalid" `
        "Agent Board returned an invalid lease timestamp."
    return $parsed.ToUniversalTime()
}

function Invoke-AgentBoardCli {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][string[]] $Arguments,
        [int[]] $AllowedExitCodes = @(0)
    )
    Assert-Condition (
        (Get-Sha256 -Path ([string]$Context.Config.agent_board.cli_path)) -ceq
            [string]$Context.Config.agent_board.cli_sha256
    ) "agent_board_cli_hash_mismatch" `
        "The pinned Agent Board wrapper changed after config validation."
    $pwsh = (Get-Command pwsh -ErrorAction Stop).Source
    $result = Invoke-BoundedProcess -FilePath $pwsh -Arguments (@(
            "-NoProfile",
            "-ExecutionPolicy", "Bypass",
            "-File", [string]$Context.Config.agent_board.cli_path
        ) + $Arguments) -TimeoutSeconds 30
    Assert-Condition ($result.ExitCode -in $AllowedExitCodes) `
        "agent_board_command_rejected" `
        "Agent Board rejected the exact reservation operation."
    Assert-Condition (
        -not [string]::IsNullOrWhiteSpace($result.Stdout) -and
        $result.Stdout.Length -le 65536
    ) "agent_board_json_invalid" `
        "Agent Board returned no bounded JSON receipt."
    Assert-NoDuplicateJsonProperties -Json $result.Stdout
    try {
        return $result.Stdout |
            ConvertFrom-Json -AsHashtable -Depth 32 -DateKind String
    } catch {
        throw (New-AcceptanceError "agent_board_json_invalid" `
            "Agent Board returned invalid JSON.")
    }
}

function Assert-AgentBoardExternalLease {
    param(
        [Parameter(Mandatory)][Collections.IDictionary] $Lease,
        [Parameter(Mandatory)][object] $Binding,
        [string] $ExpectedLeaseId = "",
        [Parameter(Mandatory)][string[]] $AllowedStatuses,
        [switch] $RequireFuture
    )
    Assert-ExactProperties -Value $Lease -Required @(
        "id", "resource", "owner", "task", "reason", "status", "host",
        "owner_pid", "created_at", "expected_until", "lease_until",
        "released_at", "result", "note"
    ) -Context "Agent Board external lease"
    Assert-Condition (
        [string]$Lease.id -cmatch
            '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' -and
        (-not $ExpectedLeaseId -or
            [string]$Lease.id -ceq $ExpectedLeaseId) -and
        [string]$Lease.resource -ceq [string]$Binding.Resource -and
        [string]$Lease.owner -ceq [string]$Binding.Owner -and
        [string]$Lease.task -ceq [string]$Binding.Task -and
        [string]$Lease.reason -ceq [string]$Binding.Reason -and
        $AllowedStatuses -ccontains [string]$Lease.status
    ) "agent_board_lease_binding_invalid" `
        "Agent Board did not return the exact run, slot, device, and resource binding."
    $leaseUntil = ConvertFrom-AgentBoardTimestamp `
        -Text ([string]$Lease.lease_until)
    if ($RequireFuture) {
        Assert-Condition (
            $leaseUntil -gt [DateTimeOffset]::UtcNow.AddSeconds(5)
        ) "agent_board_lease_expired" `
            "Agent Board did not return a currently active lease horizon."
    }
}

function ConvertTo-AgentBoardReceiptLease {
    param(
        [Parameter(Mandatory)][object] $Binding,
        [Parameter(Mandatory)][Collections.IDictionary] $Lease
    )
    return [ordered]@{
        slot = [string]$Binding.Slot
        device_id = [string]$Binding.DeviceId
        "usb_serial" = [string]$Binding.UsbSerial
        lease_id = [string]$Lease.id
        resource = [string]$Lease.resource
        owner = [string]$Lease.owner
        task = [string]$Lease.task
        reason = [string]$Lease.reason
        status = [string]$Lease.status
        lease_until = [string]$Lease.lease_until
    }
}

function Assert-AgentBoardReceiptShape {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Receipt
    )
    Assert-ExactProperties -Value $Receipt -Required @(
        "schema", "run_id", "config_sha256", "state",
        "created_at_ms", "updated_at_ms", "leases"
    ) -Context "private Agent Board reservation receipt"
    Assert-Condition (
        [string]$Receipt.schema -ceq $script:AgentBoardReceiptSchema -and
        [string]$Receipt.run_id -ceq [string]$Context.Config.run_id -and
        [string]$Receipt.config_sha256 -ceq [string]$Context.Sha256 -and
        @("acquiring", "bound", "expired", "released") -ccontains
            [string]$Receipt.state -and
        $Receipt.leases -is [Collections.IList] -and
        $Receipt.leases.Count -le 2
    ) "agent_board_receipt_invalid" `
        "The private Agent Board reservation receipt is not bound to this run."
    if ([string]$Receipt.state -ceq "bound") {
        Assert-Condition ($Receipt.leases.Count -eq 2) `
            "agent_board_receipt_incomplete" `
            "A bound reservation receipt must contain both logical slots."
    }
    $seen = @()
    foreach ($record in @($Receipt.leases)) {
        Assert-ExactProperties -Value $record -Required @(
            "slot", "device_id", "usb_serial", "lease_id", "resource",
            "owner", "task", "reason", "status", "lease_until"
        ) -Context "private Agent Board lease binding"
        Assert-Condition (
            [string]$record.slot -cin @("device_a", "device_b") -and
            [string]$record.lease_id -cmatch
                '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' -and
            @("active", "expired", "released") -ccontains
                [string]$record.status
        ) "agent_board_receipt_lease_invalid" `
            "The private Agent Board receipt contains an invalid lease."
        $binding = Get-AgentBoardBinding -Context $Context `
            -Slot ([string]$record.slot)
        Assert-Condition (
            [string]$record.device_id -ceq [string]$binding.DeviceId -and
            [string]$record.usb_serial -ceq [string]$binding.UsbSerial -and
            [string]$record.resource -ceq [string]$binding.Resource -and
            [string]$record.owner -ceq [string]$binding.Owner -and
            [string]$record.task -ceq [string]$binding.Task -and
            [string]$record.reason -ceq [string]$binding.Reason
        ) "agent_board_receipt_binding_invalid" `
            "The private receipt does not bind the exact configured device."
        [void](ConvertFrom-AgentBoardTimestamp `
            -Text ([string]$record.lease_until))
        $seen += [string]$record.slot
    }
    Assert-Condition (@($seen | Sort-Object -Unique).Count -eq $seen.Count) `
        "agent_board_receipt_slot_duplicate" `
        "The private receipt contains a duplicate logical slot."
}

function Read-AgentBoardReceipt {
    param([Parameter(Mandatory)][object] $Context)
    $path = Get-AgentBoardReceiptPath -Context $Context
    $receipt = (Read-StrictJsonFile -Path $path `
            -Context "private Agent Board reservation receipt" `
            -MaximumBytes 65536).Value
    Assert-AgentBoardReceiptShape -Context $Context -Receipt $receipt
    return $receipt
}

function New-AgentBoardReceipt {
    param([Parameter(Mandatory)][object] $Context)
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    return [ordered]@{
        schema = $script:AgentBoardReceiptSchema
        run_id = [string]$Context.Config.run_id
        config_sha256 = [string]$Context.Sha256
        state = "acquiring"
        created_at_ms = $now
        updated_at_ms = $now
        leases = @()
    }
}

function Set-AgentBoardReceiptLease {
    param(
        [Parameter(Mandatory)][Collections.IDictionary] $Receipt,
        [Parameter(Mandatory)][Collections.IDictionary] $Record
    )
    $others = @($Receipt.leases | Where-Object {
        [string]$_.slot -cne [string]$Record.slot
    })
    $Receipt.leases = @($others + $Record | Sort-Object {
        [string]$_.slot
    })
    $Receipt.updated_at_ms =
        [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
}

function Invoke-AgentBoardReserve {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][object] $Binding
    )
    $seconds = [long]$Context.Config.agent_board.lease_duration_seconds
    $response = Invoke-AgentBoardCli -Context $Context -Arguments @(
        "reserve", [string]$Binding.Resource,
        "--owner", [string]$Binding.Owner,
        "--task", [string]$Binding.Task,
        "--reason", [string]$Binding.Reason,
        "--duration", "$seconds" + "s",
        "--json"
    )
    Assert-ExactProperties -Value $response -Required @(
        "ok", "status", "lease"
    ) -Context "Agent Board reserve response"
    Assert-Condition (
        $response.ok -eq $true -and
        [string]$response.status -ceq "reserved" -and
        $response.lease -is [Collections.IDictionary]
    ) "agent_board_reserve_failed" `
        "Agent Board did not reserve the exact Quest resource."
    Assert-AgentBoardExternalLease -Lease $response.lease `
        -Binding $Binding -AllowedStatuses @("active") -RequireFuture
    return ConvertTo-AgentBoardReceiptLease `
        -Binding $Binding -Lease $response.lease
}

function Invoke-AgentBoardHeartbeat {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][object] $Binding,
        [Parameter(Mandatory)][Collections.IDictionary] $Record
    )
    $seconds = [long]$Context.Config.agent_board.lease_duration_seconds
    $response = Invoke-AgentBoardCli -Context $Context -Arguments @(
        "heartbeat", [string]$Record.lease_id,
        "--duration", "$seconds" + "s",
        "--json"
    ) -AllowedExitCodes @(0, 2)
    Assert-ExactProperties -Value $response -Required @(
        "ok", "status", "lease"
    ) -Context "Agent Board heartbeat response"
    Assert-Condition (
        $response.ok -eq $true -and
        [string]$response.status -ceq "active" -and
        $response.lease -is [Collections.IDictionary]
    ) "agent_board_heartbeat_failed" `
        "Agent Board could not revalidate an exact Quest reservation."
    Assert-AgentBoardExternalLease -Lease $response.lease `
        -Binding $Binding -ExpectedLeaseId ([string]$Record.lease_id) `
        -AllowedStatuses @("active") -RequireFuture
    return ConvertTo-AgentBoardReceiptLease `
        -Binding $Binding -Lease $response.lease
}

function Assert-AgentBoardReservation {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    $receipt = $null
    try {
        Assert-Condition (
            [string]$State.agent_board_reservation -ceq "bound"
        ) "agent_board_reservation_required" `
            "Both exact Quest reservations are required before device access."
        $receipt = Read-AgentBoardReceipt -Context $Context
        Assert-Condition ([string]$receipt.state -ceq "bound") `
            "agent_board_reservation_required" `
            "The private two-device reservation receipt is not bound."
        foreach ($slot in @("device_a", "device_b")) {
            $record = @($receipt.leases | Where-Object {
                [string]$_.slot -ceq $slot
            })
            Assert-Condition ($record.Count -eq 1) `
                "agent_board_reservation_required" `
                "The private reservation receipt is missing a logical slot."
            $binding = Get-AgentBoardBinding -Context $Context -Slot $slot
            $renewed = Invoke-AgentBoardHeartbeat `
                -Context $Context -Binding $binding -Record $record[0]
            Set-AgentBoardReceiptLease -Receipt $receipt -Record $renewed
            Write-AgentBoardReceipt -Context $Context -Receipt $receipt
        }
        $receipt.state = "bound"
        $receipt.updated_at_ms =
            [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        Write-AgentBoardReceipt -Context $Context -Receipt $receipt
        return $receipt
    } catch {
        if ($null -ne $receipt) {
            $receipt.state = "expired"
            $receipt.updated_at_ms =
                [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            Write-AgentBoardReceipt -Context $Context -Receipt $receipt
        }
        $State.agent_board_reservation = "expired"
        Write-SanitizedState -Context $Context -State $State
        throw
    }
}

function Ensure-AgentBoardReservation {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [switch] $AllowRepair
    )
    $path = Get-AgentBoardReceiptPath -Context $Context
    if (Test-Path -LiteralPath $path -PathType Leaf) {
        try {
            $State.agent_board_reservation = "bound"
            return Assert-AgentBoardReservation `
                -Context $Context -State $State
        } catch {
            if (-not $AllowRepair) {
                throw
            }
        }
        $receipt = Read-AgentBoardReceipt -Context $Context
    } else {
        $receipt = New-AgentBoardReceipt -Context $Context
        Write-AgentBoardReceipt -Context $Context -Receipt $receipt
    }

    $receipt.state = "acquiring"
    Write-AgentBoardReceipt -Context $Context -Receipt $receipt
    try {
        foreach ($slot in @("device_a", "device_b")) {
            $binding = Get-AgentBoardBinding -Context $Context -Slot $slot
            $record = @($receipt.leases | Where-Object {
                [string]$_.slot -ceq $slot
            })
            $active = $null
            if (
                $record.Count -eq 1 -and
                [string]$record[0].status -ceq "active"
            ) {
                try {
                    $active = Invoke-AgentBoardHeartbeat `
                        -Context $Context -Binding $binding -Record $record[0]
                } catch {
                    $active = $null
                }
            }
            if ($null -eq $active) {
                $active = Invoke-AgentBoardReserve `
                    -Context $Context -Binding $binding
            }
            Set-AgentBoardReceiptLease -Receipt $receipt -Record $active
            Write-AgentBoardReceipt -Context $Context -Receipt $receipt
        }
        $receipt.state = "bound"
        $receipt.updated_at_ms =
            [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        Write-AgentBoardReceipt -Context $Context -Receipt $receipt
        $State.agent_board_reservation = "bound"
        Write-SanitizedState -Context $Context -State $State
        return $receipt
    } catch {
        $receipt.state = "expired"
        $receipt.updated_at_ms =
            [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        Write-AgentBoardReceipt -Context $Context -Receipt $receipt
        $State.agent_board_reservation = "expired"
        Write-SanitizedState -Context $Context -State $State
        throw
    }
}

function Release-AgentBoardReservation {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    Assert-Condition (
        [string]$State.cleanup.status -ceq "complete" -and
        [string]$State.status -ceq "complete"
    ) "agent_board_release_before_cleanup" `
        "Quest reservations remain held until terminal cleanup is durable."
    $receipt = Read-AgentBoardReceipt -Context $Context
    $releaseFailed = $false
    foreach ($slot in @("device_a", "device_b")) {
        $record = @($receipt.leases | Where-Object {
            [string]$_.slot -ceq $slot
        })
        if ($record.Count -ne 1) {
            $releaseFailed = $true
            continue
        }
        if (@("released", "expired") -ccontains
            [string]$record[0].status) {
            continue
        }
        $binding = Get-AgentBoardBinding -Context $Context -Slot $slot
        try {
            $response = Invoke-AgentBoardCli -Context $Context -Arguments @(
                "release", [string]$record[0].lease_id,
                "--result", "done",
                "--note", "terminal cleanup complete",
                "--json"
            ) -AllowedExitCodes @(0, 2)
            Assert-ExactProperties -Value $response -Required @(
                "ok", "status", "lease"
            ) -Context "Agent Board release response"
            Assert-Condition (
                $response.ok -eq $true -and
                @("released", "expired") -ccontains
                    [string]$response.status -and
                $response.lease -is [Collections.IDictionary]
            ) "agent_board_release_failed" `
                "Agent Board did not terminalize an exact Quest reservation."
            Assert-AgentBoardExternalLease -Lease $response.lease `
                -Binding $binding `
                -ExpectedLeaseId ([string]$record[0].lease_id) `
                -AllowedStatuses @("released", "expired")
            $released = ConvertTo-AgentBoardReceiptLease `
                -Binding $binding -Lease $response.lease
            Set-AgentBoardReceiptLease -Receipt $receipt -Record $released
            Write-AgentBoardReceipt -Context $Context -Receipt $receipt
        } catch {
            $releaseFailed = $true
        }
    }
    if ($releaseFailed) {
        $receipt.state = "expired"
        $State.agent_board_reservation = "expired"
        Write-AgentBoardReceipt -Context $Context -Receipt $receipt
        Write-SanitizedState -Context $Context -State $State
        throw (New-AcceptanceError "agent_board_release_incomplete" `
            "Terminal cleanup is complete, but reservation release is incomplete.")
    }
    $receipt.state = "released"
    $receipt.updated_at_ms =
        [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    Write-AgentBoardReceipt -Context $Context -Receipt $receipt
    $State.agent_board_reservation = "released"
    Write-SanitizedState -Context $Context -State $State
    return $receipt
}

function Set-AgentBoardStatusProjection {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    if ([string]$State.agent_board_reservation -cne "bound") {
        return
    }
    try {
        $receipt = Read-AgentBoardReceipt -Context $Context
        if (
            [string]$receipt.state -cne "bound" -or
            @($receipt.leases | Where-Object {
                (ConvertFrom-AgentBoardTimestamp `
                    -Text ([string]$_.lease_until)) -le
                    [DateTimeOffset]::UtcNow
            }).Count -ne 0
        ) {
            $State.agent_board_reservation = "expired"
        }
    } catch {
        $State.agent_board_reservation = "expired"
    }
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
        [string]$Mutation.isolation_scope,
        [string]$Mutation.isolation_before_sha256,
        [string]$Mutation.isolation_before_boot_sha256,
        [string][long]$Mutation.isolation_before_elapsed_a_ms,
        [string][long]$Mutation.isolation_before_elapsed_b_ms,
        [string]$Mutation.isolation_after_sha256,
        [string]$Mutation.isolation_after_boot_sha256,
        [string][long]$Mutation.isolation_after_elapsed_a_ms,
        [string][long]$Mutation.isolation_after_elapsed_b_ms,
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
            [string]$record.stage -cin @("confirmed", "terminal") -and
            [string]$record.isolation_scope -cin @(
                "device_a", "device_b", "both", "modeled_not_executed"
            ) -and
            (
                [string]$record.isolation_scope -ceq
                    "modeled_not_executed" -or
                (
                    [string]$record.isolation_after_sha256 -cmatch
                        '^[0-9a-f]{64}$' -and
                    [string]$record.isolation_after_sha256 -cne ("0" * 64) -and
                    [string]$record.isolation_after_boot_sha256 -cmatch
                        '^[0-9a-f]{64}$' -and
                    [string]$record.isolation_after_boot_sha256 -cne ("0" * 64)
                )
            )
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
            [string]$State.mutation.isolation_scope -cin @(
                "device_a", "device_b", "both", "modeled_not_executed"
            ) -and
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
        [string] $RequestId = "",
        [switch] $ModeledNoDeviceProjection
    )
    Assert-Condition ($null -eq $State.mutation) "mutation_already_active" `
        "A durable mutation must be reconciled before another can start."
    if (-not $ModeledNoDeviceProjection) {
        [void](Assert-AgentBoardReservation `
            -Context $Context -State $State)
    }
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
    $isolationScope = if ($ModeledNoDeviceProjection) {
        "modeled_not_executed"
    } elseif ($Slot -ceq "device_a") {
        "device_b"
    } elseif ($Slot -ceq "device_b") {
        "device_a"
    } else {
        "both"
    }
    $isolation = if ($ModeledNoDeviceProjection) {
        [pscustomobject]@{
            ProjectionSha256 = "0" * 64
            BootSha256 = "0" * 64
            ElapsedA = 0L
            ElapsedB = 0L
        }
    } else {
        Get-DurableMutationIsolationProjection `
            -Context $Context -Scope $isolationScope
    }
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
        isolation_scope = $isolationScope
        isolation_before_sha256 = $isolation.ProjectionSha256
        isolation_before_boot_sha256 = $isolation.BootSha256
        isolation_before_elapsed_a_ms = $isolation.ElapsedA
        isolation_before_elapsed_b_ms = $isolation.ElapsedB
        isolation_after_sha256 = "0" * 64
        isolation_after_boot_sha256 = "0" * 64
        isolation_after_elapsed_a_ms = 0L
        isolation_after_elapsed_b_ms = 0L
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
    if (
        [string]$State.mutation.isolation_scope -cne
            "modeled_not_executed"
    ) {
        [void](Assert-AgentBoardReservation `
            -Context $Context -State $State)
    }
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
    Set-DurableMutationIsolationAfter -Context $Context -State $State
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

function Complete-DurableMutationTerminal {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][string] $ReconciliationCode
    )
    Assert-Condition ($null -ne $State.mutation) "mutation_absent" `
        "No durable mutation is available to terminalize."
    Set-DurableMutationIsolationAfter -Context $Context -State $State
    $State.mutation.stage = "terminal"
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
                adb_tcp_port_state = $snapshot.adb_tcp_port_state
                adb_tls_port_state = $snapshot.adb_tls_port_state
                adb_listener_state = $snapshot.adb_listener_state
                wireless_session_state = $snapshot.wireless_session_state
                wireless_pending_state = $snapshot.wireless_pending_state
                host_forward_count = $snapshot.host_forward_count
                host_reverse_count = $snapshot.host_reverse_count
                adb_manager_format = $snapshot.adb_manager_format
                adb_retained_pairing_state =
                    $snapshot.adb_retained_pairing_state
                adb_retained_pairing_sha256 =
                    $snapshot.adb_retained_pairing_sha256
                adb_manager_state_sha256 = $snapshot.adb_manager_state_sha256
                helper_status_state = $snapshot.helper_status_state
                helper_in_flight = $snapshot.helper_in_flight
                helper_proof_listener_discovered =
                    $snapshot.helper_proof_listener_discovered
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
        agent_board_reservation = $null
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
            planned_only = "not_claimed"
            installed = "not_evaluated"
            reachable = "not_evaluated"
            authorized = "not_evaluated"
            effective = "not_evaluated"
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

function Convert-AdbPortState {
    param(
        [AllowEmptyString()][string] $Value,
        [Parameter(Mandatory)][string] $PropertyName
    )
    $normalized = $Value.Trim()
    Assert-Condition (
        $normalized -cin @("", "-1", "0") -or
        $normalized -cmatch '^[1-9][0-9]{0,4}$'
    ) "adb_port_readback_invalid" `
        "$PropertyName returned an unsupported value."
    if ($normalized -cin @("", "-1", "0")) {
        return [pscustomobject]@{ State = "inactive"; Port = 0 }
    }
    $port = [int]$normalized
    Assert-Condition ($port -le 65535) "adb_port_readback_invalid" `
        "$PropertyName returned an out-of-range port."
    return [pscustomobject]@{ State = "active"; Port = $port }
}

function New-UnknownAdbManagerReadback {
    param([Parameter(Mandatory)][string] $Reason)
    return [pscustomobject]@{
        Format = "unknown"
        ParseState = "unknown"
        Reason = $Reason
        ManagerConnectedToAdbd = $null
        RetainedPairingState = "unknown"
        RetainedPairingCount = 0
        TrustedWifiNetworkCount = 0
        RetainedPairingSha256 = "0" * 64
        ListenerState = "unknown"
        WirelessSessionState = "unknown"
        WirelessPendingState = "unknown"
    }
}

function ConvertFrom-ClosedAdbManagerDump {
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Text)

    if ([string]::IsNullOrWhiteSpace($Text) -or $Text.Length -gt 1048576) {
        return New-UnknownAdbManagerReadback -Reason "empty_or_unbounded"
    }
    $normalized = $Text.Replace("`r`n", "`n").Replace("`r", "`n").Trim()
    $lines = @($normalized -split "`n")
    if (
        $lines.Count -lt 6 -or
        $lines[0] -cne "ADB MANAGER STATE (dumpsys adb):" -or
        $lines[1] -cne "{" -or
        $lines[2] -cne "  debugging_manager={" -or
        $lines[$lines.Count - 2] -cne "  }" -or
        $lines[$lines.Count - 1] -cne "}"
    ) {
        return New-UnknownAdbManagerReadback -Reason "unsupported_envelope"
    }

    $allowedFields = @(
        "connected_to_adb",
        "last_key_received",
        "user_keys",
        "system_keys",
        "keystore"
    )
    $fields = [ordered]@{}
    $current = ""
    for ($index = 3; $index -lt ($lines.Count - 2); $index++) {
        $line = [string]$lines[$index]
        if ($line -ceq "" -or $line -ceq "    ") {
            continue
        }
        if ($line -cmatch '^    ([a-z][a-z0-9_]*)=(.*)$') {
            $name = [string]$Matches[1]
            if ($name -cnotin $allowedFields -or $fields.Contains($name)) {
                return New-UnknownAdbManagerReadback `
                    -Reason "unsupported_or_duplicate_field"
            }
            $current = $name
            $fields[$name] = @([string]$Matches[2])
            continue
        }
        if (
            -not $current -or
            $current -cnotin @("user_keys", "system_keys", "keystore") -or
            -not $line.StartsWith("    ", [StringComparison]::Ordinal)
        ) {
            return New-UnknownAdbManagerReadback `
                -Reason "unsupported_continuation"
        }
        $fields[$current] = @($fields[$current]) + $line.Substring(4)
    }
    if (
        -not $fields.Contains("connected_to_adb") -or
        @($fields.connected_to_adb).Count -ne 1 -or
        [string]$fields.connected_to_adb[0] -cnotin @("true", "false") -or
        -not $fields.Contains("user_keys") -or
        -not $fields.Contains("keystore")
    ) {
        return New-UnknownAdbManagerReadback -Reason "incomplete_v1_fields"
    }

    $userKeys = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal)
    foreach ($line in @($fields.user_keys)) {
        $key = ([string]$line).Trim()
        if (-not $key) {
            continue
        }
        if ($key.Length -gt 16384 -or $key -cmatch '[\x00-\x08\x0b\x0c\x0e-\x1f]') {
            return New-UnknownAdbManagerReadback -Reason "invalid_user_key"
        }
        [void]$userKeys.Add($key)
    }

    $xmlText = (@($fields.keystore) -join "`n").Trim()
    if ([string]::IsNullOrWhiteSpace($xmlText)) {
        return New-UnknownAdbManagerReadback -Reason "missing_keystore"
    }
    $document = [Xml.XmlDocument]::new()
    $document.XmlResolver = $null
    $reader = $null
    try {
        $settings = [Xml.XmlReaderSettings]::new()
        $settings.DtdProcessing = [Xml.DtdProcessing]::Prohibit
        $settings.XmlResolver = $null
        $settings.MaxCharactersInDocument = 1048576
        $reader = [Xml.XmlReader]::Create(
            [IO.StringReader]::new($xmlText),
            $settings)
        $document.Load($reader)
    } catch {
        return New-UnknownAdbManagerReadback -Reason "invalid_keystore_xml"
    } finally {
        if ($null -ne $reader) {
            $reader.Dispose()
        }
    }
    $root = $document.DocumentElement
    if (
        $null -eq $root -or
        $root.Name -cne "keyStore" -or
        $root.Attributes.Count -ne 1 -or
        $root.GetAttribute("version") -cne "1"
    ) {
        return New-UnknownAdbManagerReadback `
            -Reason "unsupported_keystore_version"
    }

    $keystoreKeys = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal)
    $trustedNetworks = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal)
    foreach ($node in @($root.ChildNodes)) {
        if ($node.NodeType -ne [Xml.XmlNodeType]::Element) {
            continue
        }
        if ($node.Name -ceq "adbKey") {
            if (
                $node.Attributes.Count -ne 2 -or
                -not $node.HasAttribute("key") -or
                -not $node.HasAttribute("lastConnection") -or
                $node.GetAttribute("lastConnection") -cnotmatch '^[0-9]+$'
            ) {
                return New-UnknownAdbManagerReadback `
                    -Reason "invalid_keystore_key"
            }
            $key = $node.GetAttribute("key")
            if (-not $key -or $key.Length -gt 16384) {
                return New-UnknownAdbManagerReadback `
                    -Reason "invalid_keystore_key"
            }
            [void]$keystoreKeys.Add($key)
        } elseif ($node.Name -ceq "wifiAP") {
            if (
                $node.Attributes.Count -ne 1 -or
                -not $node.HasAttribute("bssid")
            ) {
                return New-UnknownAdbManagerReadback `
                    -Reason "invalid_trusted_network"
            }
            $bssid = $node.GetAttribute("bssid")
            if ($bssid -cnotmatch '^[0-9A-Fa-f:.-]{1,64}$') {
                return New-UnknownAdbManagerReadback `
                    -Reason "invalid_trusted_network"
            }
            [void]$trustedNetworks.Add($bssid.ToLowerInvariant())
        } else {
            return New-UnknownAdbManagerReadback `
                -Reason "unsupported_keystore_element"
        }
    }
    foreach ($key in $userKeys) {
        if (-not $keystoreKeys.Contains($key)) {
            return New-UnknownAdbManagerReadback `
                -Reason "user_keystore_key_mismatch"
        }
    }
    $retainedMaterial = @(
        "rusty.fleet.adb-retained-authorization.v1"
        @($keystoreKeys | Sort-Object | ForEach-Object { "key:$($_)" })
        @($trustedNetworks | Sort-Object | ForEach-Object { "wifi:$($_)" })
    ) -join "`0"
    $retainedCount = $keystoreKeys.Count + $trustedNetworks.Count
    return [pscustomobject]@{
        Format = "android.debugging_manager.text.v1"
        ParseState = "known"
        Reason = "closed_v1"
        ManagerConnectedToAdbd =
            [string]$fields.connected_to_adb[0] -ceq "true"
        RetainedPairingState = if ($retainedCount -gt 0) {
            "present"
        } else {
            "absent"
        }
        RetainedPairingCount = $keystoreKeys.Count
        TrustedWifiNetworkCount = $trustedNetworks.Count
        RetainedPairingSha256 = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes($retainedMaterial))
        # AOSP v1 does not expose these facts. They may only become known
        # through the independent mDNS and socket-owner readbacks below.
        ListenerState = "unknown"
        WirelessSessionState = "unknown"
        WirelessPendingState = "unknown"
    }
}

function ConvertFrom-ClosedAdbMdnsServices {
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Text)
    if ([string]::IsNullOrWhiteSpace($Text) -or $Text.Length -gt 1048576) {
        return [pscustomobject]@{
            ParseState = "unknown"
            Services = @()
        }
    }
    $lines = @($Text.Replace("`r`n", "`n").Replace("`r", "`n").Trim() `
        -split "`n")
    if (
        $lines.Count -lt 1 -or
        $lines[0].Trim() -cne "List of discovered mdns services"
    ) {
        return [pscustomobject]@{
            ParseState = "unknown"
            Services = @()
        }
    }
    $services = @()
    foreach ($line in @($lines | Select-Object -Skip 1)) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        if ($line -cnotmatch (
            '^\s*(\S+)\s+(_adb(?:-tls-(?:pairing|connect))?\._tcp)\.?' +
            '\s+((?:[0-9]{1,3}\.){3}[0-9]{1,3}):([0-9]{1,5})\s*$'
        )) {
            return [pscustomobject]@{
                ParseState = "unknown"
                Services = @()
            }
        }
        $address = $null
        $port = [int]$Matches[4]
        if (
            -not [Net.IPAddress]::TryParse(
                [string]$Matches[3],
                [ref]$address) -or
            $address.AddressFamily -ne
                [Net.Sockets.AddressFamily]::InterNetwork -or
            $port -lt 1 -or $port -gt 65535
        ) {
            return [pscustomobject]@{
                ParseState = "unknown"
                Services = @()
            }
        }
        $services += [pscustomobject]@{
            Instance = [string]$Matches[1]
            Type = [string]$Matches[2]
            Address = $address.ToString()
            Port = $port
        }
    }
    return [pscustomobject]@{
        ParseState = "known"
        Services = @($services)
    }
}

function New-UnknownAdbdSocketOwnerReadback {
    return [pscustomobject]@{
        ParseState = "unknown"
        ListenerPorts = @()
        EstablishedPorts = @()
    }
}

function ConvertFrom-ClosedAdbdSocketOwnerReadback {
    param([Parameter(Mandatory)][AllowEmptyString()][string] $Text)
    $unknown = New-UnknownAdbdSocketOwnerReadback
    if ([string]::IsNullOrWhiteSpace($Text) -or $Text.Length -gt 2097152) {
        return $unknown
    }
    $lines = @($Text.Replace("`r`n", "`n").Replace("`r", "`n").Trim() `
        -split "`n")
    if (
        $lines.Count -lt 29 -or
        $lines[0] -cne "rusty.fleet.adbd_socket_owner.v3"
    ) {
        return $unknown
    }

    $lineIndex = 1
    $samples = @()
    foreach ($sampleNumber in @(1, 2)) {
        if (
            $lineIndex -ge $lines.Count -or
            $lines[$lineIndex] -cne "sample_begin=$sampleNumber"
        ) {
            return $unknown
        }
        $lineIndex++
        if (
            $lineIndex -ge $lines.Count -or
            $lines[$lineIndex] -cnotmatch '^pre_pid=([1-9][0-9]*)$'
        ) {
            return $unknown
        }
        $prePid = [string]$Matches[1]
        $lineIndex++
        if (
            $lineIndex -ge $lines.Count -or
            $lines[$lineIndex] -cnotmatch '^pre_starttime=([1-9][0-9]*)$'
        ) {
            return $unknown
        }
        $preStartTime = [string]$Matches[1]
        $lineIndex++
        if (
            $lineIndex -ge $lines.Count -or
            $lines[$lineIndex] -cne "pre_fd_begin=1"
        ) {
            return $unknown
        }
        $lineIndex++

        $preInodes = [Collections.Generic.HashSet[string]]::new(
            [StringComparer]::Ordinal)
        while (
            $lineIndex -lt $lines.Count -and
            $lines[$lineIndex] -cne "pre_fd_end=1"
        ) {
            if (
                $lines[$lineIndex] -cnotmatch '^pre_inode=([1-9][0-9]*)$'
            ) {
                return $unknown
            }
            [void]$preInodes.Add([string]$Matches[1])
            $lineIndex++
        }
        if (
            $lineIndex -ge $lines.Count -or
            $lines[$lineIndex] -cne "pre_fd_end=1"
        ) {
            return $unknown
        }
        $lineIndex++

        $rows = @()
        $canonicalRows = [Collections.Generic.HashSet[string]]::new(
            [StringComparer]::Ordinal)
        foreach ($family in @(4, 6)) {
            if (
                $lineIndex -ge $lines.Count -or
                $lines[$lineIndex] -cne "tcp${family}_begin=1"
            ) {
                return $unknown
            }
            $lineIndex++
            while (
                $lineIndex -lt $lines.Count -and
                $lines[$lineIndex] -cne "tcp${family}_end=1"
            ) {
                $line = [string]$lines[$lineIndex]
                if ($line -cnotmatch (
                    "^tcp${family}=([0-9A-F]{8}|[0-9A-F]{32}):" +
                    '([0-9A-F]{4}),' +
                    '([0-9A-F]{8}|[0-9A-F]{32}):([0-9A-F]{4}),' +
                    '([0-9A-F]{2}),([1-9][0-9]*)$'
                )) {
                    return $unknown
                }
                $localAddress = [string]$Matches[1]
                $remoteAddress = [string]$Matches[3]
                if (
                    ($family -eq 4 -and (
                        $localAddress.Length -ne 8 -or
                        $remoteAddress.Length -ne 8
                    )) -or
                    ($family -eq 6 -and (
                        $localAddress.Length -ne 32 -or
                        $remoteAddress.Length -ne 32
                    )) -or
                    -not $canonicalRows.Add($line)
                ) {
                    return $unknown
                }
                $port = [Convert]::ToInt32([string]$Matches[2], 16)
                if ($port -lt 1 -or $port -gt 65535) {
                    return $unknown
                }
                $rows += [pscustomobject]@{
                    Canonical = $line
                    LocalPort = $port
                    State = [string]$Matches[5]
                    Inode = [string]$Matches[6]
                }
                $lineIndex++
            }
            if (
                $lineIndex -ge $lines.Count -or
                $lines[$lineIndex] -cne "tcp${family}_end=1"
            ) {
                return $unknown
            }
            $lineIndex++
        }

        if (
            $lineIndex -ge $lines.Count -or
            $lines[$lineIndex] -cne "post_fd_begin=1"
        ) {
            return $unknown
        }
        $lineIndex++
        $postInodes = [Collections.Generic.HashSet[string]]::new(
            [StringComparer]::Ordinal)
        while (
            $lineIndex -lt $lines.Count -and
            $lines[$lineIndex] -cne "post_fd_end=1"
        ) {
            if (
                $lines[$lineIndex] -cnotmatch '^post_inode=([1-9][0-9]*)$'
            ) {
                return $unknown
            }
            [void]$postInodes.Add([string]$Matches[1])
            $lineIndex++
        }
        if (
            $lineIndex -ge $lines.Count -or
            $lines[$lineIndex] -cne "post_fd_end=1"
        ) {
            return $unknown
        }
        $lineIndex++

        if (
            $lineIndex -ge $lines.Count -or
            $lines[$lineIndex] -cnotmatch '^post_pid=([1-9][0-9]*)$'
        ) {
            return $unknown
        }
        $postPid = [string]$Matches[1]
        $lineIndex++
        if (
            $lineIndex -ge $lines.Count -or
            $lines[$lineIndex] -cnotmatch '^post_starttime=([1-9][0-9]*)$'
        ) {
            return $unknown
        }
        $postStartTime = [string]$Matches[1]
        $lineIndex++
        if (
            $lineIndex -ge $lines.Count -or
            $lines[$lineIndex] -cne "sample_end=$sampleNumber"
        ) {
            return $unknown
        }
        $lineIndex++

        if (
            $prePid -cne $postPid -or
            $preStartTime -cne $postStartTime -or
            $preInodes.Count -ne $postInodes.Count -or
            @($preInodes | Where-Object {
                -not $postInodes.Contains([string]$_)
            }).Count -ne 0
        ) {
            return $unknown
        }
        $ownedRows = @($rows | Where-Object {
            $preInodes.Contains([string]$_.Inode)
        })
        $projectionParts = @(
            @($preInodes | Sort-Object | ForEach-Object { "inode=$_" })
            @($ownedRows | ForEach-Object {
                [string]$_.Canonical
            } | Sort-Object)
        )
        $samples += [pscustomobject]@{
            Pid = $prePid
            StartTime = $preStartTime
            Projection = $projectionParts -join "`n"
            OwnedRows = $ownedRows
        }
    }
    if (
        $lineIndex -ne $lines.Count -or
        $samples.Count -ne 2 -or
        [string]$samples[0].Pid -cne [string]$samples[1].Pid -or
        [string]$samples[0].StartTime -cne
            [string]$samples[1].StartTime -or
        [string]$samples[0].Projection -cne
            [string]$samples[1].Projection
    ) {
        return $unknown
    }

    $stableOwnedRows = @($samples[1].OwnedRows)
    return [pscustomobject]@{
        ParseState = "known"
        ListenerPorts = @($stableOwnedRows | Where-Object {
            [string]$_.State -ceq "0A"
        } | ForEach-Object { [int]$_.LocalPort } | Sort-Object -Unique)
        EstablishedPorts = @($stableOwnedRows | Where-Object {
            [string]$_.State -ceq "01"
        } | ForEach-Object { [int]$_.LocalPort } | Sort-Object -Unique)
    }
}

function Resolve-AdbOwnerNetworkFacts {
    param(
        [Parameter(Mandatory)][object] $Manager,
        [Parameter(Mandatory)][object] $Mdns,
        [Parameter(Mandatory)][object] $OwnerSockets,
        [Parameter(Mandatory)][object] $Tcp,
        [Parameter(Mandatory)][object] $Tls,
        [Parameter(Mandatory)][string] $WifiSetting,
        [Parameter(Mandatory)][string] $PersistentTlsSetting,
        [Parameter(Mandatory)][AllowEmptyCollection()][object[]] $TargetServices
    )
    $unknown = [pscustomobject]@{
        ListenerState = "unknown"
        WirelessSessionState = "unknown"
        WirelessPendingState = "unknown"
    }
    if (
        $Manager.ParseState -cne "known" -or
        $Mdns.ParseState -cne "known" -or
        $OwnerSockets.ParseState -cne "known"
    ) {
        return $unknown
    }

    $connectServices = @($TargetServices | Where-Object {
        [string]$_.Type -cin @("_adb._tcp", "_adb-tls-connect._tcp")
    })
    $pairingServices = @($TargetServices | Where-Object {
        [string]$_.Type -ceq "_adb-tls-pairing._tcp"
    })
    $ownerListenerPorts = @($OwnerSockets.ListenerPorts)
    $ownerEstablishedPorts = @($OwnerSockets.EstablishedPorts)
    $propertyMismatch =
        ($Tcp.State -ceq "active" -and
            $ownerListenerPorts -notcontains [int]$Tcp.Port) -or
        ($Tls.State -ceq "active" -and
            $ownerListenerPorts -notcontains [int]$Tls.Port)
    $advertisementMismatch = @($connectServices | Where-Object {
        $ownerListenerPorts -notcontains [int]$_.Port
    }).Count -gt 0
    if ($propertyMismatch -or $advertisementMismatch) {
        return $unknown
    }

    # The typed owner readback resolves /proc/<adbd>/fd socket inodes against
    # the kernel TCP tables. Therefore zero here is an actual process-owned
    # absence, not an inference from properties or a discovery timeout.
    $listenerState = if (
        $ownerListenerPorts.Count -gt 0 -or
        $pairingServices.Count -gt 0
    ) {
        "active"
    } else {
        "absent"
    }
    $sessionState = if ($ownerEstablishedPorts.Count -gt 0) {
        "active"
    } else {
        "absent"
    }

    $pendingState = if ($pairingServices.Count -gt 0) {
        "pending"
    } elseif (
        (
            $WifiSetting -ceq "1" -or
            $PersistentTlsSetting -cin @("1", "true")
        ) -and
        $listenerState -cne "active"
    ) {
        "pending"
    } elseif (
        $WifiSetting -cin @("", "0", "null") -and
        $PersistentTlsSetting -cin @("", "0", "false", "null")
    ) {
        "absent"
    } else {
        # AOSP v1 does not dump pairing-in-progress. A missing host mDNS
        # discovery while Wireless Debugging is active is not proof that no
        # pairing request exists.
        "unknown"
    }
    return [pscustomobject]@{
        ListenerState = $listenerState
        WirelessSessionState = $sessionState
        WirelessPendingState = $pendingState
    }
}

function Invoke-AdbHostExact {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][string[]] $Arguments,
        [ValidateRange(1, 600)][int] $TimeoutSeconds = 30
    )
    $result = Invoke-BoundedProcess `
        -FilePath $Context.Artifacts["adb"].Path `
        -Arguments $Arguments -TimeoutSeconds $TimeoutSeconds
    Assert-Condition ($result.ExitCode -eq 0) "host_adb_readback_failed" `
        "ADB's bounded host-side owner readback failed."
    return $result
}

function Get-AdbNetworkObservation {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device
    )
    $wifiResult = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "settings", "get", "global", "adb_wifi_enabled")
    $wifiValue = $wifiResult.Stdout.Trim()
    Assert-Condition ($wifiValue -cin @("0", "1", "null", "")) `
        "wifi_setting_readback_invalid" `
        "Wireless Debugging setting readback was not bounded."
    $persistentResult = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @(
            "shell", "getprop", "persist.adb.tls_server.enable")
    $persistentValue = $persistentResult.Stdout.Trim().ToLowerInvariant()
    Assert-Condition (
        $persistentValue -cin @("", "0", "1", "false", "true", "null")
    ) "wifi_persistent_readback_invalid" `
        "Wireless Debugging's persistent owner flag was not bounded."

    $tcpResult = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "getprop", "service.adb.tcp.port")
    $tlsResult = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "getprop", "service.adb.tls.port")
    $tcp = Convert-AdbPortState -Value $tcpResult.Stdout `
        -PropertyName "service.adb.tcp.port"
    $tls = Convert-AdbPortState -Value $tlsResult.Stdout `
        -PropertyName "service.adb.tls.port"

    # Resolve the exact adbd process's socket FDs against the kernel TCP
    # tables twice. Each complete sample is bounded by the same PID,
    # /proc/<pid>/stat start time, and complete socket-FD inode set before and
    # after the TCP tables. Only identical consecutive owner projections can
    # prove presence or absence.
    $ownerSocketCommand = @'
command -v awk >/dev/null 2>&1 || exit 40
printf "rusty.fleet.adbd_socket_owner.v3\n"
for sample in 1 2; do
  pre_pid=$(pidof adbd) || exit 41
  case "$pre_pid" in ''|*[!0-9]*) exit 42 ;; esac
  pre_starttime=$(awk -v expected="$pre_pid" '
    NF >= 22 && $1 == expected && $2 == "(adbd)" &&
      $22 ~ /^[0-9]+$/ { print $22; found=1 }
    END { if (!found) exit 1 }
  ' /proc/"$pre_pid"/stat) || exit 43
  case "$pre_starttime" in ''|*[!0-9]*) exit 43 ;; esac

  printf "sample_begin=%s\npre_pid=%s\npre_starttime=%s\npre_fd_begin=1\n" "$sample" "$pre_pid" "$pre_starttime"
  for fd in /proc/"$pre_pid"/fd/*; do
    link=$(readlink "$fd") || exit 44
    case "$link" in
      socket:\[*\])
        inode=$(printf "%s" "$link" | awk -F'[][]' '{print $2}')
        case "$inode" in ''|*[!0-9]*) exit 45 ;; esac
        printf "pre_inode=%s\n" "$inode"
        ;;
    esac
  done
  printf "pre_fd_end=1\ntcp4_begin=1\n"
  awk 'NR > 1 { print "tcp4=" $2 "," $3 "," $4 "," $10 }' /proc/net/tcp || exit 47
  printf "tcp4_end=1\ntcp6_begin=1\n"
  awk 'NR > 1 { print "tcp6=" $2 "," $3 "," $4 "," $10 }' /proc/net/tcp6 || exit 48
  printf "tcp6_end=1\npost_fd_begin=1\n"

  for fd in /proc/"$pre_pid"/fd/*; do
    link=$(readlink "$fd") || exit 49
    case "$link" in
      socket:\[*\])
        inode=$(printf "%s" "$link" | awk -F'[][]' '{print $2}')
        case "$inode" in ''|*[!0-9]*) exit 50 ;; esac
        printf "post_inode=%s\n" "$inode"
        ;;
    esac
  done
  printf "post_fd_end=1\n"

  post_pid=$(pidof adbd) || exit 41
  case "$post_pid" in ''|*[!0-9]*) exit 42 ;; esac
  post_starttime=$(awk -v expected="$post_pid" '
    NF >= 22 && $1 == expected && $2 == "(adbd)" &&
      $22 ~ /^[0-9]+$/ { print $22; found=1 }
    END { if (!found) exit 1 }
  ' /proc/"$post_pid"/stat) || exit 43
  case "$post_starttime" in ''|*[!0-9]*) exit 43 ;; esac
  printf "post_pid=%s\npost_starttime=%s\nsample_end=%s\n" "$post_pid" "$post_starttime" "$sample"
done
'@
    $ownerSocketCommand =
        $ownerSocketCommand.Replace("`r`n", "`n").Replace("`r", "`n")
    $ownerSocketsResult = Invoke-AdbExact `
        -Context $Context -Device $Device `
        -Arguments @("shell", "sh", "-c", $ownerSocketCommand) `
        -AllowedExitCodes (@(0) + @(40..50))
    $ownerSockets = if ($ownerSocketsResult.ExitCode -eq 0) {
        ConvertFrom-ClosedAdbdSocketOwnerReadback `
            -Text $ownerSocketsResult.Stdout
    } else {
        New-UnknownAdbdSocketOwnerReadback
    }

    $manager = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "dumpsys", "adb")
    $managerText = $manager.Stdout.Trim()
    $managerReadback = ConvertFrom-ClosedAdbManagerDump -Text $managerText

    $addressResult = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @(
            "shell", "ip", "-o", "-4", "addr", "show", "up", "scope", "global")
    $targetAddresses = @()
    foreach ($line in @($addressResult.Stdout -split "`r?`n")) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        Assert-Condition (
            $line -cmatch '^\d+:\s+\S+\s+inet\s+([0-9.]+)/[0-9]+\s+'
        ) "adb_target_address_readback_invalid" `
            "The target's global IPv4 owner projection was malformed."
        $address = $null
        Assert-Condition (
            [Net.IPAddress]::TryParse([string]$Matches[1], [ref]$address) -and
            $address.AddressFamily -eq
                [Net.Sockets.AddressFamily]::InterNetwork
        ) "adb_target_address_readback_invalid" `
            "The target returned an invalid global IPv4 address."
        $targetAddresses += $address.ToString()
    }
    $serialResult = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "getprop", "ro.serialno")
    $platformSerial = $serialResult.Stdout.Trim()
    Assert-Condition (
        $platformSerial -cmatch '^[A-Za-z0-9._:-]{1,128}$'
    ) "adb_platform_serial_readback_invalid" `
        "The target returned no bounded platform serial."

    $mdnsResult = Invoke-AdbHostExact -Context $Context `
        -Arguments @("mdns", "services")
    $mdnsReadback = ConvertFrom-ClosedAdbMdnsServices `
        -Text $mdnsResult.Stdout
    if ($targetAddresses.Count -eq 0) {
        $mdnsReadback = [pscustomobject]@{
            ParseState = "unknown"
            Services = @()
        }
    }
    $instancePrefix = "adb-$platformSerial"
    $targetServices = @($mdnsReadback.Services | Where-Object {
        $targetAddresses -contains [string]$_.Address -or
        [string]$_.Instance -ceq $instancePrefix -or
        ([string]$_.Instance).StartsWith(
            "$instancePrefix-", [StringComparison]::Ordinal)
    })
    $networkFacts = Resolve-AdbOwnerNetworkFacts `
        -Manager $managerReadback -Mdns $mdnsReadback `
        -OwnerSockets $ownerSockets `
        -Tcp $tcp -Tls $tls -WifiSetting $wifiValue `
        -PersistentTlsSetting $persistentValue `
        -TargetServices $targetServices

    $forwards = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("forward", "--list")
    $forwardCount = 0
    foreach ($line in @($forwards.Stdout -split "`r?`n" |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) })) {
        $parts = @($line.Trim() -split '\s+')
        Assert-Condition ($parts.Count -eq 3) "adb_forward_readback_invalid" `
            "ADB returned a malformed forward projection."
        if ($parts[0] -ceq [string]$Device.usb_serial) {
            $forwardCount++
        }
    }
    $reverses = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("reverse", "--list")
    $reverseCount = 0
    foreach ($line in @($reverses.Stdout -split "`r?`n" |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) })) {
        $parts = @($line.Trim() -split '\s+')
        Assert-Condition ($parts.Count -in @(2, 3)) `
            "adb_reverse_readback_invalid" `
            "ADB returned a malformed reverse projection."
        $reverseCount++
    }

    return [pscustomobject]@{
        WifiSettingEnabled = $wifiValue -ceq "1"
        TcpPortState = [string]$tcp.State
        TlsPortState = [string]$tls.State
        ListenerState = [string]$networkFacts.ListenerState
        WirelessSessionState = [string]$networkFacts.WirelessSessionState
        WirelessPendingState = [string]$networkFacts.WirelessPendingState
        AdbManagerFormat = [string]$managerReadback.Format
        AdbRetainedPairingState =
            [string]$managerReadback.RetainedPairingState
        AdbRetainedPairingSha256 =
            [string]$managerReadback.RetainedPairingSha256
        HostForwardCount = $forwardCount
        HostReverseCount = $reverseCount
        AdbManagerStateSha256 = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes($managerText))
    }
}

function Get-QfmDirectLinkObservation {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device
    )
    $enrollment = Read-StrictJsonFile `
        -Path ([string]$Device.qfm_enrollment_path) `
        -Context "QFM enrollment direct-link readback" -MaximumBytes 4096
    try {
        Assert-ExactProperties -Value $enrollment.Value -Required @(
            "schema", "target", "device_id", "usb_serial", "endpoint",
            "pairing_code"
        ) -Context "QFM enrollment direct-link readback"
        Assert-Condition (
            [string]$enrollment.Value.device_id -ceq
                [string]$Device.device_id -and
            [string]$enrollment.Value.usb_serial -ceq
                [string]$Device.usb_serial
        ) "qfm_direct_link_binding_mismatch" `
            "The private Kiosk direct link did not bind the exact target."
        $pairing = [string]$enrollment.Value.pairing_code
        $endpoint = [string]$enrollment.Value.endpoint
        $result = Invoke-QfmExact -Context $Context -Arguments @(
            "kiosk-direct", "status",
            "--endpoint", $endpoint,
            "--json"
        ) -Environment @{ RUSTY_KIOSK_PAIRING_CODE=$pairing }
        Assert-Condition (
            [string]$result.transport -ceq "direct" -and
            $null -ne $result.status -and
            $null -ne $result.kiosk
        ) "qfm_direct_link_readback_invalid" `
            "QFM did not confirm the configured direct Kiosk route."
        return "confirmed"
    } finally {
        $enrollment = $null
        $pairing = $null
        $endpoint = $null
        $result = $null
    }
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

    $network = Get-AdbNetworkObservation -Context $Context -Device $Device

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
    $directObservation = Get-QfmDirectLinkObservation `
        -Context $Context -Device $Device

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
        wifi_setting_enabled = $network.WifiSettingEnabled
        transport_usb_present = $true
        adb_tcp_port_state = $network.TcpPortState
        adb_tls_port_state = $network.TlsPortState
        adb_listener_state = $network.ListenerState
        wireless_session_state = $network.WirelessSessionState
        wireless_pending_state = $network.WirelessPendingState
        host_forward_count = $network.HostForwardCount
        host_reverse_count = $network.HostReverseCount
        adb_manager_format = $network.AdbManagerFormat
        adb_retained_pairing_state = $network.AdbRetainedPairingState
        adb_retained_pairing_sha256 = $network.AdbRetainedPairingSha256
        adb_manager_state_sha256 = $network.AdbManagerStateSha256
        helper_status_state = if ($null -ne $helperStatus) {
            "confirmed"
        } else {
            "absent"
        }
        helper_in_flight = if ($null -ne $helperStatus) {
            [bool]$helperStatus.in_flight
        } else {
            $false
        }
        helper_proof_listener_discovered = if ($null -ne $helperStatus) {
            [bool]$helperStatus.proof_fresh
        } else {
            $false
        }
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
            installed = "not_evaluated"
            reachable = "not_evaluated"
            authorized = "not_evaluated"
            effective = "not_evaluated"
        }
    }
}

function Get-PhysicalIsolationProjection {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")]
        [string] $Slot
    )
    $device = Get-DeviceBySlot -Context $Context -Slot $Slot
    $snapshot = Get-DeviceSnapshot -Context $Context -Device $device
    $stable = [ordered]@{
        package_set_sha256 = $snapshot.package_set_sha256
        packages = $snapshot.packages
        helper_grants = $snapshot.helper_grants
        kiosk_helper_write_secure_settings_granted =
            $snapshot.kiosk_helper_write_secure_settings_granted
        qfm_profile_state = $snapshot.qfm_profile_state
        kiosk_direct_link_observation =
            $snapshot.kiosk_direct_link_observation
        after_boot_enabled = $snapshot.after_boot_enabled
        wifi_setting_enabled = $snapshot.wifi_setting_enabled
        adb_tcp_port_state = $snapshot.adb_tcp_port_state
        adb_tls_port_state = $snapshot.adb_tls_port_state
        adb_listener_state = $snapshot.adb_listener_state
        wireless_session_state = $snapshot.wireless_session_state
        wireless_pending_state = $snapshot.wireless_pending_state
        host_forward_count = $snapshot.host_forward_count
        host_reverse_count = $snapshot.host_reverse_count
        adb_manager_format = $snapshot.adb_manager_format
        adb_retained_pairing_state = $snapshot.adb_retained_pairing_state
        adb_retained_pairing_sha256 =
            $snapshot.adb_retained_pairing_sha256
        adb_manager_state_sha256 = $snapshot.adb_manager_state_sha256
        helper_status_state = $snapshot.helper_status_state
        helper_in_flight = $snapshot.helper_in_flight
        helper_proof_listener_discovered =
            $snapshot.helper_proof_listener_discovered
        agent_process_present = $snapshot.agent_process_present
        agent_private_inputs_absent = $snapshot.agent_private_inputs_absent
        boot_id_sha256 = $snapshot.boot_id_sha256
        termux_process_epoch_sha256 =
            $snapshot.termux_process_epoch_sha256
    }
    return [pscustomobject]@{
        BootIdSha256 = $snapshot.boot_id_sha256
        BootElapsedMilliseconds = $snapshot.boot_elapsed_milliseconds
        ProjectionSha256 = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes(
                ($stable | ConvertTo-Json -Depth 12 -Compress)))
    }
}

function Assert-PhysicalNonTargetIsolation {
    param(
        [Parameter(Mandatory)][object] $Before,
        [Parameter(Mandatory)][object] $After
    )
    Assert-Condition (
        $After.BootIdSha256 -ceq $Before.BootIdSha256 -and
        $After.BootElapsedMilliseconds -ge
            $Before.BootElapsedMilliseconds -and
        $After.ProjectionSha256 -ceq $Before.ProjectionSha256
    ) "cross_device_physical_state_changed" `
        "A non-target device changed boot, profile, helper, permission, package, process, private-input, listener, route, or transport state."
}

function Get-DurableMutationIsolationProjection {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b", "both")]
        [string] $Scope
    )
    $slots = if ($Scope -ceq "both") {
        @("device_a", "device_b")
    } else {
        @($Scope)
    }
    $stableParts = @()
    $bootParts = @()
    $elapsedA = 0L
    $elapsedB = 0L
    foreach ($slot in $slots) {
        $projection = Get-PhysicalIsolationProjection `
            -Context $Context -Slot $slot
        $stableParts += "$slot`0$($projection.ProjectionSha256)"
        $bootParts += "$slot`0$($projection.BootIdSha256)"
        if ($slot -ceq "device_a") {
            $elapsedA = [long]$projection.BootElapsedMilliseconds
        } else {
            $elapsedB = [long]$projection.BootElapsedMilliseconds
        }
    }
    return [pscustomobject]@{
        ProjectionSha256 = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes(($stableParts -join "`0")))
        BootSha256 = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes(($bootParts -join "`0")))
        ElapsedA = $elapsedA
        ElapsedB = $elapsedB
    }
}

function Set-DurableMutationIsolationAfter {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    Assert-Condition ($null -ne $State.mutation) "mutation_absent" `
        "No durable mutation is available for isolation readback."
    $scope = [string]$State.mutation.isolation_scope
    if ($scope -ceq "modeled_not_executed") {
        return
    }
    $after = Get-DurableMutationIsolationProjection `
        -Context $Context -Scope $scope
    Assert-Condition (
        [string]$after.ProjectionSha256 -ceq
            [string]$State.mutation.isolation_before_sha256 -and
        [string]$after.BootSha256 -ceq
            [string]$State.mutation.isolation_before_boot_sha256 -and
        (
            [long]$State.mutation.isolation_before_elapsed_a_ms -eq 0 -or
            [long]$after.ElapsedA -ge
                [long]$State.mutation.isolation_before_elapsed_a_ms
        ) -and
        (
            [long]$State.mutation.isolation_before_elapsed_b_ms -eq 0 -or
            [long]$after.ElapsedB -ge
                [long]$State.mutation.isolation_before_elapsed_b_ms
        )
    ) "durable_mutation_isolation_changed" `
        "A mutation changed a device outside its exact target scope."
    $State.mutation.isolation_after_sha256 = $after.ProjectionSha256
    $State.mutation.isolation_after_boot_sha256 = $after.BootSha256
    $State.mutation.isolation_after_elapsed_a_ms = $after.ElapsedA
    $State.mutation.isolation_after_elapsed_b_ms = $after.ElapsedB
    $State.mutation.journal_sha256 =
        Get-MutationJournalSha256 -Mutation $State.mutation
    Write-SanitizedState -Context $Context -State $State
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
        Assert-Condition (
            [string]$snapshot.adb_tcp_port_state -ceq "inactive" -and
            [string]$snapshot.adb_tls_port_state -ceq "inactive" -and
            [string]$snapshot.adb_listener_state -ceq "absent" -and
            [string]$snapshot.wireless_session_state -ceq "absent" -and
            [string]$snapshot.wireless_pending_state -ceq "absent"
        ) "unsafe_initial_adb_network_state" `
            "Acceptance refuses any classic, TLS, listener, session, or pending Wireless Debugging state."
        Assert-Condition (
            [int]$snapshot.host_forward_count -eq 0 -and
            [int]$snapshot.host_reverse_count -eq 0
        ) "unsafe_initial_adb_tunnel_state" `
            "Acceptance refuses pre-existing host forwards or device reverses for either target."
        Assert-Condition (-not $snapshot.after_boot_enabled) `
            "unsafe_initial_after_boot_state" `
            "Acceptance requires the after-boot request initially disabled."
        Assert-Condition (
            -not $snapshot.helper_in_flight -and
            -not $snapshot.helper_proof_listener_discovered
        ) "unsafe_initial_helper_session_state" `
            "Acceptance refuses a helper mutation in flight or a fresh pre-existing loopback proof."
        Assert-Condition ([string]$snapshot.qfm_profile_state -ceq "absent") `
            "preexisting_private_profile" `
            "Acceptance fails closed when a Fleet connectivity profile already exists."
        Assert-Condition (
            [string]$snapshot.kiosk_direct_link_observation -ceq "confirmed"
        ) "kiosk_direct_link_unconfirmed" `
            "Acceptance requires a fresh read-only confirmation of the configured Kiosk route."
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
    $permissionFacts = [ordered]@{}
    foreach ($package in @($script:HelperPackage, $script:KioskHelperPackage)) {
        $permissionFacts[$package] = [ordered]@{}
        if ([bool]$packageFacts[$package]) {
            $dump = Invoke-AdbExact -Context $Context -Device $device `
                -Arguments @("shell", "dumpsys", "package", $package)
            $permissions = if ($package -ceq $script:HelperPackage) {
                $script:HelperPermissions
            } else {
                @($script:WriteSecureSettingsPermission)
            }
            foreach ($permission in $permissions) {
                $permissionFacts[$package][$permission] =
                    Get-PermissionGrant -PackageDump $dump.Stdout `
                        -Permission $permission
            }
        }
    }
    $network = Get-AdbNetworkObservation -Context $Context -Device $device
    $transport = Invoke-AdbExact -Context $Context -Device $device `
        -Arguments @("get-state")
    $helperStatus = if ([bool]$packageFacts[$script:HelperPackage]) {
        Invoke-HelperExact -Context $Context -Device $device `
            -HelperAction "status"
    } else {
        $null
    }
    if ($null -ne $helperStatus) {
        Assert-Condition (
            [string]$helperStatus.schema -ceq
                "quest-termux-lab.wireless-adb-operator-receipt.v1"
        ) "isolation_helper_status_invalid" `
            "Isolation received an invalid helper status projection."
    }
    $qfmProfile = Invoke-QfmExact -Context $Context -Arguments @(
        "connectivity-profile", "status",
        "--device-id", [string]$device.device_id,
        "--json"
    )
    Assert-Condition (
        [string]$qfmProfile.state -cin @("enrolled", "absent", "invalid")
    ) "isolation_qfm_profile_invalid" `
        "Isolation received an invalid QFM profile projection."
    $directLink = Get-QfmDirectLinkObservation `
        -Context $Context -Device $device
    $agentPrivateInputsAbsent = $true
    if ([bool]$packageFacts[$script:FleetAgentPackage]) {
        $privateInputs = Invoke-AdbExact -Context $Context -Device $device `
            -Arguments @(
                "shell", "run-as", $script:FleetAgentPackage,
                "sh", "-c", "test ! -e files/fleet-agent"
            ) -AllowedExitCodes @(0, 1)
        $agentPrivateInputsAbsent = $privateInputs.ExitCode -eq 0
    }
    $boot = Get-DeviceBootIdentity -Context $Context -Device $device
    $termuxEpoch = Get-TermuxProcessEpochSha256 `
        -Context $Context -Device $device
    $physicalJson = [ordered]@{
        packages = $packageFacts
        permissions = $permissionFacts
        processes = $processFacts
        qfm_profile_state = [string]$qfmProfile.state
        kiosk_direct_link = $directLink
        helper_status_state = if ($null -eq $helperStatus) {
            "absent"
        } else {
            [ordered]@{
                boot_attempt_enabled = $helperStatus.boot_attempt_enabled -eq $true
                wireless_debugging_enabled =
                    $helperStatus.wireless_debugging_enabled -eq $true
                in_flight = $helperStatus.in_flight -eq $true
                proof_fresh = $helperStatus.proof_fresh -eq $true
            }
        }
        wireless_setting = $network.WifiSettingEnabled
        adb_tcp_port_state = $network.TcpPortState
        adb_tls_port_state = $network.TlsPortState
        adb_listener_state = $network.ListenerState
        wireless_session_state = $network.WirelessSessionState
        wireless_pending_state = $network.WirelessPendingState
        host_forward_count = $network.HostForwardCount
        host_reverse_count = $network.HostReverseCount
        adb_manager_format = $network.AdbManagerFormat
        adb_retained_pairing_state = $network.AdbRetainedPairingState
        adb_retained_pairing_sha256 =
            $network.AdbRetainedPairingSha256
        adb_manager_state_sha256 = $network.AdbManagerStateSha256
        usb_transport = $transport.Stdout.Trim()
        agent_private_inputs_absent = $agentPrivateInputsAbsent
        boot_id_sha256 = $boot.BootIdSha256
        termux_process_epoch_sha256 = $termuxEpoch
    } | ConvertTo-Json -Depth 8 -Compress
    return [pscustomobject]@{
        AcceptedRevision = [long]$inspect.row.accepted_revision
        SourceEpochSha256 = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes([string]$inspect.row.source_epoch))
        OperationLineageSha256 = Get-BytesSha256 -Bytes (
            [Text.Encoding]::UTF8.GetBytes($lineageJson))
        OperationCount = $lineage.Count
        BootIdSha256 = $boot.BootIdSha256
        BootElapsedMilliseconds = $boot.ElapsedMilliseconds
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
        $After.BootIdSha256 -ceq $Before.BootIdSha256 -and
        $After.BootElapsedMilliseconds -ge
            $Before.BootElapsedMilliseconds -and
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
        $otherSlot = if ($slot -ceq "device_a") {
            "device_b"
        } else {
            "device_a"
        }
        $otherBefore = Get-PhysicalIsolationProjection `
            -Context $Context -Slot $otherSlot
        try {
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
        } finally {
            $otherAfter = Get-PhysicalIsolationProjection `
                -Context $Context -Slot $otherSlot
            Assert-PhysicalNonTargetIsolation `
                -Before $otherBefore -After $otherAfter
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
        $otherSlot = if ($slot -ceq "device_a") {
            "device_b"
        } else {
            "device_a"
        }
        $otherBefore = Get-PhysicalIsolationProjection `
            -Context $Context -Slot $otherSlot
        try {
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
        } finally {
            $otherAfter = Get-PhysicalIsolationProjection `
                -Context $Context -Slot $otherSlot
            Assert-PhysicalNonTargetIsolation `
                -Before $otherBefore -After $otherAfter
        }
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
            $State.claims.installed = "confirmed"
            $State.claims.reachable = "confirmed"
            $State.claims.authorized = "confirmed"
            $State.claims.effective = "confirmed"
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

function Resolve-InterruptedCleanupMutation {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    if ($null -eq $State.mutation) {
        return
    }
    if ([string]$State.mutation.stage -ceq "confirmed") {
        Complete-DurableMutationTerminal -Context $Context -State $State `
            -ReconciliationCode "confirmed_record_recovered_after_restart"
        return
    }
    Set-DurableMutationCleanupRequired -Context $Context -State $State `
        -ReasonCode "interrupted_cleanup_no_blind_redispatch"
    Complete-DurableMutationTerminal -Context $Context -State $State `
        -ReconciliationCode "cleanup_required_fresh_final_readback_pending"
}

function Invoke-JournaledCleanupStep {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][Collections.IDictionary] $Checks,
        [Parameter(Mandatory)][string] $Name,
        [ValidateSet("none", "device_a", "device_b")][string] $Slot = "none",
        [string] $OwnerId = "acceptance-cleanup",
        [Parameter(Mandatory)][scriptblock] $Operation
    )
    if ($Checks.Contains($Name)) {
        return
    }
    $kind = "cleanup-" + ($Name -replace '[^a-z0-9-]+', '-').ToLowerInvariant()
    [void](Start-DurableMutation -Context $Context -State $State `
        -Kind $kind -ActionId "acceptance.cleanup.$Name" `
        -Slot $Slot -OwnerId $OwnerId -CleanupOwner $OwnerId)
    Set-DurableMutationSent -Context $Context -State $State
    try {
        $Checks[$Name] = (& $Operation) -eq $true
        $State.cleanup.checks = $Checks
        Write-SanitizedState -Context $Context -State $State
        if ($Checks[$Name]) {
            Complete-DurableMutation -Context $Context -State $State `
                -ReconciliationCode "cleanup_exact_readback_confirmed"
        } else {
            Set-DurableMutationCleanupRequired `
                -Context $Context -State $State `
                -ReasonCode "cleanup_exact_readback_failed"
            Complete-DurableMutationTerminal `
                -Context $Context -State $State `
                -ReconciliationCode "cleanup_step_partial_failure"
        }
    } catch {
        $Checks[$Name] = $false
        $State.cleanup.checks = $Checks
        Write-SanitizedState -Context $Context -State $State
        if ($null -ne $State.mutation) {
            Set-DurableMutationCleanupRequired `
                -Context $Context -State $State `
                -ReasonCode "cleanup_owner_or_readback_failed"
            Complete-DurableMutationTerminal `
                -Context $Context -State $State `
                -ReconciliationCode "cleanup_step_exception"
        }
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
    [void](Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @(
            "shell", "run-as", $script:FleetAgentPackage,
            "rmdir", "files/fleet-agent"
        ) -AllowedExitCodes @(0, 1))
    $readback = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @(
            "shell", "run-as", $script:FleetAgentPackage,
            "sh", "-c", "test ! -e files/fleet-agent"
        ) -AllowedExitCodes @(0, 1)
    return $result.ExitCode -in @(0, 1) -and $readback.ExitCode -eq 0
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

function Add-FinalCleanupReadback {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][Collections.IDictionary] $Checks
    )
    $unknown = $false
    foreach ($slot in @("device_a", "device_b")) {
        $device = Get-DeviceBySlot -Context $Context -Slot $slot
        $stateDevice = Get-StateDevice -State $State -Slot $slot
        foreach ($name in @(
            "$slot-final-packages-restored",
            "$slot-final-permissions-restored",
            "$slot-final-helper-inactive",
            "$slot-final-wireless-listener-absent",
            "$slot-final-agent-absent",
            "$slot-final-agent-private-inputs-absent",
            "$slot-final-qfm-profile-restored",
            "$slot-final-kiosk-route-confirmed",
            "$slot-final-boot-readback-fresh"
        )) {
            $Checks[$name] = $false
        }
        try {
            $fresh = Get-DeviceSnapshot -Context $Context -Device $device
            $packagesRestored = $true
            foreach ($package in $script:ManagedPackages) {
                if (
                    [bool]$fresh.packages[$package] -ne
                    [bool]$stateDevice.snapshot.packages[$package]
                ) {
                    $packagesRestored = $false
                }
            }
            $Checks["$slot-final-packages-restored"] = $packagesRestored

            $permissionsRestored = $true
            if ([bool]$stateDevice.snapshot.packages[$script:HelperPackage]) {
                foreach ($permission in $script:HelperPermissions) {
                    if (
                        [bool]$fresh.helper_grants[$permission] -ne
                        [bool]$stateDevice.snapshot.helper_grants[$permission]
                    ) {
                        $permissionsRestored = $false
                    }
                }
            }
            if (
                [bool]$stateDevice.snapshot.packages[$script:KioskHelperPackage] -and
                [bool]$fresh.kiosk_helper_write_secure_settings_granted -ne
                [bool]$stateDevice.snapshot.kiosk_helper_write_secure_settings_granted
            ) {
                $permissionsRestored = $false
            }
            $Checks["$slot-final-permissions-restored"] =
                $permissionsRestored

            $helperInactive = -not [bool]$fresh.packages[$script:HelperPackage]
            if ([bool]$fresh.packages[$script:HelperPackage]) {
                $helper = Invoke-HelperExact -Context $Context -Device $device `
                    -HelperAction "status"
                $helperInactive =
                    [string]$helper.schema -ceq
                        "quest-termux-lab.wireless-adb-operator-receipt.v1" -and
                    $helper.boot_attempt_enabled -eq $false -and
                    $helper.wireless_debugging_enabled -eq $false -and
                    $helper.in_flight -eq $false -and
                    $helper.proof_fresh -eq $false
            }
            $Checks["$slot-final-helper-inactive"] = $helperInactive
            $Checks["$slot-final-wireless-listener-absent"] =
                $fresh.wifi_setting_enabled -eq $false -and
                [string]$fresh.adb_tcp_port_state -ceq "inactive" -and
                [string]$fresh.adb_tls_port_state -ceq "inactive" -and
                [string]$fresh.adb_listener_state -ceq "absent" -and
                [string]$fresh.wireless_session_state -ceq "absent" -and
                [string]$fresh.wireless_pending_state -ceq "absent" -and
                [string]$fresh.adb_manager_format -ceq
                    "android.debugging_manager.text.v1" -and
                [string]$fresh.adb_retained_pairing_state -ceq
                    [string]$stateDevice.snapshot.adb_retained_pairing_state -and
                [string]$fresh.adb_retained_pairing_sha256 -ceq
                    [string]$stateDevice.snapshot.adb_retained_pairing_sha256 -and
                [int]$fresh.host_forward_count -eq 0 -and
                [int]$fresh.host_reverse_count -eq 0
            if ($Checks["$slot-final-wireless-listener-absent"]) {
                $stateDevice.acceptance.termux_usable = $false
            }
            $Checks["$slot-final-agent-absent"] =
                $fresh.agent_process_present -eq $false
            $Checks["$slot-final-agent-private-inputs-absent"] =
                $fresh.agent_private_inputs_absent -eq $true
            $Checks["$slot-final-qfm-profile-restored"] =
                [string]$fresh.qfm_profile_state -ceq
                    [string]$stateDevice.snapshot.qfm_profile_state
            $Checks["$slot-final-kiosk-route-confirmed"] =
                [string]$fresh.kiosk_direct_link_observation -ceq "confirmed"
            $Checks["$slot-final-boot-readback-fresh"] =
                [string]$fresh.boot_id_sha256 -cmatch '^[0-9a-f]{64}$' -and
                [long]$fresh.boot_elapsed_milliseconds -ge 0
        } catch {
            $unknown = $true
        }
        $State.cleanup.checks = $Checks
        Write-SanitizedState -Context $Context -State $State
    }

    $Checks["final-hub-process-absent"] = try {
        -not [bool]$State.hub.started_by_run -or
        [int]$State.hub.process_id -le 0 -or
        $null -eq (Get-Process -Id ([int]$State.hub.process_id) `
            -ErrorAction SilentlyContinue)
    } catch {
        $unknown = $true
        $false
    }
    $ruleName = "RustyFleet-WifiAdb-" + $Context.Sha256.Substring(0, 12)
    $Checks["final-firewall-rule-absent"] = try {
        $null -eq (Get-NetFirewallRule -DisplayName $ruleName `
            -ErrorAction SilentlyContinue)
    } catch {
        $unknown = $true
        $false
    }
    $Checks["final-onboarding-private-material-absent"] = @(
        Get-OnboardingGeneratedPaths -Context $Context |
            Where-Object { Test-Path -LiteralPath $_ }
    ).Count -eq 0
    $Checks["final-runtime-stage-absent"] = -not (
        Test-Path -LiteralPath (
            Join-Path ([string]$Context.Config.private_state_root) "runtime")
    )
    $State.cleanup.checks = $Checks
    Write-SanitizedState -Context $Context -State $State
    return $unknown
}

function Set-ClaimsFromFinalCleanupReadback {
    param(
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][Collections.IDictionary] $Checks,
        [Parameter(Mandatory)][bool] $Unknown
    )
    if ($Unknown) {
        $State.claims.installed = "unknown"
        $State.claims.reachable = "unknown"
        $State.claims.authorized = "unknown"
        $State.claims.effective = "unknown"
        return
    }
    $packageChecks = @($Checks.GetEnumerator() | Where-Object {
        [string]$_.Key -clike "*-final-packages-restored"
    })
    $reachabilityChecks = @($Checks.GetEnumerator() | Where-Object {
        [string]$_.Key -clike "*-final-wireless-listener-absent" -or
        [string]$_.Key -clike "*-final-agent-absent" -or
        [string]$_.Key -ceq "final-hub-process-absent" -or
        [string]$_.Key -ceq "final-firewall-rule-absent"
    })
    $authorityChecks = @($Checks.GetEnumerator() | Where-Object {
        [string]$_.Key -clike "*-final-permissions-restored" -or
        [string]$_.Key -clike "*-final-agent-private-inputs-absent" -or
        [string]$_.Key -clike "*-final-qfm-profile-restored"
    })
    $allComplete = @($Checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
    $State.claims.installed = if (
        @($packageChecks | Where-Object { $_.Value -ne $true }).Count -eq 0
    ) { "not_claimed" } else { "partial" }
    $State.claims.reachable = if (
        @($reachabilityChecks | Where-Object { $_.Value -ne $true }).Count -eq 0
    ) { "not_claimed" } else { "partial" }
    $State.claims.authorized = if (
        @($authorityChecks | Where-Object { $_.Value -ne $true }).Count -eq 0
    ) { "not_claimed" } else { "partial" }
    $State.claims.effective = if ($allComplete) {
        "not_claimed"
    } else {
        "partial"
    }
}

function Invoke-AcceptanceCleanup {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State
    )
    if ([string]$State.cleanup.status -ceq "complete") {
        if ([string]$State.agent_board_reservation -cne "released") {
            [void](Release-AgentBoardReservation `
                -Context $Context -State $State)
        }
        return $State
    }
    [void](Ensure-AgentBoardReservation `
        -Context $Context -State $State -AllowRepair)
    $State.cleanup.attempted = $true
    $State.cleanup.status = "running"
    $State.status = "cleanup_running"
    Write-SanitizedState -Context $Context -State $State
    Resolve-InterruptedCleanupMutation -Context $Context -State $State
    $checks = [ordered]@{}
    foreach ($entry in $State.cleanup.checks.GetEnumerator()) {
        $checks[[string]$entry.Key] = $entry.Value
    }

    foreach ($slot in @("device_a", "device_b")) {
        $device = Get-DeviceBySlot -Context $Context -Slot $slot
        $stateDevice = Get-StateDevice -State $State -Slot $slot
        $helperExpectedPresent =
            [bool]$stateDevice.snapshot.packages[$script:HelperPackage] -or
            @($stateDevice.run_owned.added_packages) -contains
                $script:HelperPackage

        Invoke-JournaledCleanupStep -Context $Context -State $State `
            -Checks $checks -Name "$slot-disable-boot" -Slot $slot `
            -OwnerId "wireless-adb-helper" -Operation {
            if (-not $helperExpectedPresent) {
                return $true
            }
            $receipt = Invoke-HelperExact -Context $Context -Device $device `
                -HelperAction "disable-boot-attempt" -Confirm
            return $receipt.accepted -eq $true
        }
        Invoke-JournaledCleanupStep -Context $Context -State $State `
            -Checks $checks -Name "$slot-disable-wireless" -Slot $slot `
            -OwnerId "wireless-adb-helper" -Operation {
            if (-not $helperExpectedPresent) {
                return $true
            }
            $receipt = Invoke-HelperExact -Context $Context -Device $device `
                -HelperAction "disable-wireless" -Confirm
            return $receipt.accepted -eq $true
        }
        Invoke-JournaledCleanupStep -Context $Context -State $State `
            -Checks $checks -Name "$slot-proof-absent" -Slot $slot `
            -OwnerId "fleet-hub" -Operation {
            if ($stateDevice.acceptance.Contains("operation_id") -and
                [string]$stateDevice.acceptance.operation_id) {
                [void](Wait-WifiProofAbsent -Context $Context `
                    -OperationId ([string]$stateDevice.acceptance.operation_id))
            }
            return $true
        }
        Invoke-JournaledCleanupStep -Context $Context -State $State `
            -Checks $checks -Name "$slot-agent-stopped" -Slot $slot `
            -OwnerId "fleet-agent" -Operation {
            if ([bool]$stateDevice.run_owned.agent_started) {
                Stop-FleetAgent -Context $Context -Device $device
            }
            return $true
        }
        Invoke-JournaledCleanupStep -Context $Context -State $State `
            -Checks $checks -Name "$slot-agent-inputs-removed" -Slot $slot `
            -OwnerId "fleet-agent-app-private-storage" -Operation {
            if ([bool]$stateDevice.run_owned.agent_profile_staged -and
                [bool]$stateDevice.snapshot.packages[$script:FleetAgentPackage]) {
                return Remove-AgentPrivateInputs -Context $Context -Device $device
            }
            return $true
        }
        Invoke-JournaledCleanupStep -Context $Context -State $State `
            -Checks $checks -Name "$slot-qfm-profile-restored" -Slot $slot `
            -OwnerId "questionable-file-manager" -Operation {
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
            Invoke-JournaledCleanupStep -Context $Context -State $State `
                -Checks $checks -Slot $slot -OwnerId "android-package-manager" `
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
        Invoke-JournaledCleanupStep -Context $Context -State $State `
            -Checks $checks -Slot $slot -OwnerId "android-package-manager" `
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
            Invoke-JournaledCleanupStep -Context $Context -State $State `
                -Checks $checks -Slot $slot -OwnerId "android-package-manager" `
                -Name "$slot-package-$packageKey-removed" -Operation {
                    return Remove-RunAddedPackage -Context $Context `
                        -Device $device -Package $package
                }
        }
        Invoke-JournaledCleanupStep -Context $Context -State $State `
            -Checks $checks -Slot $slot -OwnerId "questionable-file-manager" `
            -Name "$slot-kiosk-direct-link-unchanged" -Operation {
                return [string](Get-QfmDirectLinkObservation `
                    -Context $Context -Device $device) -ceq "confirmed"
            }
    }

    Invoke-JournaledCleanupStep -Context $Context -State $State `
        -Checks $checks -Name "hub-stopped" -OwnerId "fleet-hub" -Operation {
        return Stop-RunOwnedHub -Context $Context -State $State
    }
    Invoke-JournaledCleanupStep -Context $Context -State $State `
        -Checks $checks -Name "firewall-removed" `
        -OwnerId "windows-firewall" -Operation {
        return Remove-RunFirewallRule -Context $Context -State $State
    }
    Invoke-JournaledCleanupStep -Context $Context -State $State `
        -Checks $checks -Name "onboarding-seeds-removed" `
        -OwnerId "fleet-onboard" -Operation {
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
    Invoke-JournaledCleanupStep -Context $Context -State $State `
        -Checks $checks -Name "runtime-stage-empty" `
        -OwnerId "acceptance-runtime" -Operation {
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

    $finalReadbackUnknown = Add-FinalCleanupReadback `
        -Context $Context -State $State -Checks $checks
    $truth = Get-CleanupTruth -Checks $checks
    $State.cleanup.checks = $checks
    $State.cleanup.status = $truth.Status
    $State.status = if ($truth.Status -ceq "complete") {
        "complete"
    } else {
        "cleanup_partial_failure"
    }
    $State.phase = "cleanup"
    Set-ClaimsFromFinalCleanupReadback -State $State -Checks $checks `
        -Unknown $finalReadbackUnknown
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
    if ($truth.Status -ceq "complete") {
        [void](Release-AgentBoardReservation `
            -Context $Context -State $State)
    }
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
                installed = "not_evaluated"
                reachable = "not_evaluated"
                authorized = "not_evaluated"
                effective = "not_evaluated"
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
            if ($null -ne $state) {
                Set-AgentBoardStatusProjection `
                    -Context $context -State $state
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
            [void](Ensure-AgentBoardReservation `
                -Context $context -State $state -AllowRepair)
            return Invoke-ResumeTransition -Context $context -State $state `
                -ConfirmCurrentCheckpoint:$false
        }
        "Resume" {
            Assert-Condition $ConfirmMutation "mutation_confirmation_required" `
                "Resume requires -ConfirmMutation."
            $state = Read-SanitizedState -Context $context
            Assert-NoAmbiguousMutation -Context $context -State $state
            [void](Assert-AgentBoardReservation `
                -Context $context -State $state)
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
    "Ensure-AgentBoardReservation",
    "Assert-AgentBoardReservation",
    "Release-AgentBoardReservation",
    "Start-DurableMutation",
    "Set-DurableMutationSent",
    "Complete-DurableMutation",
    "Assert-NoAmbiguousMutation",
    "ConvertFrom-ClosedAdbManagerDump",
    "ConvertFrom-ClosedAdbMdnsServices",
    "ConvertFrom-ClosedAdbdSocketOwnerReadback",
    "Resolve-AdbOwnerNetworkFacts"
)
