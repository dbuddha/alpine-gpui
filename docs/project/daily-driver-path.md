# Alpine Studio private daily-driver path

This document owns the stable execution path. GitHub owns live work status,
priority, blockers, owners, and acceptance. Read the
[daily-driver Project](https://github.com/users/dbuddha/projects/1),
[milestones](https://github.com/dbuddha/alpine-gpui/milestones), and linked
issues for current state. If Project access is unavailable, the issue hierarchy
and dependencies are authoritative; an inaccessible Project is not an empty
Project.

## Overall goal

Alpine GPUI is an Apple-first Rust GPU UI framework designed to make demanding
native applications immediate while keeping correctness, ownership, failure,
latency, allocation, and residency observable. Alpine Studio is its first
product proof: a local-only editor for one developer that approaches Zed's
accepted editing quality with Sublime Text's focused product boundary.

The program succeeds in two independently visible stages:

1. Alpine Studio becomes a safe, smooth, memory-bounded private daily driver for
   sustained work on the Alpine repositories.
2. Alpine GPUI and Alpine Studio earn narrowly scoped comparative claims through
   semantically equivalent, revision-pinned E4 evidence.

The daily-driver stage does not depend on proving renderer dominance. The claim
stage never receives credit for omitted behavior.

## Ordering principles

Work is ordered by:

1. Correctness.
2. Responsiveness and renderer performance.
3. CPU, GPU, and memory efficiency.
4. Delivery speed within the earlier gates.

Research or governance work belongs on the critical path only when it resolves
an accepted implementation decision, repairs conflicting project truth,
preserves required evidence, or enables an acceptance gate.

## Private daily-driver definition

The accepted product capability is
[Capability #28](https://github.com/dbuddha/alpine-gpui/issues/28). A private
daily driver is a revision-pinned production `.app` that supports sustained
Alpine repository work with:

- Immediate native typing and scrolling.
- Correct local text editing, Unicode, IME, selection, clipboard, undo, redo,
  atomic saving, external-change handling, and recovery.
- Folder opening, a virtualized file tree, tabs, splits, navigation history,
  quick open, command palette, find and replace, and bounded project search.
- Built-in Rust, plain text, Markdown, TOML, and JSON behavior.
- Local `rust-analyzer` diagnostics, completion, hover, navigation, references,
  rename, formatting, symbols, cancellation, restart, and stale-result rejection.
- Typed settings, themes, keymaps, project precedence, reload, and migration.
- Correct keyboard, pointer, scroll, focus, accessibility, lifecycle, and close
  behavior in one owned AppKit window.
- No idle frame submission, bounded foreground and background queues, bounded
  caches and indexes, no unbounded memory slope, and accepted post-close drain.
- Local diagnostics that perform no telemetry and no network output.

Private dogfood does not require public signing, notarization, packaging,
updates, release support, or a comparative speed claim.

## Accepted requirement families

| Requirement | Contribution to the daily-driver path |
| --- | --- |
| [#32](https://github.com/dbuddha/alpine-gpui/issues/32) | Restorable local workspaces, tabs, panes, and navigation |
| [#33](https://github.com/dbuddha/alpine-gpui/issues/33) | Production local text editing and bounded large-file viewport behavior |
| [#34](https://github.com/dbuddha/alpine-gpui/issues/34) | Search, syntax, symbols, and local language intelligence |
| [#35](https://github.com/dbuddha/alpine-gpui/issues/35) | Protection against terminal, task, and Git UI scope entering the daily-driver gate |
| [#36](https://github.com/dbuddha/alpine-gpui/issues/36) | Typed settings, themes, keymaps, and command discovery |
| [#37](https://github.com/dbuddha/alpine-gpui/issues/37) | Single-window input, IME, accessibility, lifecycle, and recovery qualification |
| [#38](https://github.com/dbuddha/alpine-gpui/issues/38) | Calibrated correctness and scoped comparative qualification |
| [#39](https://github.com/dbuddha/alpine-gpui/issues/39) | Dedicated optical display-latency qualification |
| [#40](https://github.com/dbuddha/alpine-gpui/issues/40) | Deliberate requalification without silent comparator drift |

The issue label and body are authoritative for approval and acceptance. A link
in this document does not approve or close a requirement.

## Dependency graph

The milestone numbers are identifiers, not a promise that every milestone runs
serially.

```text
M0 governed foundation supports every path

Product path
M2 native macOS presentation -> M4 input, IME, accessibility
M3 local workspace shell -------------------------------> M5 private daily driver
M4 input, IME, accessibility ---------------------------> M5 private daily driver
M5 private daily driver --------------------------------> M7 version 1 stabilization

Renderer qualification path
M1 Direct Metal semantic foundation -> realistic matched traces
    -> E4 renderer and product qualification -> M7 scoped release claims

Deferred platform path
M6 Vulkan, Wayland, D3D12, and Win32
```

M6 does not block the Apple Silicon private daily driver or macOS version 1.
M5 does not require Alpine to outperform GPUI, Zed, or Sublime. M7 cannot publish
comparative claims until the independent renderer-qualification path is green.

## Critical sequence

The stable sequence is:

1. Measure and correct release typing latency through
   [Defect #304](https://github.com/dbuddha/alpine-gpui/issues/304) and
   [Experiment #331](https://github.com/dbuddha/alpine-gpui/issues/331) before
   changing the runtime or renderer architecture.
2. Finish native input, IME, focus, accessibility, lifecycle, and physical
   behavior required by [Requirement #37](https://github.com/dbuddha/alpine-gpui/issues/37).
3. Preserve the implemented language and configuration contracts under
   [Requirements #34](https://github.com/dbuddha/alpine-gpui/issues/34) and
   [#36](https://github.com/dbuddha/alpine-gpui/issues/36), and turn dogfood
   failures into revision-bound regression defects.
4. Run sustained private dogfood and residency journeys; promote each incident
   to a reproduced defect with a production-path regression.
5. Preserve the E3 GPUI admission established by
   [Tasks #334](https://github.com/dbuddha/alpine-gpui/issues/334), #61, and
   #353, then complete #53 timing and adaptation calibration #470, renderer
   residency #471, and independent-window E4 qualification #472.
6. Qualify scoped renderer and product claims, then enter M7 release work.

These links are entry points, not copied status. The issue timeline and Project
fields determine whether an item is ready, active, blocked, or accepted.

## Framework direction

Alpine retains Direct Metal, narrow AppKit and CoreText boundaries, immutable
scenes, structure-of-arrays primitive storage, explicit painter order,
demand-driven invalidation, bounded asynchronous frame ownership, visible-range
work, lookup-before-rasterize glyph admission, delta atlas publication, bounded
worker queues, revision identities, exact resource accounting, and independent
semantic oracles.

Alpine does not recreate GPUI as a compatibility target. A reusable framework
contract is added only when an accepted Studio slice consumes it, two concrete
uses establish the contract, or measured evidence identifies a correctness,
latency, or memory bottleneck. The
[lineage package](../research/alpine-lineage/index.md) records whether each
mechanism is adapted, independently convergent, Alpine-original,
comparator-only, rejected, or deferred.

The [editor rendering doctrine](../concepts/editor-rendering-doctrine.md) makes
this composition explicit: a full native graphical shell, a specialized
visible-range text canvas, purpose-built editor layout, and demand-driven
Direct Metal rendering. It rejects fixed-cell TUI shortcuts, browser layout,
game-engine process, and unmeasured framework generalization while preserving
Unicode, IME, accessibility, smooth scrolling, and native semantics.

The doctrine is enforced through the existing critical path rather than a new
parallel program: exact-main assurance, physical typing attribution #304/#331,
physical accessibility #253/#273, sustained dogfood and residency #239 through
#242, and renderer E4 qualification #470 through #472 under #53. Completed
capture task #238 and E3 lifecycle task #353 are retained admission evidence,
not active blockers. Architecture language does not advance any open gate
without its retained evidence.

## Qualification relationship

Renderer-only comparison, adapter cost, framework scene construction, and full
editor journeys are different measurements. Equivalence is admitted before
timing. Every comparative claim follows the
[comparator protocol](../quality/comparator-protocol.md) and
[claim-readiness rules](claim-readiness.md).

For an editor, idle zero frames is correct. Active typing and scrolling should
meet calibrated display deadlines. Neither outcome alone proves a universal
frame-rate or framework-superiority claim.

## Exit and review

Private daily-driver readiness closes only from accepted leaf evidence for the
complete production journey. Review this path when an accepted requirement,
milestone outcome, product exclusion, platform boundary, comparator protocol,
or evidence policy changes. Routine task status does not require editing this
document.
