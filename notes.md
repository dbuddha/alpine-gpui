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

Main at inspection: 6dd8ff0, diagnostic PR #675 merged. M1 is a limited
baseline, not proof of typing latency, comparable LSP memory or 120 Hz.
The delivery automation is paused. New screen sessions require approval.

## Active work

- Document the November 3 approved contract on codex/project-contract.
  No product or CI gate code changes belong in this PR.
- Next implementation: qualify screen-based physical evidence, retaining the
  existing timestamp failure as a visible result until replacement qualifies.
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
- Existing benchmarks do not prove everyday startup readiness. Audit their
  endpoints and original statistics before revising memory/startup budgets.
- OCaml/Haskell, structural parsing, Vim, Git, Markdown preview, preferences
  UI and debugger are planned scope, not implemented claims.

## Next steps and decisions

1. Review and land the documentation contract with its instruction checks.
2. Specify and test physical scene/text verification offline; request a timed
   screen session only when the live experiment is ready.
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
