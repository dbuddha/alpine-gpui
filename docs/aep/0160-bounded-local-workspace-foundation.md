# AEP 0160: Bounded local workspace foundation

- Status: accepted 2026-08-16
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Requirement: [#32](https://github.com/dbuddha/alpine-gpui/issues/32)
- Parent task: [#127](https://github.com/dbuddha/alpine-gpui/issues/127)
- Implementation task: [#160](https://github.com/dbuddha/alpine-gpui/issues/160)
- Launch decision: [#161](https://github.com/dbuddha/alpine-gpui/issues/161)
- Research: [#117](https://github.com/dbuddha/alpine-gpui/issues/117)

## Motivation and selected journey

The qualified one-file editor has no folder ownership or navigation surface.
The first workspace slice must become usable without introducing recursive
indexing, a retained component framework, or a second editor state graph.

One process path may now identify a regular file or directory. A directory is
canonicalized and enumerated before native construction. Studio paints a
top-level tree beside the existing editor, scrolls it independently when the
pointer is over the sidebar, and opens a clicked regular UTF-8 file through the
existing conflict-aware `Editor`.

## Atomic claims

- **AEP-0160-C01:** Enumeration inspects no more than 4,096 top-level entries,
  retains no more than 1,024 UTF-8 names or 256 KiB of name bytes, records every
  omission, and fails structurally rather than returning a nondeterministic
  partial snapshot when the scan ceiling is exceeded.
- **AEP-0160-C02:** Retained directory and file names are sorted
  deterministically, and one scene shapes only the visible tree rows plus at
  most three rows of overscan.
- **AEP-0160-C03:** A clicked target is revalidated as a non-symlink regular
  direct child of the canonical root. Directory, invalid UTF-8, replacement,
  escape, missing-entry, and dirty-document failures preserve current bytes and
  document identity.
- **AEP-0160-C04:** A successful file switch advances one private monotonic
  Studio document identity before runtime publication and reuses the existing
  editor, text layout, glyph atlas, scene, and native presentation path.

## Correctness, performance, and memory

[`WorkspaceSelection.tla`](../../formal/tla/aep-0160/WorkspaceSelection.tla)
models edit, save, valid replacement, and rejected selection over finite
document bytes and identities. `FailedSelectionPreservesDocument`,
`SuccessfulReplacementAdvancesIdentity`, and
`DocumentIdentityNeverDecreases` are independently checked with two faulty
controls. The model assumes typed admission outcomes and does not claim formal
refinement to Rust, path canonicalization, UTF-8 decoding, or filesystem I/O.

The root and selected file are canonicalized separately to close the gap
between enumeration and click. `symlink_metadata` rejects a selected symlink
before canonical target admission. Only one-component retained names can form a
candidate path, and the canonical parent must equal the canonical root.

The scan ceiling bounds transient candidate count. Per-name and aggregate
retention ceilings bound persistent tree memory, while explicit omission counts
make degraded behavior observable. Sorting happens once before the AppKit loop.
Frame work is proportional to visible rows plus fixed overscan, and no worker,
timer, polling loop, idle redraw, dependency, or native handle is added.

## Scope exclusions and reversal conditions

This AEP does not authorize recursion, `.gitignore` indexing, search, tabs,
splits, history, quick open, restoration, filesystem watching, remote paths,
plugins, telemetry, AI, Git, terminal, or multi-window behavior. Those remain
separate slices under Task #127.

Revisit the fixed ceilings only with representative repository evidence and an
explicit degraded-mode contract. Replace top-level synchronous enumeration with
bounded background work before recursion, indexing, or measured launch delay
can enter the daily-driver path.
