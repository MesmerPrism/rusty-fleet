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

## Planning and quarantined trust evidence

`validate-tool` and `plan` are read-only. They do not create the output root,
temporary directories, or seeds; execute the key tool; alter ACLs; contact or
start Hub; or interact with a device. The canonical plan binds the request,
output list, pinned Quest tool and owner-profile evidence, and explicit
non-claims. `apply` rebuilds that plan and requires its exact SHA-256.

The request locates a repository-issued manifest; it does not choose the trust
anchor. The currently pinned capsule is machine-bound developer evidence for
this source checkpoint. It is not a portable or supported distribution
artifact and must not be copied, repackaged, or advertised as one. Its absolute
developer-workspace provenance and executable locations are intentionally
quarantined from onboarding and release documentation.

That quarantined developer-evidence path accepts exactly:

- manifest SHA-256
  `e92c9e000246798748ccb208567f24f72398ed6bad21cc3d05fc81c38da34f56`;
- executable SHA-256
  `74b75142ba0e7a777eb8a01fbb8ceed5aeb7f9c744a8d3fb8ec2e0fc850c0431`;
- Rusty Quest source commit
  `de1444187365b785f4ef74e24ccb40b10f34982f`;
- its exact source tree, composition fingerprint, package, and three-repository
  provenance set.

Unknown manifest fields, extra or missing repositories, dirty repository
claims, source drift, executable drift, or path substitution fail closed.
Opened manifest and executable handles are bound by volume/file identity,
link count, ACL digest, length, and content digest. The exact non-inheritable
executable handle and every mutable ancestor directory remain retained while
the tool runs. Ancestor handles deny delete sharing, the complete path identity
chain is checked immediately before and after process creation, and executable
bytes are rehashed after every invocation.

A supported distribution requires a separately owner-issued release capsule
whose portable provenance, release artifact, and installation contract are
reviewed and pinned by a later Fleet change. The current developer capsule and
pins do not satisfy that gate. Distribution work must remain blocked until the
owner release capsule exists and Fleet validates that separate contract.

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
to the owner source and fixture hashes; the committed owner fixture is a
portable shape-conformance check, not a claim that Fleet owns that schema.

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
| altered or substituted tool | exact developer-evidence pins, closed provenance set, retained non-inheritable executable and deny-rename ancestor chain, before/after path-identity checks, post-invocation rehash |
| developer capsule mistaken for a release | explicit machine-bound quarantine; supported distribution blocked on a separately owner-issued release capsule and later Fleet pin |
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
