# Critical-path reconciliation

## Snapshot

Capture repository and branch identity, token scopes, capability tree, approvals,
sub-issues, dependencies, milestone membership, Project visibility and fields,
open pull requests, required checks, and release state before mutation.

## Stale-truth checks

- Duplicate issues for the same acceptance unit.
- Tasks with merged pull requests but ambiguous open state.
- Closed tasks missing their named evidence.
- Parent progress that counts non-leaf work.
- Project fields inconsistent with issue or pull-request state.
- Blocked items without a blocking issue, owner, evidence need, or age.
- Due dates unsupported by stable scope and throughput.
- Deferred work labeled or placed as a current blocker.
- Research left open after disposition while its Experiment is implicit.

## Algorithm

1. Validate type, approval, and one direct parent.
2. Validate native dependency edges.
3. Derive the longest unresolved dependency chain.
4. Assign milestones to accepted leaf outcomes.
5. Reconcile Project fields when readable.
6. Reconcile pull request, check, and closure state.
7. Report blockers, blocker age, scope growth, and next leaf.

Do not infer inaccessible Project state. Use issue hierarchy as the fallback and
name the permission gap.
