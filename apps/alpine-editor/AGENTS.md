---
product: Alpine Editor
parent: ../../AGENTS.md
updated: 2026-10-17
phase: "2, language agnostic: active, closes in M4"
parity:
  core_editing: {status: partial, closes: M6}
  navigation_and_search: {status: partial, closes: M8}
  language_intelligence: {status: partial, closes: M4}
  git: {status: none, closes: M7}
criteria:
  "2.1": "CI proves warm frames lex nothing and a cold first frame lexes only the laid-out range (SyntaxCache counters); the 100 ms budget, cold and warm, becomes a bench row in M1.4"
  "2.2": "Rust passes; C++ shows clangd diagnostics only; Python, Java, TypeScript and JavaScript need a server"
  "2.3": "two warm passes; sixth eviction tested with one slot detached and with five attached"
  "2.4": "passes on main"
known_defects:
  - "a sixth language with five attached slots stops the least recently used other tab's server; it restarts when shown, but unsaved overlays wait for S10"
  - "didChange sends the whole document (lsp_language.rs did_change_params)"
  - "Open With and drag to Dock fail: no CFBundleDocumentTypes or open-document handler (was #543)"
  - "no horizontal scroll, so a caret past the right edge stays hidden (rest of #555)"
  - "unsaved workspace overlays lost on tab switch; fix on local fix/576-overlay-acceptance (was #576)"
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
Vim subset; 500 cursors on 10,000 lines within one display refresh per
keystroke at p95; undo restores every cursor.

Phase 4 (M7): breadcrumbs; branch name within 1 s of checkout; blame for the
visible range; gutter diff; a stage-and-commit panel; no layout shift.

Phase 5 (M8): independent windows; the design system as one tokens module; no
element shift; the final run against Zed.

Parity also needs auto-indent, bracket pairing, deep undo, huge files, inline
diagnostics, completion, rename, format, code actions, inlay hints, signature
help, file finder, outline.

## Performance targets

Against Zed: win every `beat_zed_on` metric in the root frontmatter, checked
at milestone closes. Against the AppKit reference app: keypress-to-screen p50
and p95 each within one display refresh (8.33 ms at 120 Hz) of AppKit's, the
same scroll cadence, and lower memory and energy where the bench shows room.
Every frame-producing interaction holds the display's refresh at p95 (8.33 ms
at 120 Hz, 16.7 ms at 60 Hz) with at most 1% missed frames (gaps over 1.5
refresh periods): typing, scroll, resize, selection drags, overlays, big files
and 500 cursors, with servers and git running. Launch and file-open are exempt
and get their own startup budgets. The AppKit and refresh targets each gate PRs
once a milestone first meets them.

Final comparisons use separate controlled and everyday settings, frozen before
measurement. Require favorable confidence intervals on differences beyond
measurement uncertainty in two independent sessions, with no fixed percentage
margin. Report every required metric; missing values, ties and zero failures
on both sides do not prove superiority. Never pool profiles, and never add
trials after seeing results to force a win.

## Two bars

**A real macOS app:** File, Edit and Window menus exist; new menus install
before first activation. Open and save panels, an
installed bundle, a stable identity, an icon, one visual language. A command
reachable only from the command palette is not shipped.

**The framework's budget:** read nothing until it is opened; folder open lists a
directory and nothing more. Every cache, history or result set declares a
ceiling and eviction rule first. Degrade visibly on huge input; keep indexing
and restore off the startup path.

## Language intelligence and git

Design rules for M4 and M7, and the M4 slices, live in src/AGENTS.md.

## Verification

Launch `~/Applications/Alpine Editor.app` from the Dock with a disposable HOME.
That hides `~/.rustup`, so set `RUSTUP_HOME` or `ALPINE_RUST_ANALYZER`. Capture
through ScreenCaptureKit (`tools/onscreen-sdr-capture`); `screencapture -l`
cannot see the Metal layer. The capturing terminal needs Screen Recording
permission. Clean up only the processes a capture owns; never close unrelated
apps. Window > Save Performance Log writes `perf-<unix s>-frames.tsv` and
`-samples.tsv` to `~/Library/Logs/Alpine Editor/` for T3 dogfood; a panic
leaves `crash-<unix>-<pid>.log` there.

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
