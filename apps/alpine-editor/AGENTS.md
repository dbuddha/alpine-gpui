---
product: Alpine Editor
current_phase: 1
phase_name: reachable
phase_gate: criteria must pass on main with evidence before phase 2 starts
execution: serial, one feature at a time, one worktree
verification: launch ~/Applications/Alpine Editor.app and look at it
parity_reference: pinned Zed v1.15.0 at alpine-zed-lab/.lab/zed
delivery: docs/delivery.md
---

# Alpine Editor

One code editor for Apple Silicon macOS, on Alpine GPUI, that its author uses
every day. Owned end to end and understandable without a plugin API. The root
[AGENTS.md](../../AGENTS.md) owns the framework, its performance contract and
the shared engineering rules; this file owns the product.

## Current phase: 1, reachable

Do not start phase 2 work. The full table is in
[docs/delivery.md](../../docs/delivery.md). Phase 1 closes when all of these
pass on main, each exercised in the installed app from a Dock launch with no
terminal, each with a screenshot:

| # | Criterion |
| --- | --- |
| 1.1 | `cmd-o` opens a picker taking a file or folder, and the choice opens |
| 1.2 | One click on a file tree row opens that file |
| 1.3 | Folder open shows the tree and an empty buffer, not the sample text |
| 1.4 | Keys match pinned Zed: `f12` definition, `f2` rename, `cmd-shift-i` format, `alt-shift-f12` references, `cmd-k cmd-i` hover, `ctrl-g` go to line, `cmd-shift-o` outline, `cmd-shift-e` project panel, `cmd-o` open, `cmd-s` save, `cmd-shift-s` save as |
| 1.5 | Edit menu Undo, Cut, Copy, Paste, Select All are enabled and work |
| 1.6 | rust-analyzer starts with `ALPINE_RUST_ANALYZER` unset |
| 1.7 | Launches from `~/Applications/Alpine Editor.app` with an icon |

Phase 1 is wiring: the capability exists and is unreachable.

The bet is that a real editor can hold a real project in a fraction of the
memory the alternatives need, and stay at 120 Hz while doing it. Every product
decision here is downstream of keeping that true.

## Two bars, both required

**It must behave as a macOS application.** None of these is polish:

- A menu bar with File, Edit, View and Window, so commands are discoverable
  without memorising shortcuts. Installed before first activation, otherwise
  macOS shows the executable name.
- Open and save panels, so a file or folder can be chosen from inside the app
  rather than only as a launch argument.
- Multiple windows in one process, an installed bundle, a stable identity, an
  icon.
- One coherent visual language across every surface.

**It must not spend the framework's budget.** The editor is where a bounded
framework gets turned into an unbounded application, so:

- Read nothing until it is opened. Folder open takes a directory listing and a
  scratch buffer, never file contents. The lazy tree, the lazy quick-open
  inventory and the absence of a startup index are load-bearing, not accidents.
- Lay out the visible range plus overscan. Never the document.
- Every new cache, history, journal or result set declares a ceiling and an
  eviction rule before it is written.
- Long work goes to a bounded worker and its result is admitted by document and
  workspace revision, never applied because it arrived.
- The language server is the largest process in the system. Bound what is
  retained from it and never let its lifetime follow a document's.

Feature target: local editing, tabs, panes, search, and Rust language support.
Not in scope: AI, multiplayer, extensions, terminal, database views, agent dock.

## Design

Zed is the visual reference. Every surface follows one written design spec
covering type scale, spacing, colour roles, focus treatment and chrome density.
Do not invent per-surface styling; if the spec does not answer a question,
extend the spec and then apply it everywhere it applies.

Surfaces that must agree: file tree, tab strip, gutter, status bar, find and
replace, command palette, quick open, project search. Incoherence between any
two of them is a defect, and it is the current state.

## Verification

A command in `commands.rs` with no menu item, no key and no visible affordance
is not a feature. That error has already been made here.

Check every user-visible change by launching the app and capturing the window
through ScreenCaptureKit, as `tools/onscreen-sdr-capture` does, comparing
against the spec and against Zed for the same surface. `screencapture -l`
cannot see the Metal layer and reports a blank window. Use a disposable `HOME` so a restored session is not mistaken for
current behavior. Latency and memory claims follow the root guide's measurement
rules and need a matched workload on both sides.

## Correctness that must never regress

Own document, workspace and focus revisions across bounded worker results.
Preserve unsaved documents across language-server restarts. Test multiline
lexical invalidation, grapheme and UTF-16 boundaries, atomic-save failure,
external changes on disk, restore corruption, quit and cancel, IME composition,
and accessibility text ranges. Use disposable fixtures and clean up processes
the capture owns.

Settings, session and recovery journals live under
`~/Library/Application Support/Alpine Editor`. Data already in the current
location wins per file, the pre-rename `Alpine Studio` directory is never
modified, and corrupt or conflicting input produces a visible recovery outcome
rather than silent replacement.

## Known open defects

Open issues are the backlog; read them, not a list here. One matters before
touching rendering: `presentedTime` is always zero on the development Mac
(#511), so presentation cannot be timed there.

Daily use is the acceptance test. A defect you hit while editing is the backlog.
