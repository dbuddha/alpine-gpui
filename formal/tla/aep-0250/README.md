# AEP 0250 accessibility-transport model

`AccessibilityTransport.tla` models one foreground accessibility owner, exact
revision-tagged requests, one synchronous response slot, current selection
mutation, revision change, close, and reopen.

Rust event mapping:

- `Issue` maps to constructing one validated `AccessibilityRequest`.
- `ChangeRevision` maps to a document or buffer identity change.
- `Respond` maps to `AppContext::respond_accessibility` during event dispatch.
- `ApplyCurrent` maps to Studio's checked selection action.
- `Close` maps to revoking event dispatch and dropping the response slot.
- `Reopen` represents a new owned single-window runtime generation.

Pull-request checking uses four revisions and four requests. Nightly checking
doubles both finite bounds. `FaultyStale.cfg` mutates with a stale revision and
must violate `MutationIsCurrent`. `FaultyDuplicate.cfg` installs a second
response and must violate `AtMostOneResponse`.

The model assumes one synchronous main-thread event owner, finite monotonic
identities, and typed request construction. It excludes Rust refinement, UTF-16
and rope implementation, allocation, AppKit reentrancy, elapsed time, native
element caching, and VoiceOver behavior. Those remain covered by mapped Rust
tests and Tasks #252 and #253.
