# AEP 0137 bounded runtime handoff model

`RuntimeHandoff.tla` models fixed request and result capacities, standard worker
ownership, workspace and document revision tags, request saturation, result
omission, panic containment, current application, stale rejection, and shutdown
drain. Jobs move through unused, queued, running, result, and terminal states.

The pull-request model admits three jobs with one workspace and document
advance and one slot in each queue. The nightly model admits four jobs, two
revision advances, and two slots in each queue. Weak fairness applies to worker
start and completion, foreground result resolution, and shutdown cancellation.
Threads, mutex implementation details, elapsed time, native wake delivery,
application data, and Rust refinement are excluded.

`Faulty.cfg` enables an action that marks a stale result current.
`CurrentApplicationIsCurrent` must fail. The compiled companion is
`alpine_runtime::Application`, covered by bounded-channel, stale-result,
panic-containment, dirty-frame, mutation, Miri, and native transport evidence.
