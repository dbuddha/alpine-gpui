# AEP 0168 quick-open admission model

`QuickOpenAdmission.tla` models the finite foreground identity rules for lazy inventory publication, query publication, close, and selection.

Rust event mapping:

- `OpenQuick` maps to `QuickOpenState::open` and the first inventory request.
- `PublishCurrentInventory` and `DropStaleInventory` map to inventory output admission.
- `ChangeQuery` maps to committed IME text or deletion.
- `PublishCurrentQuery` and `DropStaleQuery` map to query output admission.
- `SelectCurrent` maps to Enter followed by recursive path revalidation.

Pull-request checking uses four generations. Nightly checking uses eight. `FaultyStale.cfg` permits stale inventory publication and must violate `PublishedInventoryIsCurrent`. `FaultySelection.cfg` permits selection from a non-current query and must violate `SelectionUsesCurrentQuery`.

The model assumes typed worker outcomes and finite monotonic generations. It excludes Rust refinement, ignore parsing, filesystem I/O, allocation, runtime queue scheduling, path bytes, native events, and elapsed time.
