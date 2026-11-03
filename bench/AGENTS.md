---
scope: bench/, non-shipping benchmark
role: benchmark operating rules
author: Deepak Buddha
authority: ../project.md
related: [../AGENTS.md, ../project.md, ../notes.md]
parent: ../AGENTS.md
updated: 2026-11-03
gate: "in bench/: cargo build, test, clippy --all-targets -- -D warnings, fmt --check; then helpers/build.sh"
known_defects:
  - "unverified: the reference editor's caret blink suppression"
  - "typing/caret: hashes cannot verify text/caret; semantic timing unqualified"
---

# Bench

Compare Alpine, Zed and AppKit. Read ../project.md and ../notes.md first.

## Commands

```sh
bench/helpers/build.sh          # helpers, reference editor, self-tests
cd bench && cargo build --release
target/release/bench workloads | stamp | permissions | fixtures
target/release/bench run <workload> --app alpine|zed|appkit [--trials 12] [--wait-idle MIN] [--dry-run]
target/release/bench baseline results/<run> --milestone M1 [--note TEXT]
target/release/bench zed-isolation [--allow-window] [--open-fixture]
```

`--dry-run` starts nothing. Measure clean installed `~/Applications/Alpine Editor.app`.

## Approved screen session

Request each session with duration. Use one input-idle minute and a supervised
five-minute maximum condition wait (not yet enforced by CLI). Stop on input or
lock; do independent work while approval waits.

1. Quit Zed; `bench zed-isolation --allow-window --open-fixture` must report
   no change.
2. The runner app needs Accessibility and Screen Recording (`bench
   permissions`); helpers never prompt.
3. Stay hands-off until results print.

## Protocol

- Quiet machine, AC power, Low Power Mode off, clean trees. `bench baseline`
  needs ten valid measured trials; prospectively schedule twelve, one warm-up,
  with no added trials after results. Retain invalid/adverse outcomes.
  Typing/caret require semantic and raw latency evidence in every measured
  trial; hash-only cannot qualify. Short diagnostic rows are skipped; absent
  children count as zero bytes.
- Every workload uses one warm-up, then fresh processes with a
  disposable HOME under `/tmp/alpine-bench` (`BENCH_HOME_ROOT`) and the
  Dock's PATH. Quit the measured app first, or the bench refuses.
- `highlight` is Alpine-only: each round opens 20,000 lines
  per language with an empty PATH; children void the trial. `--trials` counts
  per group; other rows pool them.
- Every second, including before measurement, the app must be frontmost and
  uncovered by other apps' visible windows at any layer but the pointer's; a
  failure stops the run before measuring (except what `--wait-idle` voids) and
  invalidates the trial after.
- `--wait-idle MIN`: each trial waits for MIN idle minutes (per-type input
  timers), AC power and an unlocked screen; owner input, a lock or an
  unreadable clock voids it (not replaced after seeing results). The display
  stays awake once the gate opens.
- Report 95% Student t intervals across trials.

## Permissions and input

bench-window (visibility) and bench-sample (same-user resource sampling) need
no permission. bench-input needs Accessibility; bench-capture needs Screen
Recording. Helpers never prompt. Input uses a private source and measured PID
only, never HID; only explicit chords carry modifiers. Scroll/mouse events
carry PID, window and window-local coordinates, verified by readback. Missing
exports, PID or mismatched windows fail closed.

## Metrics

- Memory: phys_footprint of the app and its tree at 1 Hz, p50, p95, max per
  phase, and the largest per child name. `footprint_tool_end` cross-checks
  Apple's `footprint`.
- CPU, wakeups and energy: deltas between phase boundary samples; startup
  counts from process start. CPU and wakeups include reaped children;
  energy counts processes alive at the end.
- Keys are 250 ms apart (`--key-interval-ms`) from line 11. Hash timing is
  diagnostic: `key_hash_change_*` never claims keypress-to-screen.
  `keys_late`, `keys_missed` and `keys_ambiguous` count late, absent and
  uncertain changes; an unresolved interval stops later matching. Missing or
  unstable initial frames or bad timestamps leave timing incomplete, which
  omits hash percentiles and summary timing; raw frames and events remain.
- `key_semantic_qualified` stays zero until intended text and caret are
  verified. No-input trials fail. Sparse changes stay measured adverse
  outcomes (`key_band_distinct`, `key_band_states_sufficient`, a warning) at
  the threshold `min(keys, max(10, keys/2))`; fast trials cannot replace
  them.
- Frame interval while scrolling: display-time gaps between changed frames;
  `frame_late_pct` counts gaps over 1.5 refresh periods. A scroll posting
  nothing, or leaving under `min(events, max(20, events/50))` distinct window
  states, is invalid.
- `highlight_first_frame_ms` (the 100 ms budget): `build_end_ns` minus
  `build_begin_ns` on the first row of Alpine's `perf-<s>-frames.tsv` with
  both; it must be the cold startup build (event 0), else the trial is refused.
- Raw TSV: `results/<run>/`; `bench baseline` appends stamped rows
  to `baseline.tsv`.

## Current limitations and required qualification

Only a limited controlled profile exists. Everyday profiles, semantic typing,
ready-to-edit and equivalent workspace load remain unqualified. Follow
project.md for final comparison requirements; historical rows do not prove them.

Matched: fixtures outside git, 960x572 pt windows (over 1 pt off invalidates),
Menlo 15 pt with 22 pt lines, steady caret, environment, rust-analyzer and its
`CARGO_TARGET_DIR`, capture path. Not matched:

- Zed runs non-default settings: no updates, telemetry, AI, sign-in server,
  completion popups or git decorations; trusted worktrees; `cursor_blink` off.
- A fresh Zed profile runs database migrations at every launch.
- `launch_to_window` is the first window, loaded or not; the reference
  editor reads its file after showing it.
- Chrome differs: tab, status, title bars.
- `open-repo`: Zed gets the folder and the file, the others the file.
- For every app, latency ends at ScreenCaptureKit's display time, without
  capture delivery (`frames.tsv` keeps arrival times), and PID-targeted input
  skips HID routing.
- The reference editor has no language features.

## Isolation

Trials set HOME and `CFFIXED_USER_HOME` to disposable data. Zed also gets
`--user-data-dir`, seeded settings, `ZED_UPDATE_EXPLANATION` and a dead
`ZED_SERVER_URL`; only AppKit gets `-ApplePersistenceIgnoreState YES`.
cfprefsd can write real preferences: compare real state before/after and stop
on change (`apps.rs`). Zed Dock recents need Full Disk Access and go unchecked.
