# Package Install/Release Source Checkpoint

## Outcome

Windows product distribution channels are separate from the Quest package
operation described here. The complete Fleet Labs product channel, Stable isolation,
and signed Pages metadata path are defined in
[Windows Labs Distribution](WINDOWS_LABS_DISTRIBUTION.md); Labs does not
reduce this operation or any other current Fleet component.

Rusty Fleet can preview and explicitly confirm one immutable
`packages.install-release` operation over exact device identity revisions. The
release reference, expected Android package identity, rollout ring, owner
contract, per-target capability facts, exclusions, and expiry are frozen in
the preview. Console, `fleetctl`, and the loopback local API project the same
operation contract.

The source stack now continues beyond `dispatch_ready` through a separately
authenticated, disabled-by-default updater-owner ingress and durable bounded
claim scheduler. The owner may report monotonic staging, wearer-prompt,
cancellation, recovery, failure, and expiry progress; operators receive an
exact count projection and may explicitly archive a hash-bound terminal
operation while preserving its complete ledger. Fleet still does not itself
download a package, approve Android PackageInstaller, or contact a headset.

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

Only independently authenticated evidence from the configured package owner
may move a target beyond `dispatch_ready`. Without the private
`package_updater_owner` configuration, the claim, acknowledgement, progress,
and receipt routes return `package_owner_authenticated_ingress_unavailable`
with HTTP 501 and do not mutate durable state. The configured owner ID is pinned to
`rusty-quest.package-updater`; the bearer secret is private runtime input and
is never stored in an operation or returned by an API.

The owner first reads the authenticated, non-mutating, bounded
`GET /fleet/v1/package-updater/offers` route. Fleet offers exactly one
immutable invocation identity and digest. The subsequent one-use claim request
must repeat the exact operation ID, device ID, and invocation SHA-256 from that
offer; stale, mismatched, and replayed requests reject before durable mutation.
The returned short-lived claim contains invocation, release, and target
digests. Active nonterminal claims and acknowledged in-flight targets count
against the operation's frozen `max_parallelism`. GET and POST use the same
pure explicit-time selection predicate: logical claim expiry releases
projection capacity without mutating durable state. POST applies durable
expiry/retry to a candidate snapshot and reruns that predicate under the lock;
a stale or mismatched offer rejects without persisting the candidate. Claims,
consumed request IDs, and their expiry are
part of the durable Hub snapshot, so restart cannot silently reopen a live
slot or erase replay evidence.

## Lifecycle

```text
preview_ready
  -> explicit operator confirmation
  -> approved
  -> exact invocation prepared
  -> dispatch_ready
  -> authenticated short-lived owner claim (still accepted)
  -> owner acknowledgement (dispatched)
  -> staged / awaiting_wearer / cancellation_requested / recovery_required
     (authenticated monotonic owner progress)
  -> exact install_commit effective receipt (Applied)
```

`dispatch_ready` and an owner claim remain `accepted`, not `dispatched`.
Preparing an invocation does not consume the owner-delivery parallelism budget,
so confirmation
prepares every eligible target instead of stranding targets beyond the first
parallelism window. A later authenticated delivery scheduler must enforce the
frozen `max_parallelism` when it actually admits owner delivery evidence.

An owner acknowledgement proves `dispatched`, but never `applied`.
Intermediate progress changes the target stage and lifecycle only after exact
owner, claim, invocation, operation, device, request, timestamp, and monotonic
transition validation. It provides operator-visible state but never substitutes
for an effective receipt.
`applied` requires the pinned Rusty Quest `install_commit` receipt for the
expected package and rollout ring. Manifest admission, staging, download, an
open installer UI, or an acknowledgement is not application proof.
The effective install result also does not prove on-device cleanup. Fleet's
terminal archive is a retention action only: it moves the complete immutable
operation from the bounded active map into a separately bounded readable
archive and records its exact SHA-256. It makes no claim about temporary files,
installer UI, package state, or owner-side cleanup.

## Operator surfaces

The local API exposes:

- `POST /fleet/v1/package-install-releases/preview`;
- `POST /fleet/v1/package-install-releases/{operation_id}/execute`;
- `GET /fleet/v1/package-install-releases/{operation_id}`;
- `GET /fleet/v1/package-install-releases/{operation_id}/progress`;
- `POST /fleet/v1/package-install-releases/{operation_id}/archive`;
- `POST /fleet/v1/package-updater/claims`;
- `GET /fleet/v1/package-updater/offers`;
- authenticated owner acknowledgement, progress, and receipt routes.

Private activation adds this object to the local Hub configuration:

```json
{
  "package_updater_owner": {
    "owner_id": "rusty-quest.package-updater",
    "bearer_token": "<private-random-token-at-least-32-bytes>"
  }
}
```

The token stays in the private config. It is not a feature-lock value, release
reference, operation argument, durable claim field, CLI argument, or response.
`fleetctl` reads the corresponding private token from
`RUSTY_FLEET_PACKAGE_OWNER_TOKEN`.

`fleetctl` exposes:

