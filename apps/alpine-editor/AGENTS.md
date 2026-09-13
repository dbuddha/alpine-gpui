# Alpine Editor

One personal code editor for Apple Silicon macOS, built on Alpine GPUI. The
objective is an editor its author uses every day, owned end to end and
understandable without a plugin API. Root [AGENTS.md](../../AGENTS.md) covers
the framework and the shared engineering rules; this file owns the product.

## The bar

It must behave as a real macOS application, not a harness with a bundle around
it. That means, and none of these are polish:

- A menu bar with File, Edit, View and Window, so commands are discoverable
  without memorising shortcuts.
- Open and save panels, so a file or folder can be chosen from inside the app
  rather than only as a launch argument.
- Multiple windows in one process, with an installed bundle, a stable identity
  and an icon.
- One coherent visual language across every surface.

Feature target: local file editing, tabs, panes, search, and Rust language
support. Explicitly not in scope: AI, multiplayer, extensions, terminal,
database views, or an agent dock.

## Design

Zed is the visual reference. Every surface follows one written design spec
covering type scale, spacing, colour roles, focus treatment and chrome density.
Do not invent per-surface styling; if the spec does not answer a question,
extend the spec first and then apply it everywhere it applies.

Surfaces that must agree: file tree, tab strip, gutter, status bar, find and
replace, command palette, quick open, and project search. Incoherence between
any two of them is a defect.

## Verification

Judge the product by using it, never by reading the code that implements it. A
command that exists in `commands.rs` but has no menu item, no panel and no
visible affordance is not a feature.

Every user-visible change is checked by launching the app and looking at it.
Capture the window with `screencapture` and inspect the result. Compare against
the design spec and against Zed for the same surface.

Prefer a disposable `HOME` when launching for a check, so a restored session
cannot be mistaken for current behavior.

## Local state

Settings, session and recovery journals live under
`~/Library/Application Support/Alpine Editor`. Data already in the current
location always wins over imported legacy data, the pre-rename `Alpine Studio`
directory is never modified, and corrupt or conflicting input produces a visible
recovery outcome rather than silent replacement.

## Editor correctness

Own document, workspace and focus revisions across bounded worker results.
Preserve unsaved documents across language-server restarts. Test multiline
lexical invalidation, grapheme and UTF-16 boundaries, atomic-save failure,
external changes, restore corruption, quit and cancel, IME, and accessibility
text ranges. Use disposable fixtures and clean up processes the capture owns.

Daily use is the acceptance test. A defect you hit while editing is the backlog.
