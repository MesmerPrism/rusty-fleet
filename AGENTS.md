# Rusty Fleet Agent Notes

Rusty Fleet is a public AGPL repository and a Hostess/operator product. Keep
committed content portable and free of local paths, device serials, credentials,
private package identities, signing or pairing material, raw logs, captures,
generated binaries, and private payload details.

## Required routing

Use:

- `rusty-morphospace-context` for repo-family ownership, workflow, and
  public/private routing;
- `system-engineering` for authority, contracts, adapters, observability,
  validation, and mitigation maps;
- `rust-work-graph` for broad inventory, dependency, instruction-surface, and
  release graph work;
- `meta-quest-workflow` only for explicit headset, ADB, APK, logcat,
  screenshot, Perfetto, Wi-Fi ADB, or Meta-tooling work.

Live device work is never implied by a source or documentation task.

## Read order

1. `README.md`
2. `morphospace/project.spec.json`
3. `morphospace/feature.lock.json`
4. `morphospace/workspace.state.json`
5. the current iteration unit, if one is named
6. `docs/IMPLEMENTATION_PLAN.md`
7. `docs/WORKFLOW.md`
8. `docs/ARCHITECTURE.md`
9. `docs/VALIDATION.md`

## Ownership

- Rusty Fleet owns product composition, the Fleet Hub, Fleet Console,
  `fleetctl`, operator policy, and fleet-level projections.
- Manifold owns accepted commands, sessions, peer state, stream references,
  replay, expiry, revocation, and audit semantics.
- Rusty Quest owns Android/Quest lifecycle, permissions, platform observation,
  packaging, and effective device receipts.
- Kiosk owns app-local kiosk actions and their effective application receipts.
- QuestIonAble File Manager owns file operations and ADB-backed device utilities.
- Rusty LSL may provide LSL-compatible observations or discovery proposals; it
  does not become fleet command or admission authority.
- Media sources, processors, route/socket providers, codecs, and sinks remain
  explicit and separate from the low-rate control plane.

UI handlers collect parameters, invoke owned routes, show progress, and project
structured evidence. Every operator action requires CLI or local API parity.

## Stacked milestone rule

The planning and acceptance unit is a coherent milestone stack, not a single
method, schema, test, or documentation edit. A normal stack contains contract,
engine, adapter, projection, negative-path, evidence, and rollback work needed
for one usable capability.

Keep corrections and follow-through inside the active stack. Split only for a
real authority boundary, an independently releasable result, a separate
device/security approval, or work that can no longer be reviewed as one
coherent change. File count and test count are not split reasons.

At most one milestone stack is active or validating. Do not manufacture a new
iteration unit merely because a focused test found a defect.

## Validation and publication

- `Quick` is the normal edit loop.
- `Standard` is the milestone integration and handoff gate.
- A live-device gate runs only when the milestone explicitly requires device
  behavior and all source/static checks already pass.
- `Deep` is for architecture, security, media, relay, promotion, release, or
  broad cross-repository consolidation.

Commit coherent internal layers, not individual files. Push a green working
branch at a meaningful recovery checkpoint and publish the milestone after its
declared Standard gate. Run Deep before a release or when the invalidation
matrix requires it. Never use a device suite to prove a docs-only edit.

Run:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-Repo.ps1 -Tier Quick
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-Repo.ps1 -Tier Standard
git diff --check
```

Use `docs/VALIDATION.md` for the invalidation matrix and
`docs/WORKFLOW.md` for checkpoint policy.

## Activation and safety

Absent means inert. Source presence, adapter registration, device discovery, or
an ADB connection does not activate a feature. Runtime effects require the
current feature lock plus an approved runtime input and an effective receipt
from the consuming owner.

Do not assume ADB. Base monitoring and participating-app control must work
through authenticated app-level networking. ADB, on-device loopback,
accessibility, device-owner, file operations, media streaming, and relay access
are separate opt-in capabilities with explicit grants and truthful degraded
states.

Keep `AGENTS.md` concise. Put detailed procedures in linked docs or runbooks and
update the nearest README/router plus relevant skills when ownership,
activation, validation, device policy, or public boundaries change.
