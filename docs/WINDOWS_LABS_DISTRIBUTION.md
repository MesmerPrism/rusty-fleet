# Windows Stable and Labs Distribution

Rusty Fleet has two persistent product channels: `stable` and `labs`. Stable is
the default. Labs is an explicit opt-in and distributes the same complete
five-component Windows product, not a reduced feature build.

The release contracts keep four independent axes:

- `product_channel`: `stable|labs`, persisted by Setup and used for product
  identity and isolation;
- `maturity`: `alpha|beta|rc|released`, describing release readiness only;
- Fleet `channel`: `dev|labs|stable`, retaining the owner-specific release
  ring;
- `distribution_track`: `local-development|github-prerelease|github-release`,
  describing transport only and mapping exactly from `channel`.

`vX.Y.Z-alpha.N`, `-beta.N`, and `-rc.N` suffixes describe maturity. In
particular, `-alpha.N` does not select Labs. Release automation must supply the
product/distribution selection and maturity explicitly and validate all three.
The old `alpha` and `preview` channel selectors are rejected; they are not
silently mapped to Labs.

Labs owns product ID `rusty-fleet-labs`, display name `Rusty Fleet Labs`, assets
named `RustyFleet-Labs-*`, install root
`%LOCALAPPDATA%\RustyFleetLabs`, its own Start Menu and uninstall identities,
and metadata at `/Rusty-Fleet/metadata/labs/release.json`. Stable retains its
existing `rusty-fleet`, `RustyFleet-*`, `%LOCALAPPDATA%\RustyFleet`, shell,
uninstall, and `/Rusty-Fleet/metadata/stable/release.json` behavior unchanged.

The signed release payload v3, envelope v3, release manifest v2, descriptor
receipt v4, publication receipt v2, and Pages handoff v2 carry the exact axes.
Validators reject missing, unknown, mismatched, or legacy channel fields. Labs
identity must pair with the Labs distribution track; Stable may use only the
dev or stable track.

Setup validates its embedded manifest and channel-specific authority before
mutation. Install, rollback, and uninstall remain isolated to their exact
product root, shortcuts, registration, retained releases, and recovery state.
The owner-effective receipt and rollback boundaries are unchanged.

The protected Labs workflow delegates signing, bundle creation, descriptor
creation, publication preflight, and publication to Fleet's existing Windows
distribution owner under `windows-labs-release`. Labs releases are prereleases
and never latest. Pages replaces only the Labs metadata subtree while checking
that every non-target metadata byte, especially Stable, remains unchanged.

Checked-in production signing policy remains disabled. Tests use synthetic
ephemeral keys. Validation is local and synthetic and does not contact devices
or publish anything:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-WindowsDistribution.ps1
```
