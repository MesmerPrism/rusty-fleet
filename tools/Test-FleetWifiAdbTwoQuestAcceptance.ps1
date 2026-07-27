# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($PSVersionTable.PSEdition -ne "Core" -or
    $PSVersionTable.PSVersion -lt [version]"7.6") {
    throw "Rusty Fleet acceptance tests require PowerShell 7.6 Core or newer."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$modulePath = Join-Path $PSScriptRoot "FleetWifiAdbTwoQuestAcceptance.psm1"
$runnerPath = Join-Path $PSScriptRoot "Invoke-FleetWifiAdbTwoQuestAcceptance.ps1"
Import-Module $modulePath -Force

function Assert-True {
    param(
        [Parameter(Mandatory)][bool] $Condition,
        [Parameter(Mandatory)][string] $Message
    )
    if (-not $Condition) {
        throw $Message
    }
}

function Assert-ThrowsCode {
    param(
        [Parameter(Mandatory)][scriptblock] $Operation,
        [Parameter(Mandatory)][string] $Code
    )
    try {
        & $Operation
    } catch {
        if ([string]$_.Exception.Message -like "$Code`:*") {
            return
        }
        throw "Expected $Code, observed: $($_.Exception.Message)"
    }
    throw "Expected $Code, but the operation succeeded."
}

function Write-Json {
    param(
        [Parameter(Mandatory)][string] $Path,
        [Parameter(Mandatory)][object] $Value
    )
    [IO.File]::WriteAllText(
        $Path,
        ($Value | ConvertTo-Json -Depth 32) + [Environment]::NewLine,
        [Text.UTF8Encoding]::new($false))
}

function Copy-JsonValue {
    param([Parameter(Mandatory)][object] $Value)
    return ($Value | ConvertTo-Json -Depth 32) |
        ConvertFrom-Json -AsHashtable -Depth 32
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) (
    "rusty-fleet-wifi-adb-acceptance-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $testRoot | Out-Null
try {
    $artifactIds = @(
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
    $pins = @()
    foreach ($id in $artifactIds) {
        $path = Join-Path $testRoot ($id + ".synthetic")
        [IO.File]::WriteAllText(
            $path,
            "synthetic-$id",
            [Text.UTF8Encoding]::new($false))
        $pins += [ordered]@{
            artifact_id = $id
            path = $path
            sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).
                Hash.ToLowerInvariant()
        }
    }

    $onboardingPath = Join-Path $testRoot "onboarding-request.json"
    $onboarding = [ordered]@{
        schema = "rusty.fleet.offline_onboarding_request.v1"
        output_root = Join-Path $testRoot "generated"
        tool_manifest = Join-Path $testRoot "tool-manifest.json"
        tool_manifest_sha256 = ("1" * 64)
        hub = [ordered]@{
            bind = "127.0.0.1:18741"
            checkin_bind = "192.0.2.10:18742"
            state_directory = Join-Path $testRoot "hub-state"
            operator_id = "operator.synthetic"
            request_id_prefix = "request.synthetic"
            credential_valid_from_ms = 1
            credential_expires_at_ms = 9999999999999
        }
        devices = @(
            [ordered]@{
                device_id = "device.synthetic.a"
                display_name = "Synthetic A"
                model = "synthetic"
                hardware_class = "headset"
                identity_revision = 1
                expected_authority_revision = 1
                status_revision = 1
                source_revision = 1
                source_epoch = "source.synthetic.a"
                key_id = "key.synthetic.a"
                key_generation = 1
                trust_domain = "trust.synthetic"
                checkin_ttl_ms = 60000
                checkin_interval_ms = 10000
                hub_endpoint = "http://192.0.2.10:18742/fleet/v1/checkins"
            },
            [ordered]@{
                device_id = "device.synthetic.b"
                display_name = "Synthetic B"
                model = "synthetic"
                hardware_class = "headset"
                identity_revision = 1
                expected_authority_revision = 1
                status_revision = 1
                source_revision = 1
                source_epoch = "source.synthetic.b"
                key_id = "key.synthetic.b"
                key_generation = 1
                trust_domain = "trust.synthetic"
                checkin_ttl_ms = 60000
                checkin_interval_ms = 10000
                hub_endpoint = "http://192.0.2.10:18742/fleet/v1/checkins"
            }
        )
    }
    Write-Json -Path $onboardingPath -Value $onboarding

    $hubConfig = Join-Path $testRoot "hub.json"

    $devices = @()
    foreach ($suffix in @("a", "b")) {
        $enrollmentPath = Join-Path $testRoot "qfm-$suffix.json"
        Write-Json -Path $enrollmentPath -Value ([ordered]@{
            schema =
                "questionable.file_manager.quest_connectivity_profile_enrollment.v1"
            target = "QuestIonAbleFileManager/QuestConnectivity/device.synthetic.$suffix"
            device_id = "device.synthetic.$suffix"
            usb_serial = "SYNTHETIC$suffix"
            endpoint = "http://192.0.2.2:39873/"
            pairing_code = "synthetic-pairing-code-000-$suffix"
        })
        $devices += [ordered]@{
            slot = "device_$suffix"
            device_id = "device.synthetic.$suffix"
            identity_revision = 1
            usb_serial = "SYNTHETIC$suffix"
            qfm_enrollment_path = $enrollmentPath
            fleet_agent_profile_path = Join-Path $testRoot "future-$suffix-profile.json"
            fleet_agent_seed_path = Join-Path $testRoot "future-$suffix-seed.bin"
        }
    }

    $config = [ordered]@{
        schema = "rusty.fleet.wifi_adb_two_quest_run_config.v1"
        run_id = "wifi-adb-synthetic"
        private_state_root = Join-Path $testRoot "state"
        source_commits = [ordered]@{
            questionable_file_manager =
                "a6d8e88c9d65f642d0cbf74fc8b92c8f1cd19ae5"
            wireless_adb_helper =
                "d800e5c7c5f8c77ad2bae52450f32092f3c92ace"
        }
        artifact_pins = $pins
        onboarding = [ordered]@{
            request_path = $onboardingPath
            inventory_path = Join-Path $testRoot "future-inventory.json"
        }
        hub = [ordered]@{
            config_path = $hubConfig
            operator_url = "http://127.0.0.1:18741"
            manage_firewall = $false
        }
        devices = $devices
        timing = [ordered]@{
            baseline_timeout_seconds = 30
            proof_timeout_seconds = 30
            expiry_timeout_seconds = 61
            reboot_timeout_seconds = 60
        }
    }
    $configPath = Join-Path $testRoot "run-config.json"
    Write-Json -Path $configPath -Value $config

    $validated = Read-ValidatedRunConfig -RunConfig $configPath
    Assert-True ($validated.Artifacts.Count -eq 10) `
        "Valid config did not bind the exact artifact set."
    $plan = Invoke-FleetWifiAdbTwoQuestAcceptance `
        -Action Plan -RunConfig $configPath
    Assert-True (
        $plan.status -ceq "ready" -and
        -not (Test-Path -LiteralPath $config.private_state_root)
    ) "Plan mutated the private state root or returned the wrong status."
    $planJson = $plan | ConvertTo-Json -Depth 16
    foreach ($privateValue in @(
        $testRoot,
        $devices[0].device_id,
        $devices[0].usb_serial,
        $devices[1].device_id,
        $devices[1].usb_serial
    )) {
        Assert-True (-not $planJson.Contains(
                $privateValue, [StringComparison]::OrdinalIgnoreCase)) `
            "Sanitized plan leaked a private value."
    }

    Write-Json -Path $hubConfig -Value ([ordered]@{
        schema = "rusty.fleet.local_hub_config.v1"
        checkin_bind = "192.0.2.10:18742"
    })
    Assert-ThrowsCode -Code "onboarding_output_preexists" -Operation {
        Invoke-FleetWifiAdbTwoQuestAcceptance `
            -Action Preflight -RunConfig $configPath | Out-Null
    }
    Remove-Item -LiteralPath $hubConfig -Force

    $unknown = Copy-JsonValue $config
    $unknown["unexpected"] = $true
    $unknownPath = Join-Path $testRoot "unknown.json"
    Write-Json -Path $unknownPath -Value $unknown
    Assert-ThrowsCode -Code "config_unknown_field" -Operation {
        Read-ValidatedRunConfig -RunConfig $unknownPath | Out-Null
    }

    $duplicatePath = Join-Path $testRoot "duplicate.json"
    $baseJson = $config | ConvertTo-Json -Depth 32
    $duplicateJson = $baseJson -replace
        '"run_id":\s*"wifi-adb-synthetic",',
        '"run_id":"wifi-adb-synthetic","run_id":"wifi-adb-synthetic",'
    [IO.File]::WriteAllText($duplicatePath, $duplicateJson)
    Assert-ThrowsCode -Code "config_duplicate_field" -Operation {
        Read-ValidatedRunConfig -RunConfig $duplicatePath | Out-Null
    }

    $wrongHash = Copy-JsonValue $config
    $wrongHash.artifact_pins[0].sha256 = "0" * 64
    $wrongHashPath = Join-Path $testRoot "wrong-hash.json"
    Write-Json -Path $wrongHashPath -Value $wrongHash
    Assert-ThrowsCode -Code "artifact_hash_mismatch" -Operation {
        Read-ValidatedRunConfig -RunConfig $wrongHashPath | Out-Null
    }

    $duplicateDevice = Copy-JsonValue $config
    $duplicateDevice.devices[1].device_id = $duplicateDevice.devices[0].device_id
    $duplicateDevicePath = Join-Path $testRoot "duplicate-device.json"
    Write-Json -Path $duplicateDevicePath -Value $duplicateDevice
    Assert-ThrowsCode -Code "qfm_enrollment_mismatch" -Operation {
        Read-ValidatedRunConfig -RunConfig $duplicateDevicePath | Out-Null
    }

    $crossDevice = Copy-JsonValue $config
    $crossDevice.devices[0].qfm_enrollment_path =
        $crossDevice.devices[1].qfm_enrollment_path
    $crossDevicePath = Join-Path $testRoot "cross-device.json"
    Write-Json -Path $crossDevicePath -Value $crossDevice
    Assert-ThrowsCode -Code "qfm_enrollment_mismatch" -Operation {
        Read-ValidatedRunConfig -RunConfig $crossDevicePath | Out-Null
    }

    $now = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $proof = [pscustomobject]@{
        schema = "rusty.fleet.quest_wifi_adb_termux_proof.v1"
        proof_id = "termux-proof-synthetic"
        owner_id = "quest-termux-lab"
        device_id = "device.synthetic.a"
        identity_revision = 1
        source_epoch = "source.synthetic.a"
        source_revision = 3
        evidence_revision = 2
        route_mode = "modern_tls"
        discovery_mode = "tls_nsd"
        listener_discovered = $true
        shell_identity = "uid=2000(shell)"
        available = $true
        evidence_sha256 = "1" * 64
        observed_at_ms = $now - 1000
        fresh_until_ms = $now + 59000
    }
    $receipt = [pscustomobject]@{
        request_id = "request.synthetic.termux"
        operation_id = "operation.synthetic.termux"
        device_id = "device.synthetic.a"
        evidence_sha256 = "2" * 64
    }
    $admission = [pscustomobject]@{
        schema = "rusty.fleet.quest_wifi_adb_termux_admission.v1"
        checkin_id = "checkin.synthetic.termux"
        operation_id = "operation.synthetic.termux"
        device_id = "device.synthetic.a"
        identity_revision = 1
        source_epoch = "source.synthetic.a"
        source_revision = 3
        evidence_revision = 2
        proof_id = "termux-proof-synthetic"
        receipt_request_id = "request.synthetic.termux"
        receipt_evidence_sha256 = "2" * 64
        key_id = "key.synthetic.a"
        key_generation = 1
        public_key_sha256 = "3" * 64
        claims_jcs_sha256 = "4" * 64
        signing_message_sha256 = "5" * 64
        signature_sha256 = "6" * 64
        fleet_accepted_revision = 9
        enrollment_authority_revision = 2
        manifold_authority_revision = 9
        signature_verified = $true
        canonical_claims_verified = $true
        enrollment_active = $true
        accepted_at_ms = $now - 500
        expires_at_ms = $now + 59000
        lineage_sha256 = ""
    }
    $admission.lineage_sha256 =
        Get-TermuxAdmissionLineageSha256 -Admission $admission
    $operation = [pscustomobject]@{
        operation_id = "operation.synthetic.termux"
        targets = @([pscustomobject]@{
            device_id = "device.synthetic.a"
            identity_revision = 1
            receipt = $receipt
            termux_proof = $proof
            termux_admission = $admission
            termux_usable = $true
        })
    }
    Assert-True (
        (Test-HubTermuxAdmission -Operation $operation `
            -ExpectedOperationId "operation.synthetic.termux" `
            -ExpectedDeviceId "device.synthetic.a" `
            -ExpectedIdentityRevision 1 -NowMs $now `
            -MinimumEvidenceRevision 1).Valid
    ) "Valid exact signed Hub admission was rejected."
    $wrongUid = Copy-JsonValue $operation
    $wrongUid.targets[0].termux_proof.shell_identity = "uid=10234(app)"
    Assert-True (
        (Test-HubTermuxAdmission -Operation $wrongUid `
            -ExpectedOperationId "operation.synthetic.termux" `
            -ExpectedDeviceId "device.synthetic.a" `
            -ExpectedIdentityRevision 1 -NowMs $now).ReasonCode -ceq
            "proof_shell_uid_invalid"
    ) "Wrong-UID proof did not fail closed."
    $stale = Copy-JsonValue $operation
    $stale.targets[0].termux_proof.fresh_until_ms = $now - 1
    Assert-True (
        (Test-HubTermuxAdmission -Operation $stale `
            -ExpectedOperationId "operation.synthetic.termux" `
            -ExpectedDeviceId "device.synthetic.a" `
            -ExpectedIdentityRevision 1 -NowMs $now).ReasonCode -ceq
            "proof_stale"
    ) "Stale proof did not fail closed."
    Assert-True (
        (Test-HubTermuxAdmission -Operation $operation `
            -ExpectedOperationId "operation.synthetic.termux" `
            -ExpectedDeviceId "device.synthetic.b" `
            -ExpectedIdentityRevision 1 -NowMs $now).ReasonCode -ceq
            "admission_device_mismatch"
    ) "Cross-device proof did not fail closed."
    $negativeAdmissions = @(
        @{ name = "unsigned"; mutate = {
            param($value)
            $value.targets[0].termux_admission = $null
        }},
        @{ name = "claims-hash"; mutate = {
            param($value)
            $value.targets[0].termux_admission.claims_jcs_sha256 = "0" * 64
        }},
        @{ name = "key"; mutate = {
            param($value)
            $value.targets[0].termux_admission.key_generation = 0
        }},
        @{ name = "revision"; mutate = {
            param($value)
            $value.targets[0].termux_admission.evidence_revision = 99
        }},
        @{ name = "lineage"; mutate = {
            param($value)
            $value.targets[0].termux_admission.lineage_sha256 = "f" * 64
        }}
    )
    foreach ($case in $negativeAdmissions) {
        $mutated = Copy-JsonValue $operation
        & $case.mutate $mutated
        Assert-True (-not (
            Test-HubTermuxAdmission -Operation $mutated `
                -ExpectedOperationId "operation.synthetic.termux" `
                -ExpectedDeviceId "device.synthetic.a" `
                -ExpectedIdentityRevision 1 -NowMs $now
        ).Valid) "Altered $($case.name) admission did not fail closed."
    }
    Assert-True (-not (
        Test-HubTermuxAdmission -Operation $operation `
            -ExpectedOperationId "operation.synthetic.other" `
            -ExpectedDeviceId "device.synthetic.a" `
            -ExpectedIdentityRevision 1 -NowMs $now
    ).Valid) "Cross-operation admission did not fail closed."
    Assert-True (-not (
        Test-HubTermuxAdmission -Operation $operation `
            -ExpectedOperationId "operation.synthetic.termux" `
            -ExpectedDeviceId "device.synthetic.a" `
            -ExpectedIdentityRevision 1 -NowMs $now `
            -MinimumEvidenceRevision 2
    ).Valid) "Replay/non-advancing proof revision did not fail closed."

    New-Item -ItemType Directory -Path $config.private_state_root | Out-Null
    Write-Json -Path (Join-Path $config.private_state_root "acceptance-state.json") `
        -Value ([ordered]@{
            schema = "rusty.fleet.wifi_adb_two_quest_acceptance_state.v1"
            config_sha256 = "0" * 64
            run_id_hash = "1" * 64
        })
    Assert-ThrowsCode -Code "resume_config_mismatch" -Operation {
        Invoke-FleetWifiAdbTwoQuestAcceptance `
            -Action Status -RunConfig $configPath | Out-Null
    }
    Remove-Item -LiteralPath $config.private_state_root -Recurse -Force

    $junctionTarget = Join-Path $testRoot "junction-target"
    $junctionPath = Join-Path $testRoot "junction"
    New-Item -ItemType Directory -Path $junctionTarget | Out-Null
    Copy-Item -LiteralPath $configPath `
        -Destination (Join-Path $junctionTarget "config.json")
    New-Item -ItemType Junction -Path $junctionPath `
        -Target $junctionTarget | Out-Null
    Assert-ThrowsCode -Code "private_input_reparse" -Operation {
        Read-ValidatedRunConfig `
            -RunConfig (Join-Path $junctionPath "config.json") | Out-Null
    }

    $syntheticSnapshots = @()
    foreach ($slot in @("device_a", "device_b")) {
        $syntheticSnapshots += [pscustomobject]@{
            slot = $slot
            usb_ready = $true
            package_set_sha256 = "1" * 64
            packages = [ordered]@{}
            helper_grants = [ordered]@{}
            kiosk_helper_write_secure_settings_granted = $false
            qfm_profile_state = "absent"
            kiosk_direct_link_observation = "not_yet_observed"
            after_boot_enabled = $false
            wifi_setting_enabled = $false
            transport_usb_present = $true
            signer_checks_complete = $true
            agent_process_present = $false
            agent_private_inputs_absent = $true
            boot_id_sha256 = "2" * 64
            boot_elapsed_milliseconds = 100000
            termux_process_epoch_sha256 = "0" * 64
        }
    }
    New-Item -ItemType Directory -Path $config.private_state_root | Out-Null
    $modelState = New-SanitizedState `
        -Context $validated -Snapshots $syntheticSnapshots
    Write-SanitizedState -Context $validated -State $modelState
    $roundTrip = Read-SanitizedState -Context $validated
    Assert-True (
        [string]$roundTrip.schema -ceq
            "rusty.fleet.wifi_adb_two_quest_acceptance_state.v2" -and
        @(Get-ChildItem -LiteralPath $config.private_state_root `
            -Filter "*.pending").Count -eq 0
    ) "Write-through state publication did not round-trip cleanly."

    $unknownState = Copy-JsonValue $modelState
    $unknownState["unexpected"] = $true
    Write-Json -Path (
        Join-Path $config.private_state_root "acceptance-state.json"
    ) -Value $unknownState
    Assert-ThrowsCode -Code "config_unknown_field" -Operation {
        Read-SanitizedState -Context $validated | Out-Null
    }

    Write-SanitizedState -Context $validated -State $modelState
    $statePath =
        Join-Path $config.private_state_root "acceptance-state.json"
    $stateJson = Get-Content -LiteralPath $statePath -Raw
    $duplicateStateJson = $stateJson -replace
        '"status":\s*"preflighted",',
        '"status":"preflighted","status":"preflighted",'
    [IO.File]::WriteAllText($statePath, $duplicateStateJson)
    Assert-ThrowsCode -Code "config_duplicate_field" -Operation {
        Read-SanitizedState -Context $validated | Out-Null
    }

    $modelState = New-SanitizedState `
        -Context $validated -Snapshots $syntheticSnapshots
    Write-SanitizedState -Context $validated -State $modelState
    [void](Start-DurableMutation -Context $validated -State $modelState `
        -Kind "modeled-prepared-crash" -ActionId "test.prepared" `
        -OwnerId "modeled-owner")
    $preparedState = Read-SanitizedState -Context $validated
    Assert-True (
        [string]$preparedState.mutation.stage -ceq "prepared_not_sent"
    ) "Prepared mutation was not durable before dispatch."
    Assert-ThrowsCode -Code "mutation_cleanup_required" -Operation {
        Assert-NoAmbiguousMutation `
            -Context $validated -State $preparedState
    }
    Assert-True (
        [string](Read-SanitizedState `
            -Context $validated).mutation.stage -ceq "cleanup_required"
    ) "Prepared crash recovery did not fail closed without redispatch."

    $modelState = New-SanitizedState `
        -Context $validated -Snapshots $syntheticSnapshots
    Write-SanitizedState -Context $validated -State $modelState
    [void](Start-DurableMutation -Context $validated -State $modelState `
        -Kind "modeled-sent-crash" -ActionId "test.sent" `
        -OwnerId "modeled-owner")
    Set-DurableMutationSent -Context $validated -State $modelState
    $sentState = Read-SanitizedState -Context $validated
    Assert-True (
        [string]$sentState.mutation.stage -ceq "sent_outcome_unknown"
    ) "Sent mutation outcome was not durably unknown."
    Assert-ThrowsCode -Code "mutation_cleanup_required" -Operation {
        Assert-NoAmbiguousMutation -Context $validated -State $sentState
    }

    $modelState = New-SanitizedState `
        -Context $validated -Snapshots $syntheticSnapshots
    Write-SanitizedState -Context $validated -State $modelState
    [void](Start-DurableMutation -Context $validated -State $modelState `
        -Kind "modeled-confirmed" -ActionId "test.confirmed" `
        -OwnerId "modeled-owner")
    Set-DurableMutationSent -Context $validated -State $modelState
    Complete-DurableMutation -Context $validated -State $modelState `
        -ReconciliationCode "modeled_exact_readback"
    $confirmedState = Read-SanitizedState -Context $validated
    Assert-True (
        $null -eq $confirmedState.mutation -and
        $confirmedState.mutation_history.Count -eq 1 -and
        [string]$confirmedState.journal_head_sha256 -cne ("0" * 64)
    ) "Confirmed mutation did not advance the durable digest chain."
    $confirmedState.mutation_history[0].journal_sha256 = "f" * 64
    Write-Json -Path $statePath -Value $confirmedState
    Assert-ThrowsCode -Code "mutation_journal_invalid" -Operation {
        Read-SanitizedState -Context $validated | Out-Null
    }

    $cleanup = Get-CleanupTruth -Checks ([ordered]@{
        wireless_disabled = $true
        listener_absent = $false
        profiles_revoked = $true
    })
    Assert-True (
        $cleanup.Status -ceq "partial_failure" -and
        $cleanup.FailedCount -eq 1
    ) "Partial cleanup failure was not retained."

    foreach ($file in @($modulePath, $runnerPath, $PSCommandPath)) {
        $tokens = $null
        $errors = $null
        [Management.Automation.Language.Parser]::ParseFile(
            $file, [ref]$tokens, [ref]$errors) | Out-Null
        Assert-True (@($errors).Count -eq 0) `
            "PowerShell parser errors were found in $file."
    }

    $trackedText = @(
        Get-Content -LiteralPath $modulePath -Raw
        Get-Content -LiteralPath $runnerPath -Raw
        Get-Content -LiteralPath (
            Join-Path $repoRoot "docs/QUEST_WIFI_ADB_TWO_QUEST_ACCEPTANCE.md"
        ) -Raw -ErrorAction SilentlyContinue
    ) -join "`n"
    foreach ($forbidden in @(
        "adb kill-server",
        "adb start-server",
        "adb reconnect",
        "uiautomator",
        "shell input tap",
        "pairing_code =",
        "usb_serial ="
    )) {
        Assert-True (-not $trackedText.Contains(
                $forbidden, [StringComparison]::OrdinalIgnoreCase)) `
            "Runner contains a forbidden broad or approval-automation surface."
    }

    Write-Output "Rusty Fleet two-Quest Wi-Fi ADB acceptance host tests passed."
}
finally {
    $resolvedTestRoot = [IO.Path]::GetFullPath($testRoot)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if ($resolvedTestRoot.StartsWith(
            $tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path -Leaf $resolvedTestRoot) -like
            "rusty-fleet-wifi-adb-acceptance-*") {
        Remove-Item -LiteralPath $resolvedTestRoot -Recurse -Force `
            -ErrorAction SilentlyContinue
    }
}
