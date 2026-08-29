# AEP 0120 bounded asynchronous frame-slot model

`FrameSlots.tla` models exactly three reusable ownership slots. Each slot is
free, encoding, or submitted and carries a monotonic lease sequence plus owner
generation, revision, and surface epoch. It covers saturation, completion
reordering, current and stale publication, precommit cancellation, and shutdown
drain.

The formal saturation value is a one-bit witness. Repeated saturated admission
attempts change no guard or checked temporal property after the first witness,
so retaining their count would add only stutter-equivalent history. Exact
saturation arithmetic remains owned by the compiled `FrameSlotRing` tests.
Publication identities are retained only for `Current`; `None` and `Rejected`
canonicalize them to zero because no transition or checked property consumes
inactive identity history. `InactivePublicationHasNoIdentity` enforces that
quotient explicitly.

The pull-request model admits three leases and one revision, epoch, and
generation advance. The nightly model admits four leases and two advances. Weak
fairness applies to submit, terminal release, and encoding cancellation for each
slot. Native command execution, elapsed time, memory bytes, Objective-C retain
counts, presentation timestamps, and Rust refinement are excluded.

The driver uses TLC's default periodic liveness checks for the smaller
pull-request graph and `-lncheck final` for the expanded nightly graph. Nightly
therefore constructs the same complete unsymmetrized state graph and evaluates
the same temporal properties once after exploration, instead of repeatedly
rescanning a partial graph. Slot symmetry is intentionally not enabled because
TLC symmetry reduction is not a sound basis for liveness qualification.

`Faulty.cfg` enables a stale completion that publishes as current.
`CurrentPublicationIsCurrent` must fail. The compiled companion is
`alpine_platform::FrameSlotRing`, covered by unit, bounded-sequence, public
integration, and Kani evidence.
