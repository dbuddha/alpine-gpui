# Architecture

Alpine Studio is a local-only Apple Silicon macOS editor. Alpine GPUI supplies
its safe Rust contracts, bounded runtime and Direct Metal native backend.
Read only the topic relevant to the change; source and tests establish behavior.
The broader product ambition and current experiment are separated in the
[capability probe](docs/alpine-capability-probe.md); this map describes existing code.

| Owner | Responsibility |
| --- | --- |
| `apps/alpine-studio` | Documents, panes, commands, local search, settings and LSP |
| `crates/alpine-text`, `alpine-text-layout` | Text revisions, Unicode coordinates, layout and glyph caches |
| `crates/alpine-runtime` | Application events, bounded workers and stale-result admission |
| `crates/alpine-scene`, `alpine-renderer` | Immutable scenes, painter ordering and renderer contracts |
| `crates/alpine-metal`, `alpine-platform-macos` | GPU ownership, AppKit callbacks, presentation and teardown |
| `tools/alpine-assurance`, `alpine-trace` | Non-shipping validation and measurement tools |

Preserve document revision ownership, checked coordinate conversions, durable
save behavior, bounded queues/caches and callback generation checks. Keep blended
painter order, asynchronous GPU completion and in-flight resource lifetimes.
GPU completion is not physical presentation; hosted validation is not physical
VoiceOver, energy or daily-driver acceptance.

## Topic references

- [Implemented system](docs/architecture/implemented-system.md)
- [Ownership from state to submission](docs/architecture/ownership-from-state-to-submission.md)
- [Invalidation to present contract](docs/architecture/invalidation-to-present-contract.md)
- [Resource lifetime contract](docs/architecture/resource-lifetime-contract.md)
- [Portable contracts and native specialization](docs/architecture/portable-contracts-and-native-specialization.md)
- [Error and device-loss propagation](docs/architecture/error-and-device-loss-propagation.md)
- [Testing and evidence](docs/architecture/testing-and-evidence.md)
- [Binding invariants](docs/architecture/binding-invariants.md)

Use [testing guidance](docs/testing.md) for acceptance and
[current limitations](docs/reference/limitations.md) for product scope.
