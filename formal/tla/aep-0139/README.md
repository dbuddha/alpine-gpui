# AEP 0139 checked local text-buffer model

`LocalTextBuffer.tla` models accepted and rejected local transactions, bounded
undo and redo stacks, monotonic live revisions, external disk divergence,
accepted saves, and conflict-preserving save rejection. Text and disk bytes are
represented by small finite identities because the model owns publication and
history state, not rope, Unicode, or filesystem implementation.

The pull-request model explores three content identities, four attempts and
revisions, and two retained entries per history direction. The nightly model
increases those bounds. Threads, allocators, Ropey internals, byte conversion,
filesystems, elapsed time, and Rust refinement are excluded.

`Faulty.cfg` permits a rejected transaction to publish candidate content and a
conflicting save to overwrite external disk identity. `RejectedIsAtomic` or
`ConflictPreservesAcceptedDisk` must fail. Compiled companions are the
transaction, selection, differential-oracle, bounded-history, and real-file
tests in `alpine-text`.
