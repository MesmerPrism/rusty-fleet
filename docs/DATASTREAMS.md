# Datastream Management

## Decision

Rusty Fleet manages datastreams through one product-level lifecycle and
projection contract. It does not define a universal wire protocol and it does
not absorb the owners of LSL, media, BLE, ZeroMQ, sockets, codecs, capture, or
rendering.

Every stream remains on exactly one payload plane:

- **control:** low-rate commands, admission, leases, revisions, and stream
  references;
- **observation:** bounded status, telemetry, markers, poses, and discovery
  proposals;
- **media:** selected high-rate audio, video, image, depth, mesh, or other
  bulk frame payloads.

Control may authorize or describe a stream. It never carries high-rate samples
or frames.

## Product responsibilities

Rusty Fleet owns:

- catalog composition and operator queries over accepted stream manifests;
- current availability, lifecycle, health, cost, and sensitivity projections;
- per-device and global admission budgets;
- selected subscriptions and media-session requests;
- fairness, prioritization, and operator policy;
- Console, CLI, and local API parity;
- fleet-level evidence and alert projections.

The owning systems remain authoritative:

| Concern | Owner |
| --- | --- |
| accepted manifests, subscriptions, sessions, revisions, leases, replay, expiry, and audit | Rusty Manifold |
| LSL discovery, inlet/outlet compatibility, sample transfer, recovery, and clock observations | Rusty LSL |
| Quest capture, Android permission, codec adoption, platform socket binding, and effective runtime receipts | Rusty Quest |
| Windows process execution, normalization, decode, presentation, and evidence capture | Rusty Hostess |
| tracked-space semantics and timestamped spatial observations | Rusty Lattice |
| particle, field, and other domain payload semantics | Rusty Matter or the declaring domain owner |
| app-specific production and consumption | the participating application |
| product composition, budgets, selection, and operator projection | Rusty Fleet |

An adapter may report an observation or proposal. Only the named authority can
accept it.

## Stream classes

The first implementation must support the following semantic classes without
pretending they have identical transport or timing behavior:

| Class | Typical examples | Default plane | Delivery emphasis |
| --- | --- | --- | --- |
| status | battery, charge, thermal, lifecycle, foreground | control/observation | latest valid state, bounded history |
| event/marker | experiment marker, app transition, command result | observation | ordered identity and timestamp |
| scalar/sample | sensors, biosignals, low-rate metrics | observation | timestamp fidelity and bounded loss |
| spatial | head/hand pose, tracked relations, frame state | observation | freshness and frame-of-reference integrity |
| bulk structured | meshes, point clouds, assets | media or an explicit bulk route | chunk identity and bounded transfer |
| media | camera, display, audio, stereo image | media | frame progress, bounded latency, decode health |
| control/bulk utility | ADB/file transfer | control plus a separate utility route | exact target and completion evidence |

ADB and file transfer are not fleet telemetry or media merely because they move
bytes. BLE rendezvous is not a media transport. LSL discovery is not fleet
enrollment.

## Canonical stream descriptor

Fleet projections use a versioned descriptor derived from owner contracts. It
must carry enough information to compare streams without replacing the native
manifest:

- stable `stream_id`, semantic family, sample/frame schema, and sensitivity;
- source device, source module/application, and source role;
- payload plane, rate class, delivery model, and retention class;
- source, processor, route/socket, codec/framing, and sink identities where
  applicable;
- provider generation and owner authority revision;
- offered transports and the currently accepted transport;
- timestamp domains, clock-correlation policy, and known fixed latency;
- expected cadence or explicit irregular/event-driven declaration;
- buffering and drop policy;
- current subscription/session reference and expiry;
- lifecycle, health, and terminal cleanup evidence;
- operator-visible cost estimates and current measured use;
- visibility/export/recording policy.

Unknown fields remain round-trippable where the owner contract permits it.
Unknown required semantics fail closed.

## Identity and provider generations

A stable stream identity describes the logical stream. A provider generation
describes one concrete production epoch.

A generation changes when a producer restarts, a recoverable LSL stream is
re-resolved, a capture permission is reissued, a codec pipeline is recreated,
a route/socket provider is replaced, or an owner-defined discontinuity occurs.
Sequence numbers, codec configuration, timestamp correlation, and health
baselines do not silently continue across generations.

Fleet keys live observations by at least:

```text
stream identity + provider generation + accepted authority revision
```

Reconnect to the same logical stream may be healthy, but it is visible as a
new generation. Stale evidence from an earlier generation cannot satisfy a
current session.

## Lifecycle

The canonical lifecycle is:

```text
discovered/proposed
  -> accepted
  -> available
  -> requested
  -> admitted
  -> starting
  -> active
  -> draining/stopping
  -> cleaned
```

Every transition can instead end in `rejected`, `expired`, `cancelled`,
`failed`, or `cleanup_failed`. `recovering` is an explicit substate with a
bounded deadline and attempt count; it is not perpetual activity.

These facts remain independent:

