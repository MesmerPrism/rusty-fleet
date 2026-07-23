# Datastream Reference and Provenance Ledger

## Method

These primary and official sources were inspected on 2026-07-23. The ledger
records the rule adopted for Rusty Fleet and the overreach explicitly rejected.
External documentation supplies design pressure, not Rusty Morphospace
authority. Version-sensitive implementation details must be rechecked when a
milestone selects a dependency.

## Lab Streaming Layer

| Reference | Rule adopted | Overreach rejected |
| --- | --- | --- |
| [LSL user guide](https://labstreaminglayer.readthedocs.io/info/user_guide.html) | model discovery, lazy inlet connection, and recoverable streams as distinct lifecycle stages | treating discovery as Fleet enrollment or Manifold admission |
| [LSL time synchronization](https://labstreaminglayer.readthedocs.io/info/time_synchronization.html) | preserve raw sample timestamps and offset history; make online correction/dejitter explicit | rewriting every sample into host wall time |
| [LSL FAQ](https://labstreaminglayer.readthedocs.io/info/faqs.html) | bound newest-only backlog; record known fixed latency; measure chunk latency/overhead; surface ambiguous resolution | hard-coding one chunk size or silently selecting an unordered resolve result |
| [liblsl stream inlet reference](https://labstreaminglayer.readthedocs.io/projects/liblsl/ref/inlet.html) | require stable source identity for recoverable behavior and bound recovery | claiming every discovered stream is recoverable |

## FFmpeg and media pipelines

| Reference | Rule adopted | Overreach rejected |
| --- | --- | --- |
| [FFmpeg tool documentation](https://ffmpeg.org/ffmpeg.html) | use machine-readable `-progress` and a declared statistics interval; record the exact execution configuration | parsing interactive human statistics as the durable process contract |
| [ffprobe documentation](https://ffmpeg.org/ffprobe.html) | use explicit JSON or another machine-readable writer for input/capability probing | treating a successful probe as decoded/rendered-frame proof |
| [FFmpeg formats](https://ffmpeg.org/ffmpeg-formats.html) | use frame hashes for bounded decoded-progress tests; declare tee maps, per-output isolation, and failure policy | assuming fan-out outputs share one reliability/latency policy |
| [FFmpeg protocols](https://ffmpeg.org/ffmpeg-protocols.html) | set protocol-specific timeouts, reconnect, and buffers; test termination responsiveness | a universal undocumented “low latency” flag set |
| [GStreamer queue](https://gstreamer.freedesktop.org/documentation/coreelements/queue.html) | express queue limits in buffers, bytes, and/or time and name the leaky direction | importing GStreamer as a product dependency or leaving queue semantics implicit |
| [scrcpy video guide](https://github.com/Genymobile/scrcpy/blob/master/doc/video.md) and [developer guide](https://github.com/Genymobile/scrcpy/blob/master/doc/develop.md) | make display selection, buffering, demux/decode/display/record separation, and independent channel lifecycles explicit | copying implementation source or making scrcpy the Rusty authority |

## Android capture, codec, and local discovery

| Reference | Rule adopted | Overreach rejected |
| --- | --- | --- |
| [Android MediaProjection guide](https://developer.android.com/media/grow/media-projection) | retain per-session consent, foreground-service obligations, revocation, and changing capture dimensions | normalizing a laboratory grant bypass into production |
| [Android audiovisual capture](https://developer.android.com/media/platform/av-capture) | handle user revocation and distinguish capture silence/black output from a healthy session | inferring healthy capture from a retained token |
| [Android MediaCodec](https://developer.android.com/reference/android/media/MediaCodec) | model codec lifecycle, configuration/keyframe data, output format change, and errors | treating an instantiated codec as sink progress |
| [Android NSD](https://developer.android.com/reference/android/net/nsd/NsdManager) | treat mDNS/NSD discovery as an optional, permission- and battery-aware proposal source | using NSD as durable Fleet identity |
| [Android Wi-Fi P2P](https://developer.android.com/develop/connectivity/wifi/wifip2p) | keep platform topology lifecycle and application socket evidence distinct | inferring a usable Rust-owned route from group existence alone |

## Observability and scale

| Reference | Rule adopted | Overreach rejected |
| --- | --- | --- |
| [W3C WebRTC statistics](https://www.w3.org/TR/webrtc-stats/) | adapt clear vocabulary for packets/loss/jitter, decoded/rendered/dropped frames, processing delay, and freezes | claiming WebRTC metric semantics for a non-WebRTC route |
| [Prometheus instrumentation practices](https://prometheus.io/docs/practices/instrumentation/) | keep metric labels low-cardinality, distinguish counters/gauges, and export event timestamps instead of continuously changing age | device, stream, endpoint, or error-message metric labels |
| [OpenTelemetry metric semantic conventions](https://opentelemetry.io/docs/specs/semconv/general/metrics/) | use hierarchical stable names, UCUM units, seconds for durations, and bytes for byte quantities | adopting a semantic convention before its meaning matches the owner |

## Future remote media candidates

| Reference | Useful pressure | Current decision |
| --- | --- | --- |
| [SRT project](https://github.com/Haivision/srt) | encryption, ARQ, bounded latency, and rendezvous for difficult networks | research candidate only; not the default relay |
| [WebRTC specification](https://www.w3.org/TR/webrtc/) | standardized real-time media/session behavior and a mature browser/operator ecosystem | research candidate only; do not create a second authority engine |
| [QUIC transport](https://www.rfc-editor.org/rfc/rfc9000.html) and [QUIC datagrams](https://www.rfc-editor.org/rfc/rfc9221.html) | reliable streams and unreliable datagrams can share authenticated, congestion-controlled transport | research pressure only; no custom QUIC media protocol is selected |

## Consolidated decisions

The references support:

- raw-time preservation plus explicit correlation;
- identity plus provider-generation lineage;
- bounded recovery and queueing;
- separate connection, byte, sample/frame, decode, sink, and cleanup evidence;
- selected and admitted media rather than ambient preview;
- machine-readable process adapters;
- low-cardinality fleet metrics;
- source-specific Android consent and display selection;
- measured transport selection rather than a protocol monoculture.

They do not establish:

- a universal transport or codec;
- a production relay;
- final chunk, queue, bitrate, latency, or fleet-size thresholds;
- arbitrary Rusty LSL interoperability;
- automatic recording authority;
- supported Quest source profiles before owner-repository promotion.

Those remain explicit milestone decisions backed by exact owner contracts and
measured evidence.
