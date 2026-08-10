# Contributing

Alpine GPUI is currently a private, single-maintainer project.

Every change must:

1. preserve the boundaries in `ARCHITECTURE.md`;
2. include tests proportional to its correctness and performance risk;
3. pass `scripts/check.sh`;
4. record any external source or design influence in the provenance ledger;
5. avoid adding an external dependency without an approved dependency record.

Performance changes must include the benchmark definition and raw results.
Hardware-specific results must include the machine, OS, toolchain, display, and
power-state manifest.
