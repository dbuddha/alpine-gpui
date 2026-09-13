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

The product target is Apple Silicon running macOS 15 or newer, using Direct
Metal. Linux and Windows currently test portable contracts; native backends for
those systems are outside the active plan.

The ambition is one keyboard-first, accessible workspace containing a terminal,
Alpine Editor, database views and an agent dock. Only the editor prototype exists
today, with the executable and source paths still named `alpine-studio`. The
[capability probe](docs/alpine-capability-probe.md) first establishes native
readiness, then tests a memory advantage with responsiveness floors against
matched implementations. Superiority is an unproven hypothesis.

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
  restoration, compiled syntax, bounded local settings reload and migration,
  typed themes and keymaps, keyboard, pointer, clipboard,
  IME, and revisioned accessibility semantics;
- a bounded local process, JSON-RPC, and LSP path qualified with a pinned
  `rust-analyzer`, including revision-safe visible Rust diagnostics and no
  network, extension, AI, collaboration, or telemetry subsystem;
- fail-closed qualification tooling plus policy, formatting, lint, tests,
  rustdoc, coverage, changed-code mutation, selected models and proofs,
  three-platform CI and native Metal validation. Specialized assurance is
  available through explicit manual runs.

Physical typing latency, VoiceOver, sustained dogfood and residency still need
qualification on the target Mac. Fixed-hardware comparator evidence, API
stabilization, signing, notarization and release support remain later work.
[Architecture](ARCHITECTURE.md) describes implemented boundaries and invariants;
[documentation](docs/README.md) separates current guidance from historical plans.

## Development and project state

Run the full deterministic and tooling gate:

```sh
scripts/check.sh
```

Native execution is separate: `scripts/check-native.sh physical shipping` runs
the shipping smoke; `scripts/check-native.sh physical all` runs the broader
native suite. Neither ordinary workspace tests nor hosted CI prove physical
presentation or daily-driver readiness.

Build the canonical unsigned private-dogfood application from a clean revision:

```sh
scripts/build-alpine-studio-app.sh
```

The command creates the release bundle at
`target/release/Alpine Studio.app`. Launch a scratch editor, file, or folder
through its stable LaunchServices identity with:

```sh
scripts/launch-alpine-studio-app.sh
scripts/launch-alpine-studio-app.sh path/to/file-or-folder
```

The bundle is local dogfood infrastructure, not a public release artifact.
Signing, notarization, distribution, and updates remain later release gates.

Development is PR-first: explain the problem and outcome, change, and verification
with remaining risks. A user request is sufficient scope for a focused fix. Use
[issues](https://github.com/dbuddha/alpine-gpui/issues) for deferred defects,
blockers or multi-PR work; labels, hierarchy and Projects are optional.
See [CONTRIBUTING.md](CONTRIBUTING.md) for development and acceptance and
[Actions](https://github.com/dbuddha/alpine-gpui/actions/workflows/ci.yml) for CI.
Documentation is plain Markdown plus Rust API docs and doctests. The Wiki is retired; repository Markdown is canonical. Update existing guidance when behavior changes it.

## Ownership and license

Public visibility does not make Alpine open source. At the reviewed commit,
Zed's `gpui` crate declares Apache-2.0, and that license governs Zed source.
Alpine's independently written source remains proprietary under
[LICENSE.md](LICENSE.md), which grants no permission beyond viewing the public
repository and using GitHub's permitted repository features.
