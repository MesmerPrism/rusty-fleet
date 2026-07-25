# Package Install/Release Source Checkpoint

## Outcome

Rusty Fleet can preview and explicitly confirm one immutable
`packages.install-release` operation over exact device identity revisions. The
release reference, expected Android package identity, rollout ring, owner
contract, per-target capability facts, exclusions, and expiry are frozen in
the preview. Console, `fleetctl`, and the loopback local API project the same
operation contract.

This checkpoint stops at `dispatch_ready`. It does not deliver the invocation
to a headset, approve Android PackageInstaller, or claim a package was
installed.

## Authority boundary

Fleet owns:

- exact target selection, preflight, immutable preview, and confirmation;
- Manifold command authorization;
- durable preparation of one exact updater invocation per eligible target;
- batch projection, future delivery parallelism, and operator evidence.

Rusty Quest owns:

- signed-manifest verification and download behavior;
- Android package and signer checks;
- attended PackageInstaller launch and wearer confirmation;
- cancellation, installed-version readback, checkpointing, and effective
  receipts.

Only authenticated raw evidence from the package owner may move a target from
`dispatch_ready` to `dispatched`, `running`, or `applied`. No authenticated
owner ingress is implemented in this checkpoint. The acknowledgement and
receipt HTTP routes therefore return
`package_owner_authenticated_ingress_unavailable` with HTTP 501 and do not
mutate durable state.

The adapter's untrusted-evidence helpers validate syntax and binding only.
They are not an admission capability. Hub transition helpers for owner-only
states exist only in the Hub test build.

## Lifecycle

```text
preview_ready
  -> explicit operator confirmation
  -> approved
  -> exact invocation prepared
  -> dispatch_ready
```

`dispatch_ready` remains `accepted`, not `dispatched`. Preparing an invocation
does not consume the future owner-delivery parallelism budget, so confirmation
prepares every eligible target instead of stranding targets beyond the first
parallelism window. A later authenticated delivery scheduler must enforce the
frozen `max_parallelism` when it actually admits owner delivery evidence.

An owner acknowledgement could prove `dispatched`, but never `applied`.
`applied` requires the pinned Rusty Quest `install_commit` receipt for the
expected package and rollout ring. Manifest admission, staging, download, an
open installer UI, or an acknowledgement is not application proof.

## Operator surfaces

The local API exposes:

- `POST /fleet/v1/package-install-releases/preview`;
- `POST /fleet/v1/package-install-releases/{operation_id}/execute`;
- `GET /fleet/v1/package-install-releases/{operation_id}`;
- owner acknowledgement and receipt routes that fail closed with HTTP 501.

`fleetctl` exposes:

```text
package-preview manifest-url URL PACKAGE RING DEVICE@IDENTITY_REVISION...
package-preview release-id RELEASE_ID PACKAGE RING DEVICE@IDENTITY_REVISION...
package-execute OPERATION_ID PREVIEW_ID
package-get OPERATION_ID
```

The WPF Console accepts a credential-free HTTPS signed-manifest URL, expected
package identity, and rollout ring. Confirmation is labeled as preparation,
and the live status explicitly states that owner ingress is unavailable and
that no package was dispatched or installed.

## Persistence and restart

Preview and prepared ledgers use the existing bounded two-slot Hub state
envelope. Restart restores `accepted` / `dispatch_ready` exactly. Untrusted
acknowledgement or receipt submissions do not change the stored operation
before or after restart.

The operation store remains bounded. Release references, identifiers, request
bodies, responses, and preview lifetimes retain their declared limits.

## Validation

Focused validation covers:

- valid and damaged Rust contract fixtures;
- the versioned public JSON schema;
- exact owner binding, release/package/ring binding, and capability drift;
- two prepared targets with future owner-delivery parallelism set to one;
- CLI preview/execute/get and malformed-response rejection;
- local API 501 acknowledgement and receipt rejection without mutation;
- durable restart at `dispatch_ready`;
- WPF exact-release/target confirmation, fail-closed refresh, accessible
  non-color target evidence, and explicit no-dispatch/no-install language.

Repository Quick and Standard gates are device-free. A live package update
requires a separately authorized owner/device validation unit and cannot be
inferred from this source checkpoint.

## Rollback

Source rollback removes the package routes and projections together, removes
the package adapter and operation store field through an explicit compatible
state migration, and leaves monitoring and Kiosk control unchanged. Disabling
or omitting any future authenticated owner-ingress configuration must keep all
package operations at or before `dispatch_ready`.
