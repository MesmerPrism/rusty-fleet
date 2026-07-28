# Rusty Fleet Windows distribution

This directory owns the portable Windows bundle and per-user installer
workflow. Rusty Fleet remains directly installable: QuestIonAble File Manager
is neither bundled nor required. An unsigned developer bundle is local
development material, not a distributable release.

## Bundle boundary

Every bundle contains exactly five runtime components:

1. Fleet Console, the native WPF operator application;
2. Fleet Hub, the local authority-aware service;
3. `fleetctl`, the automation CLI;
4. `fleet-onboard`, the offline private onboarding generator;
5. one externally supplied `rusty-hostess-hotspot-provider.exe`.

The Hostess provider is preserved as a separate owner artifact. The builder
requires its exact SHA-256 and the owner-issued
`rusty-hostess-hotspot-provider.provenance.json`, `LICENSE`, and
`THIRD-PARTY-NOTICES.txt`. It independently checks the provider hash and size,
clean source commit/tree and source availability, embedded product version,
canonical PE payload, dependency and native library inventories, signing
state, distribution eligibility, and companion document hashes. Signed
publication also requires the independently supplied owner-authorized signer
thumbprint. It rejects an alternate filename, digest, or Fleet-invented
provenance. Its fixed invocation is:

```text
rusty-hostess-hotspot-provider.exe integration windows-hotspot --json
```

The request and receipt schemas remain Hostess-owned. Fleet packages and pins
the artifact; it does not redefine its hotspot behavior.

Credentials, private configuration, device serials, pairing material, ADB, and
generated runtime state are deliberately absent. Configuration is supplied
after installation through the existing private runtime boundary.
`fleet-onboard` is installed inertly: installation never invokes it or creates
device records. An operator runs it explicitly with a private request after
installation, and its machine-bound outputs are never release artifacts.

## Build an unsigned developer bundle

First obtain the exact Hostess provider from its owning release. Then run:

```powershell
$providerSha256 = (
  Get-FileHash -Algorithm SHA256 -LiteralPath <provider-path>
).Hash.ToLowerInvariant()

pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\packaging\windows\New-WindowsBundle.ps1 `
  -Version <semver> `
  -HostessProviderPath <provider-path> `
  -HostessProviderSha256 $providerSha256 `
  -HostessProviderMetadataDirectory <owner-metadata-directory>
```

Outputs are written below the ignored `artifacts/windows-distribution`
directory. They include the expanded bundle, deterministic ZIP, ZIP checksum,
standalone release manifest, payload checksums, and validation receipt.

The ZIP is reproducible for identical input bytes, version, and source-date
epoch. The recorded clean-worktree pre-build assertion is not represented as
an immutable source-to-artifact proof. An unsigned bundle and its owner provenance must both say
`development_only`; the Fleet manifest sets `publication_allowed` to false.
It is not a release artifact.

`New-WindowsSetup.ps1` embeds that already validated ZIP in one self-contained
`RustyFleet-Setup.exe`. Its exact no-change automation route is
`--plan --json`; its zero-argument route is the visible guided installer.
Setup uses the generic Fleet icon, requires no elevation, retains every
existing install-path ancestor without delete sharing, rejects reparse points,
creates every payload file new, validates the extracted inventory, and changes
no process, service, configuration, credential, onboarding, or ADB state.

## Inspect and install

Validate a bundle without contacting a network service:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\packaging\windows\Test-WindowsBundle.ps1 `
  -BundleRoot <expanded-bundle>
```

For a signed bundle, append both
`-ExpectedFleetSignerThumbprint <Fleet-authorized-thumbprint>` and
`-ExpectedHostessSignerThumbprint <owner-authorized-thumbprint>`. Obtain both
pins from the independently trusted release channel, not from the bundle being
validated.

`RustyFleet-Setup.exe` is the only install, update, and rollback authority.
It is not accompanied by a second installer script. QuestIonAble File Manager
may inspect the executable through its exact, side-effect-free planning
contract:

```text
RustyFleet-Setup.exe --plan --json
```

That command returns exactly the schema, product, version, channel, SHA-256 of
the Setup executable, and readiness flag; it never changes the install root.
Run the executable with no arguments for the visible prompt. `I` performs a
new install or update, `R` selects the previous release, and `N` exits without
changes.

