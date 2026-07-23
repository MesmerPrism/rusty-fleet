# Public / Private Boundary

Rusty Fleet is public. Commit reusable architecture, contracts, synthetic
fixtures, source, tests, and placeholder-based operating guidance.

Do not commit:

- workstation paths or repository maps;
- real device serials, hardware identifiers, endpoints, SSIDs, or account IDs;
- credentials, tokens, keys, certificates, pairing or enrollment material;
- private package identities, launch activities, signing configuration, or
  product payload details;
- APKs, AABs, keystores, logs, screenshots, recordings, traces, media frames,
  databases, or raw fleet exports;
- private study logic, tuning, content, or participant data.

Use placeholders such as `<project-root>`, `<work-environment-root>`,
`<quest-serial>`, `<package>`, `<endpoint>`, and `<out-dir>`.

Public evidence should state the contract, scenario, result, tool category,
cleanup result, and artifact type. Keep exact local values and raw artifacts in
ignored `local/` or `artifacts/` locations.
