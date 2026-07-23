# Rusty Fleet Low-Fidelity Wireframes

## Status

These wireframes are planning fixtures. They define information hierarchy and
state coverage, not final dimensions, typography, colors, or component-library
selection.

## Fleet overview

```text
┌ Rusty Fleet ────────────────────────────────────────────────────────────────┐
│ Fleet  Attention  Operations  Streams Enrollment Audit Settings  Hub Local│
├──────────────┬──────────────────────────────────────────────────────────────┤
│ Saved views  │ 1,284 devices | 1,201 fresh | 26 stale | 44 offline        │
│ All devices  │ 13 attention | 18 active | As of 14:32:05                 │
│ Attention    ├──────────────────────────────────────────────────────────────┤
│ Lab A        │ Search… [Filters 2] [Group] [Columns] [Density] [Save]     │
│ Rollout R1   │ Scope: [Enrollment=enrolled ×] [Model=Quest 3 ×]           │
│              ├──┬──┬──────────┬─────┬───────┬──────┬─────────┬────┬──────┤
│              │☐ │! │ Device   │ Age │ Route │ Power│ App     │Ctrl│Priv. │
│              ├──┼──┼──────────┼─────┼───────┼──────┼─────────┼────┼──────┤
│              │☐ │  │ Quest-01 │ 12s │ Local │ 86%  │ Kiosk   │Mon │USB   │
│              │☐ │△ │ Quest-02 │ 8m  │ Local │ 21%  │ Player  │App │No ADB│
│              │☐ │! │ Quest-03 │ 31m │Offlin.│ 64%  │ Stale   │—   │—     │
│              │  │  │ … virtualized rows …                               │
│              ├──┴──┴──────────┴─────┴───────┴──────┴─────────┴────┴──────┤
│              │ 1–54 visible | 1,284 matching | 23 order changes queued   │
└──────────────┴──────────────────────────────────────────────────────────────┘
```

## Selected-device inspector

```text
┌ Fleet grid ───────────────────────────────┬ Quest-002                 [×] ┐
│ selected row remains visible             │ Quest 3 | ID …7F2A             │
│                                          │ Last accepted 8m ago           │
│                                          ├─────────────────────────────────┤
│                                          │ ATTENTION                       │
│                                          │ Status evidence stale 8m        │
│                                          │ [Refresh] [View evidence]       │
│                                          ├─────────────────────────────────┤
│                                          │ OBSERVED                        │
│                                          │ Battery 21%, stale 8m           │
│                                          │ Player, app-reported foreground │
│                                          │ Kiosk active, evidence stale    │
│                                          ├─────────────────────────────────┤
│                                          │ CAPABILITIES                    │
│                                          │ Monitoring Ready                │
│                                          │ App control Refresh required    │
│                                          │ ADB Unsupported                 │
│                                          │ Media Route unavailable         │
│                                          ├─────────────────────────────────┤
│                                          │ CURRENT WORK                    │
│                                          │ op-7X4K Accepted, not dispatched│
└──────────────────────────────────────────┴─────────────────────────────────┘
```

## Batch preview

```text
Launch participating app: Training 4.2
Target snapshot: 120 devices at 14:28:02 | expires 14:38:02

Eligible 92 | Warning 8 | Excluded 15 | Refresh required 5

Device      Decision          Reason
Quest-001   Eligible          Capability current; local route
Quest-002   Refresh required  Capability evidence is 8m old
Quest-003   Excluded          Participating app unsupported
Quest-004   Warning           Relay route; additional latency expected
Quest-005   Excluded          Operator grant does not permit launch

Ring 1: 10 | pause 2m | then concurrency 20 | stop after 3 failures
[Export plan] [Cancel] [Confirm 100 eligible targets]
```

## Batch progress

```text
Operation op-7X4K | Running | started 14:31:12
Applied 67 | Running 14 | Accepted 8 | Queued 9
Excluded 15 | Failed 5 | Cleanup pending 2

Device      Stage             Evidence / reason              Action
Quest-001   Applied           Owner receipt r812             View
Quest-002   Running           Awaiting app receipt           Cancel request
Quest-003   Excluded          Unsupported                    —
Quest-004   Failed            Relay lost after dispatch      Retry
Quest-005   Cleanup pending   Session release unconfirmed    Cleanup

[Pause remaining rings] [Request cancellation] [Export] [Retry eligible 3]
```

## Alerts grouped by cause

```text
ATTENTION | 13 unresolved | 4 acknowledged

CRITICAL
Relay adapter authentication failure | 44 affected | 6m
Impact: remote route unavailable; 39 devices still reachable locally
[Acknowledge] [Open affected devices] [View adapter evidence]

WARNING
Low battery below operating threshold | 7 affected | oldest 21m
[Open filtered view] [Suppress during charging window]

Command cleanup pending | 2 affected | oldest 4m
[Open operations]
```

## Stream and media sessions

```text
Available 24 | Active 3 | Requested 1 | Degraded 1 | Cleanup pending 1

Device/stream   Kind     Path  Route   Payload stage     Sink      Age
Quest-04/head   Pose     p12   LSL     Samples advance  Detail    18ms
Quest-18/s92    Display   p7   Relay   Frames advance   Preview   2.4s stale
Quest-44/s93    Camera    p3   Failed  No frames        Waiting   —

Selected preview: Quest-18/s92
Route delayed; control plane normal
Epochs: source 4 | route 7 | processing 2 | sink 9
[Stop] [Change route] [Open timing] [Open lineage] [Open evidence]
```

## Ambiguous scientific source

```text
Source selection: Ambiguous
Rule: type = EEG AND source_id = amplifier-07
Expected: exactly 1 | Found: 3

Candidate                         Native UID   Created    Metadata
EEG / amplifier-07 / Quest-02    …7f          12:01:04   64 ch | 1 kHz
EEG / amplifier-07 / Quest-02    …9a          12:14:32   64 ch | 1 kHz
EEG / amplifier-07 / unknown     …c1          12:14:40   32 ch | 500 Hz

[Refresh] [Pin candidate…] [Edit saved rule]
```

## Scientific recording preflight

```text
Run: Balance-VR | protocol 3.2
Required streams: 3 | Ready: 2 | Missing: 1
Optional streams: 4 of 5 ready

Missing required
Experiment markers | no candidate found

Timing
EEG ±0.4 ms | Eye tracking ±1.8 ms
Head pose calibration expired

Storage and policy
Estimated 42 GB | Available 181 GB
Retention 90 days | Encrypted storage verified

[Cancel] [Resolve missing stream] [Record with documented deviation…]
```

## Empty and degraded fixtures

```text
OFFLINE
Quest-077 | Offline 33m
Last accepted values retained and marked stale
[View history] Commands disabled: no current route

EMPTY FLEET
No enrolled devices
[Begin enrollment] [Open enrollment guide]

PERMISSION LIMITED
Privileged access: Restricted
Monitoring remains available; ADB/File actions are hidden or disabled by role.
[View required role]
```

## Required screenshot families

Milestone screenshots cover:

- light, dark, and supported high-contrast modes;
- compact, standard, and comfortable density;
- supported scaling and large-text settings;
- wide, medium, and minimum window width;
- inspector closed, open, and pinned;
- loading, empty, no-match, cached, stale, offline, and adapter failure;
- unsupported, disabled, unauthorized, unavailable, degraded, and unknown;
- batch preview, changed target, partial result, cancellation, and cleanup
  failure;
- media ready, no route, no frame, stale frame, and cleanup pending;
- long names, localization expansion, and permission-limited content.
