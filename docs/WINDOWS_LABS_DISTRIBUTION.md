# Windows Stable and Labs Distribution

Rusty Fleet has two persistent product channels: `stable` and `labs`. Stable is
the default. Labs is an explicit opt-in and distributes the same complete
five-component Windows product, not a reduced feature build.

The release contracts keep four independent axes:

- `product_channel`: `stable|labs`, persisted by Setup and used for product
  identity and isolation;
- `maturity`: `alpha|beta|rc|released`, describing release readiness only;
- Fleet `channel`: `dev|labs|stable`, retaining the owner-specific build ring;
  public release metadata and publication admit only `labs|stable`, while
  `dev` remains local-development material;
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

The signed release payload v4, envelope v4, release manifest v3, descriptor
receipt v5, publication receipt v3, and Pages handoff v2 carry the exact axes.
Validators reject missing, unknown, mismatched, or legacy channel fields. Labs
identity must pair with the Labs distribution track; Stable releases must use
the stable track. The dev track cannot create a release descriptor, publication
preflight, or Pages deployment.

Setup validates its embedded manifest and channel-specific authority before
mutation. Install, rollback, and uninstall remain isolated to their exact
product root, shortcuts, registration, retained releases, and recovery state.
The owner-effective receipt and rollback boundaries are unchanged.

The protected Labs workflow delegates signing, bundle creation, descriptor
creation, publication preflight, and publication to Fleet's existing Windows
distribution owner under `windows-labs-release`. Labs releases are prereleases
and never latest. Pages replaces only the Labs metadata subtree while checking
that every non-target metadata byte, especially Stable, remains unchanged.

The checked-in v2 production policy enables only Labs. It authorizes the exact
`CN=MesmerPrism` Authenticode certificate with SHA-1 thumbprint
`08A5878AD6E652A94517D2C79144EB2655B0088C`, certificate SHA-256
`baead63c37e32085c3af19b4c739a6a308d700529f107d40e14fec2c94fe7ddf`,
and descriptor SPKI SHA-256
`0b3ef04dc5481d5e0a0a243df298c31052501e014a6e27516c48b95846657d0c`.
The Authenticode certificate is self-issued. Its exact pin authorizes Labs
owner identity and artifact integrity inside Fleet, but does not claim public
Windows PKI trust or SmartScreen reputation. Windows may display an Unknown
publisher warning; Setup tells the operator to confirm MesmerPrism only when
they deliberately selected Labs. Fleet never installs or changes a Windows
root certificate.

Authenticode chain status is host-dependent. The same exact certificate may be
reported as `Valid` with a clean one-element chain on a machine that already
trusts it, or as `UnknownError` with exactly `UntrustedRoot` elsewhere. Fleet
admits only those two exact shapes and validates the Hostess-recorded boundary
separately from its current-host observation. A recorded `Valid` shape must use
`host-chain-valid-no-public-trust-claim`; a recorded `UnknownError` plus
`UntrustedRoot` shape must use
`exact-pinned-self-issued-untrusted-root-only`. Subject, thumbprint,
certificate bytes, code-signing EKU, timestamp, self-issued state, and
`public_trust_claim=false` must agree in either case.

Stable publication remains disabled and `public-chain-only`; Labs evidence
cannot be substituted into Stable. Enabling Stable or replacing the Labs
certificate with managed public-trust signing requires a separate protected
policy review. Tests use synthetic ephemeral keys, never mutate the Root store,
do not contact devices, and do not publish anything:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\Test-WindowsDistribution.ps1
```
