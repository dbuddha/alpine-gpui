---
role: current project state and continuation
author: Deepak Buddha
updated: 2026-11-03
authority: observed repository state; project.md owns acceptance
related: [AGENTS.md, project.md, apps/alpine-editor/AGENTS.md, bench/AGENTS.md, ARCHIVE.md]
stage: 1, evidence and stability
owner_checkpoint: pending, five normal working days after stability checks
---

# Current state

Goal: Deepak's daily-driver editor on a reusable, performance-first Alpine
GPUI. Follow project.md for the accepted scope and stage order.

Main at inspection: bf1b394, contract PR #676 and diagnostic PR #675 merged. M1 is a limited
baseline, not proof of typing latency, comparable LSP memory or 120 Hz.
The delivery automation is paused. New screen sessions require approval.

## Active work

- Implement scene capture and offline replay diagnostics on
  codex/scene-evidence-diagnostic. Synthetic checks do not qualify live output,
  action provenance, visibility, editor typing or native presentation.
- #676 main CI passed (41663939024); clean merged build is ready at
  /tmp/alpine-676-merged/Alpine Editor.app. CLI usage-error smoke passed;
  approval for the five-minute graphical smoke is pending.
- Qualify screen-based physical evidence before replacing the timestamp gate.
- Continue the existing PR queue one at a time after the evidence decision:
  #666 pending-frame recovery, #671 horizontal scroll, #672 unsaved overlays,
  #674 Finder/Dock opening. Refresh remote heads before reviewing each.

## Evidence and blockers

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

1. Review the bounded scene diagnostic capture and replay slice; preserve the
   explicit unqualified result and current shipping gates.
2. Prepare a native action-recording fixture and timed live controls. Request
   screen access only when that component experiment is ready.
3. Qualify the method, then rerun affected final-head PR gates and review.
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
- The approved one-minute quiet/five-minute maximum wait policy is not yet
  enforced by the bench CLI. Use bounded supervision until implemented.
- The prospective twelve-trial protocol does not rewrite historical results
  or change existing CLI defaults in this documentation PR.
