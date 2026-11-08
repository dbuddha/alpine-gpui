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

Main at inspection: e482b9f. #679 merged the skill, research and project instructions. M1 remains a limited baseline.
The delivery automation is paused. Screen sessions require approval.

## Active work

- #678 is draft. CI passed at 927b108. Live capture saw the baseline and then an unexpected second fixture window. Diagnose its identity before qualifying the controls.
- Stage 1 audit on November 8: eight main editor tests passed for atomic save, recovery, quit, session restoration and navigation. A disposable regression on 3312df1 confirmed that first Quit is allowed after document save and recovery journaling both fail. The edited buffer and external disk bytes were checked before Quit.
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

- #678 CI passed at 365d8da (41676762061); 243 bench tests and required
  clippy/fmt/policy checks passed again. One later attempt expired waiting for
  AC and input quiet, before any fixture launched. The next approved attempt
  captured 15 valid baseline-A frames, then rejected an extra same-PID 66x20
  window over the title-bar area. No action or negative controls ran; no normal
  teardown record exists. The owned fixture exited. Its identity is unproven;
  do not ignore small windows. The temporary raw trace expired. The reviewed run summary is in #678.
- The earlier 502x332 outer-frame mapping fix passed review and 243 tests.
  Native submissions still have zero qualified presentations. Metal hold unchanged.

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

1. Run the reviewed AppKit lookup on rejected fixture window IDs during an approved session. Record identity and normal teardown, retaining any rejection.
2. Fix the reproduced first-quit failure and verify the edited buffer, external disk contents and deliberate discard path.
3. Qualify the physical method, then run affected PR gates and review each final head.
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
