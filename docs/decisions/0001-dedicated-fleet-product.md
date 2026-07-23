# ADR 0001: Dedicated fleet product

- Status: accepted for planning
- Date: 2026-07-23

## Context

The desired dashboard must monitor many headsets without ADB, provide
participating-app controls, expose File Manager functions when ADB is
available, and later adopt relay and media capabilities. These concerns cross
operator UI, command authority, Quest platform behavior, app-local behavior,
privileged device utilities, and high-rate data paths.

Placing all of them inside File Manager would make an optional privileged
adapter the product shell and would mix unrelated authorities.

## Decision

Create Rusty Fleet as a dedicated Hostess/operator product with Fleet Hub,
Fleet Console, and `fleetctl`/local API projections.

Keep:

- Manifold as command/session/peer/stream authority;
- Rusty Quest as platform and device-agent owner;
- Kiosk as app-local action owner;
- File Manager as ADB/file-operation owner;
- LSL, BLE, and ZeroMQ as bounded observation/rendezvous adapters;
- media as a separate source/process/route/codec/sink plane.

## Consequences

- Base monitoring works without ADB.
- Optional capabilities can disappear without removing the device from the
  fleet view.
- CLI/API parity is an architectural requirement.
- Cross-repository adapters need explicit versioned contracts and coordinated
  validation.
- The product incurs a dedicated Hub and operator shell, but avoids hidden
  authority and product coupling.

## Rejected alternatives

- **Put the dashboard in File Manager:** rejected because File Manager is an
  optional privileged adapter and should remain independently releasable.
- **Use LSL as fleet authority:** rejected because observations and discovery
  do not provide the required admission, replay, expiry, revocation, and audit
  model.
- **Use ADB as the normal control plane:** rejected because it excludes the
  required no-ADB monitoring and participating-app path.
- **Carry media over status/check-in messages:** rejected because control and
  high-rate data require different lifecycle, performance, and security
  ownership.
