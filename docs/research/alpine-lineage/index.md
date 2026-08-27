# Alpine implementation lineage and evidence

- Research record: [#315](https://github.com/dbuddha/alpine-gpui/issues/315)
- Historical audit baseline: [`de8cd6397adc81632fe1103f1834214ae6ec6a1a`](https://github.com/dbuddha/alpine-gpui/tree/de8cd6397adc81632fe1103f1834214ae6ec6a1a)
- Current reconciliation revision: [`e2564055622dce3a7d1f277d52fc53e34c16e916`](https://github.com/dbuddha/alpine-gpui/tree/e2564055622dce3a7d1f277d52fc53e34c16e916)
- Comparator pin: Zed [`v1.15.0`](https://github.com/zed-industries/zed/tree/e17dc4f9d50db73a458b64dcce50ecd4878b98a3)
- Current-upstream review: Zed [`v1.16.1`](https://github.com/zed-industries/zed/tree/eb8e1c8b5502b7007465fbbc465f4a736fa39210), WGPU [`v30.0.1`](https://github.com/gfx-rs/wgpu/tree/40f4a34ebaf56f9a046231f54125ad046239d3f3), awesome-gpui [`6571693`](https://github.com/zed-industries/awesome-gpui/tree/657169337a19a5b27f9aa7e53811e6f82b7f213c)
- Reviewed: 2026-08-22
- Evidence ceiling: E3 for selected Alpine invariants; E4 has not been reached

## Decision question

What did Alpine independently build, what upstream ideas influenced it, what did
it deliberately modify or reject, and what evidence justifies each correctness,
performance, memory, and product claim?

This package is the canonical answer. It is deliberately more precise than a
case study. A similarity is not a code lineage claim, a bounded design is not a
measured speedup, and an Alpine test is not a comparative qualification.

## Current verdict

Alpine is going in the right architectural direction. It is not a fork of GPUI
or Zed Editor. It is a smaller independent implementation of selected
GPUI-style ideas around demand-driven rendering, immutable frame data, direct
Metal presentation, visible-range text work, and short-lived layout reuse. It
adds stricter value validation, bounded queues and caches, lifecycle generation
identity, structured failure evidence, deterministic CPU oracles, formal model
checks, and a local-only editor boundary.

The project is not yet qualified as faster than GPUI, Zed, or Sublime. PR
[#344](https://github.com/dbuddha/alpine-gpui/pull/344) advances the pinned Zed
lab to composed E3 semantic agreement for the immutable control, clipped grid,
glyph grid, realistic code viewport, scroll, and resize fixture ladder. Atlas
recovery, timing, memory, latency, and E4 dominance remain open under
[#53](https://github.com/dbuddha/alpine-gpui/issues/53). The most important
product defect is visible typing latency in
[#304](https://github.com/dbuddha/alpine-gpui/issues/304). Physical
accessibility qualification, sustained dogfood, residency measurement, Rust
rename and formatting publication, and settings reload and migration remain
open. Bounded document and workspace symbols are implemented under Task #221,
with exact hosted evidence pending the implementation pull request.

## Capability accounting

Line counts cannot answer "how much GPUI did we recreate." GPUI's entity graph,
element system, style engine, executor, platform breadth, and widgets have very
different complexity from Alpine's direct renderer and editor-specific code.
The reproducible unit is an explicitly declared capability family.

The [framework matrix](framework-lineage.md) audits 24 families:

| Classification | Families | Meaning |
| --- | ---: | --- |
| Adapted concept | 8 | A pinned GPUI or Zed mechanism directly informed an independently written Alpine mechanism |
| Independent convergence | 6 | Both systems solve the same platform or editor requirement without evidence of source-level adaptation |
| Alpine-original strengthening | 4 | The audited upstream mechanism has no equivalent Alpine guarantee at the reviewed boundary |
| Rejected or deferred | 6 | Alpine intentionally does not recreate the GPUI capability today |

Therefore, 8 of 24 audited families are concept adaptations, not copied code.
Ten implemented families are independently convergent or Alpine-specific, and
six large GPUI families are absent. This count is a scope map, not a complexity,
quality, or LOC percentage.

The [Studio matrix](studio-lineage.md) audits 24 private daily-driver families.
At the current reconciliation revision, nineteen are implemented for their
selected behavior, three have implementation but incomplete qualification,
publication, or configuration behavior, and two remain incomplete. Twenty-two therefore have
some production implementation. Neither the unweighted inventory nor a
partial-credit score is daily-driver readiness: typing smoothness, data safety,
accessibility, and sustained dogfood are blocking gates whose importance is
greater than a feature count.

## Original audit project snapshot

The counts below are retained as the 2026-08-22 audit snapshot. GitHub Issues,
Milestones, and the Project own live state; this historical package must not be
used as a current burn-up report.

| Milestone | State at review | Evidence-based interpretation |
| --- | --- | --- |
| M0 | 21 closed, 2 open | Governance exists, but parent requirements remain open and should not obscure the product critical path |
| M1 | 8 closed, 4 open | Direct Metal works; realistic GPUI comparison and hardware qualification do not |
| M2 | 9 closed, 6 open | Native presentation is implemented; physical SDR, lifecycle residency, and idle-energy qualification remain |
| M3 | Closed | Local workspace shell was accepted |
| M4 | 10 closed, 8 open | Input and accessibility implementation is substantial; physical VoiceOver and real-process qualification remain |
| M5 | 19 closed, 20 open | Studio is a real editor prototype, but typing latency, Rust feature completion, dogfood, and residency are open |
| M6 | 0 closed, 4 open | Linux and Windows are deferred and are not part of the Apple-first path |
| M7 | 0 closed, 6 open | Public stabilization, claims, signing, and release qualification are future work |

Milestone counts are fetched GitHub state, not earned-value metrics. Parent
issues, research, hardware experiments, and leaf implementation tasks are mixed
inside those counts. Prototype readiness should be read from blocking leaf
gates, not from issue percentages.

## Package map

- [Methodology](methodology.md): classifications, evidence levels, and update workflow.
- [Pinned source map](source-map.md): exact revisions, licenses, boundaries, and source anchors.
- [Framework lineage](framework-lineage.md): Alpine GPUI against Zed GPUI, WGPU, and awesome-gpui.
- [Studio lineage](studio-lineage.md): Alpine Studio against Zed Editor.
- [Evidence ledger](evidence-ledger.md): mechanism-level origin, modification, evidence, and claim status.
- [Historical log](history.md): adoption, correction, and supersession chronology.
- [Adversarial review](adversarial-review.md): defects, wrong-direction risks, and retained strengths.
- [Experiment plan](experiments.md): work required to advance E2 and E3 statements to E4.
- [Alpine decisions](alpine-decisions.md): accepted include, modify, reject, and defer decisions.
- [References](references.bib): machine-readable source bibliography.

## Claim boundary

The strongest supportable statement today is:

> Alpine has a narrower, demand-driven Direct Metal and local-editor
> architecture with deterministic avoided-work and bounded-resource invariants.
> Comparative performance and memory dominance remain unproven.

Do not shorten this to "Alpine is faster than GPUI" or "Alpine runs at 120
FPS." For an editor, idle zero-frame behavior is desirable. The active target
is deadline adherence and low input-to-present latency during matched 120 Hz
typing and scrolling journeys.
