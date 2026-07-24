# Operator UI Architecture

## Decision

Rusty Fleet uses a dense, virtualized device table as its primary workspace,
paired with a persistent, resizable selected-device inspector. Card grids,
topology diagrams, alert boards, and media multiviews are specialized
secondary views. They do not replace the fleet inventory.

The Console projects the same canonical queries, target snapshots, status
conditions, operation lifecycles, and evidence exposed by `fleetctl` and the
local API. WPF controls and theme libraries are presentation adapters; they do
not own fleet state or command policy.

The evidence and product comparisons behind this decision are recorded in:

- [the product-pattern matrix](research/FLEET_UI_PRODUCT_MATRIX.md);
- [the reference and provenance ledger](research/FLEET_UI_SOURCE_LEDGER.md);
- [the low-fidelity wireframes](research/FLEET_UI_WIREFRAMES.md);
- [the cross-report integration review](research/FLEET_RESEARCH_INTEGRATION_REVIEW.md).

## Scope

This guide owns:

- operator information architecture;
- fleet-table, inspector, full-detail, alert, operation, and media projections;
- query, saved-view, selection, and navigation-restoration behavior;
- a cross-family status grammar;
- multi-device preview, confirmation, progress, retry, cancellation, and
  cleanup presentation;
- keyboard, UI Automation, contrast, scaling, and assistive-technology
  acceptance;
- WPF virtualization, update stability, and candidate scale budgets.

It does not change the authority boundaries in
[Architecture](ARCHITECTURE.md). In particular, the UI never turns an
observation into accepted state, a transport acknowledgement into an applied
effect, or an optional adapter into a base requirement.

## Enforceable design rules

1. **Fleet context persists.** Opening a device does not discard the active
   query, filters, grouping, sort, batch selection, scroll anchor, or inspector
   state.
2. **State is a vector, not a traffic light.** Enrollment, freshness, power,
   application, route, authorization, privilege, media, work, and alerts remain
   independent.
3. **Every mutable fact has provenance and age.** Source time, receive time,
   accepted revision, reporting source, authority, freshness deadline, and
   sensitivity are inspectable where applicable.
4. **Lifecycle terms are exact.** Observed, accepted, proposed, dispatched,
   applied, rejected, expired, cancelled, cleanup-pending, and cleaned are
   never synonyms.
5. **Optional capability loss is local.** Loss of ADB, File Manager, media,
   relay, or privilege removes only the affected controls and projections.
6. **Unsupported, disabled, unauthorized, disconnected, unavailable, stale,
   unknown, degraded, busy, and failed are different states.**
7. **Active scope is visible.** Search, filters, grouping, sort, saved-view
   name, result count, data age, and selection scope do not hide in an
   unmarked menu.
8. **Live data does not destabilize interaction.** A focused, selected,
   hovered, context-menu-bound, or confirmation-bound row does not move
   underneath the operator.
9. **Batch targets are previewable.** Risk-bearing work uses a frozen target
   snapshot by default and shows exclusions and changed-since-preview facts.
10. **Aggregate success cannot erase target failure.** Every device retains
    its own decision, stage, reason, receipt, cleanup state, and retry
    eligibility.
11. **Controls tell the truth before invocation.** Availability reflects
    support, enablement, authorization, reachability, identity, freshness,
    policy, conflict, and owner readiness.
12. **Compact does not mean cryptic.** Every indicator has column context, a
    visible value or short label, a programmatic name, and supplementary detail.
13. **Color is supplementary.** Shape, text, value, and programmatic state
    preserve meaning in grayscale and Windows high contrast.
14. **Routine work stays nonmodal.** Dialogs are limited to blocking input and
    risk-proportional confirmation.
15. **UI, CLI, and API share one contract.** UI-only query semantics, hidden
    action state, or presentation-owned policy are rejected.
16. **Empty and degraded states are designed.** Loading, no enrollment, no
    matches, cached/disconnected, stale, permission-limited, adapter-failed,
    partial-result, and cleanup-failed projections have fixtures.
17. **Scale is an acceptance condition.** A four-device happy path is useful
    for visual review but insufficient for UI acceptance.
