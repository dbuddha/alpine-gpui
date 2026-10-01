---
program: Alpine GPUI framework, Alpine Editor app, a terminal app later
updated: 2026-09-30
precedence: AGENTS.md files, then code comments, then README.md, then vault notes and issue text
archive: ARCHIVE.md (history only, never an operating rule)
scoped_rules: [crates/AGENTS.md, apps/alpine-editor/AGENTS.md]
dev_mac: Mac16,1 M4, built-in ProMotion display, macOS 26.6.2, Command Line Tools only
goals:
  editor: daily driver that beats Zed at parity in core editing, navigation and search, language intelligence, and git
  framework: beats Zed GPUI and is the best Metal framework for these apps; stability and durability before public API
  later: a separate terminal app that hosts the editor as a native pane
beat_zed_on: [keypress_to_screen, memory_with_servers, frame_cadence, startup_and_big_inputs, reliability, energy, under_load]
comparators:
  app: /Applications/Zed.app, same workload
  framework: Zed GPUI scenarios in bench/ (M5)
  native: AppKit reference app in bench/ (M1)
budgets:
  event_to_target_present_p95_ms: 25
  event_to_target_present_p95_ms_target: 16.7
  frame_interval_during_input_ms: 8.33
  missed_frames_pct: 1
  cpu_frame_prep_p95_ms: 4
  event_to_submit_p95_ms: 4
  idle_cpu_pct_of_one_core: 1
  idle_frame_submissions: 0
  editor_footprint_steady: first M1 baseline plus 10 percent
  language_servers: reported per server, not gated
milestones:
  - {id: M0, name: "reset: docs, CI, hygiene", status: active}
  - {id: M1, name: "measurement: perf recorder and bench", status: next}
  - {id: M2, name: "presentation latency and real 120 Hz", status: planned}
  - {id: M3, name: "daily-use defects and durability", status: planned}
  - {id: M4, name: "language intelligence, phase 2 close", status: planned}
  - {id: M5, name: "framework bench against Zed GPUI", status: planned}
  - {id: M6, name: "editing parity, phase 3", status: planned}
  - {id: M7, name: "git and context, phase 4", status: planned}
  - {id: M8, name: "real application and parity sweep, phase 5", status: planned}
ask_first: [dependency changes, destructive actions, milestone design kickoff, new subsystem or public API or relaxed invariant, anything that costs money]
---

# Alpine

Alpine GPUI is a Rust application framework for Apple Silicon macOS with a
Direct Metal renderer. Alpine Editor is the daily-driver editor built on it.
This file is the program: goals, budgets, rules and how agents work. Scoped
AGENTS.md files hold local rules. This file overrides vault notes about Alpine.

## Contract

Budgets in the frontmatter gate every PR on the dev Mac. "Beats Zed" is
checked at each milestone close on matched workloads: ten fresh-process trials,
confidence intervals, unfavorable results reported. None of it is measured
until M1 lands; hosted CI cannot measure it.

## Invariants

Relaxing one is a product decision that needs approval.

- No reactive graph and no general async executor. Work is demand-driven; a
  frame happens because an invalidation asked for one. Idle submits nothing.
- Everything is bounded: queues, caches, workers, retained bytes, frames. A new
  allocation without a ceiling and an eviction rule is a defect.
- Scenes are immutable with explicit painter order.
- Lay out the visible range plus overscan. Cost scales with the viewport.
- Stale results are rejected by revision, never applied because they arrived.
- Direct Metal in the hot path. No generic GPU abstraction.
- Optimize in this order: correctness, responsiveness, efficiency, delivery.

## Measuring

- Separate stages: event admission, mutation, layout, shaping, scene build,
  upload, encode, commit, GPU completion, display-link target, presentation.
- GPU completion is not presentation. presentedTime reads zero on the dev Mac
  even for a plain MTKView app, so use CAMetalDisplayLinkUpdate timestamps in
  process and the black-box keypress-to-screen tool across apps.
- Memory is phys_footprint, editor and each child separately, never RSS.
- Installed app, fresh process, disposable HOME, AC power, quiet machine.
- A comparison that does less work fails. Sample memory at semantic points; a
  bounded cache still fails if footprint never plateaus.
- Metal validation on for correctness runs, off for timing runs.
- Hosted runners expose "Apple Paravirtual device" and no display. They prove
  correctness, never timing, presentation or footprint.

## Testing tiers

- T0, before every commit: `cargo test --locked -p <crate>`, clippy, fmt.
- T1, CI: build, test, clippy, fmt, deny; native validation for crate changes.
  Guard latency invariants with deterministic work counters, not wall clocks.
- T2, dev Mac bench (from M1): any frame-path, startup, LSP or cache change,
  and every milestone close.
- T3, dogfood: the in-app perf recorder (from M1), local only.
- A flaky test is a defect: no blind reruns, no weaker thresholds.

## How the lead agent works

- Merge routine PRs once gates pass. Ask Deepak first for anything listed under
  `ask_first`.
- One feature in flight. Explore agents search in parallel; a Plan agent
  drafts milestone designs; one implementer at a time works in its own
  worktree; a fresh-context reviewer checks every PR.
- An implementer brief states: objective and why, the exact gate, invariants
  and budgets in play, files in and out of scope, constraints, deliverables
  (branch, commits, PR body, raw gate output), and the stop condition.
  Implementers keep their worktree's own `target/`: a shared one caches
  build-script paths from the wrong checkout.
- Before merging, the lead (never a subagent):
  1. re-runs the gate on the PR head;
  2. reads the full diff, including untracked files;
  3. resolves or explains every review finding;
  4. checks evidence by change type: Dock-launch screenshot for UI, bench rows
     for performance paths, a real-server run for LSP, a failure-path test for
     data;
  5. squash-merges, confirms main CI on the merged SHA, rebuilds the installed
     app from main and smoke-tests it.
- Milestone close: ten-trial bench run, frontmatter updated, one ARCHIVE line
  with numbers and PR links.
- Two failed attempts on the same blocker: stop and ask.

## PRs

One outcome per PR, about 500 lines of product code unless justified, and a
type prefix that matches the content. Body sections: Context, Root cause (bug
fixes), Evidence, Risk and scope, Test plan. Update frontmatter in the same PR.

## Provenance

- No copied third-party source without its license header and an ARCHIVE
  line. Zed source never enters this repo; Apache-2.0 GPUI may appear only in
  `bench/`, after approval.
- The app downloads and executes nothing at runtime. Language servers are
  discovered, not fetched.
- Every installed bundle and bench row names its commit and dirty state.

## Docs and issues

Only AGENTS.md files, ARCHIVE.md and README.md, plus LICENSE.md. No ledgers,
registries or scripts that test scripts. Code-local contracts are comments of at
most 3 lines. Caps: this file 1,200 words, scoped files 900. Rules overflow into
a scoped AGENTS.md, history into ARCHIVE.md. Issues are a thin defect inbox
(steps, expected, observed), closed by the fixing PR; no board, no hierarchy.

## Working rules

- Inspect branch, upstream and dirty state. Preserve unfinished work.
- Measure a differentiator before building on it.
- Read the affected code and tests first; review the full diff.
- Re-check an environmental blocker after a real delay before calling it one.
- Never publish secrets, rewrite published history or bypass protection.

## Commands

```sh
cargo run --locked -p alpine-editor
cargo test --locked -p <crate>
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo fmt --all -- --check
scripts/check.sh
scripts/check-native.sh physical shipping
scripts/build-alpine-editor-app.sh
scripts/launch-alpine-editor-app.sh <file-or-folder>
```
