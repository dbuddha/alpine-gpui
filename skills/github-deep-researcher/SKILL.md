---
name: github-deep-researcher
description: Produce decision-grade, source-pinned, reproducible software and agentic-systems research in GitHub. Use for deep research, architecture case studies, comparator analysis, papers and source-code synthesis, performance investigations, agent evaluations, provenance review, or converting research into requirements and experiments.
---

# GitHub Deep Researcher

Deep research is not a long summary. It is a traversable argument from a decision question through primary evidence, contradictory evidence, reproducible observations, bounded findings, and explicit project consequences.

## Research contract

1. State decision question, owner, scope, exclusions, review trigger, and evidence that could change the decision.
2. Select the empirical method before collecting sources.
3. Build a source map with stable identity, authority, relevance, license, and limits.
4. Prefer exact code revisions, specifications, official documentation, papers, issue or PR discussions, benchmark definitions, and raw data.
5. Triangulate consequential claims across independent evidence where possible.
6. Label statements as observed fact, source claim, Alpine inference, hypothesis, or measured result.
7. Seek disconfirming evidence and document contradictions.
8. Reproduce important behavior or measurement when the decision depends on it.
9. Convert findings into include, exclude, experiment, defer, or reject decisions.
10. Link decisions to requirements and preserve raw evidence identities.

Read [the evidence standard](references/evidence-standard.md) before a case study or comparator claim.

## Evidence levels

- `E0 Pointer`: discoverable source, not deeply inspected.
- `E1 Primary`: exact authoritative source and revision with bounded extraction.
- `E2 Triangulated`: multiple sources, contradiction analysis, and explicit inference.
- `E3 Reproduced`: Alpine-controlled experiment with environment and raw evidence.
- `E4 Qualified`: independent replication or fixed-protocol qualification sufficient for a scoped claim.

Architecture adoption needs E2. Performance design claims need E3. Dominance claims need E4. A paper's claim does not become Alpine's measured result.

## GitHub retention

Use a research issue for live question, owner, status, review, disposition, and implementing links. Store durable narrative in versioned Markdown. Store raw samples, manifests, checksums, traces, and reports under accepted assurance policy or immutable release assets when size requires it.

For substantial topics, use a frontmatter summary and retrieval page plus separate source map, findings, experiments, decisions, and bibliography. Read [the package schema](references/package-schema.md). If policy restricts research paths, change policy through an approved task first.

## Agentic systems

Preserve model and provider revision, harness and scaffold revision, system and task prompts, tools and permissions, repository and benchmark identity, environment lock, trials and seeds, trajectories, grader, human intervention, cost, latency, tokens, failure taxonomy, contamination controls, and raw outputs.

Read [agentic-systems research](references/agentic-systems.md) before evaluating coding agents or autonomous workflows.

## Source-code case study

Pin commit or release. Map module ownership, public contracts, lifecycle, data layout, scheduling, concurrency, rendering or I/O, resource accounting, caches, error model, tests, and performance evidence. Link stable line-level source. Distinguish code observation from author intent.

For WGPU or Zed, inspect pinned source and experiments. A feature list or architecture slogan is E0 or E1, not a deep case study.

## Stop conditions

Stop rather than inflate confidence when sources cannot be pinned, licensing blocks use, workloads differ, raw evidence is absent, environments drift, contradictions remain, reproduction fails, samples are underpowered, or claims exceed the method.

## Completion

Provide the decision answer, evidence level, supporting and disconfirming evidence, source and environment identities, limitations, threats to validity, Alpine decisions, implementing issues, review trigger, and artifact locations.
