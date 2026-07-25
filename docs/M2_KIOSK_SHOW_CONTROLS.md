# Milestone 2 Kiosk Show Controls

## Scope

The first Fleet operation vertical is `kiosk.show-controls`. It is a
source-only, app-level control path; it neither requires nor activates ADB.
Fleet owns immutable target planning, explicit confirmation, bounded fan-out,
durable operation state, and operator projections. Manifold remains the
accepted command authority. Rusty Kiosk remains the action owner and is the
only component that can issue the signed effective result proving that its
controls are open.

The owner surface is pinned to Rusty Kiosk revision
`8954228f9ae67c5995a72569e3c9cdd3758f85c0` and
`rusty.kiosk.direct_operator.v1` on port `39873`. The accepted Manifold source
pin is `40c05b27a1c1f6c6990652802e16491bfc1fbc8b`.

## Activation

Source presence is inert. The local Hub offers an operation only when the
current device observation contains a current, enabled, authorized, reachable
`rusty-kiosk.direct-operator` capability owned by `rusty-kiosk`.

Each participating device additionally needs a private local configuration
entry:

```json
{
  "kiosk_direct_operators": [
    {
      "device_id": "example-device",
      "endpoint": "http://192.0.2.10:39873",
      "pairing_key": "<private-pairing-key>"
    }
  ]
}
```

Endpoints are bounded plain-HTTP literal IP authorities on the exact owner
port. Pairing keys stay in private configuration and process memory. They are
redacted from debug output and excluded from durable state, API responses,
receipts, fixtures, logs, and public source.

## Operation flow

1. `POST /fleet/v1/operations/preview` accepts the exact action and a sorted
   map of device IDs to identity revisions.
2. Fleet freezes the current identity, capability evidence, reason, Fleet
   revision, owner contract, expiry, maximum parallelism, and attempt limit.
3. Repeating the same request while that evidence remains current returns the
   byte-equivalent stored preview.
4. `POST /fleet/v1/operations/{operation_id}/execute` must bind the exact
   `operation_id` and `preview_id`.
5. Fleet rechecks all frozen facts, applies the typed command through the
   pinned Manifold Runtime Host, allocates one stable owner action request ID,
   and persists Hub and Manifold state together.
6. Only after durable publication may a bounded worker contact Kiosk. At most
   the frozen `max_parallelism` targets are in flight; later accepted targets
   are admitted as slots complete.
7. A Kiosk invoke acknowledgement is progress, never completion. `Applied`
   requires a signed HTTP 200 `rusty.kiosk.cli_result.v1` result that binds the
   stable action request, reports `command=show-controls`,
   `accepted=true`, `completed=true`, and `controls_open=true`.
8. `GET /fleet/v1/operations/{operation_id}` is a pure read and never polls,
   retries, refreshes, or mutates the operation.

Operation request bodies require `Content-Type: application/json`, one exact
`Content-Length`, a five-second body deadline, and at most 128 KiB. Duplicate
target keys fail before planning.

## Authority and receipt boundaries

The Manifold typed-parameter digest binds the operation ID, preview ID, action
ID, device ID, and identity revision. The Runtime Host snapshot, replay
identities, audit events, Fleet operation, and raw Kiosk owner evidence share
one alternating durable envelope.

An effective Fleet receipt retains the exact owner contract, stable action
request, distinct signed transport request, response digest and signature,
owner result fields, and wrapping time. Restart rehashes the retained raw
bytes and rejects missing or mismatched evidence.

Pairing keys and endpoints are runtime inputs, not authority evidence. Fleet,
Manifold authorization, the Kiosk invoke acknowledgement, and Kiosk effective
result remain separate facts.

## Crash and retry semantics

Fleet persists an owner attempt before any network send. A crash at any point
after that boundary is ambiguous: recovery may only poll the same stable owner
action request ID with fresh signed transport-envelope IDs until the persisted
absolute deadline. It never automatically reinvokes or mints a replacement
action request.

The two durable slots are ordered only after complete nested Hub, Manifold,
operation, identity, authorization, and raw-receipt validation. A
wrapper-valid but nested-invalid newest slot falls back to the older fully
valid slot. Nonterminal recovery targets must still match the current device
identity. Terminal historical operations retain their frozen identity when a
device later re-enrolls.

Failed or expired attempts expose whether a new explicitly requested owner
action ID would be permitted below the attempt limit. Repeating `execute` is
idempotent and never acts as an implicit retry. `show-controls` creates no
temporary effect, so it has no cleanup lifecycle.

## Operator parity

The WPF Console provides exact-target preview, explicit confirmation,
per-target progress/reasons, and accessible text-first results. `fleetctl`
uses the same routes:

```powershell
cargo run --locked -p fleetctl -- operation-preview kiosk.show-controls DEVICE@IDENTITY_REVISION
cargo run --locked -p fleetctl -- operation-execute OPERATION_ID PREVIEW_ID
cargo run --locked -p fleetctl -- operation-get OPERATION_ID
```

The binary connects only to a loopback Hub. Set `RUSTY_FLEET_HUB_URL` to a
different loopback `http://IP:PORT` authority when required.

## Validation

The source gate is device-free:

```powershell
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
dotnet run --project .\apps\fleet-console-wpf.tests\RustyFleet.FleetConsole.Tests.csproj `
  -c Release -- --repo-root .
```

Tests cover owner signing vectors, response authentication before JSON,
distinct transport IDs, poll-only restart recovery, Manifold replay and
terminal revision rejection, immutable previews, identity/capability drift,
bounded dispatch, receipt-gated application, strict local routes, nested
two-slot fallback, CLI parity, and the 1,000-device WPF/UI Automation surface.
Live Kiosk behavior remains an explicit later owner/device gate.
