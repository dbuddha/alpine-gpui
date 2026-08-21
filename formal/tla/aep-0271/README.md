# AEP 0271 accessibility-notification model

`AccessibilityNotifications.tla` models current native element instances,
removed or replaced instances retained for destruction posting, adapter borrows,
ordinary notifications, handler revocation, and final drain.

Rust event mapping:

- `Remove` maps to cache reconciliation removing one semantic identity.
- `Replace` maps to semantic replacement creating a new native instance while
  retiring the old instance with the same semantic slot.
- `BeginRevoke` maps to invalidating all current identities before close posting.
- `ReleaseBorrow` maps to returning the native dispatch batch from the adapter.
- `PostDestroyed` maps to one synchronous AppKit destroyed-element post.
- `PostOrdinary` maps to layout, focus, selection, value, and announcement posts.
- `RevokeHandler` maps to dropping the application callback after destruction.

Pull-request checking uses three semantic instances and nightly checking uses
five. `FaultyBorrow.cfg` posts while the adapter borrow is held,
`FaultyOrdinary.cfg` posts ordinary state before destruction drains, and
`FaultyRevoke.cfg` revokes early and attempts a later post. Every faulty control
must violate its mapped invariant.

The model assumes one serialized AppKit owner, finite native instance identities,
and synchronous post calls. It excludes Rust refinement, Objective-C allocation,
external assistive-client delivery, announcement speech, and elapsed time. The
production native fixture, mutation, coverage, and Task #273 physical observer
evidence cover those separate boundaries.
