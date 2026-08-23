# Milestone dependency and acceptance gates

GitHub Milestones own live membership and state. This document owns stable
outcome and evidence semantics. It intentionally omits open counts, completion
percentages, owners, and due dates.

## Interpretation

- A milestone is an outcome cohort, not a numbered sprint.
- Parent issue counts do not measure delivery.
- Required accepted leaf tasks determine progress and closure.
- A merged implementation does not close remaining physical, residency, or
  comparative qualification.
- A due date is a forecast only when scope is frozen and throughput supports it.
- Dependencies below describe outcome prerequisites. Native GitHub dependency
  edges remain authoritative for individual tasks.

## Gate catalog

| Milestone | Stable outcome | Entry condition | Exit evidence | Explicit exclusion |
| --- | --- | --- | --- | --- |
| [M0: Governed foundation](https://github.com/dbuddha/alpine-gpui/milestone/1) | Safe public contracts, evidence policy, issue hierarchy, and CI foundations support consequential work | Accepted capability and requirement boundary | Required foundational leaves accepted; policy, hierarchy, and evidence gates green | Product readiness or performance dominance from governance alone |
| [M1: Direct Metal offscreen renderer](https://github.com/dbuddha/alpine-gpui/milestone/2) | Direct Metal output is admitted by semantic and pixel oracles and can participate in matched comparator traces | Accepted renderer protocol and workload semantics | Correctness-equivalent traces, retained identities, failure coverage, and the evidence required by M1 leaves | Universal framework or product superiority from a control trace |
| [M2: Native macOS presentation](https://github.com/dbuddha/alpine-gpui/milestone/3) | One owned AppKit surface presents demand-driven frames with bounded completion ownership and deterministic lifecycle behavior | M0 contracts and native architecture approval | Production close, recovery, frame ownership, zero-idle, native process, and required physical evidence accepted | Editor capability, cross-platform parity, or a continuous game loop |
| [M3: Local workspace shell](https://github.com/dbuddha/alpine-gpui/milestone/4) | Local folder, tree, tab, split, navigation, search, and restoration behavior composes into a bounded workspace shell | Text and runtime contracts sufficient for a vertical slice | Workspace journeys and failure behavior accepted through required leaves | Rust intelligence, accessibility qualification, or daily-driver dogfood |
| [M4: Text, IME, and accessibility](https://github.com/dbuddha/alpine-gpui/milestone/5) | Local text and single-window native interaction are correct through IME, focus, accessibility, lifecycle, and recovery | Native presentation and editor state identities exist | Hosted semantic tests, production-process journeys, physical accessibility and input evidence, lifecycle recovery, latency, residency, and drain required by the leaves | Multi-window, Linux, Windows, or pixel-only accessibility evidence |
| [M5: Alpine Studio daily-driver profile](https://github.com/dbuddha/alpine-gpui/milestone/6) | The selected Rust-first local editor supports sustained Alpine repository work safely and smoothly | M3 behavior and M4 correctness are accepted; release typing blocker is resolved | Complete selected language and settings behavior, no-bloat enforcement, bounded residency, no known data-loss or recurring lifecycle defect, and sustained dogfood evidence | Public release operations, comparative dominance, terminal, tasks, Git UI, plugins, AI, collaboration, cloud, remote development, or telemetry |
| [M6: Vulkan, Wayland, D3D12, and Win32](https://github.com/dbuddha/alpine-gpui/milestone/7) | Deferred non-macOS platform research and implementation has its own accepted scope | New owner-approved platform requirements and architecture decisions | Platform-specific correctness, lifecycle, renderer, accessibility, and qualification evidence | Blocking the Apple Silicon private daily driver or macOS version 1 |
| [M7: Version 1 stabilization](https://github.com/dbuddha/alpine-gpui/milestone/8) | Alpine GPUI and Alpine Studio are supportable, distributable macOS version 1 products | M5 accepted; required M1 and comparative evidence accepted for any release claim | API and compatibility review, packaging, signing, notarization, install and update recovery, checksums, manifest, SBOM, attestations, known issues, and revision-scoped qualification | Claims unsupported by the release evidence or deferred feature breadth |

## Dependency graph

```text
M0 -> M2 -> M4 -> M5 -> M7
M0 -> M3 -------> M5
M0 -> M1 -> realistic traces -> E4 claims -> M7

M6 is independently deferred.
```

M3 may close before M2 or M4 because its accepted implementation leaves are a
separate outcome cohort. M5 still requires the production behavior supplied by
both M3 and M4.

## Closure audit

Before closing a milestone:

1. Enumerate required leaf tasks and their direct parents.
2. Check merged pull requests whose tasks remain open.
3. Check closed tasks lacking their named acceptance evidence.
4. Check unresolved native dependency edges and blockers.
5. Check whether deferred work leaked into the exit contract.
6. Check physical, soak, residency, or comparative artifacts that cannot be
   represented by hosted CI alone.
7. Check exact revision identity and authoritative `ci-pass` where applicable.
8. Record exclusions, remaining risks, and the next milestone entry decision.

Milestone closure is not evidence that all work with a smaller number is done,
and an open earlier milestone does not automatically block an independent later
outcome.
