# AEP 0160 finite workspace-selection model

`WorkspaceSelection.tla` models one current document, clean or dirty state, one
valid file replacement, rejected selection, and monotonic process-local document
identity. It is an independent finite abstraction, not a formal refinement of
the Rust implementation or filesystem.

`EditDocument` and `SaveDocument` map to Studio edit and atomic-save outcomes.
`OpenValidFile` maps to successful `open_workspace_entry` publication.
`RejectSelection` maps to dirty-document, invalid UTF-8, directory, missing,
replacement, and canonical-root rejection before current-document mutation.

Pull-request bounds use four document identities and four abstract byte states.
Nightly bounds use nine of each. Weak fairness requires an enabled save and an
enabled valid open eventually to occur. It does not model elapsed time,
filesystem operations, canonical path bytes, UTF-8 decoding, allocation,
rendering, native event delivery, or background workers.

`FaultyFailedSelection.cfg` deliberately mutates bytes and identity after a
rejected selection. `FaultyReplacement.cfg` deliberately changes the active
file and bytes without advancing document identity. Each control must violate
its named invariant for the model gate to pass.
