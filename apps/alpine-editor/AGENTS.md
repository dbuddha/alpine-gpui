# Alpine Editor

One code editor for Apple Silicon macOS, on Alpine GPUI, that its author uses
every day. Owned end to end and understandable without a plugin API. The root
[AGENTS.md](../../AGENTS.md) owns the framework, its performance contract and
the shared engineering rules; this file owns the product.

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

Judge the product by using it, never by reading the code that implements it. A
command that exists in `commands.rs` with no menu item, no panel and no visible
affordance is not a feature. That error has already been made here.

Every user-visible change is checked by launching the app and looking at it.
Capture the window with `screencapture` and inspect the result against the spec
and against Zed for the same surface. Launch with a disposable `HOME` so a
restored session cannot be mistaken for current behavior.

For anything touching latency or memory, follow the measurement rules in the
root guide. Editor-side claims need a matched workload on both sides.

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

Presentation cannot be timed on the development Mac because `presentedTime` is
always zero (#511), which also plausibly explains the intermittent frame
deadline failures in the accessibility controls (#622). The caret does not
follow the active edit (#555). Settings reload is rejected on a clean launch
(#547). `LaunchServices` refuses supported documents (#543). Tab labels overlap
when a multi-tab session restores.

Daily use is the acceptance test. A defect you hit while editing is the backlog.
