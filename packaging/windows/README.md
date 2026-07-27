# Rusty Fleet Windows distribution

This directory owns the portable Windows bundle and per-user installer
workflow. Rusty Fleet remains directly installable: QuestIonAble File Manager
is neither bundled nor required. An unsigned developer bundle is local
development material, not a distributable release.

## Bundle boundary

Every bundle contains exactly four runtime components:

1. Fleet Console, the native WPF operator application;
2. Fleet Hub, the local authority-aware service;
3. `fleetctl`, the automation CLI;
4. one externally supplied `rusty-hostess-hotspot-provider.exe`.

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

The ZIP is reproducible for identical input bytes, revisions, version, and
source-date epoch. An unsigned bundle and its owner provenance must both say
`development_only`; the Fleet manifest sets `publication_allowed` to false.
It is not a release artifact.

## Inspect and install

Validate a bundle without contacting a network service:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\packaging\windows\Test-WindowsBundle.ps1 `
  -BundleRoot <expanded-bundle>
```

For a signed bundle, append
`-ExpectedHostessSignerThumbprint <owner-authorized-thumbprint>`. Obtain that
pin from the independently trusted release channel, not from the bundle being
validated.

The installer is planning-only unless `-Execute` is explicitly supplied:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File <expanded-bundle>\distribution-tools\Install-RustyFleet.ps1 `
  -Action Install
```

The same `-ExpectedHostessSignerThumbprint` argument is required when planning,
installing, or rolling back a signed bundle.
If rollback crosses an owner signer rotation, also provide
`-RollbackHostessSignerThumbprint <previous-owner-authorized-thumbprint>`;
otherwise rollback reuses the current pin.

An install uses side-by-side version directories and atomically changes only a
small current-version metadata pointer after the staged copy passes the same
offline validation. It does not register a service, launch a process, change
`PATH`, create configuration, or install ADB.

Execute a reviewed install:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File <expanded-bundle>\distribution-tools\Install-RustyFleet.ps1 `
  -Action Install `
  -Execute
```

Plan or execute rollback with `-Action Rollback`. Rollback verifies the
previous release and switches the pointer; it does not delete either release.

## Publication

GitHub Releases is the binary source of truth. Publication is blocked unless
the workflow runs in `signed-release` mode, all four executables have valid
Authenticode signatures, and Hostess's owner document says
`eligibility=signed_release` with a verified signing identity matching the
explicit workflow input `hostess_signer_thumbprint`. The workflow downloads
the provider and its three owner metadata documents, builds and signs the
Fleet executables, validates the entire bundle, and publishes the ZIP plus
hash-bound metadata.

GitHub Pages is a human-facing guide only. It links to GitHub Releases and does
not duplicate or proxy release binaries.

## Offline tests

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\packaging\windows\tests\Test-WindowsDistribution.ps1
```

The suite covers deterministic archives, exact composition, provider
provenance, digest mismatch, payload tampering, unmanifested files, plan-only
installer behavior, side-by-side update metadata, and pointer-only rollback.
