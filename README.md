# Rusty Fleet

Rusty Fleet is the planning and product control surface for a multi-headset
Meta Quest dashboard. It is designed to show every enrolled headset that is
checking in, even when ADB is unavailable, and to expose stronger operations
only when the device reports the required capability and authority.

Milestone 0 and the consolidated Milestone 1 monitoring baseline are
published. The current Milestone 2 source checkpoint adds the first bounded
participating-app operation, `kiosk.show-controls`, while retaining the
provenance-bearing Quest observation
facts, a signed check-in envelope admitted transactionally through the exact
pinned Manifold peer authority, an explicit bounded local Hub runtime, durable
two-slot restart recovery, and one cleaned private Quest Wi-Fi proof. Nothing
listens by default: the operator must supply a valid enrollment config,
absolute private state directory, and explicitly permit a non-loopback bind.
The native WPF table/inspector slice now includes canonical search and
freshness scope, Hub-owned sort choices, explicit
cohort/model/freshness/application grouping, hidden-selection preservation,
retained inspector context across scope changes, and an explicit queue for
live membership, ordering, or grouping changes while shared row values
refresh in place. Fleet Hub now also owns a bounded, revisioned, durably
restored saved-view collection; the Console captures and reapplies the exact
query, grouping, visible column order, selected device, scroll anchor, and
focus region without creating WPF-only authority. The same checkpoint adds
structured `fleetctl detail` parity and a keyboard-accessible full-device
detail surface with overview, status, capability, work, stream, and retained
condition-history tabs; returning preserves the exact fleet scope, selection,
scroll anchor, and stable identity. Its package-free
10/50/100-row off-screen layout/virtualization checks and normal 50-row
presented-window keyboard/UI Automation checks pass. The automated host-side source stack has
also passed Quick, Standard, Deep, workflow-contract, and exact Quest-owner
`Host` validation; see the
[M1 consolidation readiness record](docs/M1_CONSOLIDATION_READINESS.md).
The cumulative Narrator, Accessibility Insights, high-contrast, large-text,
scaling, and multi-monitor suite remains the Milestone 7 release gate. Media
and remote relay remain pending. The operation uses an immutable exact-target
preview, explicit confirmation, the pinned Manifold Runtime Host, Kiosk's
signed direct-operator contract, receipt-gated application, bounded
scheduling, durable same-request recovery, and Console/CLI/API parity. It does
not require ADB and remains inert without private Kiosk endpoint/key
configuration. See the
[M2 Kiosk show-controls guide](docs/M2_KIOSK_SHOW_CONTROLS.md).

The additive package stack now exposes
`packages.install-release` through the same local API, `fleetctl`, and WPF
Console. It freezes a signed release reference and exact device identities,
then prepares every eligible updater invocation after explicit confirmation.
When and only when private updater-owner configuration is present, the
authenticated owner can claim one exact invocation through the durable bounded
scheduler and return bound acknowledgement or effective install evidence.
That owner ingress is loopback-only even when the general Hub configuration
permits a non-loopback bind; it is a source-side admission surface, not an
updater transport or client.
Claims and acknowledgements never prove installation; only the exact accepted
`install_commit` receipt advances a target to Applied. The ingress remains
disabled by default. See the
[package install/release checkpoint](docs/PACKAGE_INSTALL_RELEASE.md).

The additive Quest awake-control stack exposes the bounded Meta development
hold (maximum eight hours), explicit Windows and on-device watchdog modes, and
separate stop-watchdogs and restore-normal actions. Fleet owns immutable target
policy and scheduling, Manifold owns command authorization, and a pinned
QuestIonAble File Manager provider owns exact-serial ADB effects and readback.
The surface is inert without separately SHA-256-pinned provider and `adb.exe`
artifacts plus private exact-device bindings, and public receipts contain no
serials or paths. See
[Quest awake control](docs/QUEST_AWAKE_CONTROL.md).

