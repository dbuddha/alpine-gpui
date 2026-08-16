# Alpine Studio daily-driver path

Alpine Studio is a local, editor-first application for one developer. It uses
Alpine GPUI to prove a correct, demand-driven, memory-bounded native UI while
delivering the focused product philosophy associated with Sublime Text and the
modern editing quality represented by Zed.

## Locked v1 product boundary

The first language cohort is Rust, plain text, Markdown, TOML, and JSON. The
first real language server is local `rust-analyzer`. Apple Silicon macOS 15 or
newer is the first shipping platform. One window is qualified first.

The daily-driver gate includes file open, correct text editing, Unicode, IME,
selection, clipboard, undo and redo, atomic save, external changes, folder
workspace, virtualized tree, tabs, splits, history, quick open, command palette,
find and replace, project search, syntax, Rust intelligence, settings, themes,
keymaps, restoration, focus, accessibility, and lifecycle recovery.

It excludes plugins, extension host, marketplace, collaboration, live shared
editing, AI, cloud accounts, remote development, telemetry, debugger,
integrated terminal, task runner, Git UI, and multi-window qualification.
External terminal and Git tools are the first-release workflow.

## Research evidence matrix

| Evidence | Supports | Does not prove |
| --- | --- | --- |
| [Research #27](https://github.com/dbuddha/alpine-gpui/issues/27) | Immutable Zed baseline and license boundary | Current performance superiority |
| [Zed application case study](../case-studies/zed-editor.md) | Daily-driver behavior and exclusions | Alpine implementation |
| [Zed GPUI case study](../case-studies/zed-gpui.md) | Invalidation, scene, cache, Metal, and benchmark patterns | Universal framework dominance |
| [Sublime case study](../case-studies/sublime-editor.md) | Public local-speed product principles | Proprietary internal architecture |
| [Research #113](https://github.com/dbuddha/alpine-gpui/issues/113) | Comparator baseline questions | Accepted benchmark evidence |
| [Research #114](https://github.com/dbuddha/alpine-gpui/issues/114) | Sublime evidence questions | Private implementation facts |
| [Research #115](https://github.com/dbuddha/alpine-gpui/issues/115) | Adaptation separation | Renderer qualification |
| [Research #116](https://github.com/dbuddha/alpine-gpui/issues/116) | Fixed-hardware questions | Completed hardware windows |
| [Comparator protocol v1](../quality/comparator-protocol.md) | Measurement and claim rules | Any result before raw evidence exists |

## Acceptance-gated execution

### Gate 0: authoritative execution line

- Preserve stale and dirty worktrees unchanged.
- Finish Task #107 through its current PR rather than porting a competing app
  runtime.
- Amend Requirements #32 through #37 to the product boundary below and obtain
  `owner:approved` before implementation.
- Keep issue hierarchy authoritative until GitHub Project scope is available.

### Gate 1: first production Studio window

- `NativeSurface::run` enters AppKit only on the main thread and returns after
  owned-window close, callback failure, or structured unexpected exit.
- App code uses public Alpine values and owns no native handle.
- Production-path native E2E opens, submits, closes through `windowWillClose`,
  stops AppKit, and exits under a watchdog.
- One frame is requested, idle callbacks and submissions remain stable, and
  final teardown drains retained resources.
- Synthetic `cfg(test)` launch bypasses are forbidden.

### Gate 2: non-blocking bounded presentation

- Replace callback `waitUntilCompleted` with a three-slot frame-resource ring
  approved by a presentation AEP.
- Commit and directly present inside the display-link callback, then return.
- Hold each slot until terminal completion; stale callbacks release ownership
  but cannot publish success.
- Coalesce latest work, cap in-flight slots and drawables at three, and prohibit
  unbounded command-buffer queues.
- Reuse geometrically grown upload buffers under a hard cap and release
  oversized capacity after pressure or sustained disuse.
- Expose current and peak in-flight frames, upload capacity, allocated and
  retained bytes, and terminal completion status.

Apple recommends a three-buffer dynamic-data pattern to keep CPU and GPU work
asynchronous while avoiding stalls
([Apple Metal Best Practices](https://developer.apple.com/library/archive/documentation/3DDrawing/Conceptual/MTLBestPracticesGuide/TripleBuffering.html)).

### Gate 3: correct one-file editor

- Add a narrow `alpine-runtime` with application delegate, app context, window
  context, synchronous main-thread events, and bounded worker handoff.
- Define keyboard, pointer, scroll, focus, resize, clipboard, IME, wake, and
  close events with timestamps and modifier identity.
- Use direct `StudioApp -> Workspace -> Editor -> Buffer` ownership.
- Tag every worker result with document and workspace revisions; discard stale
  work before mutation.
- Use standard threads and bounded channels, not a general async runtime.
- Add local copy-on-write text, immutable snapshots, byte offsets, explicit
  grapheme and UTF-16 conversions, transactions, and compact undo and redo.
- Admit `crop` only after approved dependency, license, unsafe, Unicode,
  startup, binary-size, and transitive review. Use Ropey only if the accepted
  corpus rejects `crop`; do not build a custom rope before dogfooding.
- Add independent `String` differential models, random Unicode edits, selection
  transforms, line endings, undo, redo, and invalid-boundary rejection.
- Add quads, clips, monochrome glyph instances, CoreText shaping, an A8 atlas,
  and ordered paint operations only.
- Start with a 16 MiB glyph-atlas budget and 32 MiB line-layout ceiling. These
  are explicit Alpine budgets subject to owner approval and measurement.
- Build visible lines plus bounded overscan. A cache hit avoids materializing
  text and shaping.

Exit when one real file can be opened, edited, selected, copied, pasted,
composed through IME, undone, redone, saved atomically, externally changed, and
closed without data loss.

### Gate 4: local workspace shell

- Add folder open, virtualized tree, tabs, splits, active editor, navigation
  history, quick open, command palette, find and replace, and project search.
- Index with low-priority bounded workers, ignore rules, streaming results,
  explicit truncation, and no input or rendering stalls.
- Persist versioned checksummed session state by atomic replacement. Corruption
  falls back cleanly without touching user files.
- Add only rows, columns, splits, overlays, and uniform virtual lists. Do not
  add browser flexbox or CSS.
- Keep indexing, settings expansion, and restoration enrichment off the
  startup-to-first-edit path.

### Gate 5: selected daily-driver profile

- Compile Rust, Markdown, TOML, and JSON grammars into the product.
- Add one bounded local JSON-RPC/LSP transport and qualify only
  `rust-analyzer` first.
- Support diagnostics, completion, hover, definition, references, rename,
  formatting, symbols, cancellation, restart, and stale-result rejection.
- Centralize typed settings, theme values, keymaps, and commands.
- Add native accessibility roles, values, selection, text ranges, focus,
  actions, and announcements.
- Add crash-safe restoration and an opt-in local diagnostic overlay with no
  telemetry or network I/O.
- Dogfood on Alpine repositories through sustained editing sessions.

Exit with zero known data-loss or lifecycle defects, no unbounded memory
growth, no idle submissions, correct IME and accessibility journeys, and a
main-thread p99 below the calibrated 120 Hz budget for selected typing and
scrolling workloads. The frame budget is a workload gate, not a universal
startup target.

## Requirement revisions required before implementation

| Requirement | Accepted v1 scope |
| --- | --- |
| #32 | Local workspace, tabs, splits, navigation, restoration |
| #33 | Local text editing, viewport mapping, shaping, large-file behavior |
| #34 | Built-in search, syntax, symbols, Rust language intelligence |
| #35 | Post-dogfood terminal, tasks, and Git UI, not a daily-driver blocker |
| #36 | Typed settings, themes, keymaps, command discovery, no extensions |
| #37 | Single-window input, clipboard, IME, focus, accessibility, recovery |
| #38 | Correctness and scoped renderer and product qualification |
| #39 | Optical latency only after timestamp qualification |
| #40 | Immutable comparator pin and approved periodic requalification |

## Correctness gates

| Subsystem | Mandatory evidence |
| --- | --- |
| Runtime | Pure transitions, bounded-model evidence where useful, mutation, native E2E, close and fault injection |
| Text | `String` oracle, Unicode properties, random edits, undo model, fuzzing, Miri where applicable |
| Rendering | Semantic scene, CPU raster oracle, offscreen readback, malformed-scene rejection, Metal validation |
| Input | Native replay, IME ordering and cancellation, focus loss, clipboard failure, key repeat |
| Workspace | Filesystem fixtures, corrupt state, stale paths, external changes, save failure, shutdown races |
| Language | Mock server, malformed messages, cancellation races, stale revisions, pinned real server |
| Accessibility | Tree snapshots, roles, values, actions, text ranges, keyboard journey, VoiceOver smoke |
| Memory | Exact accounting, allocator and footprint samples, cache ceilings, soak, post-close drain |

Critical state machines, native unsafe code, text mutation, and renderer
ownership require full changed-line coverage and viable-mutant rejection. Thin
process wrappers are validated through process behavior, never synthetic test
booleans.

## Performance qualification

The governing rules are in [Alpine comparator protocol v1](../quality/comparator-protocol.md).
Two claim families are permitted:

- Alpine GPUI versus pinned GPUI for semantically matched renderer workloads.
- Alpine Studio versus pinned Zed and externally measured Sublime for named,
  normalized local-editor journeys.

No editor result supports a universal fastest-framework claim. Correctness is
admission, performance is measured next, memory and resource efficiency are
measured with the same behavior, and delivery speed is optimized only inside
those constraints.
