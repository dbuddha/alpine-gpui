---
role: operating instructions
author: Deepak Buddha
updated: 2026-11-08
authority: owner-approved project contract of 2026-11-03 and research remit of 2026-11-08
related: [project.md, notes.md, README.md, ARCHIVE.md]
scoped_rules: [crates/AGENTS.md, apps/alpine-editor/AGENTS.md, apps/alpine-editor/src/AGENTS.md, bench/AGENTS.md]
dev_mac: Mac16,1 M4, built-in ProMotion display, macOS 26.6.2, Command Line Tools only
stage: 1, evidence and stability
autonomy: routine branches, PRs and merges with applicable gates green
ask_first: [paid actions, dependency changes, destructive actions, new subsystem design beyond project.md, public API changes, relaxed invariants, unsafe boundary or allowlist changes, license or copied-source changes, further gate relaxation]
next:
  - qualify physical recovery and matched performance for PR 666, then PRs 671, 672 and 674
  - repair semantic typing and equivalent workspace measurement
  - owner stability review and five working days of dogfood
---

# Alpine operating instructions

Alpine Editor is the first product. Alpine GPUI remains a reusable,
performance-first framework. Correctness and no lost work are mandatory.

## Read before work

Explicitly read [project.md](project.md), [notes.md](notes.md), and applicable
scoped AGENTS.md files before non-trivial work. Do not assume project or notes
files load automatically. This repository uses lowercase `notes.md` as its
current-state file instead of the parent's NOTES.md convention.

These instructions own operating rules; project.md owns goals, scope, stages
and acceptance; notes.md owns current state. Scoped rules add local contracts.
Code is evidence of implemented behavior, not proof that a target is complete.
Correct stale notes when they disagree with code. ARCHIVE.md is history, never
an operating rule. The GitHub repository is the project source of truth.

Use [app-builder](skills/app-builder/SKILL.md) for Alpine engineering and comparative research.
Add owner-named references to the [case studies index](<case studies/README.md>) and link findings to an Alpine experiment. project.md governs adoption.

## Architecture

- Direct Metal in the hot path; no generic GPU abstraction.
- No reactive graph or general async executor. Work is demand-driven; frames
  require invalidation and idle submits nothing.
- Bound queues, caches, workers, retained bytes and frames. Every allocation
  needs a ceiling and an eviction or release rule.
- Immutable scenes have explicit painter order. Layout covers the viewport
  plus overscan, with cost scaling to visible work.
- Reject stale results by revision. Never wait for language or debugger work
  on the input/render path.
- No runtime downloads. No Zed application source enters this repository.
  Approved Apache-2.0 GPUI source belongs only in bench/.
- Approved copied source retains its license header and an ARCHIVE.md entry.
- Preserve correctness, then optimize responsiveness, memory and energy.
  Reuse must not require editor-specific behavior in framework crates.

## Evidence and gates

Use the change-specific matrix and numeric contract in project.md. Targets
are not measurements. Unavailable evidence is not a pass. No product code or
CI gate is changed merely by updating this contract.

- T0 before commits: affected crate tests, workspace clippy, formatting and
  relevant policy checks. Documentation-only changes have no affected crate;
  check frontmatter, links, instruction consistency and the complete diff.
- T1: existing CI build, tests, clippy, fmt, dependency and native checks.
- T2: approved dev-Mac checks for frame-path, startup, LSP and cache changes,
  and milestone evidence. Hosted runners prove correctness, never physical
  timing, presentation or footprint.
- T3: local perf recorder and owner dogfood. Each milestone needs Deepak's
  verification; do not mark it accepted while that review is pending.
- Bug fixes need a failing-before regression test, or manual evidence for
  script fixes. Guard latency work with deterministic counters, not clocks.
  Randomized tests print a replayable seed. Re-run a flaky test only for a
  listed known defect and disclose the rerun.
- Capture event admission, mutation, layout, shaping, scene build, upload,
  encode, commit, GPU completion, target and presentation separately.
  GPU completion and display-link
  targets are not presentation. Metal timestamps are unreliable on the dev
  Mac; a replacement screen-based gate must first pass qualification.
- Measure phys_footprint, editor and each child separately and in total.
  Comparisons that do less work cannot support superiority claims.
- Bench rows and bundle stamps record commit and tree state. Measure a
  difference before building on it.

## Delivery loop

- Inspect branch, upstream and dirty state; fetch origin before remote
  reasoning. Never revert, overwrite, stash or delete the owner's work.
- One feature and one implementer in flight. Read-only exploration can run
  in parallel; use a fresh-context reviewer for each PR, at most four agents.
- An owner-enabled heartbeat avoids duplicate work and unchanged-state pings.
  Stop milestone completion at owner checkpoints.
- Each implementer owns a worktree and target/. Briefs name objective, exact
  gate, invariants, scope, deliverables and stop condition.
- One outcome per PR, about 500 product-code lines unless justified. Update
  notes.md when state changes; update project.md only when scope or acceptance
  changes. Keep this file's next list aligned without copying the roadmap.
- Before merging, the lead reruns applicable gates on the final head, reads
  the full diff including untracked files, resolves findings and verifies
  change-specific evidence. Arm auto-squash with branch deletion and a CI
  failure wake-up. A later push disarms merge until review and gates repeat.
- After merge, check main CI on the merged SHA, rebuild and smoke-test. A
  screen-dependent smoke test waits for session approval; do not substitute
  a command-line check for the physical test.
- At milestone close, deliver an installed build, test guide, measurements
  and limitations. Wait for owner acceptance; independent preparation may
  continue. Record acceptance and evidence in ARCHIVE.md.
- After two failures on one blocker, stop that path and report evidence;
  continue independent work. Do not turn a hold into a passing result.

## Screen sessions and approvals

Ask for screen access each session with expected duration. Prioritize an
approved session and continue independent work while waiting. Earlier away-time
screen permission is not standing permission for new sessions.

Use one minute of input quiet and at most five minutes waiting for conditions.
Abort and record an unavailable session if conditions fail. Use bounded helper
processes; do not rely on a UI tool timeout to end a stuck call. Stop on owner
input or a locked screen. Only stop processes owned by the check.

Routine pushes, PRs and green-gate merges in dbuddha/alpine-gpui are authorized.
Ask first for frontmatter exceptions. Never force push, amend pushed commits,
publish secrets or bypass protection. Install tools only from official
installers or binaries, never Homebrew. Record installs in ~/README.md without
private project details. Do not access unnamed repositories.

## Documentation

Keep AGENTS.md files as maps and rules, project.md as the project contract,
notes.md under 100 lines, README.md as implemented behavior, and ARCHIVE.md as
dated history. Root rules stay below 1,200 words; scoped rules below 900.
Use YAML frontmatter with role, author, update date, authority and related
documents. Credit authored documentation to Deepak Buddha. No em dashes.
Retain the parent's commit and PR writing rules. Issues and boards remain
retired; current defects live in scoped known_defects and notes.md.
Do not add extra ledgers or scripts that test scripts. Keep code comments
short and code-local, under the parent's comment rules.

Commands and native-test caveats are in [crates/AGENTS.md](crates/AGENTS.md).
