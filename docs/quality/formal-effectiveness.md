# Formal assurance effectiveness

Passing a model checker is not sufficient evidence when the wrong property can
fail or a proof can pass vacuously. Alpine therefore retains effectiveness
evidence alongside the raw TLA+ and Kani output.

## TLA+ controls

Every `Faulty*.cfg` configuration is registered in
`formal/tla/effectiveness-controls.tsv` with the exact invariant it must violate.
The gate rejects missing, extra, or duplicate controls and rejects a
counterexample that violates a different invariant. Positive runs must report
successful completion, a drained exploration queue, generated and distinct
state counts, and search depth.

`target/tla/effectiveness.tsv` retains one row per positive model and negative
control. `target/tla/effectiveness.toml` binds the report to the revision, TLC
version, mode, row hash, and control counts. State counts are reported per model
because a larger state space is not inherently stronger evidence.

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
is the right product requirement. Dynamic, native, mutation, and physical
evidence remain required according to the assurance registry.
