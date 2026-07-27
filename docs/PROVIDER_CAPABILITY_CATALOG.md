# Provider Capability Catalog

Rusty Fleet can show which pinned local owner providers describe a compatible
typed surface. The catalog is inert metadata. A valid row is not an execution
grant, target binding, backend health result, authority decision, approval, or
owner receipt.

## Contract and provenance

Fleet implements the structural and semantic requirements of
`rusty.quest.workflow.provider_capability_discovery.v1` from exact public
Meta Quest workflow commit
`fc476166f9c05f941dff7e9183f5c893426c05ca` and tree
`dbb7d894e60626f48ba51f88bdecff7429c9997e`. The source pin is compiled into
the catalog projection. Fleet does not require another repository path at
runtime and does not re-own the upstream schema.

The first compatible owner surfaces were reviewed at:

- QuestIonAble File Manager commit
  `f1e1f8fb19cca90cab54478026edc5bbcd978c79`;
- Rusty Hostess commit `de4122105e1c965f0ed6a8dbaec7a0eda5009f6b`.

Those revisions are source provenance, not runtime installation claims. Each
machine-local entry separately pins the exact provider ID and version,
capabilities, contract versions, action kinds and authentication requirements,
effect owner, receipt schema, absolute executable, and lowercase SHA-256.
Paths and digests remain in the private Hub config and are never returned by
the catalog API.

## Refresh model

`GET /fleet/v1/provider-catalog` reads the current Hub-owned snapshot and
never launches a process. An operator explicitly refreshes through:

```powershell
fleetctl provider-catalog-refresh
fleetctl provider-catalog
```

The Console's **Refresh metadata** button uses the same refresh route and
projection. One refresh is admitted at a time. At most eight configured slots
run in parallel, each with a bounded deadline and independent bounded
stdout/stderr drains. A refresh starts by invalidating the prior snapshot; a
failed, saturated, stale, future, or clock-rollback refresh cannot leave a
prior descriptor displayed as valid.

Fleet hashes and identifies the configured regular file before launch,
executes only the canonical local non-reparse artifact with the exact single
argument `--describe-json`, clears the child environment to a minimal Windows
bootstrap allowlist, supplies no stdin, and revalidates the artifact identity
and digest after the process closes. Description output must be one bounded
UTF-8 JSON document with empty stderr and exit code zero.

The stable entry states are:

- `unconfigured`: the slot has no executable/hash pair;
- `valid`: the exact descriptor and all expected bindings passed;
- `stale`: the descriptor expired;
- `rejected`: config, artifact, structure, semantics, or binding failed;
- `unavailable`: the pinned artifact or bounded description process was not
  available.

Even `valid` carries `metadata_only=true` and
`authorizes_execution=false`. Fleet never derives a provider invocation,
target, credential, health check, operation approval, or execution request
from descriptor content.

## Private configuration shape

The following illustrates fields only. Real absolute paths, hashes, versions,
and owner bindings belong in a private config outside the repository.

```json
{
  "provider_catalog": [
    {
      "catalog_id": "quest-connectivity",
      "executable_path": "<absolute-private-provider-path>",
      "executable_sha256": "<lowercase-sha256>",
      "expected_provider_id": "questionable-file-manager.quest-connectivity-provider",
      "expected_provider_version": "<exact-semver>",
      "expected_capabilities": [
        {
          "id": "questionable-file-manager.quest-connectivity.wireless-adb",
          "contract_versions": [
            "questionable.file_manager.quest_connectivity.provider_request.v1"
          ],
          "actions": [
            {
              "id": "request_wireless_adb",
              "kind": "effect",
              "authentication_requirements": [
                "process-access-control",
                "caller-authority-external",
                "exact-target-binding",
                "current-identity-revision",
                "effect-owner-profile",
                "owner-session-grant",
                "wearer-approval"
              ]
            }
          ],
          "effect_owner": "rusty-kiosk.wireless-adb",
          "receipt_schema": "questionable.file_manager.quest_connectivity.provider_receipt.v1"
        }
      ]
    }
  ]
}
```

## Split device check-in ingress

Local execution providers keep the operator/API listener on loopback. Fleet
Agent check-ins can use a distinct optional listener configured with one exact
non-loopback unicast interface address:

```json
{
  "bind": "127.0.0.1:8741",
  "allow_non_loopback": false,
  "checkin_bind": "192.0.2.10:8742",
  "allow_non_loopback_checkin": true
}
```

Use a real private interface address, not the documentation address above.
The two sockets must be distinct. Wildcard, loopback, multicast, and broadcast
check-in addresses are rejected. The LAN router mounts exactly authenticated,
bounded `POST /fleet/v1/checkins`; operator, catalog, query, owner, package,
device-operation, saved-view, and health routes do not exist there.

The LAN listener shares the same signed Manifold admission, replay checks, Hub
state, persistence-before-ack behavior, and damaged-slot recovery as loopback
check-ins, but has an independent smaller concurrency budget. It is disabled
by default. Signed check-ins authenticate integrity and identity; they do not
provide confidentiality. This source checkpoint does not add TLS, so deploy
the listener only on a trusted private network or behind a separately managed
confidential transport.

## Generic icon

The canonical replaceable mark is
`assets/branding/rusty-fleet.svg`. `tools/New-FleetIcon.ps1 -Write` derives the
16, 32, 48, and 256 pixel Windows ICO deterministically; running the script
without `-Write` rejects drift. The WPF project uses the generated ICO as its
application resource and window icon. The mark is deliberately generic and
may be replaced later by updating the canonical SVG and deterministic
generator together.
