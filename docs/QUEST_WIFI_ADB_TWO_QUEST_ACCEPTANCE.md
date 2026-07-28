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
- Agent Board owns machine-wide exclusivity. Fleet owns the private
  two-resource receipt and its run/slot/device binding, not the Board database
  or lease authority.
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
- the absolute pinned Agent Board PowerShell wrapper, its SHA-256, and a
  bounded 10-minute-to-8-hour lease duration;
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
duplicate seeds. Before every coordination command the runner opens the fixed
local-volume root, every ancestor, and the wrapper itself through retained
Windows handles that permit owner reads but deny write/delete sharing. It
rejects reparse components and multiply-linked wrapper files, binds every
component's volume/file identity plus the wrapper SHA-256 before launch, keeps
the handles open while `pwsh` reads the script, and verifies the complete
identity chain and hash again after process completion. This closes
hash-to-process-open modification, replacement, ancestor rename/junction, and
hardlink-substitution races.

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
state; a fresh authenticated Kiosk direct-route status; helper boot/in-flight
state; `adb_wifi_enabled`; classic and TLS ADB port properties; exact listening
socket, session, and pending state; target-scoped host forwards and device
reverses; bounded Android ADB-manager digest; USB transport; boot ID digest and
elapsed clock; Termux process epoch; Fleet Agent process state; and absence of
Fleet Agent app-private inputs. Unknown readback rejects. Any initially active
Wi-Fi setting, listener, TLS/classic port, session, pending request, tunnel,
after-boot request, existing Fleet profile, or pre-existing Agent state also
rejects because the transaction cannot promise exact attended restoration.
After the Hub and signed Agents are active, a Fleet `status` operation must
return a fresh direct Kiosk provider receipt; profile enrollment alone is not
treated as direct-link evidence.

The ADB-manager readback is not free-form text matching. Fleet accepts only the
closed `android.debugging_manager.text.v1` envelope emitted by the reviewed
AOSP/Quest `dumpsys adb` implementation, parses its version-1 keystore with DTDs
disabled, and retains only a stable digest plus bounded pairing/trusted-network
facts. Unknown fields, duplicate fields, an unknown envelope/version, malformed
XML, or inconsistent user/keystore keys make the manager projection `unknown`.
The v1 dump does not itself expose a TLS port, connected Wi-Fi key set, or
pairing-in-progress flag, so Fleet never derives those facts from
`connected_to_adb`.

Actual dynamic listener and pending-pairing ports come from the ADB owner's
closed `adb mdns services` projection, target-bound by the Quest's current
global IPv4 address or platform-serial service prefix. A closed
`rusty.fleet.adbd_socket_owner.v3` readback resolves the exact `adbd` process's
socket FDs to `/proc/net/tcp*`. Each complete sample captures the PID and
`/proc/<pid>/stat` start time, a full pre-table socket-FD inode set, both TCP
tables, a full post-table socket-FD inode set, and the identity again. The two
FD sets and process identity must match within the sample. Fleet then requires
two consecutive complete samples with identical identities, inode sets, and
owned TCP row/state projections before deriving listener/session facts.
A late listener or accepted session, close/reopen on a reused port, any FD
churn, a state change on the same inode, partial TCP6 read, PID reuse, or
second-sample right-edge churn therefore remains `unknown`, never `absent`.
Any unknown manager, mDNS, or socket-owner grammar fails Preflight and cannot
satisfy terminal cleanup. Because AOSP v1 omits pairing-in-progress, missing
mDNS while Wireless Debugging remains enabled also stays `unknown`; only an
observed pairing service is `pending`, and only disabled owner flags can close
it as `absent`. Final cleanup also requires the retained
pairing/trusted-network digest to equal its initial value.

This correction advances private run config from v1 to v2 and acceptance state
from v4 to v5. A v4 state cannot be migrated safely: it has no exact private
Agent Board receipt, and its v2 socket-owner capture could omit right-edge FD
churn and preserve a false `absent` fact. The current runner rejects v4 before
resume. Finish cleanup with the exact runner checkout that created that v4
state, retain its cleanup evidence, and begin v5 with a fresh absent private
state root and a v2 config. Do not copy or synthesize v5 fields. Older states
retain their previously documented exact-checkout cleanup requirement.

`Execute` and each `Resume` perform one durable transition. Both require
`-ConfirmMutation`. Before Execute, the runner obtains fresh external
reservations for both exact `quest:<usb-serial>` resources. The private receipt
binds each lease ID to the run ID, logical slot, Fleet device ID, USB serial,
resource, owner, task, reason, and deadline. Resume heartbeats and validates
both exact leases before any device-touching transition. Every durable
mutation repeats that validation before its isolation readback and again
immediately before dispatch. Reusing a different config rejects before resume.

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
   Fleet-owned admission. Trusted ingress consumes the signed proof for
   exactly one already-applied request operation and exact owner receipt before
   any projection. The durable consumption binding rejects reuse for an
   earlier or later request, including after adapter snapshot/restore. The
   admission binds the expected device, operation,
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
boot identity and monotonic uptime, QFM profile/direct route, helper boot and
in-flight state, installed package and permission facts, managed-process and
Agent private-input state, Termux process epoch, Wi-Fi setting, TLS/classic
ports, listener/session/pending state, ADB-manager digest, host
forward/reverse state, and exact USB transport. Its accepted check-in revision
may only advance monotonically. Pre-agent package/profile/permission
provisioning uses the same complete physical projection for the non-target.

