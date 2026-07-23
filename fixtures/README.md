# Fixtures

Milestone 0 includes synthetic valid and damaged contract fixtures under
`contracts/`. `scenarios/scale-and-damage.v1.json` pins the deterministic
simulator seed, representative dataset sizes, and damage families exercised
during validation. Large generated datasets are not committed.

The simulator covers replay, reordering, staleness, offline projection,
capability downgrade, partial families, malformed messages, and multi-device
check-in. Fixture size is not a supported-scale claim.

Real device exports, endpoints, serials, logs, captures, and private payloads
do not belong in this directory.
