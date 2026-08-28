# AEP 0137 bounded runtime handoff model

`RuntimeHandoff.tla` models fixed request and result capacities, standard worker
ownership, workspace and document revision tags, request saturation,
backpressured completion, panic containment, current application, stale
rejection, and shutdown drain. Jobs move through unused, queued, running,
result, and terminal states. A running job remains owned while result capacity
is unavailable and can complete only after the foreground frees a slot. Worker
capacity bounds these retained completions, and strong fairness on completion
checks that every backpressured running job eventually leaves that state.

The pull-request model admits three jobs with one workspace and document
advance, one worker, and one slot in each queue. The nightly model admits four
jobs, two revision advances, two workers, and two slots in each queue. Strong
fairness applies to capacity-gated completion; weak fairness applies to worker
start, foreground result resolution, and shutdown cancellation.
Threads, mutex implementation details, elapsed time, native wake delivery,
application data, and Rust refinement are excluded.

`Faulty.cfg` enables an action that marks a stale result current.
`CurrentApplicationIsCurrent` must fail. The compiled companion is
`alpine_runtime::Application`, covered by bounded-channel, stale-result,
panic-containment, dirty-frame, mutation, Miri, and native transport evidence.
