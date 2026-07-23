# Rusty Morphospace Datastream Matrix

## Purpose and status language

This inventory consolidates the stream families Rusty Fleet must project. It
is an architecture input, not a promise that every lane is released or
available in the Fleet baseline.

- **accepted:** promoted by the owning repository or an accepted validation
  boundary;
- **candidate:** implemented or evidenced on a current feature branch/worktree
  but not yet promoted through its owner;
- **lab-only:** useful evidence or a probe, not a product dependency;
- **planned:** contract direction without current runtime acceptance.

Private endpoints, device identities, raw captures, and workstation paths are
deliberately omitted.

## Current stream families

| Family | Semantic owner | Current adapters/transports | Time and delivery model | Evidence maturity | Rusty Fleet treatment |
| --- | --- | --- | --- | --- | --- |
| device status | Rusty Quest plus reporting app | authenticated app channel; future local/relay routes | latest accepted state, source and receive time, family freshness | planned Fleet adapter over existing platform observations | base no-ADB condition families with protected capacity |
| command/session/stream authority | Rusty Manifold | revisioned low-rate control routes | identity, revision, replay, TTL, expiry | accepted model/fixture surface | admission and lifecycle authority; never a high-rate payload carrier |
| stream registry/subscription | Rusty Manifold | manifest snapshots/diffs and accepted subscription routes | registry revision, provider/authority epoch, bounded lease | accepted model/fixture surface | canonical catalog input and subscription evidence |
| LSL markers and samples | Rusty LSL | LSL discovery plus TCP sample path; host interop adapters | source monotonic timestamps, clock-offset history, ordered samples/chunks | narrow published feature-candidate; broader qualification in progress | optional observation adapter with explicit unsupported states |
| BLE rendezvous | Rusty Quest / Manifold boundary | authenticated BLE advertisement/GATT proposal | low-rate, bounded, expiring | accepted rendezvous direction; product grants remain Manifold-owned | discovery/topology evidence only |
| BLE status/control | declaring app/Quest adapter | GATT notify/write or bounded RFCOMM profile | low-rate, bounded messages | accepted lab validation for status/control | optional low-rate adapter; never media |
| ZeroMQ bridge | Rusty Manifold adapter | bounded PUB/SUB or declared pattern | transport-specific delivery and queue policy | contract/probe surface | compatibility bridge with explicit loss/backpressure |
| WebSocket bridge | Rusty Manifold adapter | ordered message route | bounded reliable message stream | contract/probe surface | local/relay adapter candidate, not authority |
| UDP telemetry | declaring producer plus Manifold route | best-effort UDP | datagram, loss/reorder expected | contract/probe surface | observation adapter with sequence/freshness |
| OSC | declaring producer plus Manifold route | OSC over UDP | event/sample semantics over best effort | contract/probe surface | bounded compatibility adapter |
| tracked head/hand/space state | Rusty Lattice and Rusty Quest | app/runtime observations; future selected network adapters | monotonic timestamp, relation/frame identity, staleness | accepted model lanes; transport coverage varies | semantic catalog plus rate/freshness projection |
| particle/field/domain data | Rusty Matter or declaring module | explicit selected stream adapters | schema-specific sample/frame/chunk | module-dependent | no generic interpretation beyond manifest and budget |
| camera video | Rusty Quest source; Manifold session; Hostess sink | H.264 media data plane over selected local route | capture time and codec PTS; frame progress | reusable media contract accepted; source profiles vary | selected, receiver-first session only |
| Quest display video | Rusty Quest source; Hostess execution/decode/present | consent capture, privileged direct source, ADB diagnostic source, or compatibility adapter | active-display identity, capture time, H.264 PTS, decoded/rendered progress | current cross-owner implementation candidate with bounded live evidence | keep source classes distinct; do not claim promotion before owner acceptance |
| direct peer media | Rusty Quest socket/runtime under Manifold authorization | interface-bound Wi-Fi Direct socket plus H.264 framing | provider epoch, route/session revision, media timestamps | accepted two-device bounded validation; N-peer remains unproven | optional local route with exact topology evidence |
| stereo rendering | Rusty Quest sink | native OpenXR/Vulkan application route | frame timing and current runtime receipt | accepted bounded rendering evidence | sink capability, never inferred from transport |
| WPF media presentation | Rusty Hostess | native decode/presentation path; FFmpeg-compatible process adapter may assist | decode/render progress and queue timing | current implementation candidate | selected inspector preview with CLI/API evidence parity |
| ADB screen stream | QuestIonAble File Manager/Hostess diagnostic boundary | serial-scoped ADB `screenrecord` or compatibility tool | process lifetime and H.264 progress | bounded device evidence; platform support is not guaranteed | explicit privileged diagnostic source, not base media |
| file transfer | QuestIonAble File Manager | USB/Wi-Fi ADB or approved app route | chunk/progress/completion semantics | existing product capability | utility operation, not observation/media |
| online media relay | future relay adapter under Manifold authority | candidates include SRT, WebRTC, or QUIC-based routes | congestion, latency, loss, relay lease, end-to-end identity | planned/research only | select only after M5 threat, tenancy, retention, and measurements |

