# AEP 0171: Lazy bounded workspace file tree

- Status: accepted 2026-08-17
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Requirement: [#32](https://github.com/dbuddha/alpine-gpui/issues/32)
- Parent task: [#127](https://github.com/dbuddha/alpine-gpui/issues/127)
- Implementation task: [#171](https://github.com/dbuddha/alpine-gpui/issues/171)
- Decision: [#172](https://github.com/dbuddha/alpine-gpui/issues/172)
- Dependency precedent: [AEP 0168](0168-bounded-lazy-workspace-inventory.md)

## Selected journey

A folder launch admits only one canonical root and paints its first frame without directory enumeration. The empty local sidebar activates through Command-Shift-E or its pointer surface. Activation admits one immediate-directory request on the existing serial bounded worker. Directory expansion repeats that operation without recursive discovery, while files open through the existing component-revalidated workspace and tab path.

## Atomic claims

- **AEP-0171-C01:** Production folder construction performs no directory enumeration. Explicit tree activation inspects at most 16,384 immediate entries per directory, retains at most 4,096 children and 1 MiB of path bytes per result, and reports truncation, omissions, and bounded errors.
- **AEP-0171-C02:** The private cache retains at most 4,096 directory nodes, 65,536 entries, 8 MiB of path bytes, 4 KiB per path, and 256 components. Project-local repository excludes, `.gitignore`, and `.ignore` files are evaluated from root to the requested directory; deeper and later local rules win, hidden paths remain eligible, `.git` is omitted, and symlinks are never traversed.
- **AEP-0171-C03:** Every worker result carries workspace, tree, directory, and request generations. Hidden, replaced, failed, mismatched, late, and stale work cannot publish entries or open a document. Collapsing a directory releases all cached descendants.
- **AEP-0171-C04:** Prefix row counts let a frame project at most 512 rows including three-row overscan without flattening the complete project. Keyboard or pointer activation opens only a current file row through exact canonical workspace containment; directory rows only mutate bounded tree state.

## Ownership, performance, and memory

`alpine-studio` owns the tree, cache, ignore matcher stack, and row projection. It adds no dependency, public API, native handle, worker, watcher, timer, polling loop, or async runtime. The existing `ignore` dependency parses project-local rules, but each request uses immediate `read_dir` enumeration rather than a recursive walker. Result publication accounts retained paths and entry counts before mutation. Prefix rows are recomputed only after accepted tree changes; paint uses subtree spans to skip non-visible ranges and shapes only the bounded returned rows.

The initial sidebar remains fixed width and deliberately has no drag resizing, icons, thumbnails, file operations, or restoration. There is no performance superiority claim. Fixed-hardware activation and scene-build distributions remain required before any latency claim.

## Correctness and formal scope

Directory requests revalidate each normal relative component with `symlink_metadata`, reject links and non-directories, canonicalize the target, and require exact canonical identity beneath the admitted root. Ignore files must be regular files and are never followed through a symlink. Selection delegates final file identity to `Workspace::path_for_relative_file`, preserving the existing race defenses.

`FileTreeAdmission.tla` models finite activation, hide, expansion, publication, and selection generations. It checks that publication and selection use the current tree and request. Faulty stale-publication and stale-selection controls must violate those invariants. The model does not claim Rust refinement, filesystem semantics, ignore correctness, allocation behavior, runtime scheduling, native event delivery, or elapsed-time bounds.

## Exclusions and reversal conditions

This AEP does not authorize recursive startup indexing, watchers, refresh, expansion restoration, drag and drop, rename, delete, create, icons, project search, command discovery, splits, terminal, Git UI, plugins, AI, collaboration, cloud, remote development, telemetry, network access, multi-window behavior, or framework-level tree and layout APIs.

Replace the matcher stack or cache representation if measured activation, footprint, retained allocator state, or scene-build evidence exceeds later accepted budgets. Any replacement must preserve root-only startup, one-directory admission, exact generation checks, path revalidation, bounded degradation, and local-only ownership.
