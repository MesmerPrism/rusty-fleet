# Fleet UI Reference and Provenance Ledger

## Method

The linked public resources were inspected on 2026-07-23. This ledger records
why each reference matters, the lesson borrowed, the overreach rejected, the
target Rusty Fleet layer, and the remaining validation.

External products supply observable patterns only. No implementation source or
proprietary visual asset is copied. Time-sensitive version and maintenance
claims must be refreshed at the milestone that selects a dependency.

## HCI, Windows, and accessibility

| Reference | Lesson borrowed | Overreach rejected | Target / follow-up |
| --- | --- | --- | --- |
| [The Eyes Have It](https://drum.lib.umd.edu/items/155a868e-fb83-4115-9899-9187ea8c0498) | overview, filtering, details on demand, history, and extract are useful starting tasks | the 1996 mantra is not a complete modern operations UI specification | information architecture; add authority, freshness, and operation layers |
| [Windows list/details](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/list-details) | side-by-side list/detail at adequate width and stacked behavior when narrow | WinUI controls are not WPF implementation requirements | Console layout; validate focus and navigation restoration |
| [Windows NavigationView](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/navigationview) | left navigation fits many stable top-level categories and adapts by width | adopting WinUI or duplicating navigation inside Fleet | Console shell; implement equivalent WPF behavior |
| [WPF DataGrid](https://learn.microsoft.com/en-us/dotnet/desktop/wpf/controls/datagrid) | native sorting, selection, grouping, resizing, frozen columns, and keyboard foundations | assuming defaults meet Rusty Fleet semantics or accessibility | M1 primary-grid spike |
| [WPF control performance](https://learn.microsoft.com/en-us/dotnet/desktop/wpf/advanced/optimizing-performance-controls) | UI virtualization, recycling, container-state hazards, and absence of built-in data virtualization | claiming large-fleet support from UI virtualization alone | M0 Hub windowing contract and M1/M6 measurement |
| [Windows accessibility checklist](https://learn.microsoft.com/en-us/windows/apps/design/accessibility/accessibility-checklist) | keyboard, names, contrast, high contrast, Narrator/tooling, and automated regressions | inheriting conformance from stock controls | every WPF milestone |
| [WAI-ARIA grid pattern](https://www.w3.org/WAI/ARIA/apg/patterns/grid/) | bounded tab sequence and managed arrow-key navigation for large interactive grids | applying web ARIA roles literally to WPF | equivalent keyboard/UIA tests |
| [WCAG 2.2 use of color](https://www.w3.org/WAI/WCAG22/Understanding/use-of-color) | text or shape must supplement color | treating WCAG CSS units as literal WPF DIPs | status grammar and screenshot/accessibility gate |
| [WCAG 2.2 status messages](https://www.w3.org/WAI/WCAG22/Understanding/status-messages) | important background changes should be programmatically available without moving focus | announcing every telemetry update | coalesced UIA live-status policy |
| [Fluent System Icons](https://github.com/microsoft/fluentui-system-icons) | MIT-licensed vector/SVG source suitable for a semantic icon map | unexplained icon-only status or library-owned semantics | dependency/license review plus accessible names |
| [Segoe Fluent Icons](https://learn.microsoft.com/en-us/windows/apps/design/iconography/segoe-fluent-icons-font) | font availability and redistribution constraints must be explicit | essential meaning dependent on an installed glyph font | do not select as sole status-icon source |

## State, query, and operation systems

| Reference | Lesson borrowed | Overreach rejected | Target / follow-up |
| --- | --- | --- | --- |
| [Kubernetes Pod conditions](https://kubernetes.io/docs/concepts/workloads/pods/pod-condition/) | independent condition entries with type, status, transition time, reason, and message | Kubernetes readiness or schema ownership | M0 canonical condition contract |
| [Azure IoT Hub device twins](https://learn.microsoft.com/en-us/azure/iot-hub/iot-hub-devguide-device-twins) | desired and reported state are separate; reported fields have metadata and versions | using a cloud twin as Fleet authority or audit log | M0 proposal/report/freshness model |
| [AWS IoT Jobs configuration](https://docs.aws.amazon.com/iot/latest/developerguide/jobs-configurations-details.html) | rollout rate, stages, maintenance windows, timeout, retry, abort, and distinct cancellation behavior | importing AWS job statuses or assuming abort stops in-progress work | M2 target and operation policy |
| [AWS dynamic thing groups](https://docs.aws.amazon.com/iot/latest/developerguide/dynamic-thing-groups.html) | preview query-derived membership; distinguish static and dynamic target sets | silently mutable batch membership | M2 scheduled-dynamic selector contract |
| [Fleet REST API](https://fleetdm.com/docs/rest-api/rest-api) | explicit target search, filters, access-aware membership, and batch-by-ID/filter | treating a filter request as an immutable target receipt | M0 query and M2 target snapshot |
| [Fleet navigation improvements](https://fleetdm.com/releases/fleet-4-24-0) | loss of tab/filter context was a documented usability defect | copying Fleet navigation structure | foundational view-restoration contract |
| [Elastic saved queries](https://www.elastic.co/docs/explore-analyze/query-filter/tools/saved-queries) | save query text, filters, and time range as reusable scope | reducing saved scope to a display name | versioned saved-view contract |
| [Grafana No Data and Error](https://grafana.com/docs/grafana/latest/alerting/fundamentals/alert-rule-evaluation/nodata-and-error-states/) | missing data and evaluation failure are separate | importing Grafana alert rules | Fleet missing/stale/adapter-error grammar |

## Fleet-management product observations

| Reference | Lesson borrowed | Overreach rejected | Target / follow-up |
| --- | --- | --- | --- |
| [ManageXR device navigation](https://help.managexr.com/en/articles/5317137-navigating-your-devices) | search, filter, sort, explicit status families, exact-list paste, and bulk selection | copying proprietary UI or undocumented behavior | M1 query UX and M2 selection fixtures |
| [ManageXR device detail](https://help.managexr.com/en/articles/5417127-view-real-time-device-statuses-and-edit-device-details) | device list leads to detailed app, sync, battery, hardware, storage, and history facts | assuming every field is available without Quest authority/permission | inspector/full detail with truthful sources |
| [ArborXR deployment status](https://help.arborxr.com/en/articles/6342987-track-the-status-of-content-deployments) | aggregate deployment counts drill into device-level status | adopting deployment semantics as command authority | M2 operation summary and ledger |
| [Microsoft Intune reports](https://learn.microsoft.com/en-us/intune/device-management/reports/overview) | filters, timestamps, aggregate plus records, search/sort/paging/export, and independent states | cloud-report architecture as runtime dependency | fleet table and exports |
| [Microsoft Intune device actions](https://learn.microsoft.com/en-us/intune/device-management/actions/) | server `Completed` may not mean the client finished; bulk actions require explicit status review | Intune action vocabulary | command lifecycle evidence separation |
| [Cisco Meraki firmware upgrades](https://documentation.meraki.com/Switching/MS_-_Switches/Product_Information/Compatibility_and_Firmware/Meraki_Switching_Firmware_Upgrades) | staged rollout and explicit distinction between completed and successful | Meraki timing or firmware semantics | M2/M6 rollout and result vocabulary |

## Alerts and operations

| Reference | Lesson borrowed | Overreach rejected | Target / follow-up |
| --- | --- | --- | --- |
| [PagerDuty noise reduction](https://support.pagerduty.com/main/docs/noise-reduction) | alert grouping and transient-notification suppression reduce operator noise | PagerDuty algorithms or product dependency | M6 root-cause grouping and maintenance policy |
| [Google SRE monitoring](https://sre.google/workbook/monitoring/) | alerts tied to actionable outcomes reduce false positives | SRE service-level vocabulary as a Fleet schema | alert acceptance and usability studies |

## WPF dependency candidates

| Candidate | Observed evidence | Planning decision |
| --- | --- | --- |
| [dotnet/wpf](https://github.com/dotnet/wpf) | maintained .NET Foundation project under MIT; native WPF and vector high-DPI baseline | accepted platform baseline |
| [WPF UI](https://github.com/lepoco/wpfui) | active MIT Fluent-oriented shell/control library; current repository documents icon-font caveats | conditional shell candidate only after M1 accessibility/performance/removal spike |
| [MahApps.Metro](https://github.com/MahApps/MahApps.Metro) | mature MIT WPF styling project | comparison/fallback only; no selection |
| [MaterialDesignInXamlToolkit](https://github.com/MaterialDesignInXAML/MaterialDesignInXamlToolkit) | maintained MIT WPF control ecosystem with a different visual language | not the primary design-system candidate |
| [ModernWpf](https://github.com/Kinnara/ModernWpf) | MIT project whose latest GitHub release observed during review was from 2022 | reject as a new core dependency without renewed maintenance or owned fork |
| [FluentWPF](https://github.com/sourcechord/FluentWPF) | MIT project whose latest GitHub release observed during review was from 2021 | legacy reference, not a core dependency |

## Verification outcome

The references support the structural decisions recorded in
[Operator UI Architecture](../OPERATOR_UI.md). They do not establish:

- final row dimensions or visual tokens;
- a supported fleet-size ceiling;
- the candidate latency budgets;
- WPF UI or any other theme-library adoption;
- accessibility conformance;
- production alert thresholds;
- cloud, relay, or device runtime dependencies.

Those remain local milestone measurements and decisions.