18. **Datastreams are separately bounded.** Monitoring and command interaction
    remain responsive when an optional observation or media stream is absent,
    overloaded, reconnecting, frozen, or stopped.

## Information architecture

The durable top-level destinations are:

```text
Fleet
  All devices
  Saved views
  Comparison
Attention
  Alerts and exceptions
  Stale and offline
  Failed or blocked work
Operations
  Commands and deployments
  Scheduled operations
  Completed operations
Streams
  Catalog and subscriptions
  Media sessions
  Sources, routes, and budgets
  Cleanup failures
Enrollment
  Enrolled devices
  Candidates and pending enrollment
  Revoked and decommissioned
Audit
  Operator actions
  Device and capability history
  Exports
Settings
  Adapters and capability policy
  Operators and roles
  Saved-view administration
  Alert policy and maintenance windows
  Local, relay, and retention configuration
```

A left navigation surface is the default because the product has more than
five stable top-level destinations. The Fleet page does not add a second
duplicate navigation bar.

The page hierarchy is:

| Layer | Purpose | Contents |
| --- | --- | --- |
| Fleet summary | Orient without replacing inventory | total, freshness, offline, attention, active-work, data-as-of, and adapter state |
| Device collection | Primary workspace | query controls, filters, grouping, virtualized rows, and batch selection |
| Compact device row | Scan independent dimensions | identity, age, route, power, app, control, privilege, streams, work, and alerts |
| Selected-device inspector | Diagnose while retaining context | attention, observations, capabilities, current work, and quick actions |
| Full device detail | Deep and longitudinal diagnosis | status, capabilities, commands, streams/media, history, audit, and enrollment |
| Operation ledger | Track work across devices | aggregate counts plus per-target stages, evidence, retry, cancel, and cleanup |
| Alerts and exceptions | Prioritize intervention | grouped causes, affected targets, acknowledgement, suppression, and maintenance |
| Stream operations | Diagnose selected paths | catalog, subscription/session, source, generation, timing, route, codec/schema, sink, health, budget, and bounded preview |

## Fleet workspace

The wide-window composition is:

```text
navigation | summary + query + virtualized grid | selected-device inspector
```

At narrower widths the inspector becomes a stacked detail route. Returning to
the fleet restores the exact view, scroll anchor, focused device, selected
device, and inspector tab when those objects still exist and remain authorized.

### Summary strip

Use one compact, timestamped strip rather than a set of oversized cards:

```text
1,284 devices | 1,201 fresh | 26 stale | 44 offline | 13 attention | 18 active
As of 14:32:05 | Hub connected | 2 adapters degraded
```

Each count applies a documented filter and has a programmatic name that states
both count and effect. An optional capability being absent is not automatically
an alert; it becomes attention-worthy only when policy or pending work requires
it.

### Default columns

| Order | Column | Content |
| ---: | --- | --- |
| 1 | Batch selection | checkbox and hidden-selected indication |
| 2 | Attention | highest actionable severity and count, never overall health |
| 3 | Device | display name, hardware class, stable short ID, enrollment exception |
| 4 | Age | relative age of the latest accepted check-in and stale modifier |
| 5 | Route | local, relay, reconnecting, or offline; authorization is separate |
| 6 | Power | battery percentage, charging, low-power, and thermal modifiers |
| 7 | App / Kiosk | participating app, lifecycle, Kiosk state, and observation authority |
| 8 | Control | base monitoring and participating-app control readiness |
| 9 | Privileged | USB ADB, Wi-Fi ADB, File Manager, or privileged-adapter state |
| 10 | Streams | available/selected sample or media streams, strongest progress stage, and attention |
| 11 | Work | active count and most recent exceptional result |
| 12 | Tags / cohort | location, cohort, rollout ring, or operator tag |

Exact widths and row heights are design hypotheses. Test compact, standard,
and comfortable densities at all supported Windows scaling and text-size
settings before fixing them as product defaults.

Raw endpoints, complete identifiers, full timestamp chains, conflicting
observations, command bodies, file paths, media receipt chains, and audit
payloads belong in the inspector or full detail under the applicable role and
privacy policy.

### Query and saved-view contract

