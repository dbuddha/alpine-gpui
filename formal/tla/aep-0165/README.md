# AEP 0165 finite find-admission model

`FindAdmission.tla` models one current document revision, one query generation,
one pending background scan, one admitted result, bounded match count, and
replacement admission. It is an independent finite abstraction, not a formal
refinement of the Rust implementation or worker transport.

`StartSearch` captures current document and query identity. `ChangeQuery` and
`EditDocument` invalidate admitted results without canceling an already running
scan. `CompleteSearch` publishes only current work. `Replace` requires a current
admitted result. `Cancel` drops pending and admitted ownership.

Pull-request bounds use three document revisions, four query generations, and
four abstract match counts. Nightly bounds use five revisions, seven query
generations, and nine match counts. The model does not represent text bytes,
UTF-8 matching, allocation, worker duration, rendering, keyboard delivery, or
Rust channel behavior.

`FaultyStale.cfg` deliberately admits a completion after its identity becomes
stale. `FaultyReplacement.cfg` deliberately permits replacement from stale
admitted evidence. Each control must violate its named invariant for the model
gate to pass.
