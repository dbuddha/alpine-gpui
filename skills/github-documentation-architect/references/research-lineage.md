# Research lineage contract

Lineage answers where a mechanism came from, how the product changed it, why the
change was accepted, and what evidence supports any claimed benefit.

## Required identities

- Current implementation revision and source path.
- Accepted comparator revision.
- Current-upstream review revision when different.
- Research or lab revision.
- Exact upstream source path and line range when applicable.
- License and provenance mode.
- Implementing issue and pull request.

Keep accepted comparator, current upstream, Alpine implementation, and lab
revisions separate. Updating one never silently advances another.

## Mechanism classifications

- `ADAPTED-CONCEPT`: Alpine independently implements a source-backed concept.
- `INDEPENDENT-CONVERGENCE`: similar outcome without material upstream design
  dependence.
- `ALPINE-ORIGINAL`: Alpine-specific contract or strengthening with no claimed
  upstream origin.
- `COMPARATOR-ONLY`: informs workloads or validation but not shipping design.
- `REJECTED`: evaluated and intentionally excluded.
- `DEFERRED`: outside the accepted critical path and not promised future work.

Do not classify source as copied without exact origin and destination ranges,
license compatibility, transformation record, author and reviewer identity, and
an accepted provenance decision.

## Change protocol

For each material mechanism change, update the source map, mechanism matrix,
evidence row, and append-only history. Record retained, modified, rejected, or
superseded behavior and the next missing experiment. Preserve unfavorable,
invalidated, and superseded findings.
