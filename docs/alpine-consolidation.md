# Alpine consolidation, 2026-09-12

## Canonical state and recovery

The normal `personal/alpine-gpui` checkout is the active product repository,
starting at main `3b4f89ba862442bf233146118899af71c586bcfd` on
`chore/capability-probe-readiness`. The separate `alpine-zed-lab` keeps GPL
comparison sources isolated. No branches were deleted. The cleanup was subsequently published through
protected PRs #607 (product) and #30 (lab).

Local recovery is `/Users/deepak/alpine-recovery/20260912T053747Z`. It contains
Git bundles, staged/unstaged binary patches, source and untracked-file archives,
checksums, original ref/worktree inventories and restoration receipts. Five
snapshots were reconstructed from their bundles and patches, checking every
source file's contents, mode and symlink target plus staged/untracked status.

The physical and presentation worktrees were removed through Git after verified
preservation. Their ignored outputs were moved into the archive. The complete
temporary cleanup clone, including native failure logs, is under
`retired/cleanup-checkout`. Archived checkouts are recovery material, not active
instructions or skill installations. Existing product build caches remain local.

## Unfinished-work dispositions

| Source | Disposition and next action |
| --- | --- |
| Primary `3ee0ed9`, including seven unpublished commits | Deferred exact source in `primary.bundle` and `primary/`. Review overlay/LSP correctness against current main; port focused changes in native recovery, not the accumulated branch or old CI. |
| Dirty AX event bounds and untracked regression tests | Readiness candidate in `primary/unstaged.patch` and `primary/untracked.tar`. Recovered on `fix/native-readiness`: shared event budget and loss rejection, with regression controls. |
| Physical `8e23c44` dirty capture scripts/docs | Readiness candidate in `physical/`. Recovered on `fix/native-readiness`: private bounded rejected captures, including copy/hash failure preservation. Independently reviewed. |
| Presentation `429fc65` dirty Metal/platform tests | Diagnostic reference in `presentation/`. No production fix inferred. Consult issue 511 before another experiment. |
| Dirty recovery plan, project index and SUMMARY | Superseded process retained in the archive. Technical observations need current-source reproduction; old hierarchy, Wiki and skill-evaluation mandates are inactive. |
| Clean temporary cleanup clone `43eb2e0` | Its source tree was already represented by current main. Retired intact; native failure logs remain available. |
| Open scene-export PR 585 | Deferred upstream source. Required for representative capture only after its correctness and equivalence gaps are resolved. |
| Comparison lab `ff279fb` | Preserved in `lab.bundle`; remains active on main after protected PR #30. Pins and historical evidence remain unchanged. |

## Review boundary

Block 1 local checks passed: product policy and its failure/admission fixtures,
native-command controls, native harness compilation, formatting, four skill
validators and changed local links. The actual configuration-disabled harness
rejected an explicit native request as expected. The full lab check passed,
including 42 Python tests, source/pin checks, aggregate gates and bundle fixtures.
Independent review found no material blocker and rechecked all 2,488 restored
source files across the five snapshots.

The product policy scan now prunes the root build cache instead of traversing it
before filtering. Old and new inventories matched all 15 manifests; one local
scan fell from 4.055s to 0.031s. This is not a hosted CI or UI benchmark. The
initial long policy run was deliberately interrupted for this correction; the
complete suite then passed on the corrected source.

The original vault ideology note was preserved under `context/`; only its current
status pointer was updated. No other vault work was consolidated or committed.
Physical/native runtime failures remain unresolved. Existing timeout handling can
discard child output before the wrapper receives it; preserve that evidence as
part of block 2 if a parent timeout is reproduced. Product cleanup PR CI passed
in 7m50s and resulting main `76d385d` in 8m37s, including queueing. Lab cleanup
PR CI passed in 17m50s; the lab still misses the 15-minute target. No physical
performance qualification is claimed.

Block 1 consolidates source and verification entrypoints. It does not certify the
unmerged fixes, repair the physical failure, or establish a benchmark result.
The user approved block 2 on this checkpoint. Current findings belong in the
[capability probe](alpine-capability-probe.md). Local success is not hosted CI
acceptance. Three older test-app processes and the newly inspected normal release
instance still have potentially unique recovered buffers; source worktree retirement
did not establish that these in-memory buffers can be discarded. Do not force-close
them merely because their source checkouts were retired.
