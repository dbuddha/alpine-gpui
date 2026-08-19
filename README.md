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

Alpine is pre-release and its public framework contracts are not version 1
stable. Alpine Studio is a working local editor prototype on the path to the
selected Apple Silicon macOS daily-driver profile, but it is not yet qualified
or distributed as a daily driver.

The workspace currently provides:

- an immutable scene protocol, deterministic Direct Metal renderer, CPU oracle,
  native AppKit window, demand-driven display-link presentation, bounded
  asynchronous frame slots, and explicit resource accounting;
- a safe application runtime with synchronous native events, dirty-only scene
  construction, bounded workers, external-source wake admission, and no general
  async executor or reactive graph;
- local copy-on-write text, Unicode and UTF-16 mappings, transactions, bounded
  undo and redo, atomic save, CoreText shaping, visible-range layout, and
  hard-budgeted glyph and line caches;
- Alpine Studio file and folder launch, virtualized file tree, tabs, bounded
  splits, find and replace, quick open, command discovery, project search,
  restoration, compiled syntax, typed settings, keyboard, pointer, clipboard,
  IME, and revisioned accessibility semantics;
- a bounded local process, JSON-RPC, and LSP path qualified with a pinned
  `rust-analyzer`, including revision-safe visible Rust diagnostics and no
  network, extension, AI, collaboration, or telemetry subsystem;
- fail-closed qualification tooling plus policy, formatting, lint, tests,
  rustdoc, coverage, changed-code mutation, selected models and proofs,
  three-platform CI, native Metal validation, and scheduled assurance.

Remaining daily-driver work includes richer Rust intelligence, configuration
reload and migration, native VoiceOver and lifecycle qualification, sustained
repository dogfood, and defect closure. Fixed-hardware comparator evidence,
API stabilization, signing, notarization, packaging, and release support remain
later gates. The exact implemented boundaries and invariants are documented in
[Architecture](ARCHITECTURE.md); GitHub issues own live delivery state.

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

GitHub Project views are a planning projection, not a fallback source of truth.
If the active token lacks `read:project`, operators use the issue hierarchy and
must not infer Project fields, status, blockers, or charts.

## Ownership and license

Public visibility does not make Alpine open source. At the reviewed commit,
Zed's `gpui` crate declares Apache-2.0, and that license governs Zed source.
Alpine's independently written source remains proprietary under
[LICENSE.md](LICENSE.md), which grants no permission beyond viewing the public
repository and using GitHub's permitted repository features.
