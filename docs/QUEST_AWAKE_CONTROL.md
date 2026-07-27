# Quest Awake Control

Rusty Fleet exposes `quest.awake-control` as a disabled-by-default,
operator-confirmed utility for development headsets. It composes Fleet target
policy and scheduling, Manifold command authorization, and QuestIonAble File
Manager's exact-serial ADB provider. It does not add ADB to base fleet
monitoring and it does not make File Manager a fleet controller.

## Actions and meaning

| Fleet action | Provider action | Effect | Lifetime |
| --- | --- | --- | --- |
| `status` | `status` | Read current power, proximity, and watchdog facts | One observation |
| `apply_bounded` | `applyBounded` | Ask Meta's development proximity route for a bounded hold, set stay-on, and wake | 1 minute through 8 hours |
| `start_windows_watchdog` | `repairOnce` per poll | Fleet Hub periodically checks and repairs drift through the pinned provider | Until explicitly stopped, Hub/provider failure, or policy supersession |
| `start_device_watchdog` | `startDeviceWatchdog` | Start the fixed File Manager watchdog process on the exact Quest | Until explicitly stopped, process loss, or headset reboot |
| `stop_watchdogs` | `stopWatchdogs` | Stop Windows and device watchdogs | Settings are deliberately left unchanged |
| `restore_normal` | `restoreNormal` | Stop watchdogs, then restore normal proximity and stay-on settings | Immediate bounded operation |

Stopping watchdogs and restoring normal settings are separate operator
decisions. A stop receipt cannot claim restoration, and a restore receipt
cannot become Applied without an independent restored-settings readback.

The bounded Meta route has a hard maximum of `28,800,000` milliseconds
(8 hours). Both watchdogs repair only observed drift on a default five-second
interval. The interval is explicitly bounded from 1 through 60 seconds.

## Authority and activation

- Fleet owns exact device/identity targets, immutable preview, confirmation,
  scheduling, per-target lifecycle, and CLI/API/Console projection.
- Manifold owns acceptance of the typed command and its expiry/replay
  authority.
- QuestIonAble File Manager owns the ADB commands, exact serial selection,
  device watchdog process, readback parsing, and provider receipt.
- The local Fleet adapter accepts only the exact configured provider filename
  and SHA-256, stages it in a private absolute directory, uses fixed arguments,
  bounds execution/output, and removes the stage.

Source presence is inert. The local Hub returns an unavailable response until
private provider configuration supplies a separately pinned provider plus the
complete three-file Windows ADB bundle, their digests, a private stage root,
and the exact Fleet-device-to-serial map. The adapter copies all four exact
artifacts into a unique private stage, rehashes every staged file, and gives the
provider only the staged pinned ADB path; it does not
inherit whichever `adb` happens to be on `PATH`. The provider environment is
cleared and rebuilt from a small Windows/runtime allowlist. Serial numbers,
executable paths, boot IDs, and raw command output never enter public Fleet
contracts, fixtures, or operation receipts.

A sanitized config shape is:

```json
{
  "quest_awake_provider": {
    "executable_path": "C:\\private\\questionable-file-manager-awake-provider.exe",
    "executable_sha256": "<64-lowercase-hex-provider-sha256>",
    "adb_executable_path": "C:\\private\\platform-tools\\adb.exe",
    "adb_executable_sha256": "<64-lowercase-hex-adb-sha256>",
    "adb_support_artifacts": [
      {
        "source_path": "C:\\private\\platform-tools\\AdbWinApi.dll",
        "sha256": "<64-lowercase-hex-adb-win-api-sha256>"
      },
      {
        "source_path": "C:\\private\\platform-tools\\AdbWinUsbApi.dll",
        "sha256": "<64-lowercase-hex-adb-win-usb-api-sha256>"
      }
    ],
    "private_stage_root": "C:\\private\\rusty-fleet\\awake-stages",
    "targets": [
      {
        "device_id": "device.quest.1",
        "serial": "<exact-private-adb-serial>"
      }
    ]
  }
}
```

All paths and serial values are machine-local private configuration and must
not be copied into a public fixture or receipt. All four SHA-256 pins are
lowercase 64-character digests calculated from the exact files.

## Proof of application

ADB command exit is not application proof. Every Applied target requires the
receipt to bind the exact request, operation, immutable preview, device,
identity revision, action, duration, interval, and watchdog generation.

The receipt keeps power facts independent:

- stay-on effective;
- proximity hold effective;
- wake effective;
- Windows watchdog effective;
- device watchdog effective;
- settings restored;
- settings deliberately left unchanged.

A device-watchdog receipt additionally requires a fresh, active watchdog
readback with the exact generation and interval. Fleet stores only a digest of
the device boot ID so restart/reboot evidence remains comparable without
publishing device-local identity.

The Windows watchdog obtains one initial Manifold-authorized invocation and
renews that authorization on a bounded cadence while it performs provider
polls for the frozen action and generation. Each provider request has a fresh
short expiry. Failure to renew stops repairs without implicitly restoring
settings. A later stop or restore action supersedes the prior watchdog. Three
consecutive provider failures fail the target rather than preserving a false
healthy projection.

Provider calls are serialized per Fleet device and capped globally. Stop and
restore capture the latest independently reported device-watchdog generation
at dispatch. If no generation is known, the invocation carries an explicit
negative sentinel; an unexpectedly active watchdog then fails closed instead
of allowing a stale stop to terminate a newer process.

The device watchdog does not survive reboot. Its readback includes a boot-ID
digest and freshness so Fleet can show that loss explicitly.

## CLI and local API

All commands use the explicit loopback Hub configured by
`RUSTY_FLEET_HUB_URL`:

```powershell
cargo run --locked -p fleetctl -- awake-preview apply-bounded 28800000 5000 device.quest.1@7
cargo run --locked -p fleetctl -- awake-preview start-windows-watchdog 28800000 5000 device.quest.1@7
cargo run --locked -p fleetctl -- awake-preview start-device-watchdog 28800000 5000 device.quest.1@7
cargo run --locked -p fleetctl -- awake-preview stop-watchdogs 28800000 5000 device.quest.1@7
cargo run --locked -p fleetctl -- awake-preview restore-normal 28800000 5000 device.quest.1@7
cargo run --locked -p fleetctl -- awake-execute OPERATION_ID PREVIEW_ID
cargo run --locked -p fleetctl -- awake-get OPERATION_ID
```

The uniform duration and interval fields remain frozen even for status, stop,
and restore so one operation shape and one exact owner binding cover every
action.

The equivalent routes are:

- `POST /fleet/v1/quest-awake/preview`
- `POST /fleet/v1/quest-awake/{operation_id}/execute`
- `GET /fleet/v1/quest-awake/{operation_id}`

## Validation boundary

Repository validation uses synthetic contracts and a fake provider transport.
It does not contact ADB, Meta tools, or a headset. A live proof is a separate
explicit device gate requiring exact-device reservation, current provider
artifact/hash, before/after readbacks, reboot-loss evidence for the device
watchdog, stop-versus-restore checks, and final restoration of the prior state.
