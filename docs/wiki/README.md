# GitHub Wiki mirror

Issue [#175](https://github.com/dbuddha/alpine-gpui/issues/175) implements
Decision [#174](https://github.com/dbuddha/alpine-gpui/issues/174).

The repository and its mdBook documentation are authoritative. The GitHub Wiki
is a bounded, one-way retrieval mirror. It must not contain unique research,
requirements, decisions, qualification evidence, or implementation guidance.

## Publication contract

- Every Wiki page has a source template under `docs/wiki/pages`.
- `manifest.tsv` records the page title, canonical mdBook source, and tracking
  issues.
- Generated pages identify the exact 40-character `main` revision they mirror.
- Publication is allowed only from a clean checkout whose `HEAD` is exactly
  `origin/main`.
- The publisher refuses pages outside the approved manifest and never commits
  or pushes on behalf of its caller.
- WGPU remains a research source, differential oracle candidate, and possible
  later portability backend. This mirror does not authorize WGPU as a shipping
  dependency or replace Alpine's direct Metal v1 path.

Run `scripts/check-wiki.sh` to validate sources, `scripts/test-wiki.sh` to test
the pipeline, and `scripts/wiki.sh render REVISION OUTPUT` to render locally.

Local checks do not prove the live Wiki is current. From clean current `main`,
run `scripts/wiki.sh audit-remote /path/to/alpine-gpui-wiki` to fetch and compare
the live remote without committing or pushing. Run this after source changes,
publication, failed publication, checkout migration, and before reporting Wiki
freshness. Network failure leaves live state unknown and does not weaken offline
repository checks.
