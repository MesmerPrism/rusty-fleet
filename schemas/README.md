# Schemas

Milestone 0 defines versioned Rusty Fleet product contracts here:

- device observations;
- canonical fleet queries;
- stream descriptors;
- operator projection envelopes;
- revisioned saved-view collections, mutations, and navigation restoration;
- operation ledgers;
- the Fleet-owned `kiosk.show-controls` preview, per-target ledger, and
  Kiosk-effective-receipt wrapper.

Rust validation remains normative for cross-field invariants that JSON Schema
cannot express clearly, including identity/source-epoch/revision transitions,
source-selection
cardinality, component-epoch continuity, timing transforms, per-edge bounds,
and operation lifecycle.

The Kiosk operation schema pins the owner-issued direct-link v1 method,
targets, authentication vocabulary, `show-controls` command, and
`rusty.kiosk.cli_result.v1` readback fields. It does not copy or replace the
owner schema.

Do not copy Manifold, Quest, Kiosk, File Manager, or LSL owner schemas into
this directory. Reference owner-issued artifacts or wrap them with a separately
named product projection that preserves provenance and authority.
