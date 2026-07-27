# Two-Quest Wi-Fi ADB Acceptance

`Invoke-FleetWifiAdbTwoQuestAcceptance.ps1` is Fleet's resumable, attended
acceptance transaction for the modern-TLS Wi-Fi ADB path. It coordinates
owner tools; it does not absorb their authority.

## Boundary

- Fleet owns the two-device order, immutable target binding, checkpoints,
  acceptance matrix, sanitized resumable state, and cleanup truth.
- `fleet-onboard` owns offline generation of distinct Fleet Agent seeds,
  profiles, public enrollment records, and private Hub configuration.
- Fleet Hub and Manifold own signed check-in admission and command
  authorization.
- QuestIonAble File Manager owns private Fleet-device-to-USB/Kiosk resolution,
  inspected APK installation, and its exact connectivity-provider effects.
- Rusty Kiosk and its same-signer setup helper own the supported Wi-Fi ADB
  setting request and readback.
- The Wireless ADB recovery helper owns fixed modern-TLS discovery, Termux
  preparation, loopback execution, and the short-lived exact-shell proof.
- Rusty Quest's Fleet Agent authenticates, sanitizes, signs, and publishes that
  owner proof. It is non-sticky and requires an explicit relaunch after reboot.
- Meta owns the protected wearer prompt. The runner never presses, bypasses,
  infers, or fabricates approval.

Plan, generated files, profile import, provider delivery, setting application,
listener discovery, wearer approval, signed proof admission, usability,
expiry, renewal, disable, loss, recovery, and cleanup remain separate facts.

## Private input

Make a private copy of
[`private-run-config.example.json`](../fixtures/wifi-adb-two-quest/private-run-config.example.json)
and replace every placeholder. The completed config must remain outside the
checkout. It is the runner's only private input and contains:

- exact reviewed QFM and helper source commits;
- SHA-256 pins and private paths for the ten closed owner artifacts;
- a private offline-onboarding request and expected inventory;
- the future private Hub-config path plus loopback operator origin;
- exactly two distinct logical slots, Fleet device IDs, identity revisions,
  USB serials, QFM enrollment documents, and generated profile/seed paths; and
- finite acceptance deadlines.

Do not pass serials, device IDs, endpoints, pairing codes, APK paths, seeds,
Hub paths, or tool paths as separate CLI arguments.

The parser rejects unknown and duplicate JSON properties, extra artifacts,
wrong hashes, wrong source commits, nonlocal or reparse input paths, duplicate
slots/identities/serials, partial profile/seed pairs, cross-device QFM
enrollments, wrong profile identities, seed lengths other than 32 bytes, and
duplicate seeds.

`Preflight` requires the expected inventory, Hub config, and both profile/seed
pairs to be absent. This gives the transaction exclusive, auditable ownership
of the onboarding output set; it never overwrites or later cleans up material
from another run. It also rejects an existing QFM Fleet profile, Fleet Agent
process, or Fleet Agent app-private input directory. The run therefore never
adopts a pre-existing process and cleanup never stops one.

## Phases

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\Invoke-FleetWifiAdbTwoQuestAcceptance.ps1 `
  -Action Plan -RunConfig <private-run-config>

pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\Invoke-FleetWifiAdbTwoQuestAcceptance.ps1 `
  -Action Preflight -RunConfig <private-run-config>

pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\Invoke-FleetWifiAdbTwoQuestAcceptance.ps1 `
  -Action Execute -RunConfig <private-run-config> -ConfirmMutation

pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\Invoke-FleetWifiAdbTwoQuestAcceptance.ps1 `
  -Action Resume -RunConfig <private-run-config> -ConfirmMutation