The additive Quest Wi-Fi ADB stack exposes Kiosk-backed modern wireless
debugging requests, after-boot request policy, explicit disable, and a
separate classic USB `tcpip` action through a pinned QuestIonAble File Manager
provider. Fleet and Manifold retain policy and command authority; private
serial, endpoint, and pairing resolution stays in File Manager. Termux
usability is projected only from an enrolled signed check-in carrying fresh
exact `uid=2000(shell)` capability evidence, never from the provider receipt
or a caller-submitted proof. The Hub adds a separate Fleet-owned verified
admission that binds the proof to its enrolled key, canonical signed claims,
accepted revisions, exact device/operation, and owner receipt. See
[Quest Wi-Fi ADB control](docs/QUEST_WIFI_ADB_CONTROL.md).
The separate
[two-Quest attended acceptance transaction](docs/QUEST_WIFI_ADB_TWO_QUEST_ACCEPTANCE.md)
binds offline onboarding, File Manager profiles, the proof helper, signed
Fleet state, two-device isolation, typed wearer/reboot checkpoints, and exact
cleanup without putting private run inputs in this repository. Its host
mutation journal is write-through, digest-chained, and never redispatches an
interrupted unknown outcome. A pinned Agent Board wrapper supplies a private,
run-bound reservation receipt for both exact Quest resources; the runner
revalidates both leases before every mutation, retains them through cleanup,
and exposes only a sanitized reservation disposition.

The accepted operator-information architecture uses a dense virtualized fleet
table, a persistent selected-device inspector, independent timestamped status
conditions, visible query/selection scope, and per-device operation evidence.
See the [operator UI guide](docs/OPERATOR_UI.md).

The datastream architecture composes LSL, status, spatial, media, and future
relay streams without forcing them through one transport. It standardizes
generic/native descriptors, source selection, component epochs, timing,
profile health, per-edge queues, scientific recording/replay, admission
budgets, cleanup, and evidence while preserving every owner boundary. See
the [datastream guide](docs/DATASTREAMS.md).

## Product shape

Rusty Fleet is a Hostess/operator product composed of three projections over
one authority-aware engine:

- **Fleet Hub** maintains the device directory, accepted status, command
  lifecycle, audit trail, and adapter registry.
- **Fleet Console** is the Windows WPF dashboard for humans.
- **`fleetctl`** and a local API expose the same operations and evidence to
  automation.

The headset-side Fleet Agent belongs in the Rusty Quest platform lane. Manifold
owns accepted command, session, peer, and stream authority. Existing Kiosk and
File Manager products remain independent applications behind versioned
adapters. Media transport remains a separate data plane.

This avoids turning QuestIonAble File Manager into a fleet controller or putting
device, relay, media, and operator authority into one application.

The current implementation is split into:

- `fleet-contracts`: versioned identity, condition, capability, query,
  projection, command, and datastream contracts;
- `fleet-hub`: deterministic in-memory acceptance, freshness, query, inspect,
  summary, watch, and revisioned saved-view behavior;
- `fleet-manifold-adapter`: exact Manifold enrollment/status admission,
  Ed25519/JCS verification, replay-window enforcement, and all-or-neither
  Manifold/Fleet state application, plus pinned Runtime Host command review,
  application, replay, and audit state;
- `fleet-kiosk-adapter`: bounded no-redirect signed Kiosk status, invoke, and
  result transport with owner-vector parity and poll-only restart recovery;
- `fleet-package-updater-adapter`: bounded invocation and untrusted
  owner-evidence validation for the pinned attended Rusty Quest updater,
  without Android or evidence-admission authority;
- `fleet-quest-awake-adapter`: pinned local File Manager provider execution,
  exact invocation/receipt validation, and public readback projection without
  serial or path disclosure;
- `fleet-quest-connectivity-adapter`: pinned local File Manager connectivity
  provider execution and strict sanitized receipt binding, with no private
  target details or Termux authority;
- `fleet-hub-local`: explicit bounded HTTP check-in ingress plus health,
  query, summary, inspect, detail, watch, saved-view, and operation projections
  over the same Hub and fully validated two-slot durable envelope;
