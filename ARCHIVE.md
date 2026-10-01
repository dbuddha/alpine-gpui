# Archive

History and overflow, newest first. Nothing here is an operating rule; the
AGENTS.md files are. Read this file to learn why something is the way it is.
Older material: tag `pre-cleanup-2026-09`, and the last `docs/` tree before the
2026-09-30 reset at commit `6e6282b` (`git show 6e6282b:docs/<path>`).

## 2026-09-30: reset after an adversarial review

Decisions (Deepak):
- Editor first: beat Zed at parity in core editing, navigation and search,
  language intelligence and git. A separate terminal app comes later.
- Absolute budgets gate PRs; "beats Zed" is measured at milestone close.
- Docs reduced to AGENTS.md files, ARCHIVE.md and README.md. AGENTS.md
  overrides vault notes.
- alpine-zed-lab archived. Comparisons move to `bench/` in this repo.
- Language servers are discovered, never downloaded. Intellisense is quiet by
  default. Git goes through the git CLI with long-lived processes.
- No bench Mac; performance is measured on the dev MacBook; the repo stays
  public.
- Agents may push branches, open PRs and merge in this repo, except the
  `ask_first` list in AGENTS.md (dependencies, destructive actions, milestone
  designs, new subsystems or APIs, unsafe and licensing changes, weaker CI,
  spending).

Findings on `main` at `6e6282b`:
- No contract number was measured. Hosted runners expose "Apple Paravirtual
  device", which the production Metal initializer rejects, and no display.
- #304 (2026-09-02) measured about 49 ms p50 keypress to actual presentation,
  with CPU stages under 1 ms. The target-presentation gap was a fixed
  33,333,250 ns. Presentation is configured with `preferredFrameLatency(2.0)`,
  3 drawables and no `preferredFrameRateRange` (`native.rs:4346-4376`).
- didChange sends the whole document each sync (`lsp_language.rs:122-143`,
  `rust_diagnostics.rs:2598`).
- Server eviction and idle shutdown run on the main thread and join the
  supervisor for up to 5 s (`language_services.rs:332-366`,
  `lsp_process.rs:33`). A sixth language with five attached slots kills an
  open tab's server, untested.
- `"applyEdit": true` is advertised (`lsp_language.rs:448`) while
  server-initiated requests get MethodNotFound (`lsp_json.rs:600`).
- The 2.1 timing test measured warm cache hits (`syntax.rs:1188-1229`).
- 14 of 17 `test-*.sh` scripts tested other scripts. 13 of 14
  `harness = false` targets compiled to an empty `main` in the quality job.
  `classify-ci.sh:112` never selected native validation for
  `alpine-text-layout`.
- `build-alpine-editor-app.sh:236` leaked one staging directory per build.
  Dirty-tree bundles were stamped with a clean revision through
  `ALPINE_BUNDLE_FIXTURE_REVISION`.
- The GitHub Project held 490 items; #576 reached 157 comments without a merge.
- The retired registry also enforced that every `#[kani::proof]` harness was
  registered. Kani no longer runs, so the `proofs.rs` harnesses have no runner.
- Suspected, from reading only: when `windowWillClose` arrives with work in
  flight, the drain callback (`alpine-platform-macos/src/native.rs` near 3070)
  pauses the display link without `invalidate()`. `NativeSurface::drop` then
  skips invalidation but the validation probe still records one.

