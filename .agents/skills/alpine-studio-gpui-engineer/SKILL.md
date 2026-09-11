---
name: alpine-studio-gpui-engineer
description: Deliver Alpine Studio and Alpine GPUI through bounded Rust changes, production editor semantics, measured performance, and evidence-backed macOS acceptance.
---

# Alpine Studio and GPUI engineering

Use for ordinary Alpine implementation, editor defects and scoped acceptance.
Start from the user request or issue, affected production code and a reproducible
outcome. Read the repository operating guide; use historical plans only as context.

## Choose the next useful change

- Keep two independent outcomes: a safe, smooth, bounded private daily driver;
  and correctness-equivalent scoped renderer/product advantages. Neither is
  proved by implementation percentages. Do not hold offscreen work behind an
  unrelated onscreen timestamp or Accessibility gate.
- Choose one bounded change and state its failure, hypothesis, regression check
  and observable outcome. Tooling must remove a concrete execution or evidence gap.
- Prioritize correctness, responsiveness, memory, then delivery. Keep Direct
  Metal, bounded ownership and queues, local-only scope, and explicit scene
  ordering. No shipping GPUI/WGPU, game engine, plugin or network-service scope.
- A sub-millisecond mutation stage with tens of milliseconds to presentation
  does not justify rewriting the rope, renderer or reactive runtime. Route
  timestamp validity and scheduling to `$apple-metal-performance-engineer`.
- Route source translation to `$zed-gpui-architecture-expert`, and measured
  foreground scaling or allocation work to `$algorithmic-performance-engineer`.
  Load only the supporting skill the current question requires.

## Editor correctness before polish

Own document/workspace/focus revisions explicitly across bounded worker results.
Preserve unsaved documents across local LSP switches; test cross-file visibility,
version ordering, cancellation and restart, not just a message-count mock.
Test multiline lexical invalidation, grapheme and UTF-16 boundaries, atomic-save
failure, external changes, restore corruption, quit/cancel, IME and AX text
ranges. Use disposable fixtures and only capture-owned process cleanup.
Select only test families affected by the change and its risks, unless repository
mandatory gates require more; this list is not a blanket suite for every task.

Prefer a small private-module extraction needed by the fix over a speculative
entity graph or element framework. Review Rust ownership, drop order, checked
arithmetic, allocation failure, lock/RefCell reentrancy across native callbacks,
and main-thread blocking. New public APIs, dependencies and unsafe boundaries need approval unless already
authorized in the session, with the rationale and risks in the PR.

## Acceptance and reporting

Use the existing harness and exact artifact identities, not reconstructed ad hoc
commands. Discover required tests and reject zero selection. Pair production-path
tests with discriminating negative controls; connect Kani/TLA+ properties to real
events and assumptions rather than claiming they verify all native behavior.
Hosted tests, physical interaction, residency and dogfood have different ceilings.

Report implemented, reproduced, calibrated, optimized, qualified and product
accepted separately. End a change with the artifact, observed result, omissions
and next leaf. Keep outputs proportional to scope: a small task needs a bounded
answer and relevant evidence, not a new general plan or unrelated changes.
For CI work, inspect selected jobs and exact source/base identities; distinguish
queue time, execution and retries. Preserve native execution and aggregate failure
propagation. Specialized assurance is opt-in and does not substitute for behavioral
tests. Review the diff for missed failure paths and misleading acceptance claims.
Before merging, verify live branch protection and terminal required checks for the
tested source and base. After merge, inspect the actual main run.
