# Zed Editor stable application

- Reviewed: 2026-08-15
- Research anchor: [#27](https://github.com/dbuddha/alpine-gpui/issues/27)
- Comparative research: [#113](https://github.com/dbuddha/alpine-gpui/issues/113)
- Release: `v1.15.0`
- Revision: [`e17dc4f9d50db73a458b64dcce50ecd4878b98a3`](https://github.com/zed-industries/zed/tree/e17dc4f9d50db73a458b64dcce50ecd4878b98a3)
- License boundary: the Zed application declares GPL-3.0-or-later
- Influence: behavioral, workload-based, validation-oriented, and differential
- Evidence strength: pinned primary source plus official engineering articles

## Research question

Which parts of Zed make a modern local editor correct and responsive, which
parts exist for a broader networked product, and what is the smallest coherent
subset Alpine Studio needs to become a solo developer's daily driver?

## Scope and method

The review follows Zed from application ownership through workspace, editor,
buffer, viewport, text layout, scene construction, Metal submission, language
services, input, settings, accessibility, and shutdown. A source observation is
not an Alpine requirement by itself. Each included behavior must have an
approved Alpine Requirement and independent tests.

The pinned application is a comparator and workload specimen, not a source
base for Alpine. GPL application code, assets, patches, and build products stay
inside `alpine-zed-lab`. GPUI concepts may influence an independently designed
Alpine boundary, but source adaptation requires separate provenance approval.

## Product decomposition

### Keep for the first daily driver

- One local workspace with folder open, file navigation, tabs, splits, history,
  quick open, command discovery, project search, and crash-safe restoration.
- Correct local editing with Unicode, multi-selection, undo and redo, IME,
  clipboard, external-change detection, and atomic save.
- Viewport-bounded line construction, shaping, glyph upload, syntax work, and
  search result retention.
- Built-in Rust, plain-text, Markdown, TOML, and JSON behavior, with one local
  `rust-analyzer` transport.
- Typed settings, themes, keymaps, focus, accessibility, and deterministic
  lifecycle recovery.
- Demand-driven frames and explicit evidence for latency, retained memory,
  cache pressure, queue depth, and post-close drain.

### Exclude from the product architecture

- Collaboration, shared editing, replica synchronization, channels, calls, and
  presence.
- Hosted AI, model providers, agent workflows, cloud accounts, remote
  development, telemetry, and business analytics.
- Plugins, extension host, package marketplace, runtime grammar installation,
  and third-party executable UI code.
- Debugger, integrated terminal, task runner, and Git UI before daily-driver
  qualification. External tools are the first-release workflow.
- Multi-window qualification in v1. One owned window is the first lifecycle
  contract.

## Pinned source findings

### ZED-APP-001: explicit application-owned state is useful, GPUI compatibility is not

Zed's official ownership description says the application owns entity state,
while typed handles can access or mutate that state only through an application
context. It also documents temporary leasing to satisfy Rust's aliasing rules
and notes that reentrant updates must be avoided
([Zed, 2024](https://zed.dev/blog/gpui-ownership)).

Alpine consequence: keep explicit `StudioApp -> Workspace -> Editor -> Buffer`
ownership and main-thread mutation, but do not reproduce GPUI entities,
subscriptions, a reactive graph, or temporary entity leasing before a Studio
slice proves they are needed. Revision-tagged worker results are a smaller and
more auditable solution for a single local product.

### ZED-APP-002: Zed's text state contains collaboration costs Alpine should not inherit

The pinned `Buffer` stores deferred operations, replica sets, a Lamport clock,
and remote-version waiters
([text.rs lines 59-68](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/text/src/text.rs#L59-L68)).
Its snapshot retains visible and deleted ropes, fragment and insertion trees,
an undo map, global version, remote buffer identity, and replica identity
([text.rs lines 113-124](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/text/src/text.rs#L113-L124)).
History also retains timestamped operations in addition to undo and redo stacks
([text.rs lines 153-160](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/text/src/text.rs#L153-L160)).

Alpine consequence: use a local copy-on-write rope, monotonic local revision,
compact transactions, deterministic undo and redo, and immutable snapshots.
Do not add replicas, operation broadcast, deleted-text CRDT retention, remote
IDs, or global clocks.

### ZED-APP-003: visible work is a correctness and residency boundary

GPUI's uniform list computes a visible item range and invokes its renderer only
for that range
([uniform_list.rs lines 474-500](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/elements/uniform_list.rs#L474-L500)).
This is direct evidence for list virtualization, not evidence that Zed's editor
has a particular undocumented memory bound.

Alpine consequence: the file tree, quick open, search results, and code viewport
must expose their visible range and overscan. Tests must prove that offscreen
item count does not linearly increase per-frame layout, shaping, primitives, or
retained result objects.

### ZED-APP-004: settings should be centralized before product scale arrives

Zed reports that distributed definitions and runtime registration left it
without a unified strongly typed settings model and made changes difficult to
trace ([Zed, 2025](https://zed.dev/blog/settings-ui)).

Alpine consequence: settings, commands, themes, and keymaps are compiled into a
central typed schema with deterministic global and project precedence. There is
no runtime plugin registration layer.

### ZED-APP-005: daily-driver evidence must include behavior Zed's renderer benchmark omits

A practical editor journey includes startup to accepted input, edit and undo,
IME, save failure, external changes, search streaming, stale language-server
responses, accessibility, idle residency, hide and restore, and close during
work. Renderer submission alone cannot validate these outcomes.

Alpine consequence: renderer-only claims and product claims remain separate.
No editor comparison is valid until file bytes, selections, visible output,
accessibility, lifecycle, and omission logs are equivalent.

## Adversarial verdict

| Decision | Correctness | Performance | Efficiency | Delivery speed |
| --- | --- | --- | --- | --- |
| Keep explicit ownership and revision-tagged work | Rejects stale mutation and ambiguous lifetime | Avoids redundant publication | Bounds queued results | Small model before framework |
| Keep visible-range construction | Preserves deterministic mapping | Avoids offscreen layout and paint | Bounds memory and CPU work | Reusable in editor, tree, and search |
| Replace collaborative buffer state | Simplifies undo and local revisions | Removes remote-operation maintenance | Avoids replica and tombstone residency | Smaller test surface |
| Centralize settings and commands | Deterministic precedence | Keeps startup registration out of critical path | No dynamic registries | One schema and one migration path |
| Exclude product services | Prevents hidden network and lifecycle states | Removes background wakeups | Removes resident services and caches | Focuses all work on editing |

## Comparator implications

- Normalized Zed runs disable accounts, AI, collaboration, telemetry,
  extensions, and remote features where configurable, and disclose anything
  that cannot be disabled.
- Stock Zed footprint is reported separately and never used to explain a
  normalized result.
- Zed source instrumentation and patches remain in `alpine-zed-lab`.
- The exact release, revision, patch hash, build profile, settings, grammar,
  language server, font set, repository fixture, and workload hash are required
  for every result.
- Passing a narrower behavior is an invalid run, not a speedup.

## Sources

- [Pinned Zed revision](https://github.com/zed-industries/zed/tree/e17dc4f9d50db73a458b64dcce50ecd4878b98a3): immutable source identity.
- [Ownership and data flow in GPUI](https://zed.dev/blog/gpui-ownership): application and entity ownership model.
- [How We Rebuilt Settings in Zed](https://zed.dev/blog/settings-ui): distributed settings and runtime-registration costs.
- [Pinned text buffer](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/text/src/text.rs): local and collaborative text state.
- [Pinned uniform list](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/elements/uniform_list.rs): visible-range construction.
