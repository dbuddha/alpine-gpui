# Alpine development

Alpine Studio is a local-only macOS editor built on Alpine GPUI and Direct Metal.
Keep editor correctness, bounded ownership and queues, explicit painter order,
and responsive native interaction. Do not grow a general GPUI clone, network
service, plugin platform or shipping WGPU dependency.

## Start and finish

- Identify the branch, upstream and dirty state; preserve unfinished work and
  parked worktrees. Fetch before comparing remote branches.
- A user request or existing issue supplies scope. State the observable outcome
  and relevant verification before implementation. Read `ARCHITECTURE.md` when
  changing ownership, native or subsystem boundaries.
- Respect authorization already given in the session. Ask only for missing scope
  or a new consequential decision, not repeated approval of authorized work.
- Prefer one focused PR. Keep at most two active implementation changes as a
  working convention. Issues are for deferred defects, blockers or multi-PR work;
  include the problem, observable acceptance and relevant links. Hierarchies,
  milestones, labels and Projects are optional.
- Run relevant local checks once before publication. Repeat after changes or
  failures justify it. Inspect all diffs and untracked files before committing;
  keep behavior and regression evidence together.
- Complete a change through observed behavior, meaningful regression evidence,
  reviewed changes and applicable green CI. Report untested behavior and risks.
  A passing hosted test does not establish physical interaction or performance.

## Commands and checks

```sh
cargo run --locked -p alpine-studio
cargo test --locked -p <affected-crate>
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
scripts/check.sh # complete local gate when the scope warrants it
```

Ordinary CI selects workspace and native checks from source changes. Coverage,
mutation, Kani, Miri and TLA+ are manual assurance options, selected for a concrete
risk. They are not prerequisites for ordinary development. Do not weaken a failed
check or rerun it merely to obtain green; investigate its failure.

## Rust and native pitfalls

- Safe Rust is the default. New unsafe boundaries need explicit approval, a local
  safety argument, focused tests and independent review. Keep native handles,
  callback generations, in-flight resources and teardown ownership explicit.
- Avoid lock or RefCell reentrancy across callbacks and main-thread blocking.
  Reject stale worker results using document/workspace revisions. Preserve Unicode,
  IME, Accessibility, save durability and unsaved documents across lifecycle events.
- Rendering needs semantic/CPU oracles and native readback where applicable.
  Preserve blended painter order; a pixel hash alone is not a cross-GPU oracle.
- Performance claims need matched endpoints, representative workloads and measured
  distributions. GPU completion is not presentation; requested bytes are not
  physical residency. Preserve unfavorable results and state hardware limitations.

## Boundaries and publication

- Obtain approval for new public contracts, dependencies, unsafe boundaries,
  licensing or copied source when not already authorized. Review dependency
  licenses, transitive features, ownership and cost; no shipping Git dependencies.
- Source adaptation needs exact upstream provenance, applicable licensing/notices,
  `provenance.toml` when copying source, and independent tests. Zed application
  source stays in the isolated GPL lab; verify GPUI licensing at the inspected pin.
- Never publish secrets, rewrite published history, bypass branch protection or
  hide failures. Before merge, inspect live protected-branch requirements, resolved
  conversations, mergeability, the tested source SHA and current tested base.
  Require terminal-successful applicable checks and `ci-pass`; use normal protected
  merge and verify the resulting main run. Caller-supplied success flags are no proof.
- PRs explain Problem and outcome, Change, and Verification and remaining risks.
  Include screenshots for visual changes and measurements for performance claims.
- Use plain Markdown and Rust API docs. Update guidance when behavior makes it
  wrong; no document, claim ID or research issue is required for every change.
  `docs/README.md` separates current guidance from historical reference.

## Optional engineering skills

Repository-context discovery uses `.agents/skills`. Load a skill when its domain
is reached, and supporting skills only for the relevant boundary:

- `alpine-studio-gpui-engineer`: implementation and editor acceptance.
- `apple-metal-performance-engineer`: native lifecycle, presentation and residency.
- `zed-gpui-architecture-expert`: pinned source inspection and justified adaptation.
- `algorithmic-performance-engineer`: measured algorithms and data layouts.

The old `skills/` collection and governance documents are historical, not operating
instructions. Skill checks validate packaging, not expertise or merge readiness.
