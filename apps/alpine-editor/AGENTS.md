---
product: Alpine Editor
parent: ../../AGENTS.md
updated: 2026-10-01
phase: "2, language agnostic: active, closes in M4"
parity:
  core_editing: {status: partial, closes: M6}
  navigation_and_search: {status: partial, closes: M8}
  language_intelligence: {status: partial, closes: M4}
  git: {status: none, closes: M7}
criteria:
  "2.1": "recorded pass; its test times warm cache hits only, re-measure cold in M1"
  "2.2": "Rust passes; C++ shows clangd diagnostics only; Python, Java, TypeScript and JavaScript need a server"
  "2.3": "two warm passes; sixth eviction tested only with one slot already detached"
  "2.4": "passes on main"
known_defects:
  - "server eviction and idle shutdown join the supervisor on the main thread, up to 5 s (language_services.rs evict_one, reap_idle)"
  - "a sixth language with five attached slots kills an open tab's server; untested"
  - "didChange sends the whole document (lsp_language.rs did_change_params)"
  - "workspace.applyEdit is advertised but server requests get MethodNotFound"
defects_to_reverify: ["#543", "#555", "#576", "#533", "#622", "#304", "#511", "#522"]
parked_branch: "feat/lsp-manager at 6600f40, local only: fix its blockers (ARCHIVE F1-F10) before salvaging the Cow JSON fix, scroll skip and pool tests in M4"
out_of_scope: [AI, collaboration, extensions, remote development, debugger, built-in terminal]
---

# Alpine Editor

The daily-driver editor on Alpine GPUI: beat Zed at parity within the
framework's budgets, and keep the core embeddable for the later terminal.

## Criteria by phase

Phase 2 (M4): each language group highlights within 100 ms with no server;
definition, hover and references work once a server is ready; two languages stay
warm and a sixth evicts by idle order; deleting a registry entry removes a
language with no code change.

Phase 3 (M6): `cmd-shift-l` edits all matches; syntax-node expand and shrink; a
Vim subset; 500 cursors on 10,000 lines under 16 ms per keystroke; undo restores
every cursor. `SelectionSet` and `Transaction` already handle multi-cursor.

Phase 4 (M7): breadcrumbs; branch name within 1 s of checkout; blame for the
visible range; gutter diff; a stage-and-commit panel; no layout shift.

Phase 5 (M8): independent windows; the design system as one tokens module; no
element shift; the parity sweep and final run against Zed.

Parity also needs auto-indent, bracket pairing, deep undo, huge-file budgets,
inline diagnostics, completion, rename, format, code actions, inlay hints,
signature help, file finder and outline.

## Two bars

**A real macOS app:** File, Edit and Window menus exist; add the rest as
features land, installed before first activation. Open and save panels, an
installed bundle, a stable identity, an icon, one visual language. A command
reachable only from the command palette is not shipped.

**The framework's budget:** read nothing until it is opened; folder open lists a
directory and nothing more. Lay out the visible range. Every cache, history or
result set declares a ceiling and eviction rule first. Long work goes to a
bounded worker and is admitted by revision. Degrade visibly on huge input; keep
indexing and restore off the startup path.

## Language intelligence: quiet by default (M4 target)

Today: one server per workspace and language, hard cap 5, idle shutdown of
unattached slots, local-lexer highlighting. The `known_defects` above still
hold. rust-analyzer alone is 1 to 4 GB on a real crate.

The M4 design:
- Start a server on the first visible file of its language. Evict and shut down
  off the main thread.
- Incremental didChange from edit transactions, coalesced per tick, through a
  bounded queue that merges when full. Typing never waits on a server.
- Completion on trigger characters, a typing pause or a key: one request in
  flight, cancelled on supersede, filtered locally as the prefix grows, details
  resolved only for the selected item.
- Hover on a key or a deliberate mouse rest; signature help on `(` and `,`;
  code actions and references only when asked. Diagnostics render after a
  typing pause, capped, visible range only. Inlay hints off by default. Never
  semantic tokens.
- Parse JSON off the main thread; admit results within a per-frame budget, by
  document revision. Cancellation is advisory; local revocation by request ID
  is authoritative. While a server indexes, suppress optional requests.
- A missing server shows its install command. Never download one.

## Git: the CLI, long-lived (M7 target)

Detect the repository without spawning; start nothing until a feature needs it.
One persistent `git cat-file --batch` serves file contents. The gutter diff runs
on a worker after a typing pause. `git blame --porcelain -L` covers the visible
range on demand. The branch comes from reading `.git/HEAD`. `git status
--porcelain=v2 -z` is debounced and capped. Stage and commit are explicit. At
most two git processes, each with a timeout.

## Verification

Launch `~/Applications/Alpine Editor.app` from the Dock with a disposable HOME.
That hides `~/.rustup`, so set `RUSTUP_HOME` or `ALPINE_RUST_ANALYZER`. Capture
through ScreenCaptureKit (`tools/onscreen-sdr-capture`); `screencapture -l`
cannot see the Metal layer. The capturing terminal needs Screen Recording
permission. Clean up only the processes a capture owns; never close unrelated
apps. Compare against the same surface in Zed.

## Correctness that must never regress

Own document, workspace and focus revisions across worker results. Preserve
unsaved documents across server restarts. Test multiline lexical invalidation,
grapheme and UTF-16 boundaries, atomic-save failure, external changes on disk,
restore corruption, quit and cancel, IME composition and accessibility ranges.
Panes hold a tab identity and their own scroll; the tab store owns buffers.

Settings, session and recovery live under
`~/Library/Application Support/Alpine Editor`. Current data wins per file, the
pre-rename `Alpine Studio` directory is never modified, and corrupt input gives
a visible recovery outcome.
