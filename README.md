# Alpine GPUI

Alpine GPUI is a publicly readable, proprietary desktop application framework
written in Rust for applications that need predictable latency, bounded memory
use, and native desktop behavior. It is intended first for editors, terminals,
database tools, and other data-heavy productivity applications.

The programming model is conceptually adapted from
[Zed GPUI](https://github.com/zed-industries/zed/tree/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui),
with additional lessons drawn from GPUI-CE, `gpui-component`, WGPUI, the
`gpui-wgpu` lineage, Kael, and the wider GPUI ecosystem. Alpine is an
independent implementation, not a fork or source-compatible distribution. It
is not affiliated with or endorsed by Zed Industries.

The flagship target is Apple Silicon running macOS 15 or newer, using a direct
Metal backend. Direct Vulkan with Wayland on Linux and direct D3D12 with Win32
on Windows are later directions. Cross-platform contracts must preserve native
specialization rather than reduce Metal to a least-common-denominator API.

## Version 1 boundaries

Alpine aims to own its application runtime, demand-driven scheduling, immutable
scene protocol, renderer policy, resource lifetimes, native windowing, input,
text, accessibility, headless testing, and application-ready components.

Version 1 does not target Intel Macs, web, mobile, GPUI source compatibility,
or a generic GPU abstraction in the direct Metal hot path. Upstream source is
not copied, vendored, or linked. Source-level adaptation requires explicit
owner approval and conditional provenance records.

## Current maturity

Alpine is pre-release foundation software and is not ready for application
development. The workspace currently provides:

- `alpine-core`, with validated geometry and linear color values;
- `alpine-scene`, with immutable scene snapshots and painter-ordered quads;
- `alpine-renderer`, with a backend-generic renderer contract, capabilities,
  and observable frame reports;
- `alpine-metal`, with deterministic frame planning and a private Apple Silicon
  Metal device, offline pipeline, one synchronous command submission, and
  deterministic compact BGRA8 readback, cancellation, shutdown, device-loss
  generations, and complete frame-resource accounting;
- `alpine-platform`, with a portable demand-driven presentation transition
  system, and `alpine-platform-macos`, with one native AppKit window,
  callback-provided Direct Metal drawable submission, bounded compositor-drop
  retry, direct presentation, nonzero presented-time correlation, synchronized
  resize and display epochs, and an explicit standard-sRGB presentation path;
- non-shipping `alpine-trace` and `alpine-assurance` tooling that fail closed on
  malformed renderer workloads and can produce CPU-oracle or Direct Metal BGRA8
  artifacts from the same versioned solid-quad trace;
- a pinned Rust toolchain and locked dependency graph;
- policy, formatting, lint, unit test, doctest, rustdoc, coverage, changed-code
  mutation, selected Kani proofs, and three-platform CI;
- risk-selected Miri and Metal validation plus scheduled exhaustive assurance.

There is no shipping event-loop runtime, onscreen color capture or display-profile
qualification, resource cache, layout engine, input system, text stack, or
component system yet. The exact implemented boundaries and binding invariants
are documented in [Architecture](ARCHITECTURE.md).

The [engineering guide](docs/SUMMARY.md) owns durable mission principles, user
journeys, case-study conclusions, enhancement proposals, and the assurance
method. GitHub issues remain authoritative for active research, approvals, and
delivery state. The guide describes accepted knowledge, not a second roadmap.

## Development and project state

Run the repository acceptance gate:

```sh
scripts/check.sh
```

GitHub is the operational system for this project:

- [Project](https://github.com/users/dbuddha/projects/1) for priority and state;
- [issues](https://github.com/dbuddha/alpine-gpui/issues) for capabilities,
  requirements, tasks, decisions, defects, and research;
- [Actions](https://github.com/dbuddha/alpine-gpui/actions/workflows/ci.yml) for
  CI evidence and downloadable rustdoc artifacts;
- [releases](https://github.com/dbuddha/alpine-gpui/releases) for shipped
  history;
- [agent and contributor policy](AGENTS.md) for how changes are made;
- [engineering guide](docs/SUMMARY.md) for durable requirements and assurance
  concepts;
- [evidence registry](assurance/evidence.toml) for machine-checked claim and
  evidence traceability.

## Ownership and license

Public visibility does not make Alpine open source. At the reviewed commit,
Zed's `gpui` crate declares Apache-2.0, and that license governs Zed source.
Alpine's independently written source remains proprietary under
[LICENSE.md](LICENSE.md), which grants no permission beyond viewing the public
repository and using GitHub's permitted repository features.
