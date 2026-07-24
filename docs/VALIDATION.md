# Validation

## Goal

Validation should prove the boundary changed by the current work while avoiding
unrelated expensive suites. Quick, Standard, device, and Deep gates are
separate and cumulative only at the checkpoint where their evidence is needed.

## Repository tiers

### Quick

Quick is safe during normal editing and checks:

- required public repository files;
- JSON and JSONL syntax;
- whitespace through `git diff --check`;
- public-boundary and secret-like patterns;
- key planning/workflow invariants.
- pinned Rust formatting and the complete source-only workspace test suite;
- committed valid/damaged contract fixtures;
- deterministic 4/50/250/1,000/5,000 simulator generation and canonical
  mixed-freshness operator projection;
- Hub revision, replay, staleness, identity, and projection behavior;
- exact `fleetctl`/local-API projection parity;
- saved-view valid/damaged contracts, canonical ordering, optimistic
  revision conflict, durable restart restoration, HTTP CRUD, and structured
  `fleetctl` round-trip parity;
- stable .NET 10 WPF build plus the package-free native DataGrid validation
  against the real 1,000-device Rust projection;
- native grid/inspector UI Automation peers and names, grouped recycling
  virtualization, bounded realized rows, readable default column widths,
  stable view models, canonical search/freshness expressions and Hub-owned
  sort field/direction,
  pointer/keyboard/UI Automation batch selection, hidden-selection
  preservation, empty-scope behavior, retained out-of-scope inspector
  context, applied-sort preservation, stable live ordering with explicit
  accessible application, saved-view query/grouping/selection/focus
  and detail-tab restoration, accessible saved-view controls, canonical
  full-detail projection and return-context preservation, safe shared-row value refresh, and mixed
  fresh/stale/offline state;
- fail-closed non-loopback Hub, bounded response, mismatched-query, and
  wrong-device inspector/detail fixtures.

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-Repo.ps1 -Tier Quick
```

### Standard

Standard includes Quick and adds:

- internal Markdown link resolution;
- project/feature/workspace identity consistency;
- milestone-stack and inert-lock assertions;
- repository instruction and CI surface checks.
- operator-UI planning links, reference-ledger links, and public-safe research
  provenance.
- datastream architecture, current-state matrix, primary-source ledger, and
  cross-plane planning invariants.
- Clippy with warnings denied across all workspace targets;
- a structured four-device `fleetctl list` smoke projection.

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-Repo.ps1 -Tier Standard
```

When the Rusty Morphospace Work Environment is available, also validate the
portable project contract:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\Test-Repo.ps1 `
  -Tier Standard `
  -WorkEnvironmentRoot <work-environment-root>
```

### Device

There is no implicit device command in repository validation. A milestone that
requires a headset declares a separate run through the Meta Quest workflow,
with:

- exact target identity;
- source/build/profile preflight;
- bounded log and fatal window;
- owner-effective evidence;
- cleanup and prior-state restoration;
- a sanitized receipt kept within the public/private boundary.

### Deep

Deep includes Standard and adds:

- tracked-file reconciliation and large/generated artifact checks;
- architecture/authority/public-boundary review markers;
- any implementation-era full workspace, security, performance, or
  cross-repository checks registered by the active milestone.

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-Repo.ps1 -Tier Deep
```

Deep never performs live device work by itself.

## Milestone-specific validation

Each milestone adds one scenario suite rather than many aggregate scripts named
after micro-units. Focused tests may be numerous, but the stable operator
entrypoints remain:

- component-focused test command;
- milestone scenario suite;
- repository Quick/Standard/Deep gate;
- explicit device suite when required.

Rust 1.96, edition 2024, and the Cargo workspace are the selected Milestone 0
source toolchain. CI calls `Test-Repo.ps1`, so local and hosted Quick/Standard
gates exercise the same locked dependency graph and source suite.

Focused commands:

```powershell
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run --locked -p fleetctl -- list 4
cargo run --locked -p fleetctl -- detail sim-00001 4
cargo run --locked -p fleetctl -- operator-fixture mixed-freshness 50
cargo run --locked -p fleetctl -- saved-view-roundtrip 50
dotnet build .\apps\fleet-console-wpf.tests\RustyFleet.FleetConsole.Tests.csproj -c Release
dotnet run --project .\apps\fleet-console-wpf.tests\RustyFleet.FleetConsole.Tests.csproj `
  -c Release --no-build -- --repo-root .
