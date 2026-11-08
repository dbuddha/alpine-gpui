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

Main at inspection: 741a8f1. #678 merged and its main CI and installed smoke passed. M1 remains a limited baseline.
The delivery automation is paused. Screen sessions require approval.

## Active work

- #671 integrates main 741a8f1. Seven before-fix assertions covered tab/pane offsets, scoped Find, RTL marked text, large-line rejection, explicit navigation and decoded views. The installed b0f5805 also lost its manual pan on tab return. Corrected headless routes pass; installed verification of the fixes and qualified performance rows are pending.
- #680 at ecf1e21 passed its installed file-backed rescue scenarios. Draft #681 at 5831a04 passed its installed healthy and degraded scratch journeys. Dock launch in an isolated profile remains open.
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

- The #678 positive, stale and wrong-scene fixture controls were checked before merge. This evidence does not qualify editor scrolling or replace the missing Metal timestamp.
- #675 improves missing-presentation diagnostics only. Its merged-main live
  check still failed qualification: three submissions, zero qualified
  presentations. It did not repair the platform measurement limitation.
- #666 at 20bb4a9 has green CI, visible bounded recovery and 12 valid matched idle pairs. Window-filtered scroll capture was refused for a 66 by 20 point auxiliary. The app-filtered display crop was non-opaque. Qualified scroll rows remain unavailable.
- #671 at b0f5805 has passing CI on its previous base. Matched physical rows and the installed horizontal-scroll journey must cover the updated head before merge.
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

1. Review the #671 integration and verify wheel, hit-testing, caret and input behavior on the clean installed build.
2. Complete the isolated Dock journey for UI PRs and the applicable physical performance gates. Keep unavailable outcomes visible.
3. Review the remaining stability PRs one at a time.
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
