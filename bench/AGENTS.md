---
scope: bench/, the local black-box benchmark (never shipped, never in CI)
parent: ../AGENTS.md
updated: 2026-10-08
gate: "in bench/: cargo build, test, clippy --all-targets -- -D warnings, fmt --check; then helpers/build.sh"
known_defects:
  - "unverified: the reference editor's caret blink suppression"
  - "typing/caret: hashes cannot verify text/caret; semantic timing unqualified"
---

# Bench

`bench/` compares Alpine, Zed and AppKit using a separate std-only Cargo
workspace and Swift helpers (`xcrun swiftc`).

## Commands

```sh
bench/helpers/build.sh          # helpers, reference editor, self-tests
cd bench && cargo build --release
target/release/bench workloads | stamp | permissions | fixtures
target/release/bench run <workload> --app alpine|zed|appkit [--trials 10] [--dry-run]
target/release/bench baseline results/<run> --milestone M1 [--note TEXT]
target/release/bench zed-isolation [--allow-window] [--open-fixture]
```

`src/workload.rs` defines workloads; `--dry-run` starts nothing. Alpine is
the clean installed `~/Applications/Alpine Editor.app`.

## Owner-present session

1. Quit Zed, then run `bench zed-isolation --allow-window --open-fixture`,
   which must report no change.
2. Grant Accessibility and Screen Recording to the runner app;
   `bench permissions` shows both; helpers never prompt.
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
- `highlight` (criterion 2.1) is Alpine-only.
  Each round opens 20,000 lines per language with an empty PATH; children
  void the trial. `--trials` counts per group;
  other rows pool the groups.
- Every second, including before measurement, the app must be frontmost and
  uncovered by other apps' visible windows at any layer but the pointer's; a
  failure stops the run before measuring and invalidates the trial after.
- Report 95% Student t intervals across trials.

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
- Keys are 250 ms apart (`--key-interval-ms`) from line 11. Hash timing is
  diagnostic: `key_hash_change_*` never claims keypress-to-screen.
  `keys_late` counts first changes beyond an input's interval, `keys_missed`
  counts absent later changes, and `keys_ambiguous` counts uncertain matches.
  An unresolved interval prevents later matching. Missing/unstable initial
  frames or bad timestamps prevent complete timing. Incomplete trials omit
  hash percentiles/max; an incomplete trial also suppresses summary timing.
  Raw frames/events remain.
- `key_semantic_qualified` is always zero until intended text/caret is
  verified. No-input trials fail. Sparse changes remain measured adverse
  outcomes with `key_band_distinct` and `key_band_states_sufficient`, plus a
  warning; fast trials cannot replace them. The threshold remains
  `min(keys, max(10, keys/2))`. Historical artifacts stay unchanged.
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
  completion popups or git decorations; trusted worktrees; `cursor_blink` off;
  Menlo.
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
moves `NSHomeDirectory` and user Library folders. Zed also gets
`--user-data-dir`, seeded settings, `ZED_UPDATE_EXPLANATION` and a dead
`ZED_SERVER_URL`. Only the reference editor gets `-ApplePersistenceIgnoreState
YES` (why: `apps.rs`). cfprefsd still writes preferences to the real home, so
each trial compares the app's real state (`apps.rs`) before and after
and stops loudly on any change. Zed's Dock recents (`noteNewRecentDocumentURL`)
need Full Disk Access to inspect and go unchecked.
