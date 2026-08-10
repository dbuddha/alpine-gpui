# Alpine GPUI

Alpine GPUI is a proprietary, high-performance desktop UI framework written in
Rust. It owns its runtime, scene protocol, renderer boundaries, and native
platform integrations.

The flagship implementation targets Apple Silicon Macs with a direct Metal
backend. Linux and Windows remain first-class architectural targets, with
direct Vulkan and D3D12 backends planned after the macOS foundation is proven.

## Status

The project is in foundation phase. The current workspace establishes:

- backend-neutral geometry and scene crates;
- a zero-cost renderer contract based on associated types;
- an explicit architecture and upstream provenance policy;
- repository-owned, pinned CI for Linux, macOS ARM64, and Windows;
- a dependency-free baseline from which every external dependency must be
  justified.

No upstream GPUI implementation is vendored or linked into the product.

## Start here

- [Architecture](ARCHITECTURE.md)
- [Roadmap](docs/ROADMAP.md)
- [Upstream analysis](docs/research/upstream-analysis.md)
- [CI strategy](docs/ci/strategy.md)
- [Dependency policy](docs/DEPENDENCIES.md)

## Development

```sh
scripts/check.sh
```

The repository is private and proprietary. No license to use, copy, modify, or
distribute the source is granted.
