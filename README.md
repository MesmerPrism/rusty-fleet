# Rusty Fleet

Rusty Fleet is the planning and product control surface for a multi-headset
Meta Quest dashboard. It is designed to show every enrolled headset that is
checking in, even when ADB is unavailable, and to expose stronger operations
only when the device reports the required capability and authority.

The initial repository is deliberately planning-first. No runtime feature,
listener, Android permission, device mutation, media route, or remote relay is
active in this baseline.

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

This avoids turning Meta Quest File Manager into a fleet controller or putting
device, relay, media, and operator authority into one application.

## Start here

1. Read the [implementation plan](docs/IMPLEMENTATION_PLAN.md).
2. Read the [stacked milestone workflow](docs/WORKFLOW.md).
3. Review the [architecture and ownership boundaries](docs/ARCHITECTURE.md).
4. Use the [validation matrix](docs/VALIDATION.md) to select the smallest
   sufficient check.
5. Resume project state from [the Morphospace workspace](morphospace/README.md).

The first proposed implementation stack is
`fleet-m0-foundation-and-simulator`. It produces contracts, a deterministic
multi-device simulator, and a CLI/API-observable Hub skeleton as one coherent
vertical slice. It is not split into separate lifecycle units for each schema,
class, command, or test.

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

The repository currently contains the accepted planning baseline and an inert
Morphospace protocol-v2 project workspace. Runtime implementation begins only
after the first milestone stack is reviewed into `ready`.

## License

Rusty Fleet is licensed under the GNU Affero General Public License,
version 3 or later (`AGPL-3.0-or-later`). See [LICENSE](LICENSE).