1. the manifest or discovery proposal exists;
2. the accepted subscription/session is current;
3. the transport is connected or a process is running;
4. payload bytes are arriving;
5. sample/frame sequence and source timestamps are advancing;
6. decode or schema validation succeeds;
7. the selected sink applies or renders current payloads;
8. cleanup completed.

Only the strongest observed stage may be shown. A running process, open socket,
or nonzero byte counter is not proof of a healthy sink.

## Time and correlation

Fleet never converts all time into one supposedly universal timestamp.
Descriptors name every domain, including:

- source monotonic/sample time;
- LSL local clock and measured clock-offset history;
- capture monotonic time;
- codec presentation timestamp;
- device wall time when supplied;
- Hub receive time;
- accepting-authority transition time;
- operator-host wall time.

Adapters preserve raw source timestamps and correlation evidence. They may
offer a derived display time, but the policy, uncertainty, generation, and
source chain remain inspectable.

LSL clock correction, media PTS mapping, network transit time, fixed capture
latency, and Hub receipt time are separate quantities. Timestamp smoothing or
dejittering is explicit and never overwrites raw evidence.

## Health and freshness

Health is a vector, not a single score:

- manifest/subscription/session validity;
- producer availability;
- transport/process state;
- payload progress;
- schema or decode validity;
- sink progress;
- latency and jitter;
- loss, drops, and queue pressure;
- recovery state;
- cleanup state.

The following conditions are distinct:

- **no data:** no payload has arrived within the declared expectation;
- **stalled:** bytes or callbacks stopped advancing;
- **frozen:** payload continues but decoded sample/frame identity or source
  progress does not meaningfully advance;
- **stale:** the last accepted observation exceeds its freshness policy;
- **degraded:** a useful subset remains, with the missing behavior named;
- **failed:** a bounded operation terminated unsuccessfully.

Event-driven streams declare their heartbeat or absence policy so silence is
not misclassified as a stall. For video, decoded-frame hashes or an equivalent
progress signal are used in bounded validation; byte flow alone is
insufficient.

## Buffering and backpressure

Every queue has explicit bounds in items, bytes, and/or time. Each stream
declares one policy:

- `block_or_throttle` when loss is unacceptable and the producer can safely
  slow;
- `drop_oldest` for newest-state previews and live pose/display surfaces;
- `drop_newest` when preserving an already admitted ordered prefix matters;
- `disconnect_slow_consumer` when a subscriber violates a bounded contract;
- `spill_bounded` only when retention, storage quota, encryption, and cleanup
  are explicitly approved.

Status and command work has reserved capacity and cannot be starved by media.
Decode, preview, recording, and relay fan-out use separate queues and failure
domains. A slow preview must not stop the receiver from observing current
frames. Keyframe/configuration-aware queues must retain enough restart context
to recover a decoder after drops.

No unbounded channel, retry loop, reconnect loop, process, recording, or
offline queue is accepted.

## Admission and resource budgets

Fleet admission evaluates current authority and measured/declared cost before
starting a subscription or media session. Budgets apply:

- per device;
- per adapter/provider;
- per route/network interface;
- per operator host;
- per tenant or relay;
- globally.

The budget vector includes at least bandwidth, sample/frame rate, decode
slots, CPU/GPU, memory, queue bytes, disk/retention, concurrent sessions, and
operator preview count. It also records priority and preemption policy.

Base status and control have protected capacity. Optional previews, recording,
and relay fan-out cannot consume that reserve. Admission uses deterministic
reasons, supports partial acceptance where safe, and creates an auditable
decision. Fair scheduling prevents one headset or high-rate stream from
monopolizing shared resources.

## Transport and capability negotiation

A semantic stream may offer multiple transports. Selection binds:

- exact manifest and provider generation;
- current authority and registry revision;
- source and sink capabilities;
- framing/schema/codec compatibility;
- route security and topology authorization;
- latency, reliability, and cost policy;
- fallback order and expiry.

Fallback is a new explicit decision, not an invisible transport swap.
Android `Network` observation, direct interface-bound socket proof, ADB
reachability, LSL resolve, and relay reachability remain different capability
classes.

## LSL adapter contract

Rusty LSL is used for timestamped samples and markers when its bounded,
explicitly activated compatibility surface fits the stream. Fleet requires:

- explicit resolve query and deterministic ambiguity handling;
- stable source identity when recovery is requested;
- bounded resolve, connect, pull, recovery, and cleanup;
- declared channel count, format, nominal rate, chunk policy, and buffer
  limits;
- raw sample timestamp plus clock-offset history;
- explicit online time-correction/dejitter policy;
- loss, backlog, recovery, and clock observations;
- a new provider generation after recovery or producer restart.

Resolve results are proposals, not enrollment or Manifold admission. A
newest-only consumer drains or bounds old samples rather than reading an
ever-growing backlog. High-rate small samples may use chunking, but the chosen
latency/overhead tradeoff is measured and operator-configurable within bounds.

Current Rusty LSL evidence is a narrow, release-candidate compatibility
surface, not a claim of arbitrary shape, ambient discovery, broad host/device
interop, or Manifold authority. Fleet fixtures must model both the proven
surface and truthful unsupported cases.

