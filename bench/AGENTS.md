---
scope: bench/, the local black-box benchmark (never shipped, never in CI)
parent: ../AGENTS.md
updated: 2026-10-03
gate: "inside bench/: cargo build, cargo test, cargo clippy --all-targets -- -D warnings, cargo fmt --check; then helpers/build.sh"
known_defects:
  - "Zed shows none of 100 typed keys in the band, at 960x572 too (run 20261004T044624Z); cause open"
  - "highlight: a launch event replaces the pending startup frame before it draws, so no event-0 row exists (run 20261004T045057Z)"
  - "unverified: the reference editor's caret blink suppression"
---

# Bench

`bench/` measures Alpine Editor, Zed and an AppKit reference editor the same
way, from outside: a std-only Cargo workspace outside the root one, plus
Swift helpers built with `xcrun swiftc`.

## Commands

```sh
bench/helpers/build.sh          # helpers, reference editor, self-tests
cd bench && cargo build --release
target/release/bench workloads | stamp | permissions | fixtures
target/release/bench run <workload> --app alpine|zed|appkit [--trials 10] [--dry-run]
target/release/bench baseline results/<run> --milestone M1 [--note TEXT]
target/release/bench zed-isolation [--allow-window] [--open-fixture]
```

Workloads live in `src/workload.rs`; `--dry-run` prints a run's launches and
input, starting nothing. Alpine is the installed
`~/Applications/Alpine Editor.app`, built clean.

## Owner-present session

1. Quit Zed, then run `bench zed-isolation --allow-window --open-fixture`,
   which must report no change.
2. Grant Accessibility and Screen Recording to the terminal running `bench`;
   `bench permissions` shows both; helpers never prompt.
3. Stay hands-off from each run's `Hands off` line until its directory
   prints.

## Protocol

- Quiet machine, AC power, Low Power Mode off, clean trees. `bench baseline`
  refuses anything else and needs ten valid measured trials; it skips rows
  with fewer values; a child absent from a trial counts as zero bytes.
- Every workload runs one untimed warm-up, then fresh processes, each with a
  disposable HOME under `/tmp/alpine-bench` (`BENCH_HOME_ROOT`) and the
  Dock's PATH. Quit the measured app first, or the bench refuses.
- `highlight` (criterion 2.1) is an Alpine-only budget row, not a comparison.
  Each round opens a 20,000-line file per language group with an empty PATH
  directory; a child process voids the trial. `--trials` counts per group;
  other rows pool the groups.
- From before measuring, every second, the app must be frontmost and
  uncovered by other apps' visible windows at any layer but the pointer's; a
  failure stops the run before measuring and invalidates the trial after.
- Report 95% Student t intervals of per-trial values.

## Permissions

- `bench-window`: none; visibility, activation, displays.
- `bench-sample`: none (same-user `proc_pid_rusage`); footprint, CPU, wakeups.
- `bench-input`: Accessibility (post events); `typing`, `caret`, `scroll`,
  `highlight`.
- `bench-capture`: Screen Recording; `typing`, `caret`, `scroll`.

`bench-input` posts only to the measured app's PID (`CGEventPostToPid`),
from a private event source, never to the HID stream, and refuses to run
without a PID. Events carry no modifier flags but a `chord`'s named ones.
Scrolls and mouse events carry the target pid, window number (private field
51) and a window-local point (`CGEventSetWindowLocation`) so AppKit can route
them; the helper reads them back and refuses another pid's window or a missing
export.

## Metrics

- Memory: phys_footprint of the app and its tree at 1 Hz, p50, p95 and max
  per phase (below 20 samples, p95 is the max), and the largest per child
  name. `footprint_tool_end` cross-checks Apple's `footprint`.
- CPU, wakeups and energy: deltas between phase boundary samples; startup
  counts from process start. CPU and wakeups include reaped children;
  energy counts processes alive at the end.
- Keypress-to-screen: keys 250 ms apart (`--key-interval-ms`) from line 11,
  each timed to the first changed frame of the text band, else late or missed
  (rules in `analysis.rs`).
- Frame interval while scrolling: display-time gaps between changed frames;
  `frame_late_pct` counts gaps over 1.5 refresh periods. A scroll posting
  nothing, or leaving under `min(events, max(20, events/50))` distinct window
  states, is invalid.
- `highlight_first_frame_ms` (the 100 ms budget): `build_end_ns` minus
  `build_begin_ns` on the first row of Alpine's `perf-<s>-frames.tsv` with
  both; it must be the cold startup build (event 0), else the trial is refused.
- Output: raw TSV in `results/<run>/`; `bench baseline` appends stamped
  milestone rows to the tracked `baseline.tsv`.

## Comparable and not

Matched: fixture bytes (outside any git checkout), the requested window frame
(960x572 pt, read after the last phase; over 1 pt off invalidates), Menlo 15 pt
on 22 pt lines, a steady caret, environment, the rust-analyzer binary and its
`CARGO_TARGET_DIR` under `bench/target`, and the capture path. Not matched:

- Zed runs non-default settings: no updates, telemetry, AI, sign-in server,
  completion popups or git decorations; `cursor_blink` off; Menlo.
- A fresh Zed profile runs its database migrations at every launch.
- `launch_to_window` is the first window, loaded or not; the reference
  editor reads its file only after showing it.
- Chrome differs: tab, status and title bars.
- `open-repo` gives Alpine and the reference editor the file; Zed gets the
  folder and the file.
- For every app, latency ends at ScreenCaptureKit's display time, without
  capture delivery (`frames.tsv` keeps arrival times), and PID-targeted input
  skips HID routing.
- The reference editor has no language features.

## Isolation

Trials set HOME and `CFFIXED_USER_HOME` to the disposable home; the latter
moves `NSHomeDirectory` and the user Library folders. Zed also gets
`--user-data-dir`, seeded settings, `ZED_UPDATE_EXPLANATION` and a dead
`ZED_SERVER_URL`. Only the reference editor gets `-ApplePersistenceIgnoreState
YES` (why: `apps.rs`). cfprefsd still writes preferences to the real home, so
each trial compares the app's real state (`apps.rs` lists it) before and after
and stops loudly on any change. Zed's Dock recents (`noteNewRecentDocumentURL`)
need Full Disk Access to inspect and go unchecked.
