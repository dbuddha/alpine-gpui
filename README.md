# Alpine GPUI

Alpine GPUI is a private, proprietary, high-performance desktop application
framework written in Rust. Its programming model is strongly inspired by
[Zed's GPUI](https://github.com/zed-industries/zed/tree/1271f8b0e8f3278eed5dd3fc12ad4bd30dce2c5d/crates/gpui),
whose production architecture demonstrates entity-owned state, hybrid
immediate and retained UI construction, element phases, GPU rendering, and
headless testing at application scale.

Alpine is an independent implementation, not a fork or source-compatible
distribution of Zed GPUI. It owns its runtime, scheduling, scene protocol,
renderer policy, resource lifetimes, and native platform integrations. The
project is not affiliated with or endorsed by Zed Industries.

The flagship implementation targets Apple Silicon Macs running macOS 15 or
newer through a direct Metal backend. Direct Vulkan on Linux and direct D3D12 on
Windows follow after the Metal and shared semantic contracts are proven. Intel
Macs, web, and mobile are outside the version 1 scope.

## Product direction

Alpine is optimized first for editors, terminals, database tools, and
data-heavy productivity applications. Its public programming model may feel
familiar to GPUI users, but source compatibility is not a goal.

The framework will provide:

- transactional application state and scoped invalidation;
- hybrid immediate and retained view construction;
- layout, text, input, accessibility, animation, and virtualization;
- direct native windowing and rendering backends;
- headless behavior and renderer conformance harnesses;
- typed Rust styling and theme tokens;
- headless UI primitives plus an application-ready component library;
- embedded native GPU surfaces and custom material support;
- first-party diagnostics through Alpine Lab and Alpine Inspector.

## Current status

The project is in its foundation milestone. The workspace currently contains:

- `alpine-core`: backend-neutral geometry and color types;
- `alpine-scene`: immutable renderer input and painter ordering;
- `alpine-renderer`: backend contracts and observable frame reports;
- pinned Rust 1.97.1;
- strict lint, test, documentation, and three-platform CI gates;
- branch protection and immutable GitHub Actions policy;
- architecture, provenance, dependency, hardware, and agentic workflow policy.

No upstream GPUI implementation is vendored or linked into the product.

## Documentation map

Start with the [internal engineering guide](docs/README.md). It contains the
architecture, source-influence, frame-lifecycle, and agentic change-flow
diagrams.

- [Architecture](ARCHITECTURE.md)
- [Crate architecture](crates/README.md)
- [Master plan](docs/MASTER_PLAN.md)
- [Milestone roadmap](docs/ROADMAP.md)
- [Source influence map](docs/research/source-map.md)
- [Upstream analysis](docs/research/upstream-analysis.md)
- [Provenance ledger](docs/research/provenance-ledger.md)
- [Dependency policy](docs/DEPENDENCIES.md)
- [CI and hardware strategy](docs/ci/strategy.md)
- [Agentic engineering workflow](docs/engineering/agentic-workflow.md)
- [Changelog policy](docs/engineering/changelog.md)
- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md)

## Development

Run the complete local acceptance gate:

```sh
scripts/check.sh
```

The same policy, lint, test, and documentation commands run in CI. Native tests
also run on Linux, Apple Silicon macOS, and Windows.

## Lineage, ownership, and license

Proprietary describes how Alpine GPUI is distributed, not an absence of open
source influence. Zed GPUI is the primary conceptual influence, and GPUI-CE,
`gpui-component`, WGPUI, `gpui-wgpu` descendants, and Kael provide additional
architecture, behavior, workload, and failure-mode evidence. The exact reviewed
commits and permitted influence paths are recorded in the
[source influence map](docs/research/source-map.md).

At the reviewed commit, Zed's `gpui` crate declares the
[Apache-2.0 license](https://github.com/zed-industries/zed/blob/1271f8b0e8f3278eed5dd3fc12ad4bd30dce2c5d/crates/gpui/Cargo.toml).
That license governs Zed GPUI source. Alpine's independently written source
remains under Alpine's own private distribution policy.

No upstream GPUI implementation source is currently copied, vendored, or linked
into Alpine. Any future adaptation of upstream source requires explicit
approval, license review, and symbol-level recording in the
[provenance ledger](docs/research/provenance-ledger.md). The repository remains
private and proprietary, and no license to use, copy, modify, or distribute its
source is granted.