- `fleet-simulator`: reproducible 4, 50, 250, 1,000, and 5,000-device
  datasets, a canonical mixed-freshness operator fixture, and damage/lifecycle
  mutations;
- `fleetctl`: structured JSON list/inspect/detail/watch projections,
  saved-view parity fixtures, package, Quest awake, Quest connectivity, and
  Windows host-hotspot preview/execute/get commands over the same loopback
  local API;
- `fleet-console-wpf`: a native WPF `DataGrid`, visible canonical
  scope/sort/grouping, revisioned saved-view controls, stable live-order
  application, bounded monotonic watch synchronization, distinct inspection
  and batch selection, a host-scoped Windows Mobile Hotspot pane, and a
  persistent selected-device inspector plus full-device detail over the
  canonical local API;
- `fleet-console-wpf.tests`: package-free native UI Automation,
  watch-cursor/reset/damage, grouped virtualization, stable-context/order,
  capability-family, normal 50-row presented keyboard, and 10/50/100-row
  off-screen layout checks.
- `fleet-onboarding` and `fleet-onboard`: a standalone, offline,
  confirmation-bound generator for new current-user-only Fleet Agent
  profiles, seeds, public key records, and Hub enrollment configuration. Its
  pinned developer-evidence tool runs from a deny-rename retained path inside
  a bounded Windows Job Object, and rollback/cleanup use a closed
  retained-handle inventory. The current portable Rusty Quest capsule is
  explicitly pre-build-snapshot, unproven source binding usable only by exact
  artifact hash, and development-only; distribution still requires a
  separately signed owner-issued release capsule. The generator neither
  installs nor enrolls devices and is
  documented in
  [Offline Fleet Onboarding](docs/OFFLINE_ONBOARDING.md).

## Consolidation maturity

The current source candidate pins every resolved Manifold crate to exact
revision `ef1d40b8e0b7e7b47270509eddf53787c23b9fea`. Fleet consumes Runtime Host
v4 and explicitly migrates retained v2 command/replay state without creating a
lease, revocation, barrier, convergence result, or retaining-consumer
acknowledgement. Manifold command application, effect-owner acknowledgement,
effective result, and terminal cleanup remain separate evidence.

This composition is a headless source candidate. Repository validation does
not claim a GUI-attended pass, Quest/device behavior, signed Windows bundles,
publication, or release availability. Those remain separately gated.

The Windows distribution source now composes exactly five inert components,
including `fleet-onboard`, into a deterministic validated ZIP and embeds it in
the generic-icon `RustyFleet-Setup.exe`. Setup exposes exact no-change
`--plan --json` automation plus a visible zero-argument guided install. QFM
may verify and launch that Setup through the v2 signed-descriptor handoff, but
Fleet owns composition, installation, updates, and rollback metadata. Pages
is the documentation authority and, once the protected metadata deployment
exists, will host short-lived signed `release.json`; immutable binaries belong
only to GitHub Releases. No supported download or metadata deployment is
claimed until the protected workflows produce and independently validate one.

The Pages handoff is renewable rather than release-run-only: a protected
12-hour workflow revalidates the exact visible ten-asset GitHub Release,
tag/commit/tree, tagged trust policy, Authenticode identities, and descriptor
SPKI before signing fresh 23-hour metadata. It deploys only the human site,
descriptor, receipt, public SPKI, and a relative hash-bound renewal handoff;
Setup and archives remain GitHub Release assets. The workflow is inert until
its explicit deployment-enable variable, reviewed public pins, exact release
identity, and protected descriptor key are configured.

The deterministic M1 negative-path harness runs four independent devices
through sleep/wake aging, route loss/recovery, duplicate and stale check-ins,
agent upgrade, old-epoch replay, and final canonical recovery. The exact
pinned Manifold adapter also proves key rotation rejects the old signer and
accepts the replacement only under a fresh producer epoch.

