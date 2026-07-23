# ADR 0003: Datastream lifecycle and authority

- Status: accepted for planning
- Date: 2026-07-23

## Context

Rusty Morphospace already has multiple low-rate and high-rate stream families:
status, commands, LSL samples and markers, BLE, ZeroMQ, UDP/OSC, spatial data,
camera/display video, direct-peer media, rendering, ADB diagnostics, and future
relay routes. Their owners and maturity differ.

The original Fleet plan separated control, observation, and media planes, but
did not yet define common product semantics for provider generations, clocks,
progress, buffering, admission budgets, freeze detection, cleanup, and
operator evidence. Newer LSL and display-stream work makes those omissions
material.

## Decision

Adopt the normative
[Datastream Management](../DATASTREAMS.md) contract.

Fleet composes owner manifests and receipts into a common catalog, lifecycle,
budget, health, and operator projection. It does not define a universal wire
protocol or duplicate Manifold, LSL, Quest, Hostess, domain, or application
authority.

Require:

- exactly one payload plane per stream;
- logical stream identity plus provider generation and authority revision;
- named raw timestamp domains and explicit correlation policy;
- bounded lifecycle, queues, recovery, fan-out, retention, and cleanup;
- separate evidence for accepted session, transport/process, bytes,
  sample/frame progress, decode/schema validity, sink progress, and cleanup;
- explicit no-data, stall, freeze, stale, degraded, and failed conditions;
- protected control capacity plus per-device/provider/route/host/relay/global
  admission budgets and fairness;
- selected media preview with no automatic all-device wall;
- low-cardinality metrics and Console/CLI/local API parity.

Treat Rusty LSL as a bounded timestamped observation adapter. Treat FFmpeg as a
bounded Hostess process adapter. Neither becomes Fleet or Manifold authority.

## Consequences

- Milestone 0 gains source-only descriptor, generation, time, health, queue,
  budget, and damaged-stream fixtures without activating a transport.
- Milestone 1 can add bounded status and LSL observations against exact
  promoted owner contracts.
- Milestone 4 becomes a complete selected-stream stack rather than only a video
  preview feature.
- Candidate Quest/Hostess display implementations inform fixtures but are not
  described as supported until owner promotion.
- Remote relay selection remains deferred until identity, tenancy, congestion,
  retention, security, and measured route evidence are available.

## Rejected alternatives

- **One universal transport:** rejected because control, timestamped samples,
  media, and bulk utilities have different delivery and lifecycle needs.
- **One online/healthy bit:** rejected because it hides stale, frozen,
  undecodable, unsunk, or cleanup-failed streams.
- **Convert every timestamp to wall time at intake:** rejected because it
  destroys raw clock evidence and conceals uncertainty.
- **Unbounded buffering for reliability:** rejected because it converts slow
  consumers into latency and memory failures.
- **Treat current feature-worktree evidence as released:** rejected because
  support follows owner-repository promotion, not private or candidate tests.
