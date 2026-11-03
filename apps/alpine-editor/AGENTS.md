---
product: Alpine Editor
role: editor operating rules
author: Deepak Buddha
authority: ../../project.md
related: [../../AGENTS.md, ../../project.md, ../../notes.md, src/AGENTS.md]
parent: ../../AGENTS.md
updated: 2026-11-03
stage: evidence and stability
language_targets: [Rust, Python, C++, Java, TypeScript, OCaml, Haskell]
implemented_language_evidence: Rust workflows; C++ diagnostics; other servers need qualification
known_defects:
  - "physical native validation: missing or zero presentedTime withholds close until timeout; the error identifies the missing evidence, but qualification still fails"
  - "a sixth language with five attached slots stops the least recently used other tab's server; it restarts when shown, but unsaved overlays await PR #672"
  - "didChange sends the whole document (lsp_language.rs did_change_params)"
  - "Open With and drag to Dock fail: no CFBundleDocumentTypes or open-document handler (was #543)"
  - "no horizontal scroll, so a caret past the right edge stays hidden (rest of #555)"
  - "unsaved workspace overlays lost on tab switch; PR #672, refresh its remote head before review"
parked_branch: "feat/lsp-manager at 6600f40, local only: fix its blockers (ARCHIVE F1-F10) before salvaging the Cow JSON fix, scroll skip and pool tests in stage 3"
out_of_scope: [AI, collaboration, extensions, remote development, multi-cursor, built-in terminal]
---

# Alpine Editor

Alpine Editor is the first product on Alpine GPUI. Read
[project.md](../../project.md) for the seven-language scope, stage order,
performance contract and owner checkpoints, and [notes.md](../../notes.md)
for current evidence. Keep the core embeddable for the later terminal.

## Feature contract

Implement the workflow scope in project.md, not full Zed feature parity.
The daily Vim subset, structural editing, local Git essentials, preferences
and Markdown preview are release requirements. Multi-cursor is excluded.
Debugging follows daily-driver functionality in stage 6; it is no longer
excluded. Standard language tools remain in scope, with inlay hints off by
default. No AI or cloud functionality is planned.

The language registry must eventually cover all seven target languages.
OCaml and Haskell are not implemented today. Deleting a registry entry removes
its language lookup without a code change; it does not remove a required
language from release acceptance. Highlighting must work without a server.
Keep the existing 100 ms cold/warm highlighting target and deterministic
visible-range work counters; M1's recorded highlight rows do not qualify the
new languages.

## Performance

The release targets and existing qualified budgets live only in project.md.
Use correctness and qualified non-regression gates during development; a
release shortfall needs Deepak's explicit acceptance. Missing screen evidence
is not a pass. Final product claims use the everyday profile and equivalent
workspace/server work, with controlled results reported separately.

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

Implementation contracts for language tools and Git live in src/AGENTS.md.
Debugger and structural-parser implementation need their scoped designs before
code; dependency, public API and unsafe-boundary changes need approval.

## Verification

Launch `~/Applications/Alpine Editor.app` from the Dock with a disposable HOME.
That hides `~/.rustup`, so set `RUSTUP_HOME` or `ALPINE_RUST_ANALYZER`. Capture
through ScreenCaptureKit (`tools/onscreen-sdr-capture`); `screencapture -l`
cannot see the Metal layer. The capturing terminal needs Screen Recording
permission. Clean up only the processes a capture owns; never close unrelated
apps. Window > Save Performance Log writes `perf-<unix s>-frames.tsv` and
`-samples.tsv` to `~/Library/Logs/Alpine Editor/` for T3 dogfood; a panic
leaves `crash-<unix>-<pid>.log` there, and a main-thread dispatch over 1 s
leaves `stall-<unix>-<pid>.log`.

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
