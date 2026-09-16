# Delivery

Alpine Editor is delivered in five phases, executed **serially**. Phase N does
not begin until Phase N-1's acceptance criteria pass on `main` with captured
evidence. The parity reference is pinned Zed v1.15.0, commit `e17dc4f`, checked
out in the comparison lab at `.lab/zed`.

This file holds the phase table, the criteria, and the measurements. The current
phase's criteria are repeated in [apps/alpine-editor/AGENTS.md](../apps/alpine-editor/AGENTS.md)
so an agent reads them before starting work.

## Status

| Phase | Scope | Estimate | State |
| --- | --- | --- | --- |
| 1 | Reachable | 3 to 5 days | **closed**, 7 of 7 criteria pass on `main` |
| 2 | Language agnostic | 1.5 weeks | **active** |
| 3 | Editing parity | 2 to 2.5 weeks | blocked on 2 |
| 4 | Context | 1 week | blocked on 3 |
| 5 | A real application | 2 weeks | blocked on 4 |

## How a phase closes

1. Every criterion is exercised **in the installed application**, launched from
   the Dock, with no terminal involved.
2. Each criterion has evidence: a screenshot, or a recorded number for the
   measured ones.
3. Evidence and measurements are written into this file under that phase.
4. The gate is `main`, not a branch. A criterion passing on a feature branch
   does not close it.

Reading the code is not verification. A command reachable only from the command
palette is not shipped.

## Phase 1: reachable

All wiring. Converts the editor from unusable to usable. No new capability.

| # | Criterion | Evidence |
| --- | --- | --- |
| 1.1 | `cmd-o` opens a picker that accepts a file or a folder, and the choice opens | **passes.** File > Open chose `Cargo.toml` and it opened in a tab; File > Open Folder chose `crates/alpine-scene` and the tree showed its contents. `cmd-o` accepts either kind, as Zed does; Open Folder has no key equivalent because `cmd-shift-o` is the outline |
| 1.2 | One click on a file tree row opens that file | **passes.** Already worked; one click on `Cargo.toml` opened `alpine-scene`'s manifest |
| 1.3 | Folder open shows the tree and an empty buffer, never the `INITIAL_TEXT` sample | **passes.** Criterion reworded from "a real file": the tree loads asynchronously so there is no file to choose at construction, and Zed shows an empty editor here too. What mattered was removing the placeholder sample, which is done |
| 1.4 | Keybindings match pinned Zed: `f12` definition, `f2` rename, `cmd-shift-i` format, `alt-shift-f12` references, `cmd-k cmd-i` hover, `ctrl-g` go to line, `cmd-shift-o` outline, `cmd-shift-e` project panel, `cmd-o` open, `cmd-shift-s` save as, `cmd-s` save | **passes.** Installed `~/Applications/Alpine Editor.app` at `058087b`: Ctrl+G opened the Go to line field; Return on line 8 then Cmd+K Cmd+I showed rust-analyzer hover for `mod` ("Organize code into modules") on `apps/alpine-editor/src/lib.rs`. F12, F2, Cmd+Shift+I, Opt+Shift+F12, Cmd+Shift+O, Cmd+T, Cmd+Shift+E remain bound; File menu still owns Cmd+O, Cmd+S, Cmd+Shift+S |
| 1.5 | Edit menu Undo, Cut, Copy, Paste and Select All are enabled and perform the action | **passes.** All report enabled through the accessibility tree, and Select All from the menu selected the document |
| 1.6 | rust-analyzer starts with `ALPINE_RUST_ANALYZER` unset | **passes.** Re-checked on `main` `058087b`. `open -n ~/Applications/Alpine Editor.app --args` on `apps/alpine-editor/src/lib.rs`, from `/`, `ALPINE_RUST_ANALYZER` unset: process cwd is `/`, child is `~/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rust-analyzer` plus its proc-macro server. The rustup shim is not spawned |
| 1.7 | The app launches from `~/Applications/Alpine Editor.app` with an icon | **passes.** Installed, registered with `lsregister`, launches with `CFBundleIconFile` set, the correct menu bar, and the requested file rendered and highlighted |

