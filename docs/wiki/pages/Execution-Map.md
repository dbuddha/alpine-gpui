# Alpine Studio execution map

> Retrieval mirror synchronized from Alpine `main` revision
> `{{ALPINE_MAIN_REVISION}}`. The repository mdBook is canonical.

Alpine Studio is a local-only, Apple Silicon editor proving Alpine's own Direct
Metal UI framework. Private daily-driver readiness means safe and smooth
sustained Alpine repository work with correct native interaction, selected Rust
intelligence, bounded residency, no idle redraw, and no known data-loss defect.

Stable dependency graph:

```text
M0 governed foundation supports every path

M2 native presentation -> M4 input, IME, accessibility -> M5 daily driver -> M7
M3 local workspace shell -------------------------------> M5 daily driver
M1 semantic renderer -> realistic traces -> E4 claims --------------------> M7

M6 non-macOS platforms is independently deferred.
```

The current execution sequence is retrieved from:

- [Capability #28](https://github.com/dbuddha/alpine-gpui/issues/28) for the
  accepted daily-driver outcome.
- [Project #1](https://github.com/users/dbuddha/projects/1) for live priority and
  blockers when the reader has access.
- [GitHub Milestones](https://github.com/dbuddha/alpine-gpui/milestones) for live
  outcome cohorts.
- [Typing latency #304](https://github.com/dbuddha/alpine-gpui/issues/304) and
  [physical experiment #331](https://github.com/dbuddha/alpine-gpui/issues/331)
  for the first measured product blocker.
- [Input and accessibility #37](https://github.com/dbuddha/alpine-gpui/issues/37),
  [language intelligence #34](https://github.com/dbuddha/alpine-gpui/issues/34),
  and [settings #36](https://github.com/dbuddha/alpine-gpui/issues/36) for the
  remaining accepted behavior families.
- [Renderer task #61](https://github.com/dbuddha/alpine-gpui/issues/61) and
  [qualification #38](https://github.com/dbuddha/alpine-gpui/issues/38) for the
  separate GPUI evidence path.

Issue timelines and Project fields own live state. This page does not copy issue
counts, readiness percentages, due dates, or Project status. M5 does not require
renderer superiority. Comparative claims remain blocked until semantically
equivalent E4 evidence exists.

- [Implementation lineage](Research-Lineage)
- [Rendering doctrine](Rendering-Doctrine)
- [Comparator qualification](Comparator-Qualification)

Canonical source: [private daily-driver path](https://github.com/dbuddha/alpine-gpui/blob/{{ALPINE_MAIN_REVISION}}/docs/project/daily-driver-path.md)
