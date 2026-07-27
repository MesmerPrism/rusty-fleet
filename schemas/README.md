# Schemas

Milestone 0 defines versioned Rusty Fleet product contracts here:

- device observations;
- canonical fleet queries;
- stream descriptors;
- operator projection envelopes;
- revisioned saved-view collections, mutations, and navigation restoration;
- operation ledgers;
- the Fleet-owned `kiosk.show-controls` preview, per-target ledger, and
  Kiosk-effective-receipt wrapper;
- the Fleet-owned `packages.install-release` immutable signed-release preview,
  per-target preparation ledger, pinned attended-updater owner contract, and
  explicit owner-evidence fields.

Rust validation remains normative for cross-field invariants that JSON Schema
cannot express clearly, including identity/source-epoch/revision transitions,
source-selection
cardinality, component-epoch continuity, timing transforms, per-edge bounds,
and operation lifecycle.

The Kiosk operation schema pins the owner-issued direct-link v1 method,
targets, authentication vocabulary, `show-controls` command, and
`rusty.kiosk.cli_result.v1` readback fields. It does not copy or replace the
owner schema.

The package operation and claim schemas pin the Rusty Quest attended-updater owner,
manifest-envelope and receipt schema names, expected package/ring, and exact
target identities. `dispatch_ready` means only that Fleet durably prepared the
owner invocation. A separately authenticated, disabled-by-default owner route
issues short-lived one-use claims with exact invocation/release/target digests.
The authenticated offer schema exposes only the next exact operation, device,
and immutable invocation digest; the claim request must repeat all three.
Owner acknowledgement and effective receipt objects remain untrusted evidence
until the claim and every frozen binding validate.
The operation keeps at most 16 full prior claims as readable evidence and
separately retains up to 64 consumed claim/request identity pairs as
non-truncating replay authority. Exhausting that authority fails the target
closed.

Do not copy Manifold, Quest, Kiosk, File Manager, or LSL owner schemas into
this directory. Reference owner-issued artifacts or wrap them with a separately
named product projection that preserves provenance and authority.
