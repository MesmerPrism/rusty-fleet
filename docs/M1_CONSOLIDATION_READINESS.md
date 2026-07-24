# Milestone 1 Consolidation Readiness

## Decision

The automated host-side Milestone 1 stack is ready for consolidation review.
This is not Milestone 1 acceptance. The remaining acceptance work is
operator-attended accessibility validation followed by the declared workflow
and publication transition.

The checkpoint preserves the no-ADB product baseline. It does not promote
optional LSL, media, relay, privileged, or device capabilities.

## Exact source scope

| Owner | Revision or surface | Evidence in this checkpoint |
| --- | --- | --- |
| Rusty Fleet | current `codex/fleet-m1-local-monitoring` branch | Quick, Standard, Deep, workflow-contract, Rust, WPF, CLI/API-parity, damaged-fixture, lifecycle, and repository-boundary gates |
| Rusty Quest | [`8ec9442375355a3202b0bcaa90ab94820f2ec5ac`](https://github.com/MesmerPrism/rusty-quest/commit/8ec9442375355a3202b0bcaa90ab94820f2ec5ac) | explicit fail-closed `Host` tier, Fleet Agent contract tests, Android host bridge tests, and static packaging/activation checks |
| Rusty Manifold | exact Fleet-pinned dependency | enrollment/status admission, signer rotation, revision, replay, and old-signer rejection exercised through Fleet tests |
| Rusty LSL | no promoted runtime adapter | optional owner boundary remains closed and is not required for base monitoring |

The Quest owner command is:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File <rusty-quest-root>\tools\Test-FleetAgentAndroid.ps1 `
  -Tier Host
```

`Host` is an explicit declared tier. Unknown tier names fail parameter
validation instead of being accepted as unused arguments. `-Build` remains a
separate opt-in package-build action; the host gate does not install, launch,
contact, or reserve a device.

## Automated evidence

| Boundary | Result | Meaning |
| --- | --- | --- |
| Fleet focused and Quick gates | pass | edited contracts, fixtures, projections, and repository invariants agree |
| Fleet Standard gate | pass | full source integration, Markdown/project consistency, local Hub scenario, and WPF test surface agree |
| Fleet Deep gate | pass | tracked-tree, authority, public-boundary, and registered cross-repository checks agree |
| Portable workflow validation | pass | the public project/workspace contracts remain internally consistent |
| Quest Fleet Agent `Host` gate | pass | the exact owner-side source/static contract agrees with Fleet's pinned golden boundary |
| Quest undeclared-tier negative path | pass | an unknown validation tier fails before tests or build work |
| Hosted Fleet CI for the preceding source checkpoint | pass | GitHub Actions run [`30063642220`](https://github.com/MesmerPrism/rusty-fleet/actions/runs/30063642220) accepted commit `09494e264d7ac9d0db440b1dd2a71cac177462ff` |

The final Fleet repository checks are rerun after this readiness record is
added. Hosted CI for the resulting documentation checkpoint remains a
publication receipt, not a substitute for the manual accessibility gate.

## Authority and safety review

- Fleet still owns the operator projection, canonical queries, saved views,
  local Hub behavior, and base-monitoring product composition.
- Manifold still owns admitted identity, revisions, replay, signer rotation,
  expiry, and revocation.
- Rusty Quest still owns Android lifecycle, platform observations, packaging,
  opt-in activation, and effective device receipts.
- A transport write, process exit, or signature check is not treated as an
  effective device or application receipt.
- No runtime listener, device route, package install, permission, ADB path,
  media decoder, relay, or LSL discovery adapter is activated by this
  checkpoint.
- No raw private device evidence, local paths, credentials, or participant
  data enters this public repository.

## Instruction-impact review

The repository routing and public/private rules remain sufficient:

- `rusty-morphospace-context`: no change required;
- `system-engineering`: no change required;
- `rust-work-graph`: no change required;
- `meta-quest-workflow`: no change required;
- Rusty Fleet and Rusty Quest first-hop instructions now name the exact Quest
  `Host` gate where it is actionable.

No installed skill or shared work-environment contract needs to change for
this source checkpoint.

## Remaining acceptance gates

The following work is intentionally not automated or inferred:

1. Complete the declared Narrator workflow over search, fleet navigation,
   inspection, batch selection, detail navigation, and return-to-fleet.
2. Verify Windows high-contrast modes, large text, supported display scaling,
   focus visibility, clipping, and stable keyboard restoration.
3. Preserve sanitized accessibility evidence without changing global settings
   outside a dedicated reversible operator-attended plan.
4. Run the formal workflow validation/acceptance transition against the exact
   final Fleet and Quest revisions, then publish the accepted planning state
   last.

Optional Rusty LSL runtime/discovery/clock/recovery/XDF support remains
deferred until its owner repository supplies and promotes that exact contract.
It is not a blocker for the no-ADB base-monitoring milestone and must not be
simulated into a support claim.

## Next coherent slice

The next slice is the operator-attended accessibility gate and formal
workflow transition. Additional source microsteps are not justified unless
that gate finds a specific defect. Device, LSL, media, and relay work remain
separate owner-qualified milestones.
