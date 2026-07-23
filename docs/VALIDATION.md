# Validation

## Goal

Validation should prove the boundary changed by the current work while avoiding
unrelated expensive suites. Quick, Standard, device, and Deep gates are
separate and cumulative only at the checkpoint where their evidence is needed.

## Repository tiers

### Quick

Quick is safe during normal editing and checks:

- required public repository files;
- JSON and JSONL syntax;
- whitespace through `git diff --check`;
- public-boundary and secret-like patterns;
- key planning/workflow invariants.

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-Repo.ps1 -Tier Quick
```

### Standard

Standard includes Quick and adds:

- internal Markdown link resolution;
- project/feature/workspace identity consistency;
- milestone-stack and inert-lock assertions;
- repository instruction and CI surface checks.
- operator-UI planning links, reference-ledger links, and public-safe research
  provenance.

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-Repo.ps1 -Tier Standard
```

When the Rusty Morphospace Work Environment is available, also validate the
portable project contract:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\Test-Repo.ps1 `
  -Tier Standard `
  -WorkEnvironmentRoot <work-environment-root>
```

### Device

There is no implicit device command in repository validation. A milestone that
requires a headset declares a separate run through the Meta Quest workflow,
with:

- exact target identity;
- source/build/profile preflight;
- bounded log and fatal window;
- owner-effective evidence;
- cleanup and prior-state restoration;
- a sanitized receipt kept within the public/private boundary.

### Deep

Deep includes Standard and adds:

- tracked-file reconciliation and large/generated artifact checks;
- architecture/authority/public-boundary review markers;
- any implementation-era full workspace, security, performance, or
  cross-repository checks registered by the active milestone.

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-Repo.ps1 -Tier Deep
```

Deep never performs live device work by itself.

## Milestone-specific validation

Each milestone adds one scenario suite rather than many aggregate scripts named
after micro-units. Focused tests may be numerous, but the stable operator
entrypoints remain:

- component-focused test command;
- milestone scenario suite;
- repository Quick/Standard/Deep gate;
- explicit device suite when required.

When an implementation language and build system land in Milestone 0, update
this document, `AGENTS.md`, the README, CI, and `Test-Repo.ps1` in that same
milestone.

## Operator UI validation

The normative behavior and candidate budgets are in
[Operator UI Architecture](OPERATOR_UI.md). The gates accumulate only when a
WPF surface exists:

| Check | M0 | M1+ edit loop | WPF milestone | Release |
| --- | ---: | ---: | ---: | ---: |
| canonical condition/query/projection fixtures | required | focused | Standard | Deep |
| deterministic 4/50/250/1k/5k datasets | required | affected profile | Standard | Deep |
| Console/CLI/API membership and reason parity | contract only | focused | Standard | Deep |
| keyboard and UI Automation regression | not applicable | focused | Standard | Standard |
| Narrator, high contrast, large text, scaling | not applicable | targeted | manual milestone gate | full release gate |
| stable ordering, hidden selection, navigation restoration | contract fixture | focused | scenario gate | regression |
| target snapshot and per-target ledger | M2 contract | focused | M2 Standard | regression |
| measured latency, memory, and update churn | candidate fixtures | nearest profile | declared milestone | Deep |

Performance thresholds in the UI guide are candidates until a milestone
records reference hardware, data profile, method, distribution, achieved
result, and headroom. Do not convert a single fast run into a supported-scale
claim.

Screenshot matrices detect layout drift but do not replace keyboard, UI
Automation, screen-reader, or interaction tests.

## Evidence vocabulary

Keep these facts distinct:

- **observed:** an adapter reported something;
- **accepted:** the authority admitted it at a revision;
- **dispatched:** an accepted command was sent;
- **applied:** the owning consumer reported the effect;
- **cleaned:** terminal cleanup was independently observed;
- **rejected/expired/cancelled:** no successful application is claimed.

An aggregate fleet result is a projection over per-device facts, not a
replacement for them.

## GitHub cadence

- Quick runs on every push and pull request.
- Standard runs on pull requests and `main`.
- Deep is manual/release-triggered until implementation needs a scheduled
  integration gate.
- Device suites run outside generic GitHub-hosted CI.
