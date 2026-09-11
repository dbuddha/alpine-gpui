# Alpine documentation

Current guidance:

- [Build and run](../README.md#development-and-project-state), including current limitations.
- [Operating guide](../AGENTS.md): focused development, verification and PR acceptance.
- [Architecture](../ARCHITECTURE.md): implemented ownership and subsystem invariants.
- [Studio settings](reference/studio-settings.md).
- [Dogfood capture](quality/studio-dogfood-capture.md), [release profiling](quality/studio-release-profiling.md), and [residency diagnosis](quality/studio-residency.md).
- [Rendering doctrine](concepts/editor-rendering-doctrine.md) and [measurement boundaries](quality/performance.md).
- Rust API documentation and doctests remain part of the build.

Use plain Markdown. Correct existing guidance when behavior changes; a document
is not required for every implementation change. No mdBook or Wiki build,
synchronization or drift audit is required. The remote Wiki is left as history
and is no longer presented as maintained.

## Historical reference

[Research](research/index.md), [case studies](case-studies/README.md),
[AEPs](aep/), [project plans](project/README.md), and the old
[governance operations](operations/) are retained at their existing paths. They
contain dated status and superseded procedures, not current development gates.
Use current code, tests and the operating guide when those procedures conflict.
Technical product contracts and measurement evidence remain in the
[evidence registry](../assurance/evidence.toml).

The pre-cleanup revision is
[`f3c7cfb561b1a250859ea3162d0dff105a5cc61c`](https://github.com/dbuddha/alpine-gpui/tree/f3c7cfb561b1a250859ea3162d0dff105a5cc61c).
It retains removed governance evidence, installation/evaluation tooling and book
configuration. Their removal does not invalidate retained technical measurements.
