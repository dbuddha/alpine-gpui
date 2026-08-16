# AEP 0153 formal model

`ClipboardCloseResponse.tla` models the portable event-response cardinality and
close decision added by AEP 0153. `PullRequest.cfg` and `Nightly.cfg` must
satisfy the bounded clipboard, cancelled-close liveness, allowed-close
shutdown, and close-resolution properties. `Faulty.cfg` deliberately closes a
cancelled request and must fail `CancelledCloseStaysLive`.

This model does not represent AppKit pasteboard conversion, native callback
reentrancy, two-phase cut completion, or process teardown. Task #154 must add
native evidence for those boundaries.