Setup serializes all work for an install root with one bounded exclusive
transaction. Before changing state, it validates the complete current and
historical release inventories—not just metadata—and retains the install
volume, directories, payload files, and state leaves without write/delete
sharing. It rejects reparse points and hard-linked payloads. New payloads are
extracted into an unpredictable `.candidate-<digest>-<random>` directory;
an interruption leaves that directory inert because only a fully validated
candidate may be named by state. The state file is written and verified
through a retained handle, renamed atomically by handle, then read back through
that same handle. Rollback repeats the full historical validation and changes
only the pointer. Setup never automatically deletes a retained release or an
interrupted candidate.

Install, update, and rollback do not register a service, launch a Fleet
process, change `PATH`, create configuration or credentials, invoke
`fleet-onboard`, or install/use ADB.

## Publication

GitHub Releases is the binary source of truth. Publication is blocked unless
the workflow runs in `signed-release` mode, all five executables have valid
Authenticode signatures, every Fleet-owned executable matches an independently
supplied Fleet signer pin, and Hostess's owner document says
`eligibility=signed_release` with a verified signing identity matching the
explicit workflow input `hostess_signer_thumbprint`. The workflow downloads
the provider and its three owner metadata documents, builds and signs the
Fleet executables, validates the entire bundle, and publishes the ZIP plus
hash-bound metadata. It then embeds the ZIP in Setup, signs Setup, hashes the
signed bytes, and creates a compact RFC 8785 JCS v2 payload signed with
RSA-PSS/SHA-256. The exact public SubjectPublicKeyInfo bytes are exported as
`release-descriptor.spki.der`; their SHA-256 must equal the independently
authorized policy pin.

`Publish-WindowsRelease.ps1 -Mode Preflight` is the only publication
preflight, and `-Mode Publish` is the only GitHub Release mutation authority.
The workflow stages a flat, closed directory containing exactly ten release
assets plus the revisioned policy. Before consulting a release token or
invoking `gh`, the authority retains the fixed local volume root, every
ancestor, and every exact input leaf without write/delete sharing. It rejects
reparse points, hard links, additions, omissions, casing changes, and identity
changes.

Preflight independently rechecks the exact local tag, commit, tree, and tagged
policy blob; Setup Authenticode, its full signed hash, and canonical pre-sign
receipt; the ZIP sidecar and fully extracted bundle; and byte equality between
the published manifest, checksums, validation receipt, and their inner ZIP
copies. It reconstructs the exact JCS v2 payload, verifies RSA-PSS using the
exported SPKI, checks every descriptor-receipt hash, and builds a closed
filename-to-SHA-256-and-size inventory.

Publish separately resolves the current GitHub tag through a bounded,
cycle-checked lightweight or annotated-tag chain and requires its terminal
commit to equal the retained source revision. It performs this check
immediately before upload, immediately before visibility, and once more after
the visible asset inventory is verified but before a passing publication
receipt can be issued. Release lookup uses a bounded successful inventory
query: authentication, network, malformed JSON, and an exhausted page bound
are errors, never evidence that a release is absent.

The authority creates an empty draft or resumes the one exact existing draft.
An existing non-draft, duplicate tag, unexpected asset, duplicate asset, or
asset with a mismatched or missing GitHub `sha256:<lowerhex>` digest, size, or
upload state fails closed. A valid partial draft uploads only missing expected
assets without clobbering anything. It then re-fetches and matches every remote
digest and size to the retained local inventory before making that exact draft
visible. Create or upload interruption may leave a hidden resumable draft;
mismatched remote state is never deleted or rewritten automatically.

The signing job is read-only, checks out without retained credentials, requires
the exact pre-existing `v<version>` tag, and exposes the dedicated release-only
publication token only to the final step-scoped authority. Actions artifacts
contain evidence only and are never publication inputs. `release.json` carries
an exact bounded validity duration (23 hours in the release workflow), points
only to the immutable numeric-version GitHub Release asset, and is destined for
`https://mesmerprism.com/Rusty-Fleet/metadata/<channel>/release.json`.

GitHub Pages is a human-facing guide and signed-metadata surface only. It does
not duplicate or proxy release binaries. Until the protected signing keys and
an actual signed release and protected metadata deployment exist, the site
must say that no supported download or deployed metadata is available.

### Renewable Pages metadata handoff

The protected Pages workflow renews the 23-hour descriptor every 12 hours and
may also be dispatched explicitly. It checks out the exact existing release
tag for release validation while retaining the human site from the triggering
main revision. It requires the configured commit, tree, and tagged policy,
resolves the visible GitHub Release tag to that commit, downloads its complete
ten-asset inventory, and verifies every remote SHA-256 and size before
generating new metadata. The unchanged Setup and bundle remain GitHub Release
assets. Pages receives only the human site plus:

