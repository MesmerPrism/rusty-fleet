# ADR 0005: Milestone 1 check-in authority

- Status: accepted for Milestone 1 implementation
- Date: 2026-07-23

## Context

Fleet needs useful local monitoring when no ADB route exists. A Quest-side
producer can observe its own power and lifecycle state, but neither a socket
peer nor a self-asserted device label is enrollment authority. Manifold already
owns accepted peer enrollment, credential rotation and revocation, bounded
low-rate status, authority revision, expiry, and replay rejection.

The check-in also crosses two state engines. Applying a Manifold proposal and
then discovering that Fleet rejects the paired observation would leave the
authorities inconsistent. Device-supplied received time would likewise confuse
source evidence with host ingress evidence.

## Decision

Use one signed `rusty.fleet.signed_checkin.v1` envelope containing:

- a `rusty.manifold.peer.status_proposal.v1` whose source-owned identity,
  proposal id, status revision, timestamps, capability ids, and payload class
  are exact signed evidence;
- one `rusty.fleet.device_observation.v1`;
- a bounded issue/expiry window and unique dotted check-in identifier;
- the active enrolled Manifold key identifier; and
- an Ed25519 signature over the v1 domain separator followed by RFC 8785/JCS
  canonical claims.

The adapter verifies the bounded Fleet contract, time window, replay set,
enrolled key status and validity, peer-to-observation identity binding, and
signature before either state engine may advance. A headset does not serialize
against Manifold's fleet-global authority revision. After verification, the
trusted ingress adapter binds only the proposal's
`expected_authority_revision` optimistic-lock field to current Manifold state.
This permits independently checking-in devices to be serialized at the
authority boundary without changing their signed status evidence.

Admission is transactional:

1. preview the observation against a cloned Fleet Hub;
2. reject without Manifold mutation when Fleet admission fails;
3. bind the authority-owned optimistic lock and review the peer proposal
   against current Manifold state;
4. reject without Fleet mutation when Manifold admission fails;
5. commit both accepted candidate states and the replay record together.

The Fleet adapter replaces the envelope's device-supplied `received_time_ms`
with host ingress time before Hub admission. Source time and each fact's
provenance remain signed device evidence; received time remains adapter
evidence.

The envelope grants observation only. It does not grant command, Kiosk, file,
ADB, media, recording, or relay authority. A device remains independently
useful in the base fleet when all privileged and high-rate capabilities are
absent.

## Consequences

- The base product supports authenticated no ADB monitoring.
- A valid device signature cannot bypass enrollment, revocation, Manifold
  revision, or status-expiry policy.
- Independent devices do not need to predict or coordinate the fleet-global
  Manifold authority revision.
- Field insertion order in independent Rust and Android producers cannot
  change the signed bytes.
- A Fleet rejection cannot consume a Manifold proposal revision.
- A Manifold rejection cannot create or update a Fleet device.
- Replay evidence is retained only for its live bounded window; source and
  Manifold revisions still provide longer-lived ordering and replay defense.
- The exact Manifold dependency remains pinned until an explicit compatibility
  review advances it.

## Rejected alternatives

- **Trust a Wi-Fi peer address as identity:** network location is not
  enrollment.
- **Use LSL discovery or `source_id` as identity:** LSL remains an optional
  scientific observation adapter.
- **Sign ordinary serializer output:** map and field order would become an
  undocumented cross-repository protocol.
- **Apply Manifold before checking Fleet:** a rejected observation could still
  consume authority state.
- **Trust device received time:** a producer cannot prove when the Hub received
  its message.
- **Require devices to predict the fleet-global authority revision:** parallel
  devices would race and create false stale-authority failures.
- **Require ADB for enrollment or check-in:** this would defeat the M1 base
  capability.
