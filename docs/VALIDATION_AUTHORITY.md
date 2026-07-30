# Pull Request Validation Authority

## Decision

Rusty Fleet admits pull-request changes through a base-owned, read-only static
authority check before it relies on candidate-owned build or test workflows.
The authority validates repository identity, exact pull-request refs and
topology, protected paths, and any exact pre-approved artifact set. It never
checks out, imports, dot-sources, or executes candidate content.

The required check name is `validation-authority/base-owned`. Treat that name
as provisional until a canary pull request proves the exact observed context
and stable event behavior. Configure a repository ruleset only after that
canary. A same-name GitHub Actions check can still be spoofed by another
candidate workflow running under the GitHub Actions app; a dedicated GitHub App
is the stronger long-term check issuer.

## Scope

The base-owned workflow:

- runs on selected `pull_request_target` events against `main`;
- runs only on a fresh GitHub-hosted Windows runner and rejects self-hosted or
  non-Windows execution;
- grants only `contents: read`;
- checks out the exact base commit and the exact published shared verifier;
- extracts trusted scripts from their Git blobs into a fresh runner-temporary
  directory before execution;
- fetches only the server-owned numeric pull-request head and merge refs into a
  private ref namespace;
- verifies the event head, records a nullable event-time synthetic merge ID,
  then treats the freshly fetched merge ref as authoritative while proving its
  exact two-parent order and tree equality with the candidate tree;
- invokes the shared external authority verifier pinned at commit
  `354545a63e870c3d89254f8fb78f6ed4060a8dc3`, tree
  `1cf79cd4478e5dd5b940729b917a8beea41dac40`; and
- writes bounded external and Fleet wrapper assessments outside both Git
checkouts.

This design assumes GitHub-hosted runner isolation and the integrity of the
GitHub runner image, Actions service, pinned checkout action, and installed
Git/PowerShell runtimes. It is not a safe recipe for a persistent or
multi-tenant self-hosted runner.

The base policy is
`config/fleet-pull-request-authority.v1.json`. It protects the workflow,
policy, adapter, schemas, self-test, approval tokens, all `.github/` content,
and the risk-tier validation authority surfaces.

## Non-scope

This check does not:

- execute candidate source, workflows, scripts, builds, tests, or generated
  artifacts;
- attest that candidate behavior is correct or that validation passed;
- install, launch, mutate, or observe a device;
- grant merge, publication, signing, release, or deployment authority; or
- replace the risk-proportional candidate validation receipts.

Candidate validation remains a separate evidence lane. Publication remains a
separate human or repository-owner decision.

## Authority and trust roots

GitHub supplies the repository identity, immutable event fields, numeric pull
request number, and server-owned `refs/pull/<number>/{head,merge}` refs. The
event's `merge_commit_sha` may be null and may name an earlier synthetic merge
than a later fetch of the server-owned merge ref; the assessment records both
without equating them. The base branch owns the workflow, Fleet adapter,
policy, wrapper schema, and self-test. The published work-environment commit
owns the generic policy schema, external assessment schema, and static
verifier.

The shared verifier is pinned by commit, tree, Git blob object, byte count, and
SHA-256:

| Artifact | Git object | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `scripts/Test-ExternalValidationAuthority.ps1` | `277a3bbbabfdedc66d50263a37e06bb094acac5f` | 34411 | `89c3875c426eaa30500108644c0e2a89802d44049827aec2fe452358a5416c0e` |
| `schemas/external-validation-authority-policy-v1.schema.json` | `1a7ed651d6cbcfccbf792ca60d41cc16301c407a` | 4217 | `a89050065ea95d4f2d6edbf85c1d4e05802cef8c92c71684fb0d84e7cc616826` |
| `schemas/external-validation-authority-assessment-v1.schema.json` | `2ad62c6c6f034538a8abc530e2cfecb7a43f8614` | 2769 | `88b8b8a8d70cc5af50c9e43428017f971d04476b9235ebc545b634175e011426` |