Fleet Hub owns the canonical query. The UI may provide type-ahead over a loaded
window, but acceptance-critical membership, counts, exports, and target
snapshots use the Hub result.

The query model supports:

- free text and exact identifier lists;
- explicit field, operator, and value filters;
- OR within one facet and AND between facets by default;
- explicit negation;
- stable sort keys and grouping;
- result revision, `as_of`, total count, elapsed time, and window information;
- cancellation so an older result cannot replace a newer query.

Filter chips show the complete expression, such as `Freshness = stale`, rather
than an ambiguous label. The active query remains visible.

A versioned saved view stores:

- canonical query and facet filters;
- sort keys and directions;
- grouping;
- column order, width, and visibility;
- density;
- inspector open and pinned state;
- time-display preference;
- owner and sharing scope;
- schema version.

Deep links may add selected device, inspector tab, scroll anchor, and grouping
state. Restoring a view resolves every retained identifier against current
permissions, identity revisions, and retention.

### Stable ordering

The default sort is display name followed by stable ID. Values may refresh in
place, but order- or group-affecting changes accumulate behind an explicit
control such as:

```text
23 live changes affect the current order. Apply changes
```

An opt-in live-order mode may be evaluated later. Batch preflight always uses
current Hub facts even while the visible table order is frozen.

### Selection

Inspector selection and batch selection are distinct.

- Row activation selects the device for inspection.
- The checkbox and keyboard selection commands change batch membership.
- Selection is keyed by stable device ID plus identity revision.
- Selection survives virtualization, sorting, grouping, and refresh.
- Filtering does not silently clear hidden selections.
- The selection bar reports selected, matching, and hidden-selected counts,
  selector kind, and snapshot state.
- `Select all matching` is a separate explicit action from selecting the
  currently loaded or visible rows.
- Re-enrollment, revocation, permission loss, or identity revision invalidates
  the old actionable target and is announced.
- Destructive actions never inherit an old broad selection without a new
  preview.

## Selected-device experience

The inspector remains alongside the table when width permits and may be pinned.
It contains:

- identity, enrollment, hardware class, and last accepted check-in;
- actionable attention conditions and their root cause when known;
- power, lifecycle, foreground, app, Kiosk, and route observations;
- base, app-control, privileged, File Manager, and media readiness;
- queued through cleanup-pending current work;
- recent meaningful transitions rather than every telemetry sample.

Inspector tabs are:

| Tab | Contents |
| --- | --- |
| Overview | attention, observations, capability summary, and current work |
| Status | every status family, source, timestamps, revision, conflict, and freshness |
| Capabilities | support, enablement, authorization, reachability, freshness, and evidence |
| Commands | active and recent command lifecycle and receipts |
| Streams | catalog, subscription/session, generation, timing, health, cost, and selected sample/media view |
| History | significant state transitions and audit links |

Full device detail adds enrollment identity and rotation, complete adapter
evidence, longer command/media history, audit lineage, exports, and the
QuestIonAble File Manager deep link when authorized.

Before a scientific recording begins, the Streams surface presents a
recording preflight with required/optional stream readiness, ambiguity and
missing-stream reasons, timing/calibration quality, storage estimate and
headroom, retention and encryption-at-rest policy, and any deviation requiring
explicit authority. The final recording result remains independent from
preview health.

### Selected-stream detail

The Streams tab follows
[Datastream Management](DATASTREAMS.md). It reveals details in layers:

1. available and selected semantic streams, sensitivity, and cost;
2. saved source-selection rule, expected/found cardinality, candidates,
   selection method, complete native descriptor access, and digest;
3. current lifecycle, accepted subscription/session, expiry, source, route,
   processing, and sink epoch lineage;
4. acquisition/serializer/encoder/framing/route/demux/validator/decoder/sink/
   cleanup owner chain;
5. raw source clocks, correlation observations, uncertainty, transformations,
   calibration, receive time, and freshness;
6. transport, byte, sample/frame, decode/schema, sink, recording, and cleanup
   progress, with required/optional/not-applicable stages visible;
7. nominal, accepted, and measured cadence; silence/heartbeat policy; latency,
   jitter, loss, drops, per-edge queue pressure, and budget use;
