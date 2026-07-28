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
Import-Module $modulePath -Force -DisableNameChecking

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

function Invoke-RunnerProcess {
    param(
        [Parameter(Mandatory)][string[]] $Arguments
    )
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = (Get-Command pwsh -ErrorAction Stop).Source
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $Arguments) {
        $start.ArgumentList.Add($argument)
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    Assert-True $process.Start() "Could not start isolated runner process."
    try {
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        Assert-True ($process.WaitForExit(30000)) `
            "Isolated runner process timed out."
        return [pscustomobject]@{
            ExitCode = $process.ExitCode
            Stdout = $stdout.GetAwaiter().GetResult()
            Stderr = $stderr.GetAwaiter().GetResult()
        }
    } finally {
        $process.Dispose()
    }
}

function Invoke-AgentBoardRaceAttempt {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][scriptblock] $Hook
    )
    & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
        param($InnerContext, $InnerHook)
        $script:AgentBoardExecutionRaceHook = $InnerHook
        try {
            Invoke-AgentBoardCli -Context $InnerContext -Arguments @(
                "reserve", "quest:SYNTHETICa",
                "--owner", "rusty-fleet-wifi-adb-wifi-adb-synthetic",
                "--task", "two-quest acceptance device_a",
                "--reason",
                    "run=wifi-adb-synthetic;slot=device_a;device=device.synthetic.a",
                "--duration", "3600s",
                "--json"
            ) | Out-Null
        } finally {
            $script:AgentBoardExecutionRaceHook = $null
        }
    } $Context $Hook
}

function Invoke-ModeledJournaledCleanupStep {
    param(
        [Parameter(Mandatory)][object] $Context,
        [Parameter(Mandatory)][Collections.IDictionary] $State,
        [Parameter(Mandatory)][Collections.IDictionary] $Checks,
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][scriptblock] $Operation
    )
    & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
        param(
            $InnerContext,
            $InnerState,
            $InnerChecks,
            $InnerName,
            $InnerOperation
        )
        Invoke-JournaledCleanupStep `
            -Context $InnerContext -State $InnerState `
            -Checks $InnerChecks -Name $InnerName `
            -ModeledNoDeviceProjection -Operation $InnerOperation
    } $Context $State $Checks $Name $Operation
}

function Get-TestMutationHistorySummarySha256 {
    param(
        [Parameter(Mandatory)][Collections.IDictionary] $Summary
    )
    return & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
        param($InnerSummary)
        Get-MutationHistorySummarySha256 -Summary $InnerSummary
    } $Summary
}

$ownerFixtureRoot = Join-Path $PSScriptRoot "fixtures/adb-owner"
$cleanManager = ConvertFrom-ClosedAdbManagerDump -Text (
    Get-Content -LiteralPath (
        Join-Path $ownerFixtureRoot "aosp-v1-clean.txt") -Raw)
Assert-True (
    $cleanManager.ParseState -ceq "known" -and
    $cleanManager.Format -ceq "android.debugging_manager.text.v1" -and
    $cleanManager.RetainedPairingState -ceq "absent" -and
    $cleanManager.ListenerState -ceq "unknown" -and
    $cleanManager.WirelessSessionState -ceq "unknown" -and
    $cleanManager.WirelessPendingState -ceq "unknown"
) "Closed AOSP v1 manager parsing invented a network fact."

$retainedManager = ConvertFrom-ClosedAdbManagerDump -Text (
    Get-Content -LiteralPath (
        Join-Path $ownerFixtureRoot "aosp-v1-retained.txt") -Raw)
Assert-True (
    $retainedManager.ParseState -ceq "known" -and
    $retainedManager.RetainedPairingState -ceq "present" -and
    $retainedManager.RetainedPairingCount -eq 1 -and
    $retainedManager.TrustedWifiNetworkCount -eq 1 -and
    $retainedManager.RetainedPairingSha256 -cmatch '^[0-9a-f]{64}$'
) "Closed AOSP v1 manager parsing lost retained pairing/trust."

foreach ($fixture in @(
    "aosp-v1-adversarial-dynamic.txt",
    "unknown-version.txt"
)) {
    $unknownManager = ConvertFrom-ClosedAdbManagerDump -Text (
        Get-Content -LiteralPath (Join-Path $ownerFixtureRoot $fixture) -Raw)
    Assert-True (
        $unknownManager.ParseState -ceq "unknown" -and
        $unknownManager.ListenerState -ceq "unknown" -and
        $unknownManager.WirelessSessionState -ceq "unknown" -and
        $unknownManager.WirelessPendingState -ceq "unknown" -and
        $unknownManager.RetainedPairingState -ceq "unknown"
    ) "Unrecognized ADB-manager text was falsely interpreted as absent."
}

$dynamicMdns = ConvertFrom-ClosedAdbMdnsServices -Text (
    Get-Content -LiteralPath (
        Join-Path $ownerFixtureRoot "mdns-dynamic-tls.txt") -Raw)
$pendingMdns = ConvertFrom-ClosedAdbMdnsServices -Text (
    Get-Content -LiteralPath (
        Join-Path $ownerFixtureRoot "mdns-dynamic-tls-pending.txt") -Raw)
$dynamicOwnerSockets = ConvertFrom-ClosedAdbdSocketOwnerReadback -Text (
    Get-Content -LiteralPath (
        Join-Path $ownerFixtureRoot "adbd-sockets-dynamic.txt") -Raw)
$emptyOwnerSockets = ConvertFrom-ClosedAdbdSocketOwnerReadback -Text (
    Get-Content -LiteralPath (
        Join-Path $ownerFixtureRoot "adbd-sockets-empty.txt") -Raw)
$unstableOwnerSockets = @(
    "adbd-sockets-listener-churn.txt",
    "adbd-sockets-pid-reuse.txt",
    "adbd-sockets-within-sample-late-listener.txt",
    "adbd-sockets-within-sample-late-session.txt",
    "adbd-sockets-close-reopen-reused-port.txt",
    "adbd-sockets-fd-churn-same-identity.txt",
    "adbd-sockets-tcp-state-churn-same-inode.txt",
    "adbd-sockets-partial-tcp6-read.txt",
    "adbd-sockets-second-sample-right-edge-churn.txt"
) | ForEach-Object {
    ConvertFrom-ClosedAdbdSocketOwnerReadback -Text (
        Get-Content -LiteralPath (Join-Path $ownerFixtureRoot $_) -Raw)
}
Assert-True (
    $dynamicOwnerSockets.ParseState -ceq "known" -and
    @($dynamicOwnerSockets.ListenerPorts) -contains 43127 -and
    @($dynamicOwnerSockets.EstablishedPorts) -contains 43127 -and
    $emptyOwnerSockets.ParseState -ceq "known" -and
    @($emptyOwnerSockets.ListenerPorts).Count -eq 0 -and
    @($emptyOwnerSockets.EstablishedPorts).Count -eq 0
) "The closed adbd socket-owner parser lost an owned listener/session."
Assert-True (
    @($unstableOwnerSockets | Where-Object {
        $_.ParseState -cne "unknown" -or
        @($_.ListenerPorts).Count -ne 0 -or
        @($_.EstablishedPorts).Count -ne 0
    }).Count -eq 0
) "An atomicity-damaged adbd snapshot was falsely interpreted as absent."
$inactivePort = [pscustomobject]@{ State = "inactive"; Port = 0 }
$dynamicFacts = Resolve-AdbOwnerNetworkFacts `
    -Manager $retainedManager -Mdns $dynamicMdns `
    -OwnerSockets $dynamicOwnerSockets `
    -Tcp $inactivePort -Tls $inactivePort -WifiSetting "1" `
    -PersistentTlsSetting "1" -TargetServices $dynamicMdns.Services
