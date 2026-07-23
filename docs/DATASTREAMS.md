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

Fleet projections use two linked, versioned representations derived from owner
contracts:

1. a transport-neutral semantic descriptor used for Fleet queries, admission,
   status, budgets, policy, and Console/CLI/API parity;
2. the complete native descriptor snapshot, or a role-controlled reference to
   it, used for interoperability, scientific interpretation, troubleshooting,
   replay, and reproducibility.

The native representation includes its kind/schema, adapter and native
protocol versions, capture time, sensitivity, and digest. Recognized fields
are normalized without discarding unknown native fields. Normalization alone
is insufficient evidence for a scientific or media source, while a
native-only descriptor is insufficient for fleet-wide policy.

The semantic descriptor must carry enough information to compare streams
without replacing the native manifest:

- stable `stream_id`, semantic family, sample/frame schema, and sensitivity;
- source device, source module/application, and source role;
- payload plane, rate class, delivery model, and retention class;
- source/acquisition, serializer or encoder, framing/packetization,
  route/socket, depacketizer/demux, validator or decoder, sink, and cleanup
  identities where applicable;
- source, route/connection, processing, and sink epochs plus a composite path
  generation and owner authority revision;
- offered transports and the currently accepted transport;
- timestamp domains, correlation observations and uncertainty, transformation
  lineage, reset state, and known fixed-latency calibration;
- expected cadence or explicit irregular/event-driven declaration;
- per-edge buffering and overflow policies;
- current subscription/session reference and expiry;
- lifecycle, health, and terminal cleanup evidence;
- operator-visible cost estimates and current measured use;
- visibility/export/recording policy.

Unknown fields remain round-trippable where the owner contract and sensitivity
policy permit it. Unknown required semantics fail closed.

## Source selection and ambiguity

Discovery is not selection. A resolve operation preserves:

- query language and exact query;
- expected cardinality;
- complete candidate set or its role-controlled artifact and digest;
- candidate native descriptor digests and concrete native instance identities;
- selected candidate, method, actor/policy, validity interval, and expiry;
- deterministic tie-break fields when a policy tie-break is allowed;
- manual override, reason, and audit lineage.

Selection methods include saved rule, exact owner manifest, single candidate,
manual pin, and an explicitly ordered policy tie-break. `No candidate` and
`ambiguous` are first-class outcomes. Selecting the first returned result is
prohibited because discovery result order is not an identity or policy.

A manual pin does not silently rewrite the saved rule. Candidate disappearance,
metadata change, identity change, or snapshot expiry forces re-evaluation.
LSL discovery, mDNS, BLE, ADB, and relay catalogs remain proposal sources and
never become Fleet enrollment or Manifold admission.

## Identity and component epochs

A stable stream identity describes the logical stream. Current evidence also
names owner-scoped component epochs:

- `source_epoch`: one concrete producer/acquisition epoch;
- `route_epoch`: one connection, route, or socket-provider epoch;
- `processing_epoch`: one serializer, packetizer, demux, validator, encoder,
  or decoder configuration epoch;
- `sink_epoch`: one preview, recorder, relay output, analysis, or application
  consumer epoch.

Fleet keys live observations by at least:

```text
stream identity + relevant component epochs + accepted authority revision
```

Each epoch records its predecessor, transition reason, continuity assessment,
native evidence, and first/last sequence or timestamp where applicable. A
composite `path_generation` may provide a concise projection, but it does not
erase the component epochs.

A reconnect to the same live LSL outlet may change only the route epoch. An
outlet restart with a stable logical recovery identity changes the source
epoch. Decoder recreation changes processing; presenter replacement changes
the sink. Capture reauthorization or an owner-declared acquisition
discontinuity changes the source. Evidence, counters, codec configuration,
clock baselines, and health do not cross an incompatible epoch. Stale evidence
from any superseded component cannot satisfy a current session.

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

Each stream profile marks every progress stage `required`, `optional`, or
`not_applicable` and declares its startup, no-data, stall, and cleanup
deadlines. The common vocabulary does not force a marker stream, recorder,
relay-only output, and rendered video preview through the same healthy path.

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
offer a derived display time, but the policy, uncertainty, applicable
component epochs, and source chain remain inspectable.

