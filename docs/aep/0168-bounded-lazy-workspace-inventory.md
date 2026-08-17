# AEP 0168: Bounded lazy workspace inventory and quick open

- Status: accepted 2026-08-17
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Requirement: [#32](https://github.com/dbuddha/alpine-gpui/issues/32)
- Parent task: [#127](https://github.com/dbuddha/alpine-gpui/issues/127)
- Implementation task: [#168](https://github.com/dbuddha/alpine-gpui/issues/168)
- Decision and dependency record: [#169](https://github.com/dbuddha/alpine-gpui/issues/169)
- Research: [#117](https://github.com/dbuddha/alpine-gpui/issues/117)

## Motivation and selected journey

The top-level workspace tree is intentionally cheap but cannot navigate a real repository. The next daily-driver slice must find nested files without moving recursive I/O onto launch, retaining unbounded paths, depending on machine-global ignore state, or blocking input while ranking a large inventory.

Command-P is the sole admission point. It opens a bounded overlay and submits a serial project-local inventory to the existing runtime worker. A later bounded worker request ranks the current query. Only exact current workspace, inventory, and query generations may publish or open a file.

## Atomic claims

- **AEP-0168-C01:** Direct-file launch, folder construction, and the first frame perform no recursive walk. One explicit quick-open request inspects at most 250,000 entries and 256 levels, retains at most 100,000 regular UTF-8 paths, 16 MiB of path bytes, and 4 KiB per path, and reports every omission, error, and truncation.
- **AEP-0168-C02:** Serial traversal honors project-local `.gitignore`, nested `.gitignore`, `.ignore`, and repository exclude rules while disabling global and parent ignore state. Hidden files remain eligible except `.git`, symlinks are not followed, and retained paths have deterministic UTF-8 byte order.
- **AEP-0168-C03:** Inventory and query results carry workspace and monotonic generation identity. Stale, cancelled, late, mismatched, failed, and exhausted work cannot replace current state or mutate a document.
- **AEP-0168-C04:** Query ranking retains at most 1,024 index and score records under 1 MiB, uses deterministic subsequence scoring and byte-order ties, projects at most 256 visible rows, and opens only a component-revalidated, non-symlink file under the canonical root through the existing atomic tab path.

## Ownership, performance, and memory

`alpine-studio` owns the inventory, query state, and `ignore` 0.4.33 boundary. It uses `WalkBuilder::build`, never the parallel walker. The existing one-thread runtime pool bounds scheduling, and immutable `Arc` inventory ownership lets ranking run without copying path bytes. Both inventory and ranking remain outside scene construction and input mutation. A frame clones only visible path labels plus three overscan rows.

The dependency is exact-version and default-feature-disabled. Its license, Rust version, direct transitive graph, alternatives, and reversal conditions are retained in Decision #169. Merge evidence records lockfile review, release binary impact, no startup work, exact retained bytes, and post-close worker drain.

## Measured dependency impact

Task #168 was measured against `origin/main` revision `93160b7ab9045e88922168e910166d9aee7ec097` on the same Apple Silicon host with locked dependencies and the repository release profile. The unstripped `alpine-studio` executable increased from 1,151,248 to 2,333,344 bytes, a 1,182,096 byte or 102.680% increase. Copies stripped with `strip -S -x` increased from 907,400 to 1,844,360 bytes, a 936,960 byte or 103.258% increase. The release profile contains debug information, so the stripped result is the closer shipping-code comparison; both values remain retained to prevent selective reporting.

No binary-size dominance claim or arbitrary pass threshold is inferred from this measurement. The absolute stripped cost is accepted for this slice because `ignore` supplies reviewed project-local Git ignore semantics, nested rule handling, repository excludes, and bounded serial traversal without a custom parser. Startup tests prove the walk is not constructed before Command-P. Unit and journey evidence prove exact inventory, path-byte, result-metadata, and visible-row ceilings. Runtime worker tests prove admitted work drains on application teardown, while generation tests prove a closed or replaced quick-open state cannot publish late work. Requalification must replace the dependency if fixed-hardware startup, footprint, or residency evidence shows this approximately 915 KiB shipping cost is not justified by correctness and delivery risk.

## Correctness and failure behavior

Selection accepts only normal relative components, rechecks every component with `symlink_metadata`, rejects links and non-directories, rechecks the final regular file after the testable race seam, canonicalizes it, and requires exact lexical and canonical identity under the workspace root. Any failure preserves document bytes, identity, tabs, history, selection, scroll, find state, and IME state.

`QuickOpenAdmission.tla` models open, close, inventory publication, query change, query publication, and selection. It proves that published generations are current and selection can only use one current published query. Faulty stale-publication and stale-selection controls must violate those invariants. The model does not claim refinement to Rust, filesystem traversal, ignore parsing, allocation, worker scheduling, or native input.

## Scope exclusions and reversal conditions

This AEP does not authorize content search, watching, live index maintenance, parallel traversal, command discovery, splits, restoration, plugins, terminal, Git UI, AI, collaboration, cloud, remote development, telemetry, network access, or public framework indexing APIs.

Remove or replace `ignore` if measured binary, launch, memory, maintenance, license, unsafe, or transitive impact exceeds Task #168 evidence, or fixture behavior diverges from the accepted project-local semantics. Any replacement must preserve serial bounded work, revision-safe publication, explicit degradation, no startup work, and no plugin or network boundary.