```text
package-preview manifest-url URL PACKAGE RING DEVICE@IDENTITY_REVISION...
package-preview release-id RELEASE_ID PACKAGE RING DEVICE@IDENTITY_REVISION...
package-execute OPERATION_ID PREVIEW_ID
package-get OPERATION_ID
package-progress OPERATION_ID
package-archive OPERATION_ID EXPECTED_OPERATION_SHA256
package-owner-offer
package-owner-claim OWNER_ID REQUEST_ID OPERATION_ID DEVICE_ID EXPECTED_INVOCATION_SHA256
package-owner-ack OPERATION_ID JSON
package-owner-progress OPERATION_ID JSON
package-owner-receipt OPERATION_ID JSON
```

The WPF Console continues to accept a credential-free HTTPS signed-manifest URL, expected
package identity, and rollout ring. Confirmation is labeled as preparation,
and projects the durable operation status. Its read-only target ledger validates
and distinguishes an active owner claim, accepted dispatch acknowledgement,
and the exact `install_commit` installed-version proof; it also states that
application proof is not cleanup proof. Owner authentication and evidence
submission are intentionally not activated in WPF: the owner is a machine
participant. WPF and CLI are operator-input equivalent: both refresh the same
exact count projection and can submit the exact digest-bound terminal archive
request. Neither surface manufactures owner progress or proof. Adding
human-facing cancellation/retry requests requires a later owner contract. The input lock is visible once
the preview is immutable, an accepted preparation cannot be confirmed again,
and `Close view` clears only the local Console projection; it does not cancel
or mutate the Hub operation. Batch operations are collapsed by default to
preserve the normal fleet workspace, and package inputs reflow within the
declared minimum window before the operator opens the bounded target ledger.

## Persistence and restart

Preview, prepared ledgers, and owner claims use the existing bounded two-slot
Hub state envelope. Restart restores `accepted`, `dispatch_ready`, active
claims, up to 16 prior expired unacknowledged claim/request attempts per target
as readable evidence, a separate non-truncating replay ledger of up to 64
consumed claim/request identity pairs per target, dispatched state, and
accepted receipts exactly. The replay ledger remains authoritative for the
complete invocation-validity and retained-operation lifetime. When its bound
is consumed, the target fails closed after the last live claim expires rather
than deleting an identity or becoming offerable again. An expired unacknowledged
claim releases the scheduler slot and returns the target to `dispatch_ready`;
replacement requires a fresh request ID and the current exact offer. An
acknowledgement must arrive while its claim is live. Once an accepted
acknowledgement is durable, its exactly bound receipt may arrive after claim
expiry but no later than the frozen invocation expiry. Unauthenticated,
replayed, expired, digest-mismatched, nonmonotonic, or wrong-owner submissions
do not change the stored operation.

Authenticated owner progress and terminal archives use the same two-slot
durability boundary. The active operation store fails closed at 1,000
operations. An operator may archive only a terminal operation while presenting
the exact SHA-256 returned by the fresh progress projection. Archive admission
is idempotent, retains the full operation and exact count projection, survives
restart, and reclaims one active-store slot. The archive itself fails closed at
10,000 operations; no automatic purge invents an erasure or cleanup policy.
`cleanup_required` remains false and does not claim that on-device cleanup ran
or succeeded. Release references, identifiers, request bodies, responses, and
preview lifetimes retain their declared limits.

## Validation

Focused validation covers:

- valid and damaged Rust contract fixtures;
- the versioned public JSON schema;
- exact owner binding, release/package/ring binding, and capability drift;
- two prepared targets with future owner-delivery parallelism set to one;
- CLI preview/execute/get/progress/archive and malformed-response rejection;
- local API 501 acknowledgement, progress, and receipt rejection without mutation;
- durable restart at `dispatch_ready` and with an active owner claim;
- independent owner authentication, disabled-default behavior, one-use claim
  request replay, claim expiry, and bounded parallel claim admission;
- exact operation/target/release/invocation digest binding;
- acknowledgement remains Dispatched and exact install-commit evidence gates
  Applied;
- monotonic authenticated progress admission, exact count projection, and
  rejected binding/regression paths;
- exact digest-bound terminal archival, idempotence, restart restoration,
  full-ledger preservation, and active-store capacity reclamation;
- WPF exact-release/target confirmation across the normal 50-device fixture,
  fail-closed refresh, a virtualized accessible per-target ledger, disabled
  repeat confirmation, non-color target evidence, truthful claim/dispatch/
  installed-version projections, exact progress/archive parity, damaged-receipt
  rejection, and an explicit cleanup boundary.

Repository Quick and Standard gates are device-free. A live package update
requires a separately authorized owner/device validation unit and cannot be
inferred from this source checkpoint.

This slice admits owner-reported intermediate state but does not create a
Fleet-to-owner cancellation or retry request protocol. It also does not
execute updater work, approve a wearer prompt, or infer on-device cleanup.
WPF remains read-only for owner evidence and exposes only operator-owned
refresh and terminal-retention actions.

The source-only producer ingress is an authenticated Fleet-side admission
surface. It is not a live updater transport or updater client and does not
prove unreported download progress, approve wearer prompting, execute
cancellation or retry, prove on-device cleanup, or prove installation without
the exact effective receipt.

## Rollback

Source rollback first removes or omits the private owner configuration, which
immediately prevents new claims or evidence admission without changing
monitoring or Kiosk control. A full rollback removes the owner routes and claim
projection only through a compatible state migration that retains historical
terminal evidence. Existing live claims expire; they are never converted into
Applied state by rollback. Already archived ledgers remain readable and must be
retained by any compatible migration.
