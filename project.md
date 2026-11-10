---
role: project objective and acceptance contract
author: Deepak Buddha
updated: 2026-11-08
authority: owner-approved plan of 2026-11-03 and research remit of 2026-11-08
related: [AGENTS.md, notes.md, apps/alpine-editor/AGENTS.md, bench/AGENTS.md]
audience: Deepak's daily work on his Apple Silicon Mac
distribution: local unsigned installed app, existing proprietary license
status: evidence and stability active
historical_baseline: M1 closed with limited qualified measurements
---

# Alpine project

## Objective

Build a daily-driver editor on a reusable, performance-first Alpine GPUI.
Framework speed and memory efficiency constrain every feature. Deliver the
editor first, then formally compare the framework. Correctness and no lost
work are mandatory; improve responsiveness and energy without doing less work.
There is no fixed delivery date: finish useful, verified milestones promptly.

Apple Silicon macOS is the sole platform target. The current M4 Mac and its
built-in ProMotion display are the performance reference. Keep the current
license; external distribution, signing and notarization are deferred.

Maintain the app-builder skill and source-backed case studies as part of delivery. Use the research to choose experiments for editor workflows, framework performance and headless correctness checks. Record observed behavior, test conditions and the resulting adoption decision. [Case studies](<case studies/README.md>)

## Required daily-driver scope

- Languages: Rust, Python, C++, Java, TypeScript, OCaml and Haskell. Qualify
  real-server workflows for each; existing JavaScript support is retained.
- Editing: reliable files, saves, recovery, undo, file tree, project search,
  preview tabs and back/forward navigation. Editing or pinning makes a
  preview tab permanent.
- Language tools: diagnostics, completion, hover, definition, references,
  rename, formatting and explicit code actions; inlay hints off by default.
- Structural editing: accurate multiline highlighting, syntax-aware
  selection, indentation, brackets, folding and a symbol outline. Select a
  parser through measured correctness, incremental-work, memory and license
  feasibility; dependencies and copied source need separate approval.
- Vim: normal, insert and visual modes; motions, operators, text objects,
  counts, registers, dot repeat, search, undo and common file commands.
  Broad Vim configuration compatibility and power-command expansion are
  deferred beyond this defined subset.
- Preferences: a small native UI backed by validated local configuration for
  font, theme, indentation, Vim, preview tabs and language tool paths.
- Git: local branch/status, gutter changes, visible-line blame, diff view,
  file/hunk stage and unstage, and commit. Sync and conflict resolution use
  external tools.
- Markdown: side-by-side CommonMark preview with tables, tasks, highlighted
  fenced code, local images, clickable links and scroll sync. No executable
  HTML or automatic remote resources; Mermaid and math are deferred.

Excluded: AI, collaboration, cloud/remote development, multi-cursor,
extensions and an integrated terminal. A separate terminal hosting the editor
remains a later project. Do not use the old all-Zed-features parity list as a
release requirement. Deepak's workflows govern feature acceptance.

## Stages and owner checkpoints

The sequence below supersedes the planned M2-M8 ordering. M0, M0e and the
limited M1 baseline remain historical completed work; S continues as stage 1.

| Stage | Deliverable | Acceptance |
| --- | --- | --- |
| 1: evidence and stability | Qualified physical evidence, existing PR queue, safe saves/recovery/quit/sessions/overlays/navigation | Required checks, five normal working days without data loss or a blocking failure, then Deepak's acceptance |
| 2: responsiveness | Semantic typing evidence, equivalent workspace/server load, frame and input performance, memory/startup feasibility | Qualified target results; every shortfall presented for explicit acceptance |
| 3: editing and languages | Seven-language qualification, language tools, structural editing, Vim, file/search/preview/preferences | Representative editing and real-server scenarios, then Deepak's verification |
| 4: daily-driver completion | Local Git, Markdown preview, cross-feature and everyday comparisons | Required workflows without fallback, five normal working days without data loss or blocking failures, then Deepak's release acceptance |
| 5: framework comparison | Matched Alpine GPUI and Zed GPUI scenarios, measured bottleneck fixes | Reproducible latency/memory/cadence/idle/load evidence, bounded resources and owner review |
| 6: debugger | Modern source debugger, initially Rust/C++/Python | Real sessions, failure checks and qualified editor responsiveness, then owner verification |

Every checkpoint includes an installed build, a short test guide, results and
limitations. Do not mark a milestone accepted until Deepak verifies it.
Independent preparation can continue while review or screen time is pending.
Use Alpine and disposable fixtures initially; owner trials supply real-work
validation without accessing other repositories implicitly.

