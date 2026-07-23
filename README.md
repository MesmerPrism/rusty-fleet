# Rusty Fleet

Rusty Fleet is the planning and product control surface for a multi-headset
Meta Quest dashboard. It is designed to show every enrolled headset that is
checking in, even when ADB is unavailable, and to expose stronger operations
only when the device reports the required capability and authority.

Milestone 0 is accepted and published. Milestone 1 is active on its working
branch. The current checkpoint adds provenance-bearing Quest observation
facts, a signed check-in envelope admitted transactionally through the exact
pinned Manifold peer authority, an explicit bounded local Hub runtime, durable
two-slot restart recovery, and one cleaned private Quest Wi-Fi proof. Nothing
listens by default: the operator must supply a valid enrollment config,
absolute private state directory, and explicitly permit a non-loopback bind.
The native WPF table/inspector slice now includes canonical search and
freshness scope, explicit cohort/model/freshness/application grouping,
hidden-selection preservation, and retained inspector context across scope
changes. Its package-free 1,000-device and presented-window keyboard/UI
Automation checks pass. Manual Narrator, high-contrast, scaling, final M1
consolidation, media, and remote relay remain pending.

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
  summary, and watch behavior;
- `fleet-manifold-adapter`: exact Manifold enrollment/status admission,
  Ed25519/JCS verification, replay-window enforcement, and all-or-neither
  Manifold/Fleet state application;
- `fleet-hub-local`: explicit bounded HTTP check-in ingress plus health,
  query, summary, inspect, detail, and watch projections over the same Hub;
- `fleet-simulator`: reproducible 4, 50, 250, 1,000, and 5,000-device
  datasets plus damage and lifecycle mutations;
- `fleetctl`: a structured JSON projection over the same local API;
- `fleet-console-wpf`: a native WPF `DataGrid`, visible canonical scope and
  grouping, distinct inspection and batch selection, and a persistent
  selected-device inspector over the canonical local API;
- `fleet-console-wpf.tests`: package-free native UI Automation,
  grouped virtualization, stable-context, capability-family, presented
  keyboard, and 1,000-device checks.

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
7. Resume project state from [the Morphospace workspace](morphospace/README.md).

The active implementation stack is `fleet-m1-local-no-adb-monitoring`. It
delivers the authenticated local check-in, bounded Hub runtime, Quest Fleet
Agent, shared CLI/API/WPF projections, negative paths, and final device proof
as one coherent vertical slice. It is not split into separate lifecycle units
for each schema, field, transport handler, control, or test.

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
cargo run --locked -p fleetctl -- watch 4
```

These commands create synthetic in-memory data only. The M1 local Hub remains
inert unless it is launched with an explicit enrolled config:

```powershell
cargo run --locked -p fleet-hub-local -- --config <private-local-config.json>
```

Non-loopback binding additionally requires `allow_non_loopback=true` in that
private config. Durable state additionally requires an absolute private
`state_directory`. See the
[M1 runtime guide](docs/M1_LOCAL_MONITORING.md).

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

The accepted M0 baseline and inert Morphospace protocol-v2 workspace are now
paired with the active M1 local-monitoring stack. The runtime source is
present but activates no socket, service, device route, or platform permission
by default. The bounded Quest checkpoint and a producer-stopped durable Hub
restart have passed with private evidence and complete device cleanup. M1
now also has its native WPF table/inspector, canonical scope/grouping,
stable-context behavior, and automated 1,000-device virtualization/UI
Automation baseline. A real presented-window pass verifies search, grid,
batch, and inspector keyboard focus. Acceptance remains pending until the
manual Narrator, high-contrast, scaling, full Standard, Deep, workflow, and
publication gates pass.

## License

Rusty Fleet is licensed under the GNU Affero General Public License,
version 3 or later (`AGPL-3.0-or-later`). See [LICENSE](LICENSE).
