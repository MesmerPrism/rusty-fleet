# ADR 0002: Operator information architecture

- Status: accepted for planning
- Date: 2026-07-23

## Context

Rusty Fleet must present many independent and changing facts across a fleet
without hiding provenance, optional capabilities, partial results, or
per-device failures. A card grid or single health score looks approachable for
a few devices but becomes ambiguous and space-inefficient at operational scale.

The research audit compared current Windows guidance, accessibility standards,
fleet-management products, device-state systems, job/rollout systems, and WPF
implementation constraints. The sources and rejected overreach are recorded in
`docs/research/FLEET_UI_SOURCE_LEDGER.md`.

## Decision

Use:

- a dense virtualized fleet table as the home workspace;
- a persistent list/details inspector and a separate full-detail route;
- independent timestamped status conditions rather than one health score;
- visible canonical query, saved-view, grouping, sorting, and selection scope;
- stable ordering while an operator interacts;
- frozen target snapshots and per-device preflight for fleet actions;
- aggregate operation counts plus a per-target lifecycle and cleanup ledger;
- native WPF/UI Automation semantics as the platform baseline;
- representative scale, keyboard, Narrator, high-contrast, and scaling
  evidence as milestone acceptance.

Keep WPF theme-library and exact visual-token selection deferred until the
Milestone 1 dependency spike. Keep performance thresholds provisional until
measured.

## Consequences

- Milestone 0 must define canonical condition, query, saved-view,
  operator-projection, target-independent operation, and scale-fixture
  contracts before WPF lands.
- Milestone 1 implements the table and inspector over accepted Hub routes.
- Milestone 2 treats target snapshot, exclusion reasons, progress, retry,
  cancellation, and cleanup as product contracts.
- Optional ADB, File Manager, relay, and media capabilities degrade locally.
- The Console cannot create presentation-only state or action semantics that
  `fleetctl` and the local API cannot inspect.

## Rejected alternatives

- **Device card grid as home:** rejected for density and comparison cost.
- **One traffic-light health value:** rejected because it erases authority,
  freshness, capability, and cause.
- **Live resorting by default:** rejected because rows move under focus and
  selection.
- **One batch progress bar:** rejected because it hides incomparable stages
  and target failures.
- **Adopt a Fluent WPF library now:** rejected until real grid,
  accessibility, performance, license, and removal-cost evidence exists.
- **Automatic all-device media preview:** rejected because media is separately
  selected, authorized, bounded, and backpressured.
