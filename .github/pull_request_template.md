## Summary

What changes, in one concise paragraph?

## Closing issue

Closes #

## Parent journey

Link the parent journey, or write `None` for foundation work.

## Decision or research

Link accepted decisions and research required by this change, or write `None`.

## Acceptance evidence

Map the linked issue's observable acceptance criteria to tests, artifacts,
screenshots, traces, or benchmark results.

## Risk and scope

Describe affected ownership boundaries, failure modes, explicitly excluded work,
and remaining risk.

## Test plan

List exact commands, CI jobs, and hardware-backed checks that passed. Explain any
required gate that did not run.

## Performance and memory

Record measured impact for hot-path changes, or explain why this change cannot
affect latency, throughput, allocations, idle work, memory, or binary size.

## Release impact

Name exactly one applied label: `release:breaking`, `release:feature`,
`release:fix`, or `release:none`.

## Dependencies, provenance, and unsafe code

List dependency changes, external influence or source incorporation, and unsafe
code changes. Write `None` when the section does not apply.

## Adversarial review

Record the independent or agent-assisted challenge pass, the strongest failure
hypotheses considered, and how each was resolved or accepted.

## Checklist

- [ ] The pull request has one coherent scope and no unrelated cleanup.
- [ ] The linked issue and parent chain define observable acceptance.
- [ ] New behavior has tests at the narrowest useful layer.
- [ ] Architecture changes update `ARCHITECTURE.md` and link an accepted decision.
- [ ] `scripts/check.sh` passes locally, or the exception is documented above.
- [ ] Exact CI and relevant hardware results are recorded.
- [ ] Dependency, provenance, unsafe, and release-impact policy is satisfied.
