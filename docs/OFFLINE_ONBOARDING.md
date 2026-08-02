# Offline Fleet Onboarding

`fleet-onboard` is a standalone Windows operator tool for generating a new,
private Fleet Agent configuration bundle while offline. It is deliberately
outside Fleet Hub, `fleetctl`, the local API, and the WPF Console.

## Authority and scope

Fleet owns the generation plan, private layout, Hub enrollment projection,
inventory, cleanup plan, and sanitized receipts. Rusty Quest owns the
`rusty.quest.fleet_agent_profile.v1` profile and
`fleet-agent-key-record` semantics. Manifold remains the enrollment and
revocation authority.

Generation does not install a profile, enroll a peer, contact or start a Hub,
discover or touch a device, or prove that a device is reachable, active, or
healthy. Seed deletion is local cleanup, not secure erasure or authorization
revocation.

The only input is an operator-created private request JSON. Copy the field
shape from
[`fixtures/onboarding/offline-onboarding-request.example.json`](../fixtures/onboarding/offline-onboarding-request.example.json)
into an ignored, current-user-only location and replace every placeholder.
Never commit the completed request or generated output.

The request, its output parent, the tool manifest, and the tool executable must
be on a standard local fixed or RAM-disk drive. UNC paths, alternate Windows
namespaces, reparse ancestors, removable media, and network shares fail closed.
Alternate data streams, reserved device names, and trailing-dot or
trailing-space aliases also fail closed.
The request and existing output parent must grant full control to exactly the
current Windows user, with inheritance removed. The output root must not exist.

## Commands

```powershell
cargo run --locked -p fleet-onboard -- validate-tool --request <private-request.json>
cargo run --locked -p fleet-onboard -- plan --request <private-request.json>
cargo run --locked -p fleet-onboard -- apply --request <private-request.json> --confirm-plan-sha256 <digest> --non-interactive
cargo run --locked -p fleet-onboard -- cleanup-plan --inventory <private-inventory.json>
cargo run --locked -p fleet-onboard -- cleanup-apply --inventory <private-inventory.json> --confirm-cleanup-sha256 <digest>
cargo run --locked -p fleet-onboard -- revoke-plan --inventory <private-inventory.json>
```

These six commands are the complete standalone CLI authority surface. There
are no generic tool arguments, environment overrides, supplied seeds,
tool-derived endpoints, serials, ADB routes, pairing values, overwrite,
resume, force, or installation switches.

## Planning and owner-release trust evidence

`validate-tool` and `plan` are read-only. They do not create the output root,
temporary directories, or seeds; execute the key tool; alter ACLs; contact or
start Hub; or interact with a device. The canonical plan binds the request,
output list, pinned Quest tool and owner-profile evidence, and explicit
non-claims. `apply` rebuilds that plan and requires its exact SHA-256.

The request locates an owner-issued release manifest; it does not choose the
trust anchor. Fleet pins the consumer, owner identity, manifest, artifact,
source and provenance independently in
`config/fleet-agent-key-record-owner-release.v1.json`. The supported Rusty
Quest `1.0.0` capsule is portable and contains exactly the helper executable,
release manifest, public provenance, project license, source notice and
checksums. Fleet preserves those owner bytes without augmentation when it
packages the inert helper component.

The owner release accepts exactly these six files:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `LICENSE` | 34,523 | `0d96a4ff68ad6d4b6f1f30f713b18d5184912ba8dd389f86aa7710db079abcb0` |
| `SOURCE-NOTICE.md` | 427 | `7a95b2704991263057c12f75efae64cd2c38bf35e20fceca7bc42884e69e698a` |
| `checksums.sha256` | 420 | `aae77f56355cb6129b13dbe20850fb08c01a7a4cba9a17a8d97aee84490f407b` |
| `fleet-agent-key-record.exe` | 219,648 | `6e3962726be67cf42d0fdc2dbf3792f7d665524323e5615f1907002518bfe3d7` |
| `provenance.json` | 4,596 | `dba7ba306b3f0839db54d1e965f71a1939f82b413fa72cf7d883788a7ba41676` |
| `release-manifest.json` | 1,835 | `d96baf6f3cdd5af9d79d0d98df5fd96e5ee9f689350a1d415c8a88fac101e457` |

It also accepts only:

- Rusty Quest source commit
  `cebdf368d9a2f1d2c12f9566f937f51bd5f29945`;
- exact source tree `1d23419ff6e95289b804d86ccc5a5cd66fd27afc`,
  composition fingerprint, package, public source-file hashes and the closed
  Rusty Fleet/Rusty Manifold/Rusty Quest provenance set;
- consumer identity `rusty-fleet/fleet-onboard` and owner identity
  `rusty-quest`.

Unknown manifest fields, extra or missing repositories, dirty repository
claims, source/provenance drift, executable drift, wrong consumer/owner,
duplicate capsule files, secret markers, or path substitution fail closed.
The supported helper is independently required to be a reproducible x64 PE:
its provenance binds isolated Git materializations, post-build identity
verification, source-path remapping, stripped symbols, `/Brepro`, and an exact
`IMAGE_DEBUG_TYPE_REPRO` marker. Fleet scans the executable itself for that PE
marker and for machine-local ASCII and UTF-16 paths; provenance text alone is
not sufficient.
Opened manifest and executable handles are bound by volume/file identity,
link count, ACL digest, length, and content digest. The exact non-inheritable
executable handle and every mutable ancestor directory remain retained while
the tool runs. Ancestor handles deny delete sharing, the complete path identity
chain is checked immediately before and after process creation, and executable
bytes are rehashed after every invocation.