LSL clock correction, media PTS mapping, network transit time, fixed capture
latency, and Hub receipt time are separate quantities. Timestamp smoothing or
dejittering is explicit and never overwrites raw evidence.

A correlation record includes:

- source and target domain IDs, units, and exact time base;
- raw timestamp preservation;
- offset observations or an artifact reference;
- uncertainty and its interpretation;
- estimator, processing flags, and calibration method;
- valid-from/valid-to interval and affected component epochs;
- clock reset, step, and discontinuity events;
- fixed-latency calibration value, uncertainty, owner, date, and exact
  hardware/firmware/software lineage;
- the transformation ID used for any derived display time.

Unknown fixed latency is not represented as zero. A clock reset closes the
current correlation segment rather than retroactively rewriting it.

## Cadence, absence, and completeness

Every stream declares one cadence mode: `regular`, `irregular`, or
`event_driven`. Where applicable it also declares nominal and accepted rate
ranges, measurement window, gap tolerance, heartbeat policy, no-data deadline,
and sequence semantics.

Advertised or requested cadence is not observed cadence. Fleet projects them
separately and uses measured cost for admission. An irregular marker stream can
be healthy while silent; silence becomes failure only when a required event
window or heartbeat says so. Completeness evidence names gaps, duplicates,
reordering, discarded backlog, and loss under the current source epoch.

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
- **frozen:** payload continues but a profile that expects changing content
  fails its declared semantic-progress test;
- **stale:** the last accepted observation exceeds its freshness policy;
- **degraded:** a useful subset remains, with the missing behavior named;
- **failed:** a bounded operation terminated unsuccessfully.

Event-driven streams declare their heartbeat or absence policy so silence is
not misclassified as a stall. For video, decoded-frame hashes or an equivalent
progress signal are used in bounded validation; byte flow alone is
insufficient.

Static calibration media and deliberately silent audio or marker profiles
either use a suitable semantic-progress policy or declare content-change
detection not applicable.

## Buffering and backpressure

Every producer-to-consumer graph edge has explicit bounds in items, bytes,
and/or time and declares one policy:

- `block_or_throttle` when loss is unacceptable and the producer can safely
  slow;
- `drop_oldest` for newest-state previews and live pose/display surfaces;
- `drop_newest` when preserving an already admitted ordered prefix matters;
- `disconnect_slow_consumer` when a subscriber violates a bounded contract;
- `spill_bounded` only when retention, storage quota, encryption, and cleanup
  are explicitly approved.

Each edge also declares whether its producer may block, which
keyframe/configuration state must be retained, how a slow consumer is isolated,
and what evidence is emitted on overflow. A stream class may supply defaults;
the preview, recorder, relay, and analysis branches remain separately owned.

Status and command work has reserved capacity and cannot be starved by media.
Decode, preview, recording, analysis, and every relay output use separate
queues and failure domains. A slow preview must not stop a recorder or prevent
the receiver from observing current frames. Keyframe/configuration-aware queues
must retain enough restart context to recover a decoder after drops.

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
slots, CPU/GPU, memory, per-edge queue bytes and dwell, durable recorder
throughput, disk/retention headroom, clock-quality requirements, concurrent
sessions, expected recovery cost, per-output fan-out cost, and operator preview
count. It also records priority and preemption policy.

Base status and control have protected capacity. Optional previews, recording,
and relay fan-out cannot consume that reserve. Admission uses deterministic
reasons, supports partial acceptance where safe, and creates an auditable
decision. Fair scheduling prevents one headset or high-rate stream from
monopolizing shared resources.

## Transport and capability negotiation

A semantic stream may offer multiple transports. Selection binds:

- exact manifest, source epoch, and accepted route epoch;
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
- complete native `StreamInfo`/XML snapshot or protected reference, native
  descriptor digest, selected native `uid`, and exact adapter/liblsl version;
- stable source identity when recovery is requested;
- bounded resolve, connect, pull, recovery, and cleanup;
- declared channel count, format, nominal rate, chunk policy, and buffer
  limits;
- raw sample timestamp plus clock-offset observations and uncertainty;
- explicit online time-correction/dejitter policy;
- nominal versus measured cadence, loss, backlog, recovery, and clock
  observations;
