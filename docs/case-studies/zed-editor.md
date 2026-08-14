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
