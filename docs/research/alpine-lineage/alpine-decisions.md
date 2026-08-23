# Alpine lineage decisions

| ID | Decision | Status | Rationale | Revisit gate |
| --- | --- | --- | --- | --- |
| ALD-001 | Direct Metal remains the Apple v1 shipping renderer | Accepted | Lowest controlled stack for the current platform and qualification goal | Only after M5/E4 evidence shows an unfixable backend limitation |
| ALD-002 | GPUI is a pinned research/comparator source, not a shipping dependency | Accepted | Alpine needs narrower ownership, errors, evidence, and product scope | No compatibility work without an accepted product requirement |
| ALD-003 | Zed application GPL source remains isolated in alpine-zed-lab | Accepted | Preserves legal and provenance boundary | Never silently relax |
| ALD-004 | WGPU remains E2 research and a possible differential oracle | Accepted | Its validation/lifetime discipline is useful; shipping breadth is not | After realistic trace E3 and a separate portability requirement |
| ALD-005 | awesome-gpui is workload discovery only | Accepted | Catalog metadata cannot prove code behavior or performance | Upgrade only after auditing a selected project's pinned source |
| ALD-006 | No GPUI entity graph or reactive global registry | Accepted | Direct Studio ownership is smaller and sufficient today | Repeated dogfood failure caused specifically by direct ownership |
| ALD-007 | General `Element` layout/prepaint/paint extraction is deferred | Accepted | Real repeated Studio contracts should drive the API | After sustained private dogfood and allocation baseline |
| ALD-008 | Retain strict zero-idle rendering | Accepted | Editors should not continuously render | Adopt a bounded present-only tail only after measured latency/energy win |
| ALD-009 | Retain three completion-owned frame slots | Accepted | Bounded nonblocking ownership matches Metal scheduling needs | Physical evidence shows a different calibrated bound is superior |
| ALD-010 | Keep quads, clips, and monochrome glyphs as the initial primitive set | Accepted | Covers the current editor without general graphics breadth | A daily-driver feature cannot be represented correctly |
| ALD-011 | Retain lookup-first atlas and row-delta GPU updates | Accepted | Deterministically avoids redundant warm work | Physical profiling finds a correctness or dominant-cost regression |
| ALD-012 | No performance claim below E4 | Accepted | Design inspection and local invariants are not comparative qualification | Never relax |
| ALD-013 | Report 120 Hz active deadline behavior, not universal FPS | Accepted | Idle zero frames are correct; active latency is the user outcome | Never replace with a headline FPS score |
| ALD-014 | Finish M4, typing latency, Rust/config gaps, and dogfood before feature expansion | Accepted | These are the shortest uncompromised path to a trusted editor | After M5 acceptance report |
| ALD-015 | Exclude AI, collaboration, cloud, telemetry, plugins, remote, debugger, terminal, and Git from M5 | Accepted | Avoids product weight and state machines outside the solo-editor goal | Separate accepted post-dogfood requirement |
| ALD-016 | mdBook is canonical and Wiki is a generated retrieval mirror | Accepted | Versioned review and revision identity remain authoritative | Never store unique evidence in Wiki |
| ALD-017 | Maintain this ledger in material architecture and performance PRs | Accepted | Prevents stale origin and evidence narratives | CI may automate stronger checks after the first maintenance cycle |
| ALD-018 | Keep comparator pin and current-upstream review separate | Accepted | Prevents silent benchmark drift while allowing learning | Requirement #40 requalification |

## Rejected interpretations

- Similar structure does not mean copied code.
- Narrower scope does not mean measured lower memory.
- One draw call does not mean a faster renderer.
- A deterministic avoided-work count does not mean lower physical latency.
- A working `.app` does not mean daily-driver acceptance.
- Nineteen implemented feature families do not mean 79 percent readiness.
- Zed feature exclusion does not permit unfair normalized comparison.

## Immediate execution decision

The critical path is:

1. #304 and #314 typing latency capture and correction.
2. #253, #272, and #273 physical accessibility and lifecycle qualification.
3. #219 through #222 Rust intelligence and configuration completion.
4. #238 through #242 sustained dogfood, baselines, residency, and acceptance.
5. #53 and #61 realistic renderer trace qualification.
6. M7 claims and public release work only after those gates.
