# Provenance Ledger

This ledger records source incorporated into Alpine GPUI. Architectural research
that does not copy source belongs in `upstream-analysis.md`.

Zed GPUI is Alpine's primary conceptual influence. That acknowledgement records
lineage, not source incorporation. Exact research inputs and permitted influence
modes are maintained in `source-map.md`.

## Incorporated source

None.

The initial workspace was written independently and has no vendored or copied
GPUI implementation code.

## Required entry format

Each future entry must include:

- destination file and symbol;
- upstream repository, file, and immutable commit;
- original license;
- reason incorporation is preferable to a clean implementation;
- modifications made;
- tests covering the incorporated behavior;
- reviewer and date.
