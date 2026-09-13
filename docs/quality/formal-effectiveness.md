# Optional proof effectiveness

Kani controls check for vacuous or incorrectly scoped bounded proof results.
They run when investigating a concrete risk, not as ordinary PR acceptance.

## Kani reachability

Kani runs with detailed property output. Its versioned harness inventory must
exactly match `formal/kani/effectiveness-controls.tsv`, and every harness must
own at least one satisfied cover obligation. The manifest records exact cover
counts per harness, so a new harness or a removed, unreachable, or unsatisfied
cover fails closed. Failed or undetermined properties also fail the gate.

Kani can report compiler-generated panic branches as unreachable after a proof
or optimizer establishes that the failing branch cannot execute. Those records
are retained as total and repository-source counts, but only explicit
`kani::cover!` reachability is a blocking effectiveness control. Source
assumption occurrences are retained for review rather than treated as a score.

`target/kani/effectiveness-harnesses.tsv` records the checked harness identities.
`target/kani/effectiveness.toml` records the pinned tool version, revision,
harness, cover, and assumption counts plus hashes of the inventory, raw proof
log, and harness rows.

## Interpretation limits

These reports establish bounded control sensitivity and reachability for the
executed revision. They do not establish that a model matches native AppKit or
Metal behavior, that bounds cover production state spaces, or that a property
is the right product requirement. The registry governs evidence for explicitly requested assurance runs. Ordinary
PRs use relevant behavioral tests and applicable macOS CI; mutation and full
physical qualification are not unconditional registry-driven gates.