## State and evidence

The private state file is resumable but sanitized. It stores logical slots,
hashes, bounded states (including tri-state claims), accepted revisions,
operation IDs, stable reason codes, bounded event history, run-owned inventory,
cleanup truth, and only `bound`, `expired`, or `released` for Agent Board
coordination. Lease IDs, resources, serials, owners, reasons, and deadlines
remain solely in the private `agent-board-reservation.json` receipt. `Status`
can downgrade a stale `bound` deadline to `expired` in its returned projection
without mutating durable state or querying Board. Its schema
rejects unknown and duplicate state fields. Before every write the runner
rejects any configured path, serial, device ID, endpoint, enrollment path,
profile path, seed path, or artifact path that appears in serialized state.

Every runner-owned mutation is recorded and durably published before dispatch,
then recorded as `sent_outcome_unknown` before the owner can acknowledge it,
and becomes `confirmed` only after exact owner/effective readback. Journal
records bind target, boot, proof, action, artifact pin, request, cleanup owner,
and the previous digest. Each target-scoped record also binds complete
before/after physical projections for the non-target; host-only mutations bind
both devices. These projections cover boot/uptime, QFM route/profile, helper,
permissions, packages, Agent process/private inputs, Termux epoch,
listener/session/pending state, tunnels, ADB-manager digest, and USB transport.
Interruption leaves `prepared_not_sent` or
`sent_outcome_unknown`; neither is redispatched. Resume converts either to
`cleanup_required`. Cleanup closes it as `terminal`, and acceptance/cleanup
receipts bind the final journal head.

Every cleanup effect has its own prepared, sent, confirmed or
cleanup-required/terminal journal record and is persisted independently. On
process restart an ambiguous cleanup record is terminalized without
redispatch; cleanup continues to fresh final readback and reports partial or
unknown truth instead of guessing the interrupted outcome.

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
- freshly confirm the configured Kiosk direct route and restore the QFM
  profile state;
- remove only the exact run-owned firewall rule and verified runtime stage;
- invoke the inventory-bound onboarding cleanup after the Hub stops.

Cleanup requires both exact Board reservations too. If one expired, Cleanup
may retain the still-active slot and reserve only the missing exact resource,
then writes a newly bound private bundle before device access. A partial
cleanup retains the bundle. Only after complete terminal cleanup and its final
receipt are durably written does the runner release both leases. Partial
release is projected as `expired` and a later Cleanup retries release without
redispatching device cleanup.

Seed deletion is not Manifold authorization revocation. Use the onboarding
owner's separate `revoke-plan` and the Manifold owner's authorized workflow
when enrollment revocation is required.

The terminal readback freshly re-reads packages and permissions, helper boot
and in-flight state, Wireless Debugging setting and actual listener/session,
Agent process and private inputs, QFM profile/direct route, boot identity,
host tunnels, Hub PID, firewall, onboarding material, and runtime stage.
`complete` means every cleanup fact passed. A complete cleanup changes the
installed/reachable/authorized/effective acceptance claims to `not_claimed`;
it never retains a positive runtime claim. Any missed fact becomes `partial`;
an unreadable final fact becomes `unknown`, and status remains
`cleanup_partial_failure`.

## Validation

Modeled host conformance plus integrated synthetic signed-Hub checks only:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\Test-FleetWifiAdbTwoQuestAcceptance.ps1
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\Test-Repo.ps1 -Tier Quick
```

The synthetic suite covers strict config/state parsing, unknown/duplicate
fields, parent-junction rejection, wrong hashes, duplicate and cross-device
bindings, signed-admission UID/freshness/device/operation/hash/key/revision/
unsigned/replay rejection, durable prepared/sent/confirmed crash models, a
real module process-boundary reload, no-redispatch recovery, exact-operation
proof consumption and snapshot/restore rejection of cross-operation reuse,
digest-chain tampering, write-through publication, private two-lease
acquisition/reload/revalidation, wrong-resource and expired-lease rejection,
single-slot repair, deadline-only status downgrade, pre-dispatch heartbeats,
release-before-cleanup rejection, partial release retry, deterministic
in-place/leaf/ancestor-junction/hardlink substitution attempts at the pinned
wrapper launch boundary, within-sample late listener/session, close/reopen,
FD/state/PID/TCP6/right-edge churn,
resume mismatch, partial cleanup truth, parser checks, and forbidden
ADB/approval surfaces. It does not touch a device or claim a live pass.
