# Schemas

Milestone 0 defines versioned Rusty Fleet product contracts here:

- device observations;
- canonical fleet queries;
- stream descriptors;
- operator projection envelopes;
- operation ledgers.

Rust validation remains normative for cross-field invariants that JSON Schema
cannot express clearly, including identity/source-epoch/revision transitions,
source-selection
cardinality, component-epoch continuity, timing transforms, per-edge bounds,
and operation lifecycle.

Do not copy Manifold, Quest, Kiosk, File Manager, or LSL owner schemas into
this directory. Reference owner-issued artifacts or wrap them with a separately
named product projection that preserves provenance and authority.
