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
