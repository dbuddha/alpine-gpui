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
| [WGPU case study](../case-studies/wgpu.md) and [research package](../research/wgpu/index.md) | Layered GPU validation, lifetime, staging, surface recovery, and test taxonomy | Alpine performance or a shipping backend decision |
| [Research #113](https://github.com/dbuddha/alpine-gpui/issues/113) | Comparator baseline questions | Accepted benchmark evidence |
| [Research #114](https://github.com/dbuddha/alpine-gpui/issues/114) | Sublime evidence questions | Private implementation facts |
| [Research #115](https://github.com/dbuddha/alpine-gpui/issues/115) | Adaptation separation | Renderer qualification |
| [Research #116](https://github.com/dbuddha/alpine-gpui/issues/116) | Fixed-hardware questions | Completed hardware windows |
| [Comparator protocol v1](../quality/comparator-protocol.md) | Measurement and claim rules | Any result before raw evidence exists |

## Current implementation status

Snapshot: reviewed through the Rust diagnostics implementation in
[PR #217](https://github.com/dbuddha/alpine-gpui/pull/217), 2026-08-18. This is
an implementation inventory, not a release claim. GitHub issues remain the
authority for live task and check status.

| Gate | Status | Merged evidence | Remaining exit work |
| --- | --- | --- | --- |
| 0: authoritative line | Complete | Owner-approved Requirements #32 through #37, one current `main`, issue-first tracking, and a leaf-execution Project projection | Keep issue hierarchy canonical and refuse inferred Project state when an operator lacks `read:project` |
| 1: production window | Complete | Production `NativeSurface::run`, real AppKit window, process close path, watchdog and zero-idle evidence | Continue native regression coverage as product behavior grows |
| 2: bounded presentation | Complete | AEP 0120, three completion-owned slots, asynchronous commit and present, capacity accounting, close and reorder tests | Fixed-hardware latency and residency qualification remains later evidence |
| 3: one-file editor | Functionally implemented, dogfood acceptance still open | Runtime and events, local rope text, snapshots, Unicode mappings, undo and redo, CoreText layout, glyph atlas, pointer and keyboard selection, clipboard, IME, atomic save, external-change protection | Sustained manual use, defect closure, and release-quality native journey evidence |
| 4: workspace shell | Functionally implemented by closed Task #127 | Folder launch, bounded lazy tree, tabs, splits, history, find and replace, quick open, command palette, project search, clean and dirty restoration, lazy inactive tabs | Sustained repository-scale dogfood and large-workspace evidence |
| 5: daily-driver profile | In progress | Compiled syntax, typed settings and shortcuts, no-bloat enforcement, revisioned accessibility semantics, bounded LSP framing, local process ownership, JSON-RPC, pinned rust-analyzer qualification, runtime wake admission, and visible Rust diagnostics | Finish completion, hover and navigation, rename and formatting, symbols, configuration reload, native accessibility and recovery, sustained dogfood, and blocking defect closure |

Alpine Studio is therefore a real bounded editor foundation, not merely a solid
quad demo. It is not yet the promised daily driver because Gate 5 and sustained
qualification remain open.

## Remaining critical path

The stale implementation stack and revision-safe visible Rust diagnostics are
merged. The remaining path is expressed as thin leaf issues rather than
branches or broad parent tasks:

1. Add bounded completion through
   [Task #218](https://github.com/dbuddha/alpine-gpui/issues/218).
2. Add hover, definitions, references, and validated local navigation through
   [Task #219](https://github.com/dbuddha/alpine-gpui/issues/219).
3. Add bounded atomic rename and formatting through
   [Task #220](https://github.com/dbuddha/alpine-gpui/issues/220).
4. Add document and workspace symbols through
   [Task #221](https://github.com/dbuddha/alpine-gpui/issues/221).
5. Qualify and merge typed configuration reload and migration through
   [Task #222](https://github.com/dbuddha/alpine-gpui/issues/222).
6. Complete onscreen SDR, lifecycle soak, production journey, and native idle
   qualification through [Tasks #234 through #237](https://github.com/dbuddha/alpine-gpui/issues/72),
   then qualify native VoiceOver and lifecycle recovery through
   [Task #223](https://github.com/dbuddha/alpine-gpui/issues/223).
7. Restore bounded Miri assurance through
   [Defect #183](https://github.com/dbuddha/alpine-gpui/issues/183) and include
   Studio application lines in changed-line coverage diagnostics through
   [Defect #232](https://github.com/dbuddha/alpine-gpui/issues/232).
8. Run revision-pinned capture, sustained Alpine-repository sessions,
   interaction baselines, long-session residency, and final M5 acceptance
   through [Tasks #238 through #242](https://github.com/dbuddha/alpine-gpui/issues/224)
   under dogfood Task #224.

No terminal, Git UI, plugin, AI, cloud, collaboration, telemetry, remote, or
multi-window work may preempt this path.

Tasks #218, #219, and #221 are merged Rust-intelligence leaves; #220 remains the
publication gap. Task #222 is the active independent configuration leaf. Task
#72 aggregates native leaves #234 through
#237 and remains the blocker for Task #223. Task #224 aggregates dogfood leaves
#238 through #242 and is the convergence gate; neither parent can close while a
child or linked assurance, data-loss, lifecycle, accessibility, idle-work, or
unbounded-residency defect remains open.

## How GitHub milestones map to readiness

The existing framework milestones predate the vertical Studio slices, so issue
counts alone do not describe product depth.

| Milestone | Meaning for Studio | Daily-driver interpretation |
| --- | --- | --- |
| M0 | Governance and evidence system | Closed foundation, not product readiness |
| M1 and M2 | Renderer and native presentation | Core behavior exists, but residual milestone issues still require explicit disposition |
| M3 | Local workspace shell | Closed product slice, not complete daily-driver readiness |
| M4 | Text, IME, accessibility, recovery | Text is complete; native accessibility and recovery remain open |
| M5 | Rust-first Alpine Studio daily-driver profile | Daily-driver behavior and dogfood gate for Apple Silicon macOS |
| M6 | Linux and Windows backends | Explicitly after the Apple daily driver |
| M7 | Version 1 stabilization and distribution | Release gate after daily-driver behavior and qualification |

Passing M5 means the selected Apple Silicon macOS editor profile has met its
daily-driver behavior and dogfood gate. It does not mean Alpine Studio is a
supportable version 1 release. M7 separately owns fixed-hardware regression
qualification, API stabilization, packaging, signing, notarization, update
recovery, and release evidence. M6 is not on the macOS critical path.

Milestone issue counts and total Project item counts must never be presented as
readiness percentages. They mix capabilities, requirements, research,
decisions, defects, and implementation leaves. Historical progress is measured
from a recorded leaf scope: closed evidence-producing leaves form the burn-up,
scope additions are reported separately, and parent progress is derived rather
than counted as another completed unit.

## Acceptance-gated execution

### Gate 0: authoritative execution line

- Keep `main` and the accepted issue hierarchy authoritative.
- Preserve abandoned or dirty worktrees unless a bounded reconciliation task
  proves which commits belong on current `main`.
- Keep Requirements #32 through #37 within their owner-approved product scope.
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

- Preserve the AEP 0120 three-slot frame-resource ring and prohibit callback
  `waitUntilCompleted` regressions.
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
- Preserve the Ropey boundary accepted by Decision #139 after Crop failed the
  nested-slice and UTF-16 corpus. Do not build a custom rope before dogfooding
  produces evidence that the accepted boundary is insufficient.
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

## Accepted requirement scope

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