Rusty Quest currently has no key-record-capsule signature or revocation
authority. Fleet therefore records `owner_signature.present=false` instead of
inventing a signature. Exact public repository identity, clean source
commit/tree, closed provenance and independently pinned SHA-256 values bind the
owner release. A signed Fleet bundle authenticates Fleet packaging only and is
not relabeled as a Rusty Quest signature.

Capsule validity is packaging and helper provenance only. It does not prove
onboarding acceptance, installation, activation, reachability, a lease, or
peer acceptance. Manifold remains the live enrollment and peer authority.
`onboarding_ready=true` means only that the complete pinned helper capsule is
present and valid in the bundle; live onboarding remains separately gated.

## Contained key derivation

The guarded tool is invoked with one fixed argument vector:

```text
<pinned-executable> --key-id <validated-dotted-id> --seed-file <new-private-seed-file>
```

The environment is cleared, stdin is closed, and the process is contained in
a Windows Job Object. Sensitive manifest, executable, seed, and inventory
handles are created non-inheritable; the security suite passes a retained file
handle's exact identity as a sentinel and confirms the child cannot access it.
No seed bytes are placed in arguments or environment—the command contains only
a validated key ID and private seed-file path. Stdout
and stderr are drained concurrently into zeroizing buffers with independent
16 KiB caps under one ten-second deadline that includes parent exit and pipe
EOF. The entire job is terminated before the result is accepted, so
descendants cannot outlive a successful or failed invocation. Stderr, unknown
JSON fields, schema/key mismatch, malformed key bytes, and fingerprint
mismatch fail closed without echoing private paths or child output.

## Private output transaction

Apply creates one new, non-reparse root beneath the already-private parent and
immediately assigns a non-inherited, current-user-only ACL. Every directory
and file uses create-new, non-following component access; there is no overwrite
or resume. Exactly 32 CSPRNG bytes are generated for each device. Only
zeroizing seed buffers and seed digests are retained, and a same-run collision
fails closed. Public-key bytes are independently fingerprinted, and a
cross-device duplicate public key fails closed even when the seeds differ.

The layout is:

```text
onboarding.private-inventory.json
hub/enrollment.private-config.json
devices/device-<sha256-of-device-id>/fleet-agent.seed
devices/device-<sha256-of-device-id>/fleet-agent.profile.json
devices/device-<sha256-of-device-id>/fleet-agent.public-key-record.json
```

Raw device IDs never become path components. Case-insensitive ID collisions
are rejected before planning. Hub endpoints must exactly match the requested
check-in listener. The generated Hub JSON is deserialized and validated by the
real `fleet-hub-local` offline validator before the same typed value is
serialized. The Quest profile uses a deny-unknown-fields Fleet mirror pinned
to the owner source and canonical-LF UTF-8 fixture hashes; checkout line-ending
materialization is not authority. The committed owner fixture is a portable
shape-conformance check, not a claim that Fleet owns that schema.

The inventory is written last as the transaction commit marker. It binds the
root and every expected object by relative path, object kind, volume/file
identity, link count, ACL digest, and file SHA-256. The exact expected
enumeration is closed: extra, missing, reparse, hard-linked, or
case-colliding entries fail validation.

Before the inventory commit, rollback uses only the in-memory ledger of
retained handles for objects created by the current run. It never recursively
deletes a path. If exact rollback cannot be proven complete, apply returns
`partial_generation_exact_cleanup_required` and preserves uncertain material
for operator review.

Non-Windows apply and cleanup return their platform error before loading a
request, creating output, or generating a secret. Read-only schema helpers do
not weaken that mutation boundary.

## Cleanup and revocation

`cleanup-plan` opens the inventory once and retains it while verifying the
complete closed layout. It produces a canonical digest bound to the inventory
identity, root identity, and exact entries without mutation. `cleanup-apply`
requires that exact digest, repeats validation, and marks only those retained,
identity-validated handles for deletion. It never traverses an unbound path or
recursively deletes a directory.

The cleanup receipt reports logical Windows deletion. It does not claim secure
media erasure, backup removal, seed revocation, or authorization revocation.

`revoke-plan` never mutates Hub. It emits a fail-closed plan naming the
Manifold enrollment authority and binding the private inventory. The operator
must use the authorization owner's separately approved workflow and retain
its owner-issued receipt.

## Threat and mitigation map

| Threat | Mitigation |
| --- | --- |
| altered or substituted tool | exact owner identity/manifest/artifact/source/provenance pins, closed provenance set, retained non-inheritable executable and deny-rename ancestor chain, before/after path-identity checks, post-invocation rehash |
| capsule mistaken for live authority | explicit packaging-only claim; onboarding/activation/reachability/lease/peer non-claims; Manifold remains live authority |
| absent owner signature authority | explicit `owner_signature.present=false`; exact Git/source/hash binding; no simulated signature or revocation |
| network/share or link redirection | standard local-drive requirement, component-wise non-following opens, reparse and namespace rejection |
| request/tool argument injection | strict JSON structs, dotted-ID validation, fixed argument vector, cleared environment |
| inherited disclosure | private request and parent prerequisite, immediate current-user-only root ACL, inherited private children |
| seed/key collision or disclosure | OS CSPRNG, zeroizing buffers/digests and captured child bytes, public-key recomputation/deduplication, no seed in argv, receipts, or Hub configuration |
| partial generation | create-new entries, inventory last, exact retained-handle rollback ledger |
| malicious or noisy child | Windows Job Object, concurrent bounded drains, one deadline, strict key-record validation |
| cleanup substitution | one retained inventory, closed enumeration, identity/link/ACL/hash binding, exact-handle deletion |
| accidental authority claim | explicit generated-only state and distinct cleanup/revoke plans |

Residual caveat: the generator prepares private files for a later owner
workflow. Secure transfer, device-private installation, Hub startup,
enrollment acceptance, media sanitization, backup cleanup, and authorization
revocation remain outside this tool.