Phase 1 is closed on `main`. Hover and go-to-line were already wired
in #630; this close records them in the installed app after #633.

The Dock rust-analyzer miss was not LaunchServices-specific spawn
failure. `rustup which rust-analyzer` follows process CWD. Under `open`
that CWD is `/`, so rustup consults the default toolchain (`stable`),
which has no rust-analyzer, and discovery gave up. Discovery now scans
`$RUSTUP_HOME`/`~/.rustup/toolchains` and never spawns the rustup shim
or asks `rustup which`. A `rust-toolchain.toml` next to the opened
files selects that channel when it has a working server.

The recovery banner was `LocalStatus::Workspace`, which always beat
language status and never cleared. It is now `LocalStatus::Recovery`:
language status wins while present, and the first key or pointer down
dismisses it. After a Right Arrow on the same `open` launch, the banner
was gone and rust-analyzer stayed running.

Two findings worth keeping. `screencapture -l` cannot see the Metal
layer and returns a window that looks blank, so on-screen checks go
through ScreenCaptureKit as `tools/onscreen-sdr-capture` already does.
And `~/.cargo/bin/rust-analyzer` is normally a link to `rustup`: it
answers `--version` but does not survive being run as a long-lived
server, so discovery resolves the shim before spawning.

## Phase 2: language agnostic

Rust, Python, C++, Java and TypeScript/JavaScript through one registry. Six
extensions, five servers: TypeScript and JavaScript share one.

| # | Criterion | Evidence |
| --- | --- | --- |
| 2.1 | A file of each of the five language groups highlights within 100 ms of appearing, with no language server running | **passes.** Installed `~/Applications/Alpine Editor.app` at `a4d2a25`. Isolated `HOME`, `PATH` stripped of language servers, `ALPINE_*` unset. One fixture per group: Python, Rust, Java, C++, TypeScript, JavaScript. Keywords, numbers, and comments colored; status `MissingServer`; no `rust-analyzer`/`clangd`/`pylsp`/`jdtls`/`typescript-language-server` process. Folder open of `/tmp/alpine-phase2-ra` showed the tree and an Untitled scratch with no child servers. `syntax::tests::visible_lines_of_a_5000_line_file_highlight_within_100ms`: 48 visible lines, five languages, ten trials after warmup, each under 100 ms (test wall 0.09 s). No `presentedTime` |
| 2.2 | Definition, hover and references work in all five once the server is ready | **passes on installed servers; skips recorded for missing binaries.** This Mac has rust-analyzer and clangd. No `pylsp`/`basedpyright`/`jdtls`/`typescript-language-server`: Python, Java, TypeScript, and JavaScript are recorded skips, not silent passes. Rust on `/tmp/alpine-phase2-ra/src/lib.rs` after ready: F12 on `navigation_target` showed `file:///tmp/alpine-phase2-ra/src/lib.rs`; Cmd+K Cmd+I hover showed `pub fn navigation_target(value: u32) -> u32`; Opt+Shift+F12 showed two reference rows in the same file. Diagnostics underlined the mismatched `&str` in `deliberately_invalid`. C++: opening `main.cpp` spawned `/Library/Developer/CommandLineTools/usr/bin/clangd` and admitted clangd diagnostics through the same façade. Command palette titles are language-agnostic (`Navigation: Show Hover`, `Go to Definition`, `Find References`). Isolated `HOME` needs `RUSTUP_HOME`/`CARGO_HOME` (or `ALPINE_RUST_ANALYZER`) pointing at the real toolchain; 1.6 already proved Dock discovery with the env unset |
| 2.3 | Switching between two languages keeps both servers warm; a sixth evicts by idle order rather than failing | **passes for two warm; sixth eviction is unit-tested.** One process opened the fixture folder, then `main.cpp`, then `src/lib.rs`. Process list kept clangd, rust-analyzer, and the proc-macro server together; switching back to the C++ tab left clangd running. `language_services` tests: a sixth identity evicts the idle-oldest; idle TTL drops an unattached slot without hitting the cap; TypeScript and JavaScript share `server_id = typescript`. Three- and five-warm installed-app rows are skipped: this Mac has only two of the five cohort binaries |
| 2.4 | Deleting a registry entry removes that language with no code change | **passes.** Overlay `~/Library/Application Support/Alpine Editor/languages.overlay.toml` with `disabled = ["java"]` under a disposable `HOME`. Same `Main.java` that highlighted `public`/`class`/`return` as keywords with `MissingServer` painted as plaintext with no status banner and no `jdtls`. No Rust edit. Restored by discarding that `HOME` |

