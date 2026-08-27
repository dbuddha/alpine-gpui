# Zed Editor stable application

- Reviewed: 2026-08-14
- Research: [#27](https://github.com/dbuddha/alpine-gpui/issues/27)
- Release: `v1.15.0`
- Revision: [`e17dc4f9d50db73a458b64dcce50ecd4878b98a3`](https://github.com/zed-industries/zed/tree/e17dc4f9d50db73a458b64dcce50ecd4878b98a3)
- License: the Zed application declares GPL-3.0-or-later
- Influence: behavioral, workload-based, validation-oriented, and differential

## Scope

The stable application was inventoried across its workspace shell, editor,
project model, text and input paths, platform integration, local language
tooling, terminal, Git, settings, accessibility, tests, performance tests, and
benchmark support. The application is a workload and behavior specimen, not a
source base for proprietary Alpine code.

The review follows the user path across the Zed application entry point,
workspace and project ownership, editor state and display mapping, buffer and
multibuffer operations, text layout, language and LSP integration, search,
terminal, Git, settings, themes, commands, input, IME, accessibility, and the UI
component layer. Tests and benchmark paths were reviewed alongside production
owners so an observed architecture pattern is not mistaken for a verified
contract.

## Daily-driver workload inventory

The first Alpine parity matrix derives workloads for local project open and
restore, file-tree navigation, tabs and splits, large-file first render,
virtualized scrolling, typing and undo, multi-cursor edits, search and command
filtering, syntax and language-server results, terminal output, Git diffs,
settings and keymaps, focus, IME, accessibility, resize, sleep and wake,
surface loss, cancellation, and shutdown. Each becomes an approved Requirement
before implementation rather than an automatically inherited Zed feature.

The research deliberately excludes hosted AI, collaboration, cloud accounts,
remote development, telemetry services, public marketplace compatibility,
debugging, Zed branding, and exact visual duplication from Alpine Studio's
initial daily-driver target.

## Included vs excluded evidence boundaries

- Included for parity design and requirement shaping: local project shell behavior and
  workspace navigation; editor and text behavior including cursor selection and undo
  groups; virtualized rendering and large-file behavior; language workflows
  (syntax, symbols, diagnostics, completion) under local adapter constraints;
  terminal/task and local Git workflow slices; settings, keymaps, command palette
  behavior, and accessibility scaffolding; macOS lifecycle effects for resize,
  focus, sleep/wake, surface loss, and shutdown.
- Not copied into Alpine implementation: direct source, assets, or upstream
  application code; hosted AI, collaboration, cloud-account, debugger, or
  marketplace claims; visual identity and raster duplication; behavior outside
  parent-approved requirements.

## Performance and memory research takeaways from Zed

- Zed offers a complete daily-driver workload shape across text, IME, accessibility,
  and long-running lifecycle operations; it is a strong comparator for parity stress.
- Zed benchmark paths are useful for harnessing, not a proof of user-visible latency.
  Headless submission and proxy budgets do not establish display-path timing.
- Alpine needs separate scene-level, full-path, and comparator-level measurement.
- Adaptation/adapter cost must be measured separately from core editor behavior to
  avoid false performance claims.
- Memory direction should track retained resources, allocation growth, readback
  behavior, and post-shutdown drain under soak.

## Findings

- **CS-ZED-006:** A faster editor comparison is invalid until document state,
  visible semantics, accessibility, lifecycle, and resource accounting are
  equivalent for the same journey.
- **CS-ZED-008:** A daily-driver editor exposes framework weaknesses that narrow
  renderer samples cannot: virtualization, text, focus, IME, accessibility,
  tasks, cancellation, recovery, and long-lived resource retention.
- **CS-ZED-009:** Zed application code is GPL-3.0-or-later while GPUI declares
  Apache-2.0. The internal GPL comparison lab and proprietary Alpine repository
  require separate source, artifact, and distribution boundaries.

## Alpine consequences

Alpine Studio independently implements approved local outcomes. It excludes AI,
collaboration, cloud accounts, remote development, telemetry services, a public
extension marketplace, debugger integration, Zed branding, and exact visual
duplication from the first daily-driver target.

The private lab may modify and run the pinned Zed application internally. No
combined binary is distributed without legal review, and no Zed application
source or asset enters Alpine.
