# Rusty Fleet Morphospace Workspace

This is the public protocol-v2 planning and composition surface for Rusty
Fleet. It was scaffolded from the Rusty Morphospace Work Environment at the
schema revision pinned in the JSON documents.

The workspace begins inert:

- no feature or module is selected;
- the effect union is empty;
- no runtime, listener, permission, device action, media route, or relay is
  active;
- no milestone is current.

Resume in this order:

1. `project.spec.json`
2. `feature.lock.json`
3. `workspace.state.json`
4. the current iteration unit, if state names one
5. only the event tail and receipts referenced by state
6. `../docs/IMPLEMENTATION_PLAN.md`
7. `../docs/WORKFLOW.md`
8. `../docs/DATASTREAMS.md`

The first proposed unit,
`iteration-units/fleet-m0-foundation-and-simulator.json`, is one vertical
milestone stack. Review it into `ready` through the owned workflow transition;
do not split it into schema-, class-, fixture-, or test-sized units and do not
hand-edit lifecycle state.

Validate from the work-environment clone:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File <work-environment-root>/scripts/Test-WorkflowContracts.ps1 `
  -WorkspaceRoot <project-root>/morphospace
```

Keep local repository maps, device identities, credentials, raw evidence, and
generated artifacts outside tracked files.
