# Alpine capability probe

## Goal and current boundary

Build toward one keyboard-first, accessible Apple Silicon macOS application:
a low-memory terminal, Alpine Editor, database views and an agent dock. Alpine
GPUI and Direct Metal are a candidate foundation, not an assumed winner over
pinned Zed GPUI, AppKit or SwiftUI. Source paths and the executable remain
`alpine-studio` during this probe.

The immediate deliverable is a reliable physical editor slice and a scoped
comparison. Terminal output, database grids and dock updates are bounded replays;
PTY compatibility, live queries and real agent integrations are not implemented
by this experiment. Shipping connectivity restrictions remain in effect.

## Six reviewed blocks

Each block has a four-hour budget and ends with a human review. Do not launch
six parallel implementations or automatically continue between blocks.

| Block | Outcome |
| --- | --- |
| 1. Consolidate | Restoration-verified recovery archive, one canonical checkout, consistent instructions and explicit native commands. |
| 2. Native readiness | Reproduce and fix demonstrated native blockers; essential editor journeys and one equivalent Alpine/GPUI case. |
| 3. Comparators | Matched Alpine, pinned GPUI, AppKit and SwiftUI candidates; missing candidates remain unqualified. |
| 4. Baseline | Calibrated memory, response, frame, idle and release measurements under individual and combined workloads. |
| 5. Improve | One profiled bottleneck corrected with a regression and no hidden behavior/resource loss. |
| 6. Decide | Independent rerun and a continue, narrow, pivot or inconclusive recommendation. |

If consolidation exceeds four hours, review scope. If a working physical editor
and comparison path remain absent after eight hours, stop benchmark expansion and
report recovery requirements. Do not silently spend the remaining budget repairing
infrastructure. Twenty-four hours is an investigation budget, not a guarantee of
four complete implementations or comparative superiority.

## Acceptance and provisional targets

Exercise release launch, typing, Unicode/IME, selection, clipboard, undo/redo,
save/reopen, dirty close, panes, search, diagnostics, resize, hide/show and teardown.
Use disposable files. Include physical keyboard/VoiceOver checks and the real
language-server integration separately from mocks. Keep native lifecycle, unsafe,
dependency/license and protected-merge safeguards.

| Metric | Experimental target |
| --- | --- |
| Matched footprint | At least 20% below the comparator, with no meaningful responsiveness regression |
| CPU frame preparation | p95 <=4 ms; p99 <=6 ms |
| Event handling through submission | p95 <=4 ms; p99 <=8 ms |
| Actual 120 Hz presentation | Fewer than 1% missed opportunities, only where measurable |
| Pane switching / idle CPU | <=50 ms / <1% of one core |
| One terminal-like replay | <=200 MiB |
| Combined probe | <=512 MiB steady; <=768 MiB peak |

Use the same 1280x800-point viewport at 2x, content and visible working set.
The combined corpus has eight terminal sessions with four visible, ten documents
with 20 MiB total source, a virtualized grid and a dock update stream. Bound
scrollback to 8 MiB/session, shared caches and undo to 64 MiB each, and result data
to 32 MiB. Record exact visible rows, glyphs, retained content, update cadence and
supported semantics before comparing; never omit work to meet the budgets.

Use release builds, balanced trial order, unchanged controls, at least ten
fresh-process trials per candidate per window, and two quiet physical windows.
Keep compilation outside measurement. Analyze uncertainty across trials, not
correlated frames. A memory win requires the upper 95% confidence bound on the
footprint ratio to be <=0.80; responsiveness must meet the floors and have no
more than 10% regression on the matched latency endpoints. Missing evidence or
unresolved variance makes the relevant claim inconclusive.

Report process footprint, relevant child processes and GPU allocation accounting
without double-counting unified memory. Separate renderer replay, shaping,
application interaction, submission, completion and actual presentation. GPU
completion is not physical presentation or input-to-photon latency. A result
against a pinned GPUI revision is not a result against every current Zed version.

## Starting evidence and current status

