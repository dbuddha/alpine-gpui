---
program: Alpine GPUI framework, Alpine Editor app, a terminal app later
updated: 2026-10-17
precedence: AGENTS.md files, code comments, README.md
archive: ARCHIVE.md (history only, never an operating rule)
scoped_rules: [crates/AGENTS.md, apps/alpine-editor/AGENTS.md, apps/alpine-editor/src/AGENTS.md, bench/AGENTS.md]
dev_mac: Mac16,1 M4, built-in ProMotion display, macOS 26.6.2, Command Line Tools only
goals:
  editor: daily driver that beats Zed at parity (see apps/alpine-editor/AGENTS.md)
  framework: beats Zed GPUI; stability and durability before public API
  later: a separate terminal app hosting the editor as a pane
beat_zed_on: [keypress_to_screen, memory_with_servers, frame_cadence, startup_and_big_inputs, reliability, energy, under_load]
comparators:
  app: /Applications/Zed.app, version recorded on every bench row
  framework: Zed GPUI scenarios in bench/ (M5)
  native: AppKit reference app in bench/; match targets in apps/alpine-editor/AGENTS.md
budgets:
  event_to_target_present_p95_ms: 25
  event_to_target_present_p95_ms_target: 16.7
  frame_interval_p95: one display refresh, every frame-producing interaction but launch and file-open
  missed_frames_pct: 1
  cpu_frame_prep_p95_ms: 4
  event_to_submit_p95_ms: 4
  idle_cpu_pct_of_one_core: 1
  idle_frame_submissions: 0
  editor_footprint_steady: 30.5 MiB (M1 idle tree p50 27.7 MiB plus 10 percent)
  launch_to_window_median_ms: 335 (M1 283 plus 50)
  open_50mb_to_window_median_ms: 475 (M1 426 plus 50)
  language_servers: reported per server, not gated
milestones:
  - {id: M0, name: "reset: docs, CI, hygiene", status: done}
  - {id: M0e, name: "fix hosted native flakes", status: done}
  - {id: M1, name: "measurement: perf recorder and bench", status: done}
  - {id: S, name: "stability: no lost text or hangs", status: planned}
  - {id: M2, name: "presentation latency and real 120 Hz", status: planned}
  - {id: M3, name: "daily-use defects and durability", status: planned}
  - {id: M4, name: "language intelligence, phase 2 close", status: planned}
  - {id: M5, name: "framework bench against Zed GPUI", status: planned}
  - {id: M6, name: "editing parity, phase 3", status: planned}
  - {id: M7, name: "git and context, phase 4", status: planned}
  - {id: M8, name: "real application and parity sweep, phase 5", status: planned}
autonomy: "Deepak: agents may push branches, open and merge PRs, and delete their own merged branches, except ask_first"
next:
  - "S: S7 qualification, S12 owner check, S11 Open With, flake fixes"
  - "M1 follow-up: does rust-analyzer load the open-repo workspace?"
  - "CP-S: owner dogfood day"
ask_first: [dependency changes, destructive actions, milestone design kickoff, new subsystem or public API or relaxed invariant, unsafe boundary or allowlist change, license or copied-source change, weakening a CI gate or threshold, anything that costs money]
---

# Alpine

Alpine GPUI is a Rust application framework for Apple Silicon macOS, with a
Direct Metal renderer. Alpine Editor is the daily-driver editor built on it.
This file is the program; scoped AGENTS.md files hold local rules.

## Contract

Frontmatter budgets gate every PR on the dev Mac, except `frame_interval_p95`
and the editor's performance targets, which gate once a milestone close first
meets them. "Beats Zed" and the AppKit match are checked at each milestone close
on matched workloads: ten fresh-process trials, confidence intervals,
unfavorable results reported.

## Invariants

- No reactive graph and no general async executor. Work is demand-driven; a
  frame happens only when an invalidation asks. Idle submits nothing.
- Everything is bounded: queues, caches, workers, retained bytes, frames. A new
  allocation without a ceiling and an eviction rule is a defect.
