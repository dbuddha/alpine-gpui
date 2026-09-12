# Contributing

A user request or existing issue is enough scope for a focused fix. Keep one
outcome per PR and at most two active implementation changes. Use issues for
unresolved defects, blockers or work spanning PRs; hierarchy, labels, milestones
and Projects are optional. Confirm new product scope before implementing it.

## Implementation and verification

Keep behavior and regression evidence together. Read affected production code,
state the observable acceptance, and run the relevant checks described in
[testing](docs/testing.md). Use [development](docs/development.md) for setup.
Review the complete diff, including untracked files, before publication.

The author owns the change even when an agent writes it. Use one independent
reviewer for substantive changes. Assess findings against code and evidence;
fix actionable defects and explain disagreements. Re-review changed risk areas
instead of restarting a full review loop. If the same blocker persists after two
fix attempts, report the cause and seek a decision rather than endlessly retrying.

## PR acceptance

Use three sections: Problem and outcome, Change, and Verification and remaining
risks. Include reproduction and regression results, screenshots for visual changes,
and measurements for performance claims. A small fix needs a concise description.
Do not require issue IDs, research packages, claim IDs or template phrase checks.

A change is ready when its intended behavior is observed, relevant failure paths
are covered, the complete diff is reviewed, findings are addressed or explained,
and applicable CI passes for its current source and tested base.

Before merging, inspect live branch protection, required checks, mergeability
and unresolved discussions. Use the tested source SHA with normal protected merge;
verify main afterward. Never bypass a gate. Copilot feedback is advisory and does
not authorize changes to safety or acceptance rules. A substantive change still
needs independent review if Copilot is unavailable.

## Documentation and research

Correct existing instructions in the same change when behavior makes them wrong.
Batch new explanations after a feature settles or before a release. Internal
refactors, test changes and fixes restoring documented behavior need no doc update.
No documentation skill, issue or periodic documentation bot is required.

For research, keep one useful note per question: conclusion, source revision,
relevant evidence and limitations. Split it only when material warrants it.
Existing technical contracts and provenance remain valid references; they are not
mandatory context for an ordinary fix. Preserve relevant artifacts and update
references if moving a technical document. See [docs](docs/README.md).

## Boundaries

New public contracts, dependencies, unsafe boundaries, licensing, source copying
and acceptance-safeguard changes need owner authorization unless already granted.
Keep local safety arguments, dependency/license checks, exact source provenance
and relevant regression tests. Never copy Zed application code into Alpine.
