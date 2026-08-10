# Alpine GPUI Operating Rules

Alpine GPUI is a proprietary, Rust-first desktop UI framework. macOS on Apple
Silicon is the flagship platform. Linux and Windows are architectural targets
from the beginning, but are implemented only after the macOS path is sound.

## Required context

Before non-trivial work, read:

1. `ARCHITECTURE.md`
2. `docs/ROADMAP.md`
3. The relevant ADR and research note

## Engineering contract

- Define the acceptance gate before implementation.
- Keep the scene protocol independent of windowing and graphics APIs.
- Keep platform policy out of renderer crates.
- Keep GPU resource ownership out of view and application state.
- Do not introduce a production dependency on GPUI, WGPU, winit, Blade, or a
  GPUI fork.
- Do not copy upstream code without recording its exact source, license, and
  modification history in `docs/research/provenance-ledger.md`.
- Prefer clean implementations informed by public behavior and architecture.
- Add no external dependency without an explicit dependency decision.
- Pin the Rust toolchain, CI actions, and dependency versions.
- Treat warnings, security findings, validation errors, and flaky tests as
  failures. Exceptions require a documented owner and expiry condition.
- Keep unsafe code out of safe crates. FFI crates must document every unsafe
  block with its safety invariant and provide a safe boundary.
- Do not accept performance claims without a reproducible benchmark, hardware
  manifest, raw results, and variance.

## Required gate

Run `scripts/check.sh` before a commit. Platform-specific changes also require
their platform workflow or a documented reason the remote gate has not run.

## Human-owned decisions

- External dependencies and licenses
- GitHub organization versus personal ownership
- Runner provider installation and billing
- Signing and notarization credentials
- Git push, pull request, release, and distribution approval
