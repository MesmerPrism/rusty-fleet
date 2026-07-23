# Fleet UI Product-Pattern Matrix

## Purpose

This matrix records externally observable product patterns that informed
[the Rusty Fleet operator UI](../OPERATOR_UI.md). It is design pressure, not a
feature-comparison claim or permission to copy proprietary implementation.

The sources were inspected on 2026-07-23. The authoritative link and
provenance details are in
[the source ledger](FLEET_UI_SOURCE_LEDGER.md).

## Product and system patterns

| Reference | Useful observed pattern | Rusty Fleet adaptation | Failure to avoid |
| --- | --- | --- | --- |
| Microsoft Intune | dense all-device reports, column choice, filters, timestamps, aggregate plus record views, bulk actions | timestamped fleet table with inspectable rows and role-filtered scope | treating server-side `Completed` as device-applied success |
| ManageXR | device list search/filter/sort, independent online/sync/battery/activity states, comma-separated identifiers, bulk selection, per-device detail | visible facet logic, exact-ID paste, separate state families, inspector/full detail | copying undocumented filter semantics or treating every missing optional permission as failure |
| ArborXR | aggregate deployment states with per-device drill-down | operation summary counts that always open a per-target ledger | aggregate deployment success without target evidence |
| Fleet | dense host inventory, API filters, explicit target search, batch-by-ID/filter, preserved navigation context | canonical Hub query, inspectable target snapshot, exact navigation restoration | action-by-filter without freezing or exposing membership |
| Azure IoT Hub twins | desired versus reported state, per-field metadata/versioning, last-known device state | keep proposals separate from device reports; include source/revision/freshness | adopting Azure's product schema or treating a last-known value as current |
| Kubernetes conditions | condition arrays with type, status, reason, message, and transition time | independent fleet condition families with machine and human reasons | importing Kubernetes readiness semantics or collapsing families into one `Ready` |
| AWS IoT Jobs | staged rollout, rates, scheduling, maintenance windows, abort, timeout, retry, and distinct end behavior | risk-aware rollout policy with rings, limits, expiry, cancellation, and cleanup | assuming abort or end-of-rollout proves in-progress device work stopped |
| AWS dynamic thing groups | query-derived membership and preview | make dynamic scheduled selection explicit and preserve planned plus actual targets | silently changing membership after operator confirmation |
| Grafana alerting | `No Data` and `Error` are separate states | unknown, missing, stale, adapter-error, and device-failure remain distinct | presenting missing data as normal or failed device state |
| Elastic saved queries | query text, filters, and time range are reusable state | versioned saved views over one canonical query expression | saving only a label while losing actual scope |
| PagerDuty and Google SRE | group related alerts, suppress transient noise, favor actionable signals | adapter/root-cause incidents with affected-device counts and maintenance policy | one alert per affected headset for a shared cause |
| Cisco Meraki upgrades | staged groups and explicit warning that `Completed` need not mean successful | distinguish operation-stage completion from owner-applied and cleanup evidence | reusing a generic terminal word without defining evidence |
| WPF and Windows guidance | list/details, left navigation for many categories, DataGrid behavior, UI virtualization, UI Automation | native list/detail semantics, explicit grid spike, Hub-side data windowing | assuming a theme package supplies accessibility or data virtualization |
| W3C guidance | grid keyboard model, non-color meaning, programmatic status updates | test equivalent native WPF behavior through UI Automation and Narrator | copying ARIA roles literally into WPF |

## Accepted lessons

- The inventory table is the home surface.
- Aggregate counts are navigation/filter aids, not substitutes for records.
- State is multidimensional and timestamped.
- Saved scope and navigation restoration are contracts.
- Batch membership and exclusions are inspectable before dispatch.
- Server, transport, device application, and cleanup completion are different.
- Missing, stale, unsupported, unauthorized, unavailable, and failed are
  different.
- Alert grouping should follow root cause and impact.
- Accessibility and scale are milestone acceptance concerns.

## Adapted lessons

- Web-oriented ARIA and WCAG guidance is used as an interaction and
  requirements reference; native WPF UI Automation remains authoritative.
- WinUI list/detail and navigation examples inform the layout, but the
  implementation remains WPF.
- Product-specific state labels inspire independent condition families without
  importing their schemas or thresholds.
- Cloud job and dynamic-group behavior informs target snapshots and rollout
  policy without making a cloud service a runtime dependency.

## Rejected overreach

- No product screenshot, layout, source, icon, or proprietary behavior is
  copied.
- No third-party system becomes Rusty Fleet authority.
- No single health score is adopted.
- No current library version is pinned by this research.
- No candidate performance threshold or fleet size is accepted without local
  measurement.
- No accessibility claim is inherited from a framework or theme library.
- No ambient all-device media wall becomes the default fleet workspace.
