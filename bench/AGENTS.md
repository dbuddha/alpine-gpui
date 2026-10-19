---
scope: bench/, the local black-box benchmark (never shipped, never in CI)
parent: ../AGENTS.md
updated: 2026-10-17
gate: "in bench/: cargo build, test, clippy --all-targets -- -D warnings, fmt --check; then helpers/build.sh"
known_defects:
  - "unverified: the reference editor's caret blink suppression"
  - "typing/caret: hashes cannot verify text/caret; semantic timing unqualified"
---

# Bench

`bench/` compares Alpine, Zed and AppKit: a std-only Cargo workspace plus
Swift helpers (`xcrun swiftc`).

## Commands

```sh
bench/helpers/build.sh          # helpers, reference editor, self-tests
cd bench && cargo build --release
target/release/bench workloads | stamp | permissions | fixtures
target/release/bench run <workload> --app alpine|zed|appkit [--trials 10] [--wait-idle MIN] [--dry-run]
target/release/bench baseline results/<run> --milestone M1 [--note TEXT]
target/release/bench zed-isolation [--allow-window] [--open-fixture]
```

`src/workload.rs` defines workloads; `--dry-run` starts nothing. Alpine is
the clean installed `~/Applications/Alpine Editor.app`.

## Owner-present session

1. Quit Zed; `bench zed-isolation --allow-window --open-fixture` must report
   no change.
2. The runner app needs Accessibility and Screen Recording (`bench
   permissions`); helpers never prompt.
3. Stay hands-off from `Hands off` until the results directory prints.

## Protocol

- Quiet machine, AC power, Low Power Mode off, clean trees. `bench baseline`
  needs ten valid measured trials. Typing/caret require complete, semantically
  qualified evidence and latency rows in every measured trial, including raw
  metrics; legacy or hash-only runs cannot qualify. Other short diagnostic
  rows are skipped; an absent child counts as zero bytes.
- Every workload uses one warm-up, then fresh processes with a
  disposable HOME under `/tmp/alpine-bench` (`BENCH_HOME_ROOT`) and the
  Dock's PATH. Quit the measured app first, or the bench refuses.
- `highlight` (criterion 2.1) is Alpine-only: each round opens 20,000 lines
  per language with an empty PATH; children void the trial. `--trials` counts
  per group; other rows pool them.
- Every second, including before measurement, the app must be frontmost and
  uncovered by other apps' visible windows at any layer but the pointer's; a
  failure stops the run before measuring (except what `--wait-idle` voids) and
  invalidates the trial after.
- `--wait-idle MIN`: each trial waits for MIN idle minutes (per-type input
  timers), AC power and an unlocked screen; owner input, a lock or an
  unreadable clock voids it (not replaced: pass spare `--trials`). The display
  stays awake once the gate opens.
- Report 95% Student t intervals across trials.

## Permissions

- `bench-window`: none; visibility, activation, displays.
- `bench-sample`: none (same-user `proc_pid_rusage`); footprint, CPU, wakeups.
- `bench-input`: Accessibility (post events); `typing`, `caret`, `scroll`,
  `highlight`.
- `bench-capture`: Screen Recording; `typing`, `caret`, `scroll`.

`bench-input` posts only to the measured app's PID (`CGEventPostToPid`),
from a private event source, never to the HID stream, and refuses to run
without one. Events carry no modifier flags but a `chord`'s named ones.
Scrolls and mouse events carry the target pid, window number and a
window-local point (`CGEventSetWindowLocation`) for AppKit routing; the helper
reads them back and refuses another pid's window or a missing export.

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

## Comparable and not

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

Trials set HOME and `CFFIXED_USER_HOME` (which moves `NSHomeDirectory` and
user Library folders) to the disposable home. Zed also gets
`--user-data-dir`, seeded settings, `ZED_UPDATE_EXPLANATION` and a dead
`ZED_SERVER_URL`. Only the reference editor gets `-ApplePersistenceIgnoreState
YES` (why: `apps.rs`). cfprefsd still writes preferences to the real home, so
each trial compares the app's real state (`apps.rs`) before and after
and stops on any change. Zed's Dock recents (`noteNewRecentDocumentURL`)
need Full Disk Access to inspect and go unchecked.
