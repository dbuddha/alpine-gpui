# Alpine engineering

Alpine Studio is a local-only Apple Silicon editor built on Alpine GPUI and
Direct Metal. Keep changes focused; do not grow a general GPUI clone, network
service or plugin platform.

## Working rules

- Inspect branch, upstream and dirty state. Preserve unfinished work and parked
  worktrees. Fetch before comparing remote branches.
- The user request or an existing issue supplies scope. State the observable
  outcome and relevant verification. Honor authorization already given.
- Read affected code and tests first. Use the short `ARCHITECTURE.md` map when
  needed, then only relevant topic references. Do not bulk-read docs or research.
- Follow `CONTRIBUTING.md` for publication and acceptance. Verify relevant local
  behavior once; repeat when changes or failures justify it. Review the full diff.
- Routine authorized work can proceed through implementation, independent review
  and protected merge. Ask about a new public contract, dependency, unsafe boundary,
  license, copied source or acceptance safeguard when not already authorized.
- Never publish secrets, overwrite unrelated work, rewrite published history,
  bypass branch protection or weaken a check merely to obtain green.
- Use live required checks for the tested source and current base before merging;
  verify the resulting main run. AI approval is not execution evidence.

## Concrete pitfalls

- Reject stale worker results with document/workspace revisions. Preserve Unicode,
  IME, Accessibility, unsaved documents and save durability across lifecycle events.
- Avoid lock/RefCell reentrancy across native callbacks and main-thread blocking.
  Native handles, callback generations, in-flight resources and teardown need
  explicit ownership. Unsafe code needs a local safety argument and focused tests.
- Preserve blended painter order. Native rendering needs semantic/readback checks;
  a cross-GPU pixel hash alone is insufficient.
- GPU completion is not presentation. Requested bytes are not physical residency.
  Performance claims need matched endpoints, workloads and measured distributions.
- Review dependency licenses, features and ownership; no shipping Git dependencies.
  Source copying needs exact provenance, applicable notices, approval and tests.
  Zed application source stays in the isolated GPL lab.

## Useful commands

```sh
cargo run --locked -p alpine-studio
cargo test --locked -p <affected-crate>
cargo fmt --all -- --check
scripts/check.sh # full local gate when warranted by scope
```

## On-demand context

Four optional skills live in `.agents/skills`: Alpine engineering for editor
implementation, Apple Metal for lifecycle/presentation/residency, Zed architecture
for pinned source comparison, and algorithmic performance for measured algorithms.
Load support only when the task reaches its domain. Documentation needs no skill.
`docs/AGENTS.md` applies only to documentation work.

Add standing rules only for recurring, non-obvious, actionable mistakes. Keep
research conclusions in topic notes and implementation detail near its code.