- distinct route recovery and source-restart epochs;
- LabRecorder/XDF record-and-replay compatibility fixtures before scientific
  support is claimed.

Resolve results are proposals, not enrollment or Manifold admission. A
newest-only consumer drains or bounds old samples rather than reading an
ever-growing backlog. High-rate small samples may use chunking, but the chosen
latency/overhead tradeoff is measured and operator-configurable within bounds.
`source_id` is an owner-provided recovery key beneath enrolled identity, not
authentication or global Fleet identity.

Current Rusty LSL evidence is a narrow, release-candidate compatibility
surface, not a claim of arbitrary shape, ambient discovery, broad host/device
interop, or Manifold authority. Fleet fixtures must model both the proven
surface and truthful unsupported cases.

## Media and FFmpeg process-adapter contract

Media uses the owner graph:

```text
source
  -> acquisition/source processing
  -> encoder or serializer
  -> framing/packetization
  -> route/transport
  -> depacketization/demux
  -> decoder or schema validator
  -> preview, recorder, relay, analysis, or application sink
  -> cleanup
```

Stages that do not apply remain explicitly absent. Each owner reports its own
configuration, component epoch, and effective receipt. Fleet starts
receiver-first where the selected route requires it and shows the active
display/source identity rather than assuming display zero.

The first FFmpeg integration is the CLI under bounded Hostess supervision, not
direct `libav*` and not media authority. It must:

- use an argument list, explicit executable provenance, and an allowlisted
  pipeline template;
- restrict protocols, demuxers, codecs, filters, dimensions, rates, stream
  counts, probe size, threads, memory, disk, and process count to the accepted
  profile;
- probe input and capabilities with machine-readable `ffprobe` output;
- consume machine-readable `-progress` records at a declared interval;
- capture bounded stdout/stderr without parsing human progress text as the
  primary contract;
- set protocol-specific timeouts and bounded reconnect policy;
- expose configured, probed, spawned, input-open, packet/access-unit,
  decoder/schema, sink, recovering, draining, stopped, and cleaned states with
  profile-specific deadlines;
- distinguish process-running, bytes, packets, decoded frames, sink progress,
  freeze, and terminal status;
- own and terminate the complete process tree through a bounded graceful path,
  then a declared forced-cleanup path;
- close pipes and listeners and account for temporary files, routes, and
  grants;
- redact endpoints and secrets from public/operator-default evidence;
- record the exact binary/version and selected pipeline digest.

`ffprobe` JSON is capability/input evidence, not playback proof. FFmpeg
`-progress` is process evidence, not the whole health contract; Hostess adds
first-packet, first-decode, first-sink, ongoing-progress, stop, and cleanup
watchdogs.
Frame hashes such as `framemd5`, or an owner-equivalent decoded-frame signal,
are validation tools for progress/freeze detection. A tee/fan-out pipeline
must declare mapping, per-output queue/isolation, and failure policy; no output
silently inherits “ignore failure.”

Direct `libav*` integration requires a measured inability to meet an accepted
profile through the process adapter—for example, a material zero-copy,
frame-callback, dynamic-graph, startup, or latency requirement. Promotion then
requires exact ABI/version ownership, memory/thread lifecycle, security and
fuzzing policy, upgrade/rollback, and equivalent process/resource cleanup
evidence.

The selected source may be an Android consent-based display capture, a
privileged direct display source, a raw ADB diagnostic source, or an explicit
compatibility adapter. Their permission, deployment, lifecycle, and support
claims remain separate.

## Observability

Metrics use stable, low-cardinality dimensions. Device IDs, stream IDs,
endpoints, error messages, and request IDs belong in logs/traces or bounded
detail records, not metric labels.

Permitted metric dimensions are a reviewed bounded set such as payload plane,
stream class, adapter type, required stage, condition, outcome, route type,
codec family, queue role, error class, and priority class. Arbitrary
application/source metadata, participant/session IDs, paths, and native
descriptor fields are prohibited as labels.

The common evidence vocabulary includes:

- manifests/subscriptions/sessions accepted, rejected, expired, and active;
- component/path generations and reconnect/recovery counts;
- bytes, packets, samples, frames, and keyframes observed;
- packets/samples lost and frames dropped;
- source cadence and decoded/rendered frame rate;
- queue items, bytes, duration, high-water marks, and drop counts;
- jitter, clock offset/uncertainty, end-to-end latency, decode time, and sink
  delay where measurable;
- no-data, stall, freeze, degraded, and cleanup outcomes;
- CPU/GPU/memory/network/disk budget use.

Counters are monotonic within their declared component epoch. Durations use
seconds and byte metrics use bytes. Export event timestamps rather than a
continuously updated
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

## Scientific session, recording, and replay

Scientific recording is a separate product contract, not a side effect of an
active stream. A run manifest includes:

- pseudonymous run/participant references under role policy;
- study/protocol ID and version;
- required and optional stream-selection rules;
- marker vocabulary/schema;
- exact selected candidate and native descriptor snapshots/digests;
- accepted authority and component epochs;
- timing domains, offset history, uncertainty, transformations, and
  calibrations;
- connector, adapter, library, firmware, and application versions;
- approved deviations and the actor/authority that accepted them;
- recording, retention, encryption-at-rest, export, and deletion policy.

Each artifact independently reports starting, writing, stalled, finalizing,
complete, failed, and cleaned states; bytes and last durable progress; expected
and actual streams; sample/event ranges and gaps; metadata and timing-history
presence; checksum; retention deadline; and cleanup/deletion receipt.

LabRecorder/XDF is the initial native scientific compatibility workflow.
Fleet preserves the raw artifact rather than inventing a replacement format,
adds a Fleet run/recording receipt, and validates import or replay where the
selected profile requires it. Preview health does not prove recording health,
and recording may be healthy without a preview.

## Operator projection

Fleet rows show only a compact readiness/attention summary. The selected-device
inspector provides:

- semantic source, selection rule/candidates, and native descriptor digest;
- source, route, processing, and sink epoch lineage;
- accepted session/subscription and expiry;
- owner chain and active transport;
- source, receive, decode, and sink progress;
- raw clock domains, correlation uncertainty, transformation, and calibration;
- nominal/accepted/measured cadence, absence policy, latency, loss, drops,
  per-edge queue pressure, and budget use;
- no-data/stall/freeze/recovery/cleanup reasons;
- run, recording artifact, replay, retention, and sensitivity;
- sanitized evidence and current operator actions.

No automatic all-device video wall is part of the product. Preview is selected,
bounded, independently stoppable, and never required for base status.

## Validation

Milestone fixtures cover:

- duplicate or ambiguous discovery;
- native-descriptor round trip and damaged native metadata;
- candidate-set/cardinality changes and prohibited first-result selection;
- stale authority or source/route/processing/sink epoch;
- reordered, replayed, and discontinuous samples;
- clock step, drift, reset, transform lineage, fixed-latency calibration,
  offset uncertainty, and missing correlation;
- nominal/observed cadence, marker silence, heartbeat loss, and sample gaps;
- source silence, transport stall, byte-only activity, decode failure, frozen
  frames, legitimate static content, sink failure, and recovery;
- per-edge queue saturation under every selected drop policy;
- slow consumer and media load while control remains responsive;
- wrong display/source, format, channel count, codec, framing, and sink;
- resource-budget rejection, fairness, preemption, and cleanup;
- reconnect, route fallback, process crash, and partial fan-out failure;
- revoked consent, recording denial, XDF record/replay, artifact damage,
  redaction, retention expiry, process-tree residue, and cleanup.

Quick checks validate schemas and deterministic fixtures. Standard adds the
milestone scenario suite and Console/CLI/API parity. Device gates validate only
the exact promoted source/route/sink profile. Deep adds architecture,
performance, soak, security, cleanup, and cross-repository evidence.

The current inventory and evidence maturity are recorded in
[Morphospace Datastream Matrix](research/MORPHOSPACE_DATASTREAM_MATRIX.md).
The external basis for these rules is recorded in
[Datastream Reference Ledger](research/DATASTREAM_REFERENCE_LEDGER.md).
The reviewed delta from the 2026-07-23 external research handoffs is recorded
in [Fleet Research Integration Review](research/FLEET_RESEARCH_INTEGRATION_REVIEW.md).