8. ambiguity, missing configuration/keyframe, byte-only activity, no-data,
   stall, freeze, recovery, fallback, and terminal reasons;
9. scientific run, recording artifact, replay, retention/export policy, and
   sanitized evidence.

The row never decodes media and never attempts to summarize this chain as one
green or red dot. A selected preview is bounded and independently stoppable.
The same catalog, health, budget, and terminal fields are available through
CLI and local API.

## Status-condition contract

Milestone 0 defines a versioned canonical condition with at least:

- `family`;
- `state`;
- machine-readable `reason`;
- bounded human-readable `message`;
- source time and Hub receive time;
- accepted revision and transition time;
- reporting source and accepting authority;
- freshness deadline;
- sensitivity or visibility class where needed.

Families remain independent. An aggregate attention projection may rank
actionable conditions but must not become a health score.

### Cross-family visual grammar

| State | Visible treatment | Meaning |
| --- | --- | --- |
| current | value or `Ready`; quiet family icon | valid current fact, no attention implied |
| applied / cleaned | check plus explicit label | recent owner-confirmed terminal success |
| in progress | progress or circular-arrow shape plus label | work or recovery is active |
| busy | clock/queue shape plus label | temporary conflict prevents new work |
| stale | clock modifier, age, retained value | older than family freshness policy |
| unknown | question shape plus label | no valid current fact |
| unsupported | barred family shape plus label | capability is not implemented |
| disabled | pause/minus shape plus label | supported but intentionally off |
| unauthorized / restricted | lock or shield plus label | current role or grant does not permit it |
| disconnected | broken-link/connector shape plus label | route is not connected |
| unavailable | unavailable family shape plus reason | expected capability cannot currently be supplied |
| degraded | warning shape plus remaining behavior | useful subset remains available |
| failed | error shape plus reason | operation or component ended unsuccessfully |
| critical | high-priority alert shape plus text | immediate intervention required |
| ambiguous | branching/candidate shape plus count | selection cannot proceed without a deterministic choice |
| awaiting configuration / keyframe | media-stage shape plus label | route has payload but decode cannot safely begin |
| byte-only | transport shape plus label | bytes advance without complete semantic payload |
| decoded, not rendered | split stage label | decoder advances while the selected sink does not |
| recording stalled | recorder shape plus elapsed time | durable artifact progress stopped |
| clock degraded | clock shape plus uncertainty | correlation exists but misses the selected profile |

Normal facts do not flood the table with green. Success color is reserved for
explicit recent success; warning and error colors are paired with shapes,
labels, and accessible names.

Tooltips supplement visible and programmatic meaning. They are available on
keyboard focus, contain no controls, and are never the only explanation.

## Multi-device actions

### Canonical flow

```text
select or query
  -> preview target set
  -> per-target preflight
  -> risk-proportional confirmation
  -> accepted operation
  -> bounded dispatch
  -> per-target application
  -> retry/cancel where valid
  -> terminal cleanup
  -> audit and export
```

### Target snapshot

The default target set is immutable after preview. Its contract includes:

- target-snapshot ID;
- canonical selector or explicit IDs;
- device IDs and identity revisions;
- query/view revision;
- creation time, actor, and expiry;
- result count;
- capability and policy/role revision per target.

If relevant facts change before confirmation or dispatch, the UI shows a diff.
It does not silently substitute a new device or identity into the operation.

A scheduled dynamic selector is a separate policy. It must state that
membership will be re-evaluated, set a maximum target count and growth policy,
run preflight at dispatch, preserve both planned and actual target sets, and
pause when risk or count crosses policy.

### Per-target preflight

Every target is evaluated across:

- identity and revision;
- support and enablement;
- operator, device, and app authorization;
- reachability and route;
- freshness;
- owner readiness;
- conflict or busy state;
- policy, maintenance window, and rollout ring;
- battery, storage, thermal, or bandwidth resources;
- idempotency and retry safety;
- unresolved cleanup.

The projection categories are `eligible`, `eligible-with-warning`, `excluded`,
`refresh-required`, and `changed-since-preview`. Exclusions have a stable
reason code and a human explanation.

### Confirmation and progress

