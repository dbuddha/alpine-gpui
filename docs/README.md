# Documentation

Start with the topic you need:

- [Build and run](development.md).
- [Testing and acceptance](testing.md).
- [Architecture map](../ARCHITECTURE.md), with links to detailed invariants.
- [Settings](reference/studio-settings.md) and [current limitations](reference/limitations.md).
- [Debugging](debugging/README.md).
- [Research findings](research/index.md), only when a question needs source comparison.

Use plain Markdown and Rust API docs. Correct guidance when behavior changes it;
batch new explanations after a feature settles or before a release. There is no
book, Wiki, documentation skill or mandatory per-PR documentation workflow.

Technical [AEPs](aep/) and [case studies](case-studies/README.md) remain references
because existing tests and evidence cite them. They are not default onboarding.
The [evidence registry](../assurance/evidence.toml) retains technical measurement
and verification contracts. Preserve their references when reorganizing material.

Obsolete project/governance guides and Wiki templates are retained in
[pre-cleanup Git history](https://github.com/dbuddha/alpine-gpui/tree/da69bd30cbfde922ca7e8966bfb63eee2d65a7cf/docs).
The retired remote Wiki snapshot is `aeab9e09ffea95c4080dd1459131b9e4c1d064e8`;
its content was generated from repository sources at `93df44b`.
