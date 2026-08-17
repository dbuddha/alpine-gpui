# AEP 0165: Bounded in-file find and replace

- Status: accepted 2026-08-16
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Requirement: [#32](https://github.com/dbuddha/alpine-gpui/issues/32)
- Parent task: [#127](https://github.com/dbuddha/alpine-gpui/issues/127)
- Implementation task: [#165](https://github.com/dbuddha/alpine-gpui/issues/165)
- Research: [#117](https://github.com/dbuddha/alpine-gpui/issues/117)

## Motivation and selected journey

The local workspace can open files and retain bounded tabs, but navigation
within one document still requires manual scanning. The first search slice must
make daily editing useful without regex compilation, indexing, a component
framework, or unbounded result retention.

Command-F opens one application-owned find surface. Command-Option-F exposes
replacement. Native keyboard and IME events mutate only the focused bounded
field. Literal UTF-8 matching runs on an immutable buffer snapshot through the
existing bounded runtime worker path. Results publish only when document,
buffer, and query identities are still current.

## Atomic claims

- **AEP-0165-C01:** Query and replacement text retain at most 4 KiB each, one
  scan materializes at most 16 MiB of immutable snapshot text, and one result
  retains at most 16,384 non-overlapping matches or 256 KiB of exact range
  metadata with explicit truncation.
- **AEP-0165-C02:** Every request and completion carries document, buffer, and
  query-generation identity. Stale completion preserves current selection,
  admitted results, and document bytes.
- **AEP-0165-C03:** Forward and backward navigation wrap deterministically, and
  one frame projects highlights only for visible line ranges with a separate
  2,048-match frame ceiling.
- **AEP-0165-C04:** Replace-current and replace-all reuse one checked text
  transaction. Replace-all refuses truncated evidence and more than 16 MiB of
  changed transaction bytes, and one undo restores the complete prior state.

## Correctness, performance, and memory

[`FindAdmission.tla`](../../formal/tla/aep-0165/FindAdmission.tla) models
request capture, query and document invalidation, completion admission, bounded
matches, cancellation, and replacement. `AdmittedIsCurrent`,
`StaleCompletionNeverPublishes`, and `ReplacementRequiresCurrent` are checked
with deliberately faulty stale-admission and stale-replacement controls. The
model does not claim formal refinement to Rust, text matching, allocation, or
worker scheduling.

Literal matching uses Rust UTF-8 substring semantics and emits non-overlapping
byte ranges in ascending order. A capped snapshot prefix backs up to a valid
UTF-8 boundary before materialization. The immutable buffer snapshot remains
cheap to clone, and only the capped prefix is copied for background scanning.
The result owns boxed range metadata after exact reservation, so retained byte
accounting is independent of allocator spare capacity.

Find opens with no scan for an empty query, adds no timer or idle redraw, and
uses the existing two-thread bounded handoff. Scene construction intersects the
admitted result with each already-visible line. No full-document highlight
geometry is retained or built.

## Scope exclusions and reversal conditions

This AEP does not authorize regex, case folding, whole-word rules, project
search, replacement preview, search history, persistence, plugins, telemetry,
AI, native handles, a new dependency, or a new worker pool. Those remain
separate accepted slices under Requirement #32.

Revisit fixed ceilings only with representative repository evidence. Add regex
or richer matching only after a separate dependency, worst-case execution,
cancellation, and memory contract is accepted. Move scanning away from snapshot
prefix materialization only if profiling shows that bounded copying is material
to an accepted daily-driver workload.