The adapter rejects replacement refs, shallow history, alternates, grafts,
ambient Git repository/object selectors, identity mismatches, unexpected
topology, dirty trusted checkouts, symbolic-link or reparse-point escapes,
oversized inputs, malformed strict JSON, and output reuse.

## One-time approval consumption

Protected changes fail closed unless the candidate matches exactly one
base-owned approved change set. An approval binds:

1. the complete ordinally sorted changed-path set;
2. the exact regular-file mode, byte count, and SHA-256 of every present
   artifact;
3. every required deletion as an explicitly absent artifact; and
4. a reviewed ancestor that must be in the candidate and must not already be
   in the trusted base.

The first risk-tier guardrail approval also requires deletion of
`config/validation-authority/fleet-validation-guardrails-20260730.approval-token.json`.
After the approved candidate merges, both the required ancestor and token are
consumed. Replaying the same approval therefore fails.

Authority-policy evolution is intentionally not an ordinary self-service
change: the policy cannot safely authorize its own unreviewed replacement.
Use an audited trust-root operation or land a narrowly reviewed,
base-owned successor approval before proposing the policy change. Preserve the
old policy and assessment as evidence; never weaken the current policy merely
to unblock its replacement.

## Interfaces and observability

`tools/Test-FleetPullRequestAuthority.ps1` consumes only explicit event and
runner identities plus the trusted Fleet and verifier roots. It writes:

- the generic external assessment, which contains exact changed and protected
  paths; and
- `rusty.fleet.pull_request_authority_assessment.v1`, which binds repository
  IDs, pull-request topology, workflow/run identity, every trusted artifact,
  the external assessment hash, decision, and explicit false
  execution/publication claims. The wrapper does not duplicate path arrays or
  counts; the hashed external assessment remains their single authority.

Both files are run-bound evidence, not signatures. The hosted workflow prints
only a fixed decision/count/hash summary; it does not echo candidate paths into
the job summary. A failed check may retain normal GitHub logs but must not
upload arbitrary candidate-controlled content.

## Validation

Before enabling a ruleset:

1. run the adapter self-test locally, including an end-to-end pass against the
   exact published verifier;
2. run Fleet Quick, Standard, and the risk-tier guardrail self-tests;
3. open a canary pull request that changes an unprotected documentation path;
4. observe the exact check suite/app/context and confirm the authority receipt;
5. exercise a protected negative canary and confirm fail-closed behavior; and
6. only then require the exact observed base-owned context on `main`.

Re-run the canary before changing the workflow trigger, check name, GitHub
permissions, repository ruleset, adapter, policy, schemas, or shared verifier
pin.

## Mitigation map

| Risk | Mitigation | Residual limitation |
| --- | --- | --- |
| Candidate workflow executes before review | `pull_request_target` base workflow; no candidate checkout or execution | Same Actions app can emit a same-name check |
| Fork/ref substitution | Numeric server-owned PR refs plus exact event-head comparison | GitHub remains the ref and event authority |
| Synthetic merge substitution or regeneration | Event merge ID is observational; freshly fetched numeric merge ref is authoritative and must have exact parent order and tree equality | A conflicted PR fails closed |
| Base checkout text conversion | Trusted executables and schemas are extracted from exact Git blobs | Runner and Git are still trusted computing base |
| Approval replay | Candidate-only required ancestor plus required token deletion | New protected work needs a new base-owned approval |
| Candidate validation confused with admission | Receipt fixes execution and publication claims to false | Separate trusted execution evidence is still required |
| Policy self-authorization | Audited trust-root update or pre-authorized successor | Rare policy changes intentionally carry more friction |

## Next slice

Bootstrap and canary this authority without enabling a ruleset. After the
canary proves the exact check identity, require it on `main`, publish the
approved risk-tier guardrail candidate, and confirm the consumed approval no
longer admits a replay.
