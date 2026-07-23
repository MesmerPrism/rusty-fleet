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
- one or more trusted Manifold operator identifiers;
- zero or more configured public credential enrollments; and
- finite stale, offline, history, and event limits.

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
9. Commit both states, or neither.

Independent devices therefore retain monotonic per-peer status and per-epoch
source revisions without attempting to coordinate Manifold's fleet-global
authority revision.

## Current restart boundary

The local runtime is intentionally in-memory at this checkpoint. Restart
requires reloading the private enrollment configuration and loses accepted
device/status history. Durable bounded recovery remains a required M1
acceptance item before the runtime is presented as restart-safe; the
configuration and state-engine boundaries are kept separate so persistence
does not become transport or device authority.

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

The focused tests cover signed acceptance, replay rejection, authority
rebinding, all-or-neither state mutation, canonical query parity, explicit LAN
activation, content type, body size, and finite credential rate. Live Quest
validation remains a separate serial-scoped device gate with private evidence
and cleanup.
