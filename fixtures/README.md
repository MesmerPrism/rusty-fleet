# Fixtures

Milestone 0 includes synthetic valid and damaged contract fixtures under
`contracts/`. `scenarios/scale-and-damage.v1.json` pins the deterministic
simulator seed, representative dataset sizes, and damage families exercised
during validation. Large generated datasets are not committed.

The simulator covers replay, reordering, staleness, offline projection,
capability downgrade, partial families, malformed messages, and multi-device
check-in. Its fixed four-device M1 lifecycle profile additionally exercises
sleep/wake aging, route loss/recovery, duplicate and stale check-ins, agent
upgrade with a fresh source epoch, and old-epoch replay. The exact pinned
Manifold adapter separately proves that key rotation rejects the old signer
and accepts the replacement only with a fresh source epoch. The saved-view
pair covers exact canonical-query/navigation restoration and fail-closed
bounds, duplication, density, and schema-version damage. Fixture size is not
a supported-scale claim.

The Kiosk show-controls pair binds one immutable Fleet target preview to
per-target preflight and ledger state. Its valid case accepts only a verified
Kiosk-owned effective receipt; its damaged case covers owner-contract drift,
preview mutation, false application, unsafe retry/cancellation, and forbidden
cleanup.

Real device exports, endpoints, serials, logs, captures, and private payloads
do not belong in this directory.
