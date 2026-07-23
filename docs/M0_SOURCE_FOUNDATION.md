# Milestone 0 Source Foundation

## Purpose

Milestone 0 establishes one executable but inert vertical slice. It proves
Rusty Fleet’s product-owned contracts, deterministic state engine, synthetic
multi-device behavior, and CLI/local-API projections without opening a
listener, connecting a device, or adopting another repository’s authority.

This document describes implemented source. It does not promote LSL, Quest,
Kiosk, File Manager, Manifold, media, relay, or persistence behavior.

## Source dependency direction

```text
fleet-contracts
    ↑              ↑
fleet-simulator    fleet-hub
          \         /
             fleetctl
```

- `fleet-contracts` owns public model shape and cross-field validation.
- `fleet-simulator` creates synthetic owner-shaped observations. It does not
  emulate an authority or a transport.
- `fleet-hub` accepts or rejects observations and creates canonical
  projections under caller-supplied time.
- `fleetctl` loads a deterministic scenario and invokes the same `FleetApi`
  methods used by direct callers.

No crate in this graph depends on an asynchronous runtime, network library,
Windows API, Android API, device tool, codec, LSL library, or database.

## Contract families

The v1 source contracts cover:

- enrolled identity and identity revision;
- independent condition families with source, receive, acceptance, authority,
  sensitivity, and freshness;
- capability support, enablement, authorization, reachability, and freshness;
- canonical query expressions, sorting, windows, and saved views;
- summary, row, inspector, detail, and query-result projections;
- target snapshots, per-device results, and command lifecycle;
- normalized and native stream descriptors;
- deterministic source selection and unresolved ambiguity;
- source, route, processing, and sink epochs;
- named timing domains, transforms, uncertainty, resets, and calibration;
- cadence, valid silence, and no-data policy;
- profile-specific progress-stage applicability;
- per-edge finite queue behavior and admission budgets;
- scientific run, recording, retention, cleanup, and replay provenance.

Unknown fields round-trip through declared extension maps. Cross-field
invariants fail closed in Rust even when the public JSON Schema can only
describe the structural envelope.

## Hub acceptance rules

`FleetHub` is in memory and receives both an observation and `now_ms` from its
caller. It never reads a system clock.

An observation is rejected when:

- contract validation fails;
- the source revision is duplicate or older;
- identity revision rolls back;
- an identity revision changes without restarting the source revision at one;
- stable identity fields conflict at the same identity revision.

Accepted device records advance independently. A rejection creates a watch
event but does not change the last accepted device state. Stale and offline
are time-based projections over retained enrolled records; they do not delete
the device or convert unavailable optional capabilities into device failure.

## Query and projection behavior

The local API provides:

- `list` with canonical predicates, stable sorting, result revision, as-of
  time, total count, and a bounded window;
- `inspect` with current independent conditions, stream descriptors, and
  actionable attention;
- `detail` with retained condition history;
- `summary` with fresh, stale, offline, attention, and active-work counts;
- `watch` with bounded, monotonically sequenced accept/reject events.

Queries are validated before evaluation. Tag and capability predicates require
an explicit qualifier, numeric fields reject text comparisons, and stable
device identity breaks sort ties.

## Deterministic scenarios

`fleet-simulator` uses one pinned seed and base time. It generates 4, 50, 250,
1,000, and 5,000-device datasets, then provides:

- reconnect and re-enrollment;
- replay and reordered revisions;
- conflicting duplicate identity;
- capability downgrade;
- partial status families;
- syntactically valid but contract-invalid messages.

The committed scenario manifest binds seed, sizes, and mutation names. Tests
regenerate data in memory and compare deterministic results. These sizes are
validation fixtures, not production support claims or measured budgets.

## CLI/local-API parity

`fleetctl` exposes `list`, `inspect`, `filter`, `watch`, and `scenario` as
structured JSON. Parity tests serialize direct `FleetApi` results and require
exact equality with the corresponding CLI projection. The CLI has no hidden
query or state engine.

## Closed adapter edges

Milestone 0 deliberately leaves the following edges closed:

| Edge | Current state | Later owner/gate |
| --- | --- | --- |
| Network observation ingress | No listener or protocol | M1, exact Quest/Manifold adapter |
| Persistence | In-memory records only | Later measured store contract |
| Kiosk action dispatch | Contracts only | M2, Kiosk owner receipt |
| ADB/File Manager | Disabled capability data only | M3, separate opt-in adapter |
| LSL | Native descriptor fixture only | Exact Rusty LSL promotion |
| Media/FFmpeg | Contracts and negative policy only | M4, Hostess process adapter |
| Relay/cloud | No dependency or route | M5 measured candidate decision |
| WPF | Canonical projections only | UI milestone with accessibility gate |

Source presence is not activation. Every later edge requires its feature lock,
runtime input, owner authority, negative paths, and effective receipt.

## Validation

Quick runs formatting, all locked workspace tests, public-boundary checks,
schema/fixture checks, and workflow invariants. Standard adds warnings-denied
Clippy, Markdown/link checks, a structured CLI smoke run, and optional portable
workflow validation.

No M0 validation command invokes a headset or device tool.
