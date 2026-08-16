# AEP 0153 formal model

`ClipboardCloseResponse.tla` models the portable event-response cardinality and
close decision added by AEP 0153. `PullRequest.cfg` and `Nightly.cfg` must
satisfy the bounded clipboard, cancelled-close liveness, allowed-close
shutdown, matching-cut, dirty-close, and close-resolution properties.
`Faulty.cfg` deliberately closes a cancelled request, `FaultyCut.cfg` mutates a
stale cut, and `FaultyDirtyClose.cfg` admits dirty close. Each must violate its
target invariant.

This model does not represent AppKit pasteboard conversion, native callback
reentrancy, text bytes, or process teardown. Revision and selection identities
are finite booleans, and one successful cut removes one abstract byte. Task
#154 must retain native evidence for concrete platform boundaries.
