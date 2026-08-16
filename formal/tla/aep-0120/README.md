# AEP 0120 bounded asynchronous frame-slot model

`FrameSlots.tla` models exactly three reusable ownership slots. Each slot is
free, encoding, or submitted and carries a monotonic lease sequence plus owner
generation, revision, and surface epoch. It covers saturation, completion
reordering, current and stale publication, precommit cancellation, and shutdown
drain.

The pull-request model admits three leases and one revision, epoch, and
generation advance. The nightly model admits four leases and two advances. Weak
fairness applies to submit, terminal release, and encoding cancellation for each
slot. Native command execution, elapsed time, memory bytes, Objective-C retain
counts, presentation timestamps, and Rust refinement are excluded.

`Faulty.cfg` enables a stale completion that publishes as current.
`CurrentPublicationIsCurrent` must fail. The compiled companion is
`alpine_platform::FrameSlotRing`, covered by unit, bounded-sequence, public
integration, and Kani evidence.
