---
role: current project state and continuation
author: Deepak Buddha
updated: 2026-11-08
authority: observed repository state; project.md owns acceptance
related: [AGENTS.md, project.md, apps/alpine-editor/AGENTS.md, bench/AGENTS.md, ARCHIVE.md]
stage: 1, evidence and stability
owner_checkpoint: pending, five normal working days after stability checks
---

# Current state

Goal: Deepak's daily-driver editor on a reusable, performance-first Alpine
GPUI. Follow project.md for the accepted scope and stage order.

Main at inspection: 741a8f1. #678 merged the verified diagnostic controls. M1 remains a limited baseline.
The delivery automation is paused. Screen sessions require approval.

## Active work

- #666 follows current main. The `scene-session --ended-silence` case passed offline checks and review.
  Verify visible recovery and matched idle/scroll on a clean final head.
- Draft #680 protects dirty path-backed buffers. Its clean installed release passed native Open, failed-save Quit, canceled Save As, rescue and missing-file/tab-switch journeys with disposable data. Cold Dock launch remains open.
- The main editor suite passed 707 tests with three ignored. The November 8 audit also passed eight focused tests for atomic save, recovery, quit, session restoration and navigation. A disposable regression on 3312df1 confirmed that first Quit is allowed after document save and recovery journaling both fail. The edited buffer and external disk bytes were checked before Quit.
- Maintain app-builder and the [case studies](<case studies/README.md>) alongside the six-stage delivery contract.
- The November 3 installed build at 3312df1 passed disposable edit/save/reopen. A post-quit UI query reopened the normal profile with conflicting recovered edits. Check exit by process identity after Quit and leave recovered work to its owner.
- #676 main CI passed (41663939024); clean bf1b394 was tested. Approved
  November 3 smoke passed edit/save/fresh-process reopen/quit via LaunchServices,
  with disposable data and no crash log. Cold Dock launch remains untested;
  native Open appeared, but automated path entry failed. Screen use has ended.
- Qualify screen-based physical evidence before replacing the timestamp gate.
- Continue the existing PR queue one at a time after the evidence decision:
  #666 pending-frame recovery, #671 horizontal scroll, #672 unsaved overlays,
  #674 Finder/Dock opening. Refresh remote heads before reviewing each.

## Evidence and blockers

- #678 head c91ca53 passed all three controls in one set. Positive showed A then B, stale showed A after its admission witness, and wrong showed the expected invalid pattern. Each recorded attached final proof, capture stop and exact native teardown.
- Main 741a8f1 CI passed. The clean release passed disposable edit/save, fresh-process reopen with visible saved text, and normal quit.
- A single 60.218 s main idle interval had zero new recorder submissions. Same-PID samples bracketed 92.252 s, with one completed startup attempt. See bench/results/stability/741a8f1-idle-recorder-method. Matched trials are still required.
- The main checkout retains raw results under bench/results/scene-inspection/alpine-scene-c91ca53-nov08. The fixed-scene method passed on the reference Mac. Typing, matched performance and the Metal timestamp hold require separate evidence.
- #675 improves missing-presentation diagnostics only. Its merged-main live
  check still failed qualification: three submissions, zero qualified
  presentations. It did not repair the platform measurement limitation.
- #666 remains held on physical evidence. Repeated zero timestamps do not
  justify another blind renderer change or a passing result.
- #671 at b0f5805 includes reviewed find/IME/sidebar caret-reveal fixes and
  passing CI. Physical idle/scroll qualification remains incomplete; a
  system alert blocked the first run, and the retry stopped for goal review.
- #672 remote head is 0ecc486; its local worktree is older. Do not treat the
  local checkout as the current PR head. #674 is e4bb655.
- Semantic typing analysis currently cannot qualify intended text/caret;
  repeated runs cannot fix this implementation gap.
- Open-repo gave Zed folder plus file and Alpine only a file. Comparable
  workspace loading and server-memory claims remain unqualified.
- Historical launch medians: idle 283.351 ms, 50 MiB 425.871 ms, below the
  retained 335/475 ms budgets. Idle tree p50 mean was 29,021,014 bytes, median
  28,877,664; idle app/tree matched. These do not prove language-ready startup
  or comparable workspace/server memory; no budget changed.
- OCaml/Haskell, structural parsing, Vim, Git, Markdown preview, preferences
  UI and debugger are planned scope, not implemented claims.

## Next steps and decisions

1. In an approved session run #666’s single ended-silence case. It injects
   35 pending-silence ticks immediately before B, services window inspections
   while B waits, and requires 1–3 restarts, a resumed callback, one eligible
   submitted B, timer/slot/byte drain, visible A-to-B and exact teardown.
   The original Metal timestamp and separate matched idle/scroll checks remain open.
2. Freeze matched idle/scroll workloads and trial counts. Bound preconditions to five minutes and preserve invalid or adverse results.
3. Qualify remaining stability PRs individually and verify each merged head.
4. Complete stability scenarios and give Deepak a build/test guide for five
   normal working days. Record acceptance explicitly.

No product-scope decision is pending. Separate approval remains necessary for
paid actions, dependencies, public APIs, unsafe boundaries and further gate
relaxation. Screen approval and milestone acceptance are pending when needed.

## Preservation and gotchas

- Owner worktree pending-frame-progress has seven modified files; never
  overwrite, stash, revert or delete them. Other existing worktrees stay intact.
- Every worktree owns its target/. Do not reuse build-script paths across
  worktrees. Native test filters can pass while running zero tests.
- Initial native visibility is sampled before the event loop; false there
  does not establish that a later physical failure was caused by occlusion.
- Ordinary benchmark CLI waits still need external bounds. The opt-in scene
  supervisor enforces one-minute quiet/five-minute maximum precondition wait.
- The prospective twelve-trial protocol does not rewrite historical results
  or change ordinary benchmark CLI defaults.
