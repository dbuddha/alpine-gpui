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

The first recovery slices merged through PRs #608 and #609; product main is
`e683352`. The release app
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
They bound the complete observer event count and retain bounded private diagnostic
prefixes when retention succeeds. If copying or hashing fails, the original
temporary capture survives instead. External inspection
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

PR #608 passed PR CI in 7m06s and main CI in 6m45s. PR #609 passed PR CI in
8m08s and resulting main CI in 7m18s, including queueing. The lab cleanup main run
passed in 25m08s, exceeding the 15-minute target. Its pipeline rebuilds the release
sampler after the aggregate gate instead of reusing the already verified candidate;
this overhead remains unresolved and is not a product performance result.

Further release UI checks observed an exact Find match, command-palette split/close
and repaint after narrowing the window. Find's Command-A does not select the query:
`handle_find_key` has no select-all route. This is separate from the archived LSP
workspace-overlay changes. Do not treat the latter as a ready fix for Find.
Automation entered only the ASCII prefix of an accented search string; that
observation does not qualify or diagnose real IME input. A hide shortcut was sent,
but its lifecycle outcome was not independently observed.

An original Swift/MetalKit endpoint control, using command-buffer presentation,
submitted 240 frames while its application was active and its window key. All GPU
work completed and all presented handlers ran, but every actual `presentedTime`
was zero. An independent review verified source/binary identities and matching raw
frame records. Alpine-specific code is unnecessary to reproduce this observation
in this machine/session. Focus was checked at submission, not physical presentation;
this establishes neither an Apple defect nor visibility, refresh rate or latency.
The first control was unfocused and is retained as a limited diagnostic, not the
foreground result. Sources and private receipts remain in the lab's
`.lab/block2-mtkview-control` and `.lab/block2-mtkview-foreground`, with a
checksum-verified recovery copy at
`/Users/deepak/alpine-recovery/block2-20260912T065728Z`. The foreground
observation SHA-256 is
`389ba1a1cd65cb3e8860099050a1687229e54459d10863d38afcc656fd55822c`.
Do not substitute GPU completion for presentation or repeat presentation variants
without a new discriminator.

Block 2 is not accepted yet. Physical presentation, full external editor
accessibility, human keyboard/IME/VoiceOver and unobserved native journeys remain
open. The disposable release app is available for the requested human check.
Continue independent readiness work within the block budget; comparator expansion
still requires the human checkpoint. Keep the lab CI overrun visible without
broadening native recovery into another pipeline redesign.


## Adversarial audit corrections

The earlier probe's executable had been replaced without updating its outer and
embedded identities. Preserve it as an unqualified historical artifact. The new
`prepare-readiness-probe.py` command in [development](development.md) creates a
fresh release app from clean committed source, with an isolated workspace/home
and the checksummed rust-analyzer. Run its verifier before using a probe.

At source `c49a43a1517c513841758b295cc6444c23e7f61d`, the new probe verified all
eight immutable files. Controlled UI checks observed real rust-analyzer diagnostic
rows and the expected type mismatch. Find Command-A highlighted the query;
subsequent typing replaced it and found the expected single match. The app and
observations are in `target/readiness-audit-20260912`. An external accessibility
click on the editor still failed as offscreen after foregrounding, while a visible
coordinate click worked. Full AX and human IME/VoiceOver acceptance remain open.

The previous current-Alpine smoke receipt stored a diff hash without its patch,
so it cannot independently reconstruct that tree. Its refreshed replacement in
the lab's `artifacts/readiness-audit-smoke-20260912` uses clean committed source
`c49a43a`, retained release binary/build log, toolchain and exact commands. Fresh
current-Alpine, pinned GPUI and CPU readbacks match. The fixture is only 64x32,
seven glyphs and four quads. It proves a minimal usable comparison path; it is
not representative editor, text-shaping, memory or latency evidence.

The CI correction preserves product-boundary failures through `tee` and skips
code builds only for a conservative Markdown-only change set. Regression fixtures
reject failed required native jobs and invalid selection. Full local policy,
Clippy and explicit hosted-native execution passed. One ordinary concurrent local
editor run had five mock language-server initialization timeouts; all 44 language
server tests passed in isolation and the native-enabled run passed 541 editor
tests. Retain that first failure as a test reliability concern.

[Product PR 611](https://github.com/dbuddha/alpine-gpui/pull/611) and
[lab PR 31](https://github.com/dbuddha/alpine-zed-lab/pull/31) carry the corrections
and live CI evidence. Lab publication now reuses one verified release candidate.
The first combined run passed native equivalence but stopped at the clean-source
guard because Python validation generated cache files. Redirect those outputs
under `.lab/`; keep the guard. Neither an attempted CI optimization nor the smoke
match accepts block 2. Physical presentation and the human checkpoint still block
comparator expansion.