Assert-True (
    $dynamicFacts.ListenerState -ceq "active" -and
    $dynamicFacts.WirelessSessionState -ceq "active" -and
    $dynamicFacts.WirelessPendingState -ceq "unknown"
) "Empty properties hid a dynamic TLS listener/session."

$pendingFacts = Resolve-AdbOwnerNetworkFacts `
    -Manager $retainedManager -Mdns $pendingMdns `
    -OwnerSockets ([pscustomobject]@{
        ParseState = "known"
        ListenerPorts = @(43127)
        EstablishedPorts = @()
    }) `
    -Tcp $inactivePort -Tls $inactivePort -WifiSetting "1" `
    -PersistentTlsSetting "1" -TargetServices $pendingMdns.Services
Assert-True (
    $pendingFacts.ListenerState -ceq "active" -and
    $pendingFacts.WirelessSessionState -ceq "absent" -and
    $pendingFacts.WirelessPendingState -ceq "pending"
) "Dynamic pairing-server state was falsely interpreted as absent."

$pendingWithoutListener = Resolve-AdbOwnerNetworkFacts `
    -Manager $retainedManager `
    -Mdns ([pscustomobject]@{ ParseState = "known"; Services = @() }) `
    -OwnerSockets $emptyOwnerSockets `
    -Tcp $inactivePort -Tls $inactivePort -WifiSetting "1" `
    -PersistentTlsSetting "1" -TargetServices @()
Assert-True (
    $pendingWithoutListener.ListenerState -ceq "absent" -and
    $pendingWithoutListener.WirelessSessionState -ceq "absent" -and
    $pendingWithoutListener.WirelessPendingState -ceq "pending"
) "A conclusive empty owner projection lost the pending activation."

foreach ($unstableOwner in $unstableOwnerSockets) {
    $unstableFacts = Resolve-AdbOwnerNetworkFacts `
        -Manager $retainedManager `
        -Mdns ([pscustomobject]@{ ParseState = "known"; Services = @() }) `
        -OwnerSockets $unstableOwner `
        -Tcp $inactivePort -Tls $inactivePort -WifiSetting "0" `
        -PersistentTlsSetting "0" -TargetServices @()
    Assert-True (
        $unstableFacts.ListenerState -ceq "unknown" -and
        $unstableFacts.WirelessSessionState -ceq "unknown" -and
        $unstableFacts.WirelessPendingState -ceq "unknown"
    ) "Owner-snapshot churn did not poison every derived network fact."
    $preflightWouldAdmit =
        $unstableFacts.ListenerState -ceq "absent" -and
        $unstableFacts.WirelessSessionState -ceq "absent" -and
        $unstableFacts.WirelessPendingState -ceq "absent"
    $terminalCleanupWouldAccept =
        $unstableFacts.ListenerState -ceq "absent" -and
        $unstableFacts.WirelessSessionState -ceq "absent" -and
        $unstableFacts.WirelessPendingState -ceq "absent" -and
        $retainedManager.Format -ceq "android.debugging_manager.text.v1"
    Assert-True (-not $preflightWouldAdmit) `
        "An unstable owner snapshot could pass Preflight admission."
    Assert-True (-not $terminalCleanupWouldAccept) `
        "An unstable owner snapshot could satisfy terminal cleanup."
}

$unknownFacts = Resolve-AdbOwnerNetworkFacts `
    -Manager $unknownManager -Mdns $dynamicMdns `
    -OwnerSockets ([pscustomobject]@{
        ParseState = "unknown"
        ListenerPorts = @()
        EstablishedPorts = @()
    }) `
    -Tcp $inactivePort -Tls $inactivePort -WifiSetting "1" `
    -PersistentTlsSetting "1" -TargetServices $dynamicMdns.Services
Assert-True (
    $unknownFacts.ListenerState -ceq "unknown" -and
    $unknownFacts.WirelessSessionState -ceq "unknown" -and
    $unknownFacts.WirelessPendingState -ceq "unknown"
) "Unknown ADB-manager output did not poison the combined owner readback."

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

    $fakeBoardStatePath = Join-Path $testRoot "fake-agent-board-state.json"
    $fakeBoardDirectory = Join-Path $testRoot "agent-board-owner"
    New-Item -ItemType Directory -Path $fakeBoardDirectory | Out-Null
    $fakeBoardPath = Join-Path $fakeBoardDirectory "fake-agent-board.ps1"
    $fakeBoardScript = @'
