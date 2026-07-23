# Fleet Research Integration Review

## Purpose

This note records the 2026-07-23 review of two external research handoffs:

- an operator-UI, accessibility, WPF, scale, and multi-device operations
  report;
- an LSL, scientific recording, timing, FFmpeg, media reliability,
  observability, and remote-transport report.

The handoffs were treated as untrusted research inputs. Their recommendations
were compared with the current repository, checked against primary or official
sources in the UI and datastream ledgers, and integrated only where they
strengthened an existing Rusty Fleet boundary. Raw conversation transcripts,
opaque citation tokens, private machine paths, and proprietary product assets
are not part of the public repository.

## Clean integration boundary

This review is a planning-only pre-M0 checkpoint. It does not:

- transition the proposed M0 unit;
- create sockets, WPF, LSL, FFmpeg, recording, media, relay, or device effects;
- select a runtime module, permission, transport, codec, or UI dependency;
- promote candidate behavior from another Rusty Morphospace repository;
- split M0 into research-derived micro-units.

The updated contracts and fixtures remain one coherent M0 foundation stack.

## Operator UI verdict

The existing UI plan already incorporated the consequential findings:

- dense virtualized fleet table as the primary workspace;
- persistent selected-device inspector and restorable navigation context;
- independent timestamped condition families rather than one health score;
- visible canonical query, filters, grouping, saved scope, and selection;
- stable live ordering during operator interaction;
- frozen target snapshots, per-target preflight, and per-target operation
  ledgers;
- first-class cancellation, retry, cleanup, degraded, empty, stale, and
  permission-limited states;
- WPF/UI Automation, keyboard, Narrator, high contrast, scaling, and
  representative-fleet acceptance;
- a dependency spike before adopting a Fluent shell/theme library;
- selected, bounded media preview rather than an automatic all-device wall.

A separate anti-template UI guardrail review also supports the current
practical-product direction: normal navigation, quiet structure, restrained
semantic color, purposeful status indicators, and a real table rather than
KPI-card grids, decorative badges, glass panels, gradients, or “control room”
ornament.

No new UI architecture or lifecycle unit was needed. The datastream detail and
recording-preflight additions in
[Operator UI Architecture](../OPERATOR_UI.md) are projections over the refined
stream contract, not a redesign.

## Datastream changes accepted

The follow-up report identified real gaps in the earlier consolidated plan.
This checkpoint accepts:

1. **Dual descriptors.** Fleet keeps a normalized semantic descriptor and the
   complete native descriptor or a role-controlled reference plus digest.
2. **Auditable source selection.** Query, expected cardinality, candidates,
   selected native instance, method, expiry, and override lineage are explicit.
   “First result wins” is prohibited.
3. **Owner-scoped component epochs.** Source, route, processing, and sink
   epochs replace one overloaded provider-generation authority. A composite
   path generation remains a concise projection only.
4. **Complete timing lineage.** Raw domains, offset observations, uncertainty,
   transformations, validity intervals, reset events, and fixed-latency
   calibration provenance remain inspectable.
5. **Cadence and absence policy.** Nominal/requested and measured rates remain
   distinct; irregular and event-driven silence is not automatically failure.
6. **Profile-specific progress.** Common stages are required, optional, or not
   applicable for each stream profile.
7. **Per-edge flow control.** Preview, recording, relay, analysis, and other
   consumer branches own separate finite queues and failure behavior.
8. **Scientific run and artifact provenance.** Required/optional streams,
   marker schema, native metadata, timing history, deviations, recording,
   retention, checksum, cleanup, and replay are explicit.
9. **LabRecorder/XDF compatibility.** Fleet preserves the established native
   artifact workflow and adds receipts; it does not invent a replacement file
   format.
10. **FFmpeg CLI first.** Hostess owns a pinned, allowlisted, bounded process
    adapter with stage watchdogs and complete process-tree cleanup.
11. **Bounded observability.** Metrics use a reviewed low-cardinality label
    set; detailed device, stream, endpoint, request, and metadata fields remain
    in bounded records, logs, traces, or audit.
12. **Measured relay selection.** SRT, RIST, WebRTC, and QUIC remain M5
    candidates evaluated per payload class; no universal transport is chosen.

## Recommendations retained without change

- Control, observation, and media planes remain separate.
- Manifold remains admission/session/stream authority.
- Rusty LSL remains a scientific compatibility and observation adapter.
- Rusty Quest remains platform/capture/effective-device authority.
- Rusty Hostess remains FFmpeg/process/decode/presentation authority.
- Rusty Fleet owns catalog composition, budgets, fairness, selection, and
  operator projections.
- Base status/control capacity remains protected.
- Candidate owner-repository evidence remains candidate until exact promotion.

## Deferred

- real LabRecorder or XDF process integration;
- Fleet-controlled scientific recording;
- exact liblsl and FFmpeg dependency versions;
- final queue, chunk, rate, latency, storage, and fleet-size thresholds;
- direct `libav*` integration;
- production remote transport, relay, tenancy, or cloud deployment;
- broad LSL shape/device interoperability;
- BIDS-aware export or richer experiment templates.

These require the milestone and owner gates already named in the implementation
plan.

## Rejected

- LSL discovery or `source_id` as enrollment/authentication;
- silently selecting the first discovered stream;
- discarding raw timestamps after online correction;
- one provider generation for unrelated component changes;
- one queue policy for all consumer branches;
- one universal healthy state or progress percentage;
- process-alive, byte-flow, or probe success as sink health;
- freeze detection for streams that may legitimately remain static or silent;
- unbounded buffering, retries, recovery, fan-out, recording, or offline work;
- ambient all-device media preview;
- FFmpeg, GStreamer, or a relay becoming product authority;
- direct `libav*` before a measured need and ownership plan;
- selecting a universal remote media transport in advance.

## Files affected

- [Datastream Management](../DATASTREAMS.md)
- [Architecture](../ARCHITECTURE.md)
- [Implementation Plan](../IMPLEMENTATION_PLAN.md)
- [Operator UI Architecture](../OPERATOR_UI.md)
- [Validation](../VALIDATION.md)
- [ADR 0003](../decisions/0003-datastream-lifecycle-and-authority.md)
- [Morphospace Datastream Matrix](MORPHOSPACE_DATASTREAM_MATRIX.md)
- [Datastream Reference Ledger](DATASTREAM_REFERENCE_LEDGER.md)
- the proposed M0 iteration-unit contract
