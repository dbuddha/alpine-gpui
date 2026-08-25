# AEP 0221 symbol-admission model

`SymbolAdmission.tla` models finite open, request, query supersession, identity
change, current response admission, selection, checked navigation, focus loss,
and close behavior.

Rust event mapping:

- `Open` maps to creating the active Rust language session.
- `Trigger` maps to `RustDiagnostics::open_symbols` and request submission.
- `ChangeQuery` maps to symbol query or IME commitment and request cancellation.
- `ChangeIdentity` maps to workspace, document, process, or revision change.
- `CompleteCurrent` maps to current symbol response admission.
- `SelectNext` maps to bounded keyboard picker navigation.
- `Navigate` maps to checked local `apply_selected_symbol` navigation.
- `FocusLoss` maps to production focus cancellation.
- `Close` maps to `RustDiagnostics::shutdown` from `StudioApp::drop`.

Pull-request checking uses four identities, requests, and query revisions with
eight items. Nightly checking doubles those bounds. `FaultyPublish.cfg`
publishes an older identity and must violate `PublishedIsCurrent`.
`FaultyNavigate.cfg` navigates an older identity and must violate
`NavigationRequiresCurrent`.

The model assumes one foreground symbol owner, finite monotonic identities,
and parser output within the implementation ceiling. It excludes Rust
refinement, JSON parsing, UTF-16 conversion, filesystem identity, allocation,
process scheduling, native event delivery, and elapsed time. Those behaviors
remain covered by mapped parser, process, Studio, accessibility, and local-path
tests.
