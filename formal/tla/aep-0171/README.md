# AEP 0171 file-tree admission model

`FileTreeAdmission.tla` models finite activation, hide, expansion, publication, and selection identities.

Rust event mapping:

- `Activate` maps to `FileTreeState::activate` and root request admission.
- `Expand` maps to directory-row activation and a later bounded request.
- `PublishCurrent` and `DropStale` map to `FileTreeState::admit`.
- `SelectCurrent` maps to file-row activation before workspace path revalidation.
- `Hide` maps to closing the sidebar and cancelling current publication authority.

Pull-request checking uses four generations and nightly checking uses eight. `FaultyStale.cfg` permits stale publication and must violate `PublishedIsCurrent`. `FaultySelection.cfg` permits stale selection and must violate `SelectionIsCurrent`.

The model assumes typed outcomes and finite monotonic generations. It excludes Rust refinement, filesystem and ignore semantics, allocation, runtime scheduling, native input, path bytes, and elapsed time.
