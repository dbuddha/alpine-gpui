---
product: Alpine Editor
parent: ../../AGENTS.md
updated: 2026-09-30
phase: "2, language agnostic: active, closes in M4"
parity:
  core_editing: {status: partial, closes: M6}
  navigation_and_search: {status: partial, closes: M8}
  language_intelligence: {status: partial, closes: M4}
  git: {status: none, closes: M7}
criteria:
  "2.1": "recorded pass; its test times warm cache hits only, re-measure cold in M1"
  "2.2": "Rust and C++ pass on main; Python, Java, TypeScript and JavaScript need a server on PATH"
  "2.3": "two warm passes; sixth eviction tested only with one slot already detached"
  "2.4": "passes on main"
defects_to_reverify: ["#543", "#555", "#576", "#533", "#622", "#304", "#511", "#522"]
parked_branch: "feat/lsp-manager at 6600f40, local only: salvage the Cow JSON fix, scroll skip and pool tests in M4; the downloader is dropped"
out_of_scope: [AI, collaboration, extensions, remote development, debugger, built-in terminal]
---

# Alpine Editor

The daily-driver code editor on Alpine GPUI. It must beat Zed at parity while
staying inside the framework's budgets. Keep the editor core embeddable: the
later terminal app hosts it as a native pane.

## Criteria by phase

Phase 2 (M4): each of the five language groups highlights within 100 ms with
no server; definition, hover and references work once a server is ready; two
languages stay warm and a sixth evicts by idle order; deleting a registry entry
removes a language with no code change.

Phase 3 (M6): `cmd-shift-l` edits all matches; syntax-node expand and shrink;
a Vim subset; 500 cursors on 10,000 lines under 16 ms per keystroke; undo
restores every cursor. `SelectionSet` and `Transaction` already handle
multi-cursor.

Phase 4 (M7): breadcrumbs; branch name within 1 s of checkout; blame for the
visible range only; gutter diff; a stage-and-commit panel; no layout shift.

Phase 5 (M8): independent windows; the design system as one tokens module in
code; no element shift; the parity sweep and final run against Zed.

Parity also needs auto-indent, bracket pairing, deep undo, huge-file budgets,
inline diagnostics, completion, rename, format, code actions, inlay hints and
signature help for all five groups, file finder and outline.

## Two bars

**A real macOS app:** the File, Edit and Window menus exist; add the rest as
features land, installed before first activation. Open and save panels, an
installed bundle, a stable identity, an icon, one visual language. A command
with no menu item, key or visible affordance is not shipped.

**The framework's budget:** read nothing until it is opened; folder open lists
a directory and nothing more. Lay out the visible range. Every cache, history
or result set declares a ceiling and eviction rule first. Long work goes to a
bounded worker and is admitted by revision. Degrade visibly on huge input; keep
indexing and restore off the startup path.

## Language intelligence: quiet by default

- One server per workspace and language, started on the first visible file of
  that language. Idle shutdown applies only to unattached slots; hard cap 5.
  rust-analyzer alone is 1 to 4 GB on a real crate. Eviction and shutdown never
  run on the main thread.
- didChange is incremental, built from edit transactions and coalesced per
  tick. The outbound queue is bounded; when full, pending changes merge.
  Typing never waits on a server.
- Completion fires on trigger characters, a typing pause or a key: one request
  in flight, cancelled on supersede, filtered locally while the prefix grows,
  details resolved only for the selected item.
- Hover on a key or a deliberate mouse rest; signature help on `(` and `,`;
  code actions and references only when asked. Diagnostics render after a
  typing pause, capped, visible range only. Inlay hints are off by default.
  Highlighting uses local lexers, never semantic tokens.
- JSON parses off the main thread; the main thread admits results within a
  per-frame budget, by document revision. Cancellation is advisory; local
  revocation by request ID is authoritative.
- While a server reports indexing, suppress optional requests and show status.
- Missing server: show the exact install command. Never download one.

## Git: the CLI, long-lived

Detect the repository without spawning and start nothing until a feature needs
it. One persistent `git cat-file --batch` serves file contents. The gutter diff
runs on a worker after a typing pause. `git blame --porcelain -L` covers the
visible range on demand. The branch comes from reading `.git/HEAD`. `git status
--porcelain=v2 -z` is debounced and capped. Stage and commit are explicit. At
most two git processes, each with a timeout.

## Verification

Launch `~/Applications/Alpine Editor.app` from the Dock with a disposable HOME,
which hides `~/.rustup`: set `RUSTUP_HOME` and `CARGO_HOME` or
`ALPINE_RUST_ANALYZER`. Capture through ScreenCaptureKit
(`tools/onscreen-sdr-capture`); `screencapture -l` cannot see the Metal layer.
The capturing terminal needs Screen Recording permission; accessibility tools
need Accessibility trust. Compare against the same surface in Zed.

## Correctness that must never regress

Own document, workspace and focus revisions across worker results. Preserve
unsaved documents across server restarts. Test multiline lexical invalidation,
grapheme and UTF-16 boundaries, atomic-save failure, external changes on disk,
restore corruption, quit and cancel, IME composition and accessibility ranges.
Panes hold a tab identity and their own scroll; the tab store alone owns
buffers.

Settings, session and recovery live under
`~/Library/Application Support/Alpine Editor`. Current data wins per file, the
pre-rename `Alpine Studio` directory is never modified, and corrupt input gives
a visible recovery outcome.
