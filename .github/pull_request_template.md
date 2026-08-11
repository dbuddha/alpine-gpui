## Summary

What changes, in one concise paragraph?

## Context

Why is this change needed now?

## Decision or root cause

For a feature or architecture change, record the decision and alternatives.
For a bug fix, record the root cause.

## Evidence

Link specifications, ADRs, research notes, benchmark output, traces, screenshots,
or issue reproduction as applicable.

## Risk and scope

Describe affected ownership boundaries, compatibility risk, and explicitly
excluded work.

## Test plan

List the exact commands and hardware-backed checks that passed. Explain any
required gate that did not run.

## Performance and memory

Record benchmark impact for hot-path changes, or explain why the change cannot
affect performance, allocations, idle work, or binary size.

## Dependencies, provenance, and unsafe code

List dependency changes, external source influence or incorporation, and unsafe
code changes. Write `None` when the section does not apply.

## Change record

Name the fragment under `changes/`, or explain why this is an internal-only
change that does not require one.

## Checklist

- [ ] This PR has one coherent scope and no unrelated cleanup.
- [ ] I read the nearest applicable `AGENTS.md` files.
- [ ] Architecture and behavior changes have durable specifications or ADRs.
- [ ] New behavior has tests at the narrowest useful layer.
- [ ] `scripts/check.sh` passes locally, or the exception is documented above.
- [ ] Shipping changes include a valid change fragment.
- [ ] Dependency additions received owner approval and are recorded.
- [ ] External source incorporation has license and symbol-level provenance.
- [ ] Unsafe code has a documented safety contract and focused tests.
