---
scope: bench/, the local black-box benchmark (never shipped, never in CI)
parent: ../AGENTS.md
updated: 2026-10-02
gate: "inside bench/: cargo build, cargo test, cargo clippy --all-targets -- -D warnings, cargo fmt --check; then helpers/build.sh"
known_defects:
  - "bench-input and bench-capture have never run: they wait for the first permissioned trials"
  - "unverified until then: the reference editor's 960x540 content fix (it measured 944x562 before), its caret blink suppression, and that PID-targeted scrolls (negative pixels) move each app down"
---

# Bench

`bench/` measures Alpine Editor, Zed and an AppKit reference editor the same
way, from outside the process, so every performance claim reproduces with one
command on the dev Mac. It is its own Cargo workspace (std only, excluded from
the root) plus Swift helpers built with `xcrun swiftc` from Command Line Tools.

## Commands

```sh
bench/helpers/build.sh          # helpers, reference editor, self-tests
cd bench && cargo build --release
target/release/bench workloads | stamp | permissions | fixtures
target/release/bench run <workload> --app alpine|zed|appkit [--trials 10]
target/release/bench baseline results/<run> --milestone M1 [--note TEXT]
target/release/bench zed-isolation [--allow-window] [--open-fixture]
```

Workloads live in `src/workload.rs`: `typing`, `caret`, `scroll`,
`open-50mb`, `open-repo` and `idle`. Alpine is the installed
`~/Applications/Alpine Editor.app`, built from a clean tree.

## Protocol

- Quiet machine: close other apps, no builds, downloads, indexing or calls.
  AC power with Low Power Mode off; `bench baseline` refuses anything else.
- Ten trials, each a fresh process with a disposable HOME under
  `/tmp/alpine-bench` (`BENCH_HOME_ROOT`; Zed's crash-handler socket caps its
  length) and the Dock's PATH. Alpine and the reference editor run their
  binaries directly; Zed runs `Contents/MacOS/zed`.
- Hands-off window: from the `Hands off` line until the run directory prints,
  touch nothing. Before measuring, the app must be frontmost with its window
  uncovered, or the trial is refused. Each phase ends with the same check; a
  failure marks the trial invalid and the summary drops it.
- `open-repo` runs one untimed warm-up trial so rust-analyzer's build
  scripts and proc macros are built; both editors get the same binary.
- Quit Zed before Zed runs. Its single-instance check makes a second Zed
  exit, and a Dock click during a run would reach the bench copy.
- Report 95% Student t intervals of per-trial values; never compare a run
  with fewer than ten valid trials.

## Permissions

Grant both to the terminal app that runs `bench`; macOS attributes the
helpers to it. Helpers only preflight and never prompt; `bench permissions`
prints the state.

| Helper | Permission | Used for |
| --- | --- | --- |
| `bench-window` | none (PID, owner and bounds need none) | visibility, activation, displays |
| `bench-sample` | none (same-user `proc_pid_rusage`) | footprint, CPU, wakeups |
| `bench-input` | Accessibility (post events) | `typing`, `caret`, `scroll` |
| `bench-capture` | Screen Recording | `typing`, `caret`, `scroll` |

`bench-input` posts every event to the measured app's PID
(`CGEventPostToPid`), never to the HID stream, so no other process can
receive it; it refuses to run without a PID. It still aborts when the target
loses the foreground, and occlusion still invalidates the trial.

## Metrics

- Memory: phys_footprint of the app and of its process tree at 1 Hz, p50,
  p95 and max per phase, plus the largest footprint per child name.
  `footprint_tool_end` cross-checks Apple's `footprint` at trial end.
- CPU, idle wakeups, interrupt wakeups and energy: exact deltas between the
  phase's boundary samples. CPU and wakeups include reaped children; energy
  counts processes alive at the phase end.
- Keypress-to-screen: post time to the display time of the first captured
  frame whose region hash changed. PID-targeted input skips HID routing,
  the same for every app. A key sent before the previous key's response is
  counted as overlapped, not timed.
- Frame interval while scrolling: display-time gaps between changed frames;
  `frame_late_pct` counts gaps over 1.5 refresh periods.
- Output: raw TSV in `results/<run>/` (gitignored). `bench baseline` appends
  milestone rows to the tracked `baseline.tsv`, each with commit, tree state,
  machine, macOS build, display mode, power state and Zed's version.

## Comparable and not

Matched across apps: fixture bytes, the requested window frame (960x572 pt;
trials record the actual one), Menlo 15 pt on 22 pt lines, a steady caret,
environment, rust-analyzer binary and the capture path. Not matched, so
never claim them:

- Zed runs non-default settings: no updates, telemetry, AI or sign-in
  server, `cursor_blink` off, Menlo. Its default blinking caret would cost
  idle frames this bench does not count.
- Chrome differs: tab bars, status bars and title bars.
- `open-repo` gives Alpine and the reference editor the file; Zed gets the
  folder and the file.
- Capture latency is included and equal for all apps; the numbers are
  black-box latency, not presentation timestamps.
- The reference editor has no language features; it is the native floor for
  input, scrolling and memory only.

## Zed isolation

A bench Zed gets a disposable HOME (logs, caches and config follow HOME),
`--user-data-dir`, seeded settings, `ZED_UPDATE_EXPLANATION` and a dead
`ZED_SERVER_URL`. `bench zed-isolation` launches it that way and compares
about 56,000 real Zed paths and the app version before and after. On 2026-10-02,
with the owner's Zed running, the probe migrated its databases in the
disposable home, met the single-instance check after 409 ms and exited with
no window; nothing real changed. macOS state outside HOME stays shared:
Zed calls `noteNewRecentDocumentURL`, so fixture paths can join Zed's Dock
recents, and that list needs Full Disk Access to inspect.