Read-only or reversible actions use inline review. Operationally disruptive
actions use a review pane. Destructive, security-sensitive, or irreversible
multi-device work uses a dedicated confirmation surface with the exact target
snapshot, consequences, role check, exclusions, concurrency, expiry, and pause
policy.

Progress contains aggregate stage counts and a filterable per-target ledger.
One global progress bar is never the sole representation.

Cancellation distinguishes requested, cancelled-before-dispatch,
owner-accepted, already-applied, timed-out, and cleanup-required. Retry creates
a linked operation with a new request ID, reruns current preflight, omits
terminal or unsafe targets, and preserves the original result.

Cleanup is a first-class dimension. It may include session release, route
closure, temporary-file removal, media-sink stop, prior-state restoration, and
lease/concurrency release.

## Empty, stale, and degraded behavior

| Condition | Required projection |
| --- | --- |
| Initial loading | stable headers and skeleton rows, never a false zero-device state |
| Background refresh | retain current data and show bounded refresh status |
| Empty fleet | explain enrollment and expose UI plus CLI/API entrypoints |
| No matches | preserve scope and show `0 of N`, with targeted filter removal |
| Hub disconnected | cached read-only data with exact age; actions disabled with reason |
| Adapter failed | one adapter incident plus affected cells, not false device failures |
| Device offline | retain enrolled row, last accepted values, and explicit age |
| Family stale | retain the value with clock modifier until policy makes it unknown |
| Permission limited | show `Restricted` or `Unauthorized`, not blank or zero |
| Unsupported | static `Unsupported`; alert only when policy expects support |
| Partial data | each missing family reports its own state |
| Refresh failed | retain last accepted data, mark stale, and show last success |
| Revoked/deleted | remove from active inventory only under explicit lifecycle policy; retain audit |

## Accessibility acceptance

WCAG 2.2 is a requirements reference, adapted to native Windows semantics.
Microsoft UI Automation, Windows accessibility guidance, Narrator, high
contrast, and real WPF behavior are the platform acceptance surface.

Every WPF milestone verifies:

- all primary tasks work without a mouse;
- the grid is a bounded tab stop with managed row/cell navigation;
- focus, selection, and batch membership remain distinct and visible;
- focus survives virtualization and returns to the initiating context;
- names, roles, values, states, headers, sorting, row counts, and selection are
  exposed through UI Automation;
- status changes are announced without moving focus or reading every telemetry
  update;
- color is never the only meaning;
- light, dark, high-contrast, large-text, reduced-motion, and supported display
  scaling remain usable;
- disabled actions expose their reason;
- virtualized off-screen items do not leave stale or duplicate automation
  nodes;
- Narrator can find, inspect, filter, select, preview exclusions, and follow a
  partial result;
- automated accessibility regression checks cover critical screens, with
  manual Accessibility Insights and Narrator gates at milestone and release
  checkpoints.

## WPF and performance baseline

Native .NET WPF and native UI Automation behavior are the semantic baseline.
The initial primary-grid candidate is the native WPF `DataGrid`, exercised
with explicit columns, virtualization, recycling, selection, keyboard, and
automation tests.

The current M1 implementation checkpoint retains that dependency-free
baseline. It exercises 1,000 real Rust-projected rows with canonical
search/freshness filtering, explicit cohort/model/freshness/application
grouping, grouped recycling virtualization, separate inspection and batch
selection, hidden-selection accounting, and cached inspector context when the
selected device falls outside the applied scope. Background refresh updates
shared row facts without moving the collection; membership, order, and group
changes are counted and retained behind an explicit accessible application
control. Applying the latest queued snapshot preserves identity-based hidden
selection and cached inspection. The canonical operator fixture supplies 500
fresh, 250 stale, and 250 offline rows plus deterministic low-power and
capability-downgrade states; an unknown-freshness filter verifies the
zero-match state without clearing hidden selection or cached inspection. A
real presented-window pass also verifies the search → grid → batch → inspector
keyboard path through native UI Automation. Narrator, high-contrast,
large-text, and supported display-scaling review remain milestone gates and
are not claimed by this checkpoint.

WPF provides UI virtualization but not built-in data virtualization.
Therefore:

