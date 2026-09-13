# Architecture

Alpine GPUI is the framework: safe Rust contracts, a bounded runtime, an
immutable scene protocol and a Direct Metal backend for Apple Silicon macOS.
Alpine Editor is the application built on it. Read only the topic relevant to
the change; source and tests establish behavior.

| Owner | Responsibility |
| --- | --- |
| `crates/alpine-core` | Validated geometry and value contracts |
| `crates/alpine-scene`, `alpine-renderer` | Immutable scenes, painter ordering and renderer contracts |
| `crates/alpine-text`, `alpine-text-layout` | Text revisions, Unicode coordinates, layout and glyph caches |
| `crates/alpine-runtime` | Application events, bounded workers and stale-result admission |
| `crates/alpine-platform` | Portable presentation state machine and frame slots |
| `crates/alpine-metal`, `alpine-platform-macos` | GPU ownership, AppKit callbacks, presentation and teardown |
| `apps/alpine-editor` | Documents, panes, commands, local search, settings and LSP |
| `tools/alpine-assurance`, `alpine-trace` | Non-shipping validation and measurement tools |

Preserve document revision ownership, checked coordinate conversions, durable
save behavior, bounded queues and caches, and callback generation checks. Keep
blended painter order, asynchronous GPU completion and in-flight resource
lifetimes. GPU completion is not physical presentation, and hosted validation is
not physical accessibility, energy or daily-driver acceptance.

## Topic references

- [Implemented system](implemented-system.md)
- [Ownership from state to submission](ownership-from-state-to-submission.md)
- [Invalidation to present contract](invalidation-to-present-contract.md)
- [Resource lifetime contract](resource-lifetime-contract.md)
- [Portable contracts and native specialization](portable-contracts-and-native-specialization.md)
- [Error and device-loss propagation](error-and-device-loss-propagation.md)
- [Testing and evidence](testing-and-evidence.md)
- [Binding invariants](binding-invariants.md)

Use [testing guidance](../testing.md) for acceptance and
[current limitations](../reference/limitations.md) for product scope.
