# ADR 0004: Milestone 0 source boundary and threat model

- Status: accepted for Milestone 0
- Date: 2026-07-23

## Context

Milestone 0 introduces executable contracts, an in-memory Hub, deterministic
synthetic observations, and CLI/local-API projections. It intentionally has no
network, device, persistence, or media adapter, but its data model will become
the first trust boundary used by those later adapters.

Treating a deserialized observation as trustworthy would make later transport
authentication ineffective. Treating one monotonically increasing revision as
sufficient would also make legitimate producer restarts ambiguous and could
allow an old producer instance to overwrite current state.

## Assets and trust boundaries

The protected assets are:

- enrolled device identity and identity revision;
- current producer epoch and revision;
- last accepted device and stream state;
- authority, timing, sensitivity, and freshness lineage;
- canonical query membership and result revision;
- per-device command and cleanup evidence;
- role-controlled native descriptors and recording provenance;
- bounded Hub memory, query work, and event history.

Synthetic fixtures, local CLI arguments, future adapter observations, native
descriptors, extension maps, timestamps, revisions, and saved queries are all
untrusted input until contract and authority checks pass. Rusty Fleet does not
turn LSL discovery, ADB identity, a socket peer, or a device-provided label into
enrollment authority.

## Threats considered in Milestone 0

| Threat | Milestone 0 mitigation |
| --- | --- |
| Duplicate, stale, or reordered observation | Current source epoch and source revision must advance monotonically |
| Old producer resumes after restart | Every producer restart changes `source_epoch`; previously seen epochs cannot become current again |
| Revision reset without a restart | A new source epoch or identity revision must begin at source revision one |
| Identity rollback or substitution | Identity revision cannot decrease; stable identity fields cannot conflict at one revision |
| Receive-time regression | A higher revision in the same source epoch cannot move receive time backwards |
| Optimistic device replacement after rejection | Rejected input creates an event but leaves the last accepted record unchanged |
| Missing/stale data presented as current | Condition deadlines and Hub stale/offline policy produce explicit independent states |
| Query ambiguity or hidden broad scope | Canonical expressions, qualifiers, sort keys, result revision, total, and window remain explicit |
| Memory or work amplification | Observation, condition, capability, query, stream, queue, history, target, extension, and event collections have finite limits |
| Oversized native metadata | Inline native descriptors and complete observations have byte ceilings; future ingress must also bound bytes before deserialization |
| Unknown-field erasure | Declared extension maps retain bounded unknown fields for forward compatibility |
| ADB/media/relay activation by source presence | Empty feature lock plus no runtime dependencies, listeners, or adapter implementations |
| CLI semantics diverge from UI/API | `fleetctl` invokes the same `FleetApi`; parity tests require exact serialized projections |

## Decision

Adopt a three-part device observation sequence:

```text
enrolled identity revision
    + producer source epoch
    + source revision within that epoch
```

The Hub retains previously seen source epochs per enrolled device. A new epoch
is accepted only at revision one. Returning to an old epoch is replay, even if
that producer advertises a higher revision. A new identity revision must also
start its observation sequence at revision one.

Keep time explicit. Source time is evidence in its named producer domain;
receive time is adapter evidence. Neither becomes authority merely because it
is newer. The caller supplies Hub evaluation time, which keeps tests and
projections deterministic.

Apply finite contract limits before state admission. A future byte ingress must
add a pre-deserialization envelope limit and authenticated owner binding; M0’s
in-memory validation is not claimed as a secure network parser.

Keep the M0 dependency graph source-only. Persistence, listeners, protocol
authentication, role enforcement, secret storage, device grants, sandboxed
processes, and rate limiting remain closed later boundaries rather than mocked
security claims.

## Consequences

- Agent restart and reconnect are no longer conflated.
- An observation may be syntactically valid but rejected without disturbing
  current device state.
- Source epoch is visible in canonical row/inspector/detail projections.
- Later adapters must bind authenticated producer identity to the enrolled
  identity and current epoch before calling Hub admission.
- Contract limits may require explicit version changes for legitimate larger
  payloads rather than silent unbounded growth.
- M0 can validate replay, ordering, ambiguity, freshness, and resource bounds
  without opening a real attack surface.

## Deferred security work

Before M1 networking, define authenticated ingress, key rotation, byte-rate and
connection limits, pre-deserialization framing bounds, revocation, and adapter
restart ownership. Before media or FFmpeg, define process confinement, input
allowlists, credential handling, resource ceilings, and process-tree cleanup.
Before relay, complete tenancy, consent, retention, incident, congestion, and
cost threat models.

## Rejected alternatives

- **Revision without source epoch:** rejected because restart and replay cannot
  be distinguished safely.
- **Trust wall time to select the newest observation:** rejected because clock
  skew and forgery are not authority.
- **Accept unknown producer epochs at any revision:** rejected because an old
  or substituted producer could skip restart proof.
- **Add a network/authentication dependency in M0:** rejected because it would
  activate a boundary before an exact owner and protocol are selected.
- **Claim secure parsing from post-deserialization validation:** rejected
  because allocation and parser limits must also exist before deserialization.
