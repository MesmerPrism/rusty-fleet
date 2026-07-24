# ADR 0006: Milestone 1 local ingress threat model

- Status: accepted for Milestone 1 implementation
- Date: 2026-07-23

## Context

Milestone 1 opens Rusty Fleet's first runtime network boundary. The route is for
permission-minimal, low-rate local check-ins when no ADB connection is present.
It is not a media plane, scientific sample route, file-transfer service, remote
relay, or command channel.

An authenticated payload alone does not make a listener safe. Framing,
connection count, body size, parse work, request rate, stale connections,
logging, persistence, restart, and shutdown all require bounded behavior.
Signed status is also integrity evidence, not confidentiality for data exposed
on the local network.

## Assets and trust boundaries

Protect:

- enrolled Manifold peer identity, active credential, and authority revision;
- current Fleet device state, received time, history, and replay evidence;
- Hub availability and bounded memory, CPU, disk, connection, and event use;
- low-sensitivity status provenance;
- exact adapter configuration and effective listener receipt; and
- clean shutdown of listeners, tasks, files, and secrets.

Treat every network peer, header, byte count, body, JSON value, timestamp,
identifier, extension, and device observation as untrusted before admission.

## Decision

The first ingress profile is an explicit local Hub endpoint carrying signed
JSON check-ins over a standard bounded request/response transport. It has no
ambient discovery or broadcast authority. Runtime activation requires the
selected feature lock, an approved listener address/port, enrolled credentials,
and an effective Hub receipt.

The implementation must enforce, before or during parsing:

- a small header limit and an M1 body ceiling no larger than the contract;
- finite concurrent connections and pending work;
- connect, header, body, processing, response, and idle deadlines;
- finite per-peer and global request rates;
- exact accepted method, route, content type, schema, and signature profile;
- no request pipelining or upgrade path unless separately accepted;
- no high-rate samples, media, file bytes, arbitrary commands, or native
  descriptors in the check-in route;
- bounded replay state with expiry, plus Manifold and source revisions;
- sanitized logs without keys, signatures, full device serials, private
  endpoints, or raw bodies;
- bounded persistence with atomic recovery and a damaged-state failure mode;
- deterministic listener stop and task/resource cleanup.

The listener records host received time. Device source time is retained as a
separate signed fact and never substitutes for received time.

Because the first local profile does not promise transport confidentiality,
only low-sensitivity status approved by the Quest Fleet Agent profile may be
sent. Future remote relay, sensitive telemetry, recording, or administrative
operation requires an independently accepted encrypted and mutually
authenticated route.

## Required negative paths

- unsigned, wrongly signed, unknown-key, expired-key, and revoked-key requests;
- duplicate check-in and replayed Manifold proposal;
- stale/future window and source/authority revision rollback;
- peer identity different from the observation identity;
- truncated, oversized, malformed, slow, or compressed-amplification input;
- connection flood, request-rate excess, and stalled client;
- Hub persistence damage or restart between requests;
- adapter stop while work is active;
- optional LSL, ADB, File Manager, media, or relay capability absent;
- one failed device or adapter without deleting unaffected devices.

## Consequences

- M1 can be useful over ordinary Wi-Fi without treating the LAN as trusted.
- Integrity and authority are independently testable before transport
  confidentiality is introduced.
- The low-rate route cannot silently become a bulk or media tunnel.
- Hub restart and degraded-adapter behavior become acceptance paths rather
  than release-day surprises.
- Discovery remains a later, separate proposal mechanism and cannot enroll or
  select a device by itself.

## Durable-state realization

M1 implements the decision as two alternating, generation-numbered state
slots below an explicitly configured absolute private directory. A check-in is
acknowledged only after the candidate Fleet and Manifold snapshots fit the
fixed state-size ceiling and the new slot has been flushed and published.
Startup chooses the newest fully valid slot, falls back to the prior valid slot
when only the newest is damaged, and fails closed when no valid slot remains.

The restored snapshot must agree with the current Hub policy and active
enrollment bindings. Fleet device identities must exactly match accepted
Manifold peers. Condition history, watch events, unexpired replay evidence,
applied proposal evidence, and source-epoch tombstones remain finite. A full
source-epoch tombstone allowance rejects further epoch rotation instead of
evicting evidence that would let an old producer epoch return.

## Rejected alternatives

- **Unbounded HTTP/WebSocket body or connection handling:** authentication
  occurs too late to prevent resource exhaustion.
- **UDP broadcast as enrollment or status authority:** delivery, ambiguity,
  fragmentation, and source addressing do not provide identity.
- **Reuse the low-rate route for media or files:** it would erase backpressure,
  consent, and owner boundaries.
- **Log rejected raw payloads by default:** this leaks untrusted or sensitive
  content into evidence.
- **Call a signed request confidential:** signatures do not hide payloads.
- **Require USB or Wi-Fi ADB:** no ADB remains the base product condition.
