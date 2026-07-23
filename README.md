# Rusty Fleet

Rusty Fleet is the planning and product control surface for a multi-headset
Meta Quest dashboard. It is designed to show every enrolled headset that is
checking in, even when ADB is unavailable, and to expose stronger operations
only when the device reports the required capability and authority.

Milestone 0 is now active as a source-only Rust workspace. It contains
versioned contracts, deterministic synthetic fleets, an in-memory Hub, and
CLI/local-API projections. No runtime listener, Android permission, device
mutation, media route, persistence service, or remote relay is active.

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

The current source-only implementation is split into:

- `fleet-contracts`: versioned identity, condition, capability, query,
  projection, command, and datastream contracts;
- `fleet-hub`: deterministic in-memory acceptance, freshness, query, inspect,
  summary, and watch behavior;
- `fleet-simulator`: reproducible 4, 50, 250, 1,000, and 5,000-device
  datasets plus damage and lifecycle mutations;
- `fleetctl`: a structured JSON projection over the same local API.

See the [Milestone 0 source foundation](docs/M0_SOURCE_FOUNDATION.md) for the
current boundary and scenario model, and the
[M0 graph/instruction review](docs/M0_GRAPH_AND_INSTRUCTION_REVIEW.md) for the
bounded dependency, authority, activation, and instruction audit.

## Start here

1. Read the [implementation plan](docs/IMPLEMENTATION_PLAN.md).
2. Read the [stacked milestone workflow](docs/WORKFLOW.md).
3. Review the [architecture and ownership boundaries](docs/ARCHITECTURE.md).
   The executable M0 trust boundary is recorded in
   [ADR 0004](docs/decisions/0004-m0-source-boundary-and-threat-model.md).
4. Review [datastream management](docs/DATASTREAMS.md), the
   [current Morphospace stream matrix](docs/research/MORPHOSPACE_DATASTREAM_MATRIX.md),
   [primary-source ledger](docs/research/DATASTREAM_REFERENCE_LEDGER.md), and
   [research integration review](docs/research/FLEET_RESEARCH_INTEGRATION_REVIEW.md).
5. Review the [operator UI architecture](docs/OPERATOR_UI.md) and its
   [reference ledger](docs/research/FLEET_UI_SOURCE_LEDGER.md).
6. Use the [validation matrix](docs/VALIDATION.md) to select the smallest
   sufficient check.
7. Resume project state from [the Morphospace workspace](morphospace/README.md).

The active implementation stack is
`fleet-m0-foundation-and-simulator`. It produces contracts, a deterministic
multi-device simulator, and a CLI/API-observable Hub skeleton as one coherent
vertical slice. It is not split into separate lifecycle units for each schema,
class, command, or test.

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

These commands create synthetic in-memory data only.

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

The accepted planning baseline and inert Morphospace protocol-v2 workspace are
now paired with the active Milestone 0 source foundation. The implementation
does not activate a stream, socket, service, device route, or platform
permission. Milestone acceptance remains pending until the complete Standard
gate, workflow receipts, and publication checkpoint pass.

## License

Rusty Fleet is licensed under the GNU Affero General Public License,
version 3 or later (`AGPL-3.0-or-later`). See [LICENSE](LICENSE).