See the [Milestone 0 source foundation](docs/M0_SOURCE_FOUNDATION.md) for the
accepted source boundary, the
[M1 local monitoring runtime](docs/M1_LOCAL_MONITORING.md) for the active
ingress contract, and the
[M0 graph/instruction review](docs/M0_GRAPH_AND_INSTRUCTION_REVIEW.md) for the
bounded dependency, authority, activation, and instruction audit.

## Start here

1. Read the [implementation plan](docs/IMPLEMENTATION_PLAN.md).
2. Read the [stacked milestone workflow](docs/WORKFLOW.md).
3. Review the [architecture and ownership boundaries](docs/ARCHITECTURE.md).
   The executable M0 trust boundary is recorded in
   [ADR 0004](docs/decisions/0004-m0-source-boundary-and-threat-model.md);
   M1 check-in authority and local-ingress security are recorded in
   [ADR 0005](docs/decisions/0005-m1-checkin-authority.md) and
   [ADR 0006](docs/decisions/0006-m1-local-ingress-threat-model.md).
4. Review [datastream management](docs/DATASTREAMS.md), the
   [current Morphospace stream matrix](docs/research/MORPHOSPACE_DATASTREAM_MATRIX.md),
   [primary-source ledger](docs/research/DATASTREAM_REFERENCE_LEDGER.md), and
   [research integration review](docs/research/FLEET_RESEARCH_INTEGRATION_REVIEW.md).
5. Review the [operator UI architecture](docs/OPERATOR_UI.md) and its
   [reference ledger](docs/research/FLEET_UI_SOURCE_LEDGER.md).
6. Use the [validation matrix](docs/VALIDATION.md) to select the smallest
   sufficient check.
7. Review the
   [M2 Kiosk show-controls authority and recovery boundary](docs/M2_KIOSK_SHOW_CONTROLS.md).
8. Review the
   [package install/release source boundary](docs/PACKAGE_INSTALL_RELEASE.md).
9. Review the
   [Quest awake-control boundary](docs/QUEST_AWAKE_CONTROL.md).

The current source stack contains the first M2 participating-app operation,
`kiosk.show-controls`, plus the additive `packages.install-release`
preparation checkpoint. Dedicated private planning owns iteration state; the
public repository contains only coherent contracts, source, sanitized
fixtures, and negative-path validation.

## Source workflow

The repository pins Rust 1.96 and edition 2024. Run focused checks directly:

```powershell
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Inspect the deterministic four-device projection:

```powershell
cargo run --locked -p fleetctl -- list 4
cargo run --locked -p fleetctl -- inspect sim-00001 4
cargo run --locked -p fleetctl -- detail sim-00001 4
cargo run --locked -p fleetctl -- watch 4
cargo run --locked -p fleetctl -- m1-lifecycle
cargo run --locked -p fleetctl -- operator-fixture mixed-freshness 50
cargo run --locked -p fleetctl -- saved-view-roundtrip 50
```

These commands create synthetic in-memory data only. The operator fixture
projects fresh, stale, offline, low-power, and capability-downgrade examples
through the same Hub query and summary APIs. `saved-view-roundtrip` is an
in-process parity/conformance projection; persistent operator views use the
explicit local Hub HTTP routes. The M1 local Hub remains inert
unless it is launched with an explicit enrolled config:

```powershell
cargo run --locked -p fleet-hub-local -- --config <private-local-config.json>
```

Non-loopback binding additionally requires `allow_non_loopback=true` in that
private config. Durable state additionally requires an absolute private
`state_directory`. See the
[M1 runtime guide](docs/M1_LOCAL_MONITORING.md).
The M2 operation commands use the explicit loopback Hub and are documented in
the [M2 guide](docs/M2_KIOSK_SHOW_CONTROLS.md).
Package commands use that same loopback Hub and are documented in the
[package checkpoint](docs/PACKAGE_INSTALL_RELEASE.md).
Quest awake commands use that same loopback Hub and are documented in
[Quest awake control](docs/QUEST_AWAKE_CONTROL.md).
Quest Wi-Fi ADB commands use that same loopback Hub and are documented in
[Quest Wi-Fi ADB control](docs/QUEST_WIFI_ADB_CONTROL.md).
Live two-headset qualification uses the separately confirmed, resumable
[two-Quest acceptance transaction](docs/QUEST_WIFI_ADB_TWO_QUEST_ACCEPTANCE.md);
generic repository tiers run only its host/synthetic tests.
Windows host-hotspot commands and Console behavior use that same loopback Hub
and are documented in
[Windows Mobile Hotspot control](docs/WINDOWS_HOTSPOT_CONTROL.md).
Inert owner-provider metadata uses an explicit fail-closed refresh and a
Hub-owned snapshot shared by the local API, `fleetctl`, and Console; see the
[provider capability catalog](docs/PROVIDER_CAPABILITY_CATALOG.md). That guide
also documents the optional check-in-only LAN listener which lets enrolled
Fleet Agents report signed evidence while all provider/operator routes remain
loopback-only.

Build and exercise the native WPF projection against the real deterministic
Rust query result:

```powershell
dotnet build .\apps\fleet-console-wpf.tests\RustyFleet.FleetConsole.Tests.csproj -c Release
dotnet run --project .\apps\fleet-console-wpf.tests\RustyFleet.FleetConsole.Tests.csproj `
  -c Release --no-build -- --repo-root .
```

