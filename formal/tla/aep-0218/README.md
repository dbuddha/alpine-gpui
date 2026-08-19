# AEP 0218 completion-admission model

`CompletionAdmission.tla` models finite open, request, supersession, current
response admission, cancelled-response drop, application, focus loss, identity
change, and close behavior.

Rust event mapping:

- `Open` maps to creating the active Rust language session.
- `Trigger` maps to `RustDiagnostics::request_completion`.
- `ChangeIdentity` maps to a document, buffer, selection, or process identity
  change before response admission.
- `CompleteCurrent` maps to current completion response admission.
- `DropCancelled` maps to local rejection of a late cancelled response.
- `ApplyCurrent` maps to `take_selected_completion` before the editor
  transaction.
- `FocusLoss` maps to the Studio focus event cancellation path.
- `Close` maps to the production `RustDiagnostics::shutdown` path invoked by
  `StudioApp::drop`.

Pull-request checking uses four identities, four request IDs, and eight items.
Nightly checking doubles those bounds. `FaultyLate.cfg` publishes a cancelled
request and must violate `PublishedIsCurrent`. `FaultyApply.cfg` applies an edit
for an older identity and must violate `ApplyRequiresCurrent`.

The model assumes one foreground completion owner, monotonic finite identities
and request IDs, and typed process outcomes. It excludes Rust refinement,
UTF-16 and byte conversion, JSON parsing, process scheduling, allocation,
native event delivery, and elapsed time. Those behaviors remain covered by the
mapped Rust, process, Studio, and native tests.