The debugger uses a separate bounded adapter manager, sharing language and
workspace configuration with the LSP manager. Start only on explicit action,
with local adapter paths and no runtime downloads. Required capabilities:
launch/attach, breakpoints, step in/over/out, stacks, variables, watches,
evaluation console, and conditional breakpoints, logpoints, exception stops
and thread selection where supported. Java and TypeScript follow; OCaml and
Haskell require adapter feasibility before promising equivalent support.
Reverse debugging, machine-level views and child-process sessions are deferred.
No public framework API commitment is required for these stages.

## Acceptance matrix

All changes retain existing CI correctness checks. No gate implementation is
removed or bypassed by this document. Record the exact applicable gate before
each change and distinguish pass, fail, unavailable and owner-review pending.

| Change | Additional required evidence |
| --- | --- |
| Documentation only | Frontmatter, local links, instruction hierarchy, scope consistency, full tracked/untracked diff, fmt/clippy/policy |
| Product correctness | Affected crate tests; failing-before regression for bugs; failure-path tests for data; hosted native lifecycle checks where affected |
| UI/input | Deterministic state/geometry tests and approved installed-app journeys, including Dock-launch evidence |
| Frame path/startup/cache | Qualified physical performance rows on the final head, lifecycle checks and comparison against matched main |
| LSP | Real-server document/workspace workflows, stale-result/cancellation/eviction tests, equivalent-load memory and input responsiveness |
| Git/debugger | Explicit-operation and failure-path tests, bounded workers/processes, real tool sessions and input responsiveness |
| Milestone | Relevant matched benchmark results, installed build/test guide, known limitations and owner acceptance |

An unrelated known platform probe failure must not substitute for assessing a
change's applicable gates. For changes requiring physical evidence, the current
timestamp failure remains a hold until the screen-based replacement qualifies.
Keep the probe result visible after replacement and retain native lifecycle and
teardown checks. Missing timestamps, GPU completion, display-link targets and
hash changes alone are never measured presentation.

## Numeric contract and measurement

Release targets are exact: 120 Hz on the reference display, fewer than 1%
missed frames, and p95 input-to-visible-change at or below 25 ms (16.7 ms is
the stretch target). At 120 Hz, frame-interval p95 targets one 8.33 ms refresh;
missed frames are gaps over 1.5 refresh periods. Apply to frame-producing
interactions, including typing, scroll, resize and selection with services
active. Launch and file-open use separate budgets.

Development uses correctness and qualified non-regression gates. Preserve
existing qualified budgets below pending the feasibility audit. A measured
regression beyond uncertainty blocks the affected change; inconclusive or
missing measurements cannot establish non-regression. A release shortfall
requires Deepak's explicit acceptance; close to target is not a pass.

| Existing budget | Limit and qualification |
| --- | --- |
| CPU frame preparation p95 | 4 ms, retain existing stage endpoint |
| Event to submit p95 | 4 ms, not presentation |
| Idle CPU | 1% of one core |
| Idle frame submissions | 0 |
| Idle footprint | 30.5 MiB, original M1 idle-tree statistic; audit editor/tree attribution before reuse |
| Launch to first window median | 335 ms, retained diagnostic budget; not ready-to-edit |
| 50 MiB open to window median | 475 ms, retained diagnostic budget |
| Language-server memory | Report per process and tree; establish matched-load evidence before setting a new numeric budget |

Audit the historical startup statistics and footprint attribution before any
revision. Do not silently replace these budgets or compare idle memory to
workspace-loaded totals. Track first window, editable requested text and
required language readiness separately; the last is product startup acceptance.

Qualify intended text/caret or scene changes with valid screen capture display
timestamps. Report this endpoint and its uncertainty, not physical pixel-light
time. Measure phys_footprint for editor, children and total, including plateau
behavior. Both servers must demonstrably load equivalent workspaces.

Freeze workloads, versions, settings, trial counts and invalidation rules
before runs. Prospective default: one warm-up, twelve measured trials, at least
ten valid; keep invalid and unfavorable outcomes and never add trials to force
a win. Use controlled and everyday profiles; everyday governs product claims.
Final superiority claims require two independent sessions and favorable 95%
confidence intervals beyond measurement uncertainty. Report ties and failures;
zero failures in both apps is not a reliability win. AppKit is a component
reference, not an equivalent language-enabled editor.

The historical M1 baseline remains closed and limited. Semantic typing and
equivalent workspace-loaded LSP results are missing evidence, not completed
work. All unimplemented methods in this contract remain targets until their
qualification is recorded in notes.md and milestone evidence.