The pre-consolidation audit used the tree of main `3b4f89b`. Workspace tests and
lab policy fixtures passed; main CI completed in 7m32s. Physical and locally
hosted-direct shipping smoke failed with `UnexpectedRunLoopExit` and no qualified
presentation. This does not establish a blank window or an Apple defect.
[Issue 511](https://github.com/dbuddha/alpine-gpui/issues/511) retains the existing
presentation investigation. Require a discriminating hypothesis before another
variant. This Mac selected Command Line Tools without the `metal` compiler.

[Issue 521](https://github.com/dbuddha/alpine-gpui/issues/521) records an older
equivalent renderer-submit-readback comparison favoring pinned GPUI by about 12%.
The lab's prepared-atlas traces omit font shaping. Production scene export in
[PR 585](https://github.com/dbuddha/alpine-gpui/pull/585) is not on this main and
has reported equivalence failures. These are readiness gaps, not accepted results.

Block 1 work and source dispositions are in [consolidation](alpine-consolidation.md).
No physical performance target or daily-driver acceptance is currently claimed.

## Block 2 readiness findings, 2026-09-12

Work is on `fix/native-readiness` from merged main `76d385d`. The release app
visibly renders and, in an isolated disposable home, passed Unicode paste/save,
keyboard typing, one-character undo/redo, dirty-close rejection, clean close and
reopen. Clipboard automation reported an acknowledgement timeout, although saved
bytes verified the paste. These are controlled UI observations, not latency, IME
or VoiceOver qualification. Two real pinned rust-analyzer lifecycle/product tests
passed with the CI-checksummed binary.

Fresh hosted shipping and all-target native suites passed without concurrent UI
input, including the new selector/enabled-state and revoked-element checks. Physical
shipping still failed: one submitted frame, zero qualified presentations, zero
keyboard/pointer events, `presentedTime=0`, and the five-second timeout. Its full
output is retained in `target/block2-physical-shipping.log`. The quiet control
rejects input interference as an explanation for this physical failure. No
speculative presentation change or weakened assertion was made.

The recovered AX accumulator and rejected-capture fixes passed focused independent review.
They bound the complete observer event count and preserve private diagnostic
prefixes on failure, including when copy or hashing fails. External inspection
also demonstrated missing role descriptions and incorrect enabled state: the
small selector/getter correction exposes container, tab-group, radio-button and
outline roles with the appropriate enabled state. A subsequent external capture confirmed the text-area node exists but has an
empty title. Exposing its existing semantic name through `accessibilityTitle`
makes the editor discoverable as a named text entry area. The native client now
keeps Alpine semantic IDs and assigns snapshot-scoped IDs to AppKit chrome, which
can have absent or duplicate native identifiers. Chrome cannot become an automatic
action target, output rows bind the actual application root, and queued events
from replaced snapshots are rejected. A physical diagnostic traversed 89 nodes;
zero observer events and a still-live stale control leave full AX qualification
unfulfilled. Raw menu/window data stays local.

The isolated lab admitted its existing verified release sampler bundle on this
physical M4: Alpine, pinned GPUI and CPU readbacks matched exactly for one viewport
fixture. Evidence is `artifacts/block2-physical-admission-20260912` in the lab. Its
Alpine pin is `2fdf5aa`, Zed pin `e17dc4f`, and bundle lab revision `f6aa2c2`;
this historical admission alone does not qualify current Alpine. The current
release `alpine-assurance` built from this readiness tree then independently
matched that same CPU/GPUI readback exactly. Its source-diff and binary identities
are in `current-alpine-identity.json` beside the readback. No performance claim
was made. Existing offline shader binaries made this path usable without Xcode.

PR #608 passed CI and merged the first recovery slice. The snapshot/title
correction follows separately. Block 2 is not accepted yet. Physical presentation,
full external editor accessibility,
IME/VoiceOver and the remaining native journeys must be resolved or explicitly
reviewed before comparator expansion. Keep the separate lab CI timing overrun
visible without broadening this slice into another pipeline redesign.