The Console starts disconnected and accepts only an explicit loopback HTTP
Hub address. It does not start the Hub, discover devices, or activate a
headset route.

## Validation

Run the edit-sized checks:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-Repo.ps1 -Tier Quick
```

Run the repository checkpoint before a milestone handoff:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-Repo.ps1 -Tier Standard
```

Deep validation is reserved for architecture, security, relay, media, release,
or broad integration checkpoints:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-Repo.ps1 -Tier Deep
```

These commands do not contact or mutate a headset.

## Status

The accepted M0 and consolidated M1 baselines are now paired with the first
M2 participating-app control slice. The runtime source is
present but activates no socket, service, device route, or platform permission
by default. The bounded Quest checkpoint and a producer-stopped durable Hub
restart have passed with private evidence and complete device cleanup. M1
now also has its native WPF table/inspector, canonical scope/sort/grouping,
Hub-owned saved-view persistence/restoration, stable-context behavior,
explicit queued live ordering, and automated
10/50/100-row off-screen layout/virtualization coverage, with a normal
50-device mixed 25-fresh/13-stale/12-offline projection and a separate
50-row presented-window keyboard/UI Automation pass. Larger generated
fleets remain separate stress evidence rather than the default Console design
target. A real presented-window
pass verifies search, grid, batch, and inspector keyboard focus. Its
self-checking lifecycle projection and exact-owner key-rotation gate cover the
remaining Fleet-owned deterministic lifecycle cases. M1 is functionally
closed through an additive corrective unit. The current-settings presented
keyboard pass and preliminary Narrator confirmation are informative, not
comprehensive accessibility conformance. Automated keyboard and UI Automation
remain milestone regressions; the cumulative Narrator, Accessibility Insights,
high-contrast, large-text, scaling, and multi-monitor suite remains an explicit
Milestone 7 release gate after the full operator workflow exists. The completed
M1 evidence and exact boundary are recorded in
[M1 Consolidation Readiness](docs/M1_CONSOLIDATION_READINESS.md).
The source-only M2 Kiosk slice is implemented and device-free validation is
green; a live owner/device proof remains a separate explicit gate. The package
source stack now includes disabled-by-default authenticated updater ingress and
durable bounded owner claims. Live updater/device proof, attended wearer
behavior, and WPF activation of post-claim controls remain separate explicit
gates.

## License

Rusty Fleet is licensed under the GNU Affero General Public License,
version 3 or later (`AGPL-3.0-or-later`). See [LICENSE](LICENSE).
