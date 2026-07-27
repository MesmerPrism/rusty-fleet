# Rusty Fleet Windows distribution

This directory owns the portable Windows bundle and per-user installer
workflow. Rusty Fleet remains directly installable: QuestIonAble File Manager
is neither bundled nor required.

## Bundle boundary

Every bundle contains exactly four runtime components:

1. Fleet Console, the native WPF operator application;
2. Fleet Hub, the local authority-aware service;
3. `fleetctl`, the automation CLI;
4. one externally supplied `rusty-hostess-hotspot-provider.exe`.

The Hostess provider is preserved as a separate owner artifact. The builder
requires its exact SHA-256, public source repository, full source revision,
version, and release URL. It rejects an alternate filename or digest. Its
fixed invocation is:

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
  -HostessProviderVersion <provider-version> `
  -HostessProviderSourceRepository <public-https-repository> `
  -HostessProviderSourceRevision <full-40-character-commit> `
  -HostessProviderSourceTree <full-40-character-tree> `
  -HostessProviderProvenanceUrl <public-https-release-or-commit-url>
```

Outputs are written below the ignored `artifacts/windows-distribution`
directory. They include the expanded bundle, deterministic ZIP, ZIP checksum,
standalone release manifest, payload checksums, and validation receipt.

The ZIP is reproducible for identical input bytes, revisions, version, and
source-date epoch. It is an unsigned developer artifact unless the signed
release workflow verifies valid Authenticode signatures for all four
executables.

## Inspect and install

Validate a bundle without contacting a network service:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\packaging\windows\Test-WindowsBundle.ps1 `
  -BundleRoot <expanded-bundle>
```

The installer is planning-only unless `-Execute` is explicitly supplied:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File <expanded-bundle>\distribution-tools\Install-RustyFleet.ps1 `
  -Action Install
```

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

GitHub Releases is the binary source of truth. The release workflow downloads
the exact external provider, builds the three Fleet executables, optionally
signs the Fleet executables, verifies all signatures for a signed release,
builds the bundle, and publishes the ZIP plus hash-bound metadata.

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
