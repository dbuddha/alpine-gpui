# AEP 0180 project-search admission model

`ProjectSearchAdmission.tla` models finite open, query change, bounded batch
publication, terminal completion, stale-result drop, selection, and close.

Rust event mapping:

- `Open` maps to `ProjectSearchState::open`.
- `ChangeQuery` maps to committed text and backward deletion.
- `PublishCurrentBatch` maps to current `ProjectSearchWorkerOutput::Batch`
  admission.
- `CompleteCurrent` maps to terminal batch admission.
- `DropStale` maps to rejected inventory or batch identity.
- `SelectCurrent` maps to selected-match retrieval followed by exact path and
  buffer revalidation.
- `Close` maps to `ProjectSearchState::close` and foreground release.

Pull-request checking uses four query generations, eight retained matches, and
two matches per batch. Nightly checking doubles those bounds.
`FaultyStale.cfg` publishes a non-current generation and must violate
`PublishedIsCurrent`. `FaultyOverflow.cfg` exceeds the retained-result ceiling
and must violate `ResultsAreBounded`.

The model assumes typed worker outcomes, monotonic finite generations, and one
foreground owner. It excludes Rust refinement, query bytes, filesystem and
ignore semantics, allocation, runtime scheduling, native input, and elapsed
time.
