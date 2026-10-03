---
scope: bench/, the local black-box benchmark (never shipped, never in CI)
parent: ../AGENTS.md
updated: 2026-10-03
gate: "inside bench/: cargo build, cargo test, cargo clippy --all-targets -- -D warnings, cargo fmt --check; then helpers/build.sh"
known_defects:
  - "bench-input and bench-capture have never run: they wait for the first permissioned trials"
  - "unverified until then: the reference editor's 960x540 content fix, its caret blink suppression, PID-targeted scrolls moving each app down, highlight's palette keys, and CFFIXED_USER_HOME keeping saved state out of the real home"
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

1. Quit Zed, then run `bench zed-isolation --allow-window --open-fixture`;
   it refuses while another Zed runs and must report no change.
2. Grant Accessibility and Screen Recording to the terminal running `bench`;
   `bench permissions` shows both, and helpers never prompt.
3. Stay hands-off from each run's `Hands off` line until its directory
   prints.

## Protocol

- Quiet machine, AC power, Low Power Mode off, clean trees. `bench baseline`
  refuses anything else and needs ten valid measured trials; it skips rows
  with fewer values, and a child absent from a trial counts as zero bytes.
- Every workload runs one untimed warm-up, then fresh processes, each with a
  disposable HOME under `/tmp/alpine-bench` (`BENCH_HOME_ROOT`) and the
  Dock's PATH. Quit the measured app first, or the bench refuses.
- `highlight` (criterion 2.1) is an Alpine-only budget row, not a comparison.
  Each round opens a 20,000-line file per language group with an empty PATH
  directory; a child process voids the trial. `--trials` counts per group;
  other rows pool the groups.
- From before measuring, every second, the app must be frontmost and
  uncovered by other apps' visible windows at any layer; a failure stops the
  run before measuring and invalidates the trial after.
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

## Metrics

- Memory: phys_footprint of the app and its tree at 1 Hz, p50, p95 and max
  per phase (below 20 samples, p95 is the max), and the largest per child
  name. `footprint_tool_end` cross-checks Apple's `footprint`.
- CPU, wakeups and energy: deltas between phase boundary samples; startup
  counts from process start. CPU and wakeups include reaped children;
  energy counts processes alive at the end.
- Keypress-to-screen: keys 250 ms apart (`--key-interval-ms`) from line 11,
  each timed to the display time of the first frame whose text band changed,
  if that precedes the next key and the previous key's did too; else it is
  late, or missed. The band skips the top 30%, the bottom 40 pt and 32 pt of
  scroll bar.
- Frame interval while scrolling: display-time gaps between changed frames;
  `frame_late_pct` counts gaps over 1.5 refresh periods.
- `highlight_first_frame_ms` (the 100 ms budget): `build_end_ns` minus
  `build_begin_ns` on the first row of Alpine's `perf-<s>-frames.tsv` with
  both, which must be the startup build (event 0, empty `event`) that lexes
  and lays out the first visible range cold; an event's row is warm, so the
  trial is refused.
- Output: raw TSV in `results/<run>/`; `bench baseline` appends stamped
  milestone rows to the tracked `baseline.tsv`.

## Comparable and not

Matched: fixture bytes (outside any git checkout), the requested window frame
(960x572 pt, recorded per trial), Menlo 15 pt on 22 pt lines, a steady caret,
environment, the rust-analyzer binary and its `CARGO_TARGET_DIR` under
`bench/target`, and the capture path. Not matched:

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
each trial compares the app's real state (preferences, saved state, HTTP
storage, caches, data, Zed.app) before and after and stops loudly on any
change. Zed's Dock recents (`noteNewRecentDocumentURL`) need Full Disk Access
to inspect and go unchecked.
