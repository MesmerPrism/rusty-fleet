# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:ConfigSchema = "rusty.fleet.wifi_adb_two_quest_run_config.v1"
$script:StateSchema = "rusty.fleet.wifi_adb_two_quest_acceptance_state.v1"
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

function Test-TermuxProof {
    [CmdletBinding()]
    param(
        [AllowNull()][object] $Proof,
        [Parameter(Mandatory)][string] $ExpectedDeviceId,
        [Parameter(Mandatory)][long] $ExpectedIdentityRevision,
        [Parameter(Mandatory)][long] $NowMs,
        [long] $MinimumEvidenceRevision = 0
    )
    if ($null -eq $Proof) {
        return [pscustomobject]@{ Valid = $false; ReasonCode = "proof_absent" }
    }
    $valid = (
        [string]$Proof.schema -ceq "rusty.fleet.quest_wifi_adb_termux_proof.v1" -and
        [string]$Proof.owner_id -ceq "quest-termux-lab" -and
        [string]$Proof.device_id -ceq $ExpectedDeviceId -and
        [long]$Proof.identity_revision -eq $ExpectedIdentityRevision -and
        [string]$Proof.route_mode -ceq "modern_tls" -and
        [string]$Proof.discovery_mode -cin @("tls_nsd", "tls_mdns") -and
        $Proof.listener_discovered -eq $true -and
        [string]$Proof.shell_identity -ceq "uid=2000(shell)" -and
        $Proof.available -eq $true -and
        [long]$Proof.evidence_revision -gt $MinimumEvidenceRevision -and
        [long]$Proof.observed_at_ms -le $NowMs -and
        [long]$Proof.fresh_until_ms -ge $NowMs -and
        [long]$Proof.fresh_until_ms -gt [long]$Proof.observed_at_ms -and
        ([long]$Proof.fresh_until_ms - [long]$Proof.observed_at_ms) -le 60000 -and
        [string]$Proof.evidence_sha256 -cmatch '^[0-9a-f]{64}$'
    )
    if ($valid) {
        return [pscustomobject]@{ Valid = $true; ReasonCode = "proof_valid" }
    }
    $reason = if ([string]$Proof.device_id -cne $ExpectedDeviceId) {
        "proof_device_mismatch"
    } elseif ([string]$Proof.shell_identity -cne "uid=2000(shell)") {
        "proof_shell_uid_invalid"
    } elseif ([long]$Proof.fresh_until_ms -lt $NowMs) {
        "proof_stale"
    } elseif ([long]$Proof.evidence_revision -le $MinimumEvidenceRevision) {
        "proof_revision_not_advanced"
    } else {
        "proof_invalid"
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
    $temporary = Join-Path $root "acceptance-state.pending.json"
    [IO.File]::WriteAllText(
        $temporary,
        $json + [Environment]::NewLine,
        [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporary -Destination $path -Force
}

function Read-SanitizedState {
    param([Parameter(Mandatory)][object] $Context)
    $path = Get-StatePath -Context $Context
    Assert-Condition (Test-Path -LiteralPath $path -PathType Leaf) `
        "state_missing" "Preflight has not created acceptance state."
    $state = Get-Content -LiteralPath $path -Raw |
        ConvertFrom-Json -AsHashtable -Depth 32
    Assert-Condition (
        [string]$state.schema -ceq $script:StateSchema -and
        [string]$state.config_sha256 -ceq [string]$Context.Sha256 -and
        [string]$state.run_id_hash -ceq
            (Get-BytesSha256 -Bytes ([Text.Encoding]::UTF8.GetBytes(
                [string]$Context.Config.run_id)))
    ) "resume_config_mismatch" `
        "The private run config does not match this resumable state."
    return $state
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
    $directObservation = if ($profileState -ceq "enrolled") {
        "owner_profile_enrolled"
    } else {
        "owner_profile_absent"
    }

    $agentProcess = Invoke-AdbExact -Context $Context -Device $Device `
        -Arguments @("shell", "pidof", $script:FleetAgentPackage) `
        -AllowedExitCodes @(0, 1)

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
        [Parameter(Mandatory)][string] $ReasonCode
    )
    $State.status = "blocked_attended"
    $State.checkpoint = [ordered]@{
        kind = $Kind
        slot = $Slot
        reason_code = $ReasonCode
        entered_at_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
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
    $StateDevice.run_owned.added_packages =
        @($StateDevice.run_owned.added_packages) + $Package
    Write-SanitizedState -Context $Context -State $State
}

function Set-FixedPackagePermission {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $Device,
        [Parameter(Mandatory)][ValidateSet(
            "org.questtermuxlab.wirelessadbrecovery",
            "io.github.mesmerprism.rustykiosk.setuphelper")]
        [string] $Package,
        [Parameter(Mandatory)][string] $Permission,
        [Parameter(Mandatory)][bool] $Granted
    )
    $verb = if ($Granted) { "grant" } else { "revoke" }
    [void](Invoke-AdbExact -Context $Context -Device $Device -Arguments @(
        "shell", "pm", $verb, $Package, $Permission
    ))
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
        [Parameter(Mandatory)][Collections.IDictionary] $Device,
        [Parameter(Mandatory)][ValidateSet(
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
        $proofResult = Test-TermuxProof `
            -Proof $operation.targets[0].termux_proof `
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
    param([Parameter(Mandatory)][object] $Context)
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
            $stateDevice.run_owned.qfm_profile_created = $true
            Write-SanitizedState -Context $Context -State $State
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
                -Package $script:HelperPackage `
                -Permission $permission -Granted $true
        }
        Set-FixedPackagePermission -Context $Context -Device $device `
            -Package $script:KioskHelperPackage `
            -Permission $script:WriteSecureSettingsPermission -Granted $true
        $termux = if ([bool]$stateDevice.run_owned.termux_restart_confirmed) {
            Invoke-HelperExact -Context $Context -Device $device `
                -HelperAction "status"
        } else {
            Invoke-HelperExact -Context $Context -Device $device `
                -HelperAction "prepare-termux-prerequisites" `
                -Confirm -ConfirmTermuxPackageInstall
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
            Set-Checkpoint -State $State -Kind "awaiting_termux_restart" `
                -Slot $slot -ReasonCode "termux_restart_required"
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
        [void](Invoke-AdbExact -Context $Context -Device $device -Arguments @(
            "shell", "run-as", $script:FleetAgentPackage,
            "mkdir", "-p", "files/fleet-agent"
        ))
        [void](Invoke-AdbExact -Context $Context -Device $device -Arguments @(
            "shell", "run-as", $script:FleetAgentPackage,
            "chmod", "700", "files/fleet-agent"
        ))
        $stateDevice.run_owned.agent_profile_staged = $true
        Write-SanitizedState -Context $Context -State $State
        Write-FleetAgentPrivateInput -Context $Context -Device $device `
            -SourcePath $device.fleet_agent_profile_path `
            -Destination "files/fleet-agent/profile.json"
        Write-FleetAgentPrivateInput -Context $Context -Device $device `
            -SourcePath $device.fleet_agent_seed_path `
            -Destination "files/fleet-agent/signing-seed.bin"
        [void](Invoke-AdbExact -Context $Context -Device $device -Arguments @(
            "shell", "run-as", $script:FleetAgentPackage,
            "chmod", "600",
            "files/fleet-agent/profile.json",
            "files/fleet-agent/signing-seed.bin"
        ))
        Start-FleetAgent -Context $Context -Device $device
        $stateDevice.run_owned.agent_started = $true
        Write-SanitizedState -Context $Context -State $State
    }

    foreach ($slot in @("device_a", "device_b")) {
        $device = Get-DeviceBySlot -Context $Context -Slot $slot
        $stateDevice = Get-StateDevice -State $State -Slot $slot
        $projection = Wait-FleetInspect -Context $Context -Device $device `
            -TimeoutSeconds ([int]$Context.Config.timing.baseline_timeout_seconds) `
            -FailureCode "baseline_checkin_timeout" `
            -Predicate {
                param($value)
                [string]$value.row.identity.device_id -ceq [string]$device.device_id -and
                [long]$value.row.identity.identity_revision -eq
                    [long]$device.identity_revision -and
                [string]$value.row.freshness -ceq "fresh" -and
                [long]$value.row.accepted_revision -gt 0
            }
        $stateDevice.acceptance.baseline_revision =
            [long]$projection.row.accepted_revision
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
    $other = Get-StateDevice -State $State -Slot $otherSlot
    $baseline = [long]$other.acceptance.baseline_revision
    $operation = Invoke-WifiOperation -Context $Context -Device $device `
        -WifiAction "request-wireless-adb"
    $target = $operation.targets[0]
    Assert-Condition (
        $target.receipt.request_delivered -eq $true -and
        $target.receipt.kiosk_setting_applied -eq $true -and
        [string]$target.receipt.wearer_approval -ceq "pending" -and
        $target.termux_usable -eq $false -and
        $null -eq $target.termux_proof
    ) "request_effect_boundary_invalid" `
        "A Wi-Fi request must remain incomplete pending wearer approval and proof."

    $otherDevice = Get-DeviceBySlot -Context $Context -Slot $otherSlot
    $otherProjection = Invoke-FleetCtlExact -Context $Context -Arguments @(
        "inspect", [string]$otherDevice.device_id
    )
    Assert-Condition (
        [long]$otherProjection.row.accepted_revision -ge $baseline -and
        [string]$otherProjection.row.freshness -ceq "fresh"
    ) "cross_device_request_mixup" `
        "Requesting one device changed or degraded the other device."
    $stateDevice = Get-StateDevice -State $State -Slot $Slot
    $stateDevice.acceptance.operation_id = [string]$operation.operation_id
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
    $helper = Invoke-HelperExact -Context $Context -Device $device `
        -HelperAction "restore-now" -Confirm
    Assert-Condition ($helper.accepted -eq $true) "helper_restore_rejected" `
        "The proof owner rejected its fixed restore request."
    $operation = Wait-WifiProof -Context $Context -Device $device `
        -OperationId ([string]$stateDevice.acceptance.operation_id)
    $proof = $operation.targets[0].termux_proof
    $stateDevice.acceptance.proof_revision = [long]$proof.evidence_revision
    $stateDevice.acceptance.termux_usable = $true
}

function Invoke-ProofExpiry {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot
    )
    $stateDevice = Get-StateDevice -State $State -Slot $Slot
    [void](Wait-WifiProofAbsent -Context $Context `
        -OperationId ([string]$stateDevice.acceptance.operation_id))
    $stateDevice.acceptance.termux_usable = $false
    $stateDevice.acceptance.expiry_observed = $true
}

function Invoke-ProofRenewal {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot
    )
    $device = Get-DeviceBySlot -Context $Context -Slot $Slot
    $stateDevice = Get-StateDevice -State $State -Slot $Slot
    $helper = Invoke-HelperExact -Context $Context -Device $device `
        -HelperAction "restore-now" -Confirm
    Assert-Condition ($helper.accepted -eq $true) "helper_renewal_rejected" `
        "The proof owner rejected its fixed renewal request."
    $operation = Wait-WifiProof -Context $Context -Device $device `
        -OperationId ([string]$stateDevice.acceptance.operation_id) `
        -MinimumEvidenceRevision ([long]$stateDevice.acceptance.proof_revision)
    $stateDevice.acceptance.renewed_proof_revision =
        [long]$operation.targets[0].termux_proof.evidence_revision
    $stateDevice.acceptance.termux_usable = $true
}

function Invoke-DisableAndCheckpointReboot {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][ValidateSet("device_a", "device_b")][string] $Slot
    )
    $device = Get-DeviceBySlot -Context $Context -Slot $Slot
    $stateDevice = Get-StateDevice -State $State -Slot $Slot
    $disabled = Invoke-WifiOperation -Context $Context -Device $device `
        -WifiAction "disable-wireless-adb"
    Assert-Condition (
        $disabled.targets.Count -eq 1 -and
        $disabled.targets[0].receipt.effect_applied -eq $true
    ) "wifi_disable_not_applied" "Fleet did not receive an owner-applied disable receipt."
    [void](Invoke-HelperExact -Context $Context -Device $device `
        -HelperAction "disable-boot-attempt" -Confirm)
    [void](Invoke-HelperExact -Context $Context -Device $device `
        -HelperAction "disable-wireless" -Confirm)
    [void](Wait-WifiProofAbsent -Context $Context `
        -OperationId ([string]$stateDevice.acceptance.operation_id))
    $stateDevice.acceptance.termux_usable = $false
    $stateDevice.acceptance.disable_observed = $true
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
    $ready = Invoke-AdbExact -Context $Context -Device $device `
        -Arguments @("get-state")
    Assert-Condition ($ready.Stdout.Trim() -ceq "device") `
        "reboot_usb_not_ready" "The attended reboot has not returned to exact USB readiness."
    $pid = Invoke-AdbExact -Context $Context -Device $device `
        -Arguments @("shell", "pidof", $script:FleetAgentPackage) `
        -AllowedExitCodes @(0, 1)
    Assert-Condition ([string]::IsNullOrWhiteSpace($pid.Stdout)) `
        "agent_unexpectedly_sticky" `
        "The non-sticky Fleet Agent was active before explicit relaunch."
    $stateDevice.acceptance.reboot_loss_observed = $true
    Start-FleetAgent -Context $Context -Device $device
    $projection = Wait-FleetInspect -Context $Context -Device $device `
        -TimeoutSeconds ([int]$Context.Config.timing.reboot_timeout_seconds) `
        -FailureCode "agent_relaunch_recovery_timeout" `
        -Predicate {
            param($value)
            [string]$value.row.freshness -ceq "fresh" -and
            [long]$value.row.accepted_revision -gt
                [long]$stateDevice.acceptance.baseline_revision
        }
    $stateDevice.acceptance.recovery_observed = $true
    $stateDevice.acceptance.baseline_revision =
        [long]$projection.row.accepted_revision
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
            Assert-OnboardingOutputsAbsent -Context $Context
            $State.onboarding.apply_attempted_by_run = $true
            Write-SanitizedState -Context $Context -State $State
            Invoke-OnboardingApply -Context $Context
            $State.onboarding.applied_by_run = $true
            return Complete-Transition -Context $Context -State $State `
                -NextPhase "onboarding-applied"
        }
        "onboarding-applied" {
            $State.hub.firewall_created = New-RunFirewallRule -Context $Context
            Write-SanitizedState -Context $Context -State $State
            $State.hub.process_id = Start-FleetHub -Context $Context
            $State.hub.started_by_run = $true
            Write-SanitizedState -Context $Context -State $State
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
            Invoke-ProofRenewal -Context $Context -State $State -Slot "device_a"
            return Complete-Transition -Context $Context -State $State `
                -NextPhase "device_a-proof-renewed" -Slot "device_a" `
                -ReasonCode "higher_evidence_revision"
        }
        "device_a-proof-renewed" {
            Invoke-DisableAndCheckpointReboot `
                -Context $Context -State $State -Slot "device_a"
            Write-SanitizedState -Context $Context -State $State
            return $State
        }
        "device_a-recovered" {
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
            Invoke-ProofRenewal -Context $Context -State $State -Slot "device_b"
            return Complete-Transition -Context $Context -State $State `
                -NextPhase "device_b-proof-renewed" -Slot "device_b" `
                -ReasonCode "higher_evidence_revision"
        }
        "device_b-proof-renewed" {
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
    Add-StateEvent -State $State -Phase "cleanup" -Status $truth.Status `
        -ReasonCode (
            if ($truth.Status -ceq "complete") {
                "cleanup_truth_complete"
            } else {
                "cleanup_partial_failure"
            })
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
            Assert-Condition ([string]$state.phase -ceq "preflight") `
                "execute_requires_preflight" "Execute starts only from completed Preflight."
            return Invoke-ResumeTransition -Context $context -State $state `
                -ConfirmCurrentCheckpoint:$false
        }
        "Resume" {
            Assert-Condition $ConfirmMutation "mutation_confirmation_required" `
                "Resume requires -ConfirmMutation."
            $state = Read-SanitizedState -Context $context
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
    "Test-TermuxProof",
    "Get-CleanupTruth"
)
