# AEP 0222: Bounded settings reload and migration

- Status: Accepted
- Decision: [#354](https://github.com/dbuddha/alpine-gpui/issues/354)
- Task: [#222](https://github.com/dbuddha/alpine-gpui/issues/222)
- Requirement: [#36](https://github.com/dbuddha/alpine-gpui/issues/36)
- Parent task: [#129](https://github.com/dbuddha/alpine-gpui/issues/129)
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)

## Context

Studio already owns compiled typed settings, themes, keymaps, static commands,
and atomic global-then-project admission. The missing daily-driver boundary is
loading local files without moving filesystem or JSON work onto the input and
render path, publishing stale results, retaining unbounded configuration, or
creating a plugin-style runtime registry.

## Decision

Studio reads one optional global JSON file and one optional project JSON file
through the existing bounded worker pool. A coalescing owner permits at most one
in-flight load and tags every request and completion with a monotonic
generation. A completion can publish only when it equals both the submitted
and latest requested generation.

The worker reads regular non-symlink files with before/open/after identity
checks, parses a closed versioned schema, migrates the accepted v0 editor
fields in memory, and returns one complete `SettingsUpdate`. The main-thread
owner resolves compiled, global, then project settings into a candidate and
replaces the active snapshot only after complete validation. Every failure
preserves the prior settings, document, workspace, selection, tabs, and frame
state.

Startup records one pending load but does no settings filesystem I/O until the
first production event submits background work. The command palette exposes
one static reload command. No watcher, timer, polling loop, public API,
dependency, extension host, executable configuration, network, plugin, AI,
cloud, account, or telemetry boundary is added.

## Locked limits

| Resource | Limit | Failure behavior |
| --- | ---: | --- |
| One settings file | 64 KiB | Reject before JSON admission |
| One settings path | 4 KiB | Reject before filesystem access |
| JSON depth | 8 | Reject the complete reload |
| Parsed JSON values | 512 | Reject the complete reload |
| JSON key and string bytes | 32 KiB | Reject the complete reload |
| Key bindings | 64 | Reject before active-state mutation |
| Font name | 256 bytes | Reject before active-state mutation |
| Active retained settings | 64 KiB | Reject before publication |
| In-flight settings loads | 1 | Coalesce the latest requested generation |

## Atomic claims and evidence contract

- **AEP-0222-C01:** Only the exact latest submitted generation can atomically
  replace active settings; stale, failed, invalid, and concurrently changed
  inputs preserve the complete prior snapshot.
- **AEP-0222-C02:** File, path, JSON, migration, keymap, theme, diagnostic, and
  retained-state ownership remains inside explicit byte and count ceilings.
- **AEP-0222-C03:** Settings work performs no filesystem or JSON work on the
  input or render path and introduces no dynamic registration or networked
  product subsystem.

The claims require exact boundary fixtures, precedence and rollback tests,
concurrent-edit injection, stale completion controls, document/workspace state
preservation, command discovery, Kani generation refinement, TLA+ positive and
faulty models, changed-line coverage, viable mutation rejection, and hosted
`ci-pass`.

## Reversal conditions

Reopen this decision if dogfood proves the fixed limits reject normal Alpine
workspaces, a filesystem watcher is required to avoid material workflow cost,
or measured reload work requires a dedicated queue. Any widening requires a
new accepted requirement with identity, resource, startup, and failure
evidence.
