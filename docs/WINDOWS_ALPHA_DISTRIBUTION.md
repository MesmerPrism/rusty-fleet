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

Every signed channel registers an explicit `--uninstall` Setup route under its
own product identity. Uninstall validates the installed release channel and
registered install root before removing only that channel's shortcuts,
registration, and install tree. Setup stages its copied authority, shortcuts,
and uninstall registration before committing `state/current.json`; shell or
state-commit failure restores the prior files and registry values. Synthetic
failure injection exists only in unsigned development builds with an isolated
filesystem-backed shell root.

Stable retains its existing names, identity, root, and metadata path. Setup
binds the embedded manifest channel and channel-specific Setup authority before
mutation. `preview` remains accepted only as a deprecated compatibility input;
new automation uses `dev`, `alpha`, or `stable`.

The separate protected alpha workflow delegates exact owner artifact,
provenance, bundle, signing, descriptor, preflight, and publication inputs to
Fleet's existing distribution owner under `windows-alpha-release`. That owner
creates a prerelease and never marks it latest. Pages deployment composes the
complete site and verifies every non-target metadata byte, especially stable,
before replacing only the alpha subtree. Renewal resolves the canonical alpha
tag once and carries it through checkout, remote lookup, publication preflight,
and Pages handoff. Remote publication also reads back `/releases/latest` after
visibility. An authoritative GitHub `404 Not Found` is accepted when no stable
release exists; otherwise the endpoint must return a different canonical
stable `vX.Y.Z` tag. Other failures, malformed tags, and alpha tags reject.

Rollback rewrites shortcut and uninstall-registration readback from the
activated historical release, including that release's version. Uninstall
snapshots the complete channel-owned shortcut directory and uninstall
registration before removal. The fully validated, reparse-free install root is
first renamed atomically on the same volume to a unique product-owned
quarantine beside the original root. A failure before recursive deletion
renames it back and restores shell identity byte-for-byte. Once recursive
deletion has started, failure never recreates shortcuts to damaged content:
shell identity stays absent and Setup returns `recoverable_cleanup` with the
exact quarantine and adjacent recovery-receipt names for reviewed cleanup.

Checked-in production signing policy remains disabled. Synthetic tests use
ephemeral keys and no fallback production key exists.

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-WindowsDistribution.ps1
```
