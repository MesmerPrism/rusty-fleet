# Quest Wi-Fi ADB Control

Rusty Fleet exposes `quest.wifi-adb-control` as a disabled-by-default,
operator-confirmed connectivity utility. Fleet owns target policy, immutable
preview, confirmation, bounded scheduling, and per-target ledger. Manifold
authorizes each typed command. A separately SHA-256-pinned QuestIonAble File
Manager provider resolves the device's private Credential Manager profile and
uses the existing Kiosk or USB ADB route.

The provider does not make File Manager a fleet controller. Fleet passes only
portable Fleet identities and operation bindings. Serial numbers, Kiosk
endpoints, pairing codes, executable paths, and raw command output remain in
private machine configuration or File Manager's private profile.

## Actions

| Fleet action | Owner route | Meaning |
| --- | --- | --- |
| `status` | Kiosk readback | Read current Wi-Fi ADB request/setting facts |
| `request_wireless_adb` | Kiosk `request_wifi_adb` | Apply the modern TLS request; the protected Meta wearer prompt remains explicit |
| `enable_request_after_boot` | Kiosk setup helper | Request the same attended prompt after boot |
| `disable_request_after_boot` | Kiosk setup helper | Stop requesting after boot |
| `disable_wireless_adb` | Kiosk `disable_wifi_adb` | Apply the supported disable setting |
| `enable_classic_tcpip_from_usb` | exact USB ADB target | Enable and verify the separate classic `tcpip` route |

Modern TLS request state and classic USB `tcpip` state never alias. A classic
receipt cannot claim Kiosk setting application or wearer approval. A modern
request cannot claim classic transport.

## Independent evidence

The operation keeps these facts separate:

1. provider request delivered;
2. Kiosk setting applied;
3. wearer approval pending, unknown, rejected, or not applicable;
4. listener discovered;
5. authenticated Termux loopback evidence for exact `uid=2000(shell)`.

The File Manager receipt contains only the first four families and cannot
assert Termux usability. Kiosk setting readback does not prove that the wearer
accepted Meta's protected prompt, and absence of listener discovery does not
undo an otherwise valid setting receipt.

Termux usability is reconciled only from the exact
`capability.quest-termux-loopback-adb-shell` capability in an enrolled
Ed25519-signed Fleet check-in. Rusty Quest emits that capability with owner
`quest-termux-lab`, exact ready states, and a strict seven-field extension
payload for the modern-TLS discovery route, listener state, shell UID, and
owner evidence digest. Fleet binds the admitted fact to the signed device
identity, identity revision, source epoch/revision, capability evidence
revision, and observation/freshness window. It contains no operation or
preview identity. After admission, Manifold projects the fact onto matching
device operations only when it is fresh, admitted by the Hub after the
provider receipt, and not superseded by a newer signed check-in where the
capability is absent, a later-admitted disable receipt, or source-epoch
change. Fleet never orders the device-owned proof clock against the Windows
provider clock; it uses trusted local admission order and fails closed when
two admissions share the same millisecond. Superseded and expired proofs are
pruned, and proof IDs and source revisions are replay protected.

There is deliberately no local API or CLI proof-submission route. A caller
cannot make Termux usable by posting a shape-valid JSON object.

## Provider activation

Source presence is inert. The Hub requires a loopback bind and private config:

```json
{
  "quest_connectivity_provider": {
    "executable_path": "C:\\private\\questionable-file-manager-connectivity-provider.exe",
    "executable_sha256": "<64-lowercase-hex-sha256>",
    "private_stage_root": "C:\\private\\rusty-fleet\\connectivity-stages",
    "targets": ["device.quest.1"]
  }
}
```

The adapter verifies and privately stages the exact executable, clears the
environment to a small Windows/runtime allowlist, invokes only
`integration quest-connectivity --json`, bounds output and runtime, validates
every operation binding, and removes the stage. File Manager resolves
`questionable.file_manager.quest_connectivity_profile.v1` for the Fleet
device ID from the current user's Credential Manager.

## CLI and local API

All commands use the explicit loopback Hub selected by
`RUSTY_FLEET_HUB_URL`:

```powershell
cargo run --locked -p fleetctl -- wifi-adb-preview status device.quest.1@7
cargo run --locked -p fleetctl -- wifi-adb-preview request-wireless-adb device.quest.1@7
cargo run --locked -p fleetctl -- wifi-adb-preview enable-request-after-boot device.quest.1@7
cargo run --locked -p fleetctl -- wifi-adb-preview disable-request-after-boot device.quest.1@7
cargo run --locked -p fleetctl -- wifi-adb-preview disable-wireless-adb device.quest.1@7
cargo run --locked -p fleetctl -- wifi-adb-preview enable-classic-tcpip-from-usb device.quest.1@7
cargo run --locked -p fleetctl -- wifi-adb-execute OPERATION_ID PREVIEW_ID
cargo run --locked -p fleetctl -- wifi-adb-get OPERATION_ID
```

Equivalent routes:

- `POST /fleet/v1/quest-wifi-adb/preview`
- `POST /fleet/v1/quest-wifi-adb/{operation_id}/execute`
- `GET /fleet/v1/quest-wifi-adb/{operation_id}`

## Validation boundary

Repository gates use fake provider transport and signed synthetic check-ins.
They prove strict provider binding, private-field exclusion, command
authorization, durable operation state, signed capability presence/absence,
signature mutation, replay, other-device, stale, expiry, disable, reboot, and
renewal behavior. They do not contact ADB, Kiosk, Meta tooling, Credential
Manager, Termux, or a headset.

A live modern-TLS proof remains attended: delivery and setting application can
be automated, but Meta's protected wearer prompt must be accepted in-headset
before the on-device loopback helper can produce a current shell proof.
