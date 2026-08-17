# AEP 0177 command-palette model

`CommandPalette.tla` models finite open, query, selection, availability,
execution, cancellation, and release transitions.

Rust event mapping:

- `Open` maps to `CommandPalette::open`.
- `ChangeQuery` maps to committed text and backward deletion.
- `MoveSelection` maps to bounded up and down navigation.
- `AvailabilityChange` maps to execution-time `CommandContext` refresh.
- `ExecuteCurrent` maps to `CommandPalette::execute_selected`.
- `Cancel` maps to `CommandPalette::cancel`.

Pull-request checking uses four generations and eight commands. Nightly checking
uses eight generations. `FaultyCancel.cfg` retains selection after close and
must violate `ClosedOwnsNoSelection`. `FaultyExecute.cfg` executes an identity
from the prior query generation and must violate `ExecutedWasCurrent`.

The model assumes a non-empty statically bounded registry and typed foreground
events. It excludes Rust refinement, string matching, allocation, rendering,
native input delivery, command side effects, and elapsed time.