## Media and FFmpeg process-adapter contract

Media uses the owner graph:

```text
source -> processor -> route/socket -> codec -> sink -> cleanup
```

Each owner reports its own configuration and effective receipt. Fleet starts
receiver-first where the selected route requires it and shows the active
display/source identity rather than assuming display zero.

An FFmpeg integration is a bounded Hostess process adapter, not the media
authority. It must:

- use an argument list, explicit executable provenance, and an allowlisted
  pipeline template;
- probe input and capabilities with machine-readable `ffprobe` output;
- consume machine-readable `-progress` records at a declared interval;
- capture bounded stdout/stderr without parsing human progress text as the
  primary contract;
- set protocol-specific timeouts and bounded reconnect policy;
- expose input, demux, decode, filter, encode, mux/output, and terminal stages;
- distinguish process-running, bytes, packets, decoded frames, sink progress,
  freeze, and terminal status;
- terminate through a bounded graceful path, then a declared forced-cleanup
  path;
- redact endpoints and secrets from public/operator-default evidence;
- record the exact binary/version and selected pipeline digest.

`ffprobe` JSON is capability/input evidence, not playback proof.
Frame hashes such as `framemd5`, or an owner-equivalent decoded-frame signal,
are validation tools for progress/freeze detection. A tee/fan-out pipeline
must declare mapping, per-output queue/isolation, and failure policy; no output
silently inherits “ignore failure.”

The selected source may be an Android consent-based display capture, a
privileged direct display source, a raw ADB diagnostic source, or an explicit
compatibility adapter. Their permission, deployment, lifecycle, and support
claims remain separate.

## Observability

Metrics use stable, low-cardinality dimensions. Device IDs, stream IDs,
endpoints, error messages, and request IDs belong in logs/traces or bounded
detail records, not metric labels.

The common evidence vocabulary includes:

- manifests/subscriptions/sessions accepted, rejected, expired, and active;
- provider generation and reconnect/recovery counts;
- bytes, packets, samples, frames, and keyframes observed;
- packets/samples lost and frames dropped;
- source cadence and decoded/rendered frame rate;
- queue items, bytes, duration, high-water marks, and drop counts;
- jitter, clock offset/uncertainty, end-to-end latency, decode time, and sink
  delay where measurable;
- no-data, stall, freeze, degraded, and cleanup outcomes;
- CPU/GPU/memory/network/disk budget use.

Counters are monotonic within a generation. Durations use seconds and byte
metrics use bytes. Export event timestamps rather than a continuously updated
“age” metric. The UI computes age against its current clock.

WebRTC statistics vocabulary may inform names such as frames decoded, rendered,
and dropped, jitter-buffer delay, and freeze count. Fleet does not claim
WebRTC semantics for non-WebRTC transports.

## Privacy, recording, and retention

Observation is not permission to record. Every stream declares sensitivity,
operator visibility, recording eligibility, retention, export policy, and
redaction. MediaProjection and other user-visible capture retains platform
consent and revocation behavior. A relay cannot broaden recording authority.

Default previews are ephemeral. Recording, raw sample export, diagnostics, and
public evidence are separate grants. Cleanup removes temporary processes,
rules, services, files, and grants or reports exactly what remains.

## Operator projection

Fleet rows show only a compact readiness/attention summary. The selected-device
inspector provides:

- semantic source and current generation;
- accepted session/subscription and expiry;
- owner chain and active transport;
- source, receive, decode, and sink progress;
- clock domain/correction summary;
- current rate, latency, loss, drops, queue pressure, and budget use;
- no-data/stall/freeze/recovery/cleanup reasons;
- recording/retention/sensitivity;
- sanitized evidence and current operator actions.

No automatic all-device video wall is part of the product. Preview is selected,
bounded, independently stoppable, and never required for base status.

## Validation

Milestone fixtures cover:

- duplicate or ambiguous discovery;
- stale authority or provider generation;
- reordered, replayed, and discontinuous samples;
- clock step, drift, offset uncertainty, and missing correlation;
- source silence, transport stall, byte-only activity, decode failure, frozen
  frames, sink failure, and recovery;
- queue saturation under every selected drop policy;
- slow consumer and media load while control remains responsive;
- wrong display/source, format, channel count, codec, framing, and sink;
- resource-budget rejection, fairness, preemption, and cleanup;
- reconnect, route fallback, process crash, and partial fan-out failure;
- revoked consent, recording denial, redaction, and retention expiry.

Quick checks validate schemas and deterministic fixtures. Standard adds the
milestone scenario suite and Console/CLI/API parity. Device gates validate only
the exact promoted source/route/sink profile. Deep adds architecture,
performance, soak, security, cleanup, and cross-repository evidence.

The current inventory and evidence maturity are recorded in
[Morphospace Datastream Matrix](research/MORPHOSPACE_DATASTREAM_MATRIX.md).
The external basis for these rules is recorded in
[Datastream Reference Ledger](research/DATASTREAM_REFERENCE_LEDGER.md).