Lab footprint snapshots (`/usr/bin/footprint` `phys_footprint`, Alpine plus children, not a ten-trial CI). 1-warm tiny Rust crate: alpine-editor 37 MB + rust-analyzer 288 MB + proc-macro-srv 6.8 MB = 332 MB. 2-warm after also attaching clangd in the same process: alpine-editor 47 MB + clangd 22 MB + rust-analyzer 289 MB + proc-macro-srv 6.8 MB = 365 MB. rust-analyzer is GB-class on a real crate; these numbers are the fixture, not the daily-use bound.

The server pool is the memory lever. One server per workspace and language,
lazy start, idle shutdown, hard concurrency cap. Five servers at once, with
rust-analyzer alone at 1 to 4 GB, would end the footprint premise.

## Phase 3: editing parity

| # | Criterion | Evidence |
| --- | --- | --- |
| 3.1 | `cmd-shift-l` selects all matches and edits them together | |
| 3.2 | `cmd-ctrl-up` and `cmd-ctrl-down` expand and shrink by syntax node | |
| 3.3 | Vim normal, insert and visual; `hjkl`, `w/b/e`, `0/$/gg/G`; `i/a/o/O`; `d/c/y` with motions; counts; `x/p/u`; `/` search; `:w`, `:q`, `:wq` | |
| 3.4 | 500 simultaneous cursors editing a 10,000 line file stay under 16 ms per keystroke | |
| 3.5 | Undo of a multi-cursor edit restores every cursor position | |

`SelectionSet` in [crates/alpine-text/src/lib.rs](../crates/alpine-text/src/lib.rs)
already sorts, dedups and transforms selections across edits, and `Transaction`
already validates non-overlap. Multi-cursor is an app-layer change.

## Phase 4: context

| # | Criterion | Evidence |
| --- | --- | --- |
| 4.1 | Breadcrumbs show the enclosing scope and update on caret move | |
| 4.2 | Branch name visible, updating within one second of checkout | |
| 4.3 | Blame renders for the visible range only; scrolling 10,000 lines does not block input | |
| 4.4 | No layout shift when breadcrumbs, blame or diagnostics appear | |

## Phase 5: a real application

| # | Criterion | Evidence |
| --- | --- | --- |
| 5.1 | Two windows edit different projects independently; closing one leaves the other working | |
| 5.2 | Every surface matches the written design spec | |
| 5.3 | No element shifts position during normal editing | |

## Measurements

Recorded as phases close, so regressions are visible.

| Date | Phase | Metric | Value | Conditions |
| --- | --- | --- | --- | --- |
| 2026-09-14 | 2 | Visible-line highlight, 5,000-line buffer | 10/10 trials under 100 ms; test wall 0.09 s | 48 visible lines, five cohort lexers, fingerprint cache, no `presentedTime` |
| 2026-09-14 | 2 | 1-warm `phys_footprint` | 332 MB | Isolated `HOME`, tiny `/tmp/alpine-phase2-ra` crate; alpine-editor + rust-analyzer + proc-macro-srv. Lab snapshot, not ten trials |
| 2026-09-14 | 2 | 2-warm `phys_footprint` | 365 MB | Same process after opening `main.cpp` then `lib.rs`; adds clangd 22 MB. 3-warm and 5-warm skipped (binaries absent) |