- Scenes are immutable, with explicit painter order.
- Lay out the visible range plus overscan; cost scales with the viewport.
- Stale results are rejected by revision, never applied because they arrived.
- Direct Metal in the hot path. No generic GPU abstraction.
- Optimize for correctness, then responsiveness, efficiency, delivery.

## Measuring

- Separate stages: event admission, mutation, layout, shaping, scene build,
  upload, encode, commit, GPU completion, display-link target, presentation.
- Neither GPU completion nor a display-link target is presentation; missing
  presentation is missing evidence. presentedTime is zero on the dev Mac (even
  in a plain MTKView app), so keypress-to-screen comes from the bench.
- Memory is phys_footprint, editor and each child separately, never RSS.
- Installed app, fresh process, disposable HOME, AC power, quiet machine.
- A comparison that does less work fails. Sample memory at semantic points; a
  bounded cache still fails if footprint never plateaus.
- Hosted runners expose "Apple Paravirtual device" and no display. They prove
  correctness, never timing, presentation or footprint.

## Testing tiers

- T0, before every commit: `cargo test --locked -p <crate>`, clippy, fmt.
- T1, CI on every change: build, test, clippy, fmt, deny and native validation.
- T2, dev Mac bench: frame-path, startup, LSP or cache changes, and every
  milestone close.
- T3, dogfood: the in-app perf recorder, local only.
- Guard latency invariants with deterministic work counters, not wall clocks.
  A bug fix lands with a test that fails before it (script fixes: manual
  evidence in the PR). Randomized tests print a replayable seed. A flaky test is a defect; re-run only a failure listed in
  `known_defects`, and say so in the PR.

## How the lead agent works

- Merge routine PRs once gates pass, except `ask_first` items. Keep
  `next:`, the loop's checklist, current in every PR.
- One feature in flight. Explore agents search in parallel; a Plan agent
  drafts milestone designs; one implementer at a time works in its own
  worktree; a fresh-context reviewer checks every PR.
- Briefs name objective, rationale, exact gate, invariants, budgets, scope,
  constraints, deliverables and stop condition. Each worktree owns `target/`
  (shared, it caches build-script paths from the wrong checkout).
- At most four active agents. An owner-enabled heartbeat recovers work without
  duplicates or unchanged-state pings. Stop at milestone checkpoints.
- Before merging, the lead (never a subagent):
  1. re-runs the gate on the PR head;
  2. reads the full diff, including untracked files;
  3. resolves or explains every review finding;
  4. checks evidence by change type: Dock-launch screenshot for UI, bench rows
     for performance paths, a real-server run for LSP, a failure-path test for
     data;
  5. arms `gh pr merge --auto --squash --delete-branch` and the app's CI-failure
     wake-up; any later push disarms it until steps 1-4 repeat. After merge:
     main CI on the merged SHA, rebuild, smoke-test.
- Milestone close: ten-trial bench run, frontmatter updated, one ARCHIVE line
  with numbers and PR links.
- Two failed attempts on one blocker: stop and ask.

## PRs

One outcome per PR, about 500 lines of product code unless justified, and a
type prefix that matches the content. Body sections: Context, Root cause (bug
fixes), Evidence, Risk and scope, Test plan. Update frontmatter in the same PR.

## Provenance

- Copied third-party source needs approval, its license header and an ARCHIVE
  line. Zed application (GPL) source never enters this repo; Apache-2.0 GPUI
  may appear only in `bench/`.
- The app downloads and executes nothing at runtime.
- Bench rows and bundle stamps record commit and tree state.

## Docs and issues

Only AGENTS.md files, ARCHIVE.md, README.md and LICENSE.md. Label targets;
correct instructions with the code that changes them. No ledgers, registries
or scripts that test scripts. Code-local contracts are comments of at most 3
lines. Caps: this file
1,200 words, scoped files 900; rules overflow into a scoped AGENTS.md, history
into ARCHIVE.md. Issues and the board are retired; defects live in scoped
`known_defects` lists and close with the fixing PR.

## Working rules

- Inspect branch, upstream and dirty state. Preserve unfinished work.
- Measure a difference before building on it.
- Never publish secrets, rewrite published history or bypass protection.

## Commands

In crates/AGENTS.md.