Blockers found on the parked `feat/lsp-manager` commit `6600f40`:
- F1. The pylsp hash pin never applied: the requirement line lacks a `\`
  continuation (`lsp_provision.rs:787-791`), the `--require-hashes` failure is
  ignored (`:806`), and the fallback pulls 17 unpinned PyPI packages
  (`:809-820`).
- F2. `npm install` ran without a lockfile and its result was ignored (`:739`).
- F3. `typescript@7.0.2` likely ships no tsserver; re-capture InvalidEnvelope.
- F4. Compiled recipe URLs fail `check-product-boundary.sh:115-120`.
- F5. curl had no timeouts, no https-only protocol limit and honored
  `~/.curlrc`.
- F6. Install Server fetched even when a server was found
  (`language_services.rs:503`).
- F7. The 512 MiB cache cap was not enforced: `dir_size` stops after 256
  entries (`lsp_provision.rs:39`, `:1079-1104`).
- F8. Eviction joins the supervisor for up to 5 s on the main thread
  (`lsp_process.rs:33`, `:788-806`), reachable from scroll. Also on main.
- F9. A sixth language with five attached slots kills the least recently used
  open tab's server (`language_services.rs:348-352`, `:544-556`). Untested.
  Also on main.
- F10. The framework crate gained an editor "Language" menu and public
  MenuAction variants; SHA-256 was tested only on "" and "abc".
- Worth salvaging: the `Cow<str>` JSON fix (pylsp escapes `/` as `\/`), the
  scroll skip of language sync, and the pool tests.

## 2026-09-14 to 2026-09-20: phase 2 on main

- 2.1: five groups highlighted with no server in the installed app at `a4d2a25`
  (isolated HOME, servers stripped from PATH).
- 2.2: Rust passed definition, hover and references; C++ spawned the CLT clangd
  and admitted diagnostics. Python, Java, TypeScript and JavaScript were
  recorded skips (no server on that Mac).
- 2.3: clangd and rust-analyzer stayed warm together; the sixth-identity
  eviction was unit-tested.
- 2.4: `languages.overlay.toml` with `disabled = ["java"]` rendered `.java` as
  plain text with no code change.
- Footprint snapshots, alpine-editor plus children (phys_footprint, not ten
  trials):
  - 1 warm server: 332 MB (editor 37, rust-analyzer 288, proc-macro 6.8)
  - 2 warm servers: 365 MB (adds clangd 22)
  - pylsp tab on the dirty build, 2026-09-20: editor 32 MB (peak 146 MB),
    pylsp 21 MB

## 2026-09-13: phase 1 closed on main (`9f54f95`, installed app at `058087b`)

1.1 `cmd-o` and File > Open opened a file and a folder. 1.2 One click opened a
tree row. 1.3 Folder open showed the tree and an empty buffer. 1.4 Keybindings
matched pinned Zed (`f12`, `f2`, `cmd-shift-i`, `alt-shift-f12`,
`cmd-k cmd-i`, `ctrl-g`, `cmd-shift-o`, `cmd-shift-e`, `cmd-o`,
`cmd-shift-s`, `cmd-s`). 1.5 Edit menu actions were enabled and worked. 1.6
rust-analyzer started with `ALPINE_RUST_ANALYZER` unset: discovery scans
toolchains instead of asking rustup, which follows the process cwd (`/` under
`open`). 1.7 The app launched from `~/Applications` with its icon.

## 2026-09-12/13: the assurance regime retired

The capability, requirement and task hierarchy, the evidence registry, Kani,
Miri, mutation and TLA+ gates, and the skill programme were retired. The
rename to Alpine Editor (#620) also deleted the dogfood and profile measurement
tooling (about 4,000 lines), because its evidence was hash-bound to the old
name.

Pointers:
- old docs tree: `da69bd30`
- Wiki snapshot: `aeab9e09` in the `alpine-gpui.wiki` repository, built from
  `93df44b`
- last TLA+: `a52fc06`
- recovery archive: local `alpine-recovery/20260912T053747Z` on the dev Mac
- deferred at that time: `3ee0ed9`, PR 585, `429fc65`

## Withdrawn or corrected claims

- "Pinned GPUI is about 12% faster at renderer-submit-readback." The lab
  calibration (2026-09-01) was `statistics_qualified=false` and
  `performance_qualified=false`, ran entirely on battery, at Alpine `2fdf5aa`.
  Not a result.
- "presentedTime is always zero." #304 recorded actual-presentation samples
  on 2026-09-02. Later Swift and MetalKit controls with no Alpine code read
  zero. It is unreliable on the dev Mac, not always zero.
- An early footprint comparison measured an idle shell against an IDE doing
  work.

## Retired budgets (capability probe, never measured)

Quoted from `docs/alpine-capability-probe.md` at `6e6282b`:
"Pane switching / idle CPU | <=50 ms / <1% of one core";
"One terminal-like replay | <=200 MiB";
"Combined probe | <=512 MiB steady; <=768 MiB peak";
"eight terminal sessions with four visible, ten documents with 20 MiB total
source, a virtualized grid and a dock update stream. Bound scrollback to 8
MiB/session, shared caches and undo to 64 MiB each, and result data to 32 MiB."
Input for the terminal app's budgets.

## Design decisions

- 2026-08: the terminal was deferred (PTY, shell integration, cancellation and
  escape-sequence surface).
- 2026-08: zero idle submissions became mandatory after a reported Apple
  Silicon continuous-redraw regression in another GPUI port.
- 2026-08-27: #371 saw zero presented-handler samples, so presentation
  telemetry may not own progress, and the drop retry was removed.
- 2026-08-21/22, text hot path: #293 orientation, #295 lookup before
  rasterizing, #298 index, #300 and #301 row deltas.
- 2026-08-17: the `ignore` crate grew the stripped release binary from 907,400
  to 1,844,360 bytes.
- 2026-08-16: #135 and #136 replaced the in-callback GPU wait with three
  completion-owned frame slots.
- 2026-08-15, text: Ropey 1.6.1 and unicode-segmentation 1.13.3. Crop 0.4.3
  failed the nested-slice and UTF-16 surrogate corpus.
- 2026-08-14, colour: BGRA8Unorm_sRGB, standard sRGB layer colour space, EDR
  off.
- 2026-08-14, pacing: layer-bound CAMetalDisplayLink, not Zed's per-display
  CVDisplayLink. Kept the drawable timeout and framebuffer-only; rejected a
  permanent animation loop and indefinite drawable waits.

## Research conclusions

- Apple GPU families: M1 Apple7, M2 Apple8, M3 and M4 Apple9, M5 Apple10.
- #521 on the dev Mac: host wait 336 µs p50 against GPU 35 µs. The shader is
  not the first target.
- Zed GPUI study: a `waitUntilCompleted` in the frame path caused jank; the GPU
  may run past the deadline; hash-only reuse is not accepted.
- Lineage audit (2026-08-27): of 24 mechanism families, 8 adapted from GPUI, 6
  convergent, 4 Alpine-original, 6 rejected or deferred; no copied code. Report
  120 Hz deadline adherence, never FPS.
- Accessibility (2026-08-20): one semantic model with a pull transport.
  AccessKit was rejected (no macOS tests at `2dfdd7b`). Hosted counters prove
  intent, not delivery.
- Idle energy (2026-08-20): Alpine counters and OS counters are separate
  authorities. Energy Impact is not portable; platform-idle wakeups are the key
  subset (`TASK_POWER_INFO_V2`, `powermetrics`).
- WGPU v30 (2026-08-18): not a shipping dependency; useful as a test taxonomy
  or a differential oracle.

## AEP index (files removed at the 2026-09-30 reset)

| AEP | Decision | Still true? | Code |
| --- | --- | --- | --- |
| 0009 | Evidence registry | Retired | removed |
| 0016 | Value constructors return Option | Yes | `alpine-core` |
| 0025 | Synchronous offscreen render and CPU oracle | Yes | `alpine-metal` |
| 0028 | Zed golden workloads | Tooling only | `alpine-trace` |
| 0064 | Native presentation | Yes; callback wait superseded | `alpine-platform-macos/src/native.rs` |
| 0120 | Three frame slots, 8 MiB upload each | Yes | `alpine-metal` |
| 0137 | Bounded runtime | Yes | `alpine-runtime` |
| 0139 | Text buffer, atomic save | Yes | `alpine-text` |
| 0141 | Layout cache 32 MiB, A8 atlas 16 MiB | Yes | `alpine-text-layout` |
| 0153 | Clipboard at most 64 MiB, synchronous close veto | Yes | `alpine-platform-macos` |
| 0160 | Eager folder enumeration | Superseded by 0171 | none |
| 0165 | In-file find | Yes | `find.rs` |
| 0168 | Quick open | Yes | `quick_open.rs` |
| 0171 | Lazy file tree | Yes | `file_tree.rs` |
| 0177 | Static palette, at most 32 commands | Yes | `commands.rs` |
| 0180 | Streaming project search | Yes | `project_search.rs` |
| 0218 | Completion | Yes | `rust_completion.rs` |
| 0221 | Symbols | Yes | `rust_symbols.rs` |
| 0222 | Settings reload | Yes | `settings/loader.rs` |
| 0250 | Accessibility transport, 271 nodes | Yes | `accessibility.rs` |
| 0255 | Line and grapheme mapping | Yes | `alpine-text` |
| 0268 | Input epoch | Yes | `alpine-platform-macos` |
| 0270 | Accessibility activate and bounds | Yes | `accessibility.rs` |
| 0271 | Accessibility notifications | Yes | `native_accessibility.rs` |
| 0272 | Native accessibility process journey | Test only | `native_validation` |
| 0273 | Physical accessibility validator | Never run | `tools/alpine-ax-client` |