$ErrorActionPreference = "Stop"
$statePath = '__STATE_PATH__'
$commandArguments = @($args)
if (Test-Path -LiteralPath $statePath) {
    $state = Get-Content -LiteralPath $statePath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 16 -DateKind String
} else {
    $state = [ordered]@{
        mode = "normal"
        reserve_count = 0
        heartbeat_count = 0
        release_count = 0
        leases = @()
    }
}
function Save-State {
    [IO.File]::WriteAllText(
        $statePath,
        ($state | ConvertTo-Json -Depth 16) + [Environment]::NewLine,
        [Text.UTF8Encoding]::new($false))
}
function Get-Option {
    param([string] $Name)
    $items = $commandArguments
    for ($i = 0; $i -lt $items.Count - 1; $i++) {
        if ([string]$items[$i] -ceq $Name) {
            return [string]$items[$i + 1]
        }
    }
    return ""
}
function Send-Json {
    param([object] $Value, [int] $ExitCode = 0)
    $Value | ConvertTo-Json -Depth 16
    Save-State
    exit $ExitCode
}
function Get-IsoTimestamp {
    param([DateTimeOffset] $Value)
    return $Value.ToUniversalTime().ToString(
        "yyyy-MM-ddTHH:mm:ssZ",
        [Globalization.CultureInfo]::InvariantCulture)
}
$command = [string]$args[0]
$now = [DateTimeOffset]::UtcNow
if ($command -ceq "reserve") {
    $state.reserve_count = [int]$state.reserve_count + 1
    $resource = [string]$args[1]
    if (
        [string]$state.mode -ceq "reserve_second_failure" -and
        $resource.EndsWith("SYNTHETICb", [StringComparison]::Ordinal)
    ) {
        Send-Json -Value ([ordered]@{
            ok = $false
            status = "busy"
            blocking_lease = [ordered]@{}
        }) -ExitCode 2
    }
    $lease = [ordered]@{
        id = [guid]::NewGuid().ToString()
        resource = $resource
        owner = Get-Option "--owner"
        task = Get-Option "--task"
        reason = Get-Option "--reason"
        status = "active"
        host = "synthetic-host"
        owner_pid = 1234
        created_at = Get-IsoTimestamp -Value $now
        expected_until = Get-IsoTimestamp -Value ($now.AddHours(1))
        lease_until = Get-IsoTimestamp -Value ($now.AddHours(1))
        released_at = $null
        result = $null
        note = $null
    }
    $state.leases = @($state.leases) + $lease
    Send-Json -Value ([ordered]@{
        ok = $true
        status = "reserved"
        lease = $lease
    })
}
if ($command -ceq "heartbeat") {
    $state.heartbeat_count = [int]$state.heartbeat_count + 1
    $leaseId = [string]$args[1]
    $lease = @($state.leases | Where-Object {
        [string]$_.id -ceq $leaseId
    })[0]
    if (
        [string]$state.mode -ceq "heartbeat_second_expired" -and
        [string]$lease.resource -clike "*SYNTHETICb"
    ) {
        $lease.status = "expired"
        Send-Json -Value ([ordered]@{
            ok = $false
            status = "expired"
            lease = $lease
        }) -ExitCode 2
    }
    $lease.lease_until = Get-IsoTimestamp -Value ($now.AddHours(1))
    $responseLease = $lease
    if (
        [string]$state.mode -ceq "heartbeat_wrong_resource" -and
        [string]$lease.resource -clike "*SYNTHETICb"
    ) {
        $responseLease = [ordered]@{}
        foreach ($entry in $lease.GetEnumerator()) {
            $responseLease[[string]$entry.Key] = $entry.Value
        }
        $responseLease.resource = "quest:WRONG"
    }
    Send-Json -Value ([ordered]@{
        ok = $true
        status = "active"
        lease = $responseLease
    })
}
if ($command -ceq "release") {
    $state.release_count = [int]$state.release_count + 1
    $leaseId = [string]$args[1]
    $lease = @($state.leases | Where-Object {
        [string]$_.id -ceq $leaseId
    })[0]
    if (
        [string]$state.mode -ceq "release_second_failure" -and
        [string]$lease.resource -clike "*SYNTHETICb"
    ) {
        Send-Json -Value ([ordered]@{
            ok = $false
            status = "busy"
            lease = $lease
        }) -ExitCode 2
    }
    $lease.status = "released"
    $lease.released_at = Get-IsoTimestamp -Value $now
    $lease.result = "done"
    $lease.note = "terminal cleanup complete"
    Send-Json -Value ([ordered]@{
        ok = $true
        status = "released"
        lease = $lease
    })
}
Send-Json -Value ([ordered]@{
    ok = $false
    status = "unsupported"
}) -ExitCode 2
'@
    $escapedFakeBoardStatePath =
        $fakeBoardStatePath.Replace("'", "''")
    [IO.File]::WriteAllText(
        $fakeBoardPath,
        $fakeBoardScript.Replace(
            "__STATE_PATH__", $escapedFakeBoardStatePath),
        [Text.UTF8Encoding]::new($false))

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
        schema = "rusty.fleet.wifi_adb_two_quest_run_config.v2"
        run_id = "wifi-adb-synthetic"
        private_state_root = Join-Path $testRoot "state"
        agent_board = [ordered]@{
            cli_path = $fakeBoardPath
            cli_sha256 = (
                Get-FileHash -LiteralPath $fakeBoardPath -Algorithm SHA256
            ).Hash.ToLowerInvariant()
            lease_duration_seconds = 3600
        }
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

    $leafReplacement = Join-Path $testRoot "replacement-agent-board.ps1"
    [IO.File]::WriteAllText(
        $leafReplacement,
        "throw 'replacement wrapper executed'",
        [Text.UTF8Encoding]::new($false))
    $ancestorReplacement = Join-Path $testRoot "agent-board-owner-malicious"
    New-Item -ItemType Directory -Path $ancestorReplacement | Out-Null
    [IO.File]::WriteAllText(
        (Join-Path $ancestorReplacement "fake-agent-board.ps1"),
        "throw 'ancestor replacement executed'",
        [Text.UTF8Encoding]::new($false))
    $movedBoardDirectory = Join-Path $testRoot "agent-board-owner-moved"
    $hardlinkSource = Join-Path $testRoot "hardlink-agent-board-source.ps1"
    $hardlinkCandidate =
        Join-Path $testRoot "hardlink-agent-board-candidate.ps1"
    [IO.File]::WriteAllText(
        $hardlinkSource,
        "throw 'hardlink replacement executed'",
        [Text.UTF8Encoding]::new($false))
    New-Item -ItemType HardLink -Path $hardlinkCandidate `
        -Target $hardlinkSource | Out-Null

    $raceCases = @(
        [ordered]@{
            name = "in-place modification"
            hook = {
                [IO.File]::AppendAllText(
                    $fakeBoardPath,
                    "`nthrow 'in-place mutation executed'")
            }.GetNewClosure()
        },
        [ordered]@{
            name = "leaf replacement"
            hook = {
                Move-Item -LiteralPath $leafReplacement `
                    -Destination $fakeBoardPath -Force
            }.GetNewClosure()
        },
        [ordered]@{
            name = "ancestor rename and junction substitution"
            hook = {
                Move-Item -LiteralPath $fakeBoardDirectory `
                    -Destination $movedBoardDirectory
                New-Item -ItemType Junction -Path $fakeBoardDirectory `
                    -Target $ancestorReplacement | Out-Null
            }.GetNewClosure()
        },
        [ordered]@{
            name = "hardlink substitution"
            hook = {
                Move-Item -LiteralPath $hardlinkCandidate `
                    -Destination $fakeBoardPath -Force
            }.GetNewClosure()
        }
    )
    foreach ($raceCase in $raceCases) {
        Assert-ThrowsCode `
            -Code "agent_board_execution_race_detected" -Operation {
            Invoke-AgentBoardRaceAttempt -Context $validated `
                -Hook $raceCase.hook
        }
        Assert-True (
            -not (Test-Path -LiteralPath $fakeBoardStatePath) -and
            (Get-FileHash -LiteralPath $fakeBoardPath -Algorithm SHA256).
                Hash.ToLowerInvariant() -ceq
                    [string]$config.agent_board.cli_sha256 -and
            (Test-Path -LiteralPath $fakeBoardDirectory -PathType Container)
        ) "The $($raceCase.name) boundary attack reached Board or changed the pin."
    }

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

    $wrongBoardHash = Copy-JsonValue $config
    $wrongBoardHash.agent_board.cli_sha256 = "0" * 64
    $wrongBoardHashPath = Join-Path $testRoot "wrong-board-hash.json"
    Write-Json -Path $wrongBoardHashPath -Value $wrongBoardHash
    Assert-ThrowsCode -Code "agent_board_cli_pin_invalid" -Operation {
        Read-ValidatedRunConfig -RunConfig $wrongBoardHashPath | Out-Null
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
            kiosk_direct_link_observation = "confirmed"
            after_boot_enabled = $false
            wifi_setting_enabled = $false
            transport_usb_present = $true
            adb_tcp_port_state = "inactive"
            adb_tls_port_state = "inactive"
            adb_listener_state = "absent"
            wireless_session_state = "absent"
            wireless_pending_state = "absent"
            host_forward_count = 0
            host_reverse_count = 0
            adb_manager_format = "android.debugging_manager.text.v1"
            adb_retained_pairing_state = "absent"
            adb_retained_pairing_sha256 = "4" * 64
            adb_manager_state_sha256 = "3" * 64
            helper_status_state = "absent"
            helper_in_flight = $false
            helper_proof_listener_discovered = $false
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
            "rusty.fleet.wifi_adb_two_quest_acceptance_state.v6" -and
        [string]$roundTrip.claims.installed -ceq "not_evaluated" -and
        [string]$roundTrip.claims.reachable -ceq "not_evaluated" -and
        $null -eq $roundTrip.agent_board_reservation -and
        @(Get-ChildItem -LiteralPath $config.private_state_root `
            -Filter "*.pending").Count -eq 0
    ) "Write-through state publication did not round-trip cleanly."

    $legacyState = Copy-JsonValue $roundTrip
    $legacyState.schema =
        "rusty.fleet.wifi_adb_two_quest_acceptance_state.v5"
    $legacyState.Remove("mutation_history_summary")
    Write-Json -Path (
        Join-Path $config.private_state_root "acceptance-state.json"
    ) -Value $legacyState
    $migratedState = Read-SanitizedState -Context $validated
    Assert-True (
        [string]$migratedState.schema -ceq
            "rusty.fleet.wifi_adb_two_quest_acceptance_state.v6" -and
        [long]$migratedState.mutation_history_summary.compacted_count -eq 0 -and
        [string]$migratedState.mutation_history_summary.summary_sha256 -cmatch
            '^[0-9a-f]{64}$'
    ) "A valid v5 restart state did not migrate in memory to bounded v6 state."
    Write-SanitizedState -Context $validated -State $migratedState
    $roundTrip = Read-SanitizedState -Context $validated

    Assert-ThrowsCode -Code "agent_board_reservation_required" -Operation {
        Assert-AgentBoardReservation `
            -Context $validated -State $roundTrip | Out-Null
    }
    Assert-True (
        [string](Read-SanitizedState `
            -Context $validated).agent_board_reservation -ceq "expired"
    ) "A missing private two-device receipt did not fail closed."

    $modelState = Read-SanitizedState -Context $validated
    [void](Ensure-AgentBoardReservation `
        -Context $validated -State $modelState -AllowRepair)
    $privateReservationPath = Join-Path `
        $config.private_state_root "agent-board-reservation.json"
    $privateReservation = Get-Content `
        -LiteralPath $privateReservationPath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 16
    $sanitizedReservationJson = Get-Content -LiteralPath (
        Join-Path $config.private_state_root "acceptance-state.json") -Raw
    Assert-True (
        [string]$modelState.agent_board_reservation -ceq "bound" -and
        [string]$privateReservation.state -ceq "bound" -and
        $privateReservation.leases.Count -eq 2 -and
        @($privateReservation.leases.slot | Sort-Object -Unique).Count -eq 2 -and
        $sanitizedReservationJson.Contains(
            '"agent_board_reservation": "bound"',
            [StringComparison]::Ordinal) -and
        -not $sanitizedReservationJson.Contains(
            "SYNTHETICa", [StringComparison]::Ordinal) -and
        -not $sanitizedReservationJson.Contains(
            "SYNTHETICb", [StringComparison]::Ordinal) -and
        -not $sanitizedReservationJson.Contains(
            [string]$privateReservation.leases[0].lease_id,
            [StringComparison]::Ordinal)
    ) "The private two-device reservation was not bound or leaked into public state."

    $fakeBoardState = Get-Content -LiteralPath $fakeBoardStatePath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 16
    Assert-True (
        [int]$fakeBoardState.reserve_count -eq 2 -and
        [int]$fakeBoardState.release_count -eq 0
    ) "The exact two resources were not freshly reserved."

    $heartbeatBefore = [int]$fakeBoardState.heartbeat_count
    [void](Assert-AgentBoardReservation `
        -Context $validated -State $modelState)
    $fakeBoardState = Get-Content -LiteralPath $fakeBoardStatePath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 16
    Assert-True (
        [int]$fakeBoardState.heartbeat_count -eq $heartbeatBefore + 2
    ) "Reservation revalidation did not heartbeat both exact leases."

    $restartScript = @"
`$ErrorActionPreference = "Stop"
Import-Module -Force -DisableNameChecking '$($modulePath.Replace("'", "''"))'
`$context = Read-ValidatedRunConfig -RunConfig '$($configPath.Replace("'", "''"))'
`$state = Read-SanitizedState -Context `$context
[void](Assert-AgentBoardReservation -Context `$context -State `$state)
"@
    $encodedRestartScript = [Convert]::ToBase64String(
        [Text.Encoding]::Unicode.GetBytes($restartScript))
    $reservationRestart = Invoke-RunnerProcess -Arguments @(
        "-NoProfile", "-EncodedCommand", $encodedRestartScript)
    Assert-True ($reservationRestart.ExitCode -eq 0) `
        "A fresh process could not reload and revalidate the private lease bundle."

    $validPrivateReservation = Get-Content `
        -LiteralPath $privateReservationPath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 16 -DateKind String
    $wrongRunReservation = Copy-JsonValue $validPrivateReservation
    $wrongRunReservation.run_id = "wifi-adb-other-run"
    Write-Json -Path $privateReservationPath -Value $wrongRunReservation
    Assert-ThrowsCode -Code "agent_board_receipt_invalid" -Operation {
        Assert-AgentBoardReservation `
            -Context $validated -State $modelState | Out-Null
    }
    Write-Json -Path $privateReservationPath `
        -Value $validPrivateReservation
    $modelState = Read-SanitizedState -Context $validated
    $modelState.agent_board_reservation = "bound"
    Write-SanitizedState -Context $validated -State $modelState

    $wrongSlotReservation = Copy-JsonValue $validPrivateReservation
    $wrongSlotReservation.leases[0].slot = "device_b"
    Write-Json -Path $privateReservationPath -Value $wrongSlotReservation
    Assert-ThrowsCode -Code "agent_board_receipt_binding_invalid" -Operation {
        Assert-AgentBoardReservation `
            -Context $validated -State $modelState | Out-Null
    }
    Write-Json -Path $privateReservationPath `
        -Value $validPrivateReservation
    $modelState = Read-SanitizedState -Context $validated
    $modelState.agent_board_reservation = "bound"
    Write-SanitizedState -Context $validated -State $modelState

    $fakeBoardState = Get-Content -LiteralPath $fakeBoardStatePath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 16
    $fakeBoardState.mode = "heartbeat_wrong_resource"
    Write-Json -Path $fakeBoardStatePath -Value $fakeBoardState
    Assert-ThrowsCode -Code "agent_board_lease_binding_invalid" -Operation {
        Assert-AgentBoardReservation `
            -Context $validated -State $modelState | Out-Null
    }
    Assert-True (
        [string](Read-SanitizedState `
            -Context $validated).agent_board_reservation -ceq "expired"
    ) "A wrong-resource heartbeat did not poison the public projection."
    $fakeBoardState = Get-Content -LiteralPath $fakeBoardStatePath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 16
    $fakeBoardState.mode = "normal"
    Write-Json -Path $fakeBoardStatePath -Value $fakeBoardState
    $modelState = Read-SanitizedState -Context $validated
    [void](Ensure-AgentBoardReservation `
        -Context $validated -State $modelState -AllowRepair)

    $fakeBoardState = Get-Content -LiteralPath $fakeBoardStatePath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 16
    $fakeBoardState.mode = "heartbeat_second_expired"
    Write-Json -Path $fakeBoardStatePath -Value $fakeBoardState
    Assert-ThrowsCode -Code "agent_board_heartbeat_failed" -Operation {
        Assert-AgentBoardReservation `
            -Context $validated -State $modelState | Out-Null
    }
    $fakeBoardState = Get-Content -LiteralPath $fakeBoardStatePath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 16
    $fakeBoardState.mode = "normal"
    Write-Json -Path $fakeBoardStatePath -Value $fakeBoardState
    $modelState = Read-SanitizedState -Context $validated
    [void](Ensure-AgentBoardReservation `
        -Context $validated -State $modelState -AllowRepair)
    Assert-True (
        [string]$modelState.agent_board_reservation -ceq "bound"
    ) "Cleanup-capable repair could not replace one expired exact lease."

    $privateReservation = Get-Content `
        -LiteralPath $privateReservationPath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 16
    foreach ($lease in $privateReservation.leases) {
        $lease.lease_until = "2000-01-01T00:00:00Z"
    }
    Write-Json -Path $privateReservationPath -Value $privateReservation
    $projectedStatus = Invoke-FleetWifiAdbTwoQuestAcceptance `
        -Action Status -RunConfig $configPath
    Assert-True (
        [string]$projectedStatus.agent_board_reservation -ceq "expired" -and
        [string](Read-SanitizedState `
            -Context $validated).agent_board_reservation -ceq "bound"
    ) "Status did not derive an expired public projection without mutating state."
    $modelState = Read-SanitizedState -Context $validated
    [void](Ensure-AgentBoardReservation `
        -Context $validated -State $modelState -AllowRepair)

    Assert-ThrowsCode -Code "agent_board_release_before_cleanup" -Operation {
        Release-AgentBoardReservation `
            -Context $validated -State $modelState | Out-Null
    }

    $heartbeatBefore = [int](
        Get-Content -LiteralPath $fakeBoardStatePath -Raw |
            ConvertFrom-Json -AsHashtable -Depth 16
    ).heartbeat_count
    [void](Start-DurableMutation -Context $validated -State $modelState `
        -Kind "modeled-board-dispatch" -ActionId "test.board.dispatch" `
        -OwnerId "modeled-owner" -ModeledNoDeviceProjection)
    & (Get-Module FleetWifiAdbTwoQuestAcceptance) {
        param($context, $state)
        $state.mutation.isolation_scope = "both"
        $state.mutation.journal_sha256 =
            Get-MutationJournalSha256 -Mutation $state.mutation
        Write-SanitizedState -Context $context -State $state
    } $validated $modelState
    Set-DurableMutationSent -Context $validated -State $modelState
    $heartbeatAfter = [int](
        Get-Content -LiteralPath $fakeBoardStatePath -Raw |
            ConvertFrom-Json -AsHashtable -Depth 16
    ).heartbeat_count
    Assert-True (
        $heartbeatAfter -eq $heartbeatBefore + 2 -and
        [string]$modelState.mutation.stage -ceq "sent_outcome_unknown"
    ) "The final pre-dispatch boundary did not revalidate both reservations."

    $modelState = New-SanitizedState `
        -Context $validated -Snapshots $syntheticSnapshots
    Write-SanitizedState -Context $validated -State $modelState
    [void](Ensure-AgentBoardReservation `
        -Context $validated -State $modelState -AllowRepair)
    $modelState.cleanup.status = "complete"
    $modelState.status = "complete"
    Write-SanitizedState -Context $validated -State $modelState
    $fakeBoardState = Get-Content -LiteralPath $fakeBoardStatePath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 16
    $fakeBoardState.mode = "release_second_failure"
    Write-Json -Path $fakeBoardStatePath -Value $fakeBoardState
    Assert-ThrowsCode -Code "agent_board_release_incomplete" -Operation {
        Release-AgentBoardReservation `
            -Context $validated -State $modelState | Out-Null
    }
    Assert-True (
        [string](Read-SanitizedState `
            -Context $validated).agent_board_reservation -ceq "expired"
    ) "Partial terminal release was falsely projected as released."
    $fakeBoardState = Get-Content -LiteralPath $fakeBoardStatePath -Raw |
        ConvertFrom-Json -AsHashtable -Depth 16
    $fakeBoardState.mode = "normal"
    Write-Json -Path $fakeBoardStatePath -Value $fakeBoardState
    $modelState = Read-SanitizedState -Context $validated
    [void](Release-AgentBoardReservation `
        -Context $validated -State $modelState)
    Assert-True (
        [string]$modelState.agent_board_reservation -ceq "released"
    ) "Terminal retry did not release the retained exact reservation."

    $cleanupRetryBoardBefore =
        Get-Content -LiteralPath $fakeBoardStatePath -Raw |
            ConvertFrom-Json -AsHashtable -Depth 16
    $cleanupRetryState = New-SanitizedState `
        -Context $validated -Snapshots $syntheticSnapshots
    Write-SanitizedState -Context $validated -State $cleanupRetryState
    [void](Ensure-AgentBoardReservation `
        -Context $validated -State $cleanupRetryState -AllowRepair)
    $cleanupRetryReceipt =
        Get-Content -LiteralPath $privateReservationPath -Raw |
            ConvertFrom-Json -AsHashtable -Depth 16
    $cleanupRetryLeaseIds = @(
        $cleanupRetryReceipt.leases | ForEach-Object {
            [string]$_.lease_id
        }
    )
    $cleanupRetryOwner = [ordered]@{
        dispatch_count = 0
        unsafe_effect_count = 0
        effect_applied = $false
        recovered = $false
    }
    $cleanupRetryOperation = {
        $cleanupRetryOwner.dispatch_count =
            [int]$cleanupRetryOwner.dispatch_count + 1
        if (-not [bool]$cleanupRetryOwner.effect_applied) {
            $cleanupRetryOwner.effect_applied = $true
            $cleanupRetryOwner.unsafe_effect_count =
                [int]$cleanupRetryOwner.unsafe_effect_count + 1
        }
        return [bool]$cleanupRetryOwner.recovered
    }.GetNewClosure()
    $cleanupRetryChecks = [ordered]@{
        "device_a-final-readback" = $true
        "device_b-final-readback" = $true
    }
    [void](Assert-AgentBoardReservation `
        -Context $validated -State $cleanupRetryState)
    Invoke-ModeledJournaledCleanupStep `
        -Context $validated -State $cleanupRetryState `
        -Checks $cleanupRetryChecks -Name "owner-cleanup-confirmed" `
        -Operation $cleanupRetryOperation
    $cleanupRetryTruth = Get-CleanupTruth -Checks $cleanupRetryChecks
    $cleanupRetryState.cleanup.status = $cleanupRetryTruth.Status
    $cleanupRetryState.status = "cleanup_partial_failure"
    $cleanupRetryState.phase = "cleanup"
    Write-SanitizedState -Context $validated -State $cleanupRetryState

    $cleanupRetryState = Read-SanitizedState -Context $validated
    Assert-True (
        $cleanupRetryState.cleanup.checks["owner-cleanup-confirmed"] -eq
            $false -and
        [string]$cleanupRetryState.cleanup.status -ceq "partial_failure" -and
        [string]$cleanupRetryState.status -ceq "cleanup_partial_failure" -and
        [string]$cleanupRetryState.agent_board_reservation -ceq "bound" -and
        $null -eq $cleanupRetryState.mutation -and
        $cleanupRetryState.mutation_history.Count -eq 1 -and
        [string]$cleanupRetryState.mutation_history[0].stage -ceq "terminal" -and
        [int]$cleanupRetryOwner.dispatch_count -eq 1 -and
        [int]$cleanupRetryOwner.unsafe_effect_count -eq 1
    ) "A false cleanup readback was not durably retained as retryable partial state."

    $cleanupRetryOwner.recovered = $true
    $cleanupRetryChecks = [ordered]@{}
    foreach ($entry in $cleanupRetryState.cleanup.checks.GetEnumerator()) {
        $cleanupRetryChecks[[string]$entry.Key] = $entry.Value
    }
    [void](Assert-AgentBoardReservation `
        -Context $validated -State $cleanupRetryState)
    Invoke-ModeledJournaledCleanupStep `
        -Context $validated -State $cleanupRetryState `
        -Checks $cleanupRetryChecks -Name "owner-cleanup-confirmed" `
        -Operation $cleanupRetryOperation
    $cleanupRetryTruth = Get-CleanupTruth -Checks $cleanupRetryChecks
    $cleanupRetryState.cleanup.checks = $cleanupRetryChecks
    $cleanupRetryState.cleanup.status = $cleanupRetryTruth.Status
    $cleanupRetryState.status = "complete"
    $cleanupRetryState.phase = "cleanup"
    Write-SanitizedState -Context $validated -State $cleanupRetryState
    Invoke-ModeledJournaledCleanupStep `
        -Context $validated -State $cleanupRetryState `
        -Checks $cleanupRetryChecks -Name "owner-cleanup-confirmed" `
        -Operation { throw "A durable true cleanup check was redispatched." }
    [void](Release-AgentBoardReservation `
        -Context $validated -State $cleanupRetryState)

    $cleanupRetryState = Read-SanitizedState -Context $validated
    $cleanupRetryReceipt =
        Get-Content -LiteralPath $privateReservationPath -Raw |
            ConvertFrom-Json -AsHashtable -Depth 16
    $cleanupRetryBoardAfter =
        Get-Content -LiteralPath $fakeBoardStatePath -Raw |
            ConvertFrom-Json -AsHashtable -Depth 16
    $cleanupRetryExternalLeases = @(
        $cleanupRetryBoardAfter.leases | Where-Object {
            [string]$_.id -cin $cleanupRetryLeaseIds
        }
    )
    Assert-True (
        [int]$cleanupRetryOwner.dispatch_count -eq 2 -and
        [int]$cleanupRetryOwner.unsafe_effect_count -eq 1 -and
        $cleanupRetryState.cleanup.checks["owner-cleanup-confirmed"] -eq
            $true -and
        [string]$cleanupRetryState.cleanup.status -ceq "complete" -and
        [string]$cleanupRetryState.status -ceq "complete" -and
        [string]$cleanupRetryState.agent_board_reservation -ceq "released" -and
        $null -eq $cleanupRetryState.mutation -and
        $cleanupRetryState.mutation_history.Count -eq 2 -and
        [string]$cleanupRetryState.mutation_history[1].stage -ceq "confirmed" -and
        [string]$cleanupRetryReceipt.state -ceq "released" -and
        @($cleanupRetryReceipt.leases).Count -eq 2 -and
        @($cleanupRetryReceipt.leases | Where-Object {
            [string]$_.status -ceq "released"
        }).Count -eq 2 -and
        [int]$cleanupRetryBoardAfter.reserve_count -eq
            [int]$cleanupRetryBoardBefore.reserve_count + 2 -and
        [int]$cleanupRetryBoardAfter.release_count -eq
            [int]$cleanupRetryBoardBefore.release_count + 2 -and
        $cleanupRetryExternalLeases.Count -eq 2 -and
        @($cleanupRetryExternalLeases | Where-Object {
            [string]$_.status -ceq "released"
        }).Count -eq 2
    ) "A recovered owner did not safely redispatch, complete, and release exactly two leases."

    $repeatedFalseBoardBefore = $cleanupRetryBoardAfter
    $repeatedFalseState = New-SanitizedState `
        -Context $validated -Snapshots $syntheticSnapshots
    Write-SanitizedState -Context $validated -State $repeatedFalseState
    [void](Ensure-AgentBoardReservation `
        -Context $validated -State $repeatedFalseState -AllowRepair)
    $repeatedFalseOwner = [ordered]@{
        dispatch_count = 0
        unsafe_effect_count = 0
        effect_applied = $false
        recovered = $false
    }
    $repeatedFalseOperation = {
        $repeatedFalseOwner.dispatch_count =
            [int]$repeatedFalseOwner.dispatch_count + 1
        if (-not [bool]$repeatedFalseOwner.effect_applied) {
            $repeatedFalseOwner.effect_applied = $true
            $repeatedFalseOwner.unsafe_effect_count =
                [int]$repeatedFalseOwner.unsafe_effect_count + 1
        }
        return [bool]$repeatedFalseOwner.recovered
    }.GetNewClosure()
    $repeatedFalseChecks = [ordered]@{
        "device_a-final-readback" = $true
        "device_b-final-readback" = $true
    }
    foreach ($attempt in 1..2) {
        [void](Assert-AgentBoardReservation `
            -Context $validated -State $repeatedFalseState)
        Invoke-ModeledJournaledCleanupStep `
            -Context $validated -State $repeatedFalseState `
            -Checks $repeatedFalseChecks -Name "owner-still-unavailable" `
            -Operation $repeatedFalseOperation
        $repeatedFalseTruth = Get-CleanupTruth -Checks $repeatedFalseChecks
        $repeatedFalseState.cleanup.checks = $repeatedFalseChecks
        $repeatedFalseState.cleanup.status = $repeatedFalseTruth.Status
        $repeatedFalseState.status = "cleanup_partial_failure"
        $repeatedFalseState.phase = "cleanup"
        Write-SanitizedState -Context $validated -State $repeatedFalseState
        $repeatedFalseState = Read-SanitizedState -Context $validated
        $repeatedFalseChecks = [ordered]@{}
        foreach ($entry in $repeatedFalseState.cleanup.checks.GetEnumerator()) {
            $repeatedFalseChecks[[string]$entry.Key] = $entry.Value
        }
    }
    $repeatedFalseBoardPartial =
        Get-Content -LiteralPath $fakeBoardStatePath -Raw |
            ConvertFrom-Json -AsHashtable -Depth 16
    Assert-True (
        [int]$repeatedFalseOwner.dispatch_count -eq 2 -and
        [int]$repeatedFalseOwner.unsafe_effect_count -eq 1 -and
        $repeatedFalseState.cleanup.checks["owner-still-unavailable"] -eq
            $false -and
        [string]$repeatedFalseState.cleanup.status -ceq "partial_failure" -and
        [string]$repeatedFalseState.status -ceq "cleanup_partial_failure" -and
        [string]$repeatedFalseState.agent_board_reservation -ceq "bound" -and
        $null -eq $repeatedFalseState.mutation -and
        $repeatedFalseState.mutation_history.Count -eq 2 -and
        @($repeatedFalseState.mutation_history | Where-Object {
            [string]$_.stage -ceq "terminal"
        }).Count -eq 2 -and
        [int]$repeatedFalseBoardPartial.reserve_count -eq
            [int]$repeatedFalseBoardBefore.reserve_count + 2 -and
        [int]$repeatedFalseBoardPartial.release_count -eq
            [int]$repeatedFalseBoardBefore.release_count
    ) "Repeated false cleanup readbacks did not remain partial or duplicated an unsafe effect."

    $repeatedFalseOwner.recovered = $true
    [void](Assert-AgentBoardReservation `
        -Context $validated -State $repeatedFalseState)
    Invoke-ModeledJournaledCleanupStep `
        -Context $validated -State $repeatedFalseState `
        -Checks $repeatedFalseChecks -Name "owner-still-unavailable" `
        -Operation $repeatedFalseOperation
    $repeatedFalseTruth = Get-CleanupTruth -Checks $repeatedFalseChecks
    $repeatedFalseState.cleanup.checks = $repeatedFalseChecks
    $repeatedFalseState.cleanup.status = $repeatedFalseTruth.Status
    $repeatedFalseState.status = "complete"
    $repeatedFalseState.phase = "cleanup"
    Write-SanitizedState -Context $validated -State $repeatedFalseState
    [void](Release-AgentBoardReservation `
        -Context $validated -State $repeatedFalseState)
    Assert-True (
        [int]$repeatedFalseOwner.dispatch_count -eq 3 -and
        [int]$repeatedFalseOwner.unsafe_effect_count -eq 1 -and
        [string]$repeatedFalseState.agent_board_reservation -ceq "released"
    ) "Repeated-false test leases did not terminalize after modeled recovery."

    $mutationHistoryLimit = & (
        Get-Module FleetWifiAdbTwoQuestAcceptance
    ) {
        $script:MutationHistoryRecentLimit
    }
    $compactionAttemptCount = [int]$mutationHistoryLimit + 12
    $compactionBoardBefore =
        Get-Content -LiteralPath $fakeBoardStatePath -Raw |
            ConvertFrom-Json -AsHashtable -Depth 16
    $compactionState = New-SanitizedState `
        -Context $validated -Snapshots $syntheticSnapshots
    Write-SanitizedState -Context $validated -State $compactionState
    [void](Ensure-AgentBoardReservation `
        -Context $validated -State $compactionState -AllowRepair)
    [void](Assert-AgentBoardReservation `
        -Context $validated -State $compactionState)
    $compactionReceipt =
        Get-Content -LiteralPath $privateReservationPath -Raw |
            ConvertFrom-Json -AsHashtable -Depth 16
    $compactionLeaseIds = @(
        $compactionReceipt.leases | ForEach-Object {
            [string]$_.lease_id
        }
    )
    $compactionOwner = [ordered]@{
        dispatch_count = 0
        unsafe_effect_count = 0
        effect_applied = $false
        recovered = $false
    }
    $compactionOperation = {
        $compactionOwner.dispatch_count =
            [int]$compactionOwner.dispatch_count + 1
        if (-not [bool]$compactionOwner.effect_applied) {
            $compactionOwner.effect_applied = $true
            $compactionOwner.unsafe_effect_count =
                [int]$compactionOwner.unsafe_effect_count + 1
        }
        return [bool]$compactionOwner.recovered
    }.GetNewClosure()
    $compactionChecks = [ordered]@{
        "device_a-final-readback" = $true
        "device_b-final-readback" = $true
    }
    foreach ($attempt in 1..$compactionAttemptCount) {
        Invoke-ModeledJournaledCleanupStep `
            -Context $validated -State $compactionState `
            -Checks $compactionChecks -Name "owner-compaction-retry" `
            -Operation $compactionOperation
    }
    $compactionTruth = Get-CleanupTruth -Checks $compactionChecks
    $compactionState.cleanup.checks = $compactionChecks
    $compactionState.cleanup.status = $compactionTruth.Status
    $compactionState.status = "cleanup_partial_failure"
    $compactionState.phase = "cleanup"
    Write-SanitizedState -Context $validated -State $compactionState

    $compactionStatePath =
        Join-Path $config.private_state_root "acceptance-state.json"
    $compactionState = Read-SanitizedState -Context $validated
    $compactedBeforeRecovery =
        [long]$compactionAttemptCount - [long]$mutationHistoryLimit
    Assert-True (
        [int]$compactionOwner.dispatch_count -eq $compactionAttemptCount -and
        [int]$compactionOwner.unsafe_effect_count -eq 1 -and
        [string]$compactionState.cleanup.status -ceq "partial_failure" -and
        $compactionState.cleanup.checks["owner-compaction-retry"] -eq
            $false -and
        [string]$compactionState.agent_board_reservation -ceq "bound" -and
        $null -eq $compactionState.mutation -and
        @($compactionState.mutation_history).Count -eq
            $mutationHistoryLimit -and
        [long]$compactionState.mutation_history_summary.compacted_count -eq
            $compactedBeforeRecovery -and
        [long]$compactionState.mutation_history_summary.first_ordinal -eq 1 -and
        [long]$compactionState.mutation_history_summary.last_ordinal -eq
            $compactedBeforeRecovery -and
        [long]$compactionState.mutation_history_summary.terminal_count -eq
            $compactedBeforeRecovery -and
        [long]$compactionState.mutation_history_summary.cleanup_attempt_count -eq
            $compactedBeforeRecovery -and
        [long]$compactionState.mutation_history_summary.cleanup_partial_count -eq
            $compactedBeforeRecovery -and
        [string]$compactionState.mutation_history_summary.
            records_commitment_sha256 -cne ("0" * 64) -and
        [string]$compactionState.mutation_history_summary.
            cleanup_key_outcome_sha256 -cne ("0" * 64) -and
        (Get-Item -LiteralPath $compactionStatePath).Length -lt 1048576
    ) "Repeated false cleanup attempts did not compact into a bounded restart state."

    $compactionOwner.recovered = $true
    $compactionChecks = [ordered]@{}
    foreach ($entry in $compactionState.cleanup.checks.GetEnumerator()) {
        $compactionChecks[[string]$entry.Key] = $entry.Value
    }
    [void](Assert-AgentBoardReservation `
        -Context $validated -State $compactionState)
    Invoke-ModeledJournaledCleanupStep `
        -Context $validated -State $compactionState `
        -Checks $compactionChecks -Name "owner-compaction-retry" `
        -Operation $compactionOperation
    $compactionTruth = Get-CleanupTruth -Checks $compactionChecks
    $compactionState.cleanup.checks = $compactionChecks
    $compactionState.cleanup.status = $compactionTruth.Status
    $compactionState.status = "complete"
    $compactionState.phase = "cleanup"
    Write-SanitizedState -Context $validated -State $compactionState
    [void](Release-AgentBoardReservation `
        -Context $validated -State $compactionState)

    $compactionState = Read-SanitizedState -Context $validated
    $compactionReceipt =
        Get-Content -LiteralPath $privateReservationPath -Raw |
            ConvertFrom-Json -AsHashtable -Depth 16
    $compactionBoardAfter =
        Get-Content -LiteralPath $fakeBoardStatePath -Raw |
            ConvertFrom-Json -AsHashtable -Depth 16
    $compactionExternalLeases = @(
        $compactionBoardAfter.leases | Where-Object {
            [string]$_.id -cin $compactionLeaseIds
        }
    )
    $compactionTotalRecords =
        [long]$compactionState.mutation_history_summary.compacted_count +
        [long]@($compactionState.mutation_history).Count
    $boundedStatus = Invoke-FleetWifiAdbTwoQuestAcceptance `
        -Action Status -RunConfig $configPath
    $boundedStatusJson = $boundedStatus | ConvertTo-Json -Depth 16
    $boundedStateJson =
        Get-Content -LiteralPath $compactionStatePath -Raw
    Assert-True (
        [int]$compactionOwner.dispatch_count -eq
            $compactionAttemptCount + 1 -and
        [int]$compactionOwner.unsafe_effect_count -eq 1 -and
        $compactionState.cleanup.checks["owner-compaction-retry"] -eq
            $true -and
        [string]$compactionState.cleanup.status -ceq "complete" -and
        [string]$compactionState.status -ceq "complete" -and
        [string]$compactionState.agent_board_reservation -ceq "released" -and
        $null -eq $compactionState.mutation -and
        $compactionTotalRecords -eq $compactionAttemptCount + 1 -and
        @($compactionState.mutation_history).Count -eq
            $mutationHistoryLimit -and
        [long]$compactionState.mutation_history_summary.compacted_count -eq
            $compactionAttemptCount + 1 - $mutationHistoryLimit -and
        [string]$compactionReceipt.state -ceq "released" -and
        @($compactionReceipt.leases).Count -eq 2 -and
        @($compactionReceipt.leases | Where-Object {
            [string]$_.status -ceq "released"
        }).Count -eq 2 -and
        [int]$compactionBoardAfter.reserve_count -eq
            [int]$compactionBoardBefore.reserve_count + 2 -and
        [int]$compactionBoardAfter.release_count -eq
            [int]$compactionBoardBefore.release_count + 2 -and
        $compactionExternalLeases.Count -eq 2 -and
        @($compactionExternalLeases | Where-Object {
            [string]$_.status -ceq "released"
        }).Count -eq 2 -and
        [string]$boundedStatus.schema -ceq
            "rusty.fleet.wifi_adb_two_quest_status.v2" -and
        [Text.Encoding]::UTF8.GetByteCount($boundedStatusJson) -lt 16384 -and
        -not $boundedStatusJson.Contains(
            "mutation_history", [StringComparison]::Ordinal) -and
        -not $boundedStatusJson.Contains(
            $devices[0].device_id, [StringComparison]::Ordinal) -and
        -not $boundedStatusJson.Contains(
            $devices[0].usb_serial, [StringComparison]::Ordinal) -and
        $boundedStateJson.Length -lt 1048576 -and
        -not $boundedStateJson.Contains(
            $devices[0].device_id, [StringComparison]::Ordinal) -and
        -not $boundedStateJson.Contains(
            $devices[0].usb_serial, [StringComparison]::Ordinal)
    ) "Compacted restart recovery did not complete, remain bounded, or release exactly two leases."

    $validCompactionState = Copy-JsonValue $compactionState
    $tamperedSummaryDigest = Copy-JsonValue $validCompactionState
    $tamperedSummaryDigest.mutation_history_summary.summary_sha256 =
        "f" * 64
    Write-Json -Path $compactionStatePath -Value $tamperedSummaryDigest
    Assert-ThrowsCode -Code "mutation_history_summary_invalid" -Operation {
        Read-SanitizedState -Context $validated | Out-Null
    }

    $tamperedRecordsDigest = Copy-JsonValue $validCompactionState
    $tamperedRecordsDigest.mutation_history_summary.
        records_commitment_sha256 = "0" * 64
    $tamperedRecordsDigest.mutation_history_summary.summary_sha256 =
        Get-TestMutationHistorySummarySha256 `
            -Summary $tamperedRecordsDigest.mutation_history_summary
    Write-Json -Path $compactionStatePath -Value $tamperedRecordsDigest
    Assert-ThrowsCode -Code "mutation_history_summary_invalid" -Operation {
        Read-SanitizedState -Context $validated | Out-Null
    }

    $invalidSummaryOrdinal = Copy-JsonValue $validCompactionState
    $invalidSummaryOrdinal.mutation_history_summary.last_ordinal =
        [long]$invalidSummaryOrdinal.mutation_history_summary.last_ordinal + 1L
    $invalidSummaryOrdinal.mutation_history_summary.summary_sha256 =
        Get-TestMutationHistorySummarySha256 `
            -Summary $invalidSummaryOrdinal.mutation_history_summary
    Write-Json -Path $compactionStatePath -Value $invalidSummaryOrdinal
    Assert-ThrowsCode -Code "mutation_history_summary_invalid" -Operation {
        Read-SanitizedState -Context $validated | Out-Null
    }

    $reorderedRecentHistory = Copy-JsonValue $validCompactionState
    $firstRecent = $reorderedRecentHistory.mutation_history[0]
    $reorderedRecentHistory.mutation_history[0] =
        $reorderedRecentHistory.mutation_history[1]
    $reorderedRecentHistory.mutation_history[1] = $firstRecent
    Write-Json -Path $compactionStatePath -Value $reorderedRecentHistory
    Assert-ThrowsCode -Code "mutation_journal_invalid" -Operation {
        Read-SanitizedState -Context $validated | Out-Null
    }

    $truncatedRecentHistory = Copy-JsonValue $validCompactionState
    $truncatedRecentHistory.mutation_history = @(
        $truncatedRecentHistory.mutation_history | Select-Object -Skip 1
    )
    Write-Json -Path $compactionStatePath -Value $truncatedRecentHistory
    Assert-ThrowsCode -Code "mutation_history_bound_invalid" -Operation {
        Read-SanitizedState -Context $validated | Out-Null
    }
    Write-Json -Path $compactionStatePath -Value $validCompactionState
    $compactionState = Read-SanitizedState -Context $validated

    $modelState = New-SanitizedState `
        -Context $validated -Snapshots $syntheticSnapshots
    Write-SanitizedState -Context $validated -State $modelState
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
        -OwnerId "modeled-owner" -ModeledNoDeviceProjection)
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
        -OwnerId "modeled-owner" -ModeledNoDeviceProjection)
    Set-DurableMutationSent -Context $validated -State $modelState
    $restartResult = Invoke-RunnerProcess -Arguments @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", $runnerPath,
        "-Action", "Resume",
        "-RunConfig", $configPath,
        "-ConfirmMutation"
    )
    Assert-True (
        $restartResult.ExitCode -eq 2 -and
        -not $restartResult.Stderr -and
        [string](($restartResult.Stdout | ConvertFrom-Json).reason_code) -ceq
            "mutation_cleanup_required" -and
        [string](Read-SanitizedState `
            -Context $validated).mutation.stage -ceq "cleanup_required"
    ) "A fresh runner process did not preserve sent-outcome ambiguity and no-redispatch cleanup ownership."

    $modelState = New-SanitizedState `
        -Context $validated -Snapshots $syntheticSnapshots
    Write-SanitizedState -Context $validated -State $modelState
    [void](Start-DurableMutation -Context $validated -State $modelState `
        -Kind "modeled-confirmed" -ActionId "test.confirmed" `
        -OwnerId "modeled-owner" -ModeledNoDeviceProjection)
    Set-DurableMutationSent -Context $validated -State $modelState
    Complete-DurableMutation -Context $validated -State $modelState `
        -ReconciliationCode "modeled_exact_readback"
    $confirmedState = Read-SanitizedState -Context $validated
    Assert-True (
        $null -eq $confirmedState.mutation -and
        $confirmedState.mutation_history.Count -eq 1 -and
        [string]$confirmedState.mutation_history[0].isolation_scope -ceq
            "modeled_not_executed" -and
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
    foreach ($required in @(
        "service.adb.tcp.port",
        "service.adb.tls.port",
        "persist.adb.tls_server.enable",
        "ConvertFrom-ClosedAdbManagerDump",
        "ConvertFrom-ClosedAdbMdnsServices",
        "ConvertFrom-ClosedAdbdSocketOwnerReadback",
        "rusty.fleet.adbd_socket_owner.v3",
        "pre_starttime",
        "pre_fd_begin",
        "post_fd_begin",
        "agent_board_reservation",
        "Assert-AgentBoardReservation",
        "AcquirePinnedFileExecutionLease",
        "agent_board_execution_race_detected",
        "adb_retained_pairing_sha256",
        "wireless_pending_state",
        "host_forward_count",
        "Get-QfmDirectLinkObservation",
        "Set-DurableMutationIsolationAfter",
        "MutationHistoryRecentLimit",
        "mutation_history_summary",
        "mutation_history_summary_invalid",
        "Add-FinalCleanupReadback",
        "cleanup_exact_readback_confirmed"
    )) {
        Assert-True ($trackedText.Contains(
                $required, [StringComparison]::Ordinal)) `
            "Runner omitted required fail-closed state/readback marker $required."
    }
    $placeholderMarker = "not_yet" + "_observed"
    Assert-True (-not $trackedText.Contains(
            $placeholderMarker, [StringComparison]::Ordinal)) `
        "Runner retained a constant direct-link placeholder."

    Write-Output "Rusty Fleet two-Quest Wi-Fi ADB modeled host conformance tests passed."
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