- state such as selection, expansion, and pinning lives in device/view models,
  not recycled containers;
- no visual object is created for an off-screen device;
- one shared clock updates displayed ages instead of per-cell timers;
- templates are checked for accidental virtualization disablement;
- Hub-side query, ordering, and windowing are designed before large-fleet
  support is claimed;
- changes are coalesced per device and delivered to the UI in bounded batches;
- high-rate media facts do not traverse the fleet-row update path;
- media decoding never occurs in fleet rows.

WPF UI is a conditional shell and theming candidate, not an accepted
dependency. Before adopting it or another styling library, a milestone spike
must test:

- a real fleet grid with at least 1,000 simulated devices;
- keyboard, UI Automation, Narrator, high contrast, system colors, scaling,
  focus, selection, and virtualization;
- maintenance status, license, third-party notices, and removal cost;
- whether custom controls obscure native automation behavior;
- isolation of semantic status mapping from library-specific types.

Fluent System Icons may be consumed through a project-owned semantic vector
icon map after license and accessibility review. Essential meaning must not
depend on a glyph font that may be absent or non-redistributable.

## Scale fixtures and candidate budgets

The simulator and projections use representative datasets at 4, 50, 250,
1,000, and 5,000 devices. These are test fixtures, not a declaration that every
size is supported.

Candidate budgets must be measured and either accepted or replaced:

| Scenario | Initial candidate |
| --- | --- |
| pointer or keyboard response, p95 | under 100 ms |
| cached 2,000-device filter, p95 | under 150 ms |
| Hub query first window, p95 | under 1 second |
| meaningful initial fleet rows | under 2 seconds on reference hardware |
| inspector selection to meaningful content | under 150 ms |
| batch preview first aggregate for 1,000 targets | under 1 second |
| telemetry churn | bounded memory and no unbounded UI-dispatch backlog |
| selected stream refresh | coalesced, bounded, and unable to starve status or input |
| preview count and decode budget | explicit admission; no ambient all-device decode |

Milestone acceptance records the reference hardware, dataset, update profile,
measurement method, achieved distribution, and headroom. A single fast run is
not a support claim.

## Milestone mapping

| Milestone | Operator-projection acceptance |
| --- | --- |
| M0 | canonical condition/query/operator and stream-health projections, damaged fixtures, scale datasets, and CLI/API parity |
| M1 | fleet grid, inspector, status/LSL observation grammar, stale/offline/degraded states, keyboard/UIA baseline, and WPF dependency decision |
| M2 | target snapshot, preflight, confirmation, per-target ledger, retry, cancellation, and cleanup |
| M3 | ADB/File Manager remains independent; wrong-device, privilege-age, and route-loss projections |
| M4 | stream catalog/admission, clock and generation detail, selected sample/media views, no-data/stall/freeze/cleanup states; control remains responsive |
| M5 | local/relay/hybrid routes, partitions, tenancy, and role downgrade remain explicit |
| M6 | saved views, grouping, alert suppression, maintenance windows, measured 1k/5k performance, and soak |
| M7 | operator runbook, complete accessibility gate, rollback, and exact artifact/revision agreement |

## Mitigation map

| Risk | Required mitigation |
| --- | --- |
| single health score | independent condition families and inspectable reasons |
| silent stale values | visible family age, source chain, freshness, and unknown transition |
| optimistic success | accepted, dispatched, applied, and cleaned remain separate |
| live row movement | stable ordering and explicit application of queued changes |
| hidden batch exclusions | frozen snapshot and per-target preflight |
| global progress masking failure | aggregate counts plus per-target ledger |
| ADB-centric shell | no-ADB inventory remains primary |
| icon-only density | labels, context, accessible names, and inspector detail |
| theme-library lock-in | semantic view models and a removal-cost spike |
| alert fan-out | root-cause grouping, suppression, maintenance, and affected counts |
| unbounded media preview | explicit selection, bounded sessions, independent backpressure |

## Next slice

Milestone 0 incorporates the canonical condition, query, operator-projection,
and scale-fixture contracts without adding WPF. Milestone 1 then implements the
first grid and inspector over those accepted routes and performs the WPF
component/dependency spike. This preserves one stacked milestone at a time.
