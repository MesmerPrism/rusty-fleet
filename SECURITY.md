# Security Policy

Rusty Fleet controls network-visible devices and must treat identity,
authorization, replay protection, command expiry, audit, and evidence as
product behavior.

Do not open a public issue for a suspected vulnerability, exposed credential,
pairing secret, operator token, device identity leak, or unsafe remote-control
path. Use the repository's private GitHub security-advisory flow.

The planning baseline is not a deployed service. Supported versions and
response expectations will be added before the first runtime release.

Security-sensitive milestones require Deep validation, negative fixtures,
rollback evidence, and an explicit threat-model update. A successful transport
acknowledgement is never proof that a command was authorized or applied.