```

`Plan` verifies private config, source pins, artifacts, and static two-device
bindings. It creates no directory, file, process, listener, rule, profile,
seed, package, or device effect.

`Preflight` performs read-only owner and exact-serial observations. Only after
all config/source/hash/duplicate/static checks and both device snapshots pass
does it create a new current-user-only private state root. It snapshots the
complete installed-package-set digest; relevant package presence and signer
inspection; helper grants; QFM connectivity-profile state; Kiosk installation
state; after-boot setting; Wi-Fi ADB setting; USB transport; boot ID digest and
elapsed clock; Termux process epoch; Fleet Agent process state; and absence of
Fleet Agent app-private inputs. Initially enabled Wi-Fi ADB, enabled
after-boot request state, an existing Fleet profile, or pre-existing Agent
state rejects because the transaction cannot promise exact attended
restoration. After the Hub and signed Agents are active, a Fleet `status`
operation must return a fresh direct Kiosk provider receipt; profile enrollment
alone is not treated as direct-link evidence.

`Execute` and each `Resume` perform one durable transition. Both require
`-ConfirmMutation`. Reusing a different config rejects before resume.

The transaction stops with one of five typed attended checkpoints:

- `awaiting_kiosk_direct_link`: create or restore the private Kiosk link in
  File Manager, then confirm the current checkpoint;
- `awaiting_termux_bootstrap`: enable Termux's external-app setting once
  inside Termux or restore package-network access, then retry;
- `awaiting_termux_restart`: restart Termux after the helper has successfully
  prepared and read back its fixed prerequisites. Resume requires an observed
  nonzero process epoch different from the pre-checkpoint epoch; confirmation
  alone is insufficient;
- `awaiting_wearer_approval`: accept Meta's protected prompt physically; or
- `awaiting_attended_reboot`: reboot the named logical slot outside the
  runner, wait for exact USB readiness, and confirm so the runner can prove the
  Fleet Agent was non-sticky and relaunch it explicitly. Resume requires a
  changed kernel boot ID digest, reset elapsed clock, and then a fresh signed
  check-in with a changed source epoch; confirmation or Agent absence alone is
  insufficient.

Resume an attended checkpoint only after performing that exact action:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\Invoke-FleetWifiAdbTwoQuestAcceptance.ps1 `
  -Action Resume -RunConfig <private-run-config> `
  -ConfirmMutation -ConfirmCurrentCheckpoint
```

The runner never starts, stops, kills, or reconnects the ADB daemon. Every ADB
call includes the exact configured serial. Its internal vectors are closed:
offline onboarding plan/apply/cleanup; Hub `--config`; Fleet Agent's fixed
app-private inputs and debug start/stop actions; File Manager profile status,
import, revoke, APK inspect, install, and Kiosk status; helper status,
preparation, restore, disable-boot, and disable-wireless; and Fleet Wi-Fi
preview/execute/get plus device inspection. No raw command, generic arguments,
arbitrary package, target, component, remote path, URL, port, or setting is
accepted.

Kiosk itself is a separately distributed prerequisite and is never installed
or removed by this transaction. The runner may install the fixed same-signer
Kiosk setup helper, grants only its documented secure-settings permission, and
restores that grant or removes the run-added package during cleanup.

## Acceptance matrix

The transaction requires:

1. distinct generated seeds, profiles, public keys, and Hub enrollments;
2. two fresh signed baseline check-ins;
3. request device A while device B remains fresh and unchanged;
4. an explicit wearer checkpoint followed by the exact Hub operation
   projection containing both the Quest Termux Lab owner proof and a separate
   Fleet-owned admission. The admission binds the expected device, operation,
   owner receipt, proof, source/evidence revisions, enrolled key generation,
   canonical JCS/signing/signature digests, Manifold/Fleet revisions, expiry,
   and deterministic lineage digest. The owner route is modern TLS, proof
   lifetime is at most 60 seconds, and shell identity is exactly
   `uid=2000(shell)`;
5. proof expiry and loss of usability;
6. renewal at a strictly higher evidence revision;
7. owner-applied disable plus signed proof disappearance;
8. attended reboot proven by boot-bound evidence, observed non-sticky Agent
   loss, explicit relaunch, and fresh signed source-epoch recovery;
