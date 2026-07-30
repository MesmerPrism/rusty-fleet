# Windows Alpha Distribution

Rusty Fleet's alpha channel distributes the same complete five-component
Windows product as stable. It is not a reduced feature build.

Alpha uses numeric immutable versions (`X.Y.Z`) and Git tags of the form
`vX.Y.Z-alpha.N`. It owns `RustyFleet-Alpha-Setup.exe`, numeric-version alpha
bundle assets, product ID `rusty-fleet-alpha`, display name `Rusty Fleet
Alpha`, `%LOCALAPPDATA%\RustyFleetAlpha`, and the complete state, lock,
staging, retained-release, and rollback-history subtree below that root.
Metadata is staged at `Rusty-Fleet/metadata/alpha/release.json`.
Setup also owns a `Rusty Fleet Alpha` Start Menu folder and the per-user
`rusty-fleet-alpha` uninstall registration; both point only into the alpha
root.

Stable retains its existing names, identity, root, and metadata path. Setup
binds the embedded manifest channel and channel-specific Setup authority before
mutation. `preview` remains accepted only as a deprecated compatibility input;
new automation uses `dev`, `alpha`, or `stable`.

The separate protected alpha workflow delegates exact owner artifact,
provenance, bundle, signing, descriptor, preflight, and publication inputs to
Fleet's existing distribution owner under `windows-alpha-release`. That owner
creates a prerelease and never marks it latest. Pages deployment composes the
complete site and verifies every non-target metadata byte, especially stable,
before replacing only the alpha subtree.

Checked-in production signing policy remains disabled. Synthetic tests use
ephemeral keys and no fallback production key exists.

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-WindowsDistribution.ps1
```
