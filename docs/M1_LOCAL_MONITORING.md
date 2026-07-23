# Milestone 1 Local Monitoring Runtime

## Scope

`fleet-hub-local` is the first runnable Rusty Fleet ingress. It accepts
permission-minimal, low-rate Quest check-ins over an explicitly configured
local HTTP endpoint and projects accepted state through the same `FleetApi`
used by the CLI and future WPF Console.

The runtime does not discover devices, create enrollment, enable ADB, carry
commands, accept files or media, or provide remote relay. Source presence does
not open a listener.

## Activation

Start the runtime only with a private configuration:

```powershell
cargo run --locked -p fleet-hub-local -- --config <private-local-config.json>
```

The configuration schema is `rusty.fleet.local_hub_config.v1` and contains:

- one exact IP socket address in `bind`;
- `allow_non_loopback`, which must be `true` for any LAN bind;
- one absolute private `state_directory`;
- one or more trusted Manifold operator identifiers;
- zero or more configured public credential enrollments; and
- finite stale, offline, condition-history, source-epoch, and event limits.

Enrollment entries contain public keys and authority input, not private signing
seeds. Even so, active device identifiers, endpoints, and enrollment material
belong in private local configuration and must not be committed.

Loopback is the safe default. A LAN bind is an explicit M1 activation and
exposes low-sensitivity signed status without transport confidentiality.
Remote, sensitive, administrative, recording, or media traffic requires a
separately accepted encrypted route.

## HTTP surface

| Method and route | Purpose | Bound |
| --- | --- | --- |
| `GET /fleet/v1/health` | process readiness and aggregate enrollment/device counts | no device payload |
| `POST /fleet/v1/checkins` | signed Quest check-in admission | JSON, 256 KiB, five-second body deadline |
| `POST /fleet/v1/query` | canonical fleet query | JSON, 64 KiB, contract window limit |
| `GET /fleet/v1/summary` | canonical summary projection | current Hub state |
| `GET /fleet/v1/devices/{id}` | canonical full detail | one enrolled device |
| `GET /fleet/v1/devices/{id}/inspect` | canonical inspector projection | one enrolled device |
| `GET /fleet/v1/watch?after_sequence=N&limit=N` | bounded accepted/rejected event window | maximum 10,000 events |

The server caps concurrent requests, applies a finite global and
per-credential check-in rate, does not decompress request bodies, follows no
redirects, and accepts no protocol upgrade route. Malformed, oversized,
expired, wrongly signed, unknown-key, replayed, stale-status, identity-mismatched,
or authority-rejected requests do not advance accepted state.

## Authority sequence

1. Read the body within its size and time limits.
2. Parse the exact signed Fleet envelope.
3. Apply global and enrolled-credential rate limits.
4. Validate contract, time, enrollment, active key, identity binding, and
   Ed25519/JCS signature.
5. Add host-owned receive time and local-ingress freshness evidence.
6. Preview Fleet acceptance on a cloned Hub.
7. Rebind the authority-owned Manifold optimistic lock to current
   fleet-global state.
8. Review the signed device status through the exact pinned Manifold owner.
9. Write a bounded durable snapshot of both candidate states.
10. Commit both in-memory states and acknowledge the check-in, or commit
    neither.

Independent devices therefore retain monotonic per-peer status and per-epoch
source revisions without attempting to coordinate Manifold's fleet-global
authority revision.

## Durable restart boundary

The local runtime writes two alternating JSON state slots below the configured
private `state_directory`. Each accepted check-in advances a durable
generation only after the complete Hub and Manifold candidate snapshot has
been serialized within a 16 MiB ceiling, written to a temporary in the same
directory, flushed, and moved into its slot. The network acknowledgement is
sent only after this write succeeds.

At startup, both slots are parsed and validated against the current Hub policy
and active enrollment configuration. The newest valid generation is restored.
If the newest slot is damaged, the prior valid slot is used and later
check-ins can replay the missing suffix forward. If slots exist but neither is
valid, startup fails closed instead of presenting an empty fleet. Temporary
files are never treated as accepted state.

The snapshot retains the accepted device directory, condition history, watch
sequence, Manifold authority revision, recent unexpired check-in replay
evidence, and source-epoch tombstones. Every collection has a finite policy
bound. When a device exhausts its source-epoch evidence allowance, a new
epoch fails closed rather than evicting an old tombstone and permitting a
previous producer epoch to reappear.

`GET /fleet/v1/health` reports the current durable generation and whether the
process is new or has restored/persisted state, but never exposes the private
state path.

The Quest producer separately persists its producer epoch and next revisions
in app-private state. Ordinary service restarts retain the epoch. An app,
device-identity, identity-revision, or key generation change rotates the
source epoch and resets only its source revision; the per-peer Manifold status
revision remains monotonic.

## Focused validation

```powershell
cargo test -p fleet-manifold-adapter -p fleet-hub-local
cargo clippy -p fleet-manifold-adapter -p fleet-hub-local --all-targets --locked -- -D warnings
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-Repo.ps1 -Tier Quick
```

The focused tests cover signed acceptance, replay rejection before and after
restart, authority rebinding, all-or-neither state mutation, canonical query
parity, explicit LAN activation, content type, body size, finite credential
rate, bounded source-epoch evidence, durable restoration, and damaged-newest
slot fallback.

A private serial-scoped Quest checkpoint has also exercised the Wi-Fi route
without an ADB tunnel: the enrolled device advanced eight signed check-ins,
retained its source epoch across service restarts, transitioned
fresh → stale → offline while remaining visible, recovered to fresh, produced
no fatal error, and was removed with its app-private test inputs after
validation.

A follow-up restart checkpoint exercised the durable Hub boundary with the
producer stopped. Durable generation 8 restored with the same device row,
source epoch, accepted revision and time, condition-history count, Manifold
authority revision, and monitoring-evidence revision. Restarting the Quest
producer then advanced the restored state to durable generation 9 and
authority revision 10 without rotating the producer epoch. The follow-up also
finished with zero package fatals, an empty Hub error log, test-package
removal, and release of the exact local listener.

Raw serial, address, SSID, key seed, profile, receipt, and log evidence remains
private.