9. the same sequence for device B without cross-device substitution; and
10. a complete two-device isolation projection.

A stale, unsigned, wrong-device, cross-operation, wrong-key/revision,
wrong-identity-revision, wrong-UID, non-modern, undiscovered, unavailable,
overlong, replayed, digest-damaged, or non-advancing proof/admission fails
closed. There is no proof submission route.

Before and after every Wi-Fi/proof/reboot transition, the runner captures fresh
signed projections for both devices. The non-target device must retain the
same source epoch, operation set, effective Wi-Fi/receipt/proof lineage,
installed package facts, managed-process presence, Wi-Fi setting, and exact
USB transport. Its accepted check-in revision may only advance monotonically.

## State and evidence

The private state file is resumable but sanitized. It stores logical slots,
hashes, booleans, accepted revisions, operation IDs, stable reason codes,
bounded event history, run-owned inventory, and cleanup truth. Its schema
rejects unknown and duplicate state fields. Before every write the runner
rejects any configured path, serial, device ID, endpoint, enrollment path,
profile path, seed path, or artifact path that appears in serialized state.

Every runner-owned mutation is recorded and durably published before dispatch,
then recorded as `sent_outcome_unknown` before the owner can acknowledge it,
and becomes `confirmed` only after exact owner/effective readback. Journal
records bind target, boot, proof, action, artifact pin, request, cleanup owner,
and the previous digest. Interruption leaves `prepared_not_sent` or
`sent_outcome_unknown`; neither is redispatched. Resume converts either to
`cleanup_required`. Cleanup closes it as `terminal`, and acceptance/cleanup
receipts bind the final journal head.

State publication uses a randomized same-directory create-new file, content
flush, reparse check, and Windows `MoveFileExW` with replace and write-through
flags. This makes both bytes and the replacement publication durable before
the next owner action.

The runner never persists raw ADB/QFM/helper/Fleet output, process arguments,
pairing data, tokens, seeds, signer material, endpoints, or local paths.
Sanitized plan and preflight state explicitly claim no installation,
reachability, authorization, or effect. Acceptance is not terminal until
cleanup is complete.

## Cleanup

Cleanup is independently confirmed and safe to rerun:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\Invoke-FleetWifiAdbTwoQuestAcceptance.ps1 `
  -Action Cleanup -RunConfig <private-run-config> -ConfirmMutation
```

It attempts every bounded cleanup step even after a failure:

- disable the helper's after-boot attempt and Wireless Debugging;
- require the Fleet-signed proof/listener projection to disappear or expire;
- stop only Fleet Agents and the Hub started by this run;
- remove only run-staged Agent inputs;
- revoke only QFM profiles created by this run;
- restore helper grants for pre-existing helpers;
- uninstall only fixed packages absent in the preflight snapshot and added by
  this run;
- leave Kiosk direct-link settings untouched and restore the QFM profile state;
- remove only the exact run-owned firewall rule and verified runtime stage;
- invoke the inventory-bound onboarding cleanup after the Hub stops.

Seed deletion is not Manifold authorization revocation. Use the onboarding
owner's separate `revoke-plan` and the Manifold owner's authorized workflow
when enrollment revocation is required.

`complete` means every cleanup fact passed. Any missed fact remains
`cleanup_partial_failure`; it is never rewritten as success.

## Validation

Host and synthetic checks only:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\Test-FleetWifiAdbTwoQuestAcceptance.ps1
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\Test-Repo.ps1 -Tier Quick
```

The synthetic suite covers strict config/state parsing, unknown/duplicate
fields, parent-junction rejection, wrong hashes, duplicate and cross-device
bindings, signed-admission UID/freshness/device/operation/hash/key/revision/
unsigned/replay rejection, durable prepared/sent/confirmed crash models,
no-redispatch recovery, digest-chain tampering, write-through publication,
resume mismatch, partial cleanup truth, parser checks, and forbidden
ADB/approval surfaces. It does not touch a device or claim a live pass.
