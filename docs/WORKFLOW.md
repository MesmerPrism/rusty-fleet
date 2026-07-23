# Stacked Milestone Workflow

## Decision

Rusty Fleet adopts the Morphospace Quick/Standard/Deep validation model while
making the milestone stack—not the micro-edit—the unit of planning,
acceptance, and publication.

This keeps work resumable and auditable without paying workflow, validation,
and GitHub overhead after every small correction.

## The unit of work

A milestone stack is the smallest implementation envelope expected to produce
one operator-visible or architecture-complete capability. It normally includes:

- contract and damaged fixtures;
- deterministic engine behavior;
- one adapter or simulator route;
- CLI/local API and, when in scope, WPF projection;
- integration and negative-path tests;
- diagnostics, cleanup, rollback, and documentation;
- one acceptance receipt.

Internal checklist items do not receive separate iteration-unit IDs.

## When a split is justified

Split a milestone only when at least one of these is true:

- the work crosses a genuine authority owner and either side is independently
  useful and releasable;
- a separate security, device, privacy, or external review is required before
  the remainder may begin;
- the slice can ship or be consumed independently with a stable contract;
- the active envelope has expanded beyond what one reviewer can understand and
  validate as one coherent capability;
- a blocker makes an independent branch valuable while preserving one current
  authority in workspace state.

Do not split because a file is large, a test failed, a helper is needed, the
implementation spans several commits, or a checklist item took longer than
expected. Repair and follow-through stay in the active stack.

## WIP limit

At most one milestone stack is `active` or `validating`.

Research notes for later milestones are allowed, but they must not silently
activate source work, mutate other repositories, or become competing current
units.

## Validation cadence

| Moment | Required work | Typical cost | Git/GitHub action |
| --- | --- | --- | --- |
| Edit loop | nearest formatter/parser/unit/fixture check | seconds to a few minutes | none |
| Coherent internal layer | focused tests plus `Quick` | short | local commit |
| Meaningful recovery checkpoint | `Quick`, secret/public-boundary scan, branch is buildable or clearly marked WIP | short | push working branch |
| Milestone integration | all affected component tests plus `Standard` | moderate | handoff/PR and milestone acceptance |
| Device checkpoint | source/static/build gates first, then one bounded serial-scoped suite | expensive | attach sanitized receipt to milestone |
| Architecture/security/media/relay checkpoint | `Deep` plus named specialist gates | expensive | integration or release candidate |
| Release | Deep, affected owner full checks, device gates, graph, rollback, exact Git readback | highest | tag/release/publication accounting |

### Edit loop

Run only the closest useful check. Examples:

- Markdown/link check for documentation;
- schema parse and focused fixture test for a contract;
- one component test project for a Hub change;
- one adapter conformance scenario for an adapter change.

Do not run a workspace-wide, device, performance, or release suite to validate
an edit that cannot affect those surfaces.

### Coherent internal layer

An internal layer is a reviewable result such as:

- contract plus fixtures and conformance tests;
- state-engine transition family plus negative paths;
- adapter plus its effective receipt;
- CLI/API projection plus parity tests;
- WPF projection over an already accepted route.

Run the focused checks and `Quick`, then commit the layer. A layer may span
many files. Do not commit one file at a time merely to create activity.

### Meaningful recovery checkpoint

For a multi-session milestone, push a green working branch:

- at the end of a substantial work session;
- before a risky refactor or cross-repository integration;
- when another contributor needs the current coherent layer.

The push is a recoverable checkpoint, not acceptance. If the branch is
intentionally incomplete, name and describe it as WIP. Never push secrets,
private evidence, or a knowingly broken public boundary for backup.

### Milestone integration

Run `Standard` after the full vertical slice is assembled. Include every
affected repository's owner checks, contract fixtures, CLI/API parity,
public-boundary checks, and the milestone scenario suite.

A small repair after a failure reruns its nearest failed check first. Once the
repair is stable, rerun the aggregate Standard gate once. Do not repeatedly run
the full suite after each edit.

### Device checkpoint

Device work is an explicit gate, not a validation tier. It runs only when:

- the milestone requires real platform behavior;
- source, static, profile, build, and simulator checks pass;
- the exact package/profile/device transaction is defined;
- cleanup and fatal evidence can be captured.

Use the Meta Quest workflow for the run. A source-only milestone cannot claim
device acceptance.

### Deep checkpoint

Run Deep when a change affects:

- authority or trust boundaries;
- enrollment, relay, identity, keys, roles, replay, or revocation;
- high-rate media or performance behavior;
- persistence format or migrations;
- multiple independently published repositories;
- module promotion or public/private boundaries;
- a release candidate.

Deep should not be a default pre-commit hook.

## Invalidation matrix

| Change | Focused | Quick | Standard | Device | Deep |
| --- | ---: | ---: | ---: | ---: | ---: |
| Typo or prose-only clarification | yes | yes | no | no | no |
| Planning/authority/validation docs | yes | yes | yes before handoff | no | only if boundary changes |
| Schema or deterministic model | yes | yes | milestone end | no | if public/authority contract changes |
| Hub state engine | yes | yes | milestone end | no | for persistence/scale/security |
| Console-only projection | yes | yes | parity gate | no | no |
| Quest adapter source | yes | yes | owner checks | only at milestone gate | for release/security |
| Kiosk/File Manager adapter | yes | yes | all affected owners | only if behavior requires | for cross-repo release |
| Media route | yes | yes | integration | selected performance/device suite | yes |
| Relay/security | yes | yes | integration | when device route is involved | yes |
| Release metadata | yes | yes | yes | selected release suite | yes |

If a change crosses rows, use the highest applicable gate once at the coherent
checkpoint.

## Commit and publication policy

- Commit coherent internal layers with passing focused and Quick checks.
- Keep fixups for the current layer together before review; do not preserve
  meaningless intermediate breakage as permanent history.
- Push working branches at meaningful recovery/collaboration checkpoints.
- Open or update the milestone handoff only when Standard is green, unless it
  is explicitly marked draft/WIP.
- Merge/publish a milestone as a coherent capability, not as a sequence of
  lifecycle-only commits.
- Batch several accepted milestones only when they share one integration or
  release boundary; do not delay local commits.
- Run Deep before the declared integration/release push, not before every
  working-branch push.
- Never force-push a protected publication path.

## Failure and blocker handling

A failed test normally stays inside the current milestone:

1. preserve the failure evidence needed to understand the defect;
2. run the nearest focused diagnostic;
3. repair within the existing scope;
4. rerun that focused check;
5. rerun the aggregate gate once before handoff.

Create a corrective milestone only when an already accepted/published baseline
must change, the fix crosses a new authority boundary, or the required scope is
outside the active envelope.

Record a blocker when work requires new authority, an external decision, an
unavailable device/service, or out-of-scope repository changes. Difficulty or
test duration alone is not a blocker.

## Workflow state

The `morphospace/` directory keeps one proposed/active milestone record, compact
state, an inert feature lock, and append-only events. It is a control surface,
not runtime authority.

Use the work-environment workflow owner for state transitions. Do not hand-edit
an iteration unit from `proposed` to `ready`, `active`, `validating`, or
`accepted`.
