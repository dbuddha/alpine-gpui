# AEP 0180: Bounded streaming local project search

- Status: Accepted
- Task: [#180](https://github.com/dbuddha/alpine-gpui/issues/180)
- Decision: [#181](https://github.com/dbuddha/alpine-gpui/issues/181)
- Requirement: [#32](https://github.com/dbuddha/alpine-gpui/issues/32)
- Parent task: [#127](https://github.com/dbuddha/alpine-gpui/issues/127)

## Context

Alpine Studio needs local folder search that becomes useful before every file has
been scanned, without adding a persistent content index, watcher, regex engine,
unbounded producer channel, or startup work. Search must preserve exact UTF-8
byte identity so selecting a stale result cannot navigate to unrelated text.

## Decision

Studio owns a private project-search state machine. Explicit Command-Shift-F or
the static command palette opens it. A non-empty case-sensitive literal query
admits one serial ignore-aware inventory request, followed by bounded content
scan continuations on the existing runtime workers. Each continuation publishes
at most one fixed-size match batch and yields before later work is submitted.

Inventory, query, and request generations identify every output. Query changes,
close, workspace replacement, reordered completion, and cancelled work cannot
publish. An atomic generation observation lets active scan work stop at bounded
file seams without making the foreground state concurrent.

No public Alpine API, dependency, async runtime, persistent index, watcher,
plugin boundary, network path, telemetry path, or startup task is added.

## Locked limits

| Resource | Limit | Failure or pressure behavior |
| --- | ---: | --- |
| Query | 4 KiB | Reject atomically and preserve the prior query |
| Inspected entries | 250,000 | Stop inventory and expose truncation |
| Admitted regular files | 100,000 | Omit later files and expose truncation |
| One relative path | 4 KiB | Omit the path |
| Inventory path bytes | 16 MiB | Omit later paths and expose truncation |
| Path depth | 256 components | Omit deeper paths |
| One file read | 16 MiB | Skip as oversized |
| Total bytes read per query | 512 MiB | Stop search and expose truncation |
| Retained matches | 16,384 | Stop search and expose truncation |
| Result path, excerpt, and metadata bytes | 4 MiB | Stop before exceeding the budget |
| One publication batch | 256 matches and 256 KiB | Return a continuation before the next match |
| One display excerpt | 512 UTF-8 bytes | Project a character-boundary window around the match |
| Visible rows | 256 | Project only the requested window plus bounded overscan |
| Files per worker continuation | 64 | Yield to the bounded runtime |
| Read bytes per worker continuation | 16 MiB | Yield to the bounded runtime |
| Diagnostic display | 4 KiB | Truncate at a UTF-8 boundary |

One bounded file buffer may move between consecutive worker requests while that
file still has unpublished matches. It is neither a result nor corpus cache and
is released on completion, cancellation, stale rejection, submission failure,
or close. No result retains source file contents.

## Filesystem and matching semantics

Traversal uses the accepted project-local `ignore` policy: hidden paths remain
eligible, `.git` is omitted, project `.gitignore`, `.ignore`, and repository
exclude rules apply, global and parent state are disabled, links are not
followed, and traversal remains on one filesystem. Only regular files under the
canonical workspace root are admitted.

Each file is opened and read behind its per-file and remaining-query budgets.
Invalid UTF-8, embedded NUL, unreadable, oversized, non-regular, and replaced
files are skipped with separate bounded counters. Unix file identity includes
device, inode, size, modification, and change times. Other targets use size and
available modification time. A Boyer-Moore-Horspool byte search avoids query
normalization and quadratic prefix allocation. Matches are non-overlapping and
carry exact byte range, one-based line and byte column, bounded excerpt, and
normal root-relative path.

## Selection and rendering

Selection first revalidates every path component through the existing workspace
boundary. If the file already has a tab, its current in-memory snapshot must
still contain the exact query at the recorded range before tab activation. A new
file is opened into temporary local ownership and verified before any tab or
document mutation. Failure preserves active document, tab identity, selection,
and search state. Success uses the existing tab path, selects the exact range,
scrolls to its line, and releases search-owned foreground allocations.

The overlay shapes only its diagnostic line and the visible result window plus
three rows of overscan. Keyboard and IME focus are exclusive. Clipboard and
pointer editor actions are suppressed while search owns focus. No accepted
input or worker publication means no invalidation or frame submission.

## Evidence contract

Claims `AEP-0180-C01`, `AEP-0180-C02`, `AEP-0180-C03`, and
`AEP-0180-C04` require exact cap tests, ignore and malformed-file fixtures,
streaming continuation controls, cancellation and stale-publication sequences,
pre-mutation selection checks, Studio routing and scene semantics, native
AppKit keyboard delivery, stage-separated diagnostics, TLA+ current-publication
and bounded-retention invariants with faulty controls, changed-line coverage,
viable mutation rejection, and hosted `ci-pass`.

Unit and hosted elapsed times are diagnostic only. Comparative speed or memory
claims require the separate fixed-hardware qualification protocol.

## Explicit exclusions

Regex, project replacement, persistent content indexes, watchers, syntax-aware
search, binary search, lossy decoding, remote workspaces, plugins, terminal,
Git UI, AI, collaboration, cloud, telemetry, multi-window support, public
framework search APIs, and cross-product performance claims are excluded.
