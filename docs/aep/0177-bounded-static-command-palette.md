# AEP 0177: Bounded static command palette

- Status: Accepted
- Task: [#177](https://github.com/dbuddha/alpine-gpui/issues/177)
- Decision: [#178](https://github.com/dbuddha/alpine-gpui/issues/178)
- Requirement: [#32](https://github.com/dbuddha/alpine-gpui/issues/32)
- Parent task: [#127](https://github.com/dbuddha/alpine-gpui/issues/127)

## Context

Alpine Studio needs one discoverable route to its growing local command set.
Runtime registration, closure-backed actions, and background fuzzy matching
would create plugin architecture, lifecycle ambiguity, and unnecessary memory
for a product whose commands are known at compile time.

## Decision

The Studio owns a private closed `StudioCommand` enum, a static registry, and a
bounded foreground palette state machine. Direct shortcuts and palette actions
call the same existing Studio transitions. Availability is recomputed before
execution. Keyboard and IME focus are exclusive while the palette is open.

No public Alpine API, runtime registry, dynamic dispatch graph, worker, channel,
timer, polling loop, plugin boundary, or executable configuration is added.

## Locked limits

| Resource | Limit | Failure or pressure behavior |
| --- | ---: | --- |
| Static commands | 32 | Shipping source change and review required |
| Query | 256 bytes | Reject atomically and preserve the prior query |
| Composition plus query | 256 bytes | Reject invalid or oversized composition |
| Retained matches | 32 | Registry bound makes truncation impossible |
| Visible rows | 12 | Project one bounded window |
| Visible overscan | 3 rows per side | Never exceed 18 projected rows |
| Diagnostic display | 512 bytes | Return a structured render error |

Closing or executing releases query, composition, and match allocations to zero
current retained bytes. Peak bytes, current bytes, match count, visible rows,
executions, cancellations, and truncations remain observable through a
handle-free report.

## Matching and execution

An empty query preserves registry order. Non-empty matching ranks whole-string
prefixes, token prefixes, then ordered subsequences with gap count and registry
identity as deterministic tie breakers. ASCII command metadata is compared
case-insensitively without allocating normalized copies. Unicode query bytes
remain valid UTF-8 and deletion removes one scalar value.

Execution admits only the currently selected match when its command remains
available in a freshly derived Studio context. Stale or unavailable selection
fails closed and refreshes the visible command set. The palette closes before
the existing command transition runs, so focus ownership is unambiguous.

## Correctness and lifecycle

- Query and composition mutations are atomic on allocation or limit failure.
- IME input changes only palette state while the palette owns focus.
- Pointer and clipboard editor behavior is suppressed while the palette is open.
- Escape cancels and releases state; Enter executes at most one current command.
- Save, close, history, find, quick-open, and file-tree actions reuse existing
  state transitions and failure reporting.
- Scene work is bounded by the query plus at most 18 command rows.
- No accepted input means no invalidation and no frame submission.

## Evidence contract

Claims `AEP-0177-C01`, `AEP-0177-C02`, `AEP-0177-C03`, and
`AEP-0177-C04` require exact limit tests,
ranking controls, randomized state sequences, Studio routing, native AppKit
input, diagnostic stage separation, TLA+ current-execution and closed-release
invariants with faulty controls, changed-line coverage, viable mutation
rejection, and hosted `ci-pass`.

Timing gathered by unit or hosted tests is diagnostic only. No comparative
performance claim is authorized by this AEP.

## Explicit exclusions

Runtime command registration, plugins, extension APIs, editable keymaps,
scripts, macros, project search implementation, panes, terminal, Git UI, AI,
collaboration, cloud, remote development, telemetry, multi-window support, and
public framework generalization are excluded.
