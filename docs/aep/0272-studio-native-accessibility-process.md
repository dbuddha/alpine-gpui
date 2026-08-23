# AEP 0272: Real Studio native accessibility process journey

- Status: Accepted
- Decision: [#268](https://github.com/dbuddha/alpine-gpui/issues/268)
- Research: [#267](https://github.com/dbuddha/alpine-gpui/issues/267)
- Implementation: [#272](https://github.com/dbuddha/alpine-gpui/issues/272)
- Requirement: [#37](https://github.com/dbuddha/alpine-gpui/issues/37)

## Motivation and journey

Alpine's portable semantic tests and AppKit fixture prove their respective
layers, but neither proved that the real Studio application composes them. The
required journey opens one bounded local workspace, expands its lazy tree,
opens two Rust files, navigates tabs, admits and activates one real product
diagnostic, edits, executes save through the native command row, rejects dirty
close, saves, closes, and drains all ownership.

## Goals

- Query the complete current native Studio tree through production AppKit
  selectors while returning no native handle.
- Select an exact current action target by semantic role and bounded label.
- Prove file-tree, tab, diagnostic, command, save, dirty-close, and close
  authorities compose through one real Studio process journey.
- Prove stable queries submit no frame and accepted visible actions submit at
  most one latest coalesced frame.
- Prove exact role-and-label matching and both successful and failed native
  dispatch polarity through the production AppKit press path.
- Retain exact saved bytes and terminal native/runtime owner evidence.

## Non-goals

This AEP does not automate VoiceOver, claim external notification delivery,
replace Task #273's AX observer, measure physical latency or residency, add
another semantic tree, expose AppKit objects, or add a shipping test fixture,
dependency, plugin, AI, cloud, telemetry, GPUI, WGPU, or game-rendering path.

## Validation-only native boundary

The native inspector activates accessibility, reconciles the production cache,
and calls role, label, identifier, focus, selection, press-admission, validity,
and frame selectors on every current native element. It copies only bounded
strings, booleans, semantic IDs, and finite-frame evidence. The named action
probe requires exactly one current node matching role and label, invokes the
same `accessibilityPerformPress` implementation used by assistive technology,
and returns action and revocation evidence.

The production journey validates every copied field rather than accepting its
shape alone. Each external identifier's terminal component must parse to the
exact semantic ID returned for that element. The focus accessors must reproduce
the inspector's independently accumulated AppKit focus count, while selection
and press-admission accessors must each expose both true and false states in the
complete tree. A constant identifier, semantic ID, focus, selection, or
press-admission result therefore rejects qualification.

Named activation relates the copied semantic ID back to the terminal component
of its generation-bound external identifier and requires a nonempty native
role. A wrong-role control pairs a current label with `CodeEditor` and requires
exact lookup rejection. A dispatch-failure control admits the initial snapshot
and real action, then deliberately withholds only the post-action snapshot
response. The same `accessibilityPerformPress` path must reject the press and
publish `dispatch_failed=true`; a subsequent normal query must recover the same
current semantic identity. The valid product tree correctly requires every
screen frame to be bounded, while an isolated handle-free evidence accessor
control preserves the false polarity without injecting invalid geometry into
the production tree.

Each negative control returns a distinct nonzero validation marker only after
its full body succeeds. The child evidence retains both markers and the parent
process requires their exact values, so replacing either control with a trivial
successful return cannot qualify. Frame-drain, hosted-observation admission,
action admission, frame ceilings, omission parsing, and returned process
evidence are factored into pure contracts with true, false, and nondefault
controls. Accessibility frame admission has its own complete truth table: a
frame requires both an exact action request and an `Applied` or `Unchanged`
typed action result. This independently rejects a weakened conjunction before
native composition evidence is considered. A platform-native companion fixture pairs the
real `main.rs` tab label with an absent `ListItem` role and requires exact
rejection. The absent role leaves the same-label tab as the sole false-positive
candidate when conjunction is weakened. A second control pairs the real `Tab`
role with an absent label, independently proving the label axis. Together they
own role equality, label equality, and conjunction mutations even when a
mutation scope runs only the platform package. The production journey remains
the composition proof while each lower boundary has an independently executable
control.

Sibling file labels from one directory result must be observed together in the
same native tree after that result's frame terminalizes. The journey does not
issue a redundant worker wake between accepting those labels. Native
accessibility actions retain the strict rule that a no-frame response must
already be quiescent, so unrelated draining cannot satisfy the action contract.

Any terminal-drain failure names its observation phase and retains bounded
initial and current evidence for occupied and submitted slots, native pacing,
submission and terminal counters, supersession, and the last terminal identity.
This diagnostic contains no native handles, scenes, documents, or callback
owners and does not alter shipping frame admission or completion behavior.

The process separates a five-second correctness deadline from the eight-frame
drain bound. It samples the native run loop in bounded 100 ms slices, but a
slice without a terminal transition does not consume the frame bound. The bound
counts frames actually submitted while the helper owns terminal observation.
Success also requires at least one observed submission whenever the caller owns
an expected frame; empty slots and paused pacing alone cannot prove that frame
submission occurred.
The idle-state companion counts qualified, superseded, skipped, failed, and
cancelled terminal attempts, requires every admitted submission to reach exactly
one terminal class, and does not return after a superseded attempt until one
current qualified presentation and all completion-owned slots are drained.
Its exact admission control counts one final qualified submission plus one
replacement for every superseded or skipped attempt. A skipped drawable is a
terminal attempt that still requires replacement; omitting it from the retry
identity incorrectly rejects an otherwise fully accounted drain.
These constants detect hangs and runaway work; they are not latency budgets or
performance qualification.

Mutation jobs for this module set
`ALPINE_STUDIO_NATIVE_PROCESS_SCOPE=accessibility`. The native process target
therefore runs the accessibility child and omission controls without also
running unrelated shipping, recovery, clipboard, file-tree, or search journeys.
The complete native process remains required by Metal behavior validation. This
is test-selection ownership, not an exclusion from production-path assurance.

Both probes exist only under `alpine_native_validation`. They do not widen the
safe shipping surface, retain Studio state, create a callback registry, or
permit arbitrary actions.

Linux changed-code mutation explicitly excludes this macOS-only process file
because its native child cannot execute there. Required Apple Silicon pull
request mutation owns changed process mutants across the same eight
deterministic shards as the other native scopes, retains one artifact per shard,
and feeds aggregate `ci-pass`. A separate nightly eight-shard job exhaustively
mutates the complete process file. Policy fixtures reject either side of this
ownership transfer when the Linux exclusion, native command, shard set, or
retained artifact is missing.

The same explicit transfer applies to Studio's validation-only language
evidence adapters. Cross-platform mutation excludes their exact function names
because `alpine_native_validation` compiles them out, while Apple pull-request
and nightly mutation select those names with the validation configuration and
retain their results. Shipping visual-change composition remains in normal
changed-code mutation. No viable mutant is omitted from its executable owner.

Each probe performs one refresh. A redundant `accessibilityChildren` preflight
is prohibited because it doubles synchronous main-thread query work and allows
an asynchronous result to replace native instances between discovery and the
actual named operation.

The hosted process waits for each accepted action frame to reach a terminal
state before issuing the next action. This models successive AppKit run-loop
turns and prevents the test harness itself from exhausting the production
three-slot frame bound.

The Studio event handler remains installed for every terminal-drain run-loop
interval. This matches production ownership and ensures an asynchronous worker
wake cannot receive a default response while native accessibility is active.
The handler is cleared immediately after each bounded drain so the next probe
must acquire ownership explicitly.

An action's synchronous response may return at most one frame. Background work
admitted while that frame drains is not attributed to the action, but every
resulting frame must reach zero slot ownership and paused pacing within eight
terminal drains. A true no-frame action arms no hosted observation and must
already be quiescent.

The process inherits the established presentation-evidence mode. Physical mode
uses compositor observations and may inject only the existing surface
configuration when AppKit has not yet published visibility. Explicit
`hosted-direct` mode injects one positive post-commit observation for each
expected frame because hosted CI may provide callback drawables without a
compositor presentation. The injected observation is armed before the run loop
turn, but the production presentation driver cannot terminalize it until Metal
command completion releases the frame slot. Invalid evidence-mode values fail
closed. Hosted mode proves Direct Metal commit, composition, terminal frame
ownership, and drain; it is not physical presentation, timing, AX client, or
VoiceOver evidence.

Document edits in this journey use `NSTextInputClient`'s production
`insertText:replacementRange:` selector and the live native responder epoch.
Synthetic `SurfaceEvent::Ime` values are prohibited here because an assumed
epoch can silently turn an intended edit into the correct stale-input no-op.
The probe installs dispatch before responder activation, publishes the exact
current focus state, and detaches without suspending the still-live input epoch.
Tab and diagnostic accessibility activation transfer semantic focus away from
the file tree just as their pointer equivalents do, so subsequent native text
targets the editor rather than being correctly ignored by a focused sidebar.

An accepted close revokes the native accessibility adapter before publishing
final focus loss. This prevents post-acceptance focus dispatch from requesting
a semantic refresh after runtime has already revoked new application work.
The `Allow` response also returns to AppKit without a semantic refresh, while a
cancelled close refreshes normally to publish its blocking status.

The production request bridge admits a frame only for an exact revision-valid
action whose typed result is `Applied` or `Unchanged`. It submits that frame
through the same bounded latest-scene helper used by keyboard and pointer input.
An `Unchanged` action may expose only dirty work that was already pending. Every
query, failed action, clipboard side effect, and close side effect with a frame
fails closed. This rule is consequential: the prior fixture never returned a
frame, so it hid that real visible Studio actions were being rejected
after mutation.

Runtime also separates query and action causality from concurrent worker
publication. A query leaves newly completed bounded results queued and observes
the last complete state and projection without consuming a frame. The existing
explicit wake drains those results, emits one latest coalesced frame, and
publishes projection identity only after derived visible lines are final. An
action executes against its complete projection before the bounded drain, after
which foreground and background effects share at most one frame. Studio's
state/projection guard still rejects any unexpected mixed snapshot, and the
native adapter retains its previous complete tree without reconciliation or
notification. Portable regressions require no query drain or frame, one wake
drain and frame, foreground action ordering, one coalesced action frame, a
complete current snapshot, then zero idle frames.

The diagnostic journey must not inspect the native tree before complete
process-bound language authority is observable. After authority, inspection is
admitted only for a newer complete semantic revision or a real requested frame.
Superseded wakes are valid only when their count is bounded by actual foreground
or latch observations, current published and observed generation identity is
exact, no wake remains pending, and no restart occurred. This keeps stale work
non-mutating and qualification deterministic without unconditional inspection,
timeout widening, or synthetic frames.

Diagnostic readiness uses a separate ten-second child-language correctness
watchdog rather than the five-second GPU frame-terminal deadline. The exact-head
eight-shard mutation run proved that five seconds could expire while the generic
worker queue was empty and all admitted external results were drained. This is
a validation-capacity observation, not a product latency target. The wait checks
the complete native tree once initially, after a wake admits a visual frame, or
once when the complete semantic and ordered server authority first becomes
observable; empty wakes otherwise sleep for five milliseconds without
rebuilding the tree. The authority-triggered inspection requests no frame and
occurs at most once. A terminal failure retains wake, frame, tree-inspection,
authority-inspection, surface, worker, and external-queue counters. It also
retains one bounded validation-only
language phase record. The isolated mock server records process start,
initialize receipt and response, initialized receipt, did-open receipt, and
diagnostics write in exact order. Studio separately records language sync,
process input, wake callback, latch publication, external admission, foreground
poll, diagnostic admission, semantic invalidation, and frame-build counters.
The two records distinguish server scheduling and protocol progress from
foreground handoff and native-tree publication without retaining process
handles, messages, documents, or scenes. Successful readiness remains
demand-driven and does not add a timer poller or continuous frame loop.
Successful label discovery is not sufficient by itself. Before the diagnostic
can qualify, the child requires an active exact-generation language session,
three submitted and written startup messages, a server wake, latch publication,
accepted external handoff, foreground poll, admitted nonempty diagnostic batch,
semantic invalidation, a scene-complete accessibility projection, and a
post-initial frame build. Every external handoff
must be classified. A bounded `Full` result is accepted only when that exact
generation reaches the foreground through an admitted result or the
latest-generation latch, and the latch is then empty. Disconnect, shutdown, sequence
exhaustion, incomplete accounting, input saturation, stale wake, or restart
remain terminal qualification failures. A complete ordered server trace is
required independently. Pure polarity controls remove each authority axis in
turn so a seeded, restored, stale, or bypassed diagnostic cannot satisfy the
production-process claim.

## Process composition and ownership

The custom native-process test starts an isolated qualification child. That
child receives an exact executable path to a shell wrapper which launches the
existing bounded mock Language Server Protocol fixture in a second process.
No process environment is mutated after threads exist, and no globally
installed language server is trusted. The parent gives each isolated child a
unique phase-trace path under that child's temporary home. The wrapper appends
`wrapper-invoked:<pid>` and the server appends `process-spawned:<pid>` followed
by six fixed protocol phases. Each server phase and its newline are assembled
before one append operation so superseded processes cannot splice two logical
records at a formatting boundary. Because the wrapper replaces itself with the
server executable, matching nonzero process identifiers prove attempt ownership.
Before runtime construction, the qualification child verifies that its selected
server file is that wrapper, verifies that the configured process executable
canonicalizes to its own executable, and appends one fixed
`qualification-child` phase. The wrapper explicitly exports mock-server mode
before replacing itself with that executable. The child reads at most 4,096
bytes on a terminal readiness failure. The parent requires one qualification
phase, zero or more valid ordered-prefix attempts, and one complete final
attempt. This admits an interrupted process when a production file switch
supersedes it without conflating attempts or accepting reordered, malformed,
PID-mismatched, or incomplete final evidence. Partial trace evidence remains in
child failure output.
This validation file is not created by shipping Studio and is removed with the
fixture after terminal owner drain.

Production validation recording is compiled into the integration-process
dependency but not executed automatically by `cfg(test)` unit-library builds.
The atomics and their direct polarity tests remain available there, preventing
parallel unit tests from contaminating exact process evidence or each other.
Language handoff evidence records admitted, full, disconnected, shutting-down,
and sequence-exhausted outcomes separately, plus the last published, observed,
and pending wake generation. This mirrors the production latch contract: queue
contention may be recovered without losing the latest generation, but no fatal
admission state or undrained latch may qualify.

After the complete journey succeeds, five isolated negative children each omit
exactly one required open, edit, accessibility action, save, or close step. A
control is accepted only when the same production journey rejects that omission;
an omitted step that still reaches qualification fails the parent process test.
The `open` control rejects before the exact `main.rs` tab can become active and
before an ordinary event can start Rust diagnostics. Its exact language trace is
therefore the qualification-child ownership phase only. The edit, accessibility
action, save, and close controls first pass diagnostic readiness and must retain
the same PID-bound complete final language lifecycle as the successful journey.
The parent reports the scenario with every mismatch so a pre-language omission
cannot be mistaken for a mock-server scheduling or protocol failure.
Action totals are derived from successful native dispatches rather than encoded
as expected constants in the returned evidence.

Studio owns one real temporary Cargo workspace and uses its production runtime,
worker queues, file-tree actions, tab store, Rust diagnostic state, scene
builder, AppKit adapter, Direct Metal surface, atomic save, close response, and
teardown paths. The fixture directory is removed only after every owner drains.

## Correctness, performance, and memory

Every native query and action carries the document, text-buffer, and non-text
semantic revision returned by the last complete Studio projection. Studio
advances immediate state identity on each accessibility-visible change,
including language publication, so stale actions cannot cross selection,
diagnostic, workspace, or overlay changes that leave text unchanged. It
publishes that identity to snapshots only after scene-derived accessibility
inputs are complete. Pending queries retain the previous native tree and cannot
publish a mixed payload. Existing stale-action and input-epoch controls remain
mandatory companions. A stable query compares Metal submission counts before and after and
requires an exact zero delta. Each accepted visible action requires a delta no
greater than one. File-tree progress remains bounded to 1,024 explicit wake
turns. Diagnostic child-process admission is serviced by explicit wakes under
the separate ten-second correctness watchdog because empty polling-turn counts
vary with scheduler and process-start latency. Native tree inspection occurs
only initially, after an admitted visual frame, or once for each newer semantic
revision after complete diagnostic and server authority becomes observable. The
tree-reported semantic revision must be monotonic, and a caught-up tree is not
queried again without another frame or semantic publication. A failure retains
bounded poll, frame, tree-inspection, inspected-semantic-revision, surface,
worker, and external-queue counters. No
timer poller or continuous frame loop is introduced, and this watchdog is not a
product latency budget.

The native tree cannot exceed the existing 271-node semantic ceiling. Evidence
copies one bounded role, label, and identifier per current node and is released
at the end of each synchronous probe. Close requires all ten native owner
classes inactive, exact acquisition/release parity for the nine exercised
classes, no pasteboard acquisition in this clipboard-free journey, no
release-order violation, no occupied frame slot, and the runtime shutdown
snapshot.

## Failure behavior

Missing, duplicate, disabled, revoked, or dispatch-failed native targets return
the existing structured accessibility validation failure. Exhausting the
file-tree wake bound or diagnostic correctness deadline fails the bounded
journey. Save failure, dirty-close acceptance, unexpected bytes,
multiple query frames, more than one action frame, an active owner, or fixture
cleanup failure rejects qualification. The parent watchdog terminates a child
that exceeds fifteen seconds and retains stdout and stderr in its error.

## Formal applicability

This AEP adds no new production state machine. Input-epoch ordering remains
covered by AEP-0268, native identity and stale action rejection by AEP-0270, and
notification/revocation ordering by AEP-0271. The process journey is
implementation composition evidence and makes no TLA+ refinement claim.

## Atomic claims and evidence

- **AEP-0272-C01:** One real Studio workspace publishes bounded native roles,
  labels, focus, selection, identifiers, frames, and actions from one
  scene-complete semantic projection. Immediate state identity rejects stale
  actions, and a pending projection cannot publish new revision identity with
  old derived payload. Evidence: native child process, exact tree assertions,
  pending-projection regression, native mutation, and coverage.
- **AEP-0272-C02:** File rows, tabs, diagnostics, and command rows route through
  existing Studio authorities; stable queries submit zero frames and successful
  revision-valid action responses return at most one frame through the common
  frame helper. Exact role-and-label mismatches fail, dispatch failure preserves
  true polarity, and independently admitted background frames drain to bounded
  quiescence. Evidence: native action sequence, mismatch and failed-refresh
  controls, response-frame counts, keyboard companion journeys, rejected
  side-effect controls, and viable mutation rejection. This is a functional
  scheduling invariant, not a timing or renderer-performance claim.
- **AEP-0272-C03:** Editing, save, dirty-close rejection, second save, and close
  preserve exact fixture bytes and recovery authority. Evidence: filesystem
  assertions, production close delegate, and process watchdog controls.
- **AEP-0272-C04:** Language, worker, scene, frame, file, semantic, and native
ownership is bounded and terminally drained. Evidence: bounded file-tree wake
count, demand-driven diagnostic readiness with poll, tree, semantic-authority,
surface, and queue evidence, isolated server exit, runtime shutdown snapshot,
frame evidence, and exact
  nine-class acquisition/release parity with all ten classes inactive.

## Risks and reversal conditions

Hosted AppKit remains API behavior, not physical VoiceOver evidence. Reopen if
Task #273's external observer cannot locate the same identifiers, physical
navigation requires a missing focus action, the deterministic fixture diverges
from the pinned real-server protocol, or the journey exposes positive residency
or frame-demand slope. Those findings require a superseding AEP rather than a
test-only product bypass.