## Contract surfaces already available

The current Manifold model provides a strong starting point:

- stream manifests name source module, semantic family, sample schema, rate
  class, timestamp domains, retention, sensitivity, offers, and subscription
  policy;
- registry snapshots and diffs are revisioned;
- subscription decisions bind subscriber, stream, transport, authority and
  registry revisions, TTL/expiry, capability, and active subscriber count;
- media-session acceptance binds exact product/feature/grant/runtime/provider
  lineage and the source/processor/route/sink/stream identities;
- bridge-route fixtures distinguish plane, payload class, rate, delivery,
  timing, evidence, conditions, profiles, and fallback.

Rusty Quest's generic media plan already separates device/source/lane/runtime
endpoint/transport/security/observability and independently owns:

```text
source + processor + route + socket + codec + sink + cleanup
```

Fleet should adapt these contracts and add product composition, budgets,
queries, and projections. It should not mint a competing stream manifest.

## Newer display-stream lessons

Recent bounded Quest/Hostess implementation work adds design pressure that was
not explicit in the older connectivity plan:

- the active Quest display may not be display zero, so source selection and
  receipts must name the effective display;
- Android MediaProjection remains consent-based and revocable; a laboratory
  grant shortcut is not a production capability;
- a producer can overrun its requested cadence, so effective frame rate and
  resource use must be measured;
- newest-frame and keyframe/configuration-aware backpressure can preserve live
  preview while preventing a slow presenter from stalling the receiver;
- process alive, bytes received, decoded frames, rendered frames, and changing
  frame content are separate evidence stages;
- decoded-frame identity over a bounded visual stimulus distinguishes healthy
  progress from a frozen pipeline;
- direct, consent-based, ADB diagnostic, and compatibility sources need
  separate support and security classifications;
- cleanup must independently account for processes, temporary packages/files,
  network rules, services, and restored grants.

These are candidate implementation findings until their owning repositories
promote the exact branches. Fleet may encode the negative paths and generic
contracts now without presenting the implementations as released.

## Rusty LSL delta

Rusty LSL has advanced beyond the older connectivity evidence. Its current
release-candidate surface includes explicit receipt-bound activation, bounded
discovery and selection, exact typed transfer fixtures, finite recovery and
clock correction, bounded sample queues, typed loss/health observations, and
cleanup/address-reuse evidence.

The claims remain intentionally narrow:

- arbitrary sample shapes and broad official-liblsl interoperability are not
  assumed;
- ambient or unbounded discovery and operation remain out of scope;
- current host/device breadth is incomplete;
- recovery requires a usable stable source identity and becomes a new Fleet
  provider generation;
- observations and advisory proposals never become Manifold authority.

Fleet M0 should model the descriptor, time, health, queue, and unsupported-case
fixtures. M1 may admit a bounded LSL observation adapter only against an exact
promoted Rusty LSL contract.

## Gaps Fleet must close

1. One product descriptor/projection that composes existing owner manifests
   without replacing them.
2. Provider-generation semantics across LSL recovery, capture restart, route
   replacement, and codec recreation.
3. Explicit clock-domain and correlation records shared by status, LSL, and
   media projections.
4. Uniform no-data/stall/freeze/decode/sink/cleanup vocabulary.
5. Per-class queue bounds, drop policy, control-plane reserve, and slow-consumer
   behavior.
6. Per-device/provider/host/route/relay/global budgets plus fair admission.
7. Low-cardinality metrics and detailed evidence that remain useful at fleet
   scale.
8. Exact recording, retention, sensitivity, export, and relay policy.
9. Selected-stream UI and CLI/API parity without an ambient video wall.
10. Owner-promotion gates so candidate and lab evidence cannot masquerade as
    supported product capability.

The normative solution is
[Datastream Management](../DATASTREAMS.md).
