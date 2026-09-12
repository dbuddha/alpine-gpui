# Testing and acceptance

Choose the narrowest existing test that exercises production behavior. For a bug,
record the reproduction, verify that the regression test exposes the bug, then
verify the fix. Reuse editor action helpers and existing native harnesses before
introducing a new test framework.

| Change | Relevant evidence |
| --- | --- |
| Pure Rust or ordinary bug fix | Affected unit/integration tests and failure paths |
| Editor behavior | Production actions plus document, selection, focus or undo assertions |
| Native lifecycle or rendering | Required native executable and semantic/readback checks |
| Visual change | Before/after images and affected input/focus/Accessibility checks |
| Performance claim | Comparable workloads, source/hardware identities and distributions |
| Documentation | Changed commands/examples and local links |

```sh
cargo test --locked -p <affected-crate>
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
scripts/check.sh
```

Run relevant local verification once before publication, and repeat only after
changes or failures justify it. The full script also exercises technical tooling,
workspace tests, native-compatible tests, doctests and rustdoc. Review its output;
expected negative fixtures are not failing checks.

The full script compiles the native Studio harness on Apple Silicon but does not
execute its native journeys. Ordinary Cargo tests leave these targets disabled.
Run native acceptance explicitly:

```sh
scripts/check-native.sh physical shipping
scripts/check-native.sh physical all
scripts/check-native.sh hosted all
```

`shipping` exercises the real executable; `all` also runs Studio, macOS platform,
Metal and runtime tests with native validation enabled. `hosted` selects the
hosted-direct evidence contract, not physical acceptance. The command retains
output under `target/native-acceptance/`, propagates failures and requires the
Studio harness's completion receipt. A missing receipt is a failure, not a pass.
Use `physical` on an undisturbed target Mac. These commands do not claim release
performance or replace a physical keyboard/VoiceOver check.

## Hosted acceptance

Ordinary CI skips code builds only for changes confined to Markdown under `docs/`
and the named root guidance files. Policy still runs; mixed, unknown and empty
change sets retain code checks, as do explicit manual assurance requests.
CI selects native/portable checks from source changes and fails `ci-pass`
if an applicable job fails or is unexpectedly skipped. PR opening, source updates
and reopening trigger code CI; title/body/label edits do not. Retain the tested
source and base identities, native execution and failure artifacts. The current
runtime target is at most 15 minutes including queueing, without reruns to obtain green.

Kani, Miri, TLA+, mutation and coverage are manual assurance choices for a concrete
risk. Do not substitute them for behavioral tests or require them for every PR.
A failed check requires investigation, not a weaker threshold or blind rerun.

## Native and reproducible tests

AppKit tests use main-thread executables, including `harness = false` targets.
Any runner change must preserve their explicit execution, names and failure
propagation. Headless simulation cannot establish physical presentation, VoiceOver
or energy. See the relevant [debugging procedures](debugging/README.md).

For concurrency investigations, retain the failing input/order and source revision.
Prefer explicit worker completion and cancellation controls over wall-clock sleeps.
A new randomized test must print a replayable seed; keep a discovered failing case
as a regression. Existing bounded stress checks remain enabled. Introduce a general
scheduler or new runner only when a measured gap justifies it.

## Review checklist

- Review the complete diff.
- Exercise changed behavior and relevant failure paths.
- Address or explain review findings.
- Require applicable green checks for the current source before protected merge.

Review is judgment, not a phrase-presence test. Copilot may provide independent
feedback, but its absence or approval is not evidence that native behavior passed.
