# Windows Mobile Hotspot control

Rusty Fleet exposes Windows Mobile Hotspot as one host-scoped resource:
`windows.mobile-hotspot`. It is not a headset setting and is not owned by the
file manager. Fleet owns operator intent, immutable preview/confirmation,
the bounded singleton lease, generation tracking, durable recovery, and the
redacted status projection. Manifold authorizes the typed host command.
Rusty Hostess owns the Windows API call and its private state.

The public action is `host.windows-mobile-hotspot`, with `status`, `start`,
`ensure`, and `stop` variants. The local API is:

- `POST /fleet/v1/windows-hotspot/preview`
- `POST /fleet/v1/windows-hotspot/{operation_id}/execute`
- `GET /fleet/v1/windows-hotspot/{operation_id}`

The matching CLI commands are `hotspot-preview`, `hotspot-execute`, and
`hotspot-get`. Preview never performs an effect. Execute must name the exact
unexpired preview.

## Provider boundary

Private local configuration pins an absolute
`rusty-hostess-hotspot-provider.exe` path, its lowercase SHA-256, and an
absolute private staging root. Every launch verifies the source artifact,
copies it into a unique private stage, re-verifies the staged copy, creates a
per-launch `bundle-extract` directory, invokes exactly
`integration windows-hotspot --json`, and removes the stage. Standard input is
one `rusty.hostess.windows_hotspot.provider_request.v1` object. Standard output
is one `rusty.hostess.windows_hotspot.provider_receipt.v1` object. Exit codes
are fixed: verified `0`, failed `1`, rejected `2`, unavailable `3`.

Fleet rejects alternate filenames, digest changes, unknown receipt fields,
binding changes, exit/result disagreement, stderr, oversized output, timeout,
and incomplete process-tree cleanup.

## Ownership and recovery

Hostess creates the ownership generation after a verified start. Fleet retains
that exact token and supplies it only to `ensure` and `stop`. Fleet never
adopts or stops an externally started hotspot. A status read can report an
external active hotspot without exposing SSID, passphrase, profile, addresses,
private endpoints, or filesystem paths.

An operation lease lasts no more than five minutes and is persisted in the Hub
snapshot. Confirmation atomically rechecks the singleton lease. On restart,
expired work becomes `expired`; an unexpired dispatched invocation retains its
exact request for retry. A Hostess `state.restart_detected` receipt invalidates
the old ownership generation. `ensure` and `stop` therefore cannot use a
pre-restart token. An explicit new `start` is the only recovery path, and
Hostess permits it only after fresh readback proves the hotspot is off.

The pinned Manifold runtime currently has no public lease-issuance transition.
Fleet therefore persists and enforces the host lease, while the lease identity
and generation are included in the canonical typed Manifold parameters.
Manifold remains the replay, expiry, command-policy, and audit authority. A
future Manifold lease API can move issuance there without changing the public
Fleet or Hostess contracts.

## Private configuration example

The following keys belong only in the ignored local Hub configuration:

```json
{
  "windows_hotspot_provider": {
    "executable_path": "C:\\private\\rusty-hostess-hotspot-provider.exe",
    "executable_sha256": "<64 lowercase hexadecimal characters>",
    "private_stage_root": "C:\\private\\rusty-fleet\\hotspot-stages"
  }
}
```

Never commit real paths, hashes tied to unreleased private artifacts, hotspot
profiles, credentials, or network identifiers.
