---
name: app-builder
description: Build and qualify Alpine Editor and its reusable native GPUI framework, or study a named app to design a concrete Alpine experiment.
metadata:
  author: Deepak Buddha
  role: Alpine engineering skill
  updated: "2026-11-08"
  authority: Alpine project contract
  related: "AGENTS.md, project.md, notes.md, case studies/README.md"
---

# App Builder

Work from the active Alpine worktree. Read its root AGENTS.md, project.md, notes.md and the scoped instructions for the change. On a held PR, read its notes at the remote head as well. Use the current contract for supported workflows, milestone order, exclusions and acceptance thresholds.

## Turn a workflow into a change

Start with a user action and its observable result. For a save conflict, for example, identify who owns the edited bytes, whether recovery has acknowledged the current revision, and what the first Quit does if both writes fail. Test the exact disk and buffer contents. Preserve pending edits, document identity, selection and navigation across failures.

For reusable UI work, trace input admission through mutation, visible layout, shaping, immutable scene creation and Direct Metal submission. Bound queues, frame slots, caches and retained bytes. Name the release or eviction event. Preserve painter order, idle suspension and stale-revision rejection. Keep native accessibility and IME paths consistent with the document revision.

For language or debugger work, identify the workspace, document revision, process owner and cancellation path. Use local tools and bounded workers away from input and rendering. Test a late reply after a tab switch, an evicted server with an unsaved overlay, and a child that exits during a request when relevant. Use the accepted Vim, Git and Markdown scenarios from project.md when qualifying editor workflows.

## Choose evidence that observes the result

Use headless tests to verify file contents, session restoration, revision handling and worker cleanup. Run affected tests, formatting, clippy and policy checks under the repository rules. For a native window or presentation claim, use the clean installed build or specified fixture in an approved screen session. Verify the target scene or text and caret, capture timestamps and normal teardown. Report missing evidence explicitly.

For performance comparisons, freeze the workload, versions, profiles, trial counts and rejection rules before measuring. Verify equivalent server work. Report editor and child footprint separately and together. Measure first window, editable text and required language readiness separately. Report missed refreshes and p95 visible latency on the reference Mac. Use everyday settings for product claims.

## Study a reference

Open the case studies index in the active checkout. Read the relevant study or add a named reference with a primary source, a dated observation, an Alpine hypothesis and a bounded experiment. Preserve external code and license boundaries. Take a proposed change to project.md for owner review when it alters scope or a gate.

## Deliver and continue

Refresh remote state, preserve dirty work and advance one feature and PR at a time. Recheck applicable gates at the reviewed head and verify merged main. While waiting for display access, run independent deterministic checks or prepare a reproduction. Give Deepak the installed build, test guide, measurements and limitations at each milestone. Use root AGENTS.md for current approval boundaries.
