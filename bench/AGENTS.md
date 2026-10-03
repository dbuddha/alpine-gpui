---
scope: bench/, the local black-box benchmark (never shipped, never in CI)
parent: ../AGENTS.md
updated: 2026-10-02
gate: "inside bench/: cargo build, cargo test, cargo clippy --all-targets -- -D warnings, cargo fmt --check; then helpers/build.sh"
known_defects:
  - "bench-input and bench-capture have never run: they wait for the first permissioned trials"
  - "unverified until then: the reference editor's 960x540 content fix, its caret blink suppression, PID-targeted scrolls moving each app down, and CFFIXED_USER_HOME keeping saved state out of the real home"
---

# Bench

`bench/` measures Alpine Editor, Zed and an AppKit reference editor the same
way, from outside the process, so performance claims reproduce with one
command: a std-only Cargo workspace, excluded from the root, plus Swift
helpers built with `xcrun swiftc` (Command Line Tools).

## Commands

```sh
bench/helpers/build.sh          # helpers, reference editor, self-tests
cd bench && cargo build --release
target/release/bench workloads | stamp | permissions | fixtures
target/release/bench run <workload> --app alpine|zed|appkit [--trials 10]
target/release/bench baseline results/<run> --milestone M1 [--note TEXT]
target/release/bench zed-isolation [--allow-window] [--open-fixture]
```

Workloads live in `src/workload.rs`. Alpine is the installed
`~/Applications/Alpine Editor.app`, built from a clean tree.

## Owner-present session

1. Quit Zed, then run `bench zed-isolation --allow-window --open-fixture`.
   It refuses while another Zed runs, and must report no change.
2. Grant Accessibility and Screen Recording to the terminal that runs
   `bench`; `bench permissions` shows both, and helpers never prompt.
3. Run each workload, hands-off from the `Hands off` line until the run
   directory prints.

## Protocol

- Quiet machine, AC power, Low Power Mode off, clean trees. `bench baseline`
  refuses anything else and needs ten valid measured trials; it skips rows
  with fewer values, and a child absent from a trial counts as zero bytes.
- Every workload runs one untimed warm-up, then fresh processes, each with a
  disposable HOME under `/tmp/alpine-bench` (`BENCH_HOME_ROOT`) and the
  Dock's PATH. Quit the measured app first, or the bench refuses.
- Before measuring and every second of every phase, the app must be
  frontmost with no other app's visible window over it, at any layer. A
  failure before measuring stops the run; later, it invalidates the trial.
- Report 95% Student t intervals of per-trial values.

## Permissions

| Helper | Permission | Used for |
| --- | --- | --- |
| `bench-window` | none | visibility, activation, displays |
| `bench-sample` | none (same-user `proc_pid_rusage`) | footprint, CPU, wakeups |
| `bench-input` | Accessibility (post events) | `typing`, `caret`, `scroll` |
| `bench-capture` | Screen Recording | `typing`, `caret`, `scroll` |

`bench-input` posts every event to the measured app's PID
(`CGEventPostToPid`) from a private event source with no modifier flags,
never to the HID stream, so no other process can receive it. It refuses to
run without a PID.

## Metrics

- Memory: phys_footprint of the app and its tree at 1 Hz, p50, p95 and max
  per phase (below 20 samples, p95 is the max), and the largest per child
  name. `footprint_tool_end` cross-checks Apple's `footprint`.
- CPU, wakeups and energy: deltas between phase boundary samples; startup
  counts from process start. CPU and wakeups include reaped children;
  energy counts processes alive at the end.
- Keypress-to-screen: keys 250 ms apart (`--key-interval-ms`), after the
  caret moves to line 11. A key is timed to the display time of the first
  frame whose text band changed, if that comes before the next key and the
  previous key's did too; otherwise it counts as late, or missed. The band
  skips the top 30%, the bottom 40 pt and 32 pt of scroll bar.
- Frame interval while scrolling: display-time gaps between changed frames;
  `frame_late_pct` counts gaps over 1.5 refresh periods.
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
- Latency ends at ScreenCaptureKit's display time, so capture delivery is
  excluded (`frames.tsv` keeps arrival times), and PID-targeted input skips
  HID routing. Both apply to every app.
- The reference editor has no language features.

## Isolation

Trials set HOME and `CFFIXED_USER_HOME` to the disposable home; the latter
moves `NSHomeDirectory` and the user Library folders. Zed also gets
`--user-data-dir`, seeded settings, `ZED_UPDATE_EXPLANATION` and a dead
`ZED_SERVER_URL`. Only the reference editor gets `-ApplePersistenceIgnoreState
YES`: Alpine exits on extra arguments and Zed's parser is untested with them.
Preferences still reach the real home through cfprefsd, so each trial
compares the app's real preferences, saved state, HTTP storage, caches, data
and Zed.app before and after, and the run stops loudly on any change. Zed's
Dock recents (`noteNewRecentDocumentURL`) need Full Disk Access to inspect
and go unchecked. On 2026-10-02 a windowless probe, with the owner's Zed
running, migrated its databases in the disposable home, exited at the
single-instance check after 409 ms and changed nothing real.
