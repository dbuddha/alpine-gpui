# Alpine research catalog

This is the canonical retrieval surface for durable Alpine GPUI and Alpine
Studio research. GitHub issues own investigation state, decisions, approval,
and implementation status. These documents retain accepted findings and exact
measurement rules so implementation does not depend on chat history.

## Daily-driver decision set

| Artifact | Durable purpose | Live record |
| --- | --- | --- |
| [Alpine Studio adversarial review](alpine-studio-adversarial-review.md) | Keep, change, defer, and execution verdict for the current codebase | [Research #118](https://github.com/dbuddha/alpine-gpui/issues/118) |
| [Alpine Studio daily-driver path](../use-cases/alpine-studio-highfidelity.md) | Accepted product boundary and gate sequence | [Capability #28](https://github.com/dbuddha/alpine-gpui/issues/28) |
| [Comparator protocol v1](../quality/comparator-protocol.md) | Correctness admission, stage separation, identities, sampling, memory, and claim grammar | [Research #115](https://github.com/dbuddha/alpine-gpui/issues/115) |
| Research retention requirement | Queryable evidence and deterministic CI audit | [Requirement #132](https://github.com/dbuddha/alpine-gpui/issues/132) |
| [GitHub Wiki mirror policy](../wiki/README.md) | Revision-pinned, one-way retrieval mirror with mdBook as canonical authority | [Decision #174](https://github.com/dbuddha/alpine-gpui/issues/174), [Task #175](https://github.com/dbuddha/alpine-gpui/issues/175) |

## Comparative case studies

| Comparator | Retained conclusions | Research record |
| --- | --- | --- |
| [Zed stable application](../case-studies/zed-editor.md) | Product architecture, editor behavior, useful patterns, and excluded collaborative weight | [Research #113](https://github.com/dbuddha/alpine-gpui/issues/113) |
| [Zed GPUI and macOS renderer](../case-studies/zed-gpui.md) | Invalidation, render phases, scene organization, batching, caches, atlas ownership, and Metal scheduling | [Research #113](https://github.com/dbuddha/alpine-gpui/issues/113) |
| [Sublime Text local-speed model](../case-studies/sublime-editor.md) | Official public facts, Alpine inferences, and explicitly unknown proprietary internals | [Research #114](https://github.com/dbuddha/alpine-gpui/issues/114) |
| [WGPU case study](../case-studies/wgpu.md) and [deep research package](wgpu/index.md) | Pinned architecture, lifecycle, validation, test, memory, experiment, and non-shipping decisions | [Research #23](https://github.com/dbuddha/alpine-gpui/issues/23), [re-evaluation #99](https://github.com/dbuddha/alpine-gpui/issues/99), [Task #202](https://github.com/dbuddha/alpine-gpui/issues/202) |

## Qualification records

- [Research #115](https://github.com/dbuddha/alpine-gpui/issues/115) owns
  comparator adaptation separation and renderer-only fairness.
- [Research #116](https://github.com/dbuddha/alpine-gpui/issues/116) owns the
  fixed-hardware protocol and evidence-window qualification.
- [Decision #119](https://github.com/dbuddha/alpine-gpui/issues/119) authorizes
  this narrow catalog and its deterministic retention audit.
- [Decision #120](https://github.com/dbuddha/alpine-gpui/issues/120) authorizes
  the bounded asynchronous presentation design derived from the research.
- [Decision #174](https://github.com/dbuddha/alpine-gpui/issues/174) and
  [Task #175](https://github.com/dbuddha/alpine-gpui/issues/175) own the
  mdBook-canonical GitHub Wiki retrieval mirror.

## Retrieval rules

- Start here for accepted research, then follow the linked GitHub issue for
  current state and implementation tasks.
- Treat repository docs and mdBook as authoritative. Wiki pages are generated
  retrieval mirrors and may not contain unique evidence or decisions.
- Treat immutable revision links and official product sources as evidence.
- For substantial research, keep a decision-facing case study plus a package
  containing the pinned source map, detailed findings, experiments, and decision
  ledger. Summary prose without that chain is not deep research.
- Treat Alpine design conclusions as inferences unless implementation evidence
  proves them.
- Never infer private Sublime internals from external timing or memory results.
- Never turn a normalized product comparison into a universal framework claim.
- Preserve workload, environment, exclusion, raw sample, and invalid-run
  identities for every performance statement.
