# Documentation Instructions

These rules apply under `docs/` and refine the repository-wide contract.

## Source of truth

- Put enduring boundaries in `ARCHITECTURE.md`.
- Put ordered delivery details in `docs/MASTER_PLAN.md`.
- Keep `docs/ROADMAP.md` as a compact milestone view.
- Record accepted architectural decisions as numbered ADRs.
- Record upstream observations in `docs/research/` at immutable commits.
- Record copied or adapted source only in the provenance ledger.
- Do not repeat a decision in multiple files without linking to its authority.

## Research quality

- Prefer specifications, official platform documentation, repository source,
  issues, PRs, and reproducible measurements.
- Cite claims inline with durable URLs.
- Record the review date and exact commit for changing repositories.
- Separate observed fact, upstream claim, and Alpine inference.
- Treat README performance claims as claims until independently reproduced.
- Never convert an upstream implementation detail into an Alpine requirement
  without explaining the behavioral reason.

## Diagrams

- Use Mermaid only when relationships or sequence are clearer than prose.
- Quote node labels containing punctuation.
- Keep diagrams aligned with the authoritative prose and crate names.
- Update every affected diagram when an ownership boundary changes.

## ADRs

ADRs contain Status, Date, Context, Decision, and Consequences. Supersede an ADR
with a new ADR rather than rewriting accepted history. Corrections that do not
change the decision may be edited with a clear commit explanation.

## Verification

- Check relative links and referenced paths.
- Preserve valid Markdown headings, tables, lists, and fences.
- Search for stale project and crate names after renames.
- Run `scripts/check.sh` before handoff.