```

The deterministic scale suite generates its datasets in memory. It does not
commit large data files and does not claim that every fixture size is a
supported production fleet.

The WPF scenario suite consumes the canonical watch-event shape as well as
query, summary, inspect, and detail. It verifies the 10,000-event request
ceiling, strict sequence/cursor ordering, accepted-versus-rejected semantics,
Hub sequence-reset rebasing, canonical query/summary reread, stable ordering,
fail-closed damaged watch evidence with cached rows retained, and query-only
manual refresh when the watch route is unavailable.

## Datastream validation

The normative contract and fixture families are in
[Datastream Management](DATASTREAMS.md). Validation preserves owner maturity:
a candidate or lab result can inform a damaged/negative fixture but cannot
become a supported capability without the exact owner-repository promotion.

| Check | M0 | M1 | M4 | Relay/release |
| --- | ---: | ---: | ---: | ---: |
| generic/native descriptor, selection, component epochs, plane, time, cadence, lifecycle, and profile health | required | focused | Standard | Deep |
| no-data, stall, freeze, decode/sink, and cleanup distinctions | simulated | observation path | device/profile matrix | regression |
| finite per-edge queue/drop/recovery/fan-out policy | contract | status/LSL path | measured | soak |
| control-capacity reserve and fair admission | simulated | local transport | measured media load | remote/tenant load |
| Console/CLI/API stream catalog and reason parity | contract | observation projection | Standard | regression |
| low-cardinality metrics and units | contract | focused | measured | Deep |
| source/route/socket/codec/sink owner evidence | not applicable | exact adapter | exact promoted profiles | exact relay profiles |
| scientific run, XDF record/replay, consent, retention, redaction, and cleanup | contract | compatibility | device/security gate | Deep |

An FFmpeg adapter is tested at four independent levels: allowlisted command
construction plus protocol/codec/resource limits; machine-readable
probe/progress parsing plus Hostess watchdogs; deterministic fixture media
including configuration/keyframe, decoded/rendered, changing/static-content,
and branch-local queue behavior; and bounded full-process-tree
termination/cleanup. Live device tests are required only for exact promoted
Quest source/route/sink combinations.

## Operator UI validation

The normative behavior and candidate budgets are in
[Operator UI Architecture](OPERATOR_UI.md). The gates accumulate only when a
WPF surface exists:

| Check | M0 | M1+ edit loop | WPF milestone | Release |
| --- | ---: | ---: | ---: | ---: |
| canonical condition/query/projection fixtures | required | focused | Standard | Deep |
| deterministic 4/50/250/1k/5k datasets | required | affected profile | Standard | Deep |
| Console/CLI/API membership and reason parity | contract only | focused | Standard | Deep |
| keyboard and UI Automation regression | not applicable | focused | Standard | Standard |
| Narrator, high contrast, large text, scaling | not applicable | targeted | manual milestone gate | full release gate |
| stable ordering, hidden selection, navigation restoration | contract fixture | focused | scenario gate | regression |
| target snapshot and per-target ledger | M2 contract | focused | M2 Standard | regression |
| measured latency, memory, and update churn | candidate fixtures | nearest profile | declared milestone | Deep |

Performance thresholds in the UI guide are candidates until a milestone
records reference hardware, data profile, method, distribution, achieved
result, and headroom. Do not convert a single fast run into a supported-scale
claim.

Screenshot matrices detect layout drift but do not replace keyboard, UI
Automation, screen-reader, or interaction tests.

The optional `--present` argument opens the same package-free validation
surface as a real WPF window for bounded focus, keyboard, screen-reader, and
visual review. It does not start a Hub or contact a device. Presented-window
evidence complements the default off-screen regression run and does not make
Narrator, high-contrast, large-text, or scaling gates automatic.

## Evidence vocabulary

Keep these facts distinct:

- **observed:** an adapter reported something;
- **accepted:** the authority admitted it at a revision;
- **dispatched:** an accepted command was sent;
- **applied:** the owning consumer reported the effect;
- **cleaned:** terminal cleanup was independently observed;
- **rejected/expired/cancelled:** no successful application is claimed.

For streams, also keep distinct:

- **available:** the current owner manifest/source epoch can be considered;
- **admitted:** a current authority accepted the subscription/session;
- **connected/running:** the selected route or process is active;
- **progressing:** samples or frames advance under the current component
  epochs;
- **decoded/validated:** codec or schema application succeeds;
- **sunk:** the selected consumer applies or renders current payload;
- **stalled/frozen:** transport or content progress is not healthy even if
  another stage remains active.
- **recording:** durable artifact progress is independently healthy or failed;
- **replayed:** the selected artifact preserved the required native metadata,
  timing, sample/event, and integrity evidence.

An aggregate fleet result is a projection over per-device facts, not a
replacement for them.

## GitHub cadence

- Quick runs on every push and pull request.
- Standard runs on pull requests and `main`.
- Deep is manual/release-triggered until implementation needs a scheduled
  integration gate.
- Device suites run outside generic GitHub-hosted CI.
