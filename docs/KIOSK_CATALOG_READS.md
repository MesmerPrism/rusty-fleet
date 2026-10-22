# Kiosk catalogue summary reads

Fleet can explicitly read a Kiosk installed-app summary through the optional,
exactly pinned QuestIonAble File Manager provider. This source integration is
experimental. It provides catalogue refresh, label/tag search and exact app
identity candidate reads. It provides no app launch, wake, file operation,
package detail or signing-detail action.

The provider metadata catalogue remains a separate, target-free description
surface. Its presence never supplies an installed-app catalogue or execution
authority.

## Configure and read

Add `kiosk_v2_catalog_provider` to the private local Hub configuration only when
this integration is wanted. Its closed fields are:

- `executable`: absolute path to the dedicated
  `questionable-file-manager-kiosk-v2-provider.exe`;
- `expected_executable_sha256`: exact lowercase SHA-256;
- `profiles`: unique mappings with `device_id`, positive `identity_revision`,
  and an opaque `profile_id` of 8–128 portable characters.

Keep the configuration and QFM profile store private. Fleet never accepts a
profile, endpoint, key, scope, lease, barrier or evidence payload from a refresh
caller. QFM owns profile resolution, paired session cryptography, endpoint
selection and verification of the owner response. Fleet stages only the exact
pinned executable in a current-user-only private directory, holds the verified
artifact through process completion, and uses its existing Windows Job Object
owner to bound the complete process tree and all standard streams. The fixed
invocation is `integration kiosk-v2-catalog --json`; stderr is never exported.

Absent configuration is inert. An unavailable configured artifact fails startup
closed. The selected device needs a current enrolled signed peer, an active
credential matching that accepted peer fingerprint, a fresh ready signed Kiosk
capability and its exact operator-configured QFM profile mapping. The encrypted
exchange establishes the authenticated Kiosk owner and scoped read. QFM's outer
Fleet device/revision fields bind the request/profile; they do not constitute
Android hardware attestation.

The operator listener must be loopback for all catalogue routes, including
cached reads after a restart with no configured provider. The independent LAN
check-in listener exposes none of these routes.

| Local API | `fleetctl` | WPF |
| --- | --- | --- |
| `POST /fleet/v1/kiosk-catalog/refresh` | `kiosk-catalog-refresh DEVICE@IDENTITY_REVISION` | Refresh selected device |
| `POST /fleet/v1/kiosk-apps/search` | `kiosk-apps-search "app name or tag"` | Search |
| `GET /fleet/v1/kiosk-apps/APP_IDENTITY/candidates` | `kiosk-apps-candidates APP_IDENTITY` | Show candidate devices |

Refresh JSON has only `schema` =
`rusty.fleet.kiosk_v2_catalog_refresh_request.v1`, `device_id` and
`identity_revision`. Search JSON has only `schema` =
`rusty.fleet.kiosk_app_search_request.v1`, `query`, and `limit` (1–128).
Names/tags use the Kiosk normalization contract. Exact opaque app identity is
preserved, including when several apps share a display label. Candidate reads
select an exact identity, never the first matching label.

## Continuity and durable evidence

Each refresh uses a random request identity, fixed `kiosk.catalog-summary`
scope, exact encrypted-v2 route and at most a 30-second evidence window.
Strict parsing retains exact owner-byte hashes and checks request, device,
identity revision, capability revision, owner epoch, profile, route, grant,
canonical base64url, time and replay bindings. Details and additional scopes
are rejected. The blocking provider call runs outside the runtime lock.
Admission samples time again under that lock and rechecks target source epoch,
credential identity/generation, capability revision/freshness and profile before
persisting. Unrelated device updates or ordinary telemetry in the same source
epoch do not invalidate an otherwise exact read.

Catalogue observation, consumed replay identity and the initial identity/profile/
owner association are committed in one Hub snapshot using the existing two-slot
clone–persist–install transaction. Failed persistence installs none of them.
Caller cancellation cannot strand the read's permit or per-device marker.
Restoration rejects damaged bindings and attempts to downgrade an authenticated
catalogue into passive evidence. Older snapshots lacking all catalogue fields
restore with empty catalogue state.

Source identity/epoch changes make the old observation unusable. Replay history
and the established association remain retained. A changed identity, QFM profile
or paired Kiosk owner is refused as setup required. **This experimental source
surface has no supported association retirement/rebind operation.** Operators
must retain the durable journal; a fresh state directory or journal deletion is
not a rebind procedure. A bounded explicit compare-and-retire setup action is
future integration work.

Catalogue freshness, grant admission expiry and Fleet device freshness are
separate facts. Search counts only fresh, complete summaries. Coverage reports
stale, absent, incomplete and permission-limited devices; cached evidence never
implies a current match. Candidate views retain observation age, completeness,
grant expiry and existing device power/application/route facts. These facts do
not establish wearer availability. WPF catalogue reads preserve the main fleet
query, filters, ordering, selection and inspector context.

## Validation

The ordinary locked workspace suite covers synthetic host evidence, exact
request rejection, restart/damage, atomic failure, cancellation/concurrency,
stale and incomplete coverage, process IO/deadlines and candidate identity.
The WPF suite checks canonical read projection and retained fleet context.

One optional cross-owner source conformance test consumes immutable neutral
request and QFM subprocess-response files through the actual Fleet parser and
admission adapter. Select both `FLEET_KIOSK_INTEROP_REQUEST` and
`FLEET_KIOSK_INTEROP_RESPONSE`, then run:

```powershell
cargo test --locked -p fleet-hub-local actual_kiosk_qfm_bytes_join_strict_fleet_parser_and_admission
```

The test checks exact fixture SHA-256 values. Its absence from an ordinary run
provides no cross-owner conformance credit. Production Kotlin producer and QFM
fetch/subprocess evidence is kept by those owners. No host fixture proves live
pairing, actual installed-device state or a headset effect.