```text
Rusty-Fleet/metadata/<channel>/release.json
Rusty-Fleet/metadata/<channel>/release-descriptor.receipt.json
Rusty-Fleet/metadata/<channel>/release-descriptor.spki.der
Rusty-Fleet/metadata/<channel>/deployment-handoff.json
```

`New-WindowsPagesDeployment.ps1` rejects stale, downgraded, replayed, or
wrong-source metadata, requires the token-free closed-release preflight, and
enforces a binary-free Pages tree. Its
`rusty.fleet.windows_release_metadata_handoff.v1` output carries only fixed
relative filenames, hashes, sizes, owners, version/channel, exact source
commit/tree, freshness, and prior-handoff lineage. It names Setup and its build
receipt as `github_releases` authority assets without copying either binary to
Pages. A completed output is idempotently resumable; an explicitly resumed
partial owned staging directory is rebuilt from the retained inputs.

Source publication and metadata deployment remain disabled by default.
Activation requires the repository variable
`FLEET_METADATA_DEPLOYMENT_ENABLED=true`, plus these public trust inputs in the
protected `windows-release-metadata` environment:

| Variable | Exact meaning |
| --- | --- |
| `FLEET_METADATA_VERSION` | Numeric three-component release version |
| `FLEET_METADATA_CHANNEL` | `dev`, `preview`, or `stable` |
| `FLEET_METADATA_SOURCE_REVISION` | Full 40-hex commit resolved by `v<version>` |
| `FLEET_METADATA_SOURCE_TREE` | Full 40-hex tree for that exact commit |
| `FLEET_SIGNER_THUMBPRINT` | Reviewed uppercase Fleet Authenticode thumbprint |
| `HOSTESS_SIGNER_THUMBPRINT` | Reviewed uppercase Hostess Authenticode thumbprint |
| `FLEET_DESCRIPTOR_SIGNER_SPKI_SHA256` | Reviewed lowercase descriptor RSA SPKI SHA-256 |

The protected secret used by renewal is
`FLEET_DESCRIPTOR_SIGNING_KEY_PEM_BASE64`. The signed-release workflow also
requires `FLEET_SIGNING_PFX_BASE64`, `FLEET_SIGNING_PFX_PASSWORD`, and, only
for final GitHub Release mutation, `FLEET_RELEASE_PUBLISH_TOKEN`. Secret values
never enter workflow inputs, release policy, receipts, Actions evidence, or
Pages content. The checked-in release policy intentionally remains disabled
with empty pin arrays until the reviewed public production identities are
supplied in a dedicated release commit.

## Offline tests

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\packaging\windows\tests\Test-WindowsDistribution.ps1
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\packaging\windows\tests\Test-WindowsReleaseDescriptor.ps1
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\packaging\windows\tests\Test-WindowsReleasePolicy.ps1
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\packaging\windows\tests\Test-WindowsPagesDeployment.ps1
pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File .\packaging\windows\tests\Test-WindowsPublicationRemote.ps1
pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File .\packaging\windows\tests\Test-WindowsPublication.ps1
```

The suite covers deterministic archives, exact five-component composition,
provider provenance, digest mismatch, payload tampering, unmanifested files,
the Setup embedded-bundle and exact planning contracts, path-free build
receipts bound to source commit/tree and canonical pre-sign PE bytes, a
controlled guided install, interruption recovery, retained-leaf substitution,
full historical tamper and hard-link rejection, concurrent transaction
serialization, side-by-side update state, and fully verified pointer rollback.
The publication suite uses one ephemeral CurrentUser test signer and no real
release token or `gh`. Release descriptor, ZIP, top-level manifest, checksums,
Setup build receipt, validation receipt, and asset addition/removal
substitutions must all fail before the fake publisher can run. It also locks
de-DE, en-US, and invariant-culture handling of typed provenance timestamps
while requiring canonical UTC JSON on disk. The separate trust-free remote
publication suite uses a stateful fake `gh` process to cover create, exact
verification, visibility, partial-upload resume, complete-draft resume, remote
digest/size/state mismatch, absent digest, extra and duplicate assets,
lightweight and nested annotated tags, tag movement between stages, tag
movement after visibility, tag cycles/depth, malformed JSON, and
authentication failure.

The publication baseline additionally exercises fresh renewal, stale metadata,
wrong source/tag/asset/signer rejection, descriptor replay rejection, Pages
binary exclusion, idempotent completion, and interrupted-stage resume.
